//! Persistent storage layer for Call Graph Cache
//!
//! This module provides persistent storage capabilities for the LSP call graph cache,
//! enabling cache data to survive daemon restarts while maintaining high performance.
//!
//! ## Architecture
//!
//! The persistent cache uses sled as the storage engine with multiple trees:
//! - `nodes`: Main cache storage (NodeKey -> PersistedNode)
//! - `metadata`: Cache statistics and housekeeping
//! - `file_index`: File -> cache keys mapping for invalidation
//!
//! ## Data Flow
//!
//! ```text
//! Cache Request -> Check Memory -> Check Persistent -> Store/Retrieve
//!                      ↓              ↓               ↓
//!                 In-memory cache  sled database   Content hashing
//! ```

use crate::cache_types::{CallHierarchyInfo, NodeKey};
use crate::language_detector::Language;
use anyhow::{Context, Result};
use bincode;
use serde::{Deserialize, Serialize};
use sled::Db;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// A node as stored in persistent storage with additional metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedNode {
    /// The cache key for this node
    pub key: NodeKey,
    /// The call hierarchy information
    pub info: CallHierarchyInfo,
    /// When this node was first cached
    pub created_at: SystemTime,
    /// Language of the source file
    pub language: Language,
}

/// Cache metadata for housekeeping and statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    /// Total number of cached nodes
    pub total_nodes: u64,
    /// Estimated total size in bytes
    pub total_size_bytes: u64,
    /// Last time cleanup was performed
    pub last_cleanup: SystemTime,
    /// Cache format version for future migrations
    pub version: u32,
}

impl Default for CacheMetadata {
    fn default() -> Self {
        Self {
            total_nodes: 0,
            total_size_bytes: 0,
            last_cleanup: SystemTime::now(),
            version: 1,
        }
    }
}

/// Statistics about the persistent cache
#[derive(Debug, Clone)]
pub struct PersistentCacheStats {
    /// Number of nodes in storage
    pub total_nodes: u64,
    /// Estimated size in bytes
    pub total_size_bytes: u64,
    /// Number of files indexed
    pub total_files: usize,
    /// Database size on disk (bytes)
    pub disk_size_bytes: u64,
}

/// Configuration for the persistent cache
#[derive(Debug, Clone)]
pub struct PersistentCacheConfig {
    /// Base directory for cache storage
    pub cache_directory: Option<PathBuf>,
    /// Maximum size of cache in bytes (0 = unlimited)
    pub max_size_bytes: u64,
    /// Time-to-live for cache entries in days
    pub ttl_days: u64,
    /// Whether to compress stored data
    pub compress: bool,
}

impl Default for PersistentCacheConfig {
    fn default() -> Self {
        Self {
            cache_directory: None,
            max_size_bytes: 0, // Unlimited by default
            ttl_days: 30,      // 30 days default TTL
            compress: true,
        }
    }
}

/// Alias for PersistentCacheConfig to match user requirements
pub type PersistentStoreConfig = PersistentCacheConfig;

/// Persistent storage backend for call graph cache
pub struct PersistentCallGraphCache {
    /// Main sled database
    db: Arc<Db>,
    /// Tree for storing cached nodes
    nodes_tree: sled::Tree,
    /// Tree for storing cache metadata
    metadata_tree: sled::Tree,
    /// Tree for file -> keys index
    file_index_tree: sled::Tree,
    /// Configuration
    config: PersistentCacheConfig,
    /// In-memory metadata cache
    metadata: Arc<RwLock<CacheMetadata>>,
}

