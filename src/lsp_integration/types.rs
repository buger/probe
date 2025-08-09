use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            use_daemon: true,
            workspace_hint: None,
            timeout_ms: 30000,
        }
    }
}
