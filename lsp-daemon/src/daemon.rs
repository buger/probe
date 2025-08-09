use crate::ipc::{IpcListener, IpcStream};
use crate::language_detector::{Language, LanguageDetector};
use crate::logging::{LogBuffer, MemoryLogLayer};
use crate::lsp_registry::LspRegistry;
use crate::protocol::{
    parse_call_hierarchy_from_lsp, CallHierarchyResult, DaemonRequest, DaemonResponse,
    DaemonStatus, LanguageInfo, MessageCodec, PoolStatus,
};
use crate::server_manager::SingleServerManager;
use crate::socket_path::{get_default_socket_path, remove_socket_file};
use crate::workspace_resolver::WorkspaceResolver;
use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use tracing_subscriber::prelude::*;
use uuid::Uuid;

pub struct LspDaemon {
    socket_path: String,
    registry: Arc<LspRegistry>,
    detector: Arc<LanguageDetector>,
    server_manager: Arc<SingleServerManager>,
    workspace_resolver: Arc<tokio::sync::Mutex<WorkspaceResolver>>,
    connections: Arc<DashMap<Uuid, Instant>>,
    start_time: Instant,
    request_count: Arc<RwLock<u64>>,
    shutdown: Arc<RwLock<bool>>,
    log_buffer: LogBuffer,
    // Performance metrics
    request_durations: Arc<RwLock<Vec<Duration>>>, // Keep last 100 request durations
    error_count: Arc<RwLock<usize>>,
}

impl LspDaemon {
    pub fn new(socket_path: String) -> Result<Self> {
        Self::new_with_config(socket_path, None)
    }

    pub fn new_with_config(
        socket_path: String,
        allowed_roots: Option<Vec<PathBuf>>,
    ) -> Result<Self> {
        let registry = Arc::new(LspRegistry::new()?);
        let detector = Arc::new(LanguageDetector::new());
        let server_manager = Arc::new(SingleServerManager::new(registry.clone()));
        let workspace_resolver = Arc::new(tokio::sync::Mutex::new(WorkspaceResolver::new(
            allowed_roots,
        )));

        // Create log buffer and set up tracing subscriber
        let log_buffer = LogBuffer::new();
        let memory_layer = MemoryLogLayer::new(log_buffer.clone());

        // Set up tracing subscriber with memory layer and optionally stderr
        use tracing_subscriber::EnvFilter;

        // Always use a filter to ensure INFO level is captured
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        let subscriber = tracing_subscriber::registry()
            .with(memory_layer)
            .with(filter);

        // If LSP_LOG is set, also add stderr logging
        if std::env::var("LSP_LOG").is_ok() {
            use tracing_subscriber::fmt;

            let fmt_layer = fmt::layer().with_target(false).with_writer(std::io::stderr);

            if tracing::subscriber::set_global_default(subscriber.with(fmt_layer)).is_ok() {
                tracing::info!("Tracing initialized with memory and stderr logging");
            }
        } else {
            // Memory logging only
            if tracing::subscriber::set_global_default(subscriber).is_ok() {
                tracing::info!("Tracing initialized with memory logging layer");
            }
        }

        Ok(Self {
            socket_path,
            registry,
            detector,
            server_manager,
            workspace_resolver,
            connections: Arc::new(DashMap::new()),
            start_time: Instant::now(),
            request_count: Arc::new(RwLock::new(0)),
            shutdown: Arc::new(RwLock::new(false)),
            log_buffer,
            request_durations: Arc::new(RwLock::new(Vec::with_capacity(100))),
            error_count: Arc::new(RwLock::new(0)),
        })
    }

    pub async fn run(&self) -> Result<()> {
        // Clean up any existing socket
        remove_socket_file(&self.socket_path)?;

        let listener = IpcListener::bind(&self.socket_path).await?;
        info!("LSP daemon listening on {}", self.socket_path);

        // Set up signal handling for graceful shutdown
        let daemon_for_signals = self.clone_refs();
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate())?;
            let mut sigint = signal(SignalKind::interrupt())?;

