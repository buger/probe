use crate::cache_types::{AllCacheStats, CachedLspNode, LspCacheKey, LspCacheStats, LspOperation};
use anyhow::Result;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, info, warn};

/// Configuration for generic LSP cache
#[derive(Debug, Clone)]
pub struct LspCacheConfig {
    /// Maximum number of entries per operation type
    pub capacity_per_operation: usize,
    /// Time-to-live for cached entries
    pub ttl: Duration,
    /// How often to check for expired entries
    pub eviction_check_interval: Duration,
    /// Whether to enable persistent storage
    pub persistent: bool,
    /// Directory for persistent storage
    pub cache_directory: Option<PathBuf>,
}

impl Default for LspCacheConfig {
    fn default() -> Self {
        Self {
            capacity_per_operation: 500,    // 500 entries per operation type
            ttl: Duration::from_secs(1800), // 30 minutes
            eviction_check_interval: Duration::from_secs(60), // Check every minute
            persistent: false,
            cache_directory: None,
        }
    }
}

/// Generic LSP cache that can handle different types of LSP responses
pub struct LspCache<T> {
    /// Operation type for this cache
    operation: LspOperation,
    /// Main cache storage: LspCacheKey -> CachedLspNode<T>
    entries: DashMap<LspCacheKey, Arc<CachedLspNode<T>>>,
    /// File index for file-based invalidation
    file_index: DashMap<PathBuf, HashSet<LspCacheKey>>,
    /// In-flight deduplication
    inflight: DashMap<LspCacheKey, Arc<AsyncMutex<()>>>,
    /// Configuration
    config: LspCacheConfig,
    /// Statistics
    hit_count: Arc<AsyncMutex<u64>>,
    miss_count: Arc<AsyncMutex<u64>>,
    eviction_count: Arc<AsyncMutex<u64>>,
    /// Last eviction check time
    last_eviction: Arc<AsyncMutex<Instant>>,
    /// Persistent storage backend
    persistent_store: Option<Arc<sled::Db>>,
}

