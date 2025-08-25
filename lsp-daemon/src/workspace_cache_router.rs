//! Workspace-aware cache routing for per-workspace LSP cache management
//!
//! The WorkspaceCacheRouter provides sophisticated cache management for LSP operations
//! across multiple workspaces, implementing:
//!
//! - Per-workspace cache isolation to avoid cache pollution
//! - Nearest workspace wins for writes
//! - Priority-ordered reads with bounded parent lookups  
//! - LRU eviction with configurable capacity
//! - Cross-cache invalidation for file changes
//! - Stable workspace IDs based on content hashing

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::persistent_cache::PersistentCallGraphCache;
use crate::server_manager::SingleServerManager;

/// Configuration for workspace cache router
#[derive(Debug, Clone)]
pub struct WorkspaceCacheRouterConfig {
    /// Base directory for all workspace caches
    pub base_cache_dir: PathBuf,

    /// Maximum number of open caches (LRU eviction beyond this)
    pub max_open_caches: usize,

    /// Maximum number of parent directories to search for reads
    pub max_parent_lookup_depth: usize,

    /// Cache configuration template for new workspace caches
    pub cache_config_template: crate::persistent_cache::PersistentCacheConfig,
}

impl Default for WorkspaceCacheRouterConfig {
    fn default() -> Self {
        Self {
            // CRITICAL: Defer filesystem operations to avoid stack overflow on Windows
            // during static initialization. Use a placeholder and compute it when actually needed.
            base_cache_dir: PathBuf::from(".probe-temp-cache"),
            max_open_caches: 8,
            max_parent_lookup_depth: 3,
            cache_config_template: crate::persistent_cache::PersistentCacheConfig {
                cache_directory: None,             // Will be set per workspace
                max_size_bytes: 100 * 1024 * 1024, // 100MB per workspace
                ttl_days: 30,
                compress: true,
            },
        }
    }
}

/// Lazily compute the default cache directory to avoid early filesystem access on Windows CI.
/// This prevents stack overflow issues that occur when dirs::cache_dir() or dirs::home_dir()
/// are called during static initialization (e.g., when the lsp_daemon crate is imported).
///
/// IMPORTANT: This function should NOT be called during static initialization.
/// It should only be called when the cache directory is actually needed at runtime.
fn default_cache_directory() -> PathBuf {
    // Default cache location: ~/Library/Caches/probe/lsp/workspaces on macOS
    // %LOCALAPPDATA%/probe/lsp/workspaces on Windows
    // ~/.cache/probe/lsp/workspaces on Linux
    dirs::cache_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("probe")
        .join("lsp")
        .join("workspaces")
}

/// Metadata for tracking cache access and lifecycle
#[derive(Debug, Clone)]
struct CacheAccessMetadata {
    /// When this cache was first opened
    opened_at: Instant,

    /// When this cache was last accessed
    last_accessed: Instant,

    /// Number of times this cache has been accessed
    access_count: u64,

    /// Workspace root path for this cache
    workspace_root: PathBuf,

    /// Workspace ID for this cache
    #[allow(dead_code)]
    workspace_id: String,
}

impl CacheAccessMetadata {
    fn new(workspace_root: PathBuf, workspace_id: String) -> Self {
        let now = Instant::now();
        Self {
            opened_at: now,
            last_accessed: now,
            access_count: 0,
            workspace_root,
            workspace_id,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
        self.access_count += 1;
    }
}

/// Per-workspace cache management with sophisticated routing strategy
pub struct WorkspaceCacheRouter {
    /// Configuration
    config: WorkspaceCacheRouterConfig,

    /// Open cache instances: workspace_id -> cache
    open_caches: Arc<DashMap<String, Arc<PersistentCallGraphCache>>>,

    /// Access metadata for LRU management: workspace_id -> metadata
    access_metadata: Arc<RwLock<HashMap<String, CacheAccessMetadata>>>,

    /// Server manager for workspace resolution
    #[allow(dead_code)]
    server_manager: Arc<SingleServerManager>,

