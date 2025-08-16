use crate::ipc::{IpcListener, IpcStream};
use crate::language_detector::{Language, LanguageDetector};
use crate::logging::{LogBuffer, MemoryLogLayer};
use crate::lsp_registry::LspRegistry;
use crate::pid_lock::PidLock;
#[cfg(unix)]
use crate::process_group::ProcessGroup;
use crate::protocol::{
    parse_call_hierarchy_from_lsp, CallHierarchyResult, DaemonRequest, DaemonResponse,
    DaemonStatus, LanguageInfo, MessageCodec, PoolStatus,
};
use crate::server_manager::SingleServerManager;
use crate::socket_path::{get_default_socket_path, remove_socket_file};
use crate::watchdog::{ProcessMonitor, Watchdog};
use crate::workspace_resolver::WorkspaceResolver;
use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

// Timeout constants for client connection handling
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes
use tracing::{debug, error, info, warn};
use tracing_subscriber::prelude::*;
use uuid::Uuid;

// Keep the PID lock file distinct from the Unix socket path to avoid collisions with stale sockets
// or removing the lock when cleaning up the socket file.
#[inline]
fn pid_lock_path(socket_path: &str) -> String {
    format!("{socket_path}.lock")
}

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
    pid_lock: Option<PidLock>,
    #[cfg(unix)]
    process_group: ProcessGroup,
    child_processes: Arc<tokio::sync::Mutex<Vec<u32>>>, // Track all child PIDs
    // Performance metrics
    request_durations: Arc<RwLock<Vec<Duration>>>, // Keep last 100 request durations
    error_count: Arc<RwLock<usize>>,
    // Connection metrics
    total_connections_accepted: Arc<RwLock<usize>>,
    connections_cleaned_due_to_staleness: Arc<RwLock<usize>>,
    connections_rejected_due_to_limit: Arc<RwLock<usize>>,
    connection_durations: Arc<RwLock<Vec<Duration>>>, // Keep last 100 connection durations
    // Watchdog (disabled by default, enabled via --watchdog flag)
    watchdog: Arc<tokio::sync::Mutex<Option<Watchdog>>>,
    watchdog_enabled: Arc<AtomicBool>,
    watchdog_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    process_monitor: Arc<ProcessMonitor>,
    child_first_seen: Arc<DashMap<u32, Instant>>,
    index_grace_secs: u64,
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
        let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let server_manager = Arc::new(SingleServerManager::new_with_tracker(
            registry.clone(),
            child_processes.clone(),
        ));
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

        // Watchdog is disabled by default (can be enabled via --watchdog flag in lsp init)
        let process_monitor = Arc::new(ProcessMonitor::with_limits(80.0, 1024)); // 80% CPU, 1GB memory

        // Initialize indexing grace period from environment variable
        let index_grace_secs = std::env::var("LSP_INDEX_GRACE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30); // Default 30 seconds for language server indexing

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
            pid_lock: None,
            #[cfg(unix)]
            process_group: ProcessGroup::new(),
            child_processes,
            request_durations: Arc::new(RwLock::new(Vec::with_capacity(100))),
            error_count: Arc::new(RwLock::new(0)),
            total_connections_accepted: Arc::new(RwLock::new(0)),
            connections_cleaned_due_to_staleness: Arc::new(RwLock::new(0)),
            connections_rejected_due_to_limit: Arc::new(RwLock::new(0)),
            connection_durations: Arc::new(RwLock::new(Vec::with_capacity(100))),
            watchdog: Arc::new(tokio::sync::Mutex::new(None)),
            watchdog_enabled: Arc::new(AtomicBool::new(false)),
            watchdog_task: Arc::new(tokio::sync::Mutex::new(None)),
            process_monitor,
            child_first_seen: Arc::new(DashMap::new()),
            index_grace_secs,
        })
    }

    pub async fn run(mut self) -> Result<()> {
        // Acquire PID lock to ensure only one daemon runs
        // IMPORTANT: use a separate file from the Unix socket to avoid collisions with stale sockets.
        let lock_path = pid_lock_path(&self.socket_path);
        let mut pid_lock = PidLock::new(&lock_path);
        pid_lock
            .try_lock()
            .map_err(|e| anyhow!("Failed to acquire daemon lock: {}", e))?;
        self.pid_lock = Some(pid_lock);
        debug!(
            "Acquired daemon PID lock at {} (socket: {})",
            lock_path, self.socket_path
        );

        // Set up process group for child management
        #[cfg(unix)]
        self.process_group.become_leader()?;

        // Clean up any existing socket
        remove_socket_file(&self.socket_path)?;

        let listener = IpcListener::bind(&self.socket_path).await?;
        info!("LSP daemon listening on {}", self.socket_path);

        // Watchdog is started only when explicitly enabled via --watchdog flag
        // See enable_watchdog() method which is called from handle_init_workspaces

        // Set up signal handling for graceful shutdown
        #[cfg(unix)]
        {
            let daemon_for_signals = self.clone_refs();
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

        // Start periodic cleanup task
        let daemon_for_cleanup = self.clone_refs();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;

                // Check if daemon is shutting down
                if *daemon_for_cleanup.shutdown.read().await {
                    debug!("Periodic cleanup task stopping due to shutdown");
                    break;
                }

                let cleaned = daemon_for_cleanup.cleanup_stale_connections();
                if cleaned > 0 {
                    debug!("Periodic cleanup removed {} stale connections", cleaned);
                }
            }
        });

        // Start health monitoring
        let _health_monitor_task = self.server_manager.start_health_monitoring();
        info!("Started health monitoring for LSP servers");

        // Start process monitoring task with grace period for indexing
        let process_monitor = self.process_monitor.clone();
        let child_processes_for_monitoring = self.child_processes.clone();
        let child_first_seen = self.child_first_seen.clone();
        let index_grace_secs = self.index_grace_secs;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds
            loop {
                interval.tick().await;

                let pids = {
                    let pids_guard = child_processes_for_monitoring.lock().await;
                    pids_guard.clone()
                };

                if !pids.is_empty() {
                    debug!("Monitoring {} child processes", pids.len());
                    let now = Instant::now();

                    // Track first seen time for new processes
                    for &pid in &pids {
                        child_first_seen.entry(pid).or_insert(now);
                    }

                    // Only monitor processes that are past the grace period
                    let pids_to_monitor: Vec<u32> = pids
                        .into_iter()
                        .filter(|&pid| {
                            if let Some(first_seen) = child_first_seen.get(&pid) {
                                let age = now.duration_since(*first_seen);
                                if age < Duration::from_secs(index_grace_secs) {
                                    debug!(
                                        "Process {} is in grace period (age: {:?}, grace: {}s)",
                                        pid, age, index_grace_secs
                                    );
                                    false
                                } else {
                                    true
                                }
                            } else {
                                // Should not happen since we just inserted it, but be safe
                                true
                            }
                        })
                        .collect();

                    if !pids_to_monitor.is_empty() {
                        let unhealthy_pids =
                            process_monitor.monitor_children(pids_to_monitor).await;

                        if !unhealthy_pids.is_empty() {
                            warn!(
                                "Found {} unhealthy child processes (past grace period): {:?}",
                                unhealthy_pids.len(),
                                unhealthy_pids
                            );

                            // Kill unhealthy processes and remove from tracking
                            #[cfg(unix)]
                            for pid in &unhealthy_pids {
                                child_first_seen.remove(pid);
                                unsafe {
                                    if libc::kill(*pid as i32, libc::SIGTERM) == 0 {
                                        warn!("Sent SIGTERM to unhealthy process {}", pid);
                                    } else {
                                        warn!("Failed to send SIGTERM to process {}", pid);
                                    }
                                }
                            }
                        }
                    }

                    // Clean up tracking for processes that no longer exist
                    let current_pids: std::collections::HashSet<u32> = {
                        let guard = child_processes_for_monitoring.lock().await;
                        guard.iter().copied().collect()
                    };
                    child_first_seen.retain(|&pid, _| current_pids.contains(&pid));
                }
            }
        });

        loop {
            // Update watchdog heartbeat if enabled
            if self.watchdog_enabled.load(Ordering::Relaxed) {
                if let Some(ref watchdog) = *self.watchdog.lock().await {
                    watchdog.heartbeat();
                }
            }

            // Check shutdown flag
            if *self.shutdown.read().await {
                info!("Daemon shutting down...");
                break;
            }

            // Use select! to make accept interruptible by shutdown
            tokio::select! {
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok(stream) => {
                        // Check if we've reached the connection limit
                        const MAX_CONNECTIONS: usize = 100; // Reasonable limit for concurrent connections

                        let current_connections = self.connections.len();
                        if current_connections >= MAX_CONNECTIONS {
                            // Clean up stale connections first
                            let cleaned = self.cleanup_stale_connections();

                            // Check again after cleanup
                            let connections_after_cleanup = self.connections.len();
                            if connections_after_cleanup >= MAX_CONNECTIONS {
                                // Update rejection metrics
                                *self.connections_rejected_due_to_limit.write().await += 1;

                                warn!(
                                    "Maximum connection limit reached ({}/{}), cleaned {} stale connections, rejecting new connection",
                                    connections_after_cleanup, MAX_CONNECTIONS, cleaned
                                );
                                // Drop the stream to close the connection
                                drop(stream);
                                // Wait a bit to prevent tight loop
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                continue;
                            } else {
                                info!(
                                    "Cleaned {} stale connections, now have {}/{} connections, accepting new connection",
                                    cleaned, connections_after_cleanup, MAX_CONNECTIONS
                                );
                            }
                        }

                        // Track accepted connection
                        *self.total_connections_accepted.write().await += 1;

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
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                    // Periodic check for shutdown flag
                    if *self.shutdown.read().await {
                        info!("Daemon shutting down (periodic check)...");
                        break;
                    }
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

        let connection_start = Instant::now();

        loop {
            // Check for overall connection timeout
            if connection_start.elapsed() > CONNECTION_TIMEOUT {
                warn!(
                    "Connection timeout for client {} - closing after {}s",
                    client_id,
                    CONNECTION_TIMEOUT.as_secs()
                );
                break;
            }

            // Read message length (exactly 4 bytes) with timeout
            match timeout(READ_TIMEOUT, stream.read_exact(&mut buffer[..4])).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    debug!("Read error from client {}: {}", client_id, e);
                    break;
                }
                Err(_) => {
                    warn!(
                        "Read timeout from client {} - closing connection",
                        client_id
                    );
                    break;
                }
            }

            let msg_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

            // Validate message size to prevent OOM attacks
            if msg_len > MAX_MESSAGE_SIZE {
                error!(
                    "[{}] Attempted to send oversized message: {} bytes (max: {} bytes)",
                    client_id, msg_len, MAX_MESSAGE_SIZE
                );
                // Connection cleanup will happen in the defer-like cleanup at the end
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

            // Read message body with timeout
            match timeout(READ_TIMEOUT, stream.read_exact(&mut buffer[4..4 + msg_len])).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    error!(
                        "[{}] Failed to read message body from client: {}",
                        client_id, e
                    );
                    // Connection cleanup will happen at the end
                    return Err(e.into());
                }
                Err(_) => {
                    error!(
                        "[{}] Timeout reading message body (size: {} bytes)",
                        client_id, msg_len
                    );
                    // Connection cleanup will happen at the end
                    return Err(anyhow!(
                        "Read timeout after {} seconds",
                        READ_TIMEOUT.as_secs()
                    ));
                }
            }

            // Decode request
            let request = match MessageCodec::decode_request(&buffer[..4 + msg_len]) {
                Ok(req) => req,
                Err(e) => {
                    error!("[{}] Failed to decode request: {}", client_id, e);
                    // Connection cleanup will happen at the end
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

        // Calculate and log connection duration
        let connection_duration = connection_start.elapsed();

        // Track connection duration (keep only last 100)
        {
            let mut durations = self.connection_durations.write().await;
            durations.push(connection_duration);
            if durations.len() > 100 {
                durations.remove(0);
            }
        }

        // Remove connection
        self.connections.remove(&client_id);
        info!(
            "Client disconnected: {} (connected for {:?})",
            client_id, connection_duration
        );

        Ok(())
    }

    // Clean up connections that have been idle for too long
    fn cleanup_stale_connections(&self) -> usize {
        // Make MAX_IDLE_TIME configurable via environment variable
        let max_idle_secs = std::env::var("LSP_MAX_IDLE_TIME_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300); // Default to 5 minutes
        let max_idle_time = Duration::from_secs(max_idle_secs);
        let now = Instant::now();

        let connections_before = self.connections.len();
        let mut cleaned_connections = Vec::new();

        self.connections.retain(|client_id, last_activity| {
            let idle_time = now.duration_since(*last_activity);
            if idle_time > max_idle_time {
                cleaned_connections.push((*client_id, idle_time));
                false
            } else {
                true
            }
        });

        let cleaned_count = cleaned_connections.len();
        if cleaned_count > 0 {
            // Update metrics (use blocking_write since this is not an async function)
            if let Ok(mut count) = self.connections_cleaned_due_to_staleness.try_write() {
                *count += cleaned_count;
            }

            info!(
                "Cleaned up {} stale connections (had {} total connections)",
                cleaned_count, connections_before
            );
            for (client_id, idle_time) in cleaned_connections {
                debug!(
                    "Removed stale connection {}: idle for {:?}",
                    client_id, idle_time
                );
            }
        }

        cleaned_count
    }

    async fn handle_request(&self, request: DaemonRequest) -> DaemonResponse {
        debug!(
            "Received daemon request: {:?}",
            std::mem::discriminant(&request)
        );

        // Clean up stale connections on every request to prevent accumulation
        self.cleanup_stale_connections();

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

            DaemonRequest::InitWorkspaces {
                request_id,
                workspace_root,
                languages,
                recursive,
                enable_watchdog,
            } => {
                // Enable watchdog if requested and not already running
                if enable_watchdog && !self.watchdog_enabled.load(Ordering::Relaxed) {
                    self.enable_watchdog().await;
                }

                match self
                    .handle_init_workspaces(workspace_root, languages, recursive)
                    .await
                {
                    Ok((initialized, errors)) => DaemonResponse::WorkspacesInitialized {
                        request_id,
                        initialized,
                        errors,
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

                // Get LSP server health information
                let health_status = self
                    .server_manager
                    .health_monitor()
                    .get_health_status()
                    .await;
                let server_stats = self.server_manager.get_stats().await;

                let lsp_server_health: Vec<crate::protocol::LspServerHealthInfo> = server_stats
                    .into_iter()
                    .map(|s| {
                        let server_key = format!("{:?}", s.language);
                        let health = health_status.get(&server_key);

                        crate::protocol::LspServerHealthInfo {
                            language: s.language,
                            is_healthy: health.map(|h| h.is_healthy).unwrap_or(true),
                            consecutive_failures: health
                                .map(|h| h.consecutive_failures)
                                .unwrap_or(0),
                            circuit_breaker_open: health
                                .map(|h| h.is_circuit_breaker_open())
                                .unwrap_or(false),
                            last_check_ms: health
                                .map(|h| h.last_check.elapsed().as_millis() as u64)
                                .unwrap_or(0),
                            response_time_ms: health.map(|h| h.response_time_ms).unwrap_or(0),
                        }
                    })
                    .collect();

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

                // Get connection metrics
                let total_accepted = *self.total_connections_accepted.read().await;
                let total_cleaned = *self.connections_cleaned_due_to_staleness.read().await;
                let total_rejected = *self.connections_rejected_due_to_limit.read().await;

                // Estimate memory usage (simplified - in production you'd use a proper memory profiler)
                let memory_usage_mb = {
                    // This is a rough estimate - consider using a proper memory profiler
                    let rusage = std::mem::size_of_val(self) as f64 / 1_048_576.0;
                    rusage + (active_servers as f64 * 50.0) // Estimate 50MB per LSP server
                };

                // Health is considered good if:
                // - Not at connection limit (with some buffer)
                // - Reasonable memory usage
                // - Low error rate
                // - Reasonable response times
                // - Not rejecting too many connections
                let connection_rejection_rate = if total_accepted > 0 {
                    (total_rejected as f64 / total_accepted as f64) * 100.0
                } else {
                    0.0
                };

                let healthy = active_connections < 90
                    && memory_usage_mb < 1024.0
                    && error_rate < 5.0
                    && avg_request_duration_ms < 5000.0
                    && connection_rejection_rate < 10.0; // Less than 10% rejection rate

                // Calculate average connection duration
                let avg_connection_duration_ms = {
                    let durations = self.connection_durations.read().await;
                    if durations.is_empty() {
                        0.0
                    } else {
                        let total: Duration = durations.iter().sum();
                        total.as_millis() as f64 / durations.len() as f64
                    }
                };

                info!(
                    "Health check: connections={} (accepted={}, cleaned={}, rejected={}), memory={}MB, errors={}%, avg_req_duration={}ms, avg_conn_duration={}ms",
                    active_connections, total_accepted, total_cleaned, total_rejected, memory_usage_mb, error_rate, avg_request_duration_ms, avg_connection_duration_ms
                );

                DaemonResponse::HealthCheck {
                    request_id,
                    healthy,
                    uptime_seconds,
                    total_requests,
                    active_connections,
                    active_servers,
                    memory_usage_mb,
                    lsp_server_health,
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
                let health_status = self
                    .server_manager
                    .health_monitor()
                    .get_health_status()
                    .await;

                let pool_status: Vec<PoolStatus> = server_stats
                    .into_iter()
                    .map(|s| {
                        let server_key = format!("{:?}", s.language);
                        let health = health_status.get(&server_key);

                        // Consider a server "ready" if either:
                        // 1) we know it's initialized, or
                        // 2) the health monitor reports it as healthy (the stats lock may be busy
                        //    while the server is in active use, which used to surface as initialized=false).
                        let is_ready = if s.initialized {
                            true
                        } else if let Some(h) = health {
                            h.is_healthy && !h.is_circuit_breaker_open()
                        } else {
                            false
                        };

                        PoolStatus {
                            language: s.language,
                            ready_servers: if is_ready { 1 } else { 0 },
                            busy_servers: 0, // No busy concept in single server model
                            total_servers: 1,
                            workspaces: s
                                .workspaces
                                .iter()
                                .map(|w| {
                                    w.canonicalize()
                                        .unwrap_or_else(|_| w.clone())
                                        .to_string_lossy()
                                        .to_string()
                                })
                                .collect(),
                            uptime_secs: s.uptime.as_secs(),
                            status: format!("{:?}", s.status),
                            health_status: if let Some(h) = health {
                                if h.is_healthy {
                                    "healthy".to_string()
                                } else {
                                    "unhealthy".to_string()
                                }
                            } else {
                                "unknown".to_string()
                            },
                            consecutive_failures: health
                                .map(|h| h.consecutive_failures)
                                .unwrap_or(0),
                            circuit_breaker_open: health
                                .map(|h| h.is_circuit_breaker_open())
                                .unwrap_or(false),
                        }
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

        // Read file content - use the absolute path to ensure consistency
        let content = fs::read_to_string(&absolute_file_path)?;

        // Adaptive timing for Go/TypeScript in CI environments
        let is_ci = std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok();
        let (initial_wait, max_attempts, retry_delay) = match language {
            Language::Go | Language::TypeScript | Language::JavaScript if is_ci => {
                // Go and TypeScript need more time in CI for indexing
                (15, 5, 5) // 15s initial wait, 5 attempts, 5s between attempts
            }
            Language::Go | Language::TypeScript | Language::JavaScript => {
                // Local development - reduced wait times for better performance
                (3, 3, 2) // 3s initial wait, 3 attempts, 2s between attempts
            }
            _ => {
                // Rust and other languages - works well with shorter waits
                (1, 3, 1) // 1s initial wait, 3 attempts, 1s between attempts
            }
        };

        debug!(
            "Using adaptive timing for {:?}: initial_wait={}s, max_attempts={}, retry_delay={}s (CI={})",
            language, initial_wait, max_attempts, retry_delay, is_ci
        );

        // Open document and give language server time to process
        {
            // Lock the server instance only for document opening
            let server = server_instance.lock().await;
            server
                .server
                .open_document(&absolute_file_path, &content)
                .await?;
            // Lock is automatically released here when server goes out of scope
        }

        // Give language server time to process and index OUTSIDE the lock
        // This allows other requests to proceed while we wait
        tokio::time::sleep(tokio::time::Duration::from_secs(initial_wait)).await;

        // Try call hierarchy with adaptive retry logic
        let mut attempt = 1;
        let mut result = None;

        while attempt <= max_attempts {
            debug!("Call hierarchy attempt {} at {}:{}", attempt, line, column);

            // Lock the server instance only for the call hierarchy request
            let call_result = {
                let server = server_instance.lock().await;
                server
                    .server
                    .call_hierarchy(&absolute_file_path, line, column)
                    .await
            };

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
                // Adaptive retry delay
                tokio::time::sleep(tokio::time::Duration::from_secs(retry_delay)).await;
            }
        }

        let result = result.ok_or_else(|| {
            anyhow!(
                "Failed to get call hierarchy response after {} attempts",
                max_attempts
            )
        })?;

        // Close document - lock server instance briefly
        {
            let server = server_instance.lock().await;
            server.server.close_document(&absolute_file_path).await?;
        }

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

        // Canonicalize the workspace root to ensure it's an absolute path
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.clone());

        // Check if workspace is allowed
        {
            let resolver = self.workspace_resolver.lock().await;
            if !resolver.is_path_allowed(&canonical_root) {
                return Err(anyhow!(
                    "Workspace {:?} not in allowed roots",
                    canonical_root
                ));
            }
        }

        // Determine language - use hint if provided, otherwise detect from workspace
        let language = if let Some(lang) = language_hint {
            lang
        } else {
            // Try to detect language from common files in workspace
            self.detect_workspace_language(&canonical_root)?
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
            .ensure_workspace_registered(language, canonical_root.clone())
            .await?;

        Ok((canonical_root, language, config.command))
    }

    async fn enable_watchdog(&self) {
        if self.watchdog_enabled.load(Ordering::Relaxed) {
            info!("Watchdog already enabled");
            return;
        }

        info!("Enabling watchdog monitoring");

        // Create and start the watchdog
        let watchdog = Watchdog::new(60);
        let shutdown_for_watchdog = self.shutdown.clone();

        // Set recovery callback
        watchdog
            .set_recovery_callback(move || {
                // Set shutdown flag when watchdog detects unresponsive daemon
                if let Ok(mut shutdown) = shutdown_for_watchdog.try_write() {
                    *shutdown = true;
                    error!("Watchdog triggered daemon shutdown due to unresponsiveness");
                }
            })
            .await;

        // Start watchdog monitoring
        let watchdog_task = watchdog.start();

        // Store the watchdog in the struct
        let mut watchdog_guard = self.watchdog.lock().await;
        *watchdog_guard = Some(watchdog);

        // Mark as enabled
        self.watchdog_enabled.store(true, Ordering::Relaxed);

        // Store the task handle
        let mut task_guard = self.watchdog_task.lock().await;
        *task_guard = Some(watchdog_task);

        info!("Watchdog monitoring enabled");
    }

    async fn handle_init_workspaces(
        &self,
        workspace_root: PathBuf,
        languages: Option<Vec<Language>>,
        recursive: bool,
    ) -> Result<(Vec<crate::protocol::InitializedWorkspace>, Vec<String>)> {
        use crate::protocol::InitializedWorkspace;

        // Validate workspace root exists
        if !workspace_root.exists() {
            return Err(anyhow!(
                "Workspace root does not exist: {:?}",
                workspace_root
            ));
        }

        // Canonicalize the workspace root to ensure it's an absolute path
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.clone());

        // Discover workspaces
        let detector = crate::language_detector::LanguageDetector::new();
        let discovered_workspaces = detector.discover_workspaces(&canonical_root, recursive)?;

        if discovered_workspaces.is_empty() {
            return Ok((vec![], vec!["No workspaces found".to_string()]));
        }

        let mut initialized = Vec::new();
        let mut errors = Vec::new();

        // Filter by requested languages if specified
        for (workspace_path, detected_languages) in discovered_workspaces {
            // Canonicalize each workspace path to ensure it's absolute
            let canonical_workspace = workspace_path
                .canonicalize()
                .unwrap_or_else(|_| workspace_path.clone());

            let languages_to_init = if let Some(ref requested_languages) = languages {
                // Only initialize requested languages that were detected
                detected_languages
                    .intersection(&requested_languages.iter().copied().collect())
                    .copied()
                    .collect::<Vec<_>>()
            } else {
                // Initialize all detected languages
                detected_languages.into_iter().collect()
            };

            for language in languages_to_init {
                // Skip unknown language
                if language == Language::Unknown {
                    continue;
                }

                // Get LSP server config
                let config = match self.registry.get(language) {
                    Some(cfg) => cfg,
                    None => {
                        errors.push(format!(
                            "No LSP server configured for {language:?} in {canonical_workspace:?}"
                        ));
                        continue;
                    }
                };

                // Try to initialize the workspace
                match self
                    .server_manager
                    .ensure_workspace_registered(language, canonical_workspace.clone())
                    .await
                {
                    Ok(_) => {
                        // Ensure the workspace path is absolute before returning
                        let absolute_workspace = if canonical_workspace.is_absolute() {
                            canonical_workspace.clone()
                        } else {
                            std::env::current_dir()
                                .unwrap_or_else(|_| PathBuf::from("/"))
                                .join(&canonical_workspace)
                                .canonicalize()
                                .unwrap_or_else(|_| canonical_workspace.clone())
                        };

                        initialized.push(InitializedWorkspace {
                            workspace_root: absolute_workspace,
                            language,
                            lsp_server: config.command.clone(),
                            status: "Ready".to_string(),
                        });
                        info!(
                            "Initialized {:?} for workspace {:?}",
                            language, canonical_workspace
                        );
                    }
                    Err(e) => {
                        errors.push(format!(
                            "Failed to initialize {language:?} for {canonical_workspace:?}: {e}"
                        ));
                    }
                }
            }
        }

        Ok((initialized, errors))
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

    async fn cleanup(&mut self) -> Result<()> {
        info!("Cleaning up daemon resources");

        // Stop the watchdog if it was enabled
        if self.watchdog_enabled.load(Ordering::Relaxed) {
            info!("Stopping watchdog");
            if let Some(ref watchdog) = *self.watchdog.lock().await {
                watchdog.stop();
            }
        }

        // Shutdown all servers gracefully first, but don't block forever
        match tokio::time::timeout(Duration::from_secs(5), self.server_manager.shutdown_all()).await
        {
            Ok(_) => {
                debug!("Language servers shut down cleanly");
            }
            Err(_) => {
                warn!(
                    "Timed out waiting for language servers to shutdown; proceeding with forced cleanup"
                );
            }
        }

        // Small grace period
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Kill any remaining child processes directly
        let child_pids = self.child_processes.lock().await;
        #[cfg(unix)]
        for &pid in child_pids.iter() {
            unsafe {
                let _ = libc::kill(pid as i32, libc::SIGTERM);
                debug!("Sent SIGTERM to child process {}", pid);
            }
        }
        #[cfg(not(unix))]
        for &_pid in child_pids.iter() {
            // Windows: process cleanup handled differently
        }
        drop(child_pids);

        // Give processes time to terminate
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Force kill any remaining processes in our process group
        #[cfg(unix)]
        self.process_group.kill_all();

        // Release PID lock
        if let Some(mut lock) = self.pid_lock.take() {
            lock.unlock()?;
        }

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
            pid_lock: None, // Don't clone the PID lock
            #[cfg(unix)]
            process_group: ProcessGroup::new(), // Create new for cloned instance
            child_processes: self.child_processes.clone(), // Share child process tracking
            request_durations: self.request_durations.clone(),
            error_count: self.error_count.clone(),
            total_connections_accepted: self.total_connections_accepted.clone(),
            connections_cleaned_due_to_staleness: self.connections_cleaned_due_to_staleness.clone(),
            connections_rejected_due_to_limit: self.connections_rejected_due_to_limit.clone(),
            connection_durations: self.connection_durations.clone(),
            watchdog: self.watchdog.clone(),
            watchdog_enabled: self.watchdog_enabled.clone(),
            watchdog_task: self.watchdog_task.clone(),
            process_monitor: self.process_monitor.clone(),
            child_first_seen: self.child_first_seen.clone(),
            index_grace_secs: self.index_grace_secs,
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
    // Allow tests or callers to override the socket explicitly
    let socket_path =
        std::env::var("PROBE_LSP_SOCKET_PATH").unwrap_or_else(|_| get_default_socket_path());

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
