// LSP Daemon Library
// Exports public interfaces for client implementations

// Core modules
pub mod ipc;
pub mod language_detector;
pub mod logging;
pub mod path_safety;
pub mod pid_lock;
pub mod process_group;
pub mod protocol;
pub mod socket_path;

// Cache modules
pub mod cache_management;
pub mod cache_types;
pub mod call_graph_cache;
pub mod hash_utils;
pub mod lsp_cache;
pub mod persistent_cache;

// Handler modules
pub mod definition_handler;
pub mod hover_handler;
pub mod references_handler;

// Internal modules - exposed for direct client use
pub mod lsp_registry;
pub mod lsp_server;

// Internal modules - exposed for embedded daemon use
pub mod daemon;
pub mod health_monitor;
mod pool; // Keep for now but mark as deprecated
pub mod server_manager;
pub mod watchdog;
pub mod workspace_cache_router;
mod workspace_resolver;

// Indexing subsystem
pub mod indexing;

// File watching subsystem
pub mod file_watcher;

// Re-export commonly used types
pub use protocol::{
    parse_call_hierarchy_from_lsp,
    AgeDistribution,
    // Cache management types
    CacheStatistics,
    CallHierarchyItem,
    CallHierarchyResult,
    ClearResult,
    CompactResult,
    DaemonRequest,
    DaemonResponse,
    DaemonStatus,
    HotSpot,
    ImportResult,
    LanguageInfo,
    LogEntry,
    LogLevel,
    LspServerHealthInfo,
    MemoryUsage,
    MessageCodec,
    PoolStatus,
    ServerStatus,
    WorkspaceInfo,
};

pub use cache_management::CacheManager;
pub use ipc::{IpcListener, IpcStream};
pub use language_detector::{Language, LanguageDetector};
pub use logging::{LogBuffer, MemoryLogLayer};
pub use socket_path::{get_default_socket_path, normalize_executable, remove_socket_file};

// Re-export daemon for binary and embedded use
pub use daemon::{start_daemon_background, LspDaemon};
pub use health_monitor::HealthMonitor;
pub use lsp_registry::LspRegistry;
pub use watchdog::{ProcessHealth, ProcessMonitor, ProcessStats, Watchdog};

// Re-export indexing types for external use
pub use indexing::{
    CacheStrategy, EffectiveConfig, IndexingConfig, IndexingFeatures, IndexingManager,
    IndexingPipeline, IndexingProgress, IndexingQueue, LanguageIndexConfig, LanguagePipeline,
    ManagerConfig, ManagerStatus, PipelineConfig, PipelineResult, Priority, ProgressMetrics,
    ProgressSnapshot, QueueItem, QueueMetrics, QueueSnapshot, WorkerStats,
};

// Re-export file watcher types for external use
pub use file_watcher::{
    FileEvent, FileEventType, FileWatcher, FileWatcherConfig, FileWatcherStats,
};

// Re-export workspace cache router types for external use
pub use workspace_cache_router::{
    WorkspaceCacheRouter, WorkspaceCacheRouterConfig, WorkspaceCacheRouterStats, WorkspaceStats,
};

// Re-export pipeline-specific types
pub use indexing::pipelines::SymbolInfo;