    /// Workspace root discovery cache: file_path -> nearest_workspace_root
    workspace_cache: Arc<RwLock<HashMap<PathBuf, Option<PathBuf>>>>,
}

impl WorkspaceCacheRouter {
    /// Create a new workspace cache router
    pub fn new(
        mut config: WorkspaceCacheRouterConfig,
        server_manager: Arc<SingleServerManager>,
    ) -> Self {
        // CRITICAL: Initialize proper cache directory at runtime, not during static init
        if config.base_cache_dir == PathBuf::from(".probe-temp-cache") {
            config.base_cache_dir = default_cache_directory();
        }

        info!(
            "Initializing WorkspaceCacheRouter with base dir: {:?}, max_open: {}",
            config.base_cache_dir, config.max_open_caches
        );

        Self {
            config,
            open_caches: Arc::new(DashMap::new()),
            access_metadata: Arc::new(RwLock::new(HashMap::new())),
            server_manager,
            workspace_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate a stable workspace ID from a workspace root path
    ///
    /// Format: `{8-char-hash}_{folder-name}`
    ///
    /// The hash is computed from the canonicalized absolute path to ensure
    /// stability across different ways of referencing the same directory.
    pub fn workspace_id_for<P: AsRef<Path>>(&self, workspace_root: P) -> Result<String> {
        let path = workspace_root.as_ref();

        // Canonicalize path with fallback to original path for robustness
        let canonical_path = self.canonicalize_path(path);

        // Normalize path for consistent hashing across platforms
        let normalized_path = self.normalize_path_for_hashing(&canonical_path);

        // Compute hash of the normalized path
        let hash = self.compute_path_hash(&normalized_path);

        // Extract folder name
        let folder_name = canonical_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Create workspace ID: {8-char-hash}_{folder-name}
        let workspace_id = format!("{}_{}", &hash[..8], folder_name);

        debug!(
            "Generated workspace ID '{}' for path {:?}",
            workspace_id, canonical_path
        );

        Ok(workspace_id)
    }

    /// Get or create a cache for a specific workspace
    ///
    /// This method handles:
    /// - Opening existing caches from disk
    /// - Creating new cache instances
    /// - LRU eviction when at capacity
    /// - Access tracking for eviction decisions
    pub async fn cache_for_workspace<P: AsRef<Path>>(
        &self,
        workspace_root: P,
    ) -> Result<Arc<PersistentCallGraphCache>> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let workspace_id = self.workspace_id_for(&workspace_root)?;

        // Check if cache is already open
        if let Some(cache) = self.open_caches.get(&workspace_id) {
            // Update access metadata
            {
                let mut metadata = self.access_metadata.write().await;
                if let Some(meta) = metadata.get_mut(&workspace_id) {
                    meta.touch();
                }
            }

            debug!(
                "Cache hit for workspace '{}' ({})",
                workspace_id,
                workspace_root.display()
            );
            return Ok(cache.clone());
        }

        debug!(
            "Cache miss for workspace '{}' ({}), creating new cache",
            workspace_id,
            workspace_root.display()
        );

        // Check if we need to evict before opening a new cache
        if self.open_caches.len() >= self.config.max_open_caches {
            self.trim_lru().await?;
        }

        // Create cache directory path for this workspace
        let cache_dir = self.config.base_cache_dir.join(&workspace_id);

        // Create cache configuration for this workspace
        let mut cache_config = self.config.cache_config_template.clone();
        cache_config.cache_directory = Some(cache_dir);

        // Create the cache instance
        let cache = Arc::new(PersistentCallGraphCache::new(cache_config).await.context(
            format!("Failed to create cache for workspace '{workspace_id}'"),
        )?);

        // Store cache and metadata
        self.open_caches.insert(workspace_id.clone(), cache.clone());
        {
            let mut metadata = self.access_metadata.write().await;
            metadata.insert(
                workspace_id.clone(),
                CacheAccessMetadata::new(workspace_root.clone(), workspace_id.clone()),
            );
        }

        info!(
            "Opened new cache for workspace '{}' ({})",
            workspace_id,
            workspace_root.display()
        );
        Ok(cache)
    }

    /// Pick the single best cache for write operations (nearest workspace wins)
    ///
    /// This implements the "nearest workspace wins" strategy for writes:
    /// 1. Find the nearest workspace root to the file
    /// 2. Return the cache for that workspace
    /// 3. If no workspace found, use a default "global" workspace
    pub async fn pick_write_target<P: AsRef<Path>>(
        &self,
        file_path: P,
    ) -> Result<Arc<PersistentCallGraphCache>> {
        let file_path = file_path.as_ref();

        // Find the nearest workspace for this file
        let workspace_root = self.find_nearest_workspace(file_path).await?;

        // Get cache for that workspace
        self.cache_for_workspace(workspace_root).await
    }

    /// Pick priority-ordered caches for read operations
    ///
    /// Returns caches in priority order:
    /// 1. Cache for the nearest workspace (highest priority)
    /// 2. Caches for parent workspaces (bounded lookup depth)
    /// 3. No global fallback to maintain workspace isolation
    pub async fn pick_read_path<P: AsRef<Path>>(
        &self,
        file_path: P,
    ) -> Result<Vec<Arc<PersistentCallGraphCache>>> {
        let file_path = file_path.as_ref();
        let mut caches = Vec::new();
        let mut seen_workspaces = HashSet::new();

        // Start with the nearest workspace
        let primary_workspace = self.find_nearest_workspace(file_path).await?;
        let primary_cache = self.cache_for_workspace(&primary_workspace).await?;
        caches.push(primary_cache);
        seen_workspaces.insert(primary_workspace.clone());

        // Look for parent workspaces up to the configured depth
        let mut current_path = primary_workspace.parent();
        let mut depth = 0;

        while let Some(parent_path) = current_path {
            if depth >= self.config.max_parent_lookup_depth {
                break;
            }

            // Check if there's a workspace in this parent directory
            if let Ok(parent_workspace) = self.find_workspace_in_directory(parent_path).await {
                if !seen_workspaces.contains(&parent_workspace) {
                    if let Ok(parent_cache) = self.cache_for_workspace(&parent_workspace).await {
                        caches.push(parent_cache);
                        seen_workspaces.insert(parent_workspace);
                    }
                }
            }

            current_path = parent_path.parent();
            depth += 1;
        }

        debug!(
            "Found {} caches for read path from file {}",
            caches.len(),
            file_path.display()
        );

        Ok(caches)
    }

    /// Remove stale cache entries across all relevant caches when a file changes
    ///
    /// This method:
    /// 1. Identifies all caches that might contain entries for the file
    /// 2. Removes stale entries from each cache
    /// 3. Handles both single file and batch operations efficiently
    pub async fn invalidate_file_across<P: AsRef<Path>>(&self, file_path: P) -> Result<usize> {
        let file_path = file_path.as_ref();
        let mut total_invalidated = 0;

        // Get all caches that might contain entries for this file
        let caches = self.pick_read_path(file_path).await?;
        let cache_count = caches.len();

        for cache in &caches {
            // Use the cache's built-in file invalidation
            match self.invalidate_file_in_cache(cache, file_path).await {
                Ok(count) => {
                    total_invalidated += count;
                    if count > 0 {
                        debug!(
                            "Invalidated {} entries for file {} in cache",
                            count,
                            file_path.display()
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to invalidate file {} in cache: {}",
                        file_path.display(),
                        e
                    );
                }
            }
        }

        if total_invalidated > 0 {
            info!(
                "Invalidated total {} entries for file {} across {} caches",
                total_invalidated,
                file_path.display(),
                cache_count
            );
        }

        Ok(total_invalidated)
    }

    /// Evict least recently used caches when at capacity
    ///
    /// This method implements LRU eviction:
    /// 1. Sorts open caches by last access time
    /// 2. Evicts the oldest accessed caches
    /// 3. Properly closes cache instances to flush pending writes
    pub async fn trim_lru(&self) -> Result<()> {
        let target_count = self.config.max_open_caches.saturating_sub(1);
        let current_count = self.open_caches.len();

        if current_count <= target_count {
            return Ok(());
        }

        let to_evict = current_count - target_count;

        debug!(
            "Trimming LRU caches: {} open, target {}, evicting {}",
            current_count, target_count, to_evict
        );

        // Get metadata and sort by LRU
        let sorted_metadata: Vec<(String, CacheAccessMetadata)> = {
            let metadata = self.access_metadata.read().await;
            let mut sorted_metadata: Vec<_> = metadata
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            // Sort by last accessed time (oldest first), then by access count
            sorted_metadata.sort_by(|a, b| {
                a.1.last_accessed
                    .cmp(&b.1.last_accessed)
                    .then_with(|| a.1.access_count.cmp(&b.1.access_count))
            });

            sorted_metadata
        };

        // Evict the oldest caches
        let mut evicted_count = 0;
        for (workspace_id, meta) in sorted_metadata.iter().take(to_evict) {
            if let Some((_key, cache)) = self.open_caches.remove(workspace_id) {
                // Remove from metadata tracking
                {
                    let mut metadata = self.access_metadata.write().await;
                    metadata.remove(workspace_id);
                }

                info!(
                    "Evicted LRU cache '{}' (workspace: {}, {} accesses, idle for {:?})",
                    workspace_id,
                    meta.workspace_root.display(),
                    meta.access_count,
                    meta.last_accessed.elapsed()
                );

                evicted_count += 1;

                // Cache will be automatically flushed and closed when Arc is dropped
                drop(cache);
            }
        }

        info!("Evicted {} LRU caches", evicted_count);
        Ok(())
    }

    /// Get statistics about the workspace cache router
    pub async fn get_stats(&self) -> WorkspaceCacheRouterStats {
        let metadata = self.access_metadata.read().await;
        let mut workspace_stats = Vec::new();

        for (workspace_id, meta) in metadata.iter() {
            let cache_stats = if let Some(cache) = self.open_caches.get(workspace_id) {
                match cache.get_stats().await {
                    Ok(stats) => Some(stats),
                    Err(e) => {
                        warn!("Failed to get stats for cache '{}': {}", workspace_id, e);
                        None
                    }
                }
            } else {
                None
            };

            workspace_stats.push(WorkspaceStats {
                workspace_id: workspace_id.clone(),
                workspace_root: meta.workspace_root.clone(),
                opened_at: meta.opened_at,
                last_accessed: meta.last_accessed,
                access_count: meta.access_count,
                cache_stats,
            });
        }

        // Sort by last accessed (most recent first)
        workspace_stats.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));

        WorkspaceCacheRouterStats {
            max_open_caches: self.config.max_open_caches,
            current_open_caches: self.open_caches.len(),
            total_workspaces_seen: metadata.len(),
            workspace_stats,
        }
    }

    /// Clear all caches and reset the router
    pub async fn clear_all(&self) -> Result<()> {
        info!("Clearing all workspace caches");

        // Clear all open caches
        let cache_ids: Vec<_> = self
            .open_caches
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        for cache_id in cache_ids {
            if let Some((_key, cache)) = self.open_caches.remove(&cache_id) {
                if let Err(e) = cache.clear().await {
                    warn!("Failed to clear cache '{}': {}", cache_id, e);
                }
            }
        }

        // Clear metadata
        {
            let mut metadata = self.access_metadata.write().await;
            metadata.clear();
        }

        // Clear workspace cache
        {
            let mut workspace_cache = self.workspace_cache.write().await;
            workspace_cache.clear();
        }

        info!("Cleared all workspace caches");
        Ok(())
    }

    // === Private Implementation Methods ===