impl PersistentCallGraphCache {
    /// Create a new persistent cache instance
    pub async fn new(config: PersistentCacheConfig) -> Result<Self> {
        let cache_dir = config
            .cache_directory
            .clone()
            .unwrap_or_else(Self::default_cache_directory);

        let persistence_disabled = std::env::var("PROBE_DISABLE_PERSISTENCE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        // Ensure cache directory exists (unless persistence is disabled)
        if !persistence_disabled {
            std::fs::create_dir_all(&cache_dir)
                .context(format!("Failed to create cache directory: {cache_dir:?}"))?;
        }

        if persistence_disabled {
            info!("PROBE_DISABLE_PERSISTENCE=1 — using temporary in-memory sled store (no disk writes)");
        }

        let db_path = cache_dir.join("call_graph.db");

        if persistence_disabled {
            info!("Opening temporary call graph cache (persistence disabled)");
        } else {
            info!("Opening persistent call graph cache at: {:?}", db_path);
        }

        // Configure sled database
        let db = if persistence_disabled {
            sled::Config::default()
                .temporary(true)
                .cache_capacity(64 * 1024 * 1024) // 64MB cache
                .flush_every_ms(None) // Do not flush to disk when disabled
                .compression_factor(1)
                .use_compression(false)
                .open()
                .context("Failed to open temporary sled database")?
        } else {
            sled::Config::default()
                .path(&db_path)
                .cache_capacity(64 * 1024 * 1024) // 64MB cache
                .flush_every_ms(Some(1000)) // Flush every second
                .compression_factor(if config.compress { 5 } else { 1 })
                .use_compression(config.compress)
                .open()
                .context(format!("Failed to open sled database at: {db_path:?}"))?
        };

        let db = Arc::new(db);

        // Open trees
        let nodes_tree = db.open_tree("nodes").context("Failed to open nodes tree")?;

        let metadata_tree = db
            .open_tree("metadata")
            .context("Failed to open metadata tree")?;

        let file_index_tree = db
            .open_tree("file_index")
            .context("Failed to open file_index tree")?;

        // Load or initialize metadata and handle version compatibility
        let mut metadata = Self::load_metadata(&metadata_tree).await?;

        // Check for version compatibility and handle migrations
        let current_version = 1u32; // Current cache format version
        if metadata.version != current_version {
            info!(
                "Cache version mismatch: stored={}, current={}. Performing migration...",
                metadata.version, current_version
            );

            match Self::migrate_cache_version(&db, metadata.version, current_version).await {
                Ok(()) => {
                    info!("Cache migration completed successfully");
                    metadata.version = current_version;
                }
                Err(e) => {
                    warn!(
                        "Cache migration failed: {}. Clearing cache and starting fresh.",
                        e
                    );

                    // Clear all trees on migration failure
                    let nodes_tree = db.open_tree("nodes")?;
                    let file_index_tree = db.open_tree("file_index")?;

                    nodes_tree.clear()?;
                    file_index_tree.clear()?;

                    // Reset metadata
                    metadata = CacheMetadata::default();
                    metadata.version = current_version;
                }
            }
        }

        let metadata = Arc::new(RwLock::new(metadata));

        let cache = Self {
            db,
            nodes_tree,
            metadata_tree,
            file_index_tree,
            config,
            metadata,
        };

        info!(
            "Persistent cache initialized with {} nodes",
            cache.metadata.read().await.total_nodes
        );

        Ok(cache)
    }

    /// Get the default cache directory
    fn default_cache_directory() -> PathBuf {
        if let Ok(probe_cache_dir) = std::env::var("PROBE_LSP_CACHE_DIR") {
            PathBuf::from(probe_cache_dir)
        } else if let Some(cache_dir) = dirs::cache_dir() {
            cache_dir.join("probe").join("lsp")
        } else {
            PathBuf::from("/tmp").join("probe-lsp-cache")
        }
    }

    /// Load metadata from storage or create default
    async fn load_metadata(metadata_tree: &sled::Tree) -> Result<CacheMetadata> {
        match metadata_tree.get("cache_metadata")? {
            Some(data) => match bincode::deserialize::<CacheMetadata>(&data) {
                Ok(metadata) => Ok(metadata),
                Err(e) => {
                    warn!(
                        "Failed to deserialize cache metadata, using defaults: {}",
                        e
                    );
                    Ok(CacheMetadata::default())
                }
            },
            None => Ok(CacheMetadata::default()),
        }
    }

    /// Save metadata to storage
    async fn save_metadata(&self) -> Result<()> {
        let metadata = self.metadata.read().await;
        let data = bincode::serialize(&*metadata).context("Failed to serialize cache metadata")?;

        self.metadata_tree
            .insert("cache_metadata", data)
            .context("Failed to save cache metadata")?;

        Ok(())
    }

    /// Get a cached node by its key
    pub async fn get(&self, key: &NodeKey) -> Result<Option<PersistedNode>> {
        let key_bytes = bincode::serialize(key).context("Failed to serialize node key")?;

        debug!(
            "Persistent cache GET: symbol={}, file={}, md5={}, key_hash={:02x?}",
            key.symbol,
            key.file.display(),
            key.content_md5,
            md5::compute(&key_bytes)[..8].to_vec()
        );

        match self.nodes_tree.get(&key_bytes)? {
            Some(data) => {
                match bincode::deserialize::<PersistedNode>(&data) {
                    Ok(node) => {
                        info!(
                            "Persistent cache HIT: {}:{} (md5: {})",
                            key.file.display(),
                            key.symbol,
                            key.content_md5
                        );
                        Ok(Some(node))
                    }
                    Err(e) => {
                        warn!(
                            "Failed to deserialize cached node {}:{}: {}",
                            key.file.display(),
                            key.symbol,
                            e
                        );
                        // Remove corrupted entry
                        self.remove(key).await?;
                        Ok(None)
                    }
                }
            }
            None => {
                info!(
                    "Persistent cache MISS: {}:{} (md5: {})",
                    key.file.display(),
                    key.symbol,
                    key.content_md5
                );
                Ok(None)
            }
        }
    }

    /// Insert a node into the cache
    pub async fn insert(
        &self,
        key: NodeKey,
        info: CallHierarchyInfo,
        language: Language,
    ) -> Result<()> {
        let node = PersistedNode {
            key: key.clone(),
            info,
            created_at: SystemTime::now(),
            language,
        };

        let key_bytes = bincode::serialize(&key).context("Failed to serialize node key")?;

        debug!(
            "Persistent cache INSERT: symbol={}, file={}, md5={}, key_hash={:02x?}",
            key.symbol,
            key.file.display(),
            key.content_md5,
            md5::compute(&key_bytes)[..8].to_vec()
        );

        let node_bytes = bincode::serialize(&node).context("Failed to serialize node")?;

        // Insert the node
        self.nodes_tree
            .insert(&key_bytes, node_bytes.clone())
            .context("Failed to insert node")?;

        // Update file index
        let file_key = key.file.to_string_lossy().as_bytes().to_vec();
        let mut file_keys: Vec<Vec<u8>> = match self.file_index_tree.get(&file_key)? {
            Some(data) => bincode::deserialize(&data).unwrap_or_default(),
            None => Vec::new(),
        };

        if !file_keys.contains(&key_bytes) {
            file_keys.push(key_bytes.clone());
            let file_keys_data =
                bincode::serialize(&file_keys).context("Failed to serialize file keys")?;
            self.file_index_tree
                .insert(&file_key, file_keys_data)
                .context("Failed to update file index")?;
        }

        // Update metadata
        {
            let mut metadata = self.metadata.write().await;
            metadata.total_nodes += 1;
            metadata.total_size_bytes += node_bytes.len() as u64;
        }

        // Periodically save metadata (every 10 insertions)
        if self.metadata.read().await.total_nodes % 10 == 0 {
            if let Err(e) = self.save_metadata().await {
                warn!("Failed to save metadata: {}", e);
            }
        }

        debug!("Cached node: {}:{}", key.file.display(), key.symbol);

        Ok(())
    }

    /// Remove a node from the cache
    pub async fn remove(&self, key: &NodeKey) -> Result<bool> {
        let key_bytes = bincode::serialize(key).context("Failed to serialize node key")?;

        let was_present = self.nodes_tree.remove(&key_bytes)?.is_some();

        if was_present {
            // Update file index
            let file_key = key.file.to_string_lossy().as_bytes().to_vec();
            if let Some(data) = self.file_index_tree.get(&file_key)? {
                let mut file_keys: Vec<Vec<u8>> = bincode::deserialize(&data).unwrap_or_default();
                file_keys.retain(|k| k != &key_bytes);

                if file_keys.is_empty() {
                    self.file_index_tree.remove(&file_key)?;
                } else {
                    let file_keys_data =
                        bincode::serialize(&file_keys).context("Failed to serialize file keys")?;
                    self.file_index_tree.insert(&file_key, file_keys_data)?;
                }
            }

            // Update metadata
            {
                let mut metadata = self.metadata.write().await;
                metadata.total_nodes = metadata.total_nodes.saturating_sub(1);
            }

            debug!("Removed cached node: {}:{}", key.file.display(), key.symbol);
        }

        Ok(was_present)
    }

    /// Get all nodes for a specific file
    pub async fn get_by_file(&self, file_path: &Path) -> Result<Vec<PersistedNode>> {
        let file_key = file_path.to_string_lossy().as_bytes().to_vec();

        let key_list: Vec<Vec<u8>> = match self.file_index_tree.get(&file_key)? {
            Some(data) => bincode::deserialize(&data).unwrap_or_default(),
            None => return Ok(Vec::new()),
        };

        let mut nodes = Vec::new();
        for key_bytes in key_list {
            if let Some(node_data) = self.nodes_tree.get(&key_bytes)? {
                match bincode::deserialize::<PersistedNode>(&node_data) {
                    Ok(node) => nodes.push(node),
                    Err(e) => {
                        warn!("Failed to deserialize node in file index: {}", e);
                        // Clean up corrupted entry
                        self.nodes_tree.remove(&key_bytes)?;
                    }
                }
            }
        }

        debug!(
            "Retrieved {} nodes for file: {}",
            nodes.len(),
            file_path.display()
        );
        Ok(nodes)
    }

    /// Get cache statistics
    pub async fn get_stats(&self) -> Result<PersistentCacheStats> {
        let metadata = self.metadata.read().await;

        // Get database size on disk
        let disk_size = self.db.size_on_disk().unwrap_or(0);

        // Count files
        let total_files = self.file_index_tree.len();

        Ok(PersistentCacheStats {
            total_nodes: metadata.total_nodes,
            total_size_bytes: metadata.total_size_bytes,
            total_files,
            disk_size_bytes: disk_size,
        })
    }

    /// Clear all cache data
    pub async fn clear(&self) -> Result<()> {
        info!("Clearing persistent call graph cache");

        // Clear all trees
        self.nodes_tree.clear()?;
        self.file_index_tree.clear()?;

        // Reset metadata
        {
            let mut metadata = self.metadata.write().await;
            *metadata = CacheMetadata::default();
        }

        self.save_metadata().await?;
        self.db.flush_async().await?;

        info!("Persistent cache cleared");
        Ok(())
    }

    /// Compact the database to reclaim space
    pub async fn compact(&self) -> Result<()> {
        info!("Compacting persistent cache database");

        // Save metadata before compaction
        self.save_metadata().await?;

        // Flush and compact
        self.db.flush_async().await?;

        info!("Database compaction completed");
        Ok(())
    }

    /// Clean up expired cache entries based on age
    pub async fn cleanup_expired(&self) -> Result<usize> {
        info!("Starting cleanup of expired cache entries");

        let now = SystemTime::now();
        let mut expired_keys = Vec::new();
        let mut total_scanned = 0;

        // Scan all nodes to find expired ones
        // We use iter() to go through all key-value pairs
        for result in self.nodes_tree.iter() {
            total_scanned += 1;
            match result {
                Ok((key_bytes, value_bytes)) => {
                    match bincode::deserialize::<PersistedNode>(&value_bytes) {
                        Ok(node) => {
                            // Check if node is older than the configured TTL
                            let max_age =
                                std::time::Duration::from_secs(self.config.ttl_days * 24 * 60 * 60);

                            if let Ok(age) = now.duration_since(node.created_at) {
                                if age > max_age {
                                    expired_keys.push(key_bytes.to_vec());
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Found corrupted node during cleanup, will remove: {}", e);
                            expired_keys.push(key_bytes.to_vec());
                        }
                    }
                }
                Err(e) => {
                    warn!("Error scanning node during cleanup: {}", e);
                }
            }
        }

        // Remove expired entries
        let mut removed_count = 0;
        for key_bytes in &expired_keys {
            if self.nodes_tree.remove(key_bytes)?.is_some() {
                removed_count += 1;

                // Also clean up from file index and git index
                // Note: This is not perfectly efficient, but ensures consistency
                if let Ok(node_key) = bincode::deserialize::<NodeKey>(key_bytes) {
                    let file_key = node_key.file.to_string_lossy().as_bytes().to_vec();
                    if let Some(data) = self.file_index_tree.get(&file_key)? {
                        let mut file_keys: Vec<Vec<u8>> =
                            bincode::deserialize(&data).unwrap_or_default();
                        file_keys.retain(|k| k != key_bytes);

                        if file_keys.is_empty() {
                            self.file_index_tree.remove(&file_key)?;
                        } else {
                            let file_keys_data = bincode::serialize(&file_keys)?;
                            self.file_index_tree.insert(&file_key, file_keys_data)?;
                        }
                    }
                }
            }
        }

        // Update metadata
        {
            let mut metadata = self.metadata.write().await;
            metadata.total_nodes = metadata.total_nodes.saturating_sub(removed_count as u64);
            metadata.last_cleanup = now;
        }

        // Save updated metadata
        self.save_metadata().await?;

        info!(
            "Cleanup completed: scanned {} entries, removed {} expired entries",
            total_scanned, removed_count
        );

        Ok(removed_count)
    }

    /// Get cache directory path
    pub fn cache_directory(&self) -> PathBuf {
        self.config
            .cache_directory
            .clone()
            .unwrap_or_else(Self::default_cache_directory)
    }

    /// Handle cache version migrations
    async fn migrate_cache_version(
        _db: &Arc<sled::Db>,
        from_version: u32,
        to_version: u32,
    ) -> Result<()> {
        match (from_version, to_version) {
            (0, 1) => {
                // Migration from version 0 (no version) to version 1
                // For now, this is a no-op since our current format is version 1
                info!("Migrating cache from version 0 to 1 (initial versioning)");
                Ok(())
            }
            (v1, v2) if v1 == v2 => {
                // No migration needed
                Ok(())
            }
            (v1, v2) if v1 > v2 => {
                // Downgrade not supported - this should trigger a clear
                Err(anyhow::anyhow!(
                    "Cache downgrade from version {} to {} is not supported",
                    v1,
                    v2
                ))
            }
            _ => {
                // Unknown migration path
                Err(anyhow::anyhow!(
                    "Unknown migration path from version {} to {}",
                    from_version,
                    to_version
                ))
            }
        }
    }

    /// Iterate over all cached nodes (for cache warming)
    /// Returns an iterator over (NodeKey, PersistedNode) pairs
    pub async fn iter_nodes(&self) -> Result<Vec<(NodeKey, PersistedNode)>> {
        let mut nodes = Vec::new();

        for result in self.nodes_tree.iter() {
            match result {
                Ok((key_bytes, value_bytes)) => {
                    match (
                        bincode::deserialize::<NodeKey>(&key_bytes),
                        bincode::deserialize::<PersistedNode>(&value_bytes),
                    ) {
                        (Ok(key), Ok(node)) => {
                            nodes.push((key, node));
                        }
                        (Err(e), _) => {
                            warn!("Failed to deserialize key during iteration: {}", e);
                        }
                        (_, Err(e)) => {
                            warn!("Failed to deserialize node during iteration: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Error iterating nodes: {}", e);
                    break;
                }
            }
        }

        Ok(nodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_types::{CallHierarchyInfo, CallInfo};
    use tempfile::tempdir;

    async fn create_test_cache() -> PersistentCallGraphCache {
        let temp_dir = tempdir().unwrap();
        let config = PersistentCacheConfig {
            cache_directory: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        };

        PersistentCallGraphCache::new(config).await.unwrap()
    }

    fn create_test_node_key() -> NodeKey {
        NodeKey::new("test_function", "/test/file.rs", "abc123")
    }

    fn create_test_call_hierarchy() -> CallHierarchyInfo {
        CallHierarchyInfo {
            incoming_calls: vec![CallInfo {
                name: "caller".to_string(),
                file_path: "/test/caller.rs".to_string(),
                line: 42,
                column: 10,
                symbol_kind: "function".to_string(),
            }],
            outgoing_calls: vec![CallInfo {
                name: "callee".to_string(),
                file_path: "/test/callee.rs".to_string(),
                line: 15,
                column: 5,
                symbol_kind: "function".to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn test_basic_operations() {
        let cache = create_test_cache().await;
        let key = create_test_node_key();
        let info = create_test_call_hierarchy();

        // Initially empty
        assert!(cache.get(&key).await.unwrap().is_none());

        // Insert
        cache
            .insert(key.clone(), info.clone(), Language::Rust)
            .await
            .unwrap();

        // Retrieve
        let retrieved = cache.get(&key).await.unwrap();
        assert!(retrieved.is_some());
        let node = retrieved.unwrap();
        assert_eq!(node.key.symbol, "test_function");
        assert_eq!(node.info.incoming_calls.len(), 1);
        assert_eq!(node.info.outgoing_calls.len(), 1);
        assert_eq!(node.language, Language::Rust);

        // Remove
        let was_removed = cache.remove(&key).await.unwrap();
        assert!(was_removed);

        // No longer present
        assert!(cache.get(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_file_based_operations() {
        let cache = create_test_cache().await;
        let file_path = Path::new("/test/file.rs");

        // Insert multiple nodes for same file
        for i in 0..3 {
            let key = NodeKey::new(format!("func_{}", i), file_path, format!("hash_{}", i));
            let info = create_test_call_hierarchy();
            cache.insert(key, info, Language::Rust).await.unwrap();
        }

        // Get all nodes for file
        let nodes = cache.get_by_file(file_path).await.unwrap();
        assert_eq!(nodes.len(), 3);

        // Verify all nodes are for the correct file
        for node in &nodes {
            assert_eq!(node.key.file, file_path);
        }
    }

    #[tokio::test]
    async fn test_persistence() {
        let temp_dir = tempdir().unwrap();
        let config = PersistentCacheConfig {
            cache_directory: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        };

        let key = create_test_node_key();
        let info = create_test_call_hierarchy();

        // Create cache and insert data
        {
            let cache = PersistentCallGraphCache::new(config.clone()).await.unwrap();
            cache
                .insert(key.clone(), info.clone(), Language::Rust)
                .await
                .unwrap();
        }

        // Recreate cache and verify data persists
        {
            let cache = PersistentCallGraphCache::new(config).await.unwrap();
            let retrieved = cache.get(&key).await.unwrap();
            assert!(retrieved.is_some());

            let node = retrieved.unwrap();
            assert_eq!(node.key.symbol, "test_function");
            assert_eq!(node.language, Language::Rust);
        }
    }

    #[tokio::test]
    async fn test_clear_cache() {
        let cache = create_test_cache().await;

        // Insert some data
        for i in 0..5 {
            let key = NodeKey::new(
                format!("func_{}", i),
                "/test/file.rs",
                format!("hash_{}", i),
            );
            let info = create_test_call_hierarchy();
            cache.insert(key, info, Language::Rust).await.unwrap();
        }

        // Verify data exists
        let stats = cache.get_stats().await.unwrap();
        assert_eq!(stats.total_nodes, 5);

        // Clear cache
        cache.clear().await.unwrap();

        // Verify empty
        let stats = cache.get_stats().await.unwrap();
        assert_eq!(stats.total_nodes, 0);
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let cache = create_test_cache().await;

        // Initially empty
        let stats = cache.get_stats().await.unwrap();
        assert_eq!(stats.total_nodes, 0);
        assert_eq!(stats.total_files, 0);

        // Insert data
        let key = create_test_node_key();
        let info = create_test_call_hierarchy();
        cache.insert(key, info, Language::Rust).await.unwrap();

        // Check updated stats
        let stats = cache.get_stats().await.unwrap();
        assert_eq!(stats.total_nodes, 1);
        assert_eq!(stats.total_files, 1);
        assert!(stats.total_size_bytes > 0);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let cache = Arc::new(create_test_cache().await);
        let mut handles = Vec::new();

        // Spawn multiple tasks doing concurrent operations
        for i in 0..10 {
            let cache_clone = Arc::clone(&cache);
            let handle = tokio::spawn(async move {
                let key = NodeKey::new(
                    format!("func_{}", i),
                    "/test/file.rs",
                    format!("hash_{}", i),
                );
                let info = create_test_call_hierarchy();

                // Insert
                cache_clone
                    .insert(key.clone(), info, Language::Rust)
                    .await
                    .unwrap();

                // Retrieve
                let retrieved = cache_clone.get(&key).await.unwrap();
                assert!(retrieved.is_some());

                i
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all data was inserted
        let stats = cache.get_stats().await.unwrap();
        assert_eq!(stats.total_nodes, 10);
    }

    #[tokio::test]
    async fn test_database_recovery() {
        let temp_dir = tempdir().unwrap();
        let config = PersistentCacheConfig {
            cache_directory: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        };

        // Create cache and insert data
        {
            let cache = PersistentCallGraphCache::new(config.clone()).await.unwrap();
            let key = create_test_node_key();
            let info = create_test_call_hierarchy();
            cache.insert(key, info, Language::Rust).await.unwrap();
        }

        // Simulate corruption by writing invalid metadata
        {
            let cache = PersistentCallGraphCache::new(config.clone()).await.unwrap();
            cache
                .metadata_tree
                .insert("cache_metadata", b"invalid_data")
                .unwrap();
        }

        // Should still be able to open and recover
        {
            let cache = PersistentCallGraphCache::new(config).await.unwrap();
            let key = create_test_node_key();
            let retrieved = cache.get(&key).await.unwrap();
            assert!(retrieved.is_some());
        }
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let temp_dir = tempdir().unwrap();
        let config = PersistentCacheConfig {
            cache_directory: Some(temp_dir.path().to_path_buf()),
            ttl_days: 0, // Immediate expiry for testing
            ..Default::default()
        };

        let cache = PersistentCallGraphCache::new(config).await.unwrap();

        // Insert some data
        for i in 0..3 {
            let key = NodeKey::new(
                format!("func_{}", i),
                "/test/file.rs",
                format!("hash_{}", i),
            );
            let info = create_test_call_hierarchy();
            cache.insert(key, info, Language::Rust).await.unwrap();
        }

        // Verify data exists
        let stats = cache.get_stats().await.unwrap();
        assert_eq!(stats.total_nodes, 3);

        // Run cleanup (should remove all entries due to ttl_days = 0)
        let removed = cache.cleanup_expired().await.unwrap();
        assert_eq!(removed, 3);

        // Verify empty
        let stats = cache.get_stats().await.unwrap();
        assert_eq!(stats.total_nodes, 0);
    }
}