            tokio::spawn(async move {
                tokio::select! {
                    _ = sigterm.recv() => {
                        info!("Received SIGTERM, shutting down gracefully");
                        *daemon_for_signals.shutdown.write().await = true;
                    }
                    _ = sigint.recv() => {
                        info!("Received SIGINT, shutting down gracefully");
                        *daemon_for_signals.shutdown.write().await = true;
                    }
                }
            });
        }

        // Start idle checker
        let daemon = self.clone_refs();
        tokio::spawn(async move {
            daemon.idle_checker().await;
        });

        loop {
            // Check shutdown flag
            if *self.shutdown.read().await {
                info!("Daemon shutting down...");
                break;
            }

            match listener.accept().await {
                Ok(stream) => {
                    // Check if we've reached the connection limit
                    const MAX_CONNECTIONS: usize = 100; // Reasonable limit for concurrent connections

                    let current_connections = self.connections.len();
                    if current_connections >= MAX_CONNECTIONS {
                        warn!(
                            "Maximum connection limit reached ({}/{}), rejecting new connection",
                            current_connections, MAX_CONNECTIONS
                        );
                        // Drop the stream to close the connection
                        drop(stream);
                        continue;
                    }

                    let daemon = self.clone_refs();
                    tokio::spawn(async move {
                        if let Err(e) = daemon.handle_client(stream).await {
                            error!("Error handling client: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting connection: {}", e);
                }
            }
        }

        // Cleanup
        self.cleanup().await?;
        Ok(())
    }

    async fn handle_client(&self, mut stream: IpcStream) -> Result<()> {
        // Maximum message size: 10MB (reasonable for LSP messages)
        const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

        let client_id = Uuid::new_v4();
        info!("New client connected: {}", client_id);

        // Store connection timestamp
        self.connections.insert(client_id, Instant::now());

        let mut buffer = vec![0; 65536]; // 64KB initial buffer

        loop {
            // Read message length
            let n = stream.read(&mut buffer[..4]).await?;
            if n == 0 {
                // Connection closed - clean up
                self.connections.remove(&client_id);
                info!("Client disconnected: {}", client_id);
                break;
            }

            let msg_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

            // Validate message size to prevent OOM attacks
            if msg_len > MAX_MESSAGE_SIZE {
                error!(
                    "Client {} attempted to send oversized message: {} bytes (max: {} bytes)",
                    client_id, msg_len, MAX_MESSAGE_SIZE
                );
                self.connections.remove(&client_id);
                return Err(anyhow::anyhow!(
                    "Message size {} exceeds maximum allowed size of {} bytes",
                    msg_len,
                    MAX_MESSAGE_SIZE
                ));
            }

            // Read message body
            if msg_len > buffer.len() - 4 {
                buffer.resize(msg_len + 4, 0);
            }

            // Read with error handling that cleans up connection
            if let Err(e) = stream.read_exact(&mut buffer[4..4 + msg_len]).await {
                self.connections.remove(&client_id);
                error!(
                    "Failed to read message body from client {}: {}",
                    client_id, e
                );
                return Err(e.into());
            }

            // Decode request
            let request = match MessageCodec::decode_request(&buffer[..4 + msg_len]) {
                Ok(req) => req,
                Err(e) => {
                    self.connections.remove(&client_id);
                    error!("Failed to decode request from client {}: {}", client_id, e);
                    return Err(e);
                }
            };

            // Update activity timestamp
            self.connections.insert(client_id, Instant::now());

            // Increment request count
            *self.request_count.write().await += 1;

            // Handle request with timing
            let request_start = Instant::now();
            let response = self.handle_request(request).await;
            let request_duration = request_start.elapsed();

            // Track request duration (keep only last 100)
            {
                let mut durations = self.request_durations.write().await;
                durations.push(request_duration);
                if durations.len() > 100 {
                    durations.remove(0);
                }
            }

            // Track errors
            if let DaemonResponse::Error { .. } = &response {
                *self.error_count.write().await += 1;
            }

            // Send response
            let encoded = MessageCodec::encode_response(&response)?;
            stream.write_all(&encoded).await?;
            stream.flush().await?;

            // Check if shutdown was requested
            if let DaemonResponse::Shutdown { .. } = response {
                *self.shutdown.write().await = true;
                break;
            }
        }

        // Remove connection
        self.connections.remove(&client_id);
        info!("Client disconnected: {}", client_id);

        Ok(())
    }

    // Clean up connections that have been idle for too long
    fn cleanup_stale_connections(&self) {
        const MAX_IDLE_TIME: Duration = Duration::from_secs(300); // 5 minutes
        let now = Instant::now();

        self.connections.retain(|client_id, last_activity| {
            let idle_time = now.duration_since(*last_activity);
            if idle_time > MAX_IDLE_TIME {
                info!(
                    "Removing stale connection {}: idle for {:?}",
                    client_id, idle_time
                );
                false
            } else {
                true
            }
        });
    }

    async fn handle_request(&self, request: DaemonRequest) -> DaemonResponse {
        debug!(
            "Received daemon request: {:?}",
            std::mem::discriminant(&request)
        );

        // Periodically clean up stale connections (every 100 requests)
        let request_count = *self.request_count.read().await;
        if request_count % 100 == 0 {
            self.cleanup_stale_connections();
        }

        match request {
            DaemonRequest::Connect { client_id } => DaemonResponse::Connected {
                request_id: client_id,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            },

            DaemonRequest::InitializeWorkspace {
                request_id,
                workspace_root,
                language,
            } => {
                match self
                    .handle_initialize_workspace(workspace_root, language)
                    .await
                {
                    Ok((root, lang, server)) => DaemonResponse::WorkspaceInitialized {
                        request_id,
                        workspace_root: root,
                        language: lang,
                        lsp_server: server,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::ListWorkspaces { request_id } => {
                let workspaces = self.server_manager.get_all_workspaces().await;
                DaemonResponse::WorkspaceList {
                    request_id,
                    workspaces,
                }
            }

            DaemonRequest::HealthCheck { request_id } => {
                // Calculate health metrics
                let uptime_seconds = self.start_time.elapsed().as_secs();
                let total_requests = *self.request_count.read().await as usize;
                let active_connections = self.connections.len();
                let active_servers = self.server_manager.get_active_server_count().await;

                // Calculate average request duration
                let avg_request_duration_ms = {
                    let durations = self.request_durations.read().await;
                    if durations.is_empty() {
                        0.0
                    } else {
                        let total: Duration = durations.iter().sum();
                        total.as_millis() as f64 / durations.len() as f64
                    }
                };

                // Get error count
                let errors = *self.error_count.read().await;
                let error_rate = if total_requests > 0 {
                    (errors as f64 / total_requests as f64) * 100.0
                } else {
                    0.0
                };

                // Estimate memory usage (simplified - in production you'd use a proper memory profiler)
                let memory_usage_mb = {
                    // This is a rough estimate - consider using a proper memory profiler
                    let rusage = std::mem::size_of_val(self) as f64 / 1_048_576.0;
                    rusage + (active_servers as f64 * 50.0) // Estimate 50MB per LSP server
                };

                // Health is considered good if:
                // - Not at connection limit
                // - Reasonable memory usage
                // - Low error rate
                // - Reasonable response times
                let healthy = active_connections < 90
                    && memory_usage_mb < 1024.0
                    && error_rate < 5.0
                    && avg_request_duration_ms < 5000.0;

                info!(
                    "Health check: connections={}, memory={}MB, errors={}%, avg_duration={}ms",
                    active_connections, memory_usage_mb, error_rate, avg_request_duration_ms
                );

                DaemonResponse::HealthCheck {
                    request_id,
                    healthy,
                    uptime_seconds,
                    total_requests,
                    active_connections,
                    active_servers,
                    memory_usage_mb,
                }
            }

            DaemonRequest::CallHierarchy {
                request_id,
                file_path,
                line,
                column,
                workspace_hint,
            } => match self
                .handle_call_hierarchy(&file_path, line, column, workspace_hint)
                .await
            {
                Ok(result) => DaemonResponse::CallHierarchy { request_id, result },
                Err(e) => DaemonResponse::Error {
                    request_id,
                    error: e.to_string(),
                },
            },

            DaemonRequest::Status { request_id } => {
                let server_stats = self.server_manager.get_stats().await;
                let pool_status: Vec<PoolStatus> = server_stats
                    .into_iter()
                    .map(|s| PoolStatus {
                        language: s.language,
                        ready_servers: if s.initialized { 1 } else { 0 },
                        busy_servers: 0, // No busy concept in single server model
                        total_servers: 1,
                        workspaces: s
                            .workspaces
                            .iter()
                            .map(|w| w.to_string_lossy().to_string())
                            .collect(),
                        uptime_secs: s.uptime.as_secs(),
                        status: format!("{:?}", s.status),
                    })
                    .collect();

                DaemonResponse::Status {
                    request_id,
                    status: DaemonStatus {
                        uptime_secs: self.start_time.elapsed().as_secs(),
                        pools: pool_status,
                        total_requests: *self.request_count.read().await,
                        active_connections: self.connections.len(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        git_hash: env!("GIT_HASH").to_string(),
                        build_date: env!("BUILD_DATE").to_string(),
                    },
                }
            }

            DaemonRequest::ListLanguages { request_id } => {
                let languages = self.registry.list_available_servers();
                let language_info: Vec<LanguageInfo> = languages
                    .into_iter()
                    .map(|(lang, available)| {
                        let config = self.registry.get(lang);
                        LanguageInfo {
                            language: lang,
                            lsp_server: config
                                .map(|c| c.command.clone())
                                .unwrap_or_else(|| "unknown".to_string()),
                            available,
                        }
                    })
                    .collect();

                DaemonResponse::LanguageList {
                    request_id,
                    languages: language_info,
                }
            }

            DaemonRequest::Shutdown { request_id } => {
                info!("Shutdown requested");
                DaemonResponse::Shutdown { request_id }
            }

            DaemonRequest::Ping { request_id } => DaemonResponse::Pong { request_id },

            DaemonRequest::GetLogs { request_id, lines } => {
                let entries = self.log_buffer.get_last(lines);
                DaemonResponse::Logs {
                    request_id,
                    entries,
                }
            }

            _ => DaemonResponse::Error {
                request_id: Uuid::new_v4(),
                error: "Unsupported request type".to_string(),
            },
        }
    }

    async fn handle_call_hierarchy(
        &self,
        file_path: &Path,
        line: u32,
        column: u32,
        workspace_hint: Option<PathBuf>,
    ) -> Result<CallHierarchyResult> {
        // Use timeout to prevent hanging indefinitely
        let operation_timeout = tokio::time::Duration::from_secs(120); // 120 second timeout to accommodate rust-analyzer initialization

        tokio::time::timeout(
            operation_timeout,
            self.handle_call_hierarchy_inner(file_path, line, column, workspace_hint),
        )
        .await
        .map_err(|_| anyhow!("Call hierarchy operation timed out after 120 seconds"))?
    }

    async fn handle_call_hierarchy_inner(
        &self,
        file_path: &Path,
        line: u32,
        column: u32,
        workspace_hint: Option<PathBuf>,
    ) -> Result<CallHierarchyResult> {
        debug!(
            "handle_call_hierarchy_inner called for {:?} at {}:{}",
            file_path, line, column
        );

        // Detect language
        let language = self.detector.detect(file_path)?;

        if language == Language::Unknown {
            return Err(anyhow!("Unknown language for file: {:?}", file_path));
        }

        // Resolve workspace root
        let workspace_root = {
            let mut resolver = self.workspace_resolver.lock().await;
            resolver.resolve_workspace(file_path, workspace_hint)?
        };

        // Ensure workspace is registered with the server for this language
        let server_instance = self
            .server_manager
            .ensure_workspace_registered(language, workspace_root)
            .await?;

        // Convert relative path to absolute path for LSP server
        let absolute_file_path = file_path
            .canonicalize()
            .with_context(|| format!("Failed to resolve absolute path for {file_path:?}"))?;

        // Read file content
        let content = fs::read_to_string(file_path)?;

        // Lock the server instance to use it
        let server = server_instance.lock().await;

        // Open document
        server
            .server
            .open_document(&absolute_file_path, &content)
            .await?;

        // Give rust-analyzer a brief moment to process the document
        // Reduced from 10+2 seconds to 2 seconds since we have retry logic
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Try call hierarchy with retry logic - allow multiple attempts with shorter wait
        let max_attempts = 3; // Multiple attempts to handle cases where rust-analyzer needs more time
        let mut attempt = 1;
        let mut result = None;

        while attempt <= max_attempts {
            debug!("Call hierarchy attempt {} at {}:{}", attempt, line, column);
            let call_result = server
                .server
                .call_hierarchy(&absolute_file_path, line, column)
                .await;

            match call_result {
                Ok(response) => {
                    // Check the response from call_hierarchy method (which has already processed the LSP response)
                    // The response contains incoming/outgoing arrays or an item with name/uri info
                    if let Some(item) = response.get("item") {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            if name != "unknown" && !name.is_empty() {
                                result = Some(response);
                                break;
                            }
                        }
                    }

                    // Also check for any incoming/outgoing calls
                    if response
                        .get("incoming")
                        .and_then(|v| v.as_array())
                        .is_some_and(|arr| !arr.is_empty())
                        || response
                            .get("outgoing")
                            .and_then(|v| v.as_array())
                            .is_some_and(|arr| !arr.is_empty())
                    {
                        result = Some(response);
                        break;
                    }

                    result = Some(response); // Keep the last response even if empty
                }
                Err(e) => {
                    if attempt == max_attempts {
                        return Err(e);
                    }
                }
            }

            attempt += 1;
            if attempt <= max_attempts {
                // Shorter wait between attempts - 2 seconds instead of 5
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }

        let result = result.ok_or_else(|| {
            anyhow!(
                "Failed to get call hierarchy response after {} attempts",
                max_attempts
            )
        })?;

        // Close document
        server.server.close_document(&absolute_file_path).await?;

        // Parse result
        parse_call_hierarchy_from_lsp(&result)
    }

    async fn handle_initialize_workspace(
        &self,
        workspace_root: PathBuf,
        language_hint: Option<Language>,
    ) -> Result<(PathBuf, Language, String)> {
        // Validate workspace root exists
        if !workspace_root.exists() {
            return Err(anyhow!(
                "Workspace root does not exist: {:?}",
                workspace_root
            ));
        }

        // Check if workspace is allowed
        {
            let resolver = self.workspace_resolver.lock().await;
            if !resolver.is_path_allowed(&workspace_root) {
                return Err(anyhow!(
                    "Workspace {:?} not in allowed roots",
                    workspace_root
                ));
            }
        }

        // Determine language - use hint if provided, otherwise detect from workspace
        let language = if let Some(lang) = language_hint {
            lang
        } else {
            // Try to detect language from common files in workspace
            self.detect_workspace_language(&workspace_root)?
        };

        // Get LSP server config
        let config = self
            .registry
            .get(language)
            .ok_or_else(|| anyhow!("No LSP server configured for {:?}", language))?
            .clone();

        // Ensure workspace is registered with the server
        let _server_instance = self
            .server_manager
            .ensure_workspace_registered(language, workspace_root.clone())
            .await?;

        Ok((workspace_root, language, config.command))
    }

    fn detect_workspace_language(&self, workspace_root: &Path) -> Result<Language> {
        // Look for common language markers in the workspace
        let markers = [
            ("go.mod", Language::Go),
            ("Cargo.toml", Language::Rust),
            ("package.json", Language::JavaScript),
            ("pyproject.toml", Language::Python),
            ("setup.py", Language::Python),
            ("pom.xml", Language::Java),
            ("build.gradle", Language::Java),
        ];

        for (marker, language) in &markers {
            if workspace_root.join(marker).exists() {
                return Ok(*language);
            }
        }

        Err(anyhow!(
            "Could not detect language for workspace: {:?}",
            workspace_root
        ))
    }

    async fn idle_checker(&self) {
        let idle_timeout = std::time::Duration::from_secs(86400); // 24 hours

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

            // Check if we should shutdown due to inactivity
            let now = Instant::now();
            let mut all_idle = true;

            for entry in self.connections.iter() {
                let last_activity = *entry.value();
                if now.duration_since(last_activity) < idle_timeout {
                    all_idle = false;
                    break;
                }
            }

            if all_idle && self.connections.is_empty() && self.start_time.elapsed() > idle_timeout {
                info!("Daemon idle for too long, shutting down");
                *self.shutdown.write().await = true;
                break;
            }
        }
    }

    async fn cleanup(&self) -> Result<()> {
        info!("Cleaning up daemon resources");

        // Shutdown all servers
        self.server_manager.shutdown_all().await;

        // Remove socket file (Unix only)
        remove_socket_file(&self.socket_path)?;

        Ok(())
    }

    fn clone_refs(&self) -> Self {
        Self {
            socket_path: self.socket_path.clone(),
            registry: self.registry.clone(),
            detector: self.detector.clone(),
            server_manager: self.server_manager.clone(),
            workspace_resolver: self.workspace_resolver.clone(),
            connections: self.connections.clone(),
            start_time: self.start_time,
            request_count: self.request_count.clone(),
            shutdown: self.shutdown.clone(),
            log_buffer: self.log_buffer.clone(),
            request_durations: self.request_durations.clone(),
            error_count: self.error_count.clone(),
        }
    }
}

fn find_daemon_binary() -> Result<PathBuf> {
    use crate::socket_path::normalize_executable;

    // Try to find lsp-daemon binary in various locations
    let daemon_name = normalize_executable("lsp-daemon");

    // 1. Check if it's in PATH
    if let Ok(path) = which::which(&daemon_name) {
        debug!("Found daemon in PATH: {:?}", path);
        return Ok(path);
    }

    // 2. Check in the same directory as current executable
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let daemon_path = parent.join(&daemon_name);
            if daemon_path.exists() {
                debug!("Found daemon in same directory: {:?}", daemon_path);
                return Ok(daemon_path);
            }
        }
    }

    // 3. Check target/debug directory (for development/testing)
    if let Ok(current_exe) = std::env::current_exe() {
        // Go up directories to find the workspace root and check target/debug
        let mut check_path = current_exe.parent();
        while let Some(path) = check_path {
            let target_debug = path.join("target").join("debug").join(&daemon_name);
            if target_debug.exists() {
                debug!("Found daemon in target/debug: {:?}", target_debug);
                return Ok(target_debug);
            }
            check_path = path.parent();
        }
    }

    // 4. Check common installation directories
    let common_paths = [
        "/usr/local/bin",
        "/usr/bin",
        "/opt/local/bin",
        "~/.cargo/bin",
    ];

    for path_str in &common_paths {
        let path = if path_str.starts_with("~") {
            if let Some(home) = dirs::home_dir() {
                home.join(&path_str[2..]).join(&daemon_name)
            } else {
                continue;
            }
        } else {
            PathBuf::from(path_str).join(&daemon_name)
        };

        if path.exists() {
            debug!("Found daemon in {}: {:?}", path_str, path);
            return Ok(path);
        }
    }

    Err(anyhow!(
        "Could not find lsp-daemon binary. Please ensure it's installed and in your PATH"
    ))
}

pub async fn start_daemon_background() -> Result<()> {
    let socket_path = get_default_socket_path();

    // Check if daemon is already running by trying to connect
    if (crate::ipc::IpcStream::connect(&socket_path).await).is_ok() {
        debug!("Daemon already running");
        return Ok(());
    }

    // Clean up any stale socket
    remove_socket_file(&socket_path)?;

    // Fork daemon process - try multiple locations
    let daemon_binary = find_daemon_binary()?;

    debug!("Starting daemon binary: {:?}", daemon_binary);

    std::process::Command::new(&daemon_binary)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn daemon: {}", e))?;

    info!("Started daemon in background");
    Ok(())
}