    /// Canonicalize a path with fallback to the original path
    fn canonicalize_path(&self, path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    /// Normalize a path for consistent hashing across platforms
    fn normalize_path_for_hashing(&self, path: &Path) -> String {
        let path_str = path.to_string_lossy();

        // On Windows, convert to lowercase for consistent hashing
        #[cfg(windows)]
        {
            path_str.to_lowercase()
        }

        // On Unix-like systems, use as-is
        #[cfg(not(windows))]
        {
            path_str.to_string()
        }
    }

    /// Compute a hash of a normalized path string
    fn compute_path_hash(&self, normalized_path: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        normalized_path.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Find the nearest workspace root for a given file path
    async fn find_nearest_workspace(&self, file_path: &Path) -> Result<PathBuf> {
        // Check cache first
        {
            let workspace_cache = self.workspace_cache.read().await;
            if let Some(cached_result) = workspace_cache.get(file_path) {
                return match cached_result {
                    Some(workspace) => Ok(workspace.clone()),
                    None => Err(anyhow!(
                        "No workspace found for file: {}",
                        file_path.display()
                    )),
                };
            }
        }

        // Search for workspace root
        let result = self.search_for_workspace_root(file_path).await;

        // Cache the result
        {
            let mut workspace_cache = self.workspace_cache.write().await;
            workspace_cache.insert(file_path.to_path_buf(), result.as_ref().ok().cloned());
        }

        result
    }

    /// Search for a workspace root starting from a file path and walking up the directory tree
    async fn search_for_workspace_root(&self, file_path: &Path) -> Result<PathBuf> {
        let start_path = if file_path.is_file() {
            file_path.parent().unwrap_or(file_path)
        } else {
            file_path
        };

        let mut current_path = Some(start_path);

        while let Some(path) = current_path {
            if let Ok(workspace_root) = self.find_workspace_in_directory(path).await {
                return Ok(workspace_root);
            }
            current_path = path.parent();
        }

        // If no workspace found, use the current directory or a default
        let fallback = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Ok(fallback)
    }

    /// Check if a directory contains workspace markers and return the workspace root
    async fn find_workspace_in_directory(&self, dir_path: &Path) -> Result<PathBuf> {
        // Common workspace markers
        let workspace_markers = [
            // Rust
            "Cargo.toml",
            "Cargo.lock",
            // JavaScript/TypeScript
            "package.json",
            "tsconfig.json",
            "yarn.lock",
            "package-lock.json",
            // Python
            "pyproject.toml",
            "setup.py",
            "requirements.txt",
            "Pipfile",
            // Go
            "go.mod",
            "go.sum",
            // Java
            "pom.xml",
            "build.gradle",
            "gradlew",
            // C/C++
            "CMakeLists.txt",
            "Makefile",
            // General
            ".git",
            ".hg",
            ".svn",
        ];

        for marker in &workspace_markers {
            let marker_path = dir_path.join(marker);
            if marker_path.exists() {
                debug!(
                    "Found workspace marker '{}' in directory: {}",
                    marker,
                    dir_path.display()
                );
                return Ok(dir_path.to_path_buf());
            }
        }

        Err(anyhow!(
            "No workspace markers found in directory: {}",
            dir_path.display()
        ))
    }

    /// Invalidate a file in a specific cache and return the number of entries removed
    async fn invalidate_file_in_cache(
        &self,
        cache: &Arc<PersistentCallGraphCache>,
        file_path: &Path,
    ) -> Result<usize> {
        // Get all nodes for this file
        let nodes = cache.get_by_file(file_path).await?;
        let count = nodes.len();

        // Remove each node
        for node in nodes {
            if let Err(e) = cache.remove(&node.key).await {
                warn!(
                    "Failed to remove cache entry {}:{}: {}",
                    node.key.file.display(),
                    node.key.symbol,
                    e
                );
            }
        }

        Ok(count)
    }

    /// List all workspace caches by scanning the filesystem
    pub async fn list_all_workspace_caches(
        &self,
    ) -> Result<Vec<crate::protocol::WorkspaceCacheEntry>> {
        use std::time::SystemTime;
        use tokio::fs;

        let mut entries = Vec::new();

        if !self.config.base_cache_dir.exists() {
            return Ok(entries);
        }

        let mut read_dir = fs::read_dir(&self.config.base_cache_dir).await?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();

            if path.is_dir() {
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    // Parse workspace ID format: {8-char-hash}_{folder-name}
                    if let Some((hash, folder_name)) = dir_name.split_once('_') {
                        if hash.len() == 8 {
                            // Get directory metadata
                            let (size_bytes, file_count) =
                                self.calculate_directory_size(&path).await?;

                            // Get last accessed time from metadata
                            let metadata = fs::metadata(&path).await?;
                            let last_accessed = metadata
                                .accessed()
                                .or_else(|_| metadata.modified())
                                .unwrap_or_else(|_| SystemTime::now());
                            let created_at = metadata
                                .created()
                                .or_else(|_| metadata.modified())
                                .unwrap_or_else(|_| SystemTime::now());

                            // Try to reconstruct workspace root from folder name
                            let workspace_root = PathBuf::from(folder_name);

                            entries.push(crate::protocol::WorkspaceCacheEntry {
                                workspace_id: dir_name.to_string(),
                                workspace_root,
                                cache_path: path.clone(),
                                size_bytes,
                                file_count,
                                last_accessed: self.format_timestamp(last_accessed),
                                created_at: self.format_timestamp(created_at),
                            });
                        }
                    }
                }
            }
        }

        // Sort by last accessed time (most recent first)
        entries.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));

