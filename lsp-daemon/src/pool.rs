#![allow(dead_code)] // Pool implementation is kept for future use

use crate::language_detector::Language;
use crate::lsp_registry::LspServerConfig;
use crate::lsp_server::LspServer;
use crate::protocol::WorkspaceInfo;
use anyhow::Result;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{Duration, Instant};
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Clone)]
#[allow(dead_code)]
pub struct PooledServer {
    pub id: Uuid,
    pub server: Arc<LspServer>,
    pub last_used: Instant,
    pub request_count: usize,
    #[allow(dead_code)]
    pub workspace_root: PathBuf,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct LspServerPool {
    config: Arc<LspServerConfig>,
    workspace_root: PathBuf,
    ready: Arc<Mutex<VecDeque<PooledServer>>>,
    busy: Arc<DashMap<Uuid, PooledServer>>,
    semaphore: Arc<Semaphore>,
    min_size: usize,
    max_size: usize,
    max_requests_per_server: usize,
    is_spawning: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl LspServerPool {
    pub fn new(config: LspServerConfig, workspace_root: PathBuf) -> Self {
        let min_size = 1;
        let max_size = 4;
        let pool = Self {
            config: Arc::new(config),
            workspace_root,
            ready: Arc::new(Mutex::new(VecDeque::new())),
            busy: Arc::new(DashMap::new()),
            semaphore: Arc::new(Semaphore::new(max_size)),
            min_size,
            max_size,
            max_requests_per_server: 100, // Recycle after 100 requests
            is_spawning: Arc::new(AtomicBool::new(false)),
        };

        // Start warming minimum servers
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            pool_clone.ensure_minimum_servers().await;
        });

        pool
    }

