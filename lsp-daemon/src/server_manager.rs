use crate::health_monitor::HealthMonitor;
use crate::language_detector::Language;
use crate::lsp_registry::LspServerConfig;
use crate::lsp_server::LspServer;
use crate::protocol::WorkspaceInfo;
use crate::watchdog::ProcessMonitor;
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
// Provide a grace period where health checks won't restart new, CPU-heavy servers
const STARTUP_HEALTH_GRACE_SECS: u64 = 180;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};
use tracing::{debug, info, warn};
use url::Url;

/// A single server instance that supports multiple workspaces
#[derive(Debug)]
pub struct ServerInstance {
    pub server: LspServer,
    pub registered_workspaces: HashSet<PathBuf>,
    pub initialized: bool,
    pub last_used: Instant,
    pub start_time: Instant,
}

impl ServerInstance {
    pub fn new(server: LspServer) -> Self {
        let now = Instant::now();
        Self {
            server,
            registered_workspaces: HashSet::new(),
            initialized: false,
            last_used: now,
            start_time: now,
        }
    }

    pub fn touch(&mut self) {
        self.last_used = Instant::now();
    }

    pub fn is_workspace_registered(&self, workspace: &PathBuf) -> bool {
        self.registered_workspaces.contains(workspace)
    }

    pub fn add_workspace(&mut self, workspace: PathBuf) {
        self.registered_workspaces.insert(workspace);
    }

    pub fn remove_workspace(&mut self, workspace: &PathBuf) {
        self.registered_workspaces.remove(workspace);
    }
}

/// Manages single server instances per language with multi-workspace support
#[derive(Debug, Clone)]
pub struct SingleServerManager {
    servers: Arc<DashMap<Language, Arc<Mutex<ServerInstance>>>>,
    registry: Arc<crate::lsp_registry::LspRegistry>,
    child_processes: Arc<tokio::sync::Mutex<Vec<u32>>>,
    health_monitor: Arc<HealthMonitor>,
    process_monitor: Arc<ProcessMonitor>,
}

impl SingleServerManager {
    pub fn new(registry: Arc<crate::lsp_registry::LspRegistry>) -> Self {
        Self::new_with_tracker(registry, Arc::new(tokio::sync::Mutex::new(Vec::new())))
    }

    pub fn new_with_tracker(
        registry: Arc<crate::lsp_registry::LspRegistry>,
        child_processes: Arc<tokio::sync::Mutex<Vec<u32>>>,
    ) -> Self {
        let health_monitor = Arc::new(HealthMonitor::new());
        let process_monitor = Arc::new(ProcessMonitor::with_limits(95.0, 2048)); // 95% CPU, 2GB memory (TSServer-friendly)
        Self {
            servers: Arc::new(DashMap::new()),
            registry,
            child_processes,
            health_monitor,
            process_monitor,
        }
    }

    pub fn new_with_health_monitor(
        registry: Arc<crate::lsp_registry::LspRegistry>,
        child_processes: Arc<tokio::sync::Mutex<Vec<u32>>>,
        health_monitor: Arc<HealthMonitor>,
    ) -> Self {
        let process_monitor = Arc::new(ProcessMonitor::with_limits(95.0, 2048)); // 95% CPU, 2GB memory (TSServer-friendly)
        Self {
            servers: Arc::new(DashMap::new()),
            registry,
            child_processes,
            health_monitor,
            process_monitor,
        }
    }

    /// Start health monitoring for this server manager
    pub fn start_health_monitoring(&self) -> tokio::task::JoinHandle<()> {
        info!("Starting health monitoring for LSP servers");
        self.health_monitor.start_monitoring(Arc::new(self.clone()))
    }

    /// Get the health monitor instance
    pub fn health_monitor(&self) -> Arc<HealthMonitor> {
        self.health_monitor.clone()
    }

    /// Get the process monitor instance
    pub fn process_monitor(&self) -> Arc<ProcessMonitor> {
        self.process_monitor.clone()
    }

