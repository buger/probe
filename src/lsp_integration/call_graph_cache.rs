use crate::lsp_integration::types::{CallHierarchyInfo, NodeId, NodeKey};
use anyhow::Result;
use dashmap::DashMap;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as AsyncMutex;

/// Configuration knobs for the call graph cache.
#[derive(Clone, Debug)]
pub struct CallGraphCacheConfig {
    /// Maximum number of cached nodes (across all versions).
    pub capacity: usize,
    /// TTL for a node; entries older than this are preferentially purged.
    pub ttl: Duration,
    /// How far to propagate invalidation on changes (0 = only the node, 1 = neighbors).
    pub invalidation_depth: usize,
    /// Safety bound for BFS invalidation to prevent runaway cost.
    pub max_bfs_nodes: usize,
}

impl Default for CallGraphCacheConfig {
    fn default() -> Self {
        Self {
            capacity: 50_000, // ~10k functions * 5 versions unlikely; adjust as needed
            ttl: Duration::from_secs(30 * 60), // 30 minutes
            invalidation_depth: 1, // immediate neighbors by default
            max_bfs_nodes: 10_000, // bound propagation work
        }
    }
}

/// A cached call-hierarchy result with cheap last-access tracking.
pub struct CachedNode {
    pub key: NodeKey,
    pub info: CallHierarchyInfo,
    inserted_epoch_ms: AtomicU64,
    last_access_epoch_ms: AtomicU64,
}

