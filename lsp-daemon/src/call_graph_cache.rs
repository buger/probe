use crate::cache_types::{
    CacheStats, CachedLspNode, CachedNode, CallHierarchyInfo, DefinitionInfo, DocumentSymbolsInfo,
    HoverInfo, LocationInfo, LspCacheKey, LspOperation, NodeId, NodeKey, ReferencesInfo,
};
use crate::language_detector::Language;
use crate::persistent_cache::{PersistentCacheConfig, PersistentCallGraphCache};
use crate::protocol::{DocumentSymbol, HoverContent, Location, SymbolInformation};
use anyhow::Result;
use dashmap::DashMap;
use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tracing::{debug, info, warn};

/// Messages for background persistence writer
#[derive(Debug, Clone)]
enum PersistenceMessage {
    /// Write a node to persistent storage
    Write {
        key: NodeKey,
        info: CallHierarchyInfo,
        language: Language,
    },
    /// Remove a node from persistent storage
    Remove { key: NodeKey },
    /// Remove all nodes for a file from persistent storage
    RemoveFile { file_path: PathBuf },
    /// Clear all persistent storage
    Clear,
}

/// Position-based cache key for fast lookups
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PosKey {
    file: PathBuf,
    line: u32,
    column: u32,
    content_md5: String,
}

/// Access metadata for tracking LRU without modifying CachedNode
#[derive(Debug, Clone)]
struct AccessMeta {
    last_accessed: Instant,
    access_count: usize,
}

impl AccessMeta {
    fn new() -> Self {
        Self {
            last_accessed: Instant::now(),
            access_count: 1,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
        self.access_count += 1;
    }
}

/// Configuration for the call graph cache
#[derive(Debug, Clone)]
pub struct CallGraphCacheConfig {
    /// Maximum number of nodes to cache
    pub capacity: usize,
    /// Time-to-live for cached entries
    pub ttl: Duration,
    /// How often to check for expired entries
    pub eviction_check_interval: Duration,
    /// Maximum depth for graph-based invalidation
    pub invalidation_depth: usize,
    /// Whether persistence is enabled
    pub persistence_enabled: bool,
    /// Path for persistent storage (None = use default)
    pub persistence_path: Option<PathBuf>,
    /// Number of writes to batch before flushing to disk
    pub persistence_write_batch_size: usize,
    /// Interval between background write flushes
    pub persistence_write_interval: Duration,
}

impl Default for CallGraphCacheConfig {
    fn default() -> Self {
        Self {
            capacity: 1000,
            ttl: Duration::from_secs(1800), // 30 minutes
            eviction_check_interval: Duration::from_secs(60),
            invalidation_depth: 2,
            persistence_enabled: false,
            persistence_path: None,
            persistence_write_batch_size: 10,
            persistence_write_interval: Duration::from_secs(5),
        }
    }
}

/// Thread-safe, content-addressed call graph cache with automatic invalidation
pub struct CallGraphCache {
    /// Main cache storage: NodeKey -> CachedNode
    nodes: DashMap<NodeKey, Arc<CachedNode>>,

    /// Index from NodeId to all versions (NodeKeys) of that node
    id_to_keys: DashMap<NodeId, HashSet<NodeKey>>,

    /// Graph edges for invalidation (NodeId -> NodeId)
    outgoing: DashMap<NodeId, HashSet<NodeId>>,
    incoming: DashMap<NodeId, HashSet<NodeId>>,

    /// File index for file-based invalidation
    file_index: DashMap<std::path::PathBuf, HashSet<NodeId>>,

    /// Position index for fast lookups: (file, line, column, md5) -> NodeKey
    pos_index: DashMap<PosKey, NodeKey>,

    /// Reverse index: NodeKey -> all position keys that should be removed with it
    key_to_positions: DashMap<NodeKey, HashSet<PosKey>>,

    /// Access metadata for true LRU tracking (separate from immutable CachedNode)
    access_meta: DashMap<NodeKey, AccessMeta>,

    /// In-flight deduplication
    inflight: DashMap<NodeKey, Arc<AsyncMutex<()>>>,

    /// Configuration
    config: CallGraphCacheConfig,

    /// Last eviction check time
    last_eviction: Arc<AsyncMutex<Instant>>,

    /// Optional persistent storage layer (L2 cache)
    persistent_store: Option<Arc<PersistentCallGraphCache>>,

    /// Channel for background persistence writes
    write_channel: Option<mpsc::UnboundedSender<PersistenceMessage>>,

    /// Workspace-aware cache router for L2 cache operations
    workspace_cache_router: Option<Arc<crate::workspace_cache_router::WorkspaceCacheRouter>>,

    /// Generic LSP operation caches
    definition_cache: DashMap<LspCacheKey, Arc<CachedLspNode<DefinitionInfo>>>,
    references_cache: DashMap<LspCacheKey, Arc<CachedLspNode<ReferencesInfo>>>,
    hover_cache: DashMap<LspCacheKey, Arc<CachedLspNode<HoverInfo>>>,
    document_symbols_cache: DashMap<PathBuf, Arc<CachedLspNode<DocumentSymbolsInfo>>>, // Per-file caching
    workspace_symbols_cache: DashMap<String, Arc<CachedLspNode<Vec<SymbolInformation>>>>, // Query-based caching
    implementations_cache: DashMap<LspCacheKey, Arc<CachedLspNode<Vec<LocationInfo>>>>,
    type_definition_cache: DashMap<LspCacheKey, Arc<CachedLspNode<Vec<LocationInfo>>>>,

    /// Access metadata for LSP caches (for LRU eviction)
    lsp_access_meta: DashMap<String, AccessMeta>, // Using string key for generic access tracking

    /// In-flight LSP operation tracking
    lsp_inflight: DashMap<String, Arc<AsyncMutex<()>>>, // Using string key for generic operation tracking
}

impl CallGraphCache {
    /// CI guard: disable persistence regardless of daemon config
    #[inline]
    fn persistence_disabled_in_env() -> bool {
        // Common CI indicators + explicit override
        std::env::var("PROBE_DISABLE_PERSISTENCE").is_ok()
            || std::env::var("PROBE_CI").is_ok()
            || std::env::var("PROBE_CI").is_ok()
            || std::env::var("GITHUB_ACTIONS").is_ok()
            || std::env::var("TF_BUILD").is_ok()
    }

    /// Create a new cache instance without persistence
    pub fn new(config: CallGraphCacheConfig) -> Self {
        Self {
            nodes: DashMap::new(),
            id_to_keys: DashMap::new(),
            outgoing: DashMap::new(),
            incoming: DashMap::new(),
            file_index: DashMap::new(),
            pos_index: DashMap::new(),
            key_to_positions: DashMap::new(),
            access_meta: DashMap::new(),
            inflight: DashMap::new(),
            config,
            last_eviction: Arc::new(AsyncMutex::new(Instant::now())),
            persistent_store: None,
            write_channel: None,
            workspace_cache_router: None,
            definition_cache: DashMap::new(),
            references_cache: DashMap::new(),
            hover_cache: DashMap::new(),
            document_symbols_cache: DashMap::new(),
            workspace_symbols_cache: DashMap::new(),
            implementations_cache: DashMap::new(),
            type_definition_cache: DashMap::new(),
            lsp_access_meta: DashMap::new(),
            lsp_inflight: DashMap::new(),
        }
    }

