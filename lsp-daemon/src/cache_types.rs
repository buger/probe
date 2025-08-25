use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

/// Unique identifier for a node in the call graph (logical identity)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub symbol: String,
    pub file: PathBuf,
}

impl NodeId {
    pub fn new(symbol: impl Into<String>, file: impl Into<PathBuf>) -> Self {
        let file_path = file.into();
        // Use consistent path normalization for cache consistency
        let normalized = Self::normalize_path(file_path);

        Self {
            symbol: symbol.into(),
            file: normalized,
        }
    }

    /// Normalize path for consistent cache keys
    /// Uses absolute path without canonicalizing to avoid filesystem-dependent changes
    fn normalize_path(path: PathBuf) -> PathBuf {
        // Convert to absolute path if it isn't already
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("/"))
                .join(path)
        }
    }
}

/// Content-addressed key for cache lookups (includes MD5 hash)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeKey {
    pub symbol: String,
    pub file: PathBuf,
    pub content_md5: String,
}

impl NodeKey {
    pub fn new(
        symbol: impl Into<String>,
        file: impl Into<PathBuf>,
        content_md5: impl Into<String>,
    ) -> Self {
        let file_path = file.into();
        // Use consistent path normalization for cache consistency
        let normalized = Self::normalize_path(file_path);

        let symbol_str = symbol.into();
        let md5_str = content_md5.into();

        tracing::debug!(
            "NodeKey::new (daemon) - symbol: {}, normalized: {}, md5: {}",
            symbol_str,
            normalized.display(),
            md5_str
        );

        Self {
            symbol: symbol_str,
            file: normalized,
            content_md5: md5_str,
        }
    }

    /// Normalize path for consistent cache keys
    /// Uses absolute path without canonicalizing to avoid filesystem-dependent changes
    fn normalize_path(path: PathBuf) -> PathBuf {
        // Convert to absolute path if it isn't already
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("/"))
                .join(path)
        }
    }

    pub fn to_node_id(&self) -> NodeId {
        NodeId::new(&self.symbol, &self.file)
    }
}

impl PartialEq for NodeKey {
    fn eq(&self, other: &Self) -> bool {
        self.symbol == other.symbol
            && self.file == other.file
            && self.content_md5 == other.content_md5
    }
}

impl Eq for NodeKey {}

impl Hash for NodeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.symbol.hash(state);
        self.file.hash(state);
        self.content_md5.hash(state);
    }
}

/// Call hierarchy information returned from LSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyInfo {
    pub incoming_calls: Vec<CallInfo>,
    pub outgoing_calls: Vec<CallInfo>,
}

/// Information about a single call relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInfo {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub symbol_kind: String,
}

/// A cached node in the call graph
#[derive(Debug, Clone)]
pub struct CachedNode {
    pub key: NodeKey,
    pub info: CallHierarchyInfo,
    pub created_at: Instant,
    pub last_accessed: Instant,
    pub access_count: usize,
}

impl CachedNode {
    pub fn new(key: NodeKey, info: CallHierarchyInfo) -> Self {
        let now = Instant::now();
        Self {
            key,
            info,
            created_at: now,
            last_accessed: now,
            access_count: 1,
        }
    }

    pub fn touch(&mut self) {
        self.last_accessed = Instant::now();
        self.access_count += 1;
    }
}

/// Statistics about the cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub total_nodes: usize,
    pub total_ids: usize,
    pub total_files: usize,
    pub total_edges: usize,
    pub inflight_computations: usize,
    // Persistence statistics
    pub persistence_enabled: bool,
    pub persistent_nodes: Option<u64>,
    pub persistent_size_bytes: Option<u64>,
    pub persistent_disk_size_bytes: Option<u64>,
}

/// Generic cache key for LSP operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCacheKey {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub content_md5: String,
    pub operation: LspOperation,
    pub extra_params: Option<String>, // For operation-specific parameters (e.g., include_declaration for references)
}

impl LspCacheKey {
    pub fn new(
        file: impl Into<PathBuf>,
        line: u32,
        column: u32,
        content_md5: impl Into<String>,
        operation: LspOperation,
        extra_params: Option<String>,
    ) -> Self {
        Self {
            file: file.into(),
            line,
            column,
            content_md5: content_md5.into(),
            operation,
            extra_params,
        }
    }
}

impl PartialEq for LspCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file
            && self.line == other.line
            && self.column == other.column
            && self.content_md5 == other.content_md5
            && self.operation == other.operation
            && self.extra_params == other.extra_params
    }
}

impl Eq for LspCacheKey {}

impl Hash for LspCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.file.hash(state);
        self.line.hash(state);
        self.column.hash(state);
        self.content_md5.hash(state);
        self.operation.hash(state);
        self.extra_params.hash(state);
    }
}

/// LSP operation types for caching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LspOperation {
    CallHierarchy,
    Definition,
    References,
    Hover,
    DocumentSymbols,
}