impl CachedNode {
    fn new(key: NodeKey, info: CallHierarchyInfo) -> Self {
        let now = now_ms();
        Self {
            key,
            info,
            inserted_epoch_ms: AtomicU64::new(now),
            last_access_epoch_ms: AtomicU64::new(now),
        }
    }
    #[inline]
    pub fn touch(&self) {
        self.last_access_epoch_ms.store(now_ms(), Ordering::Relaxed);
    }
    #[inline]
    pub fn inserted_ms(&self) -> u64 {
        self.inserted_epoch_ms.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn last_access_ms(&self) -> u64 {
        self.last_access_epoch_ms.load(Ordering::Relaxed)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// In-memory call graph cache with graph-aware invalidation.
pub struct CallGraphCache {
    cfg: CallGraphCacheConfig,
    /// Versioned cache entries.
    nodes: DashMap<NodeKey, Arc<CachedNode>>,
    /// For each NodeId, track all known versioned keys.
    id_to_keys: DashMap<NodeId, HashSet<NodeKey>>,
    /// Outgoing and incoming adjacency by NodeId (graph topology).
    outgoing: DashMap<NodeId, HashSet<NodeId>>,
    incoming: DashMap<NodeId, HashSet<NodeId>>,
    /// Fast index to invalidate everything belonging to a file.
    file_index: DashMap<PathBuf, HashSet<NodeId>>,
    /// In-flight computations keyed by NodeKey to prevent duplicate work.
    inflight: DashMap<NodeKey, Arc<AsyncMutex<()>>>,
}

impl CallGraphCache {
    pub fn new(cfg: CallGraphCacheConfig) -> Self {
        Self {
            cfg,
            nodes: DashMap::new(),
            id_to_keys: DashMap::new(),
            outgoing: DashMap::new(),
            incoming: DashMap::new(),
            file_index: DashMap::new(),
            inflight: DashMap::new(),
        }
    }

    /// Fast lookup; updates last-access on hit.
    pub fn get(&self, key: &NodeKey) -> Option<Arc<CachedNode>> {
        if let Some(entry) = self.nodes.get(key) {
            entry.value().touch();
            return Some(entry.value().clone());
        }
        None
    }

    /// Get cached value or compute it using the provided async `provider`.
    /// `provider` is only called once per NodeKey even under heavy contention.
    pub async fn get_or_compute<F, Fut>(&self, key: NodeKey, provider: F) -> Result<Arc<CachedNode>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<CallHierarchyInfo>> + Send + 'static,
    {
        if let Some(hit) = self.get(&key) {
            return Ok(hit);
        }
        let lock = self
            .inflight
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        // Double-check after acquiring the lock.
        if let Some(hit) = self.get(&key) {
            return Ok(hit);
        }

        let info = provider().await?;
        let node = Arc::new(CachedNode::new(key.clone(), info));
        self.insert_node(node.clone());
        self.evict_if_needed();
        Ok(node)
    }

    /// Insert a computed node (and register it for file- and id-based invalidation).
    pub fn insert_node(&self, node: Arc<CachedNode>) {
        let key = node.key.clone();
        let id = key.id();
        self.nodes.insert(key.clone(), node);

        // Map NodeId -> NodeKey version set
        let mut versions = self.id_to_keys.entry(id.clone()).or_default();
        versions.value_mut().insert(key);

        // File index for fast invalidation
        let mut ids = self.file_index.entry(id.file.clone()).or_default();
        ids.value_mut().insert(id);
    }

    /// Update edges for a NodeId after (re)computing its call hierarchy.
    /// Callers pass NodeIds for both incoming and outgoing neighbors.
    pub fn update_edges(
        &self,
        id: &NodeId,
        new_incoming: impl IntoIterator<Item = NodeId>,
        new_outgoing: impl IntoIterator<Item = NodeId>,
    ) {
        let new_in: HashSet<NodeId> = new_incoming.into_iter().collect();
        let new_out: HashSet<NodeId> = new_outgoing.into_iter().collect();

        // ---- Update outgoing(id) and adjust incoming(neighbor) accordingly
        if let Some(mut old_out_ref) = self.outgoing.get_mut(id) {
            let old_out = old_out_ref.clone();
            // Removed neighbors: drop id from their incoming sets
            for n in old_out.difference(&new_out) {
                if let Some(mut inc) = self.incoming.get_mut(n) {
                    inc.value_mut().remove(id);
                }
            }
            // Added neighbors: add id to their incoming sets
            for n in new_out.difference(&old_out) {
                let mut inc = self.incoming.entry(n.clone()).or_default();
                inc.value_mut().insert(id.clone());
            }
            *old_out_ref.value_mut() = new_out.clone();
        } else {
            self.outgoing.insert(id.clone(), new_out.clone());
            // Initialize incoming(neighbor) links
            for n in &new_out {
                let mut inc = self.incoming.entry(n.clone()).or_default();
                inc.value_mut().insert(id.clone());
            }
        }

        // ---- Update incoming(id) and adjust outgoing(neighbor) accordingly
        if let Some(mut old_in_ref) = self.incoming.get_mut(id) {
            let old_in = old_in_ref.clone();
            // Removed neighbors: drop id from their outgoing sets
            for n in old_in.difference(&new_in) {
                if let Some(mut out) = self.outgoing.get_mut(n) {
                    out.value_mut().remove(id);
                }
            }
            // Added neighbors: add id to their outgoing sets
            for n in new_in.difference(&old_in) {
                let mut out = self.outgoing.entry(n.clone()).or_default();
                out.value_mut().insert(id.clone());
            }
            *old_in_ref.value_mut() = new_in.clone();
        } else {
            self.incoming.insert(id.clone(), new_in.clone());
            for n in &new_in {
                let mut out = self.outgoing.entry(n.clone()).or_default();
                out.value_mut().insert(id.clone());
            }
        }
    }

    /// Invalidate a whole file (all NodeIds in it) and optionally propagate to neighbors.
    pub fn invalidate_file(&self, file: &Path) {
        // Use consistent path normalization instead of canonicalize()
        let normalized = if file.is_absolute() {
            file.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("/"))
                .join(file)
        };

        if let Some(ids_ref) = self.file_index.get(&normalized) {
            let ids: Vec<NodeId> = ids_ref.iter().cloned().collect();
            drop(ids_ref);
            for id in ids {
                self.invalidate_node(&id, self.cfg.invalidation_depth);
            }
        }
    }

    /// Invalidate this NodeId and propagate with bounded BFS.
    pub fn invalidate_node(&self, root: &NodeId, depth: usize) {
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut q: VecDeque<(NodeId, usize)> = VecDeque::new();
        visited.insert(root.clone());
        q.push_back((root.clone(), 0));

        let mut processed = 0usize;
        while let Some((id, d)) = q.pop_front() {
            self.invalidate_node_local(&id);
            processed += 1;
            if processed >= self.cfg.max_bfs_nodes {
                break;
            }
            if d >= depth {
                continue;
            }
            // Explore neighbors (both directions)
            if let Some(out) = self.outgoing.get(&id) {
                for n in out.iter() {
                    if visited.insert(n.clone()) {
                        q.push_back((n.clone(), d + 1));
                    }
                }
            }
            if let Some(inc) = self.incoming.get(&id) {
                for n in inc.iter() {
                    if visited.insert(n.clone()) {
                        q.push_back((n.clone(), d + 1));
                    }
                }
            }
        }
    }

    /// Invalidate only this NodeId: remove its versions from the node cache and clear edges.
    fn invalidate_node_local(&self, id: &NodeId) {
        // Remove versioned nodes
        if let Some(mut versions_ref) = self.id_to_keys.get_mut(id) {
            let keys: Vec<NodeKey> = versions_ref.iter().cloned().collect();
            for k in &keys {
                self.nodes.remove(k);
                // Also clear any inflight lock to avoid deadlocks on retry
                self.inflight.remove(k);
            }
            versions_ref.value_mut().clear();
        }
        // Remove id from neighbors' adjacency
        if let Some(out_ref) = self.outgoing.get(id) {
            for n in out_ref.iter() {
                if let Some(mut inc) = self.incoming.get_mut(n) {
                    inc.value_mut().remove(id);
                }
            }
        }
        if let Some(in_ref) = self.incoming.get(id) {
            for n in in_ref.iter() {
                if let Some(mut out) = self.outgoing.get_mut(n) {
                    out.value_mut().remove(id);
                }
            }
        }
        self.outgoing.remove(id);
        self.incoming.remove(id);
        // Keep file_index mapping: future recomputes will simply update.
    }

    /// Capacity- and TTL-based eviction (best-effort, O(n log n) when triggered).
    pub fn evict_if_needed(&self) {
        let now = now_ms();
        let ttl_ms = self.cfg.ttl.as_millis() as u64;
        let mut to_remove: Vec<NodeKey> = Vec::new();

        // Pass 1: remove TTL-expired entries.
        for entry in self.nodes.iter() {
            let n = entry.value();
            if now.saturating_sub(n.inserted_ms()) > ttl_ms {
                to_remove.push(entry.key().clone());
            }
        }
        for k in to_remove.drain(..) {
            if let Some((_k, node)) = self.nodes.remove(&k) {
                // Also detach from id_to_keys
                let id = node.key.id();
                if let Some(mut versions) = self.id_to_keys.get_mut(&id) {
                    versions.value_mut().remove(&k);
                }
                self.inflight.remove(&k);
            }
        }

        // Pass 2: enforce capacity with LRU-approx eviction.
        let len = self.nodes.len();
        if len <= self.cfg.capacity {
            return;
        }
        let excess = len - self.cfg.capacity;
        let mut items: Vec<(NodeKey, u64)> = self
            .nodes
            .iter()
            .map(|e| (e.key().clone(), e.value().last_access_ms()))
            .collect();
        items.sort_unstable_by_key(|(_, last)| *last);
        for (k, _) in items.into_iter().take(excess) {
            if let Some((_k, node)) = self.nodes.remove(&k) {
                let id = node.key.id();
                if let Some(mut versions) = self.id_to_keys.get_mut(&id) {
                    versions.value_mut().remove(&k);
                }
                self.inflight.remove(&k);
            }
        }
    }

    /// Get cache statistics for debugging
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            total_nodes: self.nodes.len(),
            total_ids: self.id_to_keys.len(),
            total_files: self.file_index.len(),
            total_edges: self.outgoing.len() + self.incoming.len(),
            inflight_computations: self.inflight.len(),
        }
    }
}