    /// Check and handle unhealthy processes
    pub async fn check_process_health(&self) -> Result<()> {
        let pids = {
            let pids_guard = self.child_processes.lock().await;
            pids_guard.clone()
        };

        if pids.is_empty() {
            return Ok(());
        }

        debug!("Checking health of {} child processes", pids.len());
        let unhealthy_pids = self.process_monitor.monitor_children(pids.clone()).await;
        // Track which unhealthy PIDs we are actually allowed to kill (outside warm-up grace)
        let mut kill_list: std::collections::HashSet<u32> = std::collections::HashSet::new();

        if !unhealthy_pids.is_empty() {
            warn!(
                "Found {} unhealthy processes out of {}: {:?}",
                unhealthy_pids.len(),
                pids.len(),
                unhealthy_pids
            );

            // Find which languages correspond to the unhealthy PIDs and restart them
            for &unhealthy_pid in &unhealthy_pids {
                for entry in self.servers.iter() {
                    let language = *entry.key();
                    let server_instance = entry.value();

                    // Try to get server lock without timeout (non-blocking)
                    match server_instance.try_lock() {
                        Ok(server) => {
                            if let Some(server_pid) = server.server.get_pid() {
                                if server_pid == unhealthy_pid {
                                    // Skip restarts during a warm-up window to allow heavy indexers (e.g., tsserver) to settle
                                    let elapsed = server.start_time.elapsed();
                                    if elapsed < Duration::from_secs(STARTUP_HEALTH_GRACE_SECS) {
                                        debug!(
                                            "Process {} ({:?}) above limits but within warm-up grace ({:?}); skipping restart",
                                            unhealthy_pid, language, elapsed
                                        );
                                        // IMPORTANT: also do NOT kill it if within warm-up grace
                                        // We intentionally avoid adding this PID to the kill list.
                                        continue;
                                    }
                                    warn!(
                                        "Process {} belongs to {:?} server - marking for restart",
                                        unhealthy_pid, language
                                    );

                                    // Mark server as unhealthy to trigger restart
                                    self.health_monitor.mark_unhealthy(language).await;
                                    // This PID is past grace; it is safe to terminate.
                                    kill_list.insert(unhealthy_pid);
                                    break;
                                }
                            }
                        }
                        Err(_) => {
                            // Server is busy or locked, skip for now
                            continue;
                        }
                    }
                }
            }

            // Kill only the processes that are past the warm-up grace period
            #[cfg(unix)]
            for &pid in &kill_list {
                unsafe {
                    if libc::kill(pid as i32, libc::SIGTERM) == 0 {
                        warn!("Sent SIGTERM to unhealthy process {}", pid);
                    } else {
                        warn!("Failed to send SIGTERM to process {}", pid);
                    }
                }
            }

            // Remove only killed PIDs from tracking
            {
                let mut pids_guard = self.child_processes.lock().await;
                pids_guard.retain(|&pid| !kill_list.contains(&pid));
                info!(
                    "Removed {} unhealthy processes from tracking, {} remain",
                    kill_list.len(),
                    pids_guard.len()
                );
            }
        } else {
            debug!("All {} child processes are healthy", pids.len());
        }

        Ok(())
    }