impl<T> LspCache<T>
where
    T: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
{
    pub fn new(operation: LspOperation, config: LspCacheConfig) -> Result<Self> {
        let persistent_store = if config.persistent {
            let cache_dir = config.cache_directory.clone().unwrap_or_else(|| {
                dirs::cache_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join("probe-lsp-cache")
            });

            std::fs::create_dir_all(&cache_dir)?;
            let db_path = cache_dir.join(format!("{operation:?}.db"));
            let db = sled::open(db_path)?;
            Some(Arc::new(db))
        } else {
            None
        };

        Ok(Self {
            operation,
            entries: DashMap::new(),
            file_index: DashMap::new(),
            inflight: DashMap::new(),
            config,
            hit_count: Arc::new(AsyncMutex::new(0)),
            miss_count: Arc::new(AsyncMutex::new(0)),
            eviction_count: Arc::new(AsyncMutex::new(0)),
            last_eviction: Arc::new(AsyncMutex::new(Instant::now())),
            persistent_store,
        })
    }

    /// Get a cached entry or compute it if not present
    /// Simple get method for cache lookup
    pub async fn get(&self, key: &LspCacheKey) -> Option<T> {
        // Check memory cache first
        if let Some(entry) = self.entries.get(key) {
            let mut node = Arc::clone(&entry);
            // Touch the node to update access time
            if let Some(mutable_node) = Arc::get_mut(&mut node) {
                mutable_node.touch();
            }

            let mut hit_count = self.hit_count.lock().await;
            *hit_count += 1;

            return Some(entry.data.clone());
        }

        // Check persistent storage if enabled
        if let Some(ref db) = self.persistent_store {
            let key_bytes = bincode::serialize(key).ok()?;
            if let Ok(Some(value_bytes)) = db.get(&key_bytes) {
                if let Ok(node) = bincode::deserialize::<CachedLspNode<T>>(&value_bytes) {
                    // Verify TTL
                    if node.created_at.elapsed() < self.config.ttl {
                        // Load into memory cache
                        let arc_node = Arc::new(node.clone());
                        self.entries.insert(key.clone(), arc_node);

                        // Update file index
                        self.file_index
                            .entry(key.file.clone())
                            .or_default()
                            .insert(key.clone());

                        let mut hit_count = self.hit_count.lock().await;
                        *hit_count += 1;

                        return Some(node.data);
                    }
                }
            }
        }

        let mut miss_count = self.miss_count.lock().await;
        *miss_count += 1;

        None
    }

    /// Simple insert method for cache population
    pub async fn insert(&self, key: LspCacheKey, value: T) {
        let node = CachedLspNode::new(key.clone(), value);
        let arc_node = Arc::new(node.clone());

        // Insert into memory cache
        self.entries.insert(key.clone(), arc_node);

        // Update file index
        self.file_index
            .entry(key.file.clone())
            .or_default()
            .insert(key.clone());

        // Save to persistent storage if enabled
        if let Some(ref db) = self.persistent_store {
            let key_bytes = bincode::serialize(&key).ok();
            let value_bytes = bincode::serialize(&node).ok();

            if let (Some(kb), Some(vb)) = (key_bytes, value_bytes) {
                let _ = db.insert(kb, vb);
            }
        }

        // Trigger eviction check if needed
        self.check_eviction().await;
    }

    /// Check if eviction is needed and perform it
    async fn check_eviction(&self) {
        let mut last_eviction = self.last_eviction.lock().await;

        // Only check for eviction periodically
        if last_eviction.elapsed() < self.config.eviction_check_interval {
            return;
        }

        let entry_count = self.entries.len();
        if entry_count <= self.config.capacity_per_operation {
            return;
        }

        // Perform eviction - remove oldest entries
        let to_evict = entry_count - self.config.capacity_per_operation
            + self.config.capacity_per_operation / 4;

        // Collect entries with their last accessed time
        let mut entries: Vec<(LspCacheKey, Instant)> = self
            .entries
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().last_accessed))
            .collect();

        // Sort by last accessed time (oldest first)
        entries.sort_by_key(|(_, time)| *time);

        // Remove oldest entries
        for (key, _) in entries.into_iter().take(to_evict) {
            self.entries.remove(&key);

            // Update file index
            if let Some(mut file_keys) = self.file_index.get_mut(&key.file) {
                file_keys.remove(&key);
            }

            // Remove from persistent storage
            if let Some(ref db) = self.persistent_store {
                if let Ok(key_bytes) = bincode::serialize(&key) {
                    let _ = db.remove(key_bytes);
                }
            }
        }

        *self.eviction_count.lock().await += to_evict as u64;
        *last_eviction = Instant::now();
    }

    pub async fn get_or_compute<F, Fut>(
        &self,
        key: LspCacheKey,
        compute: F,
    ) -> Result<Arc<CachedLspNode<T>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        // Check memory cache first
        if let Some(node) = self.get_from_memory(&key).await {
            *self.hit_count.lock().await += 1;
            return Ok(node);
        }

        // Check persistent storage if enabled
        if let Some(ref store) = self.persistent_store {
            if let Some(node) = self.get_from_persistent(store, &key).await? {
                // Store back in memory cache
                self.insert_in_memory(node.clone());
                *self.hit_count.lock().await += 1;
                return Ok(node);
            }
        }

        *self.miss_count.lock().await += 1;

        // Deduplication: ensure only one computation per key
        let lock = self
            .inflight
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();

        let _guard = lock.lock().await;

        // Double-check after acquiring lock
        if let Some(node) = self.get_from_memory(&key).await {
            self.inflight.remove(&key);
            return Ok(node);
        }

        // Compute the value
        debug!(
            "Computing {:?} for {}:{} (md5: {})",
            self.operation,
            key.file.display(),
            format!("{}:{}", key.line, key.column),
            key.content_md5
        );

        let data = compute().await?;
        let node = Arc::new(CachedLspNode::new(key.clone(), data));

        // Insert into memory cache
        self.insert_in_memory(node.clone());

        // Insert into persistent storage if enabled
        if let Some(ref store) = self.persistent_store {
            if let Err(e) = self.insert_in_persistent(store, &node).await {
                warn!("Failed to store in persistent cache: {}", e);
            }
        }

        // Clean up in-flight tracker
        self.inflight.remove(&key);

        // Trigger eviction check if needed
        self.maybe_evict().await;

        Ok(node)
    }

    /// Get entry from memory cache
    async fn get_from_memory(&self, key: &LspCacheKey) -> Option<Arc<CachedLspNode<T>>> {
        self.entries.get(key).map(|entry| entry.clone())
    }

    /// Get entry from persistent storage
    async fn get_from_persistent(
        &self,
        store: &sled::Db,
        key: &LspCacheKey,
    ) -> Result<Option<Arc<CachedLspNode<T>>>> {
        let key_bytes = bincode::serialize(key)?;

        if let Some(value_bytes) = store.get(&key_bytes)? {
            match bincode::deserialize::<CachedLspNode<T>>(&value_bytes) {
                Ok(node) => {
                    // Check if entry is still valid (not expired)
                    if node.created_at.elapsed() <= self.config.ttl {
                        return Ok(Some(Arc::new(node)));
                    } else {
                        // Remove expired entry
                        let _ = store.remove(&key_bytes);
                    }
                }
                Err(e) => {
                    warn!("Failed to deserialize persistent cache entry: {}", e);
                    // Remove corrupted entry
                    let _ = store.remove(&key_bytes);
                }
            }
        }

        Ok(None)
    }

    /// Insert entry into memory cache
    fn insert_in_memory(&self, node: Arc<CachedLspNode<T>>) {
        let key = node.key.clone();

        // Add to main cache
        self.entries.insert(key.clone(), node);

        // Update file index
        self.file_index
            .entry(key.file.clone())
            .or_default()
            .insert(key);
    }

    /// Insert entry into persistent storage
    async fn insert_in_persistent(&self, store: &sled::Db, node: &CachedLspNode<T>) -> Result<()> {
        let key_bytes = bincode::serialize(&node.key)?;
        let value_bytes = bincode::serialize(node)?;
        store.insert(&key_bytes, value_bytes)?;
        Ok(())
    }

    /// Invalidate entries for a specific file
    pub async fn invalidate_file(&self, file: &Path) {
        if let Some((_, keys)) = self.file_index.remove(file) {
            let count = keys.len();

            for key in keys {
                // Remove from memory cache
                self.entries.remove(&key);

                // Remove from persistent storage if enabled
                if let Some(ref store) = self.persistent_store {
                    if let Ok(key_bytes) = bincode::serialize(&key) {
                        let _ = store.remove(&key_bytes);
                    }
                }
            }

            if count > 0 {
                *self.eviction_count.lock().await += count as u64;
                info!(
                    "Invalidated {} {:?} cache entries for file {}",
                    count,
                    self.operation,
                    file.display()
                );
            }
        }
    }

    /// Clear the entire cache
    pub async fn clear(&self) {
        let count = self.entries.len();

        self.entries.clear();
        self.file_index.clear();
        self.inflight.clear();

        // Clear persistent storage if enabled
        if let Some(ref store) = self.persistent_store {
            let _ = store.clear();
        }

        *self.eviction_count.lock().await += count as u64;
        info!("Cleared {} {:?} cache entries", count, self.operation);
    }

    /// Check and evict expired entries
    async fn maybe_evict(&self) {
        let mut last_check = self.last_eviction.lock().await;

        if last_check.elapsed() < self.config.eviction_check_interval {
            return;
        }

        *last_check = Instant::now();
        drop(last_check); // Release lock early

        let now = Instant::now();
        let mut expired_keys = Vec::new();
        let mut lru_candidates = Vec::new();

        // Find expired entries and collect LRU candidates
        for entry in self.entries.iter() {
            let node = entry.value();
            if now.duration_since(node.created_at) > self.config.ttl {
                expired_keys.push(entry.key().clone());
            } else {
                lru_candidates.push((entry.key().clone(), node.last_accessed, node.access_count));
            }
        }

        // Remove expired entries
        for key in &expired_keys {
            self.remove_entry(key).await;
        }

        // If over capacity, evict LRU entries
        if self.entries.len() > self.config.capacity_per_operation {
            // Sort by last accessed time (oldest first) and access count
            lru_candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));

            let to_evict = self
                .entries
                .len()
                .saturating_sub(self.config.capacity_per_operation);
            for (key, _, _) in lru_candidates.iter().take(to_evict) {
                self.remove_entry(key).await;
            }

            debug!(
                "Evicted {} expired and {} LRU {:?} cache entries",
                expired_keys.len(),
                to_evict,
                self.operation
            );
        }
    }

    /// Remove a single entry from all storage layers
    async fn remove_entry(&self, key: &LspCacheKey) {
        // Remove from memory cache
        if self.entries.remove(key).is_some() {
            // Update file index
            if let Some(mut keys) = self.file_index.get_mut(&key.file) {
                keys.remove(key);
                if keys.is_empty() {
                    drop(keys);
                    self.file_index.remove(&key.file);
                }
            }

            // Remove from persistent storage if enabled
            if let Some(ref store) = self.persistent_store {
                if let Ok(key_bytes) = bincode::serialize(key) {
                    let _ = store.remove(&key_bytes);
                }
            }

            *self.eviction_count.lock().await += 1;
        }
    }

    /// Get cache statistics
    pub async fn stats(&self) -> LspCacheStats {
        let hit_count = *self.hit_count.lock().await;
        let miss_count = *self.miss_count.lock().await;
        let eviction_count = *self.eviction_count.lock().await;

        // Estimate memory usage (rough calculation)
        let memory_usage_estimate = self.entries.len() * std::mem::size_of::<CachedLspNode<T>>();

        LspCacheStats {
            operation: self.operation,
            total_entries: self.entries.len(),
            hit_count,
            miss_count,
            eviction_count,
            inflight_count: self.inflight.len(),
            memory_usage_estimate,
        }
    }

    /// Get operation type
    pub fn operation(&self) -> LspOperation {
        self.operation
    }

    /// Check if persistent storage is enabled
    pub fn is_persistent(&self) -> bool {
        self.persistent_store.is_some()
    }

    /// Get cache directory if persistent storage is enabled
    pub fn cache_directory(&self) -> Option<&Path> {
        self.config.cache_directory.as_deref()
    }

    /// Compact persistent storage (if enabled)
    pub async fn compact_persistent_storage(&self) -> Result<()> {
        if let Some(ref store) = self.persistent_store {
            store.flush_async().await?;
            info!(
                "Compacted persistent storage for {:?} cache",
                self.operation
            );
        }
        Ok(())
    }

    /// Export cache to JSON for debugging
    pub async fn export_to_json(&self) -> Result<String> {
        let mut export_data = Vec::new();

        for entry in self.entries.iter() {
            let key = entry.key();
            let node = entry.value();

            let export_entry = serde_json::json!({
                "key": {
                    "file": key.file,
                    "line": key.line,
                    "column": key.column,
                    "content_md5": key.content_md5,
                    "operation": key.operation,
                    "extra_params": key.extra_params
                },
                "created_at": node.created_at.elapsed().as_secs(),
                "last_accessed": node.last_accessed.elapsed().as_secs(),
                "access_count": node.access_count
            });

            export_data.push(export_entry);
        }

        Ok(serde_json::to_string_pretty(&export_data)?)
    }
}