/// Cache statistics for monitoring
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_nodes: usize,
    pub total_ids: usize,
    pub total_files: usize,
    pub total_edges: usize,
    pub inflight_computations: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp_integration::types::CallInfo;

    fn create_test_hierarchy() -> CallHierarchyInfo {
        CallHierarchyInfo {
            incoming_calls: vec![CallInfo {
                name: "caller".to_string(),
                file_path: "/test/caller.rs".to_string(),
                line: 10,
                column: 5,
                symbol_kind: "function".to_string(),
            }],
            outgoing_calls: vec![CallInfo {
                name: "callee".to_string(),
                file_path: "/test/callee.rs".to_string(),
                line: 20,
                column: 10,
                symbol_kind: "function".to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn test_cache_basic_operations() {
        let cache = CallGraphCache::new(CallGraphCacheConfig::default());

        let key = NodeKey::new(
            "test_func",
            PathBuf::from("/test/file.rs"),
            "abc123".to_string(),
        );

        // Test get_or_compute
        let result = cache
            .get_or_compute(key.clone(), || async { Ok(create_test_hierarchy()) })
            .await
            .unwrap();

        assert_eq!(result.key.symbol, "test_func");
        assert_eq!(result.info.incoming_calls.len(), 1);

        // Test cache hit
        let cached = cache.get(&key).unwrap();
        assert_eq!(cached.key.symbol, "test_func");
    }

    #[test]
    fn test_edge_updates() {
        let cache = CallGraphCache::new(CallGraphCacheConfig::default());

        let id1 = NodeId::new("func1", PathBuf::from("/test/file1.rs"));
        let id2 = NodeId::new("func2", PathBuf::from("/test/file2.rs"));
        let id3 = NodeId::new("func3", PathBuf::from("/test/file3.rs"));

        // Update edges for id1
        cache.update_edges(&id1, vec![id2.clone()], vec![id3.clone()]);

        // Check adjacency
        assert!(cache.incoming.get(&id1).unwrap().contains(&id2));
        assert!(cache.outgoing.get(&id1).unwrap().contains(&id3));
        assert!(cache.outgoing.get(&id2).unwrap().contains(&id1));
        assert!(cache.incoming.get(&id3).unwrap().contains(&id1));
    }

    #[test]
    fn test_invalidation() {
        let cache = CallGraphCache::new(CallGraphCacheConfig::default());

        let key1 = NodeKey::new("func1", PathBuf::from("/test/file.rs"), "hash1".to_string());
        let node1 = Arc::new(CachedNode::new(key1.clone(), create_test_hierarchy()));
        cache.insert_node(node1);

        assert!(cache.get(&key1).is_some());

        // Invalidate the file
        cache.invalidate_file(Path::new("/test/file.rs"));

        // Node should be removed
        assert!(cache.get(&key1).is_none());
    }
}