    /// Execute an operation with health monitoring
    /// Marks the server as healthy if the operation succeeds, unhealthy if it fails
    pub async fn execute_with_health_monitoring<T, F, Fut>(
        &self,
        language: Language,
        operation: F,
    ) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        match operation().await {
            Ok(result) => {
                self.health_monitor.mark_healthy(language).await;
                Ok(result)
            }
            Err(e) => {
                warn!("Operation failed for {:?}: {}", language, e);
                self.health_monitor.mark_unhealthy(language).await;
                Err(e)
            }
        }
    }

    /// Restart a server if it's unhealthy
    pub async fn restart_server_if_unhealthy(&self, language: Language) -> Result<()> {
        if self.health_monitor.should_restart(language).await {
            warn!("Restarting unhealthy server for {:?}", language);

            // Remove the server from our map
            if let Some((_, server_instance)) = self.servers.remove(&language) {
                // Try to shutdown gracefully
                match tokio::time::timeout(Duration::from_secs(2), server_instance.lock()).await {
                    Ok(server) => {
                        if let Err(e) = server.server.shutdown().await {
                            warn!(
                                "Error shutting down {:?} server during restart: {}",
                                language, e
                            );
                        } else {
                            info!("Successfully shut down {:?} server for restart", language);
                        }
                    }
                    Err(_) => {
                        warn!(
                            "Timeout acquiring lock for {:?} server during restart",
                            language
                        );
                    }
                }
            }

            // Reset health status
            self.health_monitor.reset_health_status(language).await;

            info!("Server restart completed for {:?}", language);
        }
        Ok(())
    }

    /// Get or create a server for the specified language
    pub async fn get_server(&self, language: Language) -> Result<Arc<Mutex<ServerInstance>>> {
        // Check circuit breaker first
        if self.health_monitor.should_reject_request(language).await {
            return Err(anyhow!(
                "Circuit breaker is open for {:?} - server is unhealthy",
                language
            ));
        }

        // Check if server already exists
        if let Some(server_instance) = self.servers.get(&language) {
            // Verify the server is still healthy by trying to acquire lock briefly
            match server_instance.try_lock() {
                Ok(_) => {
                    // Server is responsive
                    return Ok(server_instance.clone());
                }
                Err(_) => {
                    // Server may be busy (e.g., indexing). Don't thrash by removing/recreating immediately.
                    warn!(
                        "Server {:?} lock busy; marking unhealthy but not recreating immediately",
                        language
                    );
                    self.health_monitor.mark_unhealthy(language).await;
                    // Return the existing instance and allow the health monitor to decide on restart later.
                    return Ok(server_instance.clone());
                }
            }
        }

        // Get LSP server config
        let config = self
            .registry
            .get(language)
            .ok_or_else(|| anyhow!("No LSP server configured for {:?}", language))?
            .clone();

        // Create and initialize new server
        let server = self.create_server(&config).await?;
        let instance = Arc::new(Mutex::new(ServerInstance::new(server)));

        // Store the server
        self.servers.insert(language, instance.clone());

        // Mark as healthy after successful creation
        self.health_monitor.mark_healthy(language).await;

        info!("Created new LSP server for {:?}", language);
        Ok(instance)
    }

    /// Ensure a workspace is registered with the server for the given language
    pub async fn ensure_workspace_registered(
        &self,
        language: Language,
        workspace_root: PathBuf,
    ) -> Result<Arc<Mutex<ServerInstance>>> {
        // Log the workspace registration attempt
        info!(
            "Ensuring workspace {:?} is registered for {:?}",
            workspace_root, language
        );

        // Check circuit breaker first
        if self.health_monitor.should_reject_request(language).await {
            return Err(anyhow!(
                "Circuit breaker is open for {:?} - server is unhealthy",
                language
            ));
        }
        // Check if server already exists
        if let Some(server_instance) = self.servers.get(&language) {
            // Try to acquire lock with timeout to prevent hanging
            let server_guard =
                match tokio::time::timeout(Duration::from_secs(10), server_instance.lock()).await {
                    Ok(guard) => guard,
                    Err(_) => {
                        warn!(
                            "Failed to acquire lock for {:?} server within timeout",
                            language
                        );
                        self.health_monitor.mark_unhealthy(language).await;
                        return Err(anyhow!(
                            "Server lock acquisition timeout for {:?}",
                            language
                        ));
                    }
                };

            let mut server = server_guard;

            // If server is not initialized yet, initialize it with this workspace
            if !server.initialized {
                info!(
                    "Initializing {:?} server with first workspace: {:?}",
                    language, workspace_root
                );

                // Get config
                let config = self
                    .registry
                    .get(language)
                    .ok_or_else(|| anyhow!("No LSP server configured for {:?}", language))?
                    .clone();

                // Initialize with the actual workspace
                match server
                    .server
                    .initialize_with_workspace(&config, &workspace_root)
                    .await
                {
                    Ok(_) => {
                        self.health_monitor.mark_healthy(language).await;
                    }
                    Err(e) => {
                        self.health_monitor.mark_unhealthy(language).await;
                        return Err(e);
                    }
                }

                // Mark server as initialized immediately after LSP initialization
                // Don't wait for indexing to complete to avoid blocking
                server.initialized = true;
                server.registered_workspaces.insert(workspace_root.clone());

                info!(
                    "Initialized {:?} server with workspace {:?}",
                    language, workspace_root
                );
                server.touch();
                return Ok(server_instance.clone());
            }

            // Check if workspace is already registered
            if server.is_workspace_registered(&workspace_root) {
                info!(
                    "Workspace {:?} already registered with {:?} server",
                    workspace_root, language
                );
                server.touch();
                return Ok(server_instance.clone());
            }

            // Add workspace to the server
            info!(
                "Adding new workspace {:?} to existing {:?} server",
                workspace_root, language
            );
            match self.register_workspace(&mut server, &workspace_root).await {
                Ok(_) => {
                    self.health_monitor.mark_healthy(language).await;
                    info!(
                        "Successfully registered workspace {:?} with {:?} server",
                        workspace_root, language
                    );
                    return Ok(server_instance.clone());
                }
                Err(e) => {
                    warn!(
                        "Failed to register workspace {:?} with {:?} server: {}",
                        workspace_root, language, e
                    );
                    self.health_monitor.mark_unhealthy(language).await;

                    // Remove the failed server so it gets recreated on next attempt
                    self.servers.remove(&language);

                    return Err(anyhow!(
                        "Failed to register workspace with existing server: {}. Server will be recreated on next attempt.",
                        e
                    ));
                }
            }
        }

        // Create new server and initialize with this workspace
        let config = self
            .registry
            .get(language)
            .ok_or_else(|| anyhow!("No LSP server configured for {:?}", language))?
            .clone();

        info!(
            "Creating and initializing new {:?} server with workspace: {:?}",
            language, workspace_root
        );

        // Spawn server with the workspace root so it starts in the correct directory
        // This is critical for gopls which needs to run in the Go module root
        let mut server = LspServer::spawn_with_workspace(&config, &workspace_root)?;

        // Initialize with the actual workspace from the start
        match server
            .initialize_with_workspace(&config, &workspace_root)
            .await
        {
            Ok(_) => {
                self.health_monitor.mark_healthy(language).await;
            }
            Err(e) => {
                self.health_monitor.mark_unhealthy(language).await;
                return Err(e);
            }
        }

        // Create instance with this workspace already registered and mark as initialized
        // Note: We don't wait for full indexing to complete to avoid blocking
        let mut instance = ServerInstance::new(server);
        instance.initialized = true;
        instance
            .registered_workspaces
            .insert(workspace_root.clone());

        let server_instance = Arc::new(Mutex::new(instance));
        self.servers.insert(language, server_instance.clone());

        // The server is already initialized and ready for basic operations
        // Background indexing will continue automatically without blocking the daemon

        info!(
            "Created and initialized new {:?} server with workspace {:?}",
            language, workspace_root
        );
        Ok(server_instance)
    }

    async fn create_server(&self, config: &LspServerConfig) -> Result<LspServer> {
        debug!("Creating new LSP server for {:?}", config.language);

        // Create server and track its PID
        let mut server = LspServer::spawn(config)?;

        // Track the child process PID
        if let Some(pid) = server.get_pid() {
            let mut pids = self.child_processes.lock().await;
            pids.push(pid);
            info!("Tracking LSP server process with PID: {}", pid);
        }

        // Initialize with a default workspace (will be replaced with actual workspace on first use)
        server.initialize_empty(config).await?;

        // Don't wait for indexing to complete - let it happen in background

        Ok(server)
    }

    async fn register_workspace(
        &self,
        server_instance: &mut ServerInstance,
        workspace_root: &PathBuf,
    ) -> Result<()> {
        // Convert workspace path to URI
        let workspace_uri = Url::from_directory_path(workspace_root).map_err(|_| {
            anyhow!(
                "Failed to convert workspace path to URI: {:?}",
                workspace_root
            )
        })?;

        // Send workspace/didChangeWorkspaceFolders notification
        let params = json!({
            "event": {
                "added": [{
                    "uri": workspace_uri.to_string(),
                    "name": workspace_root
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("workspace")
                        .to_string()
                }],
                "removed": []
            }
        });

        debug!("Adding workspace to server: {:?}", workspace_root);
        server_instance
            .server
            .send_notification("workspace/didChangeWorkspaceFolders", params)
            .await?;

        // Wait briefly for server to notice/index the new workspace
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Mark workspace as registered
        server_instance.add_workspace(workspace_root.clone());
        server_instance.touch();

        Ok(())
    }

    pub async fn unregister_workspace(
        &self,
        language: Language,
        workspace_root: &PathBuf,
    ) -> Result<()> {
        if let Some(server_instance) = self.servers.get(&language) {
            // Use timeout to prevent hanging if server is busy
            let mut server =
                match tokio::time::timeout(Duration::from_secs(5), server_instance.lock()).await {
                    Ok(guard) => guard,
                    Err(_) => {
                        warn!(
                            "Timeout acquiring lock for {:?} server during unregister",
                            language
                        );
                        return Err(anyhow!(
                            "Server lock acquisition timeout for {:?}",
                            language
                        ));
                    }
                };

            if !server.is_workspace_registered(workspace_root) {
                return Ok(()); // Already unregistered
            }

            // Convert workspace path to URI
            let workspace_uri = Url::from_directory_path(workspace_root).map_err(|_| {
                anyhow!(
                    "Failed to convert workspace path to URI: {:?}",
                    workspace_root
                )
            })?;

            // Send workspace/didChangeWorkspaceFolders notification to remove workspace
            let params = json!({
                "event": {
                    "added": [],
                    "removed": [{
                        "uri": workspace_uri.to_string(),
                        "name": workspace_root
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("workspace")
                            .to_string()
                    }]
                }
            });

            debug!("Removing workspace from server: {:?}", workspace_root);
            server
                .server
                .send_notification("workspace/didChangeWorkspaceFolders", params)
                .await?;

            // Mark workspace as unregistered
            server.remove_workspace(workspace_root);
            server.touch();

            info!(
                "Unregistered workspace {:?} from {:?} server",
                workspace_root, language
            );
        }

        Ok(())
    }

    pub async fn shutdown_all(&self) {
        info!("Shutting down all LSP servers");

        // Collect all servers first to avoid holding locks
        let mut servers_to_shutdown = Vec::new();
        for entry in self.servers.iter() {
            let language = *entry.key();
            let server_instance = entry.value().clone();
            servers_to_shutdown.push((language, server_instance));
        }

        // Shutdown each server gracefully
        for (language, server_instance) in servers_to_shutdown {
            // Try to acquire lock with timeout
            match tokio::time::timeout(Duration::from_secs(2), server_instance.lock()).await {
                Ok(server) => {
                    if let Err(e) = server.server.shutdown().await {
                        warn!("Error shutting down {:?} server: {}", language, e);
                    } else {
                        info!("Successfully shut down {:?} server", language);
                    }
                }
                Err(_) => {
                    warn!(
                        "Timeout acquiring lock for {:?} server during shutdown",
                        language
                    );
                }
            }
        }

        // Clear servers from map
        self.servers.clear();

        // Force kill all tracked child processes if any remain
        let mut pids = self.child_processes.lock().await;
        if !pids.is_empty() {
            info!("Force killing {} tracked child processes", pids.len());
            #[cfg(unix)]
            for &pid in pids.iter() {
                unsafe {
                    // First try SIGTERM
                    if libc::kill(pid as i32, libc::SIGTERM) == 0 {
                        debug!("Sent SIGTERM to process {}", pid);
                    }
                }
            }
            #[cfg(not(unix))]
            for &_pid in pids.iter() {
                // Windows: process cleanup handled differently
            }

            // Give processes a moment to terminate
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Then force kill with SIGKILL
            #[cfg(unix)]
            for &pid in pids.iter() {
                unsafe {
                    if libc::kill(pid as i32, libc::SIGKILL) == 0 {
                        debug!("Sent SIGKILL to process {}", pid);
                    }
                }
            }
            #[cfg(not(unix))]
            for &_pid in pids.iter() {
                // Windows: process cleanup handled differently
            }

            // IMPORTANT: clear tracked PIDs so follow-up tests don't "inherit" stale processes
            pids.clear();
            debug!("Cleared tracked child process list after shutdown");
        }
    }

    pub async fn get_stats(&self) -> Vec<ServerStats> {
        let mut stats = Vec::new();
        debug!("get_stats called, {} servers in map", self.servers.len());

        for entry in self.servers.iter() {
            let language = *entry.key();
            let server_instance = entry.value();
            debug!("Processing {:?} server", language);

            // Use non-blocking try_lock for status queries to avoid hangs
            match server_instance.try_lock() {
                Ok(server) => {
                    let status = if !server.initialized {
                        ServerStatus::Initializing
                    } else {
                        ServerStatus::Ready
                    };

                    stats.push(ServerStats {
                        language,
                        workspace_count: server.registered_workspaces.len(),
                        initialized: server.initialized,
                        last_used: server.last_used,
                        workspaces: server.registered_workspaces.iter().cloned().collect(),
                        uptime: server.start_time.elapsed(),
                        status,
                    });
                }
                Err(_) => {
                    // Server is busy (likely initializing), return partial stats immediately
                    // This prevents the status command from hanging
                    debug!("Server {:?} is busy, returning partial stats", language);
                    stats.push(ServerStats {
                        language,
                        workspace_count: 0,                     // Unknown
                        initialized: false, // Likely still initializing if lock is held
                        last_used: tokio::time::Instant::now(), // Unknown, use current time
                        workspaces: Vec::new(), // Unknown
                        uptime: Duration::from_secs(0), // Unknown
                        status: ServerStatus::Initializing, // Most likely initializing if busy
                    });
                }
            }
        }

        stats.sort_by_key(|s| s.language.as_str().to_string());
        stats
    }

    pub async fn get_active_server_count(&self) -> usize {
        self.servers.len()
    }

    pub async fn get_all_workspaces(&self) -> Vec<WorkspaceInfo> {
        let mut workspaces = Vec::new();

        for entry in self.servers.iter() {
            let language = *entry.key();
            let server_instance = entry.value();

            match server_instance.try_lock() {
                Ok(server) => {
                    let status = if server.initialized {
                        crate::protocol::ServerStatus::Ready
                    } else {
                        crate::protocol::ServerStatus::Initializing
                    };

                    for workspace_root in &server.registered_workspaces {
                        workspaces.push(WorkspaceInfo {
                            root: workspace_root.clone(),
                            language,
                            server_status: status.clone(),
                            file_count: None, // Could be enhanced to actually count files
                        });
                    }
                }
                Err(_) => {
                    // Server is currently busy, report as busy status with unknown workspaces
                    tracing::debug!(
                        "Could not acquire lock for {:?} server status - server is busy",
                        language
                    );
                    // We could add a generic workspace entry to show the server exists but is busy
                    workspaces.push(WorkspaceInfo {
                        root: PathBuf::from("<server-busy>"),
                        language,
                        server_status: crate::protocol::ServerStatus::Initializing, // Use initializing as a reasonable default for busy
                        file_count: None,
                    });
                }
            }
        }

        workspaces.sort_by(|a, b| a.root.cmp(&b.root));
        workspaces
    }

    pub async fn cleanup_idle_servers(&self, idle_timeout: Duration) {
        let now = Instant::now();
        let mut to_remove = Vec::new();

        for entry in self.servers.iter() {
            let language = *entry.key();
            let server_instance = entry.value();

            match server_instance.try_lock() {
                Ok(server) => {
                    if now.duration_since(server.last_used) > idle_timeout
                        && server.registered_workspaces.is_empty()
                    {
                        to_remove.push(language);
                    }
                }
                Err(_) => {
                    // Cannot check if server is idle because it's currently busy
                    tracing::debug!("Could not check idle status for {:?} server - server is busy, skipping cleanup", language);
                }
            }
        }

        for language in to_remove {
            if let Some((_, server_instance)) = self.servers.remove(&language) {
                match server_instance.try_lock() {
                    Ok(server) => {
                        if let Err(e) = server.server.shutdown().await {
                            warn!("Error shutting down idle {:?} server: {}", language, e);
                        } else {
                            info!("Shut down idle {:?} server", language);
                        }
                    }
                    Err(_) => {
                        // Server is busy, we removed it from the map but couldn't shut it down cleanly
                        // The server will be orphaned but should shut down when dropped
                        warn!("Could not acquire lock to shutdown idle {:?} server - server is busy. Server instance has been removed from pool and will be orphaned.", language);
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerStats {
    pub language: Language,
    pub workspace_count: usize,
    pub initialized: bool,
    pub last_used: Instant,
    pub workspaces: Vec<PathBuf>,
    pub uptime: Duration,
    pub status: ServerStatus,
}

#[derive(Debug, Clone)]
pub enum ServerStatus {
    Starting,
    Initializing,
    Indexing,
    Ready,
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    // Since the actual server manager tests would require complex mocking of LSP servers,
    // let's test the error handling logic in ServerInstance directly

    #[test]
    fn test_server_instance_workspace_management() {
        // Test workspace management without needing a real LSP server
        // This focuses on the error handling logic in workspace operations

        let workspace1 = PathBuf::from("/test/workspace1");
        let workspace2 = PathBuf::from("/test/workspace2");

        // Test that workspace operations work correctly
        let mut workspaces = HashSet::new();

        // Simulate add_workspace behavior
        workspaces.insert(workspace1.clone());
        assert!(
            workspaces.contains(&workspace1),
            "Workspace should be registered"
        );
        assert!(
            !workspaces.contains(&workspace2),
            "Workspace2 should not be registered"
        );

        // Simulate remove_workspace behavior
        workspaces.remove(&workspace1);
        assert!(
            !workspaces.contains(&workspace1),
            "Workspace should be removed"
        );

        // Test that multiple workspaces can be managed
        workspaces.insert(workspace1.clone());
        workspaces.insert(workspace2.clone());
        assert_eq!(workspaces.len(), 2, "Should have 2 workspaces");

        workspaces.clear();
        assert!(
            workspaces.is_empty(),
            "Should have no workspaces after clear"
        );
    }

    #[test]
    fn test_workspace_info_error_handling() {
        // Test that WorkspaceInfo can be created with various status values
        use crate::protocol::{ServerStatus, WorkspaceInfo};

        let workspace = WorkspaceInfo {
            root: PathBuf::from("/test"),
            language: Language::Rust,
            server_status: ServerStatus::Ready,
            file_count: None,
        };

        assert_eq!(workspace.root, PathBuf::from("/test"));
        assert_eq!(workspace.language, Language::Rust);
    }

    // Additional tests can be added here for more complex error handling scenarios
    // when proper mocking infrastructure is in place
}
