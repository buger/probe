// LSP Daemon Library
// Exports public interfaces for client implementations

// Core modules
pub mod ipc;
pub mod language_detector;
pub mod logging;
pub mod pid_lock;
pub mod process_group;
pub mod protocol;
pub mod socket_path;

// Internal modules - exposed for direct client use
pub mod lsp_registry;
pub mod lsp_server;

// Internal modules - exposed for embedded daemon use
pub mod daemon;
pub mod health_monitor;
mod pool; // Keep for now but mark as deprecated
pub mod server_manager;
pub mod watchdog;
mod workspace_resolver;

// Re-export commonly used types
pub use protocol::{
    parse_call_hierarchy_from_lsp, CallHierarchyItem, CallHierarchyResult, DaemonRequest,
    DaemonResponse, DaemonStatus, LanguageInfo, LogEntry, LogLevel, LspServerHealthInfo,
    MessageCodec, PoolStatus, ServerStatus, WorkspaceInfo,
};

pub use ipc::{IpcListener, IpcStream};
pub use language_detector::{Language, LanguageDetector};
pub use logging::{LogBuffer, MemoryLogLayer};
pub use socket_path::{get_default_socket_path, normalize_executable, remove_socket_file};

// Re-export daemon for binary and embedded use
pub use daemon::{start_daemon_background, LspDaemon};
pub use health_monitor::HealthMonitor;
pub use lsp_registry::LspRegistry;
pub use watchdog::{ProcessHealth, ProcessMonitor, ProcessStats, Watchdog};