/// Collection of all LSP caches for different operations
pub struct LspCacheManager {
    /// Individual caches for each operation type
    caches: DashMap<LspOperation, Arc<dyn LspCacheOperations + Send + Sync + 'static>>,
    /// Shared configuration
    config: LspCacheConfig,
}

/// Trait for type-erased cache operations
#[async_trait::async_trait]
pub trait LspCacheOperations: Send + Sync {
    async fn invalidate_file(&self, file: &Path);
    async fn clear(&self);
    async fn stats(&self) -> LspCacheStats;
    async fn export_to_json(&self) -> Result<String>;
    fn operation(&self) -> LspOperation;
    fn is_persistent(&self) -> bool;
}

#[async_trait::async_trait]
impl<T> LspCacheOperations for LspCache<T>
where
    T: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
{
    async fn invalidate_file(&self, file: &Path) {
        self.invalidate_file(file).await
    }

    async fn clear(&self) {
        self.clear().await
    }

    async fn stats(&self) -> LspCacheStats {
        self.stats().await
    }

    async fn export_to_json(&self) -> Result<String> {
        self.export_to_json().await
    }

    fn operation(&self) -> LspOperation {
        self.operation()
    }

    fn is_persistent(&self) -> bool {
        self.is_persistent()
    }
}

impl LspCacheManager {
    pub fn new(config: LspCacheConfig) -> Self {
        Self {
            caches: DashMap::new(),
            config,
        }
    }

