use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// LSP daemon status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDaemonStatus {
    pub uptime: std::time::Duration,
    pub total_requests: u64,
    pub active_connections: usize,
    pub language_pools: HashMap<String, LanguagePoolStatus>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub git_hash: String,
    #[serde(default)]
    pub build_date: String,
}

/// Status of a language server pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguagePoolStatus {
    pub language: String,
    pub ready_servers: usize,
    pub busy_servers: usize,
    pub total_servers: usize,
    pub available: bool,
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default)]
    pub uptime_secs: u64,
    #[serde(default)]
    pub status: String,
}

/// Call hierarchy information for a symbol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyInfo {
    pub incoming_calls: Vec<CallInfo>,
    pub outgoing_calls: Vec<CallInfo>,
}

impl CallHierarchyInfo {
    /// Remove entries that belong to language standard libraries (Rust std, Go std, Python stdlib, etc.).
    /// Filtering is intentionally conservative to avoid hiding user or third‑party code.
    pub fn filter_stdlib_in_place(&mut self) {
        use crate::lsp_integration::stdlib_filter::is_stdlib_path_cached;

        self.incoming_calls
            .retain(|call| !is_stdlib_path_cached(&call.file_path));
        self.outgoing_calls
            .retain(|call| !is_stdlib_path_cached(&call.file_path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_stdlib_in_place() {
        let mut call_hierarchy = CallHierarchyInfo {
            incoming_calls: vec![
                CallInfo {
                    name: "user_function".to_string(),
                    file_path: "/Users/test/project/src/main.rs".to_string(),
                    line: 10,
                    column: 5,
                    symbol_kind: "function".to_string(),
                },
                CallInfo {
                    name: "println".to_string(),
                    file_path: "/.rustup/toolchains/stable/lib/rustlib/src/rust/library/std/src/io/stdio.rs".to_string(),
                    line: 20,
                    column: 8,
                    symbol_kind: "function".to_string(),
                },
            ],
            outgoing_calls: vec![
                CallInfo {
                    name: "another_user_function".to_string(),
                    file_path: "/Users/test/project/src/utils.rs".to_string(),
                    line: 15,
                    column: 10,
                    symbol_kind: "function".to_string(),
                },
                CallInfo {
                    name: "Vec::new".to_string(),
                    file_path: "/.rustup/toolchains/stable/lib/rustlib/src/rust/library/alloc/src/vec/mod.rs".to_string(),
                    line: 388,
                    column: 14,
                    symbol_kind: "function".to_string(),
                },
            ],
        };

        // Before filtering
        assert_eq!(call_hierarchy.incoming_calls.len(), 2);
        assert_eq!(call_hierarchy.outgoing_calls.len(), 2);

        // Apply stdlib filtering
        call_hierarchy.filter_stdlib_in_place();

        // After filtering - stdlib entries should be removed
        assert_eq!(call_hierarchy.incoming_calls.len(), 1);
        assert_eq!(call_hierarchy.outgoing_calls.len(), 1);
        assert_eq!(call_hierarchy.incoming_calls[0].name, "user_function");
        assert_eq!(
            call_hierarchy.outgoing_calls[0].name,
            "another_user_function"
        );
    }

    #[test]
    fn test_lsp_config_default() {
        let config = LspConfig::default();
        assert!(config.use_daemon);
        assert!(config.workspace_hint.is_none());
        assert_eq!(config.timeout_ms, 30000);
        assert!(!config.include_stdlib); // Should default to filtering out stdlib
    }
}

/// Information about a function call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInfo {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub symbol_kind: String,
}

/// Reference information for a symbol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceInfo {
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub context: String,
}

/// Extended symbol information with LSP data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedSymbolInfo {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub symbol_kind: String,
    pub call_hierarchy: Option<CallHierarchyInfo>,
    pub references: Vec<ReferenceInfo>,
    pub documentation: Option<String>,
    pub type_info: Option<String>,
}

/// LSP configuration options
#[derive(Debug, Clone)]
pub struct LspConfig {
    pub use_daemon: bool,
    pub workspace_hint: Option<String>,
    pub timeout_ms: u64,
    /// If true, do NOT filter out standard library entries in call hierarchy results.
    pub include_stdlib: bool,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            use_daemon: true,
            workspace_hint: None,
            timeout_ms: 30000,
            include_stdlib: false, // Default to filtering out stdlib
        }
    }
}

/// Stable identifier of a symbol at a file path, independent of content hash.
/// Edges in the call graph are stored at this level.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct NodeId {
    pub symbol: String,
    pub file: PathBuf,
}

/// Content-addressed key for a particular version of a symbol.
/// This is used to cache a computed CallHierarchyInfo snapshot safely.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct NodeKey {
    pub symbol: String,
    pub file: PathBuf,
    /// Lowercase hex MD5 of the content used to compute the call graph for this symbol.
    pub content_md5: String,
}

impl NodeId {
    /// Create a NodeId with normalized path for consistent identity
    pub fn new<S: Into<String>>(symbol: S, file: PathBuf) -> Self {
        // Use the same normalization as NodeKey for consistency
        let normalized = Self::normalize_path(file);

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

impl NodeKey {
    pub fn new<S: Into<String>>(symbol: S, file: PathBuf, content_md5: String) -> Self {
        // Use consistent path normalization instead of canonicalize()
        // to avoid cache key mismatches due to filesystem changes
        let original_path = file.clone();
        let normalized = Self::normalize_path(file);
        let symbol_str = symbol.into();

        tracing::debug!(
            "NodeKey::new - symbol: {}, original: {}, normalized: {}, md5: {}",
            symbol_str,
            original_path.display(),
            normalized.display(),
            content_md5
        );

        Self {
            symbol: symbol_str,
            file: normalized,
            content_md5,
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

    /// The stable identity for this versioned key.
    pub fn id(&self) -> NodeId {
        NodeId {
            symbol: self.symbol.clone(),
            file: self.file.clone(),
        }
    }
}