    pub async fn ensure_minimum_servers(&self) {
        // Don't start new servers if one is already being spawned
        if self.is_spawning.load(Ordering::Acquire) {
            return;
        }

        let ready_count = self.ready.lock().await.len();
        let busy_count = self.busy.len();
        let total = ready_count + busy_count;

        if total < self.min_size {
            let needed = self.min_size - total;
            info!(
                "Pool for {:?}: Starting {} servers (current: {}, min: {})",
                self.config.language, needed, total, self.min_size
            );

            for _ in 0..needed {
                let config = self.config.clone();
                let ready = self.ready.clone();
                let is_spawning = self.is_spawning.clone();
                let workspace_root = self.workspace_root.clone();

                tokio::spawn(async move {
                    // Try to set the spawning flag
                    if is_spawning
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        // Another task is already spawning
                        return;
                    }

                    match Self::spawn_server_with_workspace(&config, &workspace_root).await {
                        Ok(server) => {
                            let pooled = PooledServer {
                                id: Uuid::new_v4(),
                                server: Arc::new(server),
                                last_used: Instant::now(),
                                request_count: 0,
                                workspace_root: workspace_root.clone(),
                            };
                            ready.lock().await.push_back(pooled);
                            info!(
                                "Successfully spawned and warmed LSP server for {:?}",
                                config.language
                            );
                        }
                        Err(e) => {
                            warn!(
                                "Failed to spawn LSP server for {:?}: {}",
                                config.language, e
                            );
                        }
                    }

                    // Clear the spawning flag
                    is_spawning.store(false, Ordering::Release);
                });
            }
        }
    }

    async fn spawn_server_with_workspace(
        config: &LspServerConfig,
        workspace_root: &PathBuf,
    ) -> Result<LspServer> {
        debug!(
            "Spawning new LSP server for {:?} with workspace {:?}",
            config.language, workspace_root
        );
        let mut server = LspServer::spawn_with_workspace(config, workspace_root)?;
        server
            .initialize_with_workspace(config, workspace_root)
            .await?;
        server.wait_until_ready().await?;
        Ok(server)
    }

    pub async fn get_server(&self) -> Result<PooledServer> {
        // Try to get a ready server
        if let Some(server) = self.ready.lock().await.pop_front() {
            debug!(
                "Reusing ready server {} for {:?}",
                server.id, self.config.language
            );

            // Move to busy
            self.busy.insert(server.id, server.clone());

            // Ensure minimum servers in background
            let pool = self.clone();
            tokio::spawn(async move {
                pool.ensure_minimum_servers().await;
            });

            return Ok(server);
        }

        // Check if a server is already being spawned
        if self.is_spawning.load(Ordering::Acquire) {
            info!(
                "Server for {:?} is already being spawned, waiting...",
                self.config.language
            );

            // Wait for the spawning server to become ready
            let start = Instant::now();
            let timeout = Duration::from_secs(self.config.initialization_timeout_secs);

            while start.elapsed() < timeout {
                tokio::time::sleep(Duration::from_millis(500)).await;

                if let Some(server) = self.ready.lock().await.pop_front() {
                    debug!(
                        "Got newly spawned server {} for {:?}",
                        server.id, self.config.language
                    );
                    self.busy.insert(server.id, server.clone());
                    return Ok(server);
                }

                // Check if spawning finished
                if !self.is_spawning.load(Ordering::Acquire) {
                    // Try again to get a server
                    if let Some(server) = self.ready.lock().await.pop_front() {
                        self.busy.insert(server.id, server.clone());
                        return Ok(server);
                    }
                    break;
                }
            }
        }

        // Check if we can spawn a new server
        if self.busy.len() < self.max_size {
            // Try to set the spawning flag
            if self
                .is_spawning
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                info!(
                    "No ready servers for {:?}, spawning new one",
                    self.config.language
                );

                // Acquire semaphore permit
                let _permit = self.semaphore.acquire().await?;

                let server_result =
                    Self::spawn_server_with_workspace(&self.config, &self.workspace_root).await;

                // Always clear the spawning flag
                self.is_spawning.store(false, Ordering::Release);

                let server = server_result?;
                let pooled = PooledServer {
                    id: Uuid::new_v4(),
                    server: Arc::new(server),
                    last_used: Instant::now(),
                    request_count: 0,
                    workspace_root: self.workspace_root.clone(),
                };

                let pooled_copy = pooled.clone();
                self.busy.insert(pooled.id, pooled_copy);

                return Ok(pooled);
            } else {
                // Another thread is spawning, wait for it
                return Box::pin(self.get_server()).await;
            }
        }

        // Wait for a server to become available
        info!(
            "Pool for {:?} is at capacity, waiting for available server",
            self.config.language
        );

        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;

            if let Some(server) = self.ready.lock().await.pop_front() {
                let server_copy = server.clone();
                self.busy.insert(server.id, server_copy);
                return Ok(server);
            }
        }
    }

    pub async fn return_server(&self, mut server: PooledServer) {
        // Remove from busy
        self.busy.remove(&server.id);

        server.last_used = Instant::now();
        server.request_count += 1;

        // Check if server should be recycled
        if server.request_count >= self.max_requests_per_server {
            info!(
                "Recycling server {} for {:?} after {} requests (blue-green strategy)",
                server.id, self.config.language, server.request_count
            );

            // Blue-Green Deployment: Spawn replacement FIRST, then shutdown old server
            let config = self.config.clone();
            let ready = self.ready.clone();
            let workspace_root = self.workspace_root.clone();
            let old_server = server; // Keep reference to old server

            tokio::spawn(async move {
                match Self::spawn_server_with_workspace(&config, &workspace_root).await {
                    Ok(new_server) => {
                        let pooled = PooledServer {
                            id: Uuid::new_v4(),
                            server: Arc::new(new_server),
                            last_used: Instant::now(),
                            request_count: 0,
                            workspace_root: workspace_root.clone(),
                        };

                        // Add new server to pool FIRST (Blue-Green: new server is online)
                        ready.lock().await.push_back(pooled);

                        // THEN shutdown old server gracefully (Green server going offline)
                        if let Err(e) = old_server.server.shutdown().await {
                            warn!("Error shutting down old server {}: {}", old_server.id, e);
                        } else {
                            info!(
                                "Successfully replaced server {} with new server",
                                old_server.id
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Failed to spawn replacement server: {}", e);
                        // Fallback: Keep old server running if replacement fails
                        warn!(
                            "Keeping old server {} running due to replacement failure",
                            old_server.id
                        );
                        ready.lock().await.push_back(old_server);
                    }
                }
            });
        } else {
            // Return to ready pool
            self.ready.lock().await.push_back(server);
        }
    }

    pub async fn shutdown(&self) {
        info!("Shutting down pool for {:?}", self.config.language);

        // Shutdown all ready servers
        let mut ready = self.ready.lock().await;
        while let Some(server) = ready.pop_front() {
            let _ = server.server.shutdown().await;
        }

        // Note: Busy servers will be shut down when returned
    }

    pub async fn get_stats(&self) -> PoolStats {
        let ready_count = self.ready.lock().await.len();
        let busy_count = self.busy.len();

        PoolStats {
            language: self.config.language,
            ready_servers: ready_count,
            busy_servers: busy_count,
            total_servers: ready_count + busy_count,
            min_size: self.min_size,
            max_size: self.max_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PoolStats {
    pub language: Language,
    pub ready_servers: usize,
    pub busy_servers: usize,
    pub total_servers: usize,
    #[allow(dead_code)]
    pub min_size: usize,
    #[allow(dead_code)]
    pub max_size: usize,
}

pub struct PoolManager {
    pools: Arc<DashMap<(Language, PathBuf), LspServerPool>>,
}

impl PoolManager {
    pub fn new() -> Self {
        Self {
            pools: Arc::new(DashMap::new()),
        }
    }

    pub async fn get_pool(
        &self,
        language: Language,
        workspace_root: PathBuf,
        config: LspServerConfig,
    ) -> LspServerPool {
        self.pools
            .entry((language, workspace_root.clone()))
            .or_insert_with(|| LspServerPool::new(config, workspace_root))
            .clone()
    }

    pub async fn shutdown_all(&self) {
        for pool in self.pools.iter() {
            pool.shutdown().await;
        }
        self.pools.clear();
    }

    pub async fn get_all_stats(&self) -> Vec<PoolStats> {
        let mut stats = Vec::new();
        for pool in self.pools.iter() {
            stats.push(pool.get_stats().await);
        }
        stats.sort_by_key(|s| s.language.as_str().to_string());
        stats
    }

    #[allow(dead_code)]
    pub async fn get_all_workspaces(&self) -> Vec<WorkspaceInfo> {
        let mut workspaces = Vec::new();
        for entry in self.pools.iter() {
            let (language, workspace_root) = entry.key();
            let pool = entry.value();
            let stats = pool.get_stats().await;

            let status = if stats.ready_servers > 0 {
                crate::protocol::ServerStatus::Ready
            } else if stats.busy_servers > 0 {
                crate::protocol::ServerStatus::Busy
            } else {
                crate::protocol::ServerStatus::Initializing
            };

            workspaces.push(WorkspaceInfo {
                root: workspace_root.clone(),
                language: *language,
                server_status: status,
                file_count: None, // Could be enhanced to actually count files
            });
        }
        workspaces.sort_by(|a, b| a.root.cmp(&b.root));
        workspaces
    }

    #[allow(dead_code)]
    pub async fn get_workspace_info(&self, workspace_root: &PathBuf) -> Vec<WorkspaceInfo> {
        let mut workspaces = Vec::new();
        for entry in self.pools.iter() {
            let (language, root) = entry.key();
            if root == workspace_root {
                let pool = entry.value();
                let stats = pool.get_stats().await;

                let status = if stats.ready_servers > 0 {
                    crate::protocol::ServerStatus::Ready
                } else if stats.busy_servers > 0 {
                    crate::protocol::ServerStatus::Busy
                } else {
                    crate::protocol::ServerStatus::Initializing
                };

                workspaces.push(WorkspaceInfo {
                    root: root.clone(),
                    language: *language,
                    server_status: status,
                    file_count: None,
                });
            }
        }
        workspaces
    }
}