/// Generic cached node for LSP operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedLspNode<T> {
    pub key: LspCacheKey,
    pub data: T,
    #[serde(with = "instant_serialization")]
    pub created_at: Instant,
    #[serde(with = "instant_serialization")]
    pub last_accessed: Instant,
    pub access_count: usize,
}

mod instant_serialization {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(_instant: &Instant, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Convert Instant to duration since Unix epoch for serialization
        // This is an approximation since Instant doesn't have a fixed epoch
        let duration_since_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        duration_since_unix.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Instant, D::Error>
    where
        D: Deserializer<'de>,
    {
        let duration = Duration::deserialize(deserializer)?;
        // Convert back to Instant (this is approximate)
        // For cache purposes, we'll use current time minus the stored duration
        let now = Instant::now();
        Ok(now - duration.min(now.elapsed()))
    }
}

impl<T> CachedLspNode<T> {
    pub fn new(key: LspCacheKey, data: T) -> Self {
        let now = Instant::now();
        Self {
            key,
            data,
            created_at: now,
            last_accessed: now,
            access_count: 1,
        }
    }

    pub fn touch(&mut self) {
        self.last_accessed = Instant::now();
        self.access_count += 1;
    }
}

/// Definition locations for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionInfo {
    pub locations: Vec<LocationInfo>,
}

/// References information for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferencesInfo {
    pub locations: Vec<LocationInfo>,
    pub include_declaration: bool,
}

/// Hover information for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverInfo {
    pub contents: Option<String>,
    pub range: Option<RangeInfo>,
}

/// Document symbols information for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSymbolsInfo {
    pub symbols: Vec<DocumentSymbolInfo>,
}

/// Location information for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationInfo {
    pub uri: String,
    pub range: RangeInfo,
}

/// Range information for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeInfo {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

/// Document symbol information for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSymbolInfo {
    pub name: String,
    pub kind: String,
    pub range: RangeInfo,
    pub selection_range: RangeInfo,
    pub children: Option<Vec<DocumentSymbolInfo>>,
}

/// Generic cache statistics for different operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCacheStats {
    pub operation: LspOperation,
    pub total_entries: usize,
    pub hit_count: u64,
    pub miss_count: u64,
    pub eviction_count: u64,
    pub inflight_count: usize,
    pub memory_usage_estimate: usize,
}

/// Combined cache statistics for all operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllCacheStats {
    pub per_operation: Vec<LspCacheStats>,
    pub total_memory_usage: usize,
    pub cache_directory: Option<String>,
    pub persistent_cache_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_consistency_fix() {
        println!("🔧 Testing Cache Key Consistency Fix");

        // Test that different path representations produce identical cache keys
        let symbol = "test_function";
        let content_md5 = "abcd1234efgh5678";

        // Different ways to represent the same path
        let path1 = PathBuf::from("/Users/test/project/src/main.rs");
        let path2 = PathBuf::from("/Users/test/project")
            .join("src")
            .join("main.rs");

        let key1 = NodeKey::new(symbol, path1.clone(), content_md5);
        let key2 = NodeKey::new(symbol, path2.clone(), content_md5);

        println!("Key1 path: {} -> {}", path1.display(), key1.file.display());
        println!("Key2 path: {} -> {}", path2.display(), key2.file.display());

        // These should be identical after normalization
        assert_eq!(key1.file, key2.file, "Normalized paths should be identical");
        assert_eq!(
            key1, key2,
            "NodeKeys should be equal with consistent normalization"
        );

        // Test serialization consistency
        let serialized1 = bincode::serialize(&key1).unwrap();
        let serialized2 = bincode::serialize(&key2).unwrap();

        assert_eq!(
            serialized1, serialized2,
            "Serialized keys should be identical for cache persistence"
        );

        println!("✅ Cache key consistency fix verified!");
    }

    #[test]
    fn test_relative_path_normalization() {
        let symbol = "test_function";
        let content_md5 = "hash123";

        // Test relative vs absolute paths
        let current_dir = std::env::current_dir().unwrap();
        let relative_path = PathBuf::from("src/main.rs");
        let absolute_path = current_dir.join("src/main.rs");

        let key1 = NodeKey::new(symbol, relative_path.clone(), content_md5);
        let key2 = NodeKey::new(symbol, absolute_path.clone(), content_md5);

        println!(
            "Relative: {} -> {}",
            relative_path.display(),
            key1.file.display()
        );
        println!(
            "Absolute: {} -> {}",
            absolute_path.display(),
            key2.file.display()
        );

        assert_eq!(
            key1.file, key2.file,
            "Relative and absolute should normalize to same path"
        );
        assert_eq!(
            key1, key2,
            "Keys with relative and absolute paths should be equal"
        );

        println!("✅ Relative path normalization working!");
    }
}
