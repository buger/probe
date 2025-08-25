pub mod call_graph_cache;
pub mod client;
pub mod management;
pub mod stdlib_filter;
pub mod symbol_resolver;
pub mod types;

pub use client::LspClient;
pub use management::LspManager;
pub use stdlib_filter::{is_stdlib_path, is_stdlib_path_cached};
pub use symbol_resolver::{resolve_location, ResolvedLocation};
pub use types::*;

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum LspSubcommands {
    /// Show LSP daemon status, uptime, and server pool information
    Status {
        /// Use daemon mode (auto-start if not running)
        #[clap(long = "daemon", default_value = "true")]
        daemon: bool,

        /// Workspace hint for LSP server initialization
        #[clap(long = "workspace-hint")]
        workspace_hint: Option<String>,
    },

    /// List available LSP servers and their installation status
    Languages,

    /// Health check and connectivity test for LSP daemon
    Ping {
        /// Use daemon mode (auto-start if not running)
        #[clap(long = "daemon", default_value = "true")]
        daemon: bool,

        /// Workspace hint for LSP server initialization
        #[clap(long = "workspace-hint")]
        workspace_hint: Option<String>,
    },

    /// Gracefully shutdown the LSP daemon
    Shutdown,

    /// Restart the LSP daemon (shutdown + auto-start)
    Restart {
        /// Workspace hint for LSP server initialization
        #[clap(long = "workspace-hint")]
        workspace_hint: Option<String>,
    },

    /// Show version information with git hash and build date
    Version,

    /// View LSP daemon logs
    Logs {
        /// Follow the log output (like tail -f)
        #[clap(short = 'f', long = "follow")]
        follow: bool,

        /// Number of lines to show from the end of the log
        #[clap(short = 'n', long = "lines", default_value = "50")]
        lines: usize,

        /// Clear the log file
        #[clap(long = "clear")]
        clear: bool,
    },

    /// Start the LSP daemon (embedded mode)
    Start {
        /// Path to the IPC endpoint (Unix socket or Windows named pipe)
        #[clap(short, long)]
        socket: Option<String>,

        /// Log level (trace, debug, info, warn, error)
        #[clap(short, long, default_value = "info")]
        log_level: String,

        /// Run in foreground (don't daemonize)
        #[clap(short, long)]
        foreground: bool,
    },

    /// Initialize language servers for workspaces
    Init {
        /// Workspace path to initialize (defaults to current directory)
        #[clap(short = 'w', long = "workspace")]
        workspace: Option<String>,

        /// Specific languages to initialize (comma-separated, e.g., "rust,typescript")
        #[clap(short = 'l', long = "languages")]
        languages: Option<String>,

        /// Recursively search for and initialize nested workspaces
        #[clap(short = 'r', long = "recursive")]
        recursive: bool,

        /// Use daemon mode (auto-start if not running)
        #[clap(long = "daemon", default_value = "true")]
        daemon: bool,

        /// Enable watchdog monitoring for daemon health and resource usage
        #[clap(long = "watchdog")]
        watchdog: bool,
    },

    /// Call LSP methods directly
    Call {
        #[clap(subcommand)]
        command: LspCallCommands,
    },

    /// Cache management subcommands
    Cache {
        #[clap(subcommand)]
        cache_command: CacheSubcommands,
    },

    /// Start indexing a workspace
    Index {
        /// Workspace path to index (defaults to current directory)
        #[clap(short = 'w', long = "workspace")]
        workspace: Option<String>,

        /// Specific languages to index (comma-separated, e.g., "rust,typescript")
        #[clap(short = 'l', long = "languages")]
        languages: Option<String>,

        /// Recursively index nested workspaces
        #[clap(short = 'r', long = "recursive")]
        recursive: bool,

        /// Maximum number of worker threads (default: CPU count)
        #[clap(long = "max-workers")]
        max_workers: Option<usize>,

        /// Memory budget in MB (default: 512MB)
        #[clap(long = "memory-budget")]
        memory_budget: Option<u64>,

        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,

        /// Show progress bar (disable for scripting)
        #[clap(long = "progress", default_value = "true")]
        progress: bool,

        /// Wait for indexing to complete before returning
        #[clap(long = "wait")]
        wait: bool,
    },

    /// Show detailed indexing status
    IndexStatus {
        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,

        /// Show per-file progress details
        #[clap(long = "detailed")]
        detailed: bool,

        /// Follow indexing progress (like tail -f)
        #[clap(short = 'f', long = "follow")]
        follow: bool,

        /// Update interval for follow mode (seconds)
        #[clap(long = "interval", default_value = "1")]
        interval: u64,
    },

    /// Stop ongoing indexing
    IndexStop {
        /// Force stop even if indexing is in progress
        #[clap(short = 'f', long = "force")]
        force: bool,

        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,
    },

    /// Configure indexing settings
    IndexConfig {
        #[clap(subcommand)]
        config_command: IndexConfigSubcommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum IndexConfigSubcommands {
    /// Show current indexing configuration
    Show {
        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,
    },

    /// Set indexing configuration
    Set {
        /// Maximum number of worker threads
        #[clap(long = "max-workers")]
        max_workers: Option<usize>,

        /// Memory budget in MB
        #[clap(long = "memory-budget")]
        memory_budget: Option<u64>,

        /// File patterns to exclude (comma-separated)
        #[clap(long = "exclude")]
        exclude_patterns: Option<String>,

        /// File patterns to include (comma-separated, empty=all)
        #[clap(long = "include")]
        include_patterns: Option<String>,

        /// Maximum file size to index (MB)
        #[clap(long = "max-file-size")]
        max_file_size: Option<u64>,

        /// Enable incremental indexing mode
        #[clap(long = "incremental")]
        incremental: Option<bool>,

        // LSP Caching Configuration Options
        /// Enable caching of call hierarchy operations during indexing
        #[clap(long = "cache-call-hierarchy")]
        cache_call_hierarchy: Option<bool>,

        /// Enable caching of definition lookups during indexing
        #[clap(long = "cache-definitions")]
        cache_definitions: Option<bool>,

        /// Enable caching of reference lookups during indexing
        #[clap(long = "cache-references")]
        cache_references: Option<bool>,

        /// Enable caching of hover information during indexing
        #[clap(long = "cache-hover")]
        cache_hover: Option<bool>,

        /// Enable caching of document symbols during indexing
        #[clap(long = "cache-document-symbols")]
        cache_document_symbols: Option<bool>,

        /// Perform LSP operations during indexing (vs only on-demand)
        #[clap(long = "cache-during-indexing")]
        cache_during_indexing: Option<bool>,

        /// Preload cache with common operations after indexing
        #[clap(long = "preload-common-symbols")]
        preload_common_symbols: Option<bool>,

        /// Maximum LSP operations to cache per operation type during indexing
        #[clap(long = "max-cache-entries-per-operation")]
        max_cache_entries_per_operation: Option<usize>,

        /// Timeout for LSP operations during indexing (milliseconds)
        #[clap(long = "lsp-operation-timeout-ms")]
        lsp_operation_timeout_ms: Option<u64>,

        /// Priority operations during indexing (comma-separated: call_hierarchy,definition,references,hover,document_symbols)
        #[clap(long = "lsp-priority-operations")]
        lsp_priority_operations: Option<String>,

        /// Operations to skip during indexing (comma-separated: call_hierarchy,definition,references,hover,document_symbols)
        #[clap(long = "lsp-disabled-operations")]
        lsp_disabled_operations: Option<String>,

        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,
    },

    /// Reset indexing configuration to defaults
    Reset {
        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum CacheSubcommands {
    /// Show cache statistics
    Stats {
        /// Show detailed statistics
        #[clap(long)]
        detailed: bool,

        /// Show git-aware statistics
        #[clap(long)]
        git: bool,
    },

    /// Clear cache
    Clear {
        /// Clear only entries older than N days
        #[clap(long)]
        older_than: Option<u64>,

        /// Clear only entries for specific file
        #[clap(long)]
        file: Option<std::path::PathBuf>,

        /// Clear only entries for specific git commit
        #[clap(long)]
        commit: Option<String>,

        /// Clear everything (requires confirmation)
        #[clap(long)]
        all: bool,
    },

    /// Export cache to file
    Export {
        /// Output file path
        #[clap(required = true)]
        output: std::path::PathBuf,

        /// Include only entries from current branch
        #[clap(long)]
        current_branch: bool,

        /// Compress the export
        #[clap(long)]
        compress: bool,
    },

    /// Import cache from file
    Import {
        /// Input file path
        #[clap(required = true)]
        input: std::path::PathBuf,

        /// Merge with existing cache (default: replace)
        #[clap(long)]
        merge: bool,
    },

    /// Compact the cache database
    Compact {
        /// Remove expired entries
        #[clap(long)]
        clean_expired: bool,

        /// Target size in MB (removes oldest entries)
        #[clap(long)]
        target_size_mb: Option<usize>,
    },

    /// List all workspace caches
    List {
        /// Show detailed information for each workspace cache
        #[clap(long)]
        detailed: bool,

        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,
    },

    /// Show detailed information about workspace caches
    Info {
        /// Workspace path to get info for (optional, shows all if not specified)
        workspace: Option<std::path::PathBuf>,

        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,
    },

    /// Clear workspace caches
    ClearWorkspace {
        /// Workspace path to clear (optional, clears all if not specified)
        workspace: Option<std::path::PathBuf>,

        /// Force clear without confirmation
        #[clap(short = 'f', long = "force")]
        force: bool,

        /// Output format (terminal, json)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json"])]
        format: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum LspCallCommands {
    /// Go to definition of a symbol
    Definition {
        /// Location in format 'file.rs:42:10' (line:column) or 'file.rs#symbol_name'
        location: String,

        /// Output format (terminal, json, plain)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json", "plain"])]
        format: String,
    },

    /// Find all references to a symbol
    References {
        /// Location in format 'file.rs:42:10' (line:column) or 'file.rs#symbol_name'
        location: String,

        /// Include the declaration/definition in results
        #[clap(long = "include-declaration")]
        include_declaration: bool,

        /// Output format (terminal, json, plain)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json", "plain"])]
        format: String,
    },

    /// Get hover information for a symbol
    Hover {
        /// Location in format 'file.rs:42:10' (line:column) or 'file.rs#symbol_name'
        location: String,

        /// Output format (terminal, json, plain)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json", "plain"])]
        format: String,
    },

    /// List all symbols in a document
    DocumentSymbols {
        /// File path to get symbols from
        file: std::path::PathBuf,

        /// Output format (terminal, json, plain)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json", "plain"])]
        format: String,
    },

    /// Search for symbols in the workspace
    WorkspaceSymbols {
        /// Query string to search for
        query: String,

        /// Maximum number of results to return
        #[clap(long = "max-results")]
        max_results: Option<usize>,

        /// Output format (terminal, json, plain)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json", "plain"])]
        format: String,
    },

    /// Get call hierarchy information for a symbol
    CallHierarchy {
        /// Location in format 'file.rs:42:10' (line:column) or 'file.rs#symbol_name'
        location: String,

        /// Output format (terminal, json, plain)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json", "plain"])]
        format: String,
    },

    /// Find implementations of a symbol (interfaces, traits)
    Implementations {
        /// Location in format 'file.rs:42:10' (line:column) or 'file.rs#symbol_name'
        location: String,

        /// Output format (terminal, json, plain)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json", "plain"])]
        format: String,
    },

    /// Go to type definition of a symbol
    TypeDefinition {
        /// Location in format 'file.rs:42:10' (line:column) or 'file.rs#symbol_name'
        location: String,

        /// Output format (terminal, json, plain)
        #[clap(short = 'o', long = "format", default_value = "terminal", value_parser = ["terminal", "json", "plain"])]
        format: String,
    },
}

use anyhow::Result;

/// Initialize LSP integration system
pub fn init_lsp() -> Result<()> {
    // Initialize any global LSP state if needed
    Ok(())
}