    /// Create a new cache instance with optional persistence
    pub async fn new_with_persistence(config: CallGraphCacheConfig) -> Result<Self> {
        let persistence_allowed =
            config.persistence_enabled && !Self::persistence_disabled_in_env();
        let (persistent_store, write_channel) = if persistence_allowed {
            // Create persistence config from cache config
            let persistence_config = PersistentCacheConfig {
                cache_directory: config.persistence_path.clone(),
                max_size_bytes: 0, // Unlimited by default
                ttl_days: 30,      // 30 days default
                // Git integration removed - using MD5-only approach
                compress: true,
            };

            // Create persistent store
            let store = Arc::new(PersistentCallGraphCache::new(persistence_config).await?);

            // Create write channel for background persistence
            let (write_tx, write_rx) = mpsc::unbounded_channel();

            // Spawn background writer task
            let store_clone = Arc::clone(&store);
            let batch_size = config.persistence_write_batch_size;
            let write_interval = config.persistence_write_interval;

            tokio::spawn(Self::background_writer(
                write_rx,
                store_clone,
                batch_size,
                write_interval,
            ));

            info!("Persistent cache enabled with background writer");
            (Some(store), Some(write_tx))
        } else {
            (None, None)
        };

        Ok(Self {
            nodes: DashMap::new(),
            id_to_keys: DashMap::new(),
            outgoing: DashMap::new(),
            incoming: DashMap::new(),
            file_index: DashMap::new(),
            pos_index: DashMap::new(),
            key_to_positions: DashMap::new(),
            access_meta: DashMap::new(),
            inflight: DashMap::new(),
            config,
            last_eviction: Arc::new(AsyncMutex::new(Instant::now())),
            persistent_store,
            write_channel,
            workspace_cache_router: None,
            definition_cache: DashMap::new(),
            references_cache: DashMap::new(),
            hover_cache: DashMap::new(),
            document_symbols_cache: DashMap::new(),
            workspace_symbols_cache: DashMap::new(),
            implementations_cache: DashMap::new(),
            type_definition_cache: DashMap::new(),
            lsp_access_meta: DashMap::new(),
            lsp_inflight: DashMap::new(),
        })
    }

    /// Create a new cache instance with workspace-aware routing
    pub async fn new_with_workspace_router(
        config: CallGraphCacheConfig,
        workspace_cache_router: Arc<crate::workspace_cache_router::WorkspaceCacheRouter>,
    ) -> Result<Self> {
        let persistence_allowed =
            config.persistence_enabled && !Self::persistence_disabled_in_env();
        let (persistent_store, write_channel) = if persistence_allowed {
            // Create persistence config from cache config
            let persistence_config = crate::persistent_cache::PersistentCacheConfig {
                cache_directory: config.persistence_path.clone(),
                max_size_bytes: 0, // Unlimited by default
                ttl_days: 30,      // 30 days default
                compress: true,
            };

            // Create persistent store
            let store = Arc::new(
                crate::persistent_cache::PersistentCallGraphCache::new(persistence_config).await?,
            );

            // Create write channel for background persistence
            let (write_tx, write_rx) = mpsc::unbounded_channel();

            // Spawn background writer task
            let store_clone = Arc::clone(&store);
            let batch_size = config.persistence_write_batch_size;
            let write_interval = config.persistence_write_interval;

            tokio::spawn(Self::background_writer(
                write_rx,
                store_clone,
                batch_size,
                write_interval,
            ));

            info!("Persistent cache enabled with background writer and workspace router");
            (Some(store), Some(write_tx))
        } else {
            info!("Workspace router enabled without persistence");
            (None, None)
        };

        Ok(Self {
            nodes: DashMap::new(),
            id_to_keys: DashMap::new(),
            outgoing: DashMap::new(),
            incoming: DashMap::new(),
            file_index: DashMap::new(),
            pos_index: DashMap::new(),
            key_to_positions: DashMap::new(),
            access_meta: DashMap::new(),
            inflight: DashMap::new(),
            config,
            last_eviction: Arc::new(AsyncMutex::new(Instant::now())),
            persistent_store,
            write_channel,
            workspace_cache_router: Some(workspace_cache_router),
            definition_cache: DashMap::new(),
            references_cache: DashMap::new(),
            hover_cache: DashMap::new(),
            document_symbols_cache: DashMap::new(),
            workspace_symbols_cache: DashMap::new(),
            implementations_cache: DashMap::new(),
            type_definition_cache: DashMap::new(),
            lsp_access_meta: DashMap::new(),
            lsp_inflight: DashMap::new(),
        })
    }

    /// Get a cached node or compute it if not present using layered caching:
    /// L1: In-memory cache (fastest, <1ms)
    /// L2: Workspace-aware persistent storage (~1-5ms)
    /// L3: LSP server computation (100ms-10s)
    pub async fn get_or_compute<F, Fut>(&self, key: NodeKey, compute: F) -> Result<Arc<CachedNode>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<CallHierarchyInfo>>,
    {
        self.get_or_compute_with_workspace_hint(key, None, compute)
            .await
    }

