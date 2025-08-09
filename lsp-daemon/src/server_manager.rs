use crate::language_detector::Language;
use crate::lsp_registry::LspServerConfig;
use crate::lsp_server::LspServer;
use crate::protocol::WorkspaceInfo;
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
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
}

impl SingleServerManager {
    pub fn new(registry: Arc<crate::lsp_registry::LspRegistry>) -> Self {
        Self {
            servers: Arc::new(DashMap::new()),
            registry,
        }
    }

    /// Get or create a server for the specified language
    pub async fn get_server(&self, language: Language) -> Result<Arc<Mutex<ServerInstance>>> {
        // Check if server already exists
        if let Some(server_instance) = self.servers.get(&language) {
            return Ok(server_instance.clone());
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

        info!("Created new LSP server for {:?}", language);
        Ok(instance)
    }

    /// Ensure a workspace is registered with the server for the given language
    pub async fn ensure_workspace_registered(
        &self,
        language: Language,
        workspace_root: PathBuf,
    ) -> Result<Arc<Mutex<ServerInstance>>> {
        // Check if server already exists
        if let Some(server_instance) = self.servers.get(&language) {
            let mut server = server_instance.lock().await;

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
                    Ok(_) => {}
                    Err(e) => {
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
                debug!(
                    "Workspace {:?} already registered with {:?} server",
                    workspace_root, language
                );
                server.touch();
                return Ok(server_instance.clone());
            }

            // Add workspace to the server
            self.register_workspace(&mut server, &workspace_root)
                .await?;
            info!(
                "Registered workspace {:?} with {:?} server",
                workspace_root, language
            );
            return Ok(server_instance.clone());
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

        // Spawn server
        let mut server = LspServer::spawn(&config)?;

        // Initialize with the actual workspace from the start
        server
            .initialize_with_workspace(&config, &workspace_root)
            .await?;

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

        // Create server
        let mut server = LspServer::spawn(config)?;

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

        // Wait briefly for server to index the new workspace
        tokio::time::sleep(Duration::from_millis(500)).await;

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
            let mut server = server_instance.lock().await;

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

        // Shutdown each server
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

        self.servers.clear();
    }

    pub async fn get_stats(&self) -> Vec<ServerStats> {
        let mut stats = Vec::new();
        debug!("get_stats called, {} servers in map", self.servers.len());

        for entry in self.servers.iter() {
            let language = *entry.key();
            let server_instance = entry.value();
            debug!("Processing {:?} server", language);

            // Use timeout-based lock instead of try_lock to handle busy servers
            match tokio::time::timeout(Duration::from_millis(1000), server_instance.lock()).await {
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
                    // Return stats even if we can't get the lock, mark as busy/indexing
                    stats.push(ServerStats {
                        language,
                        workspace_count: 0,                     // Unknown
                        initialized: true, // Assume initialized if we have it in the map
                        last_used: tokio::time::Instant::now(), // Unknown, use current time
                        workspaces: Vec::new(), // Unknown
                        uptime: Duration::from_secs(0), // Unknown
                        status: ServerStatus::Indexing, // Likely indexing if busy
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

            if let Ok(server) = server_instance.try_lock() {
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

            if let Ok(server) = server_instance.try_lock() {
                if now.duration_since(server.last_used) > idle_timeout
                    && server.registered_workspaces.is_empty()
                {
                    to_remove.push(language);
                }
            }
        }

        for language in to_remove {
            if let Some((_, server_instance)) = self.servers.remove(&language) {
                if let Ok(server) = server_instance.try_lock() {
                    if let Err(e) = server.server.shutdown().await {
                        warn!("Error shutting down idle {:?} server: {}", language, e);
                    } else {
                        info!("Shut down idle {:?} server", language);
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