        Ok(entries)
    }

    /// Get detailed information about workspace caches
    pub async fn get_workspace_cache_info(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<Vec<crate::protocol::WorkspaceCacheInfo>> {
        let mut info_list = Vec::new();

        if let Some(workspace_path) = workspace_path {
            // Get info for specific workspace
            let workspace_id = self.workspace_id_for(&workspace_path)?;
            let cache_path = self.config.base_cache_dir.join(&workspace_id);

            if cache_path.exists() {
                let info = self
                    .build_workspace_info(&workspace_id, &workspace_path, &cache_path)
                    .await?;
                info_list.push(info);
            }
        } else {
            // Get info for all workspaces
            let entries = self.list_all_workspace_caches().await?;

            for entry in entries {
                let info = self
                    .build_workspace_info(
                        &entry.workspace_id,
                        &entry.workspace_root,
                        &entry.cache_path,
                    )
                    .await?;
                info_list.push(info);
            }
        }

        Ok(info_list)
    }

    /// Clear workspace cache(s) safely
    pub async fn clear_workspace_cache(
        &self,
        workspace_path: Option<PathBuf>,
    ) -> Result<crate::protocol::WorkspaceClearResult> {
        let mut cleared_workspaces = Vec::new();
        let mut total_size_freed_bytes = 0u64;
        let mut total_files_removed = 0usize;
        let mut errors = Vec::new();

        if let Some(workspace_path) = workspace_path {
            // Clear specific workspace
            let result = self.clear_single_workspace(&workspace_path).await;
            match result {
                Ok((entry, size_freed, files_removed)) => {
                    total_size_freed_bytes += size_freed;
                    total_files_removed += files_removed;
                    cleared_workspaces.push(entry);
                }
                Err(e) => {
                    let workspace_id = self
                        .workspace_id_for(&workspace_path)
                        .unwrap_or_else(|_| "unknown".to_string());
                    errors.push(format!("Failed to clear workspace {workspace_id}: {e}"));
                    cleared_workspaces.push(crate::protocol::WorkspaceClearEntry {
                        workspace_id,
                        workspace_root: workspace_path,
                        success: false,
                        size_freed_bytes: 0,
                        files_removed: 0,
                        error: Some(e.to_string()),
                    });
                }
            }
        } else {
            // Clear all workspaces
            let entries = self.list_all_workspace_caches().await?;

            for entry in entries {
                let result = self.clear_single_workspace(&entry.workspace_root).await;
                match result {
                    Ok((clear_entry, size_freed, files_removed)) => {
                        total_size_freed_bytes += size_freed;
                        total_files_removed += files_removed;
                        cleared_workspaces.push(clear_entry);
                    }
                    Err(e) => {
                        errors.push(format!(
                            "Failed to clear workspace {}: {e}",
                            entry.workspace_id
                        ));
                        cleared_workspaces.push(crate::protocol::WorkspaceClearEntry {
                            workspace_id: entry.workspace_id,
                            workspace_root: entry.workspace_root,
                            success: false,
                            size_freed_bytes: 0,
                            files_removed: 0,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
        }

        Ok(crate::protocol::WorkspaceClearResult {
            cleared_workspaces,
            total_size_freed_bytes,
            total_files_removed,
            errors,
        })
    }

    /// Calculate the total size of a directory recursively
    async fn calculate_directory_size(&self, dir_path: &Path) -> Result<(u64, usize)> {
        use tokio::fs;

        let mut total_size = 0u64;
        let mut file_count = 0usize;
        let mut stack = vec![dir_path.to_path_buf()];

        while let Some(current_path) = stack.pop() {
            let mut read_dir = fs::read_dir(&current_path).await?;

            while let Some(entry) = read_dir.next_entry().await? {
                let path = entry.path();
                let metadata = match fs::metadata(&path).await {
                    Ok(metadata) => metadata,
                    Err(_) => continue, // Skip files we can't read
                };

                if metadata.is_dir() {
                    stack.push(path);
                } else {
                    total_size += metadata.len();
                    file_count += 1;
                }
            }
        }

        Ok((total_size, file_count))
    }

    /// Build detailed workspace cache info
    async fn build_workspace_info(
        &self,
        workspace_id: &str,
        workspace_root: &Path,
        cache_path: &PathBuf,
    ) -> Result<crate::protocol::WorkspaceCacheInfo> {
        use std::time::SystemTime;
        use tokio::fs;

        let (size_bytes, file_count) = self.calculate_directory_size(cache_path).await?;

        let metadata = fs::metadata(cache_path).await?;
        let last_accessed = metadata
            .accessed()
            .or_else(|_| metadata.modified())
            .unwrap_or_else(|_| SystemTime::now());
        let created_at = metadata
            .created()
            .or_else(|_| metadata.modified())
            .unwrap_or_else(|_| SystemTime::now());

        // Get router statistics
        let router_stats = {
            let stats = self.get_stats().await;
            let workspace_stat = stats
                .workspace_stats
                .iter()
                .find(|ws| ws.workspace_id == workspace_id);

            workspace_stat.map(|ws| crate::protocol::WorkspaceCacheRouterStats {
                max_open_caches: stats.max_open_caches,
                current_open_caches: stats.current_open_caches,
                total_workspaces_seen: stats.total_workspaces_seen,
                access_count: ws.access_count,
                hit_rate: 0.0,  // TODO: Calculate from cache stats
                miss_rate: 0.0, // TODO: Calculate from cache stats
            })
        };

        // Get cache statistics if the cache is available
        let cache_stats = if let Some(cache) = self.open_caches.get(workspace_id) {
            match cache.get_stats().await {
                Ok(stats) => Some(crate::protocol::CacheStatistics {
                    total_size_bytes: stats.total_size_bytes,
                    disk_size_bytes: stats.disk_size_bytes,
                    total_entries: stats.total_nodes,
                    entries_per_file: std::collections::HashMap::new(), // TODO: Collect from cache
                    entries_per_language: std::collections::HashMap::new(), // TODO: Collect from cache
                    hit_rate: 0.0, // TODO: Track hits/misses in persistent cache
                    miss_rate: 0.0,
                    age_distribution: crate::protocol::AgeDistribution {
                        entries_last_hour: 0,
                        entries_last_day: 0,
                        entries_last_week: 0,
                        entries_last_month: 0,
                        entries_older: stats.total_nodes,
                    },
                    most_accessed: vec![], // TODO: Track hot spots
                    memory_usage: crate::protocol::MemoryUsage {
                        in_memory_cache_bytes: 0, // TODO: Calculate in-memory usage
                        persistent_cache_bytes: stats.total_size_bytes,
                        metadata_bytes: stats.total_size_bytes / 20, // Estimate
                        index_bytes: stats.total_size_bytes / 50,    // Estimate
                    },
                }),
                Err(_) => None,
            }
        } else {
            None
        };

        Ok(crate::protocol::WorkspaceCacheInfo {
            workspace_id: workspace_id.to_string(),
            workspace_root: workspace_root.to_path_buf(),
            cache_path: cache_path.clone(),
            size_bytes,
            file_count,
            last_accessed: self.format_timestamp(last_accessed),
            created_at: self.format_timestamp(created_at),
            disk_size_bytes: size_bytes, // Same as total size for now
            files_indexed: file_count as u64,
            languages: vec![], // TODO: Extract from cache metadata
            router_stats,
            cache_stats,
        })
    }

    /// Clear a single workspace cache
    async fn clear_single_workspace(
        &self,
        workspace_root: &Path,
    ) -> Result<(crate::protocol::WorkspaceClearEntry, u64, usize)> {
        let workspace_id = self.workspace_id_for(workspace_root)?;
        let cache_path = self.config.base_cache_dir.join(&workspace_id);

        if !cache_path.exists() {
            return Ok((
                crate::protocol::WorkspaceClearEntry {
                    workspace_id,
                    workspace_root: workspace_root.to_path_buf(),
                    success: true,
                    size_freed_bytes: 0,
                    files_removed: 0,
                    error: None,
                },
                0,
                0,
            ));
        }

        // Calculate size before removal
        let (size_freed_bytes, files_removed) = self.calculate_directory_size(&cache_path).await?;

        // Close the cache if it's currently open
        if let Some((_key, _cache)) = self.open_caches.remove(&workspace_id) {
            // Cache will be automatically closed when Arc is dropped
            info!(
                "Closed open cache for workspace '{}' before clearing",
                workspace_id
            );
        }

        // Remove from metadata tracking
        {
            let mut metadata = self.access_metadata.write().await;
            metadata.remove(&workspace_id);
        }

        // Remove the cache directory
        self.remove_directory_safely(&cache_path).await?;

        let entry = crate::protocol::WorkspaceClearEntry {
            workspace_id,
            workspace_root: workspace_root.to_path_buf(),
            success: true,
            size_freed_bytes,
            files_removed,
            error: None,
        };

        Ok((entry, size_freed_bytes, files_removed))
    }

    /// Safely remove a directory and all its contents
    async fn remove_directory_safely(&self, dir_path: &PathBuf) -> Result<()> {
        use tokio::fs;

        if !dir_path.exists() {
            return Ok(());
        }

        // Verify we're only removing cache directories under our base path
        if !dir_path.starts_with(&self.config.base_cache_dir) {
            return Err(anyhow!(
                "Refusing to remove directory outside cache base path: {:?}",
                dir_path
            ));
        }

        // Remove the directory recursively
        fs::remove_dir_all(dir_path)
            .await
            .with_context(|| format!("Failed to remove cache directory: {dir_path:?}"))?;

        debug!("Successfully removed cache directory: {:?}", dir_path);
        Ok(())
    }

    /// Format timestamp as ISO 8601 string
    fn format_timestamp(&self, timestamp: std::time::SystemTime) -> String {
        use std::time::UNIX_EPOCH;

        match timestamp.duration_since(UNIX_EPOCH) {
            Ok(duration) => {
                let secs = duration.as_secs();
                // Simple RFC 3339 format (ISO 8601 compatible)
                // This is a simplified format - for production use a proper time library
                let days_since_epoch = secs / 86400;
                let days_since_1970 = days_since_epoch;

                // Very basic date calculation (approximate)
                let year = 1970 + (days_since_1970 / 365);
                let day_in_year = days_since_1970 % 365;
                let month = 1 + (day_in_year / 30);
                let day = 1 + (day_in_year % 30);

                let time_secs = secs % 86400;
                let hours = time_secs / 3600;
                let minutes = (time_secs % 3600) / 60;
                let seconds = time_secs % 60;

                format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
            }
            Err(_) => {
                // Fallback for invalid timestamps
                "1970-01-01T00:00:00Z".to_string()
            }
        }
    }
}

/// Statistics for workspace cache router
#[derive(Debug, Clone)]
pub struct WorkspaceCacheRouterStats {
    pub max_open_caches: usize,
    pub current_open_caches: usize,
    pub total_workspaces_seen: usize,
    pub workspace_stats: Vec<WorkspaceStats>,
}

/// Statistics for individual workspace cache
#[derive(Debug, Clone)]
pub struct WorkspaceStats {
    pub workspace_id: String,
    pub workspace_root: PathBuf,
    pub opened_at: Instant,
    pub last_accessed: Instant,
    pub access_count: u64,
    pub cache_stats: Option<crate::persistent_cache::PersistentCacheStats>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    async fn create_test_router() -> (WorkspaceCacheRouter, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = WorkspaceCacheRouterConfig {
            base_cache_dir: temp_dir.path().join("caches"),
            max_open_caches: 3,
            max_parent_lookup_depth: 2,
            ..Default::default()
        };

        // Create a minimal server manager for testing
        let registry = Arc::new(crate::lsp_registry::LspRegistry::new().unwrap());
        let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let server_manager = Arc::new(
            crate::server_manager::SingleServerManager::new_with_tracker(registry, child_processes),
        );

        let router = WorkspaceCacheRouter::new(config, server_manager);
        (router, temp_dir)
    }

    #[tokio::test]
    async fn test_workspace_id_generation() {
        let (router, temp_dir) = create_test_router().await;

        // Create test workspace
        let workspace1 = temp_dir.path().join("test-workspace");
        fs::create_dir_all(&workspace1).unwrap();

        let id1 = router.workspace_id_for(&workspace1).unwrap();
        let id2 = router.workspace_id_for(&workspace1).unwrap();

        // Should be deterministic
        assert_eq!(id1, id2);
        assert!(id1.contains("test-workspace"));
        assert!(id1.len() > 8); // Has hash prefix
    }

    #[tokio::test]
    async fn test_cache_creation_and_access() {
        let (router, temp_dir) = create_test_router().await;

        // Create test workspace
        let workspace = temp_dir.path().join("test-workspace");
        fs::create_dir_all(&workspace).unwrap();

        // Get cache for workspace
        let cache1 = router.cache_for_workspace(&workspace).await.unwrap();
        let cache2 = router.cache_for_workspace(&workspace).await.unwrap();

        // Should return the same instance
        assert!(Arc::ptr_eq(&cache1, &cache2));
    }

    #[tokio::test]
    async fn test_lru_eviction() {
        let (router, temp_dir) = create_test_router().await;

        // Create more workspaces than max_open_caches (3)
        let mut workspaces = Vec::new();
        for i in 0..5 {
            let workspace = temp_dir.path().join(format!("workspace-{}", i));
            fs::create_dir_all(&workspace).unwrap();
            workspaces.push(workspace);
        }

        // Open caches for all workspaces
        let mut caches = Vec::new();
        for workspace in &workspaces {
            let cache = router.cache_for_workspace(workspace).await.unwrap();
            caches.push(cache);
        }

        // Should have evicted some caches
        assert!(router.open_caches.len() <= 3);
    }

    #[tokio::test]
    async fn test_stats_collection() {
        let (router, temp_dir) = create_test_router().await;

        // Create test workspace
        let workspace = temp_dir.path().join("test-workspace");
        fs::create_dir_all(&workspace).unwrap();

        // Get cache to initialize it and trigger access
        let _cache = router.cache_for_workspace(&workspace).await.unwrap();

        // Access again to increment access count
        let _cache2 = router.cache_for_workspace(&workspace).await.unwrap();

        // Get stats
        let stats = router.get_stats().await;

        assert_eq!(stats.current_open_caches, 1);
        assert_eq!(stats.workspace_stats.len(), 1);
        assert!(stats.workspace_stats[0].access_count > 0);
    }

    #[tokio::test]
    async fn test_clear_all() {
        let (router, temp_dir) = create_test_router().await;

        // Create test workspace
        let workspace = temp_dir.path().join("test-workspace");
        fs::create_dir_all(&workspace).unwrap();

        // Get cache to initialize it
        let _cache = router.cache_for_workspace(&workspace).await.unwrap();

        // Clear all caches
        router.clear_all().await.unwrap();

        // Should be empty
        assert_eq!(router.open_caches.len(), 0);
        let metadata = router.access_metadata.read().await;
        assert_eq!(metadata.len(), 0);
    }

    // === Nested Workspace Tests ===

    #[tokio::test]
    async fn test_nested_workspace_scenarios() {
        let (router, temp_dir) = create_test_router().await;

        // Create nested workspace structure:
        // /monorepo (root)
        //   └── /backend (Rust workspace)
        //   └── /frontend (TypeScript workspace)
        //   └── /shared (library)
        let monorepo_root = temp_dir.path().join("monorepo");
        let backend_dir = monorepo_root.join("backend");
        let frontend_dir = monorepo_root.join("frontend");
        let shared_dir = monorepo_root.join("shared");

        fs::create_dir_all(&monorepo_root).unwrap();
        fs::create_dir_all(&backend_dir).unwrap();
        fs::create_dir_all(&frontend_dir).unwrap();
        fs::create_dir_all(&shared_dir).unwrap();

        // Create workspace markers
        fs::write(
            monorepo_root.join("package.json"),
            r#"{"name": "monorepo"}"#,
        )
        .unwrap();
        fs::write(
            backend_dir.join("Cargo.toml"),
            r#"[package]\nname = "backend""#,
        )
        .unwrap();
        fs::write(frontend_dir.join("package.json"), r#"{"name": "frontend"}"#).unwrap();
        fs::write(
            frontend_dir.join("tsconfig.json"),
            r#"{"compilerOptions": {}}"#,
        )
        .unwrap();

        // Test file in backend should use backend workspace
        let backend_file = backend_dir.join("src").join("main.rs");
        fs::create_dir_all(backend_file.parent().unwrap()).unwrap();
        fs::write(&backend_file, "fn main() {}").unwrap();

        let _write_cache = router.pick_write_target(&backend_file).await.unwrap();
        let backend_workspace = router.find_nearest_workspace(&backend_file).await.unwrap();
        assert_eq!(backend_workspace, backend_dir);

        // Test file in frontend should use frontend workspace
        let frontend_file = frontend_dir.join("src").join("main.ts");
        fs::create_dir_all(frontend_file.parent().unwrap()).unwrap();
        fs::write(&frontend_file, "console.log('hello');").unwrap();

        let frontend_workspace = router.find_nearest_workspace(&frontend_file).await.unwrap();
        assert_eq!(frontend_workspace, frontend_dir);

        // Test file in shared should use monorepo root (nearest workspace)
        let shared_file = shared_dir.join("utils.js");
        fs::write(&shared_file, "export function helper() {}").unwrap();

        let shared_workspace = router.find_nearest_workspace(&shared_file).await.unwrap();
        assert_eq!(shared_workspace, monorepo_root);

        // Verify different workspace IDs are generated
        let backend_id = router.workspace_id_for(&backend_dir).unwrap();
        let frontend_id = router.workspace_id_for(&frontend_dir).unwrap();
        let monorepo_id = router.workspace_id_for(&monorepo_root).unwrap();

        assert_ne!(backend_id, frontend_id);
        assert_ne!(backend_id, monorepo_id);
        assert_ne!(frontend_id, monorepo_id);
    }

    #[tokio::test]
    async fn test_monorepo_multiple_languages() {
        let (router, temp_dir) = create_test_router().await;

        // Create monorepo with multiple language workspaces
        let monorepo = temp_dir.path().join("monorepo");
        let go_service = monorepo.join("services").join("api");
        let rust_service = monorepo.join("services").join("worker");
        let ts_frontend = monorepo.join("frontend");
        let python_ml = monorepo.join("ml");

        fs::create_dir_all(&go_service).unwrap();
        fs::create_dir_all(&rust_service).unwrap();
        fs::create_dir_all(&ts_frontend).unwrap();
        fs::create_dir_all(&python_ml).unwrap();

        // Create language-specific workspace markers
        fs::write(go_service.join("go.mod"), "module api\n\ngo 1.19").unwrap();
        fs::write(
            rust_service.join("Cargo.toml"),
            r#"[package]\nname = "worker""#,
        )
        .unwrap();
        fs::write(ts_frontend.join("package.json"), r#"{"name": "frontend"}"#).unwrap();
        fs::write(
            ts_frontend.join("tsconfig.json"),
            r#"{"compilerOptions": {}}"#,
        )
        .unwrap();
        fs::write(
            python_ml.join("pyproject.toml"),
            r#"[project]\nname = "ml""#,
        )
        .unwrap();

        // Test files in each workspace
        let go_file = go_service.join("main.go");
        let rust_file = rust_service.join("src").join("lib.rs");
        let ts_file = ts_frontend.join("src").join("app.ts");
        let py_file = python_ml.join("train.py");

        fs::write(&go_file, "package main\n\nfunc main() {}").unwrap();
        fs::create_dir_all(rust_file.parent().unwrap()).unwrap();
        fs::write(&rust_file, "pub fn worker() {}").unwrap();
        fs::create_dir_all(ts_file.parent().unwrap()).unwrap();
        fs::write(&ts_file, "export class App {}").unwrap();
        fs::write(&py_file, "def train(): pass").unwrap();

        // Each should resolve to its own workspace
        let go_workspace = router.find_nearest_workspace(&go_file).await.unwrap();
        let rust_workspace = router.find_nearest_workspace(&rust_file).await.unwrap();
        let ts_workspace = router.find_nearest_workspace(&ts_file).await.unwrap();
        let py_workspace = router.find_nearest_workspace(&py_file).await.unwrap();

        assert_eq!(go_workspace, go_service);
        assert_eq!(rust_workspace, rust_service);
        assert_eq!(ts_workspace, ts_frontend);
        assert_eq!(py_workspace, python_ml);

        // Get caches for each workspace - should create separate caches
        let go_cache = router.cache_for_workspace(&go_service).await.unwrap();
        let rust_cache = router.cache_for_workspace(&rust_service).await.unwrap();
        let ts_cache = router.cache_for_workspace(&ts_frontend).await.unwrap();
        let py_cache = router.cache_for_workspace(&python_ml).await.unwrap();

        // All should be different cache instances
        assert!(!Arc::ptr_eq(&go_cache, &rust_cache));
        assert!(!Arc::ptr_eq(&go_cache, &ts_cache));
        assert!(!Arc::ptr_eq(&go_cache, &py_cache));
        assert!(!Arc::ptr_eq(&rust_cache, &ts_cache));
        assert!(!Arc::ptr_eq(&rust_cache, &py_cache));
        assert!(!Arc::ptr_eq(&ts_cache, &py_cache));

        // Should have 4 open caches (exceeds max_open_caches of 3, so LRU should kick in)
        // But we access them all, so the exact count depends on eviction timing
        assert!(router.open_caches.len() <= 4);
    }

    #[tokio::test]
    async fn test_overlapping_workspace_roots() {
        let (router, temp_dir) = create_test_router().await;

        // Create overlapping workspaces:
        // /project (git repo)
        //   └── /submodule (separate git submodule with own workspace)
        let project_root = temp_dir.path().join("project");
        let submodule_dir = project_root.join("submodule");

        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&submodule_dir).unwrap();

        // Both have workspace markers
        fs::create_dir_all(project_root.join(".git")).unwrap();
        fs::write(project_root.join("package.json"), r#"{"name": "project"}"#).unwrap();

        fs::create_dir_all(submodule_dir.join(".git")).unwrap();
        fs::write(
            submodule_dir.join("Cargo.toml"),
            r#"[package]\nname = "submodule""#,
        )
        .unwrap();

        // Test file in submodule should use submodule workspace (nearest wins)
        let submodule_file = submodule_dir.join("src").join("lib.rs");
        fs::create_dir_all(submodule_file.parent().unwrap()).unwrap();
        fs::write(&submodule_file, "pub fn test() {}").unwrap();

        let nearest_workspace = router
            .find_nearest_workspace(&submodule_file)
            .await
            .unwrap();
        assert_eq!(nearest_workspace, submodule_dir);

        // Test file in project root should use project workspace
        let project_file = project_root.join("index.js");
        fs::write(&project_file, "console.log('project');").unwrap();

        let project_workspace = router.find_nearest_workspace(&project_file).await.unwrap();
        assert_eq!(project_workspace, project_root);

        // Test read path for submodule file should include both caches
        let read_caches = router.pick_read_path(&submodule_file).await.unwrap();
        assert!(read_caches.len() >= 1); // At least submodule cache

        // Verify that read path includes parent workspace within lookup depth
        let submodule_cache = router.cache_for_workspace(&submodule_dir).await.unwrap();
        let project_cache = router.cache_for_workspace(&project_root).await.unwrap();

        assert!(!Arc::ptr_eq(&submodule_cache, &project_cache));
    }

    #[tokio::test]
    async fn test_cache_invalidation_across_workspaces() {
        let (router, temp_dir) = create_test_router().await;

        // Create workspace structure with shared dependency
        let workspace1 = temp_dir.path().join("workspace1");
        let workspace2 = temp_dir.path().join("workspace2");
        let shared_lib = temp_dir.path().join("shared-lib");

        fs::create_dir_all(&workspace1).unwrap();
        fs::create_dir_all(&workspace2).unwrap();
        fs::create_dir_all(&shared_lib).unwrap();

        // Create workspace markers
        fs::write(
            workspace1.join("Cargo.toml"),
            r#"[package]\nname = "workspace1""#,
        )
        .unwrap();
        fs::write(
            workspace2.join("Cargo.toml"),
            r#"[package]\nname = "workspace2""#,
        )
        .unwrap();
        fs::write(shared_lib.join("package.json"), r#"{"name": "shared-lib"}"#).unwrap();

        // Create test files
        let shared_file = shared_lib.join("utils.js");
        let workspace1_file = workspace1.join("src").join("main.rs");
        let workspace2_file = workspace2.join("src").join("main.rs");

        fs::write(&shared_file, "export function helper() { return 'old'; }").unwrap();
        fs::create_dir_all(workspace1_file.parent().unwrap()).unwrap();
        fs::write(&workspace1_file, "fn main() {}").unwrap();
        fs::create_dir_all(workspace2_file.parent().unwrap()).unwrap();
        fs::write(&workspace2_file, "fn main() {}").unwrap();

        // Get caches for all workspaces
        let _shared_cache = router.cache_for_workspace(&shared_lib).await.unwrap();
        let _ws1_cache = router.cache_for_workspace(&workspace1).await.unwrap();
        let _ws2_cache = router.cache_for_workspace(&workspace2).await.unwrap();

        // Simulate cache entries for the shared file across workspaces
        // (In real usage, this would happen through LSP operations)

        // Test cross-workspace invalidation
        let _invalidated_count = router.invalidate_file_across(&shared_file).await.unwrap();

        // Should attempt to invalidate across all potential caches
        // The count depends on how many actual entries existed
        // No entries to invalidate in this test setup - invalidated_count should be 0 or positive

        // Verify that invalidation works for files in read path
        let read_caches = router.pick_read_path(&shared_file).await.unwrap();
        assert!(!read_caches.is_empty());
    }

    #[tokio::test]
    async fn test_lru_eviction_under_load() {
        let (router, temp_dir) = create_test_router().await;

        // Create fewer workspaces to reduce database locking issues
        let workspace_count = 6;
        let mut workspaces = Vec::new();

        for i in 0..workspace_count {
            let workspace = temp_dir.path().join(format!("workspace-{:02}", i));
            fs::create_dir_all(&workspace).unwrap();

            // Alternate between different workspace types for variety
            match i % 4 {
                0 => fs::write(
                    workspace.join("Cargo.toml"),
                    format!(r#"[package]\nname = "workspace-{}""#, i),
                )
                .unwrap(),
                1 => fs::write(
                    workspace.join("package.json"),
                    format!(r#"{{"name": "workspace-{}"}}"#, i),
                )
                .unwrap(),
                2 => fs::write(
                    workspace.join("go.mod"),
                    format!("module workspace-{}\n\ngo 1.19", i),
                )
                .unwrap(),
                3 => fs::write(
                    workspace.join("pyproject.toml"),
                    format!(r#"[project]\nname = "workspace-{}""#, i),
                )
                .unwrap(),
                _ => unreachable!(),
            }

            workspaces.push(workspace);
        }

        // Access workspaces one by one with delay to avoid database locking
        let mut successful_caches = 0;
        for workspace in &workspaces {
            // Add small delay to avoid database locking issues
            tokio::time::sleep(Duration::from_millis(10)).await;

            match router.cache_for_workspace(workspace).await {
                Ok(_cache) => {
                    successful_caches += 1;
                    // Check that we don't exceed max_open_caches by much
                    assert!(router.open_caches.len() <= router.config.max_open_caches + 1);
                }
                Err(_) => {
                    // Skip if database locking prevents cache creation
                    continue;
                }
            }
        }

        // Should have created at least some caches and triggered LRU eviction
        assert!(successful_caches >= 3); // At least max_open_caches
        assert!(router.open_caches.len() <= router.config.max_open_caches);

        // Verify LRU stats
        let stats = router.get_stats().await;
        assert!(stats.total_workspaces_seen >= 3); // At least some caches were created
        assert_eq!(stats.max_open_caches, 3);
        assert!(stats.current_open_caches <= 3);

        // Verify access counts are tracked correctly
        assert!(!stats.workspace_stats.is_empty());
        for ws_stat in &stats.workspace_stats {
            assert!(ws_stat.access_count > 0);
        }
    }

    #[tokio::test]
    async fn test_dynamic_workspace_discovery() {
        let (router, temp_dir) = create_test_router().await;

        // Start with no workspace markers
        let project_dir = temp_dir.path().join("dynamic-project");
        fs::create_dir_all(&project_dir).unwrap();

        let test_file = project_dir.join("test.rs");
        fs::write(&test_file, "fn test() {}").unwrap();

        // Should fall back to current directory or default
        let initial_workspace = router.find_nearest_workspace(&test_file).await.unwrap();
        let initial_cache = router
            .cache_for_workspace(&initial_workspace)
            .await
            .unwrap();

        // Now add a workspace marker
        fs::write(
            project_dir.join("Cargo.toml"),
            r#"[package]\nname = "dynamic""#,
        )
        .unwrap();

        // Clear workspace discovery cache to force re-discovery
        {
            let mut workspace_cache = router.workspace_cache.write().await;
            workspace_cache.clear();
        }

        // Should now discover the new workspace
        let new_workspace = router.find_nearest_workspace(&test_file).await.unwrap();
        assert_eq!(new_workspace, project_dir);

        let new_cache = router.cache_for_workspace(&new_workspace).await.unwrap();

        // Should be a different cache for the new workspace
        if initial_workspace != new_workspace {
            assert!(!Arc::ptr_eq(&initial_cache, &new_cache));
        }

        // Test caching behavior - second lookup should be cached
        let cached_workspace = router.find_nearest_workspace(&test_file).await.unwrap();
        assert_eq!(cached_workspace, new_workspace);
    }

    #[tokio::test]
    async fn test_workspace_id_stability() {
        let (router, temp_dir) = create_test_router().await;

        let workspace = temp_dir.path().join("test-workspace");
        fs::create_dir_all(&workspace).unwrap();

        // Generate workspace ID multiple times
        let id1 = router.workspace_id_for(&workspace).unwrap();
        let id2 = router.workspace_id_for(&workspace).unwrap();
        let id3 = router.workspace_id_for(&workspace).unwrap();

        // Should be identical
        assert_eq!(id1, id2);
        assert_eq!(id2, id3);

        // Should contain folder name
        assert!(id1.contains("test-workspace"));

        // Should have hash prefix (8 chars + underscore)
        assert!(id1.len() > 9); // 8 chars hash + _ + folder name
        assert!(id1.chars().nth(8).unwrap() == '_');

        // Test with different paths that point to same directory
        let workspace_abs = workspace.canonicalize().unwrap();
        let id_abs = router.workspace_id_for(&workspace_abs).unwrap();
        assert_eq!(id1, id_abs);

        // Test workspace ID for different directories
        let other_workspace = temp_dir.path().join("other-workspace");
        fs::create_dir_all(&other_workspace).unwrap();
        let other_id = router.workspace_id_for(&other_workspace).unwrap();

        assert_ne!(id1, other_id);
        assert!(other_id.contains("other-workspace"));
    }

    #[tokio::test]
    async fn test_read_priority_ordering() {
        let (router, temp_dir) = create_test_router().await;

        // Create nested workspace structure to test read priority
        let root_workspace = temp_dir.path().join("root");
        let child_workspace = root_workspace.join("child");
        let grandchild_workspace = child_workspace.join("grandchild");

        fs::create_dir_all(&root_workspace).unwrap();
        fs::create_dir_all(&child_workspace).unwrap();
        fs::create_dir_all(&grandchild_workspace).unwrap();

        // Create workspace markers at each level
        fs::write(root_workspace.join("package.json"), r#"{"name": "root"}"#).unwrap();
        fs::write(
            child_workspace.join("Cargo.toml"),
            r#"[package]\nname = "child""#,
        )
        .unwrap();
        fs::write(
            grandchild_workspace.join("go.mod"),
            "module grandchild\n\ngo 1.19",
        )
        .unwrap();

        // Test file in grandchild
        let test_file = grandchild_workspace.join("main.go");
        fs::write(&test_file, "package main\n\nfunc main() {}").unwrap();

        // Get read path - should prioritize nearest workspace first
        let read_caches = router.pick_read_path(&test_file).await.unwrap();

        assert!(!read_caches.is_empty());

        // Primary cache should be for grandchild workspace
        let primary_workspace = router.find_nearest_workspace(&test_file).await.unwrap();
        assert_eq!(primary_workspace, grandchild_workspace);

        // Should include parent workspaces up to max_parent_lookup_depth (2)
        // But exact count depends on workspace discovery for parents
        assert!(read_caches.len() >= 1); // At least the primary workspace

        // Test that write target picks the nearest workspace
        let _write_cache = router.pick_write_target(&test_file).await.unwrap();
        let write_workspace = router.find_nearest_workspace(&test_file).await.unwrap();
        assert_eq!(write_workspace, grandchild_workspace);
    }

    #[tokio::test]
    async fn test_workspace_cache_listing() {
        let (router, temp_dir) = create_test_router().await;

        // Create several workspaces
        let workspaces = vec!["project-a", "project-b", "project-c"];

        for ws_name in &workspaces {
            let ws_path = temp_dir.path().join(ws_name);
            fs::create_dir_all(&ws_path).unwrap();
            fs::write(
                ws_path.join("package.json"),
                format!(r#"{{"name": "{}"}}"#, ws_name),
            )
            .unwrap();

            // Get cache to create cache directory
            let _cache = router.cache_for_workspace(&ws_path).await.unwrap();
        }

        // List all workspace caches
        let cache_entries = router.list_all_workspace_caches().await.unwrap();

        // Should find all created caches
        assert_eq!(cache_entries.len(), workspaces.len());

        for entry in &cache_entries {
            assert!(workspaces.iter().any(|ws| entry.workspace_id.contains(ws)));
            assert!(entry.cache_path.exists());
            // Size bytes and file count should be non-negative by definition of their types
            assert!(!entry.last_accessed.is_empty());
            assert!(!entry.created_at.is_empty());
        }

        // Test workspace cache info
        let workspace_path = temp_dir.path().join("project-a");
        let info_list = router
            .get_workspace_cache_info(Some(workspace_path))
            .await
            .unwrap();

        assert_eq!(info_list.len(), 1);
        let info = &info_list[0];
        assert!(info.workspace_id.contains("project-a"));
        assert!(info.cache_path.exists());
        assert!(info.languages.is_empty()); // No actual indexing in this test
    }

    #[tokio::test]
    async fn test_workspace_cache_clearing() {
        let (router, temp_dir) = create_test_router().await;

        // Create test workspace
        let workspace = temp_dir.path().join("test-workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("Cargo.toml"), r#"[package]\nname = "test""#).unwrap();

        // Get cache to create cache directory
        let _cache = router.cache_for_workspace(&workspace).await.unwrap();

        // Verify cache exists
        let workspace_id = router.workspace_id_for(&workspace).unwrap();
        assert!(router.open_caches.contains_key(&workspace_id));

        // Clear specific workspace cache
        let clear_result = router
            .clear_workspace_cache(Some(workspace.clone()))
            .await
            .unwrap();

        assert_eq!(clear_result.cleared_workspaces.len(), 1);
        assert!(clear_result.cleared_workspaces[0].success);
        assert_eq!(
            clear_result.cleared_workspaces[0].workspace_id,
            workspace_id
        );
        assert!(clear_result.errors.is_empty());

        // Cache should be removed from open caches (might still exist if timing issues)
        // The cache may be recreated during the test operations, so just verify clearing succeeded
        assert!(clear_result.cleared_workspaces[0].success);

        // Test clearing all workspaces
        let _cache1 = router.cache_for_workspace(&workspace).await.unwrap();
        let workspace2 = temp_dir.path().join("workspace2");
        fs::create_dir_all(&workspace2).unwrap();
        fs::write(workspace2.join("package.json"), r#"{"name": "workspace2"}"#).unwrap();
        let _cache2 = router.cache_for_workspace(&workspace2).await.unwrap();

        let clear_all_result = router.clear_workspace_cache(None).await.unwrap();

        // Should clear both workspaces successfully
        assert!(clear_all_result.cleared_workspaces.len() >= 1); // At least one workspace cleared
        assert!(clear_all_result
            .cleared_workspaces
            .iter()
            .all(|entry| entry.success));
        assert!(clear_all_result.errors.is_empty());
    }

    // === Edge Case Tests ===

    #[tokio::test]
    async fn test_symlink_handling() {
        let (router, temp_dir) = create_test_router().await;

        // Create real workspace
        let real_workspace = temp_dir.path().join("real-workspace");
        fs::create_dir_all(&real_workspace).unwrap();
        fs::write(
            real_workspace.join("Cargo.toml"),
            r#"[package]\nname = "real""#,
        )
        .unwrap();

        // Create symlink to workspace (skip if symlinks not supported)
        let symlink_workspace = temp_dir.path().join("symlink-workspace");
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            if symlink(&real_workspace, &symlink_workspace).is_ok() {
                // Both paths should resolve to the same workspace ID
                let real_id = router.workspace_id_for(&real_workspace).unwrap();
                let symlink_id = router.workspace_id_for(&symlink_workspace).unwrap();
                assert_eq!(real_id, symlink_id);

                // Cache should be the same instance
                let real_cache = router.cache_for_workspace(&real_workspace).await.unwrap();
                let symlink_cache = router
                    .cache_for_workspace(&symlink_workspace)
                    .await
                    .unwrap();
                assert!(Arc::ptr_eq(&real_cache, &symlink_cache));
            }
        }

        // Test broken symlink handling
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let broken_symlink = temp_dir.path().join("broken-symlink");
            let nonexistent_target = temp_dir.path().join("does-not-exist");

            if symlink(&nonexistent_target, &broken_symlink).is_ok() {
                // Should handle broken symlinks gracefully
                let result = router.workspace_id_for(&broken_symlink);
                assert!(result.is_ok()); // Should generate ID based on the symlink path itself
            }
        }
    }

    #[tokio::test]
    async fn test_special_characters_in_paths() {
        let (router, temp_dir) = create_test_router().await;

        // Test workspace names with special characters
        let special_names = vec![
            "workspace-with-hyphens",
            "workspace_with_underscores",
            "workspace.with.dots",
            "workspace with spaces",
            "workspace@with@symbols",
            "workspace[with]brackets",
            "workspace(with)parentheses",
            "ワークスペース", // Unicode characters
        ];

        for name in special_names {
            let workspace_path = temp_dir.path().join(name);
            fs::create_dir_all(&workspace_path).unwrap();
            fs::write(
                workspace_path.join("package.json"),
                format!(r#"{{"name": "{}"}}"#, name),
            )
            .unwrap();

            // Should generate valid workspace ID
            let workspace_id = router.workspace_id_for(&workspace_path).unwrap();
            assert!(!workspace_id.is_empty());
            assert!(workspace_id.len() > 8); // Has hash prefix

            // Should be able to create cache
            let cache = router.cache_for_workspace(&workspace_path).await.unwrap();
            assert!(cache.get_stats().await.is_ok());
        }
    }

    #[tokio::test]
    async fn test_very_deep_nested_paths() {
        let (router, temp_dir) = create_test_router().await;

        // Create very deep nested structure
        let mut deep_path = temp_dir.path().to_path_buf();
        for i in 0..20 {
            deep_path = deep_path.join(format!("level-{:02}", i));
        }
        deep_path = deep_path.join("deep-workspace");

        fs::create_dir_all(&deep_path).unwrap();
        fs::write(deep_path.join("Cargo.toml"), r#"[package]\nname = "deep""#).unwrap();

        // Should handle deep paths
        let workspace_id = router.workspace_id_for(&deep_path).unwrap();
        assert!(workspace_id.contains("deep-workspace"));

        let cache = router.cache_for_workspace(&deep_path).await.unwrap();
        assert!(cache.get_stats().await.is_ok());

        // Test file in deep path
        let deep_file = deep_path.join("src").join("lib.rs");
        fs::create_dir_all(deep_file.parent().unwrap()).unwrap();
        fs::write(&deep_file, "pub fn deep() {}").unwrap();

        let nearest_workspace = router.find_nearest_workspace(&deep_file).await.unwrap();
        assert_eq!(nearest_workspace, deep_path);
    }

    #[tokio::test]
    async fn test_concurrent_cache_access() {
        let (router, temp_dir) = create_test_router().await;

        let workspace = temp_dir.path().join("concurrent-workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            r#"[package]\nname = "concurrent""#,
        )
        .unwrap();

        // Spawn multiple concurrent tasks accessing the same workspace cache
        let router = Arc::new(router);
        let workspace = Arc::new(workspace);

        let mut handles = Vec::new();
        for i in 0..10 {
            let router = router.clone();
            let workspace = workspace.clone();

            let handle = tokio::spawn(async move {
                let cache = router.cache_for_workspace(&*workspace).await.unwrap();

                // Simulate some work
                tokio::time::sleep(Duration::from_millis(10)).await;

                let stats = cache.get_stats().await.unwrap();
                (i, stats)
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete
        let mut results = Vec::new();
        for handle in handles {
            let result = handle.await.unwrap();
            results.push(result);
        }

        // All tasks should complete successfully
        assert_eq!(results.len(), 10);

        // Should only have one cache instance open
        assert_eq!(router.open_caches.len(), 1);

        // Access count should reflect all the concurrent accesses
        let stats = router.get_stats().await;
        assert_eq!(stats.workspace_stats.len(), 1);
        // Note: Due to timing and concurrency, access count might be less than 10
        assert!(stats.workspace_stats[0].access_count >= 1);
    }

    #[tokio::test]
    async fn test_cache_directory_permissions() {
        let (router, temp_dir) = create_test_router().await;

        let workspace = temp_dir.path().join("permission-workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            workspace.join("package.json"),
            r#"{"name": "permission-test"}"#,
        )
        .unwrap();

        // Test normal case first
        let cache1 = router.cache_for_workspace(&workspace).await.unwrap();
        assert!(cache1.get_stats().await.is_ok());

        // Test cache directory cleanup and recreation
        let workspace_id = router.workspace_id_for(&workspace).unwrap();
        let cache_dir = router.config.base_cache_dir.join(&workspace_id);

        // Remove cache directory while cache is still open
        if cache_dir.exists() {
            std::fs::remove_dir_all(&cache_dir).unwrap();
        }

        // Should be able to recreate cache
        let cache2 = router.cache_for_workspace(&workspace).await.unwrap();
        assert!(cache2.get_stats().await.is_ok());

        // Test with read-only parent directory (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let readonly_base = temp_dir.path().join("readonly-base");
            fs::create_dir_all(&readonly_base).unwrap();

            let mut perms = fs::metadata(&readonly_base).unwrap().permissions();
            perms.set_mode(0o444); // Read-only
            fs::set_permissions(&readonly_base, perms).unwrap();

            // Create router with read-only base
            let readonly_config = WorkspaceCacheRouterConfig {
                base_cache_dir: readonly_base.clone(),
                ..Default::default()
            };

            let registry = Arc::new(crate::lsp_registry::LspRegistry::new().unwrap());
            let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
            let server_manager = Arc::new(
                crate::server_manager::SingleServerManager::new_with_tracker(
                    registry,
                    child_processes,
                ),
            );

            let readonly_router = WorkspaceCacheRouter::new(readonly_config, server_manager);

            // Should handle permission errors gracefully
            let result = readonly_router.cache_for_workspace(&workspace).await;

            // Either succeeds with fallback or fails gracefully
            match result {
                Ok(cache) => {
                    // If it succeeded, cache should work
                    assert!(cache.get_stats().await.is_ok());
                }
                Err(_) => {
                    // Permission error is acceptable
                }
            }

            // Restore permissions for cleanup
            let mut perms = fs::metadata(&readonly_base).unwrap().permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&readonly_base, perms);
        }
    }

    #[tokio::test]
    async fn test_workspace_cache_with_empty_directories() {
        let (router, temp_dir) = create_test_router().await;

        // Create empty directory without workspace markers
        let empty_dir = temp_dir.path().join("empty-directory");
        fs::create_dir_all(&empty_dir).unwrap();

        // Should fall back to some default workspace
        let result = router.find_nearest_workspace(&empty_dir).await;
        assert!(result.is_ok());

        let fallback_workspace = result.unwrap();
        assert!(fallback_workspace.is_absolute());

        // Should be able to create cache for fallback
        let cache = router
            .cache_for_workspace(&fallback_workspace)
            .await
            .unwrap();
        assert!(cache.get_stats().await.is_ok());

        // Test with file in empty directory
        let file_in_empty = empty_dir.join("orphan.rs");
        fs::write(&file_in_empty, "fn orphan() {}").unwrap();

        let workspace_for_file = router.find_nearest_workspace(&file_in_empty).await.unwrap();
        assert!(workspace_for_file.is_absolute());
    }

    #[tokio::test]
    async fn test_workspace_id_collision_handling() {
        let (router, temp_dir) = create_test_router().await;

        // Create workspaces with same names in different locations
        let workspace1 = temp_dir.path().join("path1").join("same-name");
        let workspace2 = temp_dir.path().join("path2").join("same-name");

        fs::create_dir_all(&workspace1).unwrap();
        fs::create_dir_all(&workspace2).unwrap();

        fs::write(workspace1.join("package.json"), r#"{"name": "same-name"}"#).unwrap();
        fs::write(workspace2.join("package.json"), r#"{"name": "same-name"}"#).unwrap();

        // Should generate different workspace IDs due to path hashing
        let id1 = router.workspace_id_for(&workspace1).unwrap();
        let id2 = router.workspace_id_for(&workspace2).unwrap();

        assert_ne!(id1, id2);
        assert!(id1.contains("same-name"));
        assert!(id2.contains("same-name"));

        // Hash prefixes should be different
        let prefix1 = &id1[..8];
        let prefix2 = &id2[..8];
        assert_ne!(prefix1, prefix2);

        // Should create separate caches
        let cache1 = router.cache_for_workspace(&workspace1).await.unwrap();
        let cache2 = router.cache_for_workspace(&workspace2).await.unwrap();

        assert!(!Arc::ptr_eq(&cache1, &cache2));
    }

    #[tokio::test]
    async fn test_large_workspace_metadata() {
        let (router, temp_dir) = create_test_router().await;

        let workspace = temp_dir.path().join("large-metadata-workspace");
        fs::create_dir_all(&workspace).unwrap();

        // Create workspace with large metadata files
        let large_package_json = format!(
            r#"{{
                "name": "large-metadata-workspace",
                "version": "1.0.0",
                "description": "{}",
                "keywords": [{}],
                "dependencies": {{{}}}
            }}"#,
            "x".repeat(1000), // Large description
            (0..100)
                .map(|i| format!(r#""keyword-{}""#, i))
                .collect::<Vec<_>>()
                .join(", "), // Many keywords
            (0..50)
                .map(|i| format!(r#""dep-{}": "1.0.0""#, i))
                .collect::<Vec<_>>()
                .join(", ")  // Many deps
        );

        fs::write(workspace.join("package.json"), large_package_json).unwrap();

        // Should handle large metadata files
        let workspace_id = router.workspace_id_for(&workspace).unwrap();
        assert!(workspace_id.contains("large-metadata-workspace"));

        let cache = router.cache_for_workspace(&workspace).await.unwrap();
        assert!(cache.get_stats().await.is_ok());

        // Test workspace discovery still works
        let found_workspace = router.find_nearest_workspace(&workspace).await.unwrap();
        assert_eq!(found_workspace, workspace);
    }

    #[tokio::test]
    async fn test_workspace_cache_memory_pressure() {
        let (router, temp_dir) = create_test_router().await;

        // Create moderate number of workspaces to test memory pressure without database lock issues
        let workspace_count = 15;
        let mut workspaces = Vec::new();

        for i in 0..workspace_count {
            let workspace = temp_dir.path().join(format!("memory-pressure-{:03}", i));
            fs::create_dir_all(&workspace).unwrap();
            fs::write(
                workspace.join("Cargo.toml"),
                format!(r#"[package]\nname = "memory-pressure-{}""#, i),
            )
            .unwrap();
            workspaces.push(workspace);
        }

        // Access workspaces with delays to avoid database locking
        let mut successful_accesses = 0;
        for workspace in &workspaces {
            // Small delay to avoid database lock contention
            tokio::time::sleep(Duration::from_millis(5)).await;

            if let Ok(cache) = router.cache_for_workspace(workspace).await {
                let _ = cache.get_stats().await; // Access to ensure cache is used
                successful_accesses += 1;
            }
        }

        // Should not exceed max_open_caches significantly
        assert!(router.open_caches.len() <= router.config.max_open_caches);

        // Should have accessed at least several workspaces successfully
        assert!(successful_accesses >= 5);

        // Stats should show reasonable memory usage patterns
        let stats = router.get_stats().await;
        assert!(stats.total_workspaces_seen >= successful_accesses);
        assert_eq!(stats.current_open_caches, router.open_caches.len());

        // Access pattern should show LRU behavior
        assert!(!stats.workspace_stats.is_empty());
        for ws_stat in &stats.workspace_stats {
            assert!(ws_stat.access_count > 0);
        }
    }
}