    /// Register a cache for a specific operation
    pub fn register_cache<T>(&self, operation: LspOperation, cache: LspCache<T>)
    where
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
    {
        self.caches.insert(operation, Arc::new(cache));
    }

    /// Invalidate entries for a file across all caches
    pub async fn invalidate_file(&self, file: &Path) {
        for cache in self.caches.iter() {
            cache.value().invalidate_file(file).await;
        }
    }

    /// Clear all caches
    pub async fn clear_all(&self) {
        for cache in self.caches.iter() {
            cache.value().clear().await;
        }
    }

    /// Clear a specific cache
    pub async fn clear_cache(&self, operation: LspOperation) {
        if let Some(cache) = self.caches.get(&operation) {
            cache.clear().await;
        }
    }

    /// Get combined statistics for all caches
    pub async fn all_stats(&self) -> AllCacheStats {
        let mut per_operation = Vec::new();
        let mut total_memory = 0;
        let mut persistent_enabled = false;
        let mut cache_dir = None;

        for cache in self.caches.iter() {
            let stats = cache.value().stats().await;
            total_memory += stats.memory_usage_estimate;

            if cache.value().is_persistent() {
                persistent_enabled = true;
                if cache_dir.is_none() {
                    cache_dir = self
                        .config
                        .cache_directory
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string());
                }
            }

            per_operation.push(stats);
        }

