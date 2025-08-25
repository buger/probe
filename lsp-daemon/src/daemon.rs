use crate::cache_management::CacheManager;
use crate::cache_types::{
    CallHierarchyInfo, CallInfo, DefinitionInfo, HoverInfo, LspOperation, NodeId, NodeKey,
    ReferencesInfo,
};
use crate::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
use crate::hash_utils::md5_hex_file;
use crate::indexing::{IndexingConfig, IndexingManager};
use crate::ipc::{IpcListener, IpcStream};
use crate::language_detector::{Language, LanguageDetector};
use crate::logging::{LogBuffer, MemoryLogLayer};
use crate::lsp_cache::{LspCache, LspCacheConfig};
use crate::lsp_registry::LspRegistry;
use crate::path_safety::safe_canonicalize;
use crate::persistent_cache::{PersistentCacheConfig, PersistentCallGraphCache};
use crate::pid_lock::PidLock;
#[cfg(unix)]
use crate::process_group::ProcessGroup;
use crate::protocol::{
    parse_call_hierarchy_from_lsp, CallHierarchyItem, CallHierarchyResult, ClearFilter,
    CompactOptions, DaemonRequest, DaemonResponse, DaemonStatus, DocumentSymbol, ExportOptions,
    HoverContent, LanguageInfo, Location, MessageCodec, PoolStatus, SymbolInformation,
};
use crate::server_manager::SingleServerManager;
use crate::socket_path::{get_default_socket_path, remove_socket_file};
use crate::watchdog::{ProcessMonitor, Watchdog};
use crate::workspace_cache_router::WorkspaceCacheRouter;
use crate::workspace_resolver::WorkspaceResolver;
use anyhow::Context;
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{RwLock, Semaphore};
use tokio::time::{timeout, Duration};

// Connection management constants
const MAX_CONCURRENT_CONNECTIONS: u32 = 64;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const REQ_TIMEOUT: Duration = Duration::from_secs(25);
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
    connection_semaphore: Arc<Semaphore>, // Limit concurrent connections
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
    background_tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    watchdog_enabled: Arc<AtomicBool>,
    watchdog_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    process_monitor: Arc<ProcessMonitor>,
    child_first_seen: Arc<DashMap<u32, Instant>>,
    index_grace_secs: u64,
    // Call graph cache for LSP hierarchy results
    call_graph_cache: Arc<CallGraphCache>,
    // Persistent call graph cache
    persistent_store: Arc<PersistentCallGraphCache>,
    // Cache manager for unified cache operations
    cache_manager: Arc<CacheManager>,
    // Workspace-aware cache router for multi-workspace environments
    workspace_cache_router: Arc<WorkspaceCacheRouter>,
    // Individual LSP caches for each operation type
    definition_cache: Arc<LspCache<DefinitionInfo>>,
    references_cache: Arc<LspCache<ReferencesInfo>>,
    hover_cache: Arc<LspCache<HoverInfo>>,
    // Indexing configuration and manager
    indexing_config: Arc<RwLock<IndexingConfig>>,
    indexing_manager: Arc<tokio::sync::Mutex<Option<IndexingManager>>>,
}

impl LspDaemon {
    pub fn new(socket_path: String) -> Result<Self> {
        Self::new_with_config(socket_path, None)
    }

    /// Create a new LSP daemon with async initialization for persistence
    pub async fn new_async(socket_path: String) -> Result<Self> {
        Self::new_with_config_async(socket_path, None).await
    }

    pub fn new_with_config(
        socket_path: String,
        allowed_roots: Option<Vec<PathBuf>>,
    ) -> Result<Self> {
        // Use the runtime to call the async version with persistence disabled
        let runtime = tokio::runtime::Handle::current();
        runtime.block_on(async {
            let config = CallGraphCacheConfig {
                persistence_enabled: false, // Explicitly disable for sync constructor
                ..Default::default()
            };
            Self::new_with_config_and_cache_async(socket_path, allowed_roots, config).await
        })
    }

