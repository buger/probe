use anyhow::Result;
use clap::Parser;
use lsp_daemon::get_default_socket_path;
use lsp_daemon::LspDaemon;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[clap(
    author,
    version,
    about = "LSP Daemon - Multi-language LSP server pool manager"
)]
struct Args {
    /// Path to the IPC endpoint (Unix socket or Windows named pipe)
    #[clap(short, long, default_value_t = get_default_socket_path())]
    socket: String,

    /// Log level (trace, debug, info, warn, error)
    #[clap(short, long, default_value = "info")]
    log_level: String,

    /// Run in foreground (don't daemonize)
    #[clap(short, long)]
    foreground: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&args.log_level));

    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting LSP daemon v{}", env!("CARGO_PKG_VERSION"));

    // Create daemon with async initialization for persistence support
    let daemon = LspDaemon::new_async(args.socket).await?;

    // Run daemon
    if let Err(e) = daemon.run().await {
        error!("Daemon error: {}", e);
        return Err(e);
    }

    info!("Daemon shutdown complete");
    Ok(())
}