        AllCacheStats {
            per_operation,
            total_memory_usage: total_memory,
            cache_directory: cache_dir,
            persistent_cache_enabled: persistent_enabled,
        }
    }

    /// Export all caches to JSON for debugging
    pub async fn export_all_to_json(&self) -> Result<String> {
        let mut all_exports = serde_json::Map::new();

        for cache in self.caches.iter() {
            let operation_name = format!("{:?}", cache.key());
            let export_data = cache.value().export_to_json().await?;
            all_exports.insert(operation_name, serde_json::from_str(&export_data)?);
        }

        Ok(serde_json::to_string_pretty(&all_exports)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_types::{DefinitionInfo, LocationInfo, RangeInfo};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_lsp_cache_basic_operations() {
        let config = LspCacheConfig::default();
        let cache: LspCache<DefinitionInfo> =
            LspCache::new(LspOperation::Definition, config).unwrap();

        let key = LspCacheKey::new(
            "/test/file.rs",
            10,
            5,
            "abc123",
            LspOperation::Definition,
            None,
        );

        // First call should compute
        let result = cache
            .get_or_compute(key.clone(), || async {
                Ok(DefinitionInfo {
                    locations: vec![LocationInfo {
                        uri: "file:///test/file.rs".to_string(),
                        range: RangeInfo {
                            start_line: 10,
                            start_character: 5,
                            end_line: 10,
                            end_character: 15,
                        },
                    }],
                })
            })
            .await
            .unwrap();

        assert_eq!(result.data.locations.len(), 1);
        assert_eq!(result.data.locations[0].uri, "file:///test/file.rs");

        // Second call should hit cache
        let cached = cache.get_from_memory(&key).await;
        assert!(cached.is_some());
    }

    #[tokio::test]
    async fn test_lsp_cache_file_invalidation() {
        let config = LspCacheConfig::default();
        let cache: LspCache<DefinitionInfo> =
            LspCache::new(LspOperation::Definition, config).unwrap();

        let key = LspCacheKey::new(
            "/test/file.rs",
            10,
            5,
            "abc123",
            LspOperation::Definition,
            None,
        );

        // Add entry
        cache
            .get_or_compute(key.clone(), || async {
                Ok(DefinitionInfo { locations: vec![] })
            })
            .await
            .unwrap();

        // Should be cached
        assert!(cache.get_from_memory(&key).await.is_some());

        // Invalidate the file
        cache.invalidate_file(Path::new("/test/file.rs")).await;

        // Should be gone
        assert!(cache.get_from_memory(&key).await.is_none());
    }

    #[tokio::test]
    async fn test_lsp_cache_persistent_storage() {
        let temp_dir = tempdir().unwrap();
        let mut config = LspCacheConfig::default();
        config.persistent = true;
        config.cache_directory = Some(temp_dir.path().to_path_buf());

        let cache: LspCache<DefinitionInfo> =
            LspCache::new(LspOperation::Definition, config).unwrap();

        let key = LspCacheKey::new(
            "/test/file.rs",
            10,
            5,
            "abc123",
            LspOperation::Definition,
            None,
        );

        let test_data = DefinitionInfo {
            locations: vec![LocationInfo {
                uri: "file:///test/file.rs".to_string(),
                range: RangeInfo {
                    start_line: 10,
                    start_character: 5,
                    end_line: 10,
                    end_character: 15,
                },
            }],
        };

        // Store in cache
        cache
            .get_or_compute(key.clone(), || async { Ok(test_data.clone()) })
            .await
            .unwrap();

        // Clear memory cache
        cache.entries.clear();

        // Should still be available from persistent storage
        let result = cache
            .get_or_compute(key, || async {
                panic!("Should not compute again");
            })
            .await
            .unwrap();

        assert_eq!(result.data.locations.len(), 1);
        assert_eq!(result.data.locations[0].uri, "file:///test/file.rs");
    }

    #[tokio::test]
    async fn test_cache_manager() {
        let config = LspCacheConfig::default();
        let manager = LspCacheManager::new(config.clone());

        // Register definition cache
        let def_cache: LspCache<DefinitionInfo> =
            LspCache::new(LspOperation::Definition, config.clone()).unwrap();
        manager.register_cache(LspOperation::Definition, def_cache);

        // Add some test data
        if let Some(cache) = manager.caches.get(&LspOperation::Definition) {
            // Since we have a trait object, we can't call get_or_compute directly
            // This test just verifies the registration works
            assert_eq!(cache.operation(), LspOperation::Definition);
        }

        // Test invalidation across all caches
        manager.invalidate_file(Path::new("/test/file.rs")).await;

        // Test getting stats
        let stats = manager.all_stats().await;
        assert!(stats
            .per_operation
            .iter()
            .any(|s| s.operation == LspOperation::Definition));
    }
}