    /// Create a new LSP daemon with async initialization and custom cache config
    pub async fn new_with_config_async(
        socket_path: String,
        allowed_roots: Option<Vec<PathBuf>>,
    ) -> Result<Self> {
        // Create default cache config with persistence and git settings from environment
        let cache_config = CallGraphCacheConfig {
            capacity: 1000,                                   // Cache up to 1000 nodes
            ttl: Duration::from_secs(1800),                   // 30 minutes TTL
            eviction_check_interval: Duration::from_secs(60), // Check every minute
            invalidation_depth: 2, // Invalidate connected nodes up to depth 2
            // Persistence settings (can be overridden by environment variables)
            // IMPORTANT: Always disable persistence in CI to prevent hanging
            persistence_enabled: if std::env::var("PROBE_CI").is_ok()
                || std::env::var("GITHUB_ACTIONS").is_ok()
            {
                false // Force disable in CI
            } else {
                std::env::var("PROBE_LSP_PERSISTENCE_ENABLED")
                    .map(|v| v.to_lowercase() == "true")
                    .unwrap_or(false)
            },
            persistence_path: std::env::var("PROBE_LSP_PERSISTENCE_PATH")
                .ok()
                .map(PathBuf::from),
            persistence_write_batch_size: std::env::var("PROBE_LSP_PERSISTENCE_BATCH_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            persistence_write_interval: Duration::from_millis(
                std::env::var("PROBE_LSP_PERSISTENCE_INTERVAL_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5000),
            ),
        };

        Self::new_with_config_and_cache_async(socket_path, allowed_roots, cache_config).await
    }

    async fn new_with_config_and_cache_async(
        socket_path: String,
        allowed_roots: Option<Vec<PathBuf>>,
        cache_config: CallGraphCacheConfig,
    ) -> Result<Self> {
        // Log CI environment detection and persistence status
        if std::env::var("PROBE_CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
            info!("CI environment detected - persistence disabled to prevent hanging");
        }
        info!(
            "LSP daemon starting with persistence_enabled: {}",
            cache_config.persistence_enabled
        );

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

        // If PROBE_LOG_LEVEL is set to debug or trace, also add stderr logging
        let log_level = std::env::var("PROBE_LOG_LEVEL").unwrap_or_default();
        if log_level == "debug" || log_level == "trace" {
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
        let index_grace_secs = std::env::var("PROBE_LSP_INDEX_GRACE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30); // Default 30 seconds for language server indexing

        // Initialize persistent cache store configuration first
        let persistent_cache_config = PersistentCacheConfig {
            cache_directory: cache_config.persistence_path.clone(),
            max_size_bytes: 1_000_000_000, // 1GB
            ttl_days: 30,
            compress: true,
        };

        let persistent_store = Arc::new(
            PersistentCallGraphCache::new(persistent_cache_config.clone())
                .await
                .context("Failed to initialize persistent cache store")?,
        );

        // Initialize workspace cache router before call graph cache
        let workspace_cache_router_config =
            crate::workspace_cache_router::WorkspaceCacheRouterConfig {
                max_open_caches: std::env::var("PROBE_MAX_WORKSPACE_CACHES")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(8),
                max_parent_lookup_depth: 3,
                cache_config_template: persistent_cache_config.clone(),
                ..Default::default()
            };

        let workspace_cache_router = Arc::new(WorkspaceCacheRouter::new(
            workspace_cache_router_config,
            server_manager.clone(),
        ));

        // Initialize call graph cache with workspace router support
        let call_graph_cache = Arc::new(
            CallGraphCache::new_with_workspace_router(
                cache_config.clone(),
                workspace_cache_router.clone(),
            )
            .await
            .context("Failed to initialize call graph cache with workspace router")?,
        );

        // Warm cache from persistence if enabled
        match call_graph_cache.warm_from_persistence().await {
            Ok(loaded) => {
                if loaded > 0 {
                    info!(
                        "Warmed cache with {} entries from persistent storage",
                        loaded
                    );
                }
            }
            Err(e) => {
                warn!("Failed to warm cache from persistence: {}", e);
            }
        }

        // Initialize individual LSP caches for each operation type
        let lsp_cache_config = LspCacheConfig {
            capacity_per_operation: 500,    // 500 entries per operation
            ttl: Duration::from_secs(1800), // 30 minutes TTL
            eviction_check_interval: Duration::from_secs(60), // Check every minute
            persistent: false,              // Disabled by default
            cache_directory: None,
        };

        let definition_cache = Arc::new(
            LspCache::new(LspOperation::Definition, lsp_cache_config.clone())
                .expect("Failed to create definition cache"),
        );
        let references_cache = Arc::new(
            LspCache::new(LspOperation::References, lsp_cache_config.clone())
                .expect("Failed to create references cache"),
        );
        let hover_cache = Arc::new(
            LspCache::new(LspOperation::Hover, lsp_cache_config)
                .expect("Failed to create hover cache"),
        );

        // Initialize cache manager with all caches
        let cache_manager = Arc::new(CacheManager::new(
            call_graph_cache.clone(),
            persistent_store.clone(),
            definition_cache.clone(),
            references_cache.clone(),
            hover_cache.clone(),
        ));

        // Load indexing configuration with updated defaults
        let mut indexing_config = IndexingConfig::load().unwrap_or_else(|e| {
            warn!(
                "Failed to load indexing configuration: {}. Using defaults.",
                e
            );
            IndexingConfig::default()
        });

        // Override from environment if set - these take priority
        if let Ok(val) = std::env::var("PROBE_INDEXING_ENABLED") {
            indexing_config.enabled = val == "true" || val == "1";
        }
        if let Ok(val) = std::env::var("PROBE_INDEXING_AUTO_INDEX") {
            indexing_config.auto_index = val == "true" || val == "1";
        }
        if let Ok(val) = std::env::var("PROBE_INDEXING_WATCH_FILES") {
            indexing_config.watch_files = val == "true" || val == "1";
        }

        info!(
            "Loaded indexing configuration (enabled={}, auto_index={}, watch_files={})",
            indexing_config.enabled, indexing_config.auto_index, indexing_config.watch_files
        );

        let indexing_config = Arc::new(RwLock::new(indexing_config));

        Ok(Self {
            socket_path,
            registry,
            detector,
            server_manager,
            workspace_resolver,
            connections: Arc::new(DashMap::new()),
            connection_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS as usize)),
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
            background_tasks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            watchdog_enabled: Arc::new(AtomicBool::new(false)),
            watchdog_task: Arc::new(tokio::sync::Mutex::new(None)),
            process_monitor,
            child_first_seen: Arc::new(DashMap::new()),
            index_grace_secs,
            call_graph_cache,
            persistent_store,
            cache_manager,
            workspace_cache_router,
            definition_cache,
            references_cache,
            hover_cache,
            indexing_config,
            indexing_manager: Arc::new(tokio::sync::Mutex::new(None)),
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
        let idle_handle = tokio::spawn(async move {
            daemon.idle_checker().await;
        });
        self.background_tasks.lock().await.push(idle_handle);

        // Start periodic cleanup task
        let daemon_for_cleanup = self.clone_refs();
        let cleanup_shutdown = self.shutdown.clone();
        let cleanup_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;

                // Check if daemon is shutting down
                if *cleanup_shutdown.read().await {
                    debug!("Periodic cleanup task stopping due to shutdown");
                    break;
                }

                let cleaned = daemon_for_cleanup.cleanup_stale_connections();
                if cleaned > 0 {
                    debug!("Periodic cleanup removed {} stale connections", cleaned);
                }
            }
        });
        self.background_tasks.lock().await.push(cleanup_handle);

        // Start health monitoring
        let health_monitor_handle = self.server_manager.start_health_monitoring();
        info!("Started health monitoring for LSP servers");
        self.background_tasks
            .lock()
            .await
            .push(health_monitor_handle);

        // Start process monitoring task with grace period for indexing
        let process_monitor = self.process_monitor.clone();
        let child_processes_for_monitoring = self.child_processes.clone();
        let child_first_seen = self.child_first_seen.clone();
        let index_grace_secs = self.index_grace_secs;
        let shutdown_flag = self.shutdown.clone();
        let monitor_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds
            loop {
                interval.tick().await;
                if *shutdown_flag.read().await {
                    debug!("Process monitoring task stopping due to shutdown");
                    break;
                }

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
                                // Also drop from the tracked pid list so we don't keep monitoring it.
                                {
                                    let mut guard = child_processes_for_monitoring.lock().await;
                                    guard.retain(|p| p != pid);
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
        self.background_tasks.lock().await.push(monitor_handle);

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
                                // Acquire semaphore permit before spawning handler
                                let semaphore = self.connection_semaphore.clone();
                                match semaphore.try_acquire_owned() {
                                    Ok(permit) => {
                                        // Track accepted connection
                                        *self.total_connections_accepted.write().await += 1;

                                        let daemon = self.clone_refs();
                                        tokio::spawn(async move {
                                            // Hold permit for duration of connection
                                            let _permit = permit;
                                            if let Err(e) = daemon.handle_connection(stream).await {
                                                error!("Error handling connection: {}", e);
                                            }
                                        });
                                    }
                                    Err(_) => {
                                        // No permits available - reject connection
                                        *self.connections_rejected_due_to_limit.write().await += 1;
                                        warn!(
                                            "Connection limit reached ({} connections), rejecting new connection",
                                            MAX_CONCURRENT_CONNECTIONS
                                        );
                                        drop(stream); // Close connection immediately
                                    }
                                }
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

    async fn handle_connection(&self, stream: IpcStream) -> Result<()> {
        let client_id = Uuid::new_v4();
        info!("New client connected: {}", client_id);

        let connection_start = Instant::now();
        let mut last_activity = Instant::now();

        // Store connection timestamp
        self.connections.insert(client_id, last_activity);

        // Split stream for concurrent read/write operations
        let (mut reader, mut writer) = stream.into_split();

        loop {
            // Check for idle timeout
            if last_activity.elapsed() > IDLE_TIMEOUT {
                warn!(
                    "Connection idle timeout for client {} - closing after {}s",
                    client_id,
                    IDLE_TIMEOUT.as_secs()
                );
                break;
            }

            // Check for overall connection timeout
            if connection_start.elapsed() > CONNECTION_TIMEOUT {
                warn!(
                    "Connection timeout for client {} - closing after {}s",
                    client_id,
                    CONNECTION_TIMEOUT.as_secs()
                );
                break;
            }

            // Check if shutdown was requested
            if *self.shutdown.read().await {
                info!(
                    "Daemon shutting down, closing client connection {}",
                    client_id
                );
                break;
            }

            // Read framed message with timeout
            let message_data = match MessageCodec::read_framed(&mut reader, READ_TIMEOUT).await {
                Ok(data) => data,
                Err(e) => {
                    let error_msg = e.to_string();
                    if error_msg.contains("Timeout") {
                        debug!("Read timeout from client {} - continuing", client_id);
                        continue; // Continue loop on timeout, don't close connection
                    } else if error_msg.contains("early eof") || error_msg.contains("UnexpectedEof")
                    {
                        // Client disconnected gracefully - this is normal
                        debug!("[{}] Client disconnected (early eof)", client_id);
                        break;
                    } else if error_msg.contains("Connection reset")
                        || error_msg.contains("Broken pipe")
                    {
                        // Client disconnected abruptly - also normal
                        debug!(
                            "[{}] Client disconnected abruptly: {}",
                            client_id, error_msg
                        );
                        break;
                    } else {
                        // Actual protocol or I/O error
                        error!("[{}] Failed to read message: {}", client_id, e);
                        break; // Close connection on actual errors
                    }
                }
            };

            // Decode request
            let request = match serde_json::from_slice::<DaemonRequest>(&message_data) {
                Ok(req) => req,
                Err(e) => {
                    error!("[{}] Failed to decode request: {}", client_id, e);
                    // Send error response for malformed requests
                    let error_response = DaemonResponse::Error {
                        request_id: Uuid::new_v4(),
                        error: format!("Malformed request: {e}"),
                    };

                    if let Err(write_err) = self.send_response(&mut writer, &error_response).await {
                        error!(
                            "[{}] Failed to send error response: {}",
                            client_id, write_err
                        );
                        break;
                    }
                    continue;
                }
            };

            // Update activity timestamp
            last_activity = Instant::now();
            self.connections.insert(client_id, last_activity);

            // Increment request count
            *self.request_count.write().await += 1;

            // Handle request with timeout
            let request_start = Instant::now();
            let response_result = timeout(REQ_TIMEOUT, self.handle_request(request)).await;
            let request_duration = request_start.elapsed();

            let response = match response_result {
                Ok(resp) => resp,
                Err(_) => {
                    warn!(
                        "[{}] Request processing timed out after {}s",
                        client_id,
                        REQ_TIMEOUT.as_secs()
                    );
                    DaemonResponse::Error {
                        request_id: Uuid::new_v4(),
                        error: format!("Request timed out after {}s", REQ_TIMEOUT.as_secs()),
                    }
                }
            };

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

            // Send response with timeout
            if let Err(e) = self.send_response(&mut writer, &response).await {
                error!("[{}] Failed to send response: {}", client_id, e);
                break; // Close connection on write errors
            }

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

    /// Helper method to send response with timeout
    async fn send_response(
        &self,
        writer: &mut crate::ipc::OwnedWriteHalf,
        response: &DaemonResponse,
    ) -> Result<()> {
        let json_data = serde_json::to_vec(response)?;
        MessageCodec::write_framed(writer, &json_data, WRITE_TIMEOUT).await
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
            } => {
                info!(
                    "Received DaemonRequest::CallHierarchy for {:?} at {}:{} (request_id: {})",
                    file_path, line, column, request_id
                );
                match self
                    .handle_call_hierarchy(&file_path, line, column, workspace_hint)
                    .await
                {
                    Ok(result) => DaemonResponse::CallHierarchy { request_id, result },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

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
                                .map(|w| safe_canonicalize(w).to_string_lossy().to_string())
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

            DaemonRequest::GetLogs {
                request_id,
                lines,
                since_sequence,
            } => {
                let entries = if let Some(since) = since_sequence {
                    // Get logs since sequence
                    self.log_buffer.get_since_sequence(since, lines)
                } else {
                    // Backward compatibility: get last N logs
                    self.log_buffer.get_last(lines)
                };

                DaemonResponse::Logs {
                    request_id,
                    entries,
                }
            }

            DaemonRequest::CacheStats {
                request_id,
                detailed,
                git,
            } => match self.cache_manager.get_stats(detailed, git).await {
                Ok(stats) => DaemonResponse::CacheStats { request_id, stats },
                Err(e) => DaemonResponse::Error {
                    request_id,
                    error: e.to_string(),
                },
            },

            DaemonRequest::CacheClear {
                request_id,
                older_than_days,
                file_path,
                commit_hash,
                all,
            } => {
                let filter = ClearFilter {
                    older_than_days,
                    file_path,
                    commit_hash,
                    all,
                };
                match self.cache_manager.clear(filter).await {
                    Ok(result) => DaemonResponse::CacheCleared { request_id, result },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::CacheExport {
                request_id,
                output_path,
                current_branch_only,
                compress,
            } => {
                let options = ExportOptions {
                    current_branch_only,
                    compress,
                };
                match self.cache_manager.export(&output_path, options).await {
                    Ok((entries_exported, compressed)) => DaemonResponse::CacheExported {
                        request_id,
                        output_path,
                        entries_exported,
                        compressed,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::CacheImport {
                request_id,
                input_path,
                merge,
            } => match self.cache_manager.import(&input_path, merge).await {
                Ok(result) => DaemonResponse::CacheImported { request_id, result },
                Err(e) => DaemonResponse::Error {
                    request_id,
                    error: e.to_string(),
                },
            },

            DaemonRequest::CacheCompact {
                request_id,
                clean_expired,
                target_size_mb,
            } => {
                let options = CompactOptions {
                    clean_expired,
                    target_size_mb,
                };
                match self.cache_manager.compact(options).await {
                    Ok(result) => DaemonResponse::CacheCompacted { request_id, result },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            // Indexing management requests
            DaemonRequest::StartIndexing {
                request_id,
                workspace_root,
                config,
            } => match self
                .handle_start_indexing(workspace_root.clone(), config)
                .await
            {
                Ok(session_id) => DaemonResponse::IndexingStarted {
                    request_id,
                    workspace_root,
                    session_id,
                },
                Err(e) => DaemonResponse::Error {
                    request_id,
                    error: e.to_string(),
                },
            },

            DaemonRequest::StopIndexing { request_id, force } => {
                match self.handle_stop_indexing(force).await {
                    Ok(was_running) => DaemonResponse::IndexingStopped {
                        request_id,
                        was_running,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::IndexingStatus { request_id } => {
                match self.handle_indexing_status().await {
                    Ok(status) => DaemonResponse::IndexingStatusResponse { request_id, status },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::IndexingConfig { request_id } => {
                let config = self.indexing_config.read().await;
                let protocol_config = self.convert_internal_to_protocol_config(&config);
                DaemonResponse::IndexingConfigResponse {
                    request_id,
                    config: protocol_config,
                }
            }

            DaemonRequest::SetIndexingConfig { request_id, config } => {
                match self.handle_set_indexing_config(config.clone()).await {
                    Ok(()) => DaemonResponse::IndexingConfigSet { request_id, config },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            // Git-aware requests
            DaemonRequest::GetCallHierarchyAtCommit {
                request_id,
                file_path,
                symbol,
                line,
                column,
                commit_hash,
                workspace_hint,
            } => {
                match self
                    .handle_call_hierarchy_at_commit(
                        &file_path,
                        &symbol,
                        line,
                        column,
                        &commit_hash,
                        workspace_hint,
                    )
                    .await
                {
                    Ok((result, git_context)) => DaemonResponse::CallHierarchyAtCommit {
                        request_id,
                        result,
                        commit_hash,
                        git_context: Some(git_context),
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::GetCacheHistory {
                request_id,
                file_path,
                symbol,
                workspace_hint: _,
            } => match self.handle_get_cache_history(&file_path, &symbol).await {
                Ok(history) => DaemonResponse::CacheHistory {
                    request_id,
                    history,
                },
                Err(e) => DaemonResponse::Error {
                    request_id,
                    error: e.to_string(),
                },
            },

            DaemonRequest::GetCacheAtCommit {
                request_id,
                commit_hash,
                workspace_hint: _,
            } => match self.call_graph_cache.snapshot_at_commit(&commit_hash).await {
                Ok(snapshot) => DaemonResponse::CacheAtCommit {
                    request_id,
                    commit_hash,
                    snapshot,
                },
                Err(e) => DaemonResponse::Error {
                    request_id,
                    error: e.to_string(),
                },
            },

            DaemonRequest::DiffCacheCommits {
                request_id,
                from_commit,
                to_commit,
                workspace_hint: _,
            } => {
                match self
                    .call_graph_cache
                    .diff_commits(&from_commit, &to_commit)
                    .await
                {
                    Ok(diff) => DaemonResponse::CacheCommitDiff {
                        request_id,
                        from_commit,
                        to_commit,
                        diff,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            // Workspace cache management requests
            DaemonRequest::WorkspaceCacheList { request_id } => {
                match self
                    .workspace_cache_router
                    .list_all_workspace_caches()
                    .await
                {
                    Ok(workspaces) => DaemonResponse::WorkspaceCacheList {
                        request_id,
                        workspaces,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::WorkspaceCacheInfo {
                request_id,
                workspace_path,
            } => {
                match self
                    .workspace_cache_router
                    .get_workspace_cache_info(workspace_path.clone())
                    .await
                {
                    Ok(info_list) => {
                        if workspace_path.is_some() && !info_list.is_empty() {
                            // Return single workspace info
                            DaemonResponse::WorkspaceCacheInfo {
                                request_id,
                                workspace_info: Some(Box::new(
                                    info_list.into_iter().next().unwrap(),
                                )),
                                all_workspaces_info: None,
                            }
                        } else if workspace_path.is_none() && !info_list.is_empty() {
                            // Return all workspaces info
                            DaemonResponse::WorkspaceCacheInfo {
                                request_id,
                                workspace_info: None,
                                all_workspaces_info: Some(info_list),
                            }
                        } else {
                            // No info found
                            DaemonResponse::WorkspaceCacheInfo {
                                request_id,
                                workspace_info: None,
                                all_workspaces_info: None,
                            }
                        }
                    }
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::WorkspaceCacheClear {
                request_id,
                workspace_path,
            } => {
                info!(
                    "Workspace cache clear requested for: {:?}",
                    workspace_path
                        .as_deref()
                        .unwrap_or("all workspaces".as_ref())
                );

                match self
                    .workspace_cache_router
                    .clear_workspace_cache(workspace_path)
                    .await
                {
                    Ok(result) => {
                        info!(
                            "Workspace cache clear completed: {} workspaces cleared, {} bytes freed, {} files removed",
                            result.cleared_workspaces.len(),
                            result.total_size_freed_bytes,
                            result.total_files_removed
                        );

                        if !result.errors.is_empty() {
                            warn!(
                                "Workspace cache clear had {} errors: {:?}",
                                result.errors.len(),
                                result.errors
                            );
                        }

                        DaemonResponse::WorkspaceCacheCleared { request_id, result }
                    }
                    Err(e) => {
                        error!("Workspace cache clear failed: {}", e);
                        DaemonResponse::Error {
                            request_id,
                            error: e.to_string(),
                        }
                    }
                }
            }

            DaemonRequest::Definition {
                request_id,
                file_path,
                line,
                column,
                workspace_hint,
            } => {
                info!(
                    "Received DaemonRequest::Definition for {:?} at {}:{} (request_id: {})",
                    file_path, line, column, request_id
                );
                match self
                    .handle_definition(&file_path, line, column, workspace_hint)
                    .await
                {
                    Ok(locations) => DaemonResponse::Definition {
                        request_id,
                        locations,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::References {
                request_id,
                file_path,
                line,
                column,
                include_declaration,
                workspace_hint,
            } => {
                info!(
                    "Received DaemonRequest::References for {:?} at {}:{} include_decl={} (request_id: {})",
                    file_path, line, column, include_declaration, request_id
                );
                match self
                    .handle_references(
                        &file_path,
                        line,
                        column,
                        include_declaration,
                        workspace_hint,
                    )
                    .await
                {
                    Ok(locations) => DaemonResponse::References {
                        request_id,
                        locations,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::Hover {
                request_id,
                file_path,
                line,
                column,
                workspace_hint,
            } => {
                info!(
                    "Received DaemonRequest::Hover for {:?} at {}:{} (request_id: {})",
                    file_path, line, column, request_id
                );
                match self
                    .handle_hover(&file_path, line, column, workspace_hint)
                    .await
                {
                    Ok(content) => DaemonResponse::Hover {
                        request_id,
                        content,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::DocumentSymbols {
                request_id,
                file_path,
                workspace_hint,
            } => {
                info!(
                    "Received DaemonRequest::DocumentSymbols for {:?} (request_id: {})",
                    file_path, request_id
                );
                match self
                    .handle_document_symbols(&file_path, workspace_hint)
                    .await
                {
                    Ok(symbols) => DaemonResponse::DocumentSymbols {
                        request_id,
                        symbols,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::WorkspaceSymbols {
                request_id,
                query,
                workspace_hint,
            } => {
                info!(
                    "Received DaemonRequest::WorkspaceSymbols query='{}' (request_id: {})",
                    query, request_id
                );
                match self.handle_workspace_symbols(&query, workspace_hint).await {
                    Ok(symbols) => DaemonResponse::WorkspaceSymbols {
                        request_id,
                        symbols,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::Implementations {
                request_id,
                file_path,
                line,
                column,
                workspace_hint,
            } => {
                info!(
                    "Received DaemonRequest::Implementations for {:?} at {}:{} (request_id: {})",
                    file_path, line, column, request_id
                );
                match self
                    .handle_implementations(&file_path, line, column, workspace_hint)
                    .await
                {
                    Ok(locations) => DaemonResponse::Implementations {
                        request_id,
                        locations,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            DaemonRequest::TypeDefinition {
                request_id,
                file_path,
                line,
                column,
                workspace_hint,
            } => {
                info!(
                    "Received DaemonRequest::TypeDefinition for {:?} at {}:{} (request_id: {})",
                    file_path, line, column, request_id
                );
                match self
                    .handle_type_definition(&file_path, line, column, workspace_hint)
                    .await
                {
                    Ok(locations) => DaemonResponse::TypeDefinition {
                        request_id,
                        locations,
                    },
                    Err(e) => DaemonResponse::Error {
                        request_id,
                        error: e.to_string(),
                    },
                }
            }

            // Explicit "not implemented" response for completion - not part of this implementation
            DaemonRequest::Completion { request_id, .. } => {
                warn!("Received unimplemented completion request, returning error with original request_id");
                DaemonResponse::Error {
                    request_id,
                    error: "Completion request type not implemented".to_string(),
                }
            }
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

        // Convert relative path to absolute path
        // Be tolerant to transient canonicalize issues (e.g., symlinks/overlays in test fixtures).
        let absolute_file_path = match safe_canonicalize(file_path).as_path() {
            p if p.exists() => p.to_path_buf(),
            _ => {
                if file_path.is_absolute() {
                    file_path.to_path_buf()
                } else {
                    std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("/"))
                        .join(file_path)
                }
            }
        };

        // Compute MD5 hash for cache key
        let content_md5 = md5_hex_file(&absolute_file_path)?;

        // Fast path: position-indexed cache lookup before touching the language server
        if let Some(hit) =
            self.call_graph_cache
                .get_by_position(&absolute_file_path, line, column, &content_md5)
        {
            info!(
                "Call hierarchy cache HIT for {} at {}:{} (md5: {}): symbol={}",
                absolute_file_path.display(),
                line,
                column,
                content_md5,
                hit.key.symbol
            );
            // Rebuild an LSP-like JSON response from cached info and parse it into our protocol
            let response = self.cache_to_lsp_json(&absolute_file_path, &hit.key.symbol, &hit.info);
            let protocol_result = parse_call_hierarchy_from_lsp(&response)?;
            return Ok(protocol_result);
        }

        // Detect language
        let language = self.detector.detect(file_path)?;

        if language == Language::Unknown {
            return Err(anyhow!("Unknown language for file: {:?}", file_path));
        }

        // Clone workspace_hint before it's moved to the resolver
        let workspace_hint_for_cache = workspace_hint.clone();

        // Resolve workspace root
        let workspace_root = {
            let mut resolver = self.workspace_resolver.lock().await;
            resolver.resolve_workspace(file_path, workspace_hint)?
        };

        // Ensure workspace is registered with the server for this language
        let server_instance = self
            .server_manager
            .ensure_workspace_registered(language, workspace_root.clone())
            .await?;

        // Read file content
        let content = fs::read_to_string(&absolute_file_path)?;

        // Adaptive timing for Go/TypeScript in CI environments
        let is_ci = std::env::var("PROBE_CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok();
        // New strategy: probe immediately, then back off. This removes unconditional sleeps
        // and avoids blowing up test budgets, especially in "initialization timeout" paths.
        let (initial_wait, max_attempts, retry_delay) = match language {
            Language::Go | Language::TypeScript | Language::JavaScript if is_ci => {
                (5, 5, 3) // was (15,5,5): faster in CI; still allows warm-up
            }
            Language::Go | Language::TypeScript | Language::JavaScript => (0, 3, 2),
            _ => (0, 3, 1),
        };

        debug!(
            "Using adaptive timing for {:?}: initial_wait={}s, max_attempts={}, retry_delay={}s (CI={})",
            language, initial_wait, max_attempts, retry_delay, is_ci
        );

        // Ensure document is opened and ready before querying call hierarchy
        // This is critical for rust-analyzer which returns null if the document isn't properly opened
        {
            let server = server_instance.lock().await;

            debug!(
                "Opening document for LSP analysis: {:?}",
                absolute_file_path
            );

            // Always re-open the document to ensure rust-analyzer has the latest content
            // rust-analyzer needs the file to be properly opened and processed before call hierarchy works
            server
                .server
                .open_document(&absolute_file_path, &content)
                .await?;

            // For rust-analyzer, give it time to process the file and establish context
            if language == Language::Rust {
                debug!(
                    "Allowing rust-analyzer time to process and index document: {:?}",
                    absolute_file_path
                );
                // Wait for rust-analyzer to index the file content and establish symbol context
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            }
        }

        // Additional initial wait for complex language servers in CI environments
        if initial_wait > 0 {
            tokio::time::sleep(tokio::time::Duration::from_secs(initial_wait)).await;
        }

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
                    debug!(
                        "Call hierarchy response received for attempt {}: {:?}",
                        attempt, response
                    );

                    // Check if we have a valid item
                    let has_valid_item = if let Some(item) = response.get("item") {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            if name != "unknown" && !name.is_empty() {
                                debug!("Found valid symbol '{}' in call hierarchy response", name);
                                true
                            } else {
                                debug!("Item has invalid name: '{}'", name);
                                false
                            }
                        } else {
                            debug!("Item missing name field");
                            false
                        }
                    } else {
                        debug!("Response missing item field - this indicates rust-analyzer returned null");
                        false
                    };

                    // Check for any incoming/outgoing calls
                    let has_call_data = response
                        .get("incoming")
                        .and_then(|v| v.as_array())
                        .is_some_and(|arr| !arr.is_empty())
                        || response
                            .get("outgoing")
                            .and_then(|v| v.as_array())
                            .is_some_and(|arr| !arr.is_empty());

                    if has_call_data {
                        debug!("Found call hierarchy data (incoming/outgoing calls)");
                    }

                    // Accept the result if we have either a valid item or call data
                    if has_valid_item || has_call_data {
                        result = Some(response);
                        break;
                    }

                    // For rust-analyzer, if we get a null response (no item), retry
                    if language == Language::Rust && !has_valid_item && attempt < max_attempts {
                        debug!("rust-analyzer returned null response - document may not be fully indexed yet, retrying...");
                        // Give rust-analyzer more time to process
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }

                    result = Some(response); // Keep the last response even if empty
                }
                Err(e) => {
                    warn!(
                        "Call hierarchy request failed on attempt {}: {}",
                        attempt, e
                    );
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

        // Convert the result to our protocol type and update cache edges
        let protocol_result = parse_call_hierarchy_from_lsp(&result)?;

        // Now that we have the result, extract the symbol name and cache it
        if !protocol_result.item.name.is_empty() && protocol_result.item.name != "unknown" {
            let symbol_name = protocol_result.item.name.clone();
            let node_id = NodeId::new(&symbol_name, absolute_file_path.clone());

            info!(
                "Caching call hierarchy for {}:{} (md5: {})",
                absolute_file_path.display(),
                symbol_name,
                content_md5
            );

            // Extract edges from the result
            let incoming_ids: Vec<NodeId> = protocol_result
                .incoming
                .iter()
                .map(|call| {
                    let file_path = PathBuf::from(&call.from.uri.replace("file://", ""));
                    NodeId::new(&call.from.name, file_path)
                })
                .collect();

            let outgoing_ids: Vec<NodeId> = protocol_result
                .outgoing
                .iter()
                .map(|call| {
                    let file_path = PathBuf::from(&call.from.uri.replace("file://", ""));
                    NodeId::new(&call.from.name, file_path)
                })
                .collect();

            // Update the cache edges for graph-based invalidation
            self.call_graph_cache
                .update_edges(&node_id, incoming_ids, outgoing_ids);

            // Create cache key and store the result
            let cache_key = NodeKey::new(
                &symbol_name,
                absolute_file_path.clone(),
                content_md5.clone(),
            );
            let cache_info = self.convert_to_cache_info(&protocol_result);

            // Capture request position for index
            let pos_file_for_index = absolute_file_path.clone();
            let pos_md5_for_index = content_md5.clone();
            let pos_line_for_index = line;
            let pos_col_for_index = column;

            // Store in cache for future use
            let cache_clone = self.call_graph_cache.clone();
            let workspace_hint_for_cache_closure = workspace_hint_for_cache.clone();
            let cached_future = async move {
                match cache_clone
                    .get_or_compute_with_workspace_hint(
                        cache_key.clone(),
                        workspace_hint_for_cache_closure,
                        || async { Ok(cache_info) },
                    )
                    .await
                {
                    Ok(_) => {
                        // Also index the position that produced this result
                        cache_clone.index_position(
                            &pos_file_for_index,
                            pos_line_for_index,
                            pos_col_for_index,
                            &pos_md5_for_index,
                            &cache_key,
                        );
                        debug!(
                            "Successfully cached result for {} at {}:{}",
                            cache_key.symbol, pos_line_for_index, pos_col_for_index
                        );
                    }
                    Err(e) => warn!("Failed to cache result: {}", e),
                }
            };
            tokio::spawn(cached_future);
        }

        Ok(protocol_result)
    }

    /// Convert protocol CallHierarchyResult to cache CallHierarchyInfo
    fn convert_to_cache_info(&self, result: &CallHierarchyResult) -> CallHierarchyInfo {
        let incoming_calls = result
            .incoming
            .iter()
            .map(|call| CallInfo {
                name: call.from.name.clone(),
                file_path: call.from.uri.replace("file://", ""),
                line: call.from.range.start.line,
                column: call.from.range.start.character,
                symbol_kind: call.from.kind.clone(),
            })
            .collect();

        let outgoing_calls = result
            .outgoing
            .iter()
            .map(|call| CallInfo {
                name: call.from.name.clone(),
                file_path: call.from.uri.replace("file://", ""),
                line: call.from.range.start.line,
                column: call.from.range.start.character,
                symbol_kind: call.from.kind.clone(),
            })
            .collect();

        CallHierarchyInfo {
            incoming_calls,
            outgoing_calls,
        }
    }

    /// Convert cache CallHierarchyInfo to protocol CallHierarchyResult
    #[allow(dead_code)]
    fn convert_from_cache_info(
        &self,
        info: &CallHierarchyInfo,
        item: CallHierarchyItem,
    ) -> CallHierarchyResult {
        use crate::protocol::{CallHierarchyCall, Position, Range};

        let incoming = info
            .incoming_calls
            .iter()
            .map(|call| CallHierarchyCall {
                from: CallHierarchyItem {
                    name: call.name.clone(),
                    kind: call.symbol_kind.clone(),
                    uri: format!("file://{}", call.file_path),
                    range: Range {
                        start: Position {
                            line: call.line,
                            character: call.column,
                        },
                        end: Position {
                            line: call.line,
                            character: call.column,
                        },
                    },
                    selection_range: Range {
                        start: Position {
                            line: call.line,
                            character: call.column,
                        },
                        end: Position {
                            line: call.line,
                            character: call.column,
                        },
                    },
                },
                from_ranges: vec![],
            })
            .collect();

        let outgoing = info
            .outgoing_calls
            .iter()
            .map(|call| CallHierarchyCall {
                from: CallHierarchyItem {
                    name: call.name.clone(),
                    kind: call.symbol_kind.clone(),
                    uri: format!("file://{}", call.file_path),
                    range: Range {
                        start: Position {
                            line: call.line,
                            character: call.column,
                        },
                        end: Position {
                            line: call.line,
                            character: call.column,
                        },
                    },
                    selection_range: Range {
                        start: Position {
                            line: call.line,
                            character: call.column,
                        },
                        end: Position {
                            line: call.line,
                            character: call.column,
                        },
                    },
                },
                from_ranges: vec![],
            })
            .collect();

        CallHierarchyResult {
            item,
            incoming,
            outgoing,
        }
    }

    /// Convert cached CallHierarchyInfo back into an LSP-like JSON envelope
    /// so we can reuse `parse_call_hierarchy_from_lsp(...)` and return the same protocol type.
    fn cache_to_lsp_json(
        &self,
        file: &Path,
        symbol: &str,
        cached: &CallHierarchyInfo,
    ) -> serde_json::Value {
        use serde_json::json;

        // The parser expects: { item: { name, uri }, incoming: [...], outgoing: [...] }
        let file_uri = format!("file://{}", file.display());

        let incoming = cached
            .incoming_calls
            .iter()
            .map(|c| {
                json!({
                    "from": {
                        "name": c.name,
                        "uri": format!("file://{}", c.file_path),
                        "kind": c.symbol_kind,
                        "range": {
                            "start": {"line": c.line, "character": c.column},
                            "end": {"line": c.line, "character": c.column}
                        },
                        "selectionRange": {
                            "start": {"line": c.line, "character": c.column},
                            "end": {"line": c.line, "character": c.column}
                        }
                    },
                    "fromRanges": []
                })
            })
            .collect::<Vec<_>>();

        let outgoing = cached
            .outgoing_calls
            .iter()
            .map(|c| {
                json!({
                    "from": {
                        "name": c.name,
                        "uri": format!("file://{}", c.file_path),
                        "kind": c.symbol_kind,
                        "range": {
                            "start": {"line": c.line, "character": c.column},
                            "end": {"line": c.line, "character": c.column}
                        },
                        "selectionRange": {
                            "start": {"line": c.line, "character": c.column},
                            "end": {"line": c.line, "character": c.column}
                        }
                    },
                    "fromRanges": []
                })
            })
            .collect::<Vec<_>>();

        json!({
            "item": {
                "name": symbol,
                "uri": file_uri,
                "kind": "12", // Function kind
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 0}
                },
                "selectionRange": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 0}
                }
            },
            "incoming": incoming,
            "outgoing": outgoing
        })
    }

    // ========================================================================================
    // LSP Response Parsing Helper Functions
    // ========================================================================================

    /// Parse LSP definition response (JSON) into Vec<Location>
    fn parse_definition_response(response: &serde_json::Value) -> Result<Vec<Location>> {
        if let Some(locations) = response.as_array() {
            let mut result = Vec::new();
            for loc_value in locations {
                if let Ok(location) = serde_json::from_value::<Location>(loc_value.clone()) {
                    result.push(location);
                }
            }
            Ok(result)
        } else if let Ok(location) = serde_json::from_value::<Location>(response.clone()) {
            Ok(vec![location])
        } else if response.is_null() {
            Ok(Vec::new())
        } else {
            Err(anyhow!("Invalid definition response format: {}", response))
        }
    }

    /// Parse LSP references response (JSON) into Vec<Location>
    fn parse_references_response(response: &serde_json::Value) -> Result<Vec<Location>> {
        if let Some(locations) = response.as_array() {
            let mut result = Vec::new();
            for loc_value in locations {
                if let Ok(location) = serde_json::from_value::<Location>(loc_value.clone()) {
                    result.push(location);
                }
            }
            Ok(result)
        } else if response.is_null() {
            Ok(Vec::new())
        } else {
            Err(anyhow!("Invalid references response format: {}", response))
        }
    }

    /// Parse LSP hover response (JSON) into Option<HoverContent>
    fn parse_hover_response(response: &serde_json::Value) -> Result<Option<HoverContent>> {
        if response.is_null() {
            return Ok(None);
        }

        if let Ok(hover) = serde_json::from_value::<HoverContent>(response.clone()) {
            Ok(Some(hover))
        } else {
            // Try to parse basic hover format
            if let Some(contents) = response.get("contents") {
                let contents_str = if contents.is_string() {
                    contents.as_str().unwrap_or("").to_string()
                } else if contents.is_array() {
                    // Handle array of markup content
                    contents
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .map(|v| v.as_str().unwrap_or(""))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    contents.to_string()
                };

                let range = response
                    .get("range")
                    .and_then(|r| serde_json::from_value::<crate::protocol::Range>(r.clone()).ok());

                Ok(Some(HoverContent {
                    contents: contents_str,
                    range,
                }))
            } else {
                Err(anyhow!("Invalid hover response format: {}", response))
            }
        }
    }

    // ========================================================================================
    // New LSP Operation Handler Methods
    // ========================================================================================

    async fn handle_definition(
        &self,
        file_path: &Path,
        line: u32,
        column: u32,
        workspace_hint: Option<PathBuf>,
    ) -> Result<Vec<Location>> {
        let absolute_file_path = safe_canonicalize(file_path);
        let content_md5 = md5_hex_file(&absolute_file_path)?;

        // Use cache
        let cached_result = self
            .call_graph_cache
            .get_or_compute_definition(
                absolute_file_path.clone(),
                line,
                column,
                content_md5,
                || async {
                    // Call LSP server for computation
                    let language = self.detector.detect(&absolute_file_path)?;
                    if language == Language::Unknown {
                        return Err(anyhow!(
                            "Unknown language for file: {:?}",
                            absolute_file_path
                        ));
                    }

                    let workspace_root = {
                        let mut resolver = self.workspace_resolver.lock().await;
                        resolver.resolve_workspace(&absolute_file_path, workspace_hint.clone())?
                    };

                    let server_instance = self
                        .server_manager
                        .ensure_workspace_registered(language, workspace_root)
                        .await?;

                    let server = server_instance.lock().await;
                    let response_json = server
                        .server
                        .definition(&absolute_file_path, line, column)
                        .await?;
                    let locations = Self::parse_definition_response(&response_json)?;
                    Ok(locations)
                },
            )
            .await?;

        // Convert cached LocationInfo to protocol Location
        Ok(cached_result
            .data
            .locations
            .clone()
            .into_iter()
            .map(|loc| Location {
                uri: loc.uri,
                range: crate::protocol::Range {
                    start: crate::protocol::Position {
                        line: loc.range.start_line,
                        character: loc.range.start_character,
                    },
                    end: crate::protocol::Position {
                        line: loc.range.end_line,
                        character: loc.range.end_character,
                    },
                },
            })
            .collect())
    }

    async fn handle_references(
        &self,
        file_path: &Path,
        line: u32,
        column: u32,
        include_declaration: bool,
        workspace_hint: Option<PathBuf>,
    ) -> Result<Vec<Location>> {
        let absolute_file_path = safe_canonicalize(file_path);
        let content_md5 = md5_hex_file(&absolute_file_path)?;

        // Use cache
        let cached_result = self
            .call_graph_cache
            .get_or_compute_references(
                absolute_file_path.clone(),
                line,
                column,
                content_md5,
                include_declaration,
                || async {
                    // Call LSP server for computation
                    let language = self.detector.detect(&absolute_file_path)?;
                    if language == Language::Unknown {
                        return Err(anyhow!(
                            "Unknown language for file: {:?}",
                            absolute_file_path
                        ));
                    }

                    let workspace_root = {
                        let mut resolver = self.workspace_resolver.lock().await;
                        resolver.resolve_workspace(&absolute_file_path, workspace_hint.clone())?
                    };

                    let server_instance = self
                        .server_manager
                        .ensure_workspace_registered(language, workspace_root)
                        .await?;

                    let server = server_instance.lock().await;
                    let response_json = server
                        .server
                        .references(&absolute_file_path, line, column, include_declaration)
                        .await?;
                    let locations = Self::parse_references_response(&response_json)?;
                    Ok(locations)
                },
            )
            .await?;

        // Convert cached LocationInfo to protocol Location
        Ok(cached_result
            .data
            .locations
            .clone()
            .into_iter()
            .map(|loc| Location {
                uri: loc.uri,
                range: crate::protocol::Range {
                    start: crate::protocol::Position {
                        line: loc.range.start_line,
                        character: loc.range.start_character,
                    },
                    end: crate::protocol::Position {
                        line: loc.range.end_line,
                        character: loc.range.end_character,
                    },
                },
            })
            .collect())
    }

    async fn handle_hover(
        &self,
        file_path: &Path,
        line: u32,
        column: u32,
        workspace_hint: Option<PathBuf>,
    ) -> Result<Option<HoverContent>> {
        let absolute_file_path = safe_canonicalize(file_path);
        let content_md5 = md5_hex_file(&absolute_file_path)?;

        // Use cache
        let cached_result = self
            .call_graph_cache
            .get_or_compute_hover(
                absolute_file_path.clone(),
                line,
                column,
                content_md5,
                || async {
                    // Call LSP server for computation
                    let language = self.detector.detect(&absolute_file_path)?;
                    if language == Language::Unknown {
                        return Err(anyhow!(
                            "Unknown language for file: {:?}",
                            absolute_file_path
                        ));
                    }

                    let workspace_root = {
                        let mut resolver = self.workspace_resolver.lock().await;
                        resolver.resolve_workspace(&absolute_file_path, workspace_hint.clone())?
                    };

                    let server_instance = self
                        .server_manager
                        .ensure_workspace_registered(language, workspace_root)
                        .await?;

                    let server = server_instance.lock().await;
                    let response_json = server
                        .server
                        .hover(&absolute_file_path, line, column)
                        .await?;
                    let hover = Self::parse_hover_response(&response_json)?;
                    Ok(hover)
                },
            )
            .await?;

        // Convert cached HoverInfo to protocol HoverContent
        Ok(cached_result
            .data
            .contents
            .clone()
            .map(|contents| HoverContent {
                contents,
                range: cached_result
                    .data
                    .range
                    .clone()
                    .map(|r| crate::protocol::Range {
                        start: crate::protocol::Position {
                            line: r.start_line,
                            character: r.start_character,
                        },
                        end: crate::protocol::Position {
                            line: r.end_line,
                            character: r.end_character,
                        },
                    }),
            }))
    }

    async fn handle_document_symbols(
        &self,
        _file_path: &Path,
        _workspace_hint: Option<PathBuf>,
    ) -> Result<Vec<DocumentSymbol>> {
        // TODO: Implement document symbols support in LSP server
        Err(anyhow!("Document symbols operation is not yet implemented"))
    }

    async fn handle_workspace_symbols(
        &self,
        _query: &str,
        _workspace_hint: Option<PathBuf>,
    ) -> Result<Vec<SymbolInformation>> {
        // TODO: Implement workspace symbols support in LSP server
        Err(anyhow!(
            "Workspace symbols operation is not yet implemented"
        ))
    }

    async fn handle_implementations(
        &self,
        _file_path: &Path,
        _line: u32,
        _column: u32,
        _workspace_hint: Option<PathBuf>,
    ) -> Result<Vec<Location>> {
        // TODO: Implement implementations support in LSP server
        Err(anyhow!("Implementations operation is not yet implemented"))
    }

    async fn handle_type_definition(
        &self,
        _file_path: &Path,
        _line: u32,
        _column: u32,
        _workspace_hint: Option<PathBuf>,
    ) -> Result<Vec<Location>> {
        // TODO: Implement type definition support in LSP server
        Err(anyhow!("Type definition operation is not yet implemented"))
    }

    // ========================================================================================
    // End of New LSP Operation Handler Methods
    // ========================================================================================

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
        let canonical_root = safe_canonicalize(&workspace_root);

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
        let canonical_root = safe_canonicalize(&workspace_root);

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
            let canonical_workspace = safe_canonicalize(&workspace_path);

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
                            let joined_path = std::env::current_dir()
                                .unwrap_or_else(|_| PathBuf::from("/"))
                                .join(&canonical_workspace);
                            safe_canonicalize(&joined_path)
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

        // Abort/await background tasks to stop loops quickly.
        {
            let mut guard = self.background_tasks.lock().await;
            // Abort all in reverse order to stop dependents first
            while let Some(handle) = guard.pop() {
                handle.abort();
                // It's okay if awaiting returns an error due to abort
                let _ = handle.await;
            }
        }

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

        // Wait for children to go away; escalate if needed.
        #[cfg(unix)]
        {
            use std::time::Instant as StdInstant;
            fn pid_still_exists(pid: u32) -> bool {
                // kill(pid, 0) returns 0 if the process exists and we can send signals,
                // -1 with ESRCH if it doesn't exist.
                unsafe {
                    let res = libc::kill(pid as i32, 0);
                    if res == 0 {
                        true
                    } else {
                        #[cfg(target_os = "linux")]
                        let err = *libc::__errno_location();
                        #[cfg(target_os = "macos")]
                        let err = *libc::__error();
                        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                        let err = 0;
                        err != libc::ESRCH
                    }
                }
            }

            let start = StdInstant::now();
            let soft_deadline = Duration::from_secs(2);
            let hard_deadline = Duration::from_secs(5);

            // soft wait
            loop {
                let pids_snapshot: Vec<u32> = {
                    let guard = self.child_processes.lock().await;
                    guard.clone()
                };
                let alive: Vec<u32> = pids_snapshot
                    .into_iter()
                    .filter(|&p| pid_still_exists(p))
                    .collect();
                if alive.is_empty() || start.elapsed() >= soft_deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            // escalate to SIGKILL if anything is still alive
            let pids_snapshot: Vec<u32> = {
                let guard = self.child_processes.lock().await;
                guard.clone()
            };
            for pid in pids_snapshot.into_iter().filter(|&p| pid_still_exists(p)) {
                unsafe {
                    let _ = libc::kill(pid as i32, libc::SIGKILL);
                    warn!("Escalated to SIGKILL for stubborn child process {}", pid);
                }
            }

            // hard wait
            let hard_start = StdInstant::now();
            while hard_start.elapsed() < hard_deadline {
                let guard = self.child_processes.lock().await;
                if guard.iter().all(|&pid| !pid_still_exists(pid)) {
                    break;
                }
                drop(guard);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        // Force kill any remaining processes in our process group
        #[cfg(unix)]
        self.process_group.kill_all();

        // Release PID lock
        if let Some(mut lock) = self.pid_lock.take() {
            lock.unlock()?;
        }

        // Remove socket file (Unix only)
        remove_socket_file(&self.socket_path)?;

        // Final cleanup of pid list
        *self.child_processes.lock().await = Vec::new();

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
            connection_semaphore: self.connection_semaphore.clone(), // Share semaphore
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
            background_tasks: self.background_tasks.clone(),
            watchdog_enabled: self.watchdog_enabled.clone(),
            watchdog_task: self.watchdog_task.clone(),
            process_monitor: self.process_monitor.clone(),
            child_first_seen: self.child_first_seen.clone(),
            index_grace_secs: self.index_grace_secs,
            call_graph_cache: self.call_graph_cache.clone(),
            persistent_store: self.persistent_store.clone(),
            cache_manager: self.cache_manager.clone(),
            workspace_cache_router: self.workspace_cache_router.clone(),
            definition_cache: self.definition_cache.clone(),
            references_cache: self.references_cache.clone(),
            hover_cache: self.hover_cache.clone(),
            indexing_config: self.indexing_config.clone(),
            indexing_manager: self.indexing_manager.clone(),
        }
    }

    // Note: Cache management is now handled by CacheManager

    /// Handle cache clear request
    #[allow(dead_code)]
    async fn handle_cache_clear(
        &self,
        operation: Option<LspOperation>,
    ) -> Result<(Vec<LspOperation>, usize)> {
        let mut operations_cleared = Vec::new();
        let mut total_entries_removed = 0;

        match operation {
            Some(op) => {
                // Clear specific cache
                match op {
                    LspOperation::CallHierarchy => {
                        let stats_before = self.call_graph_cache.stats().await;
                        self.call_graph_cache.clear();
                        total_entries_removed += stats_before.total_nodes;
                        operations_cleared.push(LspOperation::CallHierarchy);
                    }
                    LspOperation::Definition => {
                        let stats_before = self.definition_cache.stats().await;
                        self.definition_cache.clear().await;
                        total_entries_removed += stats_before.total_entries;
                        operations_cleared.push(LspOperation::Definition);
                    }
                    LspOperation::References => {
                        let stats_before = self.references_cache.stats().await;
                        self.references_cache.clear().await;
                        total_entries_removed += stats_before.total_entries;
                        operations_cleared.push(LspOperation::References);
                    }
                    LspOperation::Hover => {
                        let stats_before = self.hover_cache.stats().await;
                        self.hover_cache.clear().await;
                        total_entries_removed += stats_before.total_entries;
                        operations_cleared.push(LspOperation::Hover);
                    }
                    LspOperation::DocumentSymbols => {
                        // Not implemented yet
                        return Err(anyhow!("DocumentSymbols cache not implemented"));
                    }
                }
            }
            None => {
                // Clear all caches
                let call_graph_stats_before = self.call_graph_cache.stats().await;
                let def_stats_before = self.definition_cache.stats().await;
                let ref_stats_before = self.references_cache.stats().await;
                let hover_stats_before = self.hover_cache.stats().await;

                self.call_graph_cache.clear();
                self.definition_cache.clear().await;
                self.references_cache.clear().await;
                self.hover_cache.clear().await;

                total_entries_removed = call_graph_stats_before.total_nodes
                    + def_stats_before.total_entries
                    + ref_stats_before.total_entries
                    + hover_stats_before.total_entries;

                operations_cleared = vec![
                    LspOperation::CallHierarchy,
                    LspOperation::Definition,
                    LspOperation::References,
                    LspOperation::Hover,
                ];
            }
        }

        info!(
            "Cleared {} cache entries for operations: {:?}",
            total_entries_removed, operations_cleared
        );
        Ok((operations_cleared, total_entries_removed))
    }

    /// Handle cache export request
    #[allow(dead_code)]
    async fn handle_cache_export(&self, operation: Option<LspOperation>) -> Result<String> {
        match operation {
            Some(op) => {
                // Export specific cache
                match op {
                    LspOperation::CallHierarchy => {
                        // Call graph cache doesn't have export_to_json, so create basic export
                        let stats = self.call_graph_cache.stats().await;
                        let export_data = serde_json::json!({
                            "operation": "CallHierarchy",
                            "total_nodes": stats.total_nodes,
                            "total_ids": stats.total_ids,
                            "total_files": stats.total_files,
                            "total_edges": stats.total_edges,
                            "inflight_computations": stats.inflight_computations
                        });
                        Ok(serde_json::to_string_pretty(&export_data)?)
                    }
                    LspOperation::Definition => self.definition_cache.export_to_json().await,
                    LspOperation::References => self.references_cache.export_to_json().await,
                    LspOperation::Hover => self.hover_cache.export_to_json().await,
                    LspOperation::DocumentSymbols => {
                        Err(anyhow!("DocumentSymbols cache not implemented"))
                    }
                }
            }
            None => {
                // Export all caches
                let mut all_exports = serde_json::Map::new();

                // Call graph cache
                let call_graph_stats = self.call_graph_cache.stats().await;
                let call_graph_export = serde_json::json!({
                    "operation": "CallHierarchy",
                    "total_nodes": call_graph_stats.total_nodes,
                    "total_ids": call_graph_stats.total_ids,
                    "total_files": call_graph_stats.total_files,
                    "total_edges": call_graph_stats.total_edges,
                    "inflight_computations": call_graph_stats.inflight_computations
                });
                all_exports.insert("CallHierarchy".to_string(), call_graph_export);

                // Definition cache
                let def_export = self.definition_cache.export_to_json().await?;
                all_exports.insert("Definition".to_string(), serde_json::from_str(&def_export)?);

                // References cache
                let ref_export = self.references_cache.export_to_json().await?;
                all_exports.insert("References".to_string(), serde_json::from_str(&ref_export)?);

                // Hover cache
                let hover_export = self.hover_cache.export_to_json().await?;
                all_exports.insert("Hover".to_string(), serde_json::from_str(&hover_export)?);

                Ok(serde_json::to_string_pretty(&all_exports)?)
            }
        }
    }

    // Indexing management methods
    async fn handle_start_indexing(
        &self,
        workspace_root: PathBuf,
        config: crate::protocol::IndexingConfig,
    ) -> Result<String> {
        use crate::indexing::manager::{IndexingManager, ManagerConfig};
        use uuid::Uuid;

        // Convert protocol config to internal manager config
        let manager_config = ManagerConfig {
            max_workers: config.max_workers.unwrap_or_else(|| num_cpus::get().max(2)),
            memory_budget_bytes: config
                .memory_budget_mb
                .map(|mb| mb * 1024 * 1024)
                .unwrap_or(512 * 1024 * 1024),
            memory_pressure_threshold: 0.8,
            max_queue_size: 10000,
            exclude_patterns: config.exclude_patterns,
            include_patterns: config.include_patterns,
            max_file_size_bytes: config
                .max_file_size_mb
                .map(|mb| mb * 1024 * 1024)
                .unwrap_or(10 * 1024 * 1024),
            enabled_languages: config.languages,
            incremental_mode: config.incremental.unwrap_or(true),
            discovery_batch_size: 100,
            status_update_interval_secs: 5,
        };

        // Create and start indexing manager with LSP dependencies
        let manager = IndexingManager::new(
            manager_config,
            self.detector.clone(),
            self.server_manager.clone(),
            self.call_graph_cache.clone(),
            self.definition_cache.clone(),
        );
        let session_id = Uuid::new_v4().to_string();

        // Start indexing
        manager.start_indexing(workspace_root.clone()).await?;

        // Store manager for future operations
        *self.indexing_manager.lock().await = Some(manager);

        info!("Started indexing for workspace: {:?}", workspace_root);
        Ok(session_id)
    }

    async fn handle_stop_indexing(&self, force: bool) -> Result<bool> {
        let mut manager_guard = self.indexing_manager.lock().await;
        if let Some(manager) = manager_guard.as_ref() {
            manager.stop_indexing().await?;
            if force {
                *manager_guard = None;
            }
            info!("Stopped indexing (force: {})", force);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn handle_indexing_status(&self) -> Result<crate::protocol::IndexingStatusInfo> {
        use crate::protocol::{IndexingProgressInfo, IndexingQueueInfo};

        let manager_guard = self.indexing_manager.lock().await;
        if let Some(manager) = manager_guard.as_ref() {
            let status = manager.get_status().await;
            let progress = manager.get_progress().await;

            let status_info = crate::protocol::IndexingStatusInfo {
                manager_status: format!("{status:?}"),
                progress: IndexingProgressInfo {
                    total_files: progress.total_files,
                    processed_files: progress.processed_files,
                    failed_files: progress.failed_files,
                    active_files: progress.active_files,
                    skipped_files: progress.skipped_files,
                    processed_bytes: progress.processed_bytes,
                    symbols_extracted: progress.symbols_extracted,
                    memory_usage_bytes: progress.memory_usage_bytes,
                    peak_memory_bytes: progress.peak_memory_bytes,
                    progress_ratio: if progress.total_files > 0 {
                        (progress.processed_files + progress.failed_files + progress.skipped_files)
                            as f64
                            / progress.total_files as f64
                    } else {
                        0.0
                    },
                    files_per_second: if progress.elapsed_seconds > 0 {
                        progress.processed_files as f64 / progress.elapsed_seconds as f64
                    } else {
                        0.0
                    },
                    bytes_per_second: if progress.elapsed_seconds > 0 {
                        progress.processed_bytes as f64 / progress.elapsed_seconds as f64
                    } else {
                        0.0
                    },
                },
                queue: IndexingQueueInfo {
                    total_items: 0,   // TODO: Get from queue
                    pending_items: 0, // TODO: Get from queue
                    high_priority_items: 0,
                    medium_priority_items: 0,
                    low_priority_items: 0,
                    is_paused: false,
                    memory_pressure: false,
                },
                workers: vec![], // TODO: Get worker info
                session_id: Some("current".to_string()),
                started_at: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .saturating_sub(progress.elapsed_seconds),
                ),
                elapsed_seconds: progress.elapsed_seconds,
            };

            Ok(status_info)
        } else {
            // No indexing manager active
            let status_info = crate::protocol::IndexingStatusInfo {
                manager_status: "Idle".to_string(),
                progress: IndexingProgressInfo {
                    total_files: 0,
                    processed_files: 0,
                    failed_files: 0,
                    active_files: 0,
                    skipped_files: 0,
                    processed_bytes: 0,
                    symbols_extracted: 0,
                    memory_usage_bytes: 0,
                    peak_memory_bytes: 0,
                    progress_ratio: 0.0,
                    files_per_second: 0.0,
                    bytes_per_second: 0.0,
                },
                queue: IndexingQueueInfo {
                    total_items: 0,
                    pending_items: 0,
                    high_priority_items: 0,
                    medium_priority_items: 0,
                    low_priority_items: 0,
                    is_paused: false,
                    memory_pressure: false,
                },
                workers: vec![],
                session_id: None,
                started_at: None,
                elapsed_seconds: 0,
            };

            Ok(status_info)
        }
    }

    async fn handle_set_indexing_config(
        &self,
        config: crate::protocol::IndexingConfig,
    ) -> Result<()> {
        // Convert protocol config to internal config using the proper conversion function
        let internal_config = crate::indexing::IndexingConfig::from_protocol_config(&config);

        // Update stored config
        *self.indexing_config.write().await = internal_config;

        info!("Updated indexing configuration");
        Ok(())
    }

    fn convert_internal_to_protocol_config(
        &self,
        config: &crate::indexing::IndexingConfig,
    ) -> crate::protocol::IndexingConfig {
        // Use the proper conversion function
        config.to_protocol_config()
    }

    /// Handle call hierarchy at commit request (stub - git functionality removed)
    async fn handle_call_hierarchy_at_commit(
        &self,
        file_path: &Path,
        _symbol: &str,
        line: u32,
        column: u32,
        _commit_hash: &str,
        workspace_hint: Option<PathBuf>,
    ) -> Result<(
        crate::protocol::CallHierarchyResult,
        crate::protocol::GitContext,
    )> {
        // Git functionality has been removed - fall back to current call hierarchy
        let result = self
            .handle_call_hierarchy(file_path, line, column, workspace_hint)
            .await?;

        // Return a stub git context
        let git_context = crate::protocol::GitContext {
            commit_hash: "unknown".to_string(),
            branch: "unknown".to_string(),
            is_dirty: false,
            remote_url: None,
            repo_root: std::env::current_dir().unwrap_or_default(),
        };

        Ok((result, git_context))
    }

    /// Handle get cache history request
    async fn handle_get_cache_history(
        &self,
        file_path: &Path,
        symbol: &str,
    ) -> Result<Vec<crate::protocol::CacheHistoryEntry>> {
        let node_id = crate::cache_types::NodeId::new(symbol, file_path);
        self.call_graph_cache.get_history(&node_id).await
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
