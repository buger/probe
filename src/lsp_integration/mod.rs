pub mod client;
pub mod management;
pub mod types;

pub use client::LspClient;
pub use management::LspManager;
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
}

use anyhow::Result;

/// Initialize LSP integration system
pub fn init_lsp() -> Result<()> {
    // Initialize any global LSP state if needed
    Ok(())
}
