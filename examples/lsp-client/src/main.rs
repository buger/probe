use anyhow::Result;
use clap::{Parser, Subcommand};
use lsp_client::client::{DirectLspClient, LspClient};
use lsp_daemon::CallHierarchyResult;
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[clap(
    author,
    version,
    about = "LSP Test - Multi-language LSP client with daemon support"
)]
struct Args {
    #[clap(subcommand)]
    command: Option<Commands>,

    /// File to analyze
    file: Option<PathBuf>,

    /// Pattern to search for
    pattern: Option<String>,

    /// Use daemon mode (auto-starts daemon if not running)
    #[clap(long, default_value = "true")]
    daemon: bool,

    /// Force direct mode (no daemon)
    #[clap(long)]
    no_daemon: bool,

    /// Log level (trace, debug, info, warn, error)
    #[clap(short, long, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Get daemon status
    Status,

    /// List available language servers
    Languages,

    /// Shutdown the daemon
    Shutdown,

    /// Ping the daemon
    Ping,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&args.log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    // Handle subcommands
    if let Some(command) = args.command {
        return handle_command(command).await;
    }

    // Regular call hierarchy operation
    let file = args.file.expect("File path required");
    let pattern = args.pattern.expect("Pattern required");

    if !file.exists() {
        eprintln!("File not found: {file:?}");
        std::process::exit(1);
    }

    let absolute_path = if file.is_absolute() {
        file
    } else {
        std::env::current_dir()?.join(file)
    };

    println!("🚀 Analyzing: {absolute_path:?}");
    println!("   Pattern: {pattern}");

    // Determine whether to use daemon or direct mode
    let use_daemon = !args.no_daemon && args.daemon;

    let result = if use_daemon {
        println!("   Mode: Daemon (auto-start enabled)\n");

        // Try daemon mode with fallback to direct
        match execute_with_daemon(&absolute_path, &pattern).await {
            Ok(result) => result,
            Err(e) => {
                eprintln!("⚠️  Daemon failed: {e}");
                eprintln!("   Falling back to direct mode...\n");
                DirectLspClient::call_hierarchy(&absolute_path, &pattern).await?
            }
        }
    } else {
        println!("   Mode: Direct\n");
        eprintln!("About to call DirectLspClient::call_hierarchy...");
        DirectLspClient::call_hierarchy(&absolute_path, &pattern).await?
    };

    // Display results
    display_call_hierarchy(&result);

    Ok(())
}

async fn handle_command(command: Commands) -> Result<()> {
    // For shutdown command, don't auto-start. For others, auto-start if needed.
    let auto_start = !matches!(command, Commands::Shutdown);
    let mut client = LspClient::new(auto_start).await?;

    match command {
        Commands::Status => {
            let status = client.get_status().await?;
            println!("📊 Daemon Status");
            println!("   Uptime: {} seconds", status.uptime_secs);
            println!("   Total requests: {}", status.total_requests);
            println!("   Active connections: {}", status.active_connections);

            if !status.pools.is_empty() {
                println!("\n   Language Pools:");
                for pool in status.pools {
                    println!(
                        "   - {:?}: {} ready, {} busy, {} total",
                        pool.language, pool.ready_servers, pool.busy_servers, pool.total_servers
                    );
                }
            } else {
                println!("\n   No active language pools");
            }
        }

        Commands::Languages => {
            let languages = client.list_languages().await?;
            println!("📚 Available Language Servers\n");

            for lang in languages {
                let status = if lang.available { "✅" } else { "❌" };
                println!(
                    "   {} {:?} - {} {}",
                    status,
                    lang.language,
                    lang.lsp_server,
                    if !lang.available {
                        "(not installed)"
                    } else {
                        ""
                    }
                );
            }
        }

        Commands::Shutdown => {
            client.shutdown_daemon().await?;
            println!("✅ Daemon shutdown complete");
        }

        Commands::Ping => {
            client.ping().await?;
            println!("✅ Daemon is responsive");
        }
    }

    Ok(())
}

async fn execute_with_daemon(file: &Path, _pattern: &str) -> Result<CallHierarchyResult> {
    let mut client = LspClient::new(true).await?;
    // For now, use a simple heuristic: search at line 1, column 0
    // In a real implementation, we'd parse the file to find the pattern location
    client.call_hierarchy(file, 1, 0).await
}

fn display_call_hierarchy(result: &CallHierarchyResult) {
    println!("📊 Call Hierarchy for '{}':\n", result.item.name);

    if !result.incoming.is_empty() {
        println!("  📥 Incoming calls (functions that call this):");
        for call in &result.incoming {
            println!("     ← {}", call.from.name);
            if !call.from_ranges.is_empty() {
                for range in &call.from_ranges {
                    println!("       at line {}", range.start.line + 1);
                }
            }
        }
    } else {
        println!("  📥 Incoming calls: (none)");
    }

    println!();

    if !result.outgoing.is_empty() {
        println!("  📤 Outgoing calls (this function calls):");
        for call in &result.outgoing {
            println!("     → {}", call.from.name);
            if !call.from_ranges.is_empty() {
                for range in &call.from_ranges {
                    println!("       at line {}", range.start.line + 1);
                }
            }
        }
    } else {
        println!("  📤 Outgoing calls: (none)");
    }

    if result.incoming.is_empty() && result.outgoing.is_empty() {
        println!("\n  ℹ️  No calls found. This could mean:");
        println!("     - The function is not used/called anywhere");
        println!("     - The function doesn't call other functions");
        println!("     - The LSP server needs more time to index");
    }
}