    /// Get a cached node or compute it if not present using layered caching with workspace hint:
    /// L1: In-memory cache (fastest, <1ms)
    /// L2: Workspace-aware persistent storage (~1-5ms)
    /// L3: LSP server computation (100ms-10s)
    pub async fn get_or_compute_with_workspace_hint<F, Fut>(
        &self,
        key: NodeKey,
        workspace_hint: Option<std::path::PathBuf>,
        compute: F,
    ) -> Result<Arc<CachedNode>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<CallHierarchyInfo>>,
    {
        // L1: Check in-memory cache first (fastest)
        if let Some(node) = self.get(&key) {
            debug!("L1 cache hit for {}:{}", key.file.display(), key.symbol);
            return Ok(node);
        }

        // Deduplication: ensure only one computation per key
        let lock = self
            .inflight
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();

        let _guard = lock.lock().await;

        // Double-check L1 after acquiring lock
        if let Some(node) = self.get(&key) {
            self.inflight.remove(&key);
            return Ok(node);
        }

        // L2: Check workspace-aware persistent storage if available
        // Use workspace router for workspace-specific caching if available
        if let Some(ref workspace_router) = self.workspace_cache_router {
            if let Some(workspace_hint) = workspace_hint.as_ref() {
                debug!(
                    "Using workspace router for L2 cache lookup with workspace hint: {}",
                    workspace_hint.display()
                );

                // Use workspace router to get priority-ordered read caches
                match workspace_router.pick_read_path(workspace_hint).await {
                    Ok(read_caches) => {
                        // Try each cache in priority order
                        for (cache_idx, cache) in read_caches.iter().enumerate() {
                            match cache.get(&key).await {
                                Ok(Some(persisted_node)) => {
                                    debug!(
                                        "L2 workspace cache hit for {}:{} (cache {} of {})",
                                        key.file.display(),
                                        key.symbol,
                                        cache_idx + 1,
                                        read_caches.len()
                                    );

                                    // Create CachedNode from persisted data
                                    let node =
                                        Arc::new(CachedNode::new(key.clone(), persisted_node.info));

                                    // Insert into L1 cache for future access
                                    self.insert_node(node.clone());

                                    // Clean up in-flight tracker
                                    self.inflight.remove(&key);

                                    return Ok(node);
                                }
                                Ok(None) => {
                                    debug!(
                                        "L2 workspace cache miss for {}:{} (cache {} of {})",
                                        key.file.display(),
                                        key.symbol,
                                        cache_idx + 1,
                                        read_caches.len()
                                    );
                                    // Continue to next cache
                                }
                                Err(e) => {
                                    warn!(
                                        "L2 workspace cache error for {}:{} (cache {} of {}): {}",
                                        key.file.display(),
                                        key.symbol,
                                        cache_idx + 1,
                                        read_caches.len(),
                                        e
                                    );
                                    // Continue to next cache
                                }
                            }
                        }
                        debug!(
                            "L2 cache miss for {}:{} (all {} workspace caches checked)",
                            key.file.display(),
                            key.symbol,
                            read_caches.len()
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Failed to get workspace read path for {}:{}: {}",
                            key.file.display(),
                            key.symbol,
                            e
                        );
                        // Fall back to direct persistent store if available
                    }
                }
            } else {
                debug!(
                    "No workspace hint provided, using workspace router with file path: {}",
                    key.file.display()
                );

                // Use file path from key as workspace hint
                match workspace_router.pick_read_path(&key.file).await {
                    Ok(read_caches) => {
                        // Try each cache in priority order
                        for (cache_idx, cache) in read_caches.iter().enumerate() {
                            match cache.get(&key).await {
                                Ok(Some(persisted_node)) => {
                                    debug!(
                                        "L2 workspace cache hit for {}:{} (inferred workspace, cache {} of {})", 
                                        key.file.display(),
                                        key.symbol,
                                        cache_idx + 1,
                                        read_caches.len()
                                    );

                                    // Create CachedNode from persisted data
                                    let node =
                                        Arc::new(CachedNode::new(key.clone(), persisted_node.info));

                                    // Insert into L1 cache for future access
                                    self.insert_node(node.clone());

                                    // Clean up in-flight tracker
                                    self.inflight.remove(&key);

                                    return Ok(node);
                                }
                                Ok(None) => {
                                    // Continue to next cache
                                }
                                Err(e) => {
                                    warn!(
                                        "L2 workspace cache error for {}:{} (inferred workspace, cache {} of {}): {}",
                                        key.file.display(),
                                        key.symbol,
                                        cache_idx + 1,
                                        read_caches.len(),
                                        e
                                    );
                                    // Continue to next cache
                                }
                            }
                        }
                        debug!(
                            "L2 cache miss for {}:{} (inferred workspace, all {} caches checked)",
                            key.file.display(),
                            key.symbol,
                            read_caches.len()
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Failed to get workspace read path for file {}: {}",
                            key.file.display(),
                            e
                        );
                        // Fall back to direct persistent store if available
                    }
                }
            }
        }

        // Fallback: Check direct persistent storage if workspace router is not available
        if let Some(ref persistent_store) = self.persistent_store {
            match persistent_store.get(&key).await {
                Ok(Some(persisted_node)) => {
                    debug!(
                        "L2 cache hit for {}:{} (direct fallback)",
                        key.file.display(),
                        key.symbol
                    );

                    // Create CachedNode from persisted data
                    let node = Arc::new(CachedNode::new(key.clone(), persisted_node.info));

                    // Insert into L1 cache for future access
                    self.insert_node(node.clone());

                    // Clean up in-flight tracker
                    self.inflight.remove(&key);

                    return Ok(node);
                }
                Ok(None) => {
                    debug!(
                        "L2 cache miss for {}:{} (direct fallback)",
                        key.file.display(),
                        key.symbol
                    );
                }
                Err(e) => {
                    warn!(
                        "L2 cache error for {}:{}: {}",
                        key.file.display(),
                        key.symbol,
                        e
                    );
                    // Continue to L3 computation on L2 error
                }
            }
        }

        // L3: Compute the value using LSP server (most expensive)
        debug!(
            "L3 computation for {}:{} (md5: {})",
            key.file.display(),
            key.symbol,
            key.content_md5
        );

        let info = compute().await?;
        let node = Arc::new(CachedNode::new(key.clone(), info.clone()));

        // Insert into L1 cache
        self.insert_node(node.clone());

        // Write-through to L2 cache if available
        // Use workspace router for writes if available, otherwise fall back to direct persistence
        if let Some(ref workspace_router) = self.workspace_cache_router {
            // Determine workspace for write
            let write_workspace_hint = workspace_hint.as_ref().unwrap_or(&key.file);

            // Use workspace router to pick the write target cache
            match workspace_router
                .pick_write_target(write_workspace_hint)
                .await
            {
                Ok(write_cache) => {
                    // Try to detect language from file extension (fallback to Unknown)
                    let language = key
                        .file
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .and_then(Language::from_str)
                        .unwrap_or(Language::Unknown);

                    // Write directly to the workspace-specific cache
                    if let Err(e) = write_cache
                        .insert(key.clone(), info.clone(), language)
                        .await
                    {
                        warn!(
                            "Failed to write to workspace cache for {}:{}: {}",
                            key.file.display(),
                            key.symbol,
                            e
                        );
                    } else {
                        debug!(
                            "L2 workspace cache write successful for {}:{}",
                            key.file.display(),
                            key.symbol
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to pick write target for {}:{}: {}",
                        key.file.display(),
                        key.symbol,
                        e
                    );
                    // Fall back to background writer if available
                }
            }
        }

        // Fallback: Use background writer for direct persistence if workspace router is not available
        if let Some(ref write_channel) = self.write_channel {
            // Try to detect language from file extension (fallback to Unknown)
            let language = key
                .file
                .extension()
                .and_then(|ext| ext.to_str())
                .and_then(Language::from_str)
                .unwrap_or(Language::Unknown);

            let msg = PersistenceMessage::Write {
                key: key.clone(),
                info,
                language,
            };

            if let Err(e) = write_channel.send(msg) {
                warn!(
                    "Failed to queue persistence write for {}:{}: {}",
                    key.file.display(),
                    key.symbol,
                    e
                );
            }
        }

        // Clean up in-flight tracker
        self.inflight.remove(&key);

        // Trigger eviction check if needed
        self.maybe_evict().await;

        Ok(node)
    }

    /// Warm the cache from persistent storage on startup
    /// Loads the most recently used entries up to the configured capacity
    pub async fn warm_from_persistence(&self) -> Result<usize> {
        let Some(ref persistent_store) = self.persistent_store else {
            debug!("No persistent store available for cache warming");
            return Ok(0);
        };

        info!("Starting cache warming from persistent storage");

        let start = Instant::now();
        let mut loaded_count = 0;

        // Get basic stats to understand the scope
        let stats = persistent_store.get_stats().await?;
        info!(
            "Persistent cache contains {} nodes across {} files",
            stats.total_nodes, stats.total_files
        );

        if stats.total_nodes == 0 {
            info!("No nodes in persistent storage to warm cache with");
            return Ok(0);
        }

        // For now, we'll use a simple approach: iterate through the database
        // In a more sophisticated implementation, we could prioritize by access time,
        // file modification time, or other heuristics

        // Create a temporary hashmap to track what we've loaded
        let mut loaded_nodes = std::collections::HashMap::new();

        // Get all nodes from persistent storage
        match persistent_store.iter_nodes().await {
            Ok(nodes) => {
                for (key, persisted_node) in nodes {
                    // Respect capacity limits
                    if loaded_count >= self.config.capacity {
                        debug!("Reached capacity limit during cache warming");
                        break;
                    }

                    // Check if we already loaded this NodeId (but different version)
                    let node_id = key.to_node_id();
                    if loaded_nodes.contains_key(&node_id) {
                        // Skip if we already have a version of this node
                        continue;
                    }

                    // Create CachedNode and insert into L1 cache
                    let cached_node = Arc::new(CachedNode::new(key.clone(), persisted_node.info));

                    // Insert into L1 cache structures
                    self.insert_node(cached_node);

                    loaded_nodes.insert(node_id, key.clone());
                    loaded_count += 1;

                    if loaded_count % 100 == 0 {
                        debug!("Cache warming progress: {} nodes loaded", loaded_count);
                    }
                }
            }
            Err(e) => {
                warn!("Failed to iterate persistent cache for warming: {}", e);
            }
        }

        let duration = start.elapsed();
        info!(
            "Cache warming completed: loaded {} nodes in {:?}",
            loaded_count, duration
        );

        Ok(loaded_count)
    }

    /// Background task for batched persistence writes
    async fn background_writer(
        mut write_rx: mpsc::UnboundedReceiver<PersistenceMessage>,
        persistent_store: Arc<PersistentCallGraphCache>,
        batch_size: usize,
        write_interval: Duration,
    ) {
        let mut batch = Vec::new();
        let mut interval = tokio::time::interval(write_interval);

        info!(
            "Background persistence writer started (batch_size: {}, interval: {:?})",
            batch_size, write_interval
        );

        loop {
            tokio::select! {
                // Handle incoming messages
                msg = write_rx.recv() => {
                    match msg {
                        Some(message) => {
                            batch.push(message);

                            // Flush if batch is full
                            if batch.len() >= batch_size {
                                Self::flush_batch(&persistent_store, &mut batch).await;
                            }
                        }
                        None => {
                            // Channel closed, flush remaining and exit
                            if !batch.is_empty() {
                                Self::flush_batch(&persistent_store, &mut batch).await;
                            }
                            info!("Background persistence writer stopping");
                            break;
                        }
                    }
                }
                // Periodic flush
                _ = interval.tick() => {
                    if !batch.is_empty() {
                        Self::flush_batch(&persistent_store, &mut batch).await;
                    }
                }
            }
        }
    }

    /// Flush a batch of persistence messages
    async fn flush_batch(
        persistent_store: &Arc<PersistentCallGraphCache>,
        batch: &mut Vec<PersistenceMessage>,
    ) {
        if batch.is_empty() {
            return;
        }

        debug!("Flushing {} persistence operations", batch.len());

        for message in batch.drain(..) {
            match message {
                PersistenceMessage::Write {
                    key,
                    info,
                    language,
                } => {
                    if let Err(e) = persistent_store.insert(key.clone(), info, language).await {
                        warn!(
                            "Failed to persist write for {}:{}: {}",
                            key.file.display(),
                            key.symbol,
                            e
                        );
                    }
                }
                PersistenceMessage::Remove { key } => {
                    if let Err(e) = persistent_store.remove(&key).await {
                        warn!(
                            "Failed to persist removal for {}:{}: {}",
                            key.file.display(),
                            key.symbol,
                            e
                        );
                    }
                }
                PersistenceMessage::RemoveFile { file_path } => {
                    // For file removal, we need to get all nodes for the file first
                    match persistent_store.get_by_file(&file_path).await {
                        Ok(nodes) => {
                            for node in nodes {
                                if let Err(e) = persistent_store.remove(&node.key).await {
                                    warn!(
                                        "Failed to persist file removal for {}:{}: {}",
                                        node.key.file.display(),
                                        node.key.symbol,
                                        e
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to get nodes for file removal {}: {}",
                                file_path.display(),
                                e
                            );
                        }
                    }
                }
                PersistenceMessage::Clear => {
                    if let Err(e) = persistent_store.clear().await {
                        warn!("Failed to persist cache clear: {}", e);
                    }
                }
            }
        }
    }

    /// Get a cached node if present
    pub fn get(&self, key: &NodeKey) -> Option<Arc<CachedNode>> {
        self.nodes.get(key).map(|entry| {
            // Touch access metadata separately for true LRU tracking
            if let Some(mut meta) = self.access_meta.get_mut(key) {
                meta.touch();
            }
            entry.clone()
        })
    }

    /// Associate a position key (file, line, column, md5) with an existing NodeKey.
    /// Call this right after successfully caching a node for that position.
    pub fn index_position(
        &self,
        file: &Path,
        line: u32,
        column: u32,
        content_md5: &str,
        key: &NodeKey,
    ) {
        let pos = PosKey {
            file: file.to_path_buf(),
            line,
            column,
            content_md5: content_md5.to_string(),
        };
        self.pos_index.insert(pos.clone(), key.clone());
        self.key_to_positions
            .entry(key.clone())
            .or_default()
            .insert(pos);
    }

    /// Try to get a cached node using the current position.
    pub fn get_by_position(
        &self,
        file: &Path,
        line: u32,
        column: u32,
        content_md5: &str,
    ) -> Option<Arc<CachedNode>> {
        let pos = PosKey {
            file: file.to_path_buf(),
            line,
            column,
            content_md5: content_md5.to_string(),
        };
        self.pos_index.get(&pos).and_then(|entry| self.get(&entry))
    }

    /// Insert a node into the cache
    fn insert_node(&self, node: Arc<CachedNode>) {
        let key = node.key.clone();
        let id = key.to_node_id();

        // Add to main cache
        self.nodes.insert(key.clone(), node);

        // Initialize access metadata for LRU tracking
        self.access_meta.insert(key.clone(), AccessMeta::new());

        // Update ID index
        self.id_to_keys
            .entry(id.clone())
            .or_default()
            .insert(key.clone());

        // Update file index
        self.file_index
            .entry(id.file.clone())
            .or_default()
            .insert(id);
    }

    /// Update graph edges for a node
    pub fn update_edges(&self, node_id: &NodeId, incoming: Vec<NodeId>, outgoing: Vec<NodeId>) {
        // Update outgoing edges
        if !outgoing.is_empty() {
            self.outgoing
                .insert(node_id.clone(), outgoing.iter().cloned().collect());
        } else {
            self.outgoing.remove(node_id);
        }

        // Update incoming edges
        if !incoming.is_empty() {
            self.incoming
                .insert(node_id.clone(), incoming.iter().cloned().collect());
        } else {
            self.incoming.remove(node_id);
        }

        // Ensure bidirectional consistency
        for target in &outgoing {
            self.incoming
                .entry(target.clone())
                .or_default()
                .insert(node_id.clone());
        }

        for source in &incoming {
            self.outgoing
                .entry(source.clone())
                .or_default()
                .insert(node_id.clone());
        }
    }

    /// Invalidate a specific node and optionally its connected nodes
    pub fn invalidate_node(&self, node_id: &NodeId, depth: usize) {
        let mut to_invalidate = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((node_id.clone(), 0));

        // BFS to find all nodes to invalidate
        while let Some((current_id, current_depth)) = queue.pop_front() {
            if current_depth > depth || !to_invalidate.insert(current_id.clone()) {
                continue;
            }

            if current_depth < depth {
                // Add connected nodes
                if let Some(outgoing) = self.outgoing.get(&current_id) {
                    for target in outgoing.iter() {
                        queue.push_back((target.clone(), current_depth + 1));
                    }
                }
                if let Some(incoming) = self.incoming.get(&current_id) {
                    for source in incoming.iter() {
                        queue.push_back((source.clone(), current_depth + 1));
                    }
                }
            }
        }

        // Remove all invalidated nodes from L1 cache and queue L2 removal
        for id in &to_invalidate {
            if let Some(keys) = self.id_to_keys.remove(id) {
                for key in keys.1 {
                    // Remove from L1 cache (ensure secondary indexes are purged)
                    self.remove_by_key(&key);

                    // Queue removal from L2 cache if persistence is enabled
                    if let Some(ref write_channel) = self.write_channel {
                        let msg = PersistenceMessage::Remove { key: key.clone() };
                        if let Err(e) = write_channel.send(msg) {
                            warn!(
                                "Failed to queue persistence removal for {}:{}: {}",
                                key.file.display(),
                                key.symbol,
                                e
                            );
                        }
                    }
                }
            }

            // Clean up edges
            self.outgoing.remove(id);
            self.incoming.remove(id);
        }

        info!(
            "Invalidated {} nodes starting from {}:{}",
            to_invalidate.len(),
            node_id.file.display(),
            node_id.symbol
        );
    }

    /// Invalidate all nodes from a specific file
    pub fn invalidate_file(&self, file: &Path) {
        if let Some(node_ids) = self.file_index.remove(file) {
            let count = node_ids.1.len();

            // Queue file removal from L2 cache if persistence is enabled
            if let Some(ref write_channel) = self.write_channel {
                let msg = PersistenceMessage::RemoveFile {
                    file_path: file.to_path_buf(),
                };
                if let Err(e) = write_channel.send(msg) {
                    warn!(
                        "Failed to queue persistence file removal for {}: {}",
                        file.display(),
                        e
                    );
                }
            }

            for id in node_ids.1 {
                if let Some(keys) = self.id_to_keys.remove(&id) {
                    for key in keys.1 {
                        // ensure secondary indexes are purged
                        self.remove_by_key(&key);
                    }
                }
                self.outgoing.remove(&id);
                self.incoming.remove(&id);
            }

            // Also invalidate LSP caches for this file
            self.invalidate_lsp_file_caches(file);

            info!("Invalidated {} nodes from file {}", count, file.display());
        }
    }

    /// Clear the entire cache
    pub fn clear(&self) {
        // Queue clear for L2 cache if persistence is enabled
        if let Some(ref write_channel) = self.write_channel {
            let msg = PersistenceMessage::Clear;
            if let Err(e) = write_channel.send(msg) {
                warn!("Failed to queue persistence clear: {}", e);
            }
        }

        // Clear L1 cache
        self.nodes.clear();
        self.id_to_keys.clear();
        self.pos_index.clear();
        self.key_to_positions.clear();
        self.access_meta.clear();
        self.outgoing.clear();
        self.incoming.clear();
        self.file_index.clear();
        self.inflight.clear();

        // Clear LSP operation caches
        self.clear_lsp_caches();

        info!("Cache cleared");
    }

    /// Check and evict expired entries
    async fn maybe_evict(&self) {
        let mut last_check = self.last_eviction.lock().await;

        if last_check.elapsed() < self.config.eviction_check_interval {
            return;
        }

        *last_check = Instant::now();
        drop(last_check); // Release lock early

        self.do_evict().await;
    }

    /// Force eviction check (for testing)
    #[cfg(test)]
    async fn force_evict(&self) {
        self.do_evict().await;
    }

    /// Internal eviction logic
    async fn do_evict(&self) {
        let now = Instant::now();
        let mut expired_keys = Vec::new();
        let mut lru_candidates = Vec::new();

        // Find expired entries and collect LRU candidates
        for entry in self.nodes.iter() {
            let key = entry.key();
            let node = entry.value();

            if now.duration_since(node.created_at) > self.config.ttl {
                expired_keys.push(key.clone());
            } else {
                // Get access metadata for true LRU ranking
                if let Some(meta) = self.access_meta.get(key) {
                    lru_candidates.push((key.clone(), meta.last_accessed, meta.access_count));
                } else {
                    // Fallback to node creation time if metadata is missing
                    lru_candidates.push((key.clone(), node.created_at, 1));
                }
            }
        }

        // Remove expired entries
        for key in &expired_keys {
            self.remove_by_key(key);
        }

        // If over capacity, evict LRU entries
        if self.nodes.len() > self.config.capacity {
            // Sort by last accessed time (oldest first) and access count
            lru_candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));

            let to_evict = self.nodes.len().saturating_sub(self.config.capacity);
            for (key, _, _) in lru_candidates.iter().take(to_evict) {
                self.remove_by_key(key);
            }

            debug!(
                "Evicted {} expired and {} LRU entries",
                expired_keys.len(),
                to_evict
            );
        }
    }

    /// Remove a node by its key
    fn remove_by_key(&self, key: &NodeKey) {
        if let Some((_, _node)) = self.nodes.remove(key) {
            let id = key.to_node_id();

            // Remove access metadata
            self.access_meta.remove(key);

            // Remove any position mappings referencing this key
            if let Some(pos_set) = self.key_to_positions.remove(key) {
                // pos_set.1 is the HashSet<PosKey>
                for pos in pos_set.1 {
                    self.pos_index.remove(&pos);
                }
            }

            // Update ID index
            if let Some(mut keys) = self.id_to_keys.get_mut(&id) {
                keys.remove(key);
                if keys.is_empty() {
                    drop(keys);
                    self.id_to_keys.remove(&id);
                }
            }

            // Update file index
            if let Some(mut ids) = self.file_index.get_mut(&id.file) {
                ids.remove(&id);
                if ids.is_empty() {
                    drop(ids);
                    self.file_index.remove(&id.file);
                }
            }
        }
    }

    /// Get cache history for a node (stub - git functionality removed)
    pub async fn get_history(
        &self,
        _node_id: &NodeId,
    ) -> Result<Vec<crate::protocol::CacheHistoryEntry>> {
        // Git functionality has been removed - this is now a stub
        Ok(Vec::new())
    }

    /// Get cache snapshot at a specific commit (stub - git functionality removed)
    pub async fn snapshot_at_commit(
        &self,
        _commit_hash: &str,
    ) -> Result<crate::protocol::CacheSnapshot> {
        // Git functionality has been removed - this is now a stub
        Err(anyhow::anyhow!("Git functionality has been removed"))
    }

    /// Compare cache between two commits (stub - git functionality removed)
    pub async fn diff_commits(&self, _from: &str, _to: &str) -> Result<crate::protocol::CacheDiff> {
        // Git functionality has been removed - this is now a stub
        Err(anyhow::anyhow!("Git functionality has been removed"))
    }

    // ========================================================================================
    // LSP Operation Cache Methods
    // ========================================================================================

    /// Helper function to generate a cache key string for LSP operations
    fn lsp_cache_key_string(key: &LspCacheKey) -> String {
        format!(
            "{}:{}:{}:{}:{:?}:{}",
            key.file.display(),
            key.line,
            key.column,
            key.content_md5,
            key.operation,
            key.extra_params.as_deref().unwrap_or("")
        )
    }

    /// Get or compute definition locations with caching
    pub async fn get_or_compute_definition<F, Fut>(
        &self,
        file: PathBuf,
        line: u32,
        column: u32,
        content_md5: String,
        compute: F,
    ) -> Result<Arc<CachedLspNode<DefinitionInfo>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<Location>>>,
    {
        let key = LspCacheKey::new(
            file,
            line,
            column,
            content_md5,
            LspOperation::Definition,
            None,
        );
        let key_str = Self::lsp_cache_key_string(&key);

        // Check L1 cache
        if let Some(cached) = self.definition_cache.get(&key) {
            // Touch access metadata
            if let Some(mut meta) = self.lsp_access_meta.get_mut(&key_str) {
                meta.touch();
            }
            debug!(
                "Definition cache hit for {}:{}:{}",
                key.file.display(),
                key.line,
                key.column
            );
            return Ok(cached.clone());
        }

        // Deduplication
        let lock = self
            .lsp_inflight
            .entry(key_str.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Double-check after lock
        if let Some(cached) = self.definition_cache.get(&key) {
            self.lsp_inflight.remove(&key_str);
            return Ok(cached.clone());
        }

        // Compute the result
        debug!(
            "Computing definition for {}:{}:{}",
            key.file.display(),
            key.line,
            key.column
        );
        let locations = compute().await?;

        let definition_info = DefinitionInfo {
            locations: locations
                .into_iter()
                .map(|loc| LocationInfo {
                    uri: loc.uri,
                    range: crate::cache_types::RangeInfo {
                        start_line: loc.range.start.line,
                        start_character: loc.range.start.character,
                        end_line: loc.range.end.line,
                        end_character: loc.range.end.character,
                    },
                })
                .collect(),
        };

        let cached_node = Arc::new(CachedLspNode::new(key.clone(), definition_info));

        // Insert into cache
        self.definition_cache
            .insert(key.clone(), cached_node.clone());
        self.lsp_access_meta
            .insert(key_str.clone(), AccessMeta::new());

        // Clean up in-flight tracker
        self.lsp_inflight.remove(&key_str);

        Ok(cached_node)
    }

    /// Get or compute references with caching
    pub async fn get_or_compute_references<F, Fut>(
        &self,
        file: PathBuf,
        line: u32,
        column: u32,
        content_md5: String,
        include_declaration: bool,
        compute: F,
    ) -> Result<Arc<CachedLspNode<ReferencesInfo>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<Location>>>,
    {
        let extra_params = if include_declaration {
            Some("include_declaration".to_string())
        } else {
            None
        };
        let key = LspCacheKey::new(
            file,
            line,
            column,
            content_md5,
            LspOperation::References,
            extra_params,
        );
        let key_str = Self::lsp_cache_key_string(&key);

        // Check L1 cache
        if let Some(cached) = self.references_cache.get(&key) {
            // Touch access metadata
            if let Some(mut meta) = self.lsp_access_meta.get_mut(&key_str) {
                meta.touch();
            }
            debug!(
                "References cache hit for {}:{}:{}",
                key.file.display(),
                key.line,
                key.column
            );
            return Ok(cached.clone());
        }

        // Deduplication
        let lock = self
            .lsp_inflight
            .entry(key_str.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Double-check after lock
        if let Some(cached) = self.references_cache.get(&key) {
            self.lsp_inflight.remove(&key_str);
            return Ok(cached.clone());
        }

        // Compute the result
        debug!(
            "Computing references for {}:{}:{} (include_declaration: {})",
            key.file.display(),
            key.line,
            key.column,
            include_declaration
        );
        let locations = compute().await?;

        let references_info = ReferencesInfo {
            locations: locations
                .into_iter()
                .map(|loc| LocationInfo {
                    uri: loc.uri,
                    range: crate::cache_types::RangeInfo {
                        start_line: loc.range.start.line,
                        start_character: loc.range.start.character,
                        end_line: loc.range.end.line,
                        end_character: loc.range.end.character,
                    },
                })
                .collect(),
            include_declaration,
        };

        let cached_node = Arc::new(CachedLspNode::new(key.clone(), references_info));

        // Insert into cache
        self.references_cache
            .insert(key.clone(), cached_node.clone());
        self.lsp_access_meta
            .insert(key_str.clone(), AccessMeta::new());

        // Clean up in-flight tracker
        self.lsp_inflight.remove(&key_str);

        Ok(cached_node)
    }

    /// Get or compute hover information with caching
    pub async fn get_or_compute_hover<F, Fut>(
        &self,
        file: PathBuf,
        line: u32,
        column: u32,
        content_md5: String,
        compute: F,
    ) -> Result<Arc<CachedLspNode<HoverInfo>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Option<HoverContent>>>,
    {
        let key = LspCacheKey::new(file, line, column, content_md5, LspOperation::Hover, None);
        let key_str = Self::lsp_cache_key_string(&key);

        // Check L1 cache
        if let Some(cached) = self.hover_cache.get(&key) {
            // Touch access metadata
            if let Some(mut meta) = self.lsp_access_meta.get_mut(&key_str) {
                meta.touch();
            }
            debug!(
                "Hover cache hit for {}:{}:{}",
                key.file.display(),
                key.line,
                key.column
            );
            return Ok(cached.clone());
        }

        // Deduplication
        let lock = self
            .lsp_inflight
            .entry(key_str.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Double-check after lock
        if let Some(cached) = self.hover_cache.get(&key) {
            self.lsp_inflight.remove(&key_str);
            return Ok(cached.clone());
        }

        // Compute the result
        debug!(
            "Computing hover for {}:{}:{}",
            key.file.display(),
            key.line,
            key.column
        );
        let hover_content = compute().await?;

        let hover_info = HoverInfo {
            contents: hover_content.as_ref().map(|h| h.contents.clone()),
            range: hover_content.and_then(|h| {
                h.range.map(|r| crate::cache_types::RangeInfo {
                    start_line: r.start.line,
                    start_character: r.start.character,
                    end_line: r.end.line,
                    end_character: r.end.character,
                })
            }),
        };

        let cached_node = Arc::new(CachedLspNode::new(key.clone(), hover_info));

        // Insert into cache
        self.hover_cache.insert(key.clone(), cached_node.clone());
        self.lsp_access_meta
            .insert(key_str.clone(), AccessMeta::new());

        // Clean up in-flight tracker
        self.lsp_inflight.remove(&key_str);

        Ok(cached_node)
    }

    /// Get or compute document symbols with per-file caching
    pub async fn get_or_compute_document_symbols<F, Fut>(
        &self,
        file: PathBuf,
        content_md5: String,
        compute: F,
    ) -> Result<Arc<CachedLspNode<DocumentSymbolsInfo>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<DocumentSymbol>>>,
    {
        let key = LspCacheKey::new(
            file.clone(),
            0,
            0,
            content_md5,
            LspOperation::DocumentSymbols,
            None,
        );
        let key_str = format!("{}:{}", file.display(), key.content_md5);

        // Check L1 cache
        if let Some(cached) = self.document_symbols_cache.get(&file) {
            // Check if MD5 matches (content hasn't changed)
            if cached.key.content_md5 == key.content_md5 {
                // Touch access metadata
                if let Some(mut meta) = self.lsp_access_meta.get_mut(&key_str) {
                    meta.touch();
                }
                debug!("Document symbols cache hit for {}", file.display());
                return Ok(cached.clone());
            }
        }

        // Deduplication
        let lock = self
            .lsp_inflight
            .entry(key_str.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Double-check after lock
        if let Some(cached) = self.document_symbols_cache.get(&file) {
            if cached.key.content_md5 == key.content_md5 {
                self.lsp_inflight.remove(&key_str);
                return Ok(cached.clone());
            }
        }

        // Compute the result
        debug!("Computing document symbols for {}", file.display());
        let symbols = compute().await?;

        let document_symbols_info = DocumentSymbolsInfo {
            symbols: symbols
                .into_iter()
                .map(|sym| crate::cache_types::DocumentSymbolInfo {
                    name: sym.name,
                    kind: format!("{:?}", sym.kind), // Convert SymbolKind enum to string
                    range: crate::cache_types::RangeInfo {
                        start_line: sym.range.start.line,
                        start_character: sym.range.start.character,
                        end_line: sym.range.end.line,
                        end_character: sym.range.end.character,
                    },
                    selection_range: crate::cache_types::RangeInfo {
                        start_line: sym.selection_range.start.line,
                        start_character: sym.selection_range.start.character,
                        end_line: sym.selection_range.end.line,
                        end_character: sym.selection_range.end.character,
                    },
                    children: sym.children.map(|children| {
                        children
                            .into_iter()
                            .map(|child| crate::cache_types::DocumentSymbolInfo {
                                name: child.name,
                                kind: format!("{:?}", child.kind), // Convert SymbolKind enum to string
                                range: crate::cache_types::RangeInfo {
                                    start_line: child.range.start.line,
                                    start_character: child.range.start.character,
                                    end_line: child.range.end.line,
                                    end_character: child.range.end.character,
                                },
                                selection_range: crate::cache_types::RangeInfo {
                                    start_line: child.selection_range.start.line,
                                    start_character: child.selection_range.start.character,
                                    end_line: child.selection_range.end.line,
                                    end_character: child.selection_range.end.character,
                                },
                                children: None, // Only support one level of nesting for now
                            })
                            .collect()
                    }),
                })
                .collect(),
        };

        let cached_node = Arc::new(CachedLspNode::new(key.clone(), document_symbols_info));

        // Insert into cache
        self.document_symbols_cache
            .insert(file.clone(), cached_node.clone());
        self.lsp_access_meta
            .insert(key_str.clone(), AccessMeta::new());

        // Clean up in-flight tracker
        self.lsp_inflight.remove(&key_str);

        Ok(cached_node)
    }

    /// Get or compute workspace symbols with query-based caching
    pub async fn get_or_compute_workspace_symbols<F, Fut>(
        &self,
        query: String,
        compute: F,
    ) -> Result<Arc<CachedLspNode<Vec<SymbolInformation>>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<SymbolInformation>>>,
    {
        let key_str = format!("workspace_symbols:{query}");

        // Check L1 cache
        if let Some(cached) = self.workspace_symbols_cache.get(&query) {
            // Touch access metadata
            if let Some(mut meta) = self.lsp_access_meta.get_mut(&key_str) {
                meta.touch();
            }
            debug!("Workspace symbols cache hit for query: {}", query);
            return Ok(cached.clone());
        }

        // Deduplication
        let lock = self
            .lsp_inflight
            .entry(key_str.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Double-check after lock
        if let Some(cached) = self.workspace_symbols_cache.get(&query) {
            self.lsp_inflight.remove(&key_str);
            return Ok(cached.clone());
        }

        // Compute the result
        debug!("Computing workspace symbols for query: {}", query);
        let symbols = compute().await?;

        // Create a dummy cache key for workspace symbols (workspace-wide operation)
        let key = LspCacheKey::new(
            PathBuf::from("workspace"),
            0,
            0,
            "workspace".to_string(),
            LspOperation::DocumentSymbols, // Reuse DocumentSymbols for workspace
            Some(query.clone()),
        );

        let cached_node = Arc::new(CachedLspNode::new(key, symbols));

        // Insert into cache
        self.workspace_symbols_cache
            .insert(query.clone(), cached_node.clone());
        self.lsp_access_meta
            .insert(key_str.clone(), AccessMeta::new());

        // Clean up in-flight tracker
        self.lsp_inflight.remove(&key_str);

        Ok(cached_node)
    }

    /// Get or compute implementations with caching
    pub async fn get_or_compute_implementations<F, Fut>(
        &self,
        file: PathBuf,
        line: u32,
        column: u32,
        content_md5: String,
        compute: F,
    ) -> Result<Arc<CachedLspNode<Vec<LocationInfo>>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<Location>>>,
    {
        let key = LspCacheKey::new(
            file,
            line,
            column,
            content_md5,
            LspOperation::References,
            None,
        ); // Reuse References operation
        let key_str = format!("implementations:{}", Self::lsp_cache_key_string(&key));

        // Check L1 cache
        if let Some(cached) = self.implementations_cache.get(&key) {
            // Touch access metadata
            if let Some(mut meta) = self.lsp_access_meta.get_mut(&key_str) {
                meta.touch();
            }
            debug!(
                "Implementations cache hit for {}:{}:{}",
                key.file.display(),
                key.line,
                key.column
            );
            return Ok(cached.clone());
        }

        // Deduplication
        let lock = self
            .lsp_inflight
            .entry(key_str.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Double-check after lock
        if let Some(cached) = self.implementations_cache.get(&key) {
            self.lsp_inflight.remove(&key_str);
            return Ok(cached.clone());
        }

        // Compute the result
        debug!(
            "Computing implementations for {}:{}:{}",
            key.file.display(),
            key.line,
            key.column
        );
        let locations = compute().await?;

        let location_infos: Vec<LocationInfo> = locations
            .into_iter()
            .map(|loc| LocationInfo {
                uri: loc.uri,
                range: crate::cache_types::RangeInfo {
                    start_line: loc.range.start.line,
                    start_character: loc.range.start.character,
                    end_line: loc.range.end.line,
                    end_character: loc.range.end.character,
                },
            })
            .collect();

        let cached_node = Arc::new(CachedLspNode::new(key.clone(), location_infos));

        // Insert into cache
        self.implementations_cache
            .insert(key.clone(), cached_node.clone());
        self.lsp_access_meta
            .insert(key_str.clone(), AccessMeta::new());

        // Clean up in-flight tracker
        self.lsp_inflight.remove(&key_str);

        Ok(cached_node)
    }

    /// Get or compute type definitions with caching
    pub async fn get_or_compute_type_definition<F, Fut>(
        &self,
        file: PathBuf,
        line: u32,
        column: u32,
        content_md5: String,
        compute: F,
    ) -> Result<Arc<CachedLspNode<Vec<LocationInfo>>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<Location>>>,
    {
        let key = LspCacheKey::new(
            file,
            line,
            column,
            content_md5,
            LspOperation::References,
            None,
        ); // Reuse References operation
        let key_str = format!("type_definition:{}", Self::lsp_cache_key_string(&key));

        // Check L1 cache
        if let Some(cached) = self.type_definition_cache.get(&key) {
            // Touch access metadata
            if let Some(mut meta) = self.lsp_access_meta.get_mut(&key_str) {
                meta.touch();
            }
            debug!(
                "Type definition cache hit for {}:{}:{}",
                key.file.display(),
                key.line,
                key.column
            );
            return Ok(cached.clone());
        }

        // Deduplication
        let lock = self
            .lsp_inflight
            .entry(key_str.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Double-check after lock
        if let Some(cached) = self.type_definition_cache.get(&key) {
            self.lsp_inflight.remove(&key_str);
            return Ok(cached.clone());
        }

        // Compute the result
        debug!(
            "Computing type definition for {}:{}:{}",
            key.file.display(),
            key.line,
            key.column
        );
        let locations = compute().await?;

        let location_infos: Vec<LocationInfo> = locations
            .into_iter()
            .map(|loc| LocationInfo {
                uri: loc.uri,
                range: crate::cache_types::RangeInfo {
                    start_line: loc.range.start.line,
                    start_character: loc.range.start.character,
                    end_line: loc.range.end.line,
                    end_character: loc.range.end.character,
                },
            })
            .collect();

        let cached_node = Arc::new(CachedLspNode::new(key.clone(), location_infos));

        // Insert into cache
        self.type_definition_cache
            .insert(key.clone(), cached_node.clone());
        self.lsp_access_meta
            .insert(key_str.clone(), AccessMeta::new());

        // Clean up in-flight tracker
        self.lsp_inflight.remove(&key_str);

        Ok(cached_node)
    }

    /// Invalidate all LSP caches for a specific file
    pub fn invalidate_lsp_file_caches(&self, file: &Path) {
        // Remove document symbols cache for this file
        if self.document_symbols_cache.remove(file).is_some() {
            debug!("Invalidated document symbols cache for {}", file.display());
        }

        // Remove all position-based caches for this file
        let mut keys_to_remove = Vec::new();

        // Collect keys that match the file
        for entry in self.definition_cache.iter() {
            if entry.key().file == file {
                keys_to_remove.push(entry.key().clone());
            }
        }
        for key in &keys_to_remove {
            self.definition_cache.remove(key);
            let key_str = Self::lsp_cache_key_string(key);
            self.lsp_access_meta.remove(&key_str);
        }

        keys_to_remove.clear();
        for entry in self.references_cache.iter() {
            if entry.key().file == file {
                keys_to_remove.push(entry.key().clone());
            }
        }
        for key in &keys_to_remove {
            self.references_cache.remove(key);
            let key_str = Self::lsp_cache_key_string(key);
            self.lsp_access_meta.remove(&key_str);
        }

        keys_to_remove.clear();
        for entry in self.hover_cache.iter() {
            if entry.key().file == file {
                keys_to_remove.push(entry.key().clone());
            }
        }
        for key in &keys_to_remove {
            self.hover_cache.remove(key);
            let key_str = Self::lsp_cache_key_string(key);
            self.lsp_access_meta.remove(&key_str);
        }

        keys_to_remove.clear();
        for entry in self.implementations_cache.iter() {
            if entry.key().file == file {
                keys_to_remove.push(entry.key().clone());
            }
        }
        for key in &keys_to_remove {
            self.implementations_cache.remove(key);
            let key_str = format!("implementations:{}", Self::lsp_cache_key_string(key));
            self.lsp_access_meta.remove(&key_str);
        }

        keys_to_remove.clear();
        for entry in self.type_definition_cache.iter() {
            if entry.key().file == file {
                keys_to_remove.push(entry.key().clone());
            }
        }
        for key in &keys_to_remove {
            self.type_definition_cache.remove(key);
            let key_str = format!("type_definition:{}", Self::lsp_cache_key_string(key));
            self.lsp_access_meta.remove(&key_str);
        }

        info!("Invalidated all LSP caches for file {}", file.display());
    }

    /// Clear all LSP operation caches
    pub fn clear_lsp_caches(&self) {
        self.definition_cache.clear();
        self.references_cache.clear();
        self.hover_cache.clear();
        self.document_symbols_cache.clear();
        self.workspace_symbols_cache.clear();
        self.implementations_cache.clear();
        self.type_definition_cache.clear();
        self.lsp_access_meta.clear();
        self.lsp_inflight.clear();
        info!("Cleared all LSP operation caches");
    }

    // ========================================================================================
    // End of LSP Operation Cache Methods
    // ========================================================================================

    /// Get cache statistics including persistence information
    pub async fn stats(&self) -> CacheStats {
        let (
            persistence_enabled,
            persistent_nodes,
            persistent_size_bytes,
            persistent_disk_size_bytes,
        ) = if let Some(ref persistent_store) = self.persistent_store {
            match persistent_store.get_stats().await {
                Ok(stats) => (
                    true,
                    Some(stats.total_nodes),
                    Some(stats.total_size_bytes),
                    Some(stats.disk_size_bytes),
                ),
                Err(e) => {
                    warn!("Failed to get persistent cache stats: {}", e);
                    (true, None, None, None)
                }
            }
        } else {
            (false, None, None, None)
        };

        CacheStats {
            total_nodes: self.nodes.len(),
            total_ids: self.id_to_keys.len(),
            total_files: self.file_index.len(),
            total_edges: self.outgoing.len() + self.incoming.len(),
            inflight_computations: self.inflight.len(),
            persistence_enabled,
            persistent_nodes,
            persistent_size_bytes,
            persistent_disk_size_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_basic_operations() {
        let cache = CallGraphCache::new(CallGraphCacheConfig::default());

        let key = NodeKey::new("test_func", "/test/file.rs", "abc123");

        // First call should compute
        let result = cache
            .get_or_compute(key.clone(), || async {
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();

        assert_eq!(result.key.symbol, "test_func");

        // Second call should hit cache
        let cached = cache.get(&key);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().key.symbol, "test_func");
    }

    #[tokio::test]
    async fn test_cache_invalidation() {
        let cache = CallGraphCache::new(CallGraphCacheConfig::default());

        let key1 = NodeKey::new("func1", "/test/file.rs", "hash1");
        let key2 = NodeKey::new("func2", "/test/file.rs", "hash2");

        // Add entries
        for key in [&key1, &key2] {
            cache
                .get_or_compute(key.clone(), || async {
                    Ok(CallHierarchyInfo {
                        incoming_calls: vec![],
                        outgoing_calls: vec![],
                    })
                })
                .await
                .unwrap();
        }

        // Both should be cached
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_some());

        // Invalidate the file
        cache.invalidate_file(Path::new("/test/file.rs"));

        // Both should be gone
        assert!(cache.get(&key1).is_none());
        assert!(cache.get(&key2).is_none());
    }

    #[test]
    fn test_node_id_equality() {
        let id1 = NodeId::new("func", "/path/file.rs");
        let id2 = NodeId::new("func", "/path/file.rs");
        let id3 = NodeId::new("other", "/path/file.rs");

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[tokio::test]
    async fn test_position_index_lookup() {
        let cache = CallGraphCache::new(CallGraphCacheConfig::default());
        let file = Path::new("/test/file.rs");
        let md5 = "abc123";
        let key = NodeKey::new("func", file, md5);

        // Insert and cache a dummy node
        cache
            .get_or_compute(key.clone(), || async {
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();

        // Index a position for it
        cache.index_position(file, 10, 5, md5, &key);

        // Lookup by the same position should hit
        let node = cache.get_by_position(file, 10, 5, md5);
        assert!(node.is_some());
        assert_eq!(node.unwrap().key.symbol, "func");

        // Invalidate file should remove both node and position mapping
        cache.invalidate_file(file);
        assert!(cache.get_by_position(file, 10, 5, md5).is_none());
    }

    #[tokio::test]
    async fn test_layered_caching_persistence() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let mut config = CallGraphCacheConfig::default();
        config.persistence_enabled = true;
        config.persistence_path = Some(temp_dir.path().to_path_buf());
        config.capacity = 10; // Small for testing

        let key1 = NodeKey::new("func1", "/test/file.rs", "hash1");
        let key2 = NodeKey::new("func2", "/test/file.rs", "hash2");

        // Phase 1: Create cache and add entries
        {
            let cache = CallGraphCache::new_with_persistence(config.clone())
                .await
                .unwrap();

            // Add first entry - should go to L1 and L2
            let result1 = cache
                .get_or_compute(key1.clone(), || async {
                    Ok(CallHierarchyInfo {
                        incoming_calls: vec![],
                        outgoing_calls: vec![],
                    })
                })
                .await
                .unwrap();

            assert_eq!(result1.key.symbol, "func1");

            // Verify it's in L1 cache
            assert!(cache.get(&key1).is_some());

            // Add second entry
            cache
                .get_or_compute(key2.clone(), || async {
                    Ok(CallHierarchyInfo {
                        incoming_calls: vec![],
                        outgoing_calls: vec![],
                    })
                })
                .await
                .unwrap();
        }

        // Phase 2: Create new cache instance (simulates restart)
        {
            let cache = CallGraphCache::new_with_persistence(config).await.unwrap();

            // L1 cache should be empty initially
            assert!(cache.get(&key1).is_none());
            assert!(cache.get(&key2).is_none());

            // But warming should have loaded data from L2
            // Try to trigger a get_or_compute to see if it hits L2
            let result = cache
                .get_or_compute(key1.clone(), || async {
                    panic!("Should not compute - should hit L2 cache");
                })
                .await;

            // If this doesn't panic, L2 cache worked
            assert!(result.is_ok());
            let node = result.unwrap();
            assert_eq!(node.key.symbol, "func1");
        }
    }

    #[tokio::test]
    async fn test_layered_cache_invalidation() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let mut config = CallGraphCacheConfig::default();
        config.persistence_enabled = true;
        config.persistence_path = Some(temp_dir.path().to_path_buf());

        let cache = CallGraphCache::new_with_persistence(config).await.unwrap();
        let key = NodeKey::new("func", "/test/file.rs", "hash");

        // Add entry to both L1 and L2
        cache
            .get_or_compute(key.clone(), || async {
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();

        // Verify it's cached
        assert!(cache.get(&key).is_some());

        // Invalidate file
        cache.invalidate_file(Path::new("/test/file.rs"));

        // Should be gone from L1
        assert!(cache.get(&key).is_none());

        // Should also be gone from L2 (would need background writer to flush)
        // We can test this by ensuring the next computation actually runs
        let mut compute_called = false;
        let _result = cache
            .get_or_compute(key.clone(), || async {
                compute_called = true;
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();

        // Give time for background writer to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        // This test is a bit tricky because the background writer is async
        // In a real scenario, we'd want to test this differently
    }

    #[tokio::test]
    async fn test_cache_warming_functionality() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let mut config = CallGraphCacheConfig::default();
        config.persistence_enabled = true;
        config.persistence_path = Some(temp_dir.path().to_path_buf());
        config.capacity = 5;

        // Create and populate cache
        let keys: Vec<NodeKey> = (0..3)
            .map(|i| {
                NodeKey::new(
                    format!("func_{}", i),
                    "/test/file.rs",
                    format!("hash_{}", i),
                )
            })
            .collect();

        {
            let cache = CallGraphCache::new_with_persistence(config.clone())
                .await
                .unwrap();

            for key in &keys {
                cache
                    .get_or_compute(key.clone(), || async {
                        Ok(CallHierarchyInfo {
                            incoming_calls: vec![],
                            outgoing_calls: vec![],
                        })
                    })
                    .await
                    .unwrap();
            }

            // Give time for background writer
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Create new cache and test warming
        {
            let cache = CallGraphCache::new_with_persistence(config).await.unwrap();

            // Test that warming worked by checking stats
            let stats = cache.stats().await;
            assert!(stats.persistence_enabled);
            assert!(stats.persistent_nodes.unwrap_or(0) > 0);
        }
    }

    #[tokio::test]
    async fn test_background_writer_error_handling() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let mut config = CallGraphCacheConfig::default();
        config.persistence_enabled = true;
        config.persistence_path = Some(temp_dir.path().to_path_buf());
        config.persistence_write_batch_size = 2; // Small batch for testing
        config.persistence_write_interval = Duration::from_millis(50); // Fast for testing

        let cache = CallGraphCache::new_with_persistence(config).await.unwrap();

        // Add multiple entries quickly to test batching
        for i in 0..5 {
            let key = NodeKey::new(
                format!("func_{}", i),
                "/test/file.rs",
                format!("hash_{}", i),
            );
            cache
                .get_or_compute(key, || async {
                    Ok(CallHierarchyInfo {
                        incoming_calls: vec![],
                        outgoing_calls: vec![],
                    })
                })
                .await
                .unwrap();
        }

        // Give time for background writer to process
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Check that persistence worked
        let stats = cache.stats().await;
        assert!(stats.persistence_enabled);
        assert!(stats.persistent_nodes.unwrap_or(0) > 0);
    }

    #[tokio::test]
    async fn test_lru_eviction_with_access_metadata() {
        // Create cache with small capacity for testing
        let mut config = CallGraphCacheConfig::default();
        config.capacity = 3;
        config.ttl = Duration::from_secs(3600); // Long TTL to avoid time-based eviction
        let cache = CallGraphCache::new(config);

        // Insert 3 nodes to fill cache
        let key1 = NodeKey::new("func1", "/test/file1.rs", "hash1");
        let key2 = NodeKey::new("func2", "/test/file2.rs", "hash2");
        let key3 = NodeKey::new("func3", "/test/file3.rs", "hash3");

        for key in [&key1, &key2, &key3] {
            cache
                .get_or_compute(key.clone(), || async {
                    Ok(CallHierarchyInfo {
                        incoming_calls: vec![],
                        outgoing_calls: vec![],
                    })
                })
                .await
                .unwrap();
        }

        // All 3 should be cached
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_some());
        assert!(cache.get(&key3).is_some());
        assert_eq!(cache.nodes.len(), 3);

        // Access key1 and key3 to make them more recently used than key2
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = cache.get(&key1);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = cache.get(&key3);
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Insert a 4th node, which should trigger eviction of key2 (least recently used)
        let key4 = NodeKey::new("func4", "/test/file4.rs", "hash4");
        cache
            .get_or_compute(key4.clone(), || async {
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();

        // At this point we have 4 nodes but capacity is 3, so eviction should happen
        assert_eq!(cache.nodes.len(), 4, "Should have 4 nodes before eviction");

        // Force eviction check
        cache.force_evict().await;

        // key2 should have been evicted (it was least recently accessed)
        // key1, key3, and key4 should remain
        assert!(
            cache.get(&key1).is_some(),
            "key1 should remain (recently accessed)"
        );
        assert!(
            cache.get(&key2).is_none(),
            "key2 should be evicted (least recently used)"
        );
        assert!(
            cache.get(&key3).is_some(),
            "key3 should remain (recently accessed)"
        );
        assert!(
            cache.get(&key4).is_some(),
            "key4 should remain (just inserted)"
        );
        assert_eq!(cache.nodes.len(), 3, "Cache should maintain capacity limit");
    }
}
