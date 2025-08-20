use anyhow::{anyhow, Context, Result};
use colored::*;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tokio::time::{self, MissedTickBehavior};

use crate::lsp_integration::client::LspClient;
use crate::lsp_integration::types::*;
use crate::lsp_integration::{CacheSubcommands, IndexConfigSubcommands, LspSubcommands};
use lsp_daemon::{LogEntry, LogLevel, LspDaemon};

// Follow-mode tuning: keep polling light to avoid hammering the daemon and the filesystem.
const LOG_FOLLOW_POLL_MS: u64 = 500;
const LOG_FETCH_LIMIT: usize = 200;
const LOG_RPC_TIMEOUT_MS: u64 = 2000;

pub struct LspManager;

impl LspManager {
    /// Ensure LSP daemon is ready and workspace is initialized
    /// This is the main bootstrap function that ensures the LSP system is ready for operations
    pub async fn ensure_ready() -> Result<()> {
        // Check if we can connect to daemon
        let config = LspConfig::default();
        let mut client = match LspClient::new(config).await {
            Ok(client) => client,
            Err(_) => {
                // Daemon not running, auto-start it
                Self::auto_start_daemon().await?;

                // Wait a moment for daemon to fully start
                tokio::time::sleep(Duration::from_millis(500)).await;

                // Connect to newly started daemon
                LspClient::new(LspConfig::default()).await?
            }
        };

        // Quick health check
        if client.ping().await.is_err() {
            // If ping fails, restart daemon
            let _ = client.shutdown_daemon().await;
            tokio::time::sleep(Duration::from_millis(500)).await;

            Self::auto_start_daemon().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Auto-initialize current workspace if it looks like a code project
        let current_dir = std::env::current_dir()?;
        if Self::is_code_workspace(&current_dir)?
            && client
                .init_workspaces(
                    current_dir,
                    None,  // Auto-detect languages
                    false, // Not recursive by default
                    false, // No watchdog by default
                )
                .await
                .is_err()
        {
            // Init failure is not critical for basic operations
            // The workspace will be initialized on-demand during operations
        }

        Ok(())
    }

    /// Auto-start the LSP daemon in background mode
    async fn auto_start_daemon() -> Result<()> {
        // Delegate to the single, lock-protected spawn path in client.rs
        crate::lsp_integration::client::start_embedded_daemon_background().await?;

        // Wait for daemon to be ready (up to 10 seconds)
        let socket_path = lsp_daemon::get_default_socket_path();
        for _ in 0..20 {
            if lsp_daemon::ipc::IpcStream::connect(&socket_path)
                .await
                .is_ok()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err(anyhow!("Failed to auto-start LSP daemon"))
    }

    /// Check if a directory looks like a code workspace
    fn is_code_workspace(path: &Path) -> Result<bool> {
        // Check for common project indicators
        let indicators = [
            "Cargo.toml",
            "package.json",
            "go.mod",
            "pyproject.toml",
            "requirements.txt",
            "pom.xml",
            "build.gradle",
            ".git",
            "tsconfig.json",
            "composer.json",
            "Makefile",
        ];

        for indicator in &indicators {
            if path.join(indicator).exists() {
                return Ok(true);
            }
        }

        // Also check if there are source files in common locations
        let source_dirs = ["src", "lib", "app", "internal"];
        for dir in &source_dirs {
            let source_path = path.join(dir);
            if source_path.exists() && source_path.is_dir() {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Ensure project is built to avoid cargo build lock conflicts
    #[allow(dead_code)]
    fn ensure_project_built() -> Result<()> {
        let target_debug = Path::new("target/debug/probe");

        if !target_debug.exists() {
            eprintln!("⚠️  Project not built, building to avoid cargo lock conflicts...");
            let output = std::process::Command::new("cargo").arg("build").output()?;

            if !output.status.success() {
                eprintln!("❌ Build failed:");
                eprintln!("{}", String::from_utf8_lossy(&output.stderr));
                return Err(anyhow::anyhow!("Failed to build project"));
            }
            eprintln!("✅ Build completed successfully");
        }
        Ok(())
    }

    /// Handle LSP subcommands
    pub async fn handle_command(subcommand: &LspSubcommands, format: &str) -> Result<()> {
        match subcommand {
            LspSubcommands::Status {
                daemon,
                workspace_hint,
            } => Self::show_status(*daemon, workspace_hint.clone(), format).await,
            LspSubcommands::Languages => Self::list_languages(format).await,
            LspSubcommands::Ping {
                daemon,
                workspace_hint,
            } => Self::ping(*daemon, workspace_hint.clone(), format).await,
            LspSubcommands::Shutdown => Self::shutdown_daemon(format).await,
            LspSubcommands::Restart { workspace_hint } => {
                Self::restart_daemon(workspace_hint.clone(), format).await
            }
            LspSubcommands::Version => Self::show_version(format).await,
            LspSubcommands::Start {
                socket,
                log_level,
                foreground,
            } => Self::start_embedded_daemon(socket.clone(), log_level.clone(), *foreground).await,
            LspSubcommands::Logs {
                follow,
                lines,
                clear,
            } => Self::handle_logs(*follow, *lines, *clear).await,
            LspSubcommands::Init {
                workspace,
                languages,
                recursive,
                daemon,
                watchdog,
            } => {
                Self::init_workspaces(
                    workspace.clone(),
                    languages.clone(),
                    *recursive,
                    *daemon,
                    *watchdog,
                    format,
                )
                .await
            }
            LspSubcommands::Cache { cache_command } => {
                Self::handle_cache_command(cache_command, format).await
            }
            LspSubcommands::Index {
                workspace,
                languages,
                recursive,
                max_workers,
                memory_budget,
                format,
                progress,
                wait,
            } => {
                Self::handle_index_command(
                    workspace.clone(),
                    languages.clone(),
                    *recursive,
                    *max_workers,
                    *memory_budget,
                    format,
                    *progress,
                    *wait,
                )
                .await
            }
            LspSubcommands::IndexStatus {
                format,
                detailed,
                follow,
                interval,
            } => Self::handle_index_status_command(format, *detailed, *follow, *interval).await,
            LspSubcommands::IndexStop { force, format } => {
                Self::handle_index_stop_command(*force, format).await
            }
            LspSubcommands::IndexConfig { config_command } => {
                Self::handle_index_config_command(config_command, format).await
            }
        }
    }

    /// Show daemon status
    async fn show_status(
        use_daemon: bool,
        workspace_hint: Option<String>,
        format: &str,
    ) -> Result<()> {
        // Check if we're being run via cargo and warn about potential conflicts
        // Look for "cargo-run-build" in path which indicates cargo run is being used
        // Don't trigger on installed binaries in .cargo/bin
        if std::env::current_exe()
            .map(|path| {
                let path_str = path.to_string_lossy();
                // cargo run creates paths like: target/debug/deps/probe-<hash> or
                // target/debug/build/probe-<hash>/out/probe
                path_str.contains("/target/debug/deps/")
                    || path_str.contains("/target/release/deps/")
                    || path_str.contains("cargo-run-build")
            })
            .unwrap_or(false)
        {
            eprintln!(
                "⚠️  WARNING: Running via 'cargo run' may cause build lock conflicts with daemon."
            );
            eprintln!("   If this hangs, use: cargo build && ./target/debug/probe lsp status");
        }

        let config = LspConfig {
            use_daemon,
            workspace_hint,
            timeout_ms: 10000, // 10 seconds for status command
        };

        // On first run, daemon needs to start which can take up to 10s
        // Plus additional time for connection establishment and version check
        // Total timeout should be at least 20s to avoid false timeouts on first run
        let mut client =
            match tokio::time::timeout(Duration::from_secs(25), LspClient::new(config)).await {
                Ok(Ok(client)) => client,
                Ok(Err(e)) => {
                    // Check if this is a version mismatch restart
                    if e.to_string()
                        .contains("LSP daemon restarting due to version change")
                    {
                        eprintln!(
                            "\nℹ️  {}",
                            "LSP daemon is restarting due to version change.".yellow()
                        );
                        eprintln!("   Please wait a few seconds and try again.");
                        eprintln!("   The daemon will be ready shortly.");
                        return Ok(());
                    }
                    return Err(anyhow!("Failed to connect to daemon: {}", e));
                }
                Err(_) => return Err(anyhow!("Timeout connecting to daemon after 25 seconds")),
            };

        let status = match tokio::time::timeout(Duration::from_secs(10), client.get_status()).await
        {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => return Err(anyhow!("Failed to get status: {}", e)),
            Err(_) => return Err(anyhow!("Timeout getting status after 10 seconds")),
        };

        match format {
            "json" => {
                let json_output = json!({
                    "status": "connected",
                    "uptime_seconds": status.uptime.as_secs(),
                    "total_requests": status.total_requests,
                    "active_connections": status.active_connections,
                    "language_pools": status.language_pools
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            }
            _ => {
                println!("{}", "LSP Daemon Status".bold().green());
                println!("  {} {}", "Status:".bold(), "Connected".green());
                if !status.version.is_empty() {
                    println!("  {} {}", "Version:".bold(), status.version.cyan());
                }
                if !status.git_hash.is_empty() {
                    println!("  {} {}", "Git Hash:".bold(), status.git_hash.dimmed());
                }
                if !status.build_date.is_empty() {
                    println!("  {} {}", "Build Date:".bold(), status.build_date.dimmed());
                }
                println!(
                    "  {} {}",
                    "Uptime:".bold(),
                    Self::format_duration(status.uptime)
                );
                println!(
                    "  {} {}",
                    "Total Requests:".bold(),
                    status.total_requests.to_string().cyan()
                );
                println!(
                    "  {} {}",
                    "Active Connections:".bold(),
                    status.active_connections.to_string().cyan()
                );

                if !status.language_pools.is_empty() {
                    println!("\n{}", "Language Servers:".bold());
                    for (language, pool) in status.language_pools {
                        let status_text = if pool.available {
                            "Available".green()
                        } else {
                            "Unavailable".red()
                        };

                        println!(
                            "  {} {} ({})",
                            format!("{language}:").bold(),
                            status_text,
                            pool.status.dimmed()
                        );

                        if pool.uptime_secs > 0 {
                            let uptime = Self::format_duration(std::time::Duration::from_secs(
                                pool.uptime_secs,
                            ));
                            println!("    {} {}", "Uptime:".bold(), uptime.cyan());
                        }

                        println!(
                            "    {} Ready: {}, Busy: {}, Total: {}",
                            "Servers:".bold(),
                            pool.ready_servers.to_string().green(),
                            pool.busy_servers.to_string().yellow(),
                            pool.total_servers.to_string().cyan()
                        );

                        if !pool.workspaces.is_empty() {
                            println!(
                                "    {} ({})",
                                "Workspaces:".bold(),
                                pool.workspaces.len().to_string().cyan()
                            );
                            for workspace in &pool.workspaces {
                                // Show the absolute path as is
                                println!("      • {}", workspace.dimmed());
                            }
                        }
                    }
                } else {
                    println!("\n{}", "No language servers initialized".yellow());
                }
            }
        }

        Ok(())
    }

    /// List available languages
    async fn list_languages(format: &str) -> Result<()> {
        let config = LspConfig::default();
        let mut client = LspClient::new(config).await?;
        let languages = client.list_languages().await?;

        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(&languages)?);
            }
            _ => {
                println!("{}", "Available Language Servers".bold().green());

                if languages.is_empty() {
                    println!("  {}", "No language servers configured".yellow());
                    return Ok(());
                }

                for lang in languages {
                    let status_icon = if lang.available {
                        "✓".green()
                    } else {
                        "✗".red()
                    };
                    let status_text = if lang.available {
                        "Available"
                    } else {
                        "Not Available"
                    };

                    println!(
                        "  {} {} {} ({})",
                        status_icon,
                        format!("{:?}", lang.language).bold(),
                        status_text.dimmed(),
                        lang.lsp_server.dimmed()
                    );

                    if !lang.available {
                        println!(
                            "    {} {}",
                            "LSP Server:".yellow(),
                            lang.lsp_server.dimmed()
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Ping daemon for health check
    async fn ping(use_daemon: bool, workspace_hint: Option<String>, format: &str) -> Result<()> {
        let config = LspConfig {
            use_daemon,
            workspace_hint,
            timeout_ms: 5000, // 5 seconds for ping
        };

        let start_time = std::time::Instant::now();
        let mut client =
            match tokio::time::timeout(Duration::from_secs(10), LspClient::new(config)).await {
                Ok(Ok(client)) => client,
                Ok(Err(e)) => return Err(anyhow!("Failed to connect to daemon: {}", e)),
                Err(_) => return Err(anyhow!("Timeout connecting to daemon after 10 seconds")),
            };

        match tokio::time::timeout(Duration::from_secs(5), client.ping()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(anyhow!("Ping failed: {}", e)),
            Err(_) => return Err(anyhow!("Ping timeout after 5 seconds")),
        }
        let response_time = start_time.elapsed();

        match format {
            "json" => {
                let json_output = json!({
                    "status": "ok",
                    "response_time_ms": response_time.as_millis()
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            }
            _ => {
                println!(
                    "{} {} ({}ms)",
                    "✓".green(),
                    "LSP daemon is responsive".bold().green(),
                    response_time.as_millis().to_string().cyan()
                );
            }
        }

        Ok(())
    }

    /// Shutdown daemon
    async fn shutdown_daemon(format: &str) -> Result<()> {
        let config = LspConfig::default();
        let mut client = LspClient::new(config).await?;

        client.shutdown_daemon().await?;

        match format {
            "json" => {
                let json_output = json!({
                    "status": "shutdown",
                    "message": "LSP daemon shutdown successfully"
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            }
            _ => {
                println!(
                    "{} {}",
                    "✓".green(),
                    "LSP daemon shutdown successfully".bold().green()
                );
            }
        }

        Ok(())
    }

    /// Restart daemon
    async fn restart_daemon(workspace_hint: Option<String>, format: &str) -> Result<()> {
        // First shutdown existing daemon
        let config = LspConfig {
            use_daemon: true,
            workspace_hint: workspace_hint.clone(),
            timeout_ms: 30000, // Increased for rust-analyzer
        };

        let mut client = LspClient::new(config).await;

        // Try to shutdown if connected
        if let Ok(ref mut client) = client {
            let _ = client.shutdown_daemon().await;
        }

        // Wait a moment for shutdown to complete
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Start new daemon
        let config = LspConfig {
            use_daemon: true,
            workspace_hint,
            timeout_ms: 240000, // Increased to 4 minutes for full rust-analyzer indexing (90s) + call hierarchy (60s)
        };

        let mut client = LspClient::new(config).await?;

        // Verify it's working
        client.ping().await?;

        match format {
            "json" => {
                let json_output = json!({
                    "status": "restarted",
                    "message": "LSP daemon restarted successfully"
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            }
            _ => {
                println!(
                    "{} {}",
                    "✓".green(),
                    "LSP daemon restarted successfully".bold().green()
                );
            }
        }

        Ok(())
    }

    /// Show version information
    async fn show_version(format: &str) -> Result<()> {
        let version = env!("CARGO_PKG_VERSION");
        let git_hash = env!("GIT_HASH");
        let build_date = env!("BUILD_DATE");

        match format {
            "json" => {
                let json_output = json!({
                    "version": version,
                    "git_hash": git_hash,
                    "build_date": build_date,
                    "component": "probe-lsp-client"
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            }
            _ => {
                println!("{}", "Probe LSP Version Information".bold().green());
                println!("  {} {}", "Version:".bold(), version.cyan());
                println!("  {} {}", "Git Hash:".bold(), git_hash.dimmed());
                println!("  {} {}", "Build Date:".bold(), build_date.dimmed());
                println!("  {} {}", "Component:".bold(), "LSP Client".green());
            }
        }

        Ok(())
    }

    /// Handle LSP logs command
    async fn handle_logs(follow: bool, lines: usize, clear: bool) -> Result<()> {
        // Handle clear flag
        if clear {
            println!(
                "{}",
                "In-memory logs cannot be cleared (they auto-rotate)".yellow()
            );
            println!(
                "Restart the daemon to reset logs: {}",
                "probe lsp restart".cyan()
            );
            return Ok(());
        }

        // Connect to daemon to get logs (without auto-starting)
        let config = LspConfig {
            use_daemon: true,
            workspace_hint: None,
            timeout_ms: 10000, // Short timeout for logs
        };
        let mut client = match LspClient::new(config).await {
            Ok(client) => client,
            Err(_) => {
                println!("{}", "LSP daemon is not running".red());
                println!("Start the daemon with: {}", "probe lsp start".cyan());
                return Ok(());
            }
        };

        if follow {
            // Follow mode - poll for new logs using sequence numbers
            println!(
                "{}",
                "Following LSP daemon log (Ctrl+C to stop)..."
                    .green()
                    .bold()
            );
            println!("{}", "─".repeat(60).dimmed());

            // First show the last N lines with timeout
            let entries = match time::timeout(
                Duration::from_millis(LOG_RPC_TIMEOUT_MS),
                client.get_logs(lines),
            )
            .await
            {
                Err(_) => {
                    println!("{} Failed to get logs: timed out", "❌".red());
                    return Ok(());
                }
                Ok(Ok(entries)) => {
                    for entry in &entries {
                        Self::print_log_entry(entry);
                    }
                    entries
                }
                Ok(Err(e)) => {
                    println!("{} Failed to get logs: {}", "❌".red(), e);
                    return Ok(());
                }
            };

            // Track the last sequence number seen to avoid duplicates
            let mut last_seen_sequence = entries
                .iter()
                .map(|entry| entry.sequence)
                .max()
                .unwrap_or(0);

            // Poll for new logs with interval to avoid backlog
            let mut ticker = time::interval(Duration::from_millis(LOG_FOLLOW_POLL_MS));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                // Bound the RPC to avoid wedging follow-mode forever if the daemon/socket stalls
                match time::timeout(
                    Duration::from_millis(LOG_RPC_TIMEOUT_MS),
                    client.get_logs_since(last_seen_sequence, LOG_FETCH_LIMIT),
                )
                .await
                {
                    Err(_) => {
                        // Timed out talking to the daemon; continue polling without blocking the UI
                        continue;
                    }
                    Ok(Ok(new_entries)) => {
                        // Show only entries with sequence numbers newer than our last seen
                        for entry in &new_entries {
                            if entry.sequence > last_seen_sequence {
                                Self::print_log_entry(entry);
                                last_seen_sequence = entry.sequence;
                            }
                        }
                    }
                    Ok(Err(_e)) => {
                        // Check if daemon is still running
                        if client.ping().await.is_err() {
                            println!("\n{}", "Daemon connection lost".yellow());
                            break;
                        }
                        // Otherwise, continue polling
                    }
                }
            }
        } else {
            // Show last N lines with timeout
            match time::timeout(
                Duration::from_millis(LOG_RPC_TIMEOUT_MS),
                client.get_logs(lines),
            )
            .await
            {
                Err(_) => {
                    println!("{} Failed to get logs: timed out", "❌".red());
                }
                Ok(Ok(entries)) => {
                    if entries.is_empty() {
                        println!("{}", "No logs available".yellow());
                        return Ok(());
                    }

                    let total_entries = entries.len();
                    println!(
                        "{}",
                        format!("LSP Daemon Log (last {total_entries} entries)")
                            .bold()
                            .green()
                    );
                    println!("{}", "─".repeat(60).dimmed());

                    for entry in &entries {
                        Self::print_log_entry(entry);
                    }

                    println!("{}", "─".repeat(60).dimmed());
                    println!("Use {} to follow log in real-time", "--follow".cyan());
                    println!(
                        "Use {} to restart daemon (clears logs)",
                        "probe lsp restart".cyan()
                    );
                }
                Ok(Err(e)) => {
                    println!("{} Failed to get logs: {}", "❌".red(), e);
                }
            }
        }

        Ok(())
    }

    /// Start embedded LSP daemon
    async fn start_embedded_daemon(
        socket: Option<String>,
        log_level: String,
        foreground: bool,
    ) -> Result<()> {
        // Check if we're being run via cargo and warn about potential conflicts
        if std::env::current_exe()
            .map(|path| path.to_string_lossy().contains("cargo"))
            .unwrap_or(false)
        {
            eprintln!(
                "⚠️  WARNING: Running LSP daemon via 'cargo run' may cause build lock conflicts."
            );
            eprintln!("   For better performance, build first: cargo build");
            eprintln!("   Then use: ./target/debug/probe lsp start -f");
        }

        // Don't initialize tracing here - let the daemon handle it with memory logging
        // The daemon will set up both memory logging and stderr logging as needed
        if std::env::var("LSP_LOG").is_ok() {
            eprintln!("LSP logging enabled - logs stored in-memory (use 'probe lsp logs' to view)");
        }

        // Determine socket path
        let socket_path = socket.unwrap_or_else(lsp_daemon::get_default_socket_path);

        // Check if daemon is already running by trying to connect
        // Skip this check if we're in foreground mode (likely being spawned by background mode)
        if !foreground {
            match lsp_daemon::ipc::IpcStream::connect(&socket_path).await {
                Ok(_stream) => {
                    eprintln!("❌ LSP daemon is already running on socket: {socket_path}");
                    eprintln!("   Use 'probe lsp status' to check the current daemon");
                    eprintln!("   Use 'probe lsp shutdown' to stop the current daemon");
                    eprintln!("   Use 'probe lsp restart' to restart the daemon");
                    return Err(anyhow::anyhow!("Daemon already running"));
                }
                Err(_) => {
                    // Socket file might be stale, clean it up
                    if std::path::Path::new(&socket_path).exists() {
                        println!("🧹 Cleaning up stale socket file: {socket_path}");
                        if let Err(e) = std::fs::remove_file(&socket_path) {
                            eprintln!("⚠️  Warning: Failed to remove stale socket: {e}");
                        }
                    }
                }
            }
        }

        println!("🚀 Starting embedded LSP daemon...");
        println!("   Socket: {socket_path}");
        println!("   Log Level: {log_level}");

        if foreground {
            println!("   Mode: Foreground");
        } else {
            println!("   Mode: Background");
        }

        // Create and start daemon
        let daemon = LspDaemon::new(socket_path.clone())?;

        if foreground {
            println!("✓ LSP daemon started in foreground mode");
            daemon.run().await?;
        } else {
            // For background mode, fork a new process
            use std::process::{Command, Stdio};

            // Get the current executable path
            let exe_path = std::env::current_exe()?;

            // Fork the daemon as a separate process
            let child = Command::new(&exe_path)
                .args([
                    "lsp",
                    "start",
                    "-f",
                    "--socket",
                    &socket_path,
                    "--log-level",
                    &log_level,
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;

            println!(
                "✓ LSP daemon started in background mode (PID: {})",
                child.id()
            );
            println!("   Use 'probe lsp status' to check daemon status");
            println!("   Use 'probe lsp logs' to view daemon logs");

            // Wait a moment to ensure daemon starts
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // Verify daemon is running
            match lsp_daemon::ipc::IpcStream::connect(&socket_path).await {
                Ok(_) => {
                    // Daemon is running successfully
                }
                Err(e) => {
                    eprintln!("⚠️  Warning: Could not verify daemon started: {e}");
                }
            }
        }

        Ok(())
    }

    /// Print a log entry with proper formatting and colors
    fn print_log_entry(entry: &LogEntry) {
        let level_color = match entry.level {
            LogLevel::Error => "ERROR".red().bold(),
            LogLevel::Warn => "WARN".yellow().bold(),
            LogLevel::Info => "INFO".blue().bold(),
            LogLevel::Debug => "DEBUG".dimmed(),
            LogLevel::Trace => "TRACE".dimmed(),
        };

        let timestamp = entry.timestamp.dimmed();
        let target = if entry.target.is_empty() {
            "".to_string()
        } else {
            format!(" [{}]", entry.target.dimmed())
        };

        // Check if message looks like JSON and try to format it
        let formatted_message = if entry.message.trim_start().starts_with('{') {
            match serde_json::from_str::<serde_json::Value>(&entry.message) {
                Ok(parsed) => match serde_json::to_string_pretty(&parsed) {
                    Ok(pretty) => pretty,
                    Err(_) => entry.message.clone(),
                },
                Err(_) => entry.message.clone(),
            }
        } else {
            entry.message.clone()
        };

        // Apply message-specific coloring
        let colored_message = if entry.message.contains(">>> TO LSP:") {
            formatted_message.cyan()
        } else if entry.message.contains("<<< FROM LSP:") {
            formatted_message.green()
        } else {
            match entry.level {
                LogLevel::Error => formatted_message.red(),
                LogLevel::Warn => formatted_message.yellow(),
                LogLevel::Info => formatted_message.normal(),
                LogLevel::Debug | LogLevel::Trace => formatted_message.dimmed(),
            }
        };

        println!("{timestamp} {level_color}{target} {colored_message}");

        // Show file/line info if available
        if let (Some(file), Some(line)) = (&entry.file, entry.line) {
            println!(
                "    {} {}:{}",
                "at".dimmed(),
                file.dimmed(),
                line.to_string().dimmed()
            );
        }
    }

    /// Initialize language servers for workspaces
    async fn init_workspaces(
        workspace: Option<String>,
        languages: Option<String>,
        recursive: bool,
        use_daemon: bool,
        enable_watchdog: bool,
        format: &str,
    ) -> Result<()> {
        use std::path::PathBuf;

        // Determine workspace root
        let workspace_root = if let Some(ws) = workspace {
            let path = PathBuf::from(ws);
            // Always normalize to canonical absolute path to avoid mismatches due to symlinks
            // (e.g., /var vs /private/var on macOS) or case differences on Windows.
            let abs = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .context("Failed to get current directory")?
                    .join(&path)
            };
            // Try to canonicalize, but if it fails and the path exists, use the absolute path as-is
            match abs.canonicalize() {
                Ok(canonical) => canonical,
                Err(canon_err) => {
                    // Debug: Check if path exists
                    eprintln!(
                        "Canonicalization failed for {}: {}",
                        abs.display(),
                        canon_err
                    );
                    eprintln!("Checking if path exists: {}", abs.exists());

                    if abs.exists() {
                        // Path exists but can't be canonicalized (e.g., symlink issues in CI)
                        // Use the absolute path as-is
                        eprintln!(
                            "Warning: Path exists but could not canonicalize {abs:?}, using as-is"
                        );
                        abs
                    } else {
                        // Path doesn't exist - provide detailed error
                        eprintln!("Path does not exist: {}", abs.display());
                        // Check parent directory
                        if let Some(parent) = abs.parent() {
                            eprintln!(
                                "Parent directory {} exists: {}",
                                parent.display(),
                                parent.exists()
                            );
                        }
                        return Err(anyhow::anyhow!(
                            "Workspace path does not exist: '{}'",
                            abs.display()
                        ));
                    }
                }
            }
        } else {
            // Default to current directory, canonicalized
            std::env::current_dir()
                .context("Failed to get current directory")?
                .canonicalize()
                .context("Failed to canonicalize current directory")?
        };

        // Validate workspace exists (after canonicalization for relative paths)
        if !workspace_root.exists() {
            // Double-check with metadata to see if it's a permission issue
            if let Err(e) = std::fs::metadata(&workspace_root) {
                eprintln!(
                    "Failed to get metadata for workspace {}: {}",
                    workspace_root.display(),
                    e
                );
            }
            // Try listing parent directory to see what's there
            if let Some(parent) = workspace_root.parent() {
                if let Ok(entries) = std::fs::read_dir(parent) {
                    eprintln!("Contents of parent directory {}:", parent.display());
                    for entry in entries.flatten() {
                        eprintln!("  - {:?}", entry.file_name());
                    }
                }
            }
            return Err(anyhow::anyhow!(
                "Workspace does not exist: {}",
                workspace_root.display()
            ));
        }

        // Parse languages if provided
        let languages = languages.map(|langs| {
            langs
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        });

        // Create client
        let config = LspConfig {
            use_daemon,
            workspace_hint: Some(workspace_root.to_string_lossy().to_string()),
            timeout_ms: 60000, // 60 seconds for initialization
        };

        let mut client = LspClient::new(config).await?;

        match format {
            "json" => {
                // Initialize workspaces
                let (initialized, errors) = client
                    .init_workspaces(
                        workspace_root.clone(),
                        languages,
                        recursive,
                        enable_watchdog,
                    )
                    .await?;

                let json_output = json!({
                    "workspace_root": workspace_root.to_string_lossy(),
                    "recursive": recursive,
                    "initialized": initialized,
                    "errors": errors,
                    "summary": {
                        "total_initialized": initialized.len(),
                        "total_errors": errors.len()
                    }
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            }
            _ => {
                println!(
                    "{} {}",
                    "Discovering workspaces in".bold().blue(),
                    workspace_root.display().to_string().cyan()
                );
                if recursive {
                    println!("  {} {}", "Mode:".bold(), "Recursive".yellow());
                }
                if let Some(ref langs) = languages {
                    println!("  {} {}", "Languages:".bold(), langs.join(", ").green());
                }
                println!();

                // Initialize workspaces
                let (initialized, errors) = client
                    .init_workspaces(workspace_root, languages, recursive, enable_watchdog)
                    .await?;

                if initialized.is_empty() && errors.is_empty() {
                    println!("{}", "No workspaces found to initialize".yellow());
                    return Ok(());
                }

                // Group initialized workspaces by language
                use std::collections::HashMap;
                let mut by_language: HashMap<String, Vec<String>> = HashMap::new();
                for workspace in &initialized {
                    let lang = format!("{:?}", workspace.language);
                    let workspace_str = workspace.workspace_root.to_string_lossy().to_string();
                    by_language.entry(lang).or_default().push(workspace_str);
                }

                // Display results
                if !initialized.is_empty() {
                    println!("{}", "Initialized language servers:".bold().green());
                    for (language, workspaces) in &by_language {
                        println!(
                            "  {} {} {}:",
                            "✓".green(),
                            language.bold(),
                            format!("({})", workspaces.len()).dimmed()
                        );
                        for workspace in workspaces {
                            // Show the absolute path as is
                            println!("    • {}", workspace.dimmed());
                        }
                    }
                }

                if !errors.is_empty() {
                    println!("\n{}", "Errors:".bold().red());
                    for error in &errors {
                        println!("  {} {}", "✗".red(), error);
                    }
                }

                // Summary
                println!();
                if initialized.is_empty() {
                    println!("{}", "No language servers were initialized".yellow().bold());
                } else {
                    let server_count = by_language.len();
                    let workspace_count = initialized.len();
                    println!(
                        "{} {} {} {} {} {}",
                        "Successfully initialized".green(),
                        server_count.to_string().bold(),
                        if server_count == 1 {
                            "language server"
                        } else {
                            "language servers"
                        },
                        "for".green(),
                        workspace_count.to_string().bold(),
                        if workspace_count == 1 {
                            "workspace"
                        } else {
                            "workspaces"
                        }
                    );
                }
            }
        }

        Ok(())
    }

    /// Handle cache management commands
    async fn handle_cache_command(cache_command: &CacheSubcommands, format: &str) -> Result<()> {
        let config = LspConfig::default();
        let mut client = LspClient::new(config).await?;

        match cache_command {
            CacheSubcommands::Stats => {
                let stats = client.cache_stats().await?;

                match format {
                    "json" => {
                        println!("{}", serde_json::to_string_pretty(&stats)?);
                    }
                    _ => {
                        println!("{}", "LSP Cache Statistics".bold().green());
                        println!(
                            "  {} {}",
                            "Total Memory Usage:".bold(),
                            format_bytes(stats.total_memory_usage)
                        );

                        if let Some(ref cache_dir) = stats.cache_directory {
                            println!("  {} {}", "Cache Directory:".bold(), cache_dir.cyan());
                        }

                        println!(
                            "  {} {}",
                            "Persistent Cache:".bold(),
                            if stats.persistent_cache_enabled {
                                "Enabled".green()
                            } else {
                                "Disabled".dimmed()
                            }
                        );

                        if !stats.per_operation.is_empty() {
                            println!("\n{}", "Per-Operation Statistics:".bold());
                            for op_stats in &stats.per_operation {
                                let hit_rate = if op_stats.hit_count + op_stats.miss_count > 0 {
                                    (op_stats.hit_count as f64
                                        / (op_stats.hit_count + op_stats.miss_count) as f64)
                                        * 100.0
                                } else {
                                    0.0
                                };

                                println!("  {} {:?}:", "•".cyan(), op_stats.operation);
                                println!(
                                    "    {} {}",
                                    "Entries:".bold(),
                                    op_stats.total_entries.to_string().cyan()
                                );
                                println!(
                                    "    {} {} / {} ({:.1}% hit rate)",
                                    "Cache Hits/Misses:".bold(),
                                    op_stats.hit_count.to_string().green(),
                                    op_stats.miss_count.to_string().yellow(),
                                    hit_rate
                                );
                                println!(
                                    "    {} {}",
                                    "Evictions:".bold(),
                                    op_stats.eviction_count.to_string().dimmed()
                                );
                                println!(
                                    "    {} {}",
                                    "In-flight:".bold(),
                                    op_stats.inflight_count.to_string().cyan()
                                );
                                println!(
                                    "    {} {}",
                                    "Memory:".bold(),
                                    format_bytes(op_stats.memory_usage_estimate)
                                );
                            }
                        } else {
                            println!("\n{}", "No cache statistics available".yellow());
                        }
                    }
                }
            }
            CacheSubcommands::Clear { operation } => {
                let (cleared_operations, total_cleared) = if let Some(op_name) = operation {
                    // Parse the operation name
                    let lsp_operation = match op_name.to_lowercase().as_str() {
                        "definition" => Some(lsp_daemon::cache_types::LspOperation::Definition),
                        "references" => Some(lsp_daemon::cache_types::LspOperation::References),
                        "hover" => Some(lsp_daemon::cache_types::LspOperation::Hover),
                        "callhierarchy" => Some(lsp_daemon::cache_types::LspOperation::CallHierarchy),
                        "documentsymbols" => Some(lsp_daemon::cache_types::LspOperation::DocumentSymbols),
                        _ => return Err(anyhow!("Invalid operation '{}'. Valid operations: Definition, References, Hover, CallHierarchy, DocumentSymbols", op_name)),
                    };
                    client.cache_clear(lsp_operation).await?
                } else {
                    client.cache_clear(None).await?
                };

                match format {
                    "json" => {
                        let json_output = json!({
                            "cleared_operations": cleared_operations,
                            "total_entries_cleared": total_cleared
                        });
                        println!("{}", serde_json::to_string_pretty(&json_output)?);
                    }
                    _ => {
                        if cleared_operations.is_empty() {
                            println!("{}", "No cache entries to clear".yellow());
                        } else {
                            println!(
                                "{} {} from {} operations:",
                                "Cleared".bold().green(),
                                format!("{total_cleared} entries").cyan(),
                                format!("{}", cleared_operations.len()).cyan()
                            );
                            for op in &cleared_operations {
                                println!("  {} {:?}", "•".green(), op);
                            }
                        }
                    }
                }
            }
            CacheSubcommands::Export { operation } => {
                let export_data = if let Some(op_name) = operation {
                    // Parse the operation name
                    let lsp_operation = match op_name.to_lowercase().as_str() {
                        "definition" => Some(lsp_daemon::cache_types::LspOperation::Definition),
                        "references" => Some(lsp_daemon::cache_types::LspOperation::References),
                        "hover" => Some(lsp_daemon::cache_types::LspOperation::Hover),
                        "callhierarchy" => Some(lsp_daemon::cache_types::LspOperation::CallHierarchy),
                        "documentsymbols" => Some(lsp_daemon::cache_types::LspOperation::DocumentSymbols),
                        _ => return Err(anyhow!("Invalid operation '{}'. Valid operations: Definition, References, Hover, CallHierarchy, DocumentSymbols", op_name)),
                    };
                    client.cache_export(lsp_operation).await?
                } else {
                    client.cache_export(None).await?
                };

                match format {
                    "json" => {
                        // Export data is already JSON, so we can print it directly
                        println!("{export_data}");
                    }
                    _ => {
                        println!("{}", "Cache Export Data:".bold().green());
                        // Pretty print the JSON for human consumption
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&export_data)
                        {
                            println!("{}", serde_json::to_string_pretty(&parsed)?);
                        } else {
                            println!("{export_data}");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle index command - start indexing
    #[allow(clippy::too_many_arguments)]
    async fn handle_index_command(
        workspace: Option<String>,
        languages: Option<String>,
        recursive: bool,
        max_workers: Option<usize>,
        memory_budget: Option<u64>,
        format: &str,
        show_progress: bool,
        wait: bool,
    ) -> Result<()> {
        let config = LspConfig::default();
        let mut client = LspClient::new(config).await?;

        // Resolve workspace path
        let workspace_root = if let Some(ws) = workspace {
            let path = std::path::PathBuf::from(ws);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir()?.join(&path).canonicalize()?
            }
        } else {
            std::env::current_dir()?
        };

        // Parse languages if provided
        let language_list = if let Some(langs) = languages {
            langs.split(',').map(|s| s.trim().to_string()).collect()
        } else {
            vec![]
        };

        // Create indexing config
        let indexing_config = lsp_daemon::protocol::IndexingConfig {
            max_workers,
            memory_budget_mb: memory_budget,
            exclude_patterns: vec![
                "*.git/*".to_string(),
                "*/node_modules/*".to_string(),
                "*/target/*".to_string(),
                "*/build/*".to_string(),
                "*/dist/*".to_string(),
            ],
            include_patterns: vec![],
            max_file_size_mb: Some(10),
            incremental: Some(true),
            languages: language_list,
            recursive,
        };

        match client
            .start_indexing(workspace_root.clone(), indexing_config)
            .await
        {
            Ok(session_id) => {
                match format {
                    "json" => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "status": "started",
                                "session_id": session_id,
                                "workspace_root": workspace_root,
                                "recursive": recursive
                            }))?
                        );
                    }
                    _ => {
                        println!("{}", "Indexing Started".bold().green());
                        println!("  {}: {}", "Session ID".bold(), session_id);
                        println!("  {}: {:?}", "Workspace".bold(), workspace_root);
                        println!("  {}: {}", "Recursive".bold(), recursive);

                        if show_progress {
                            println!("\nStarting progress monitoring...");
                            if wait {
                                Self::monitor_indexing_progress(&mut client, &session_id).await?;
                            } else {
                                println!(
                                    "Use 'probe lsp index-status --follow' to monitor progress"
                                );
                            }
                        }
                    }
                }
                Ok(())
            }
            Err(e) => {
                match format {
                    "json" => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "status": "error",
                                "error": e.to_string()
                            }))?
                        );
                    }
                    _ => {
                        eprintln!("{} {}", "Failed to start indexing:".red(), e);
                    }
                }
                Err(e)
            }
        }
    }

    /// Handle index status command
    async fn handle_index_status_command(
        format: &str,
        detailed: bool,
        follow: bool,
        interval: u64,
    ) -> Result<()> {
        let config = LspConfig::default();
        let mut client = LspClient::new(config).await?;

        if follow {
            Self::follow_indexing_status(&mut client, format, detailed, interval).await
        } else {
            let status = client.get_indexing_status().await?;
            Self::display_indexing_status(&status, format, detailed).await
        }
    }

    /// Handle index stop command
    async fn handle_index_stop_command(force: bool, format: &str) -> Result<()> {
        let config = LspConfig::default();
        let mut client = LspClient::new(config).await?;

        match client.stop_indexing(force).await {
            Ok(was_running) => {
                match format {
                    "json" => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "status": "stopped",
                                "was_running": was_running,
                                "force": force
                            }))?
                        );
                    }
                    _ => {
                        if was_running {
                            println!("{}", "Indexing stopped successfully".green());
                        } else {
                            println!("{}", "No indexing was running".yellow());
                        }
                    }
                }
                Ok(())
            }
            Err(e) => {
                match format {
                    "json" => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "status": "error",
                                "error": e.to_string()
                            }))?
                        );
                    }
                    _ => {
                        eprintln!("{} {}", "Failed to stop indexing:".red(), e);
                    }
                }
                Err(e)
            }
        }
    }

    /// Handle index config command
    async fn handle_index_config_command(
        config_command: &IndexConfigSubcommands,
        _format: &str,
    ) -> Result<()> {
        let config = LspConfig::default();
        let mut client = LspClient::new(config).await?;

        match config_command {
            IndexConfigSubcommands::Show {
                format: output_format,
            } => {
                let config = client.get_indexing_config().await?;
                Self::display_indexing_config(&config, output_format).await
            }
            IndexConfigSubcommands::Set {
                max_workers,
                memory_budget,
                exclude_patterns,
                include_patterns,
                max_file_size,
                incremental,
                format,
            } => {
                let mut config = client.get_indexing_config().await?;

                // Update config fields if provided
                if let Some(workers) = max_workers {
                    config.max_workers = Some(*workers);
                }
                if let Some(memory) = memory_budget {
                    config.memory_budget_mb = Some(*memory);
                }
                if let Some(exclude) = exclude_patterns {
                    config.exclude_patterns =
                        exclude.split(',').map(|s| s.trim().to_string()).collect();
                }
                if let Some(include) = include_patterns {
                    config.include_patterns =
                        include.split(',').map(|s| s.trim().to_string()).collect();
                }
                if let Some(file_size) = max_file_size {
                    config.max_file_size_mb = Some(*file_size);
                }
                if let Some(inc) = incremental {
                    config.incremental = Some(*inc);
                }

                client.set_indexing_config(config.clone()).await?;
                Self::display_indexing_config(&config, format).await
            }
            IndexConfigSubcommands::Reset { format } => {
                let default_config = lsp_daemon::protocol::IndexingConfig::default();
                client.set_indexing_config(default_config.clone()).await?;
                Self::display_indexing_config(&default_config, format).await
            }
        }
    }

    /// Display indexing status information
    async fn display_indexing_status(
        status: &lsp_daemon::protocol::IndexingStatusInfo,
        format: &str,
        detailed: bool,
    ) -> Result<()> {
        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(status)?);
            }
            _ => {
                println!("{}", "Indexing Status".bold().green());
                println!("  {}: {}", "Status".bold(), status.manager_status);

                if let Some(session_id) = &status.session_id {
                    println!("  {}: {}", "Session ID".bold(), session_id);
                }

                if let Some(started_at) = status.started_at {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let elapsed_friendly =
                        Self::format_duration(std::time::Duration::from_secs(now - started_at));
                    println!("  {}: {} ago", "Started".bold(), elapsed_friendly);
                }

                println!("\n{}", "Progress".bold().cyan());
                let progress = &status.progress;
                println!(
                    "  {}: {}/{} ({:.1}%)",
                    "Files".bold(),
                    progress.processed_files + progress.failed_files + progress.skipped_files,
                    progress.total_files,
                    progress.progress_ratio * 100.0
                );

                if progress.files_per_second > 0.0 {
                    println!(
                        "  {}: {:.1} files/sec",
                        "Rate".bold(),
                        progress.files_per_second
                    );
                }

                println!("  {}: {}", "Processed".bold(), progress.processed_files);
                println!("  {}: {}", "Failed".bold(), progress.failed_files);
                println!("  {}: {}", "Skipped".bold(), progress.skipped_files);
                println!("  {}: {}", "Active".bold(), progress.active_files);
                println!(
                    "  {}: {} symbols",
                    "Extracted".bold(),
                    progress.symbols_extracted
                );
                println!(
                    "  {}: {}",
                    "Memory".bold(),
                    format_bytes(progress.memory_usage_bytes as usize)
                );

                if progress.peak_memory_bytes > progress.memory_usage_bytes {
                    println!(
                        "  {}: {}",
                        "Peak Memory".bold(),
                        format_bytes(progress.peak_memory_bytes as usize)
                    );
                }

                println!("\n{}", "Queue".bold().cyan());
                let queue = &status.queue;
                println!("  {}: {}", "Total Items".bold(), queue.total_items);
                println!("  {}: {}", "Pending".bold(), queue.pending_items);
                println!(
                    "  {}: {} / {} / {}",
                    "Priority (H/M/L)".bold(),
                    queue.high_priority_items,
                    queue.medium_priority_items,
                    queue.low_priority_items
                );

                if queue.is_paused {
                    println!("  {}: {}", "Status".bold(), "⏸️  PAUSED".yellow());
                }
                if queue.memory_pressure {
                    println!("  {}: {}", "Memory Pressure".bold(), "⚠️  HIGH".red());
                }

                if detailed && !status.workers.is_empty() {
                    println!("\n{}", "Workers".bold().cyan());
                    for worker in &status.workers {
                        let status_icon = if worker.is_active { "🟢" } else { "⚪" };
                        println!(
                            "  {} Worker {}: {} files, {} errors",
                            status_icon,
                            worker.worker_id,
                            worker.files_processed,
                            worker.errors_encountered
                        );

                        if let Some(ref current_file) = worker.current_file {
                            println!("    Currently: {current_file:?}");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Display indexing configuration
    async fn display_indexing_config(
        config: &lsp_daemon::protocol::IndexingConfig,
        format: &str,
    ) -> Result<()> {
        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(config)?);
            }
            _ => {
                println!("{}", "Indexing Configuration".bold().green());
                println!(
                    "  {}: {}",
                    "Max Workers".bold(),
                    config
                        .max_workers
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "Auto".to_string())
                );
                println!(
                    "  {}: {}MB",
                    "Memory Budget".bold(),
                    config
                        .memory_budget_mb
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "Unlimited".to_string())
                );
                println!(
                    "  {}: {}MB",
                    "Max File Size".bold(),
                    config
                        .max_file_size_mb
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "Unlimited".to_string())
                );
                println!(
                    "  {}: {}",
                    "Incremental".bold(),
                    config
                        .incremental
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "true".to_string())
                );
                println!("  {}: {}", "Recursive".bold(), config.recursive);

                if !config.languages.is_empty() {
                    println!("  {}: {}", "Languages".bold(), config.languages.join(", "));
                } else {
                    println!("  {}: All supported", "Languages".bold());
                }

                if !config.exclude_patterns.is_empty() {
                    println!(
                        "  {}: {}",
                        "Exclude Patterns".bold(),
                        config.exclude_patterns.join(", ")
                    );
                }

                if !config.include_patterns.is_empty() {
                    println!(
                        "  {}: {}",
                        "Include Patterns".bold(),
                        config.include_patterns.join(", ")
                    );
                } else {
                    println!("  {}: All files", "Include Patterns".bold());
                }
            }
        }
        Ok(())
    }

    /// Follow indexing status updates
    async fn follow_indexing_status(
        client: &mut LspClient,
        format: &str,
        detailed: bool,
        interval: u64,
    ) -> Result<()> {
        let mut interval = tokio::time::interval(Duration::from_secs(interval));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;

            match client.get_indexing_status().await {
                Ok(status) => {
                    // Clear screen for terminal output
                    if format != "json" {
                        print!("\x1B[2J\x1B[1;1H"); // ANSI escape codes to clear screen and move cursor to top
                    }

                    Self::display_indexing_status(&status, format, detailed).await?;

                    // Check if indexing is complete
                    if status.manager_status == "Idle" || status.manager_status == "Shutdown" {
                        if format != "json" {
                            println!("\n{}", "Indexing completed".green());
                        }
                        break;
                    }
                }
                Err(e) => {
                    if format == "json" {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "status": "error",
                                "error": e.to_string()
                            }))?
                        );
                    } else {
                        eprintln!("Error getting status: {e}");
                    }
                    break;
                }
            }
        }
        Ok(())
    }

    /// Monitor indexing progress until completion
    async fn monitor_indexing_progress(client: &mut LspClient, session_id: &str) -> Result<()> {
        let mut last_files_processed = 0u64;
        let mut interval = tokio::time::interval(Duration::from_secs(2));

        println!("Monitoring indexing progress (Ctrl+C to stop monitoring)...\n");

        loop {
            interval.tick().await;

            match client.get_indexing_status().await {
                Ok(status) => {
                    if let Some(ref current_session) = status.session_id {
                        if current_session != session_id {
                            println!("Session changed, stopping monitor");
                            break;
                        }
                    }

                    let progress = &status.progress;
                    let completed =
                        progress.processed_files + progress.failed_files + progress.skipped_files;

                    // Show progress bar
                    if progress.total_files > 0 {
                        let percent = (completed as f64 / progress.total_files as f64) * 100.0;
                        let bar_width = 50;
                        let filled = (percent / 100.0 * bar_width as f64) as usize;
                        let bar = "█".repeat(filled) + &"░".repeat(bar_width - filled);

                        print!(
                            "\r[{}] {:.1}% ({}/{}) {} files/sec",
                            bar,
                            percent,
                            completed,
                            progress.total_files,
                            progress.files_per_second
                        );

                        if completed != last_files_processed {
                            let memory_str = format_bytes(progress.memory_usage_bytes as usize);
                            println!(" | {} | {} symbols", memory_str, progress.symbols_extracted);
                            last_files_processed = completed;
                        }
                    } else {
                        println!("Discovering files...");
                    }

                    // Check if indexing is complete
                    if status.manager_status == "Idle" || status.manager_status == "Shutdown" {
                        println!("\n\n{}", "✅ Indexing completed successfully!".green());
                        println!(
                            "  {}: {} files processed",
                            "Final Stats".bold(),
                            progress.processed_files
                        );
                        println!(
                            "  {}: {} symbols extracted",
                            "".repeat(11),
                            progress.symbols_extracted
                        );
                        println!("  {}: {} failed", "".repeat(11), progress.failed_files);
                        break;
                    } else if status.manager_status.contains("Error") {
                        println!("\n\n{}", "❌ Indexing failed".red());
                        break;
                    }
                }
                Err(e) => {
                    println!("\nError monitoring progress: {e}");
                    break;
                }
            }
        }
        Ok(())
    }

    /// Format duration in a human-readable way
    fn format_duration(duration: Duration) -> String {
        let total_seconds = duration.as_secs();

        if total_seconds < 60 {
            format!("{total_seconds}s")
        } else if total_seconds < 3600 {
            let minutes = total_seconds / 60;
            let seconds = total_seconds % 60;
            format!("{minutes}m {seconds}s")
        } else {
            let hours = total_seconds / 3600;
            let minutes = (total_seconds % 3600) / 60;
            format!("{hours}h {minutes}m")
        }
    }
}

/// Format bytes in a human-readable way
fn format_bytes(bytes: usize) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::search_runner::format_duration;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30.00s");
        assert_eq!(format_duration(Duration::from_secs(90)), "90.00s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "3661.00s");
    }

    #[test]
    fn test_workspace_path_resolution() {
        use std::path::PathBuf;
        use tempfile::TempDir;

        // Create a temporary directory for testing
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let temp_path = temp_dir.path();

        // Create a test subdirectory
        let test_subdir = temp_path.join("test-workspace");
        std::fs::create_dir(&test_subdir).expect("Failed to create test subdirectory");

        // Test relative path resolution
        let original_dir = std::env::current_dir().expect("Failed to get current dir");
        std::env::set_current_dir(temp_path).expect("Failed to change directory");

        // Test the path resolution logic (extracted from init_workspaces)
        let workspace_path = Some("test-workspace".to_string());
        let workspace_root = if let Some(ws) = workspace_path {
            let path = PathBuf::from(ws);
            // Convert relative paths to absolute paths for URI conversion
            if path.is_absolute() {
                path
            } else {
                // For relative paths, resolve them relative to current directory
                std::env::current_dir()
                    .context("Failed to get current directory")
                    .unwrap()
                    .join(&path)
                    .canonicalize()
                    .context(format!(
                        "Failed to resolve workspace path '{}'. Make sure the path exists and is accessible",
                        path.display()
                    ))
                    .unwrap()
            }
        } else {
            std::env::current_dir().unwrap()
        };

        // Restore original directory
        std::env::set_current_dir(original_dir).expect("Failed to restore directory");

        // Verify the path was resolved correctly
        assert!(workspace_root.is_absolute());
        assert!(workspace_root.exists());

        // On Windows, canonicalization might produce different but equivalent paths
        // (e.g., UNC paths, different drive letter casing, etc.)
        // So we check that both paths canonicalize to the same result
        let expected_canonical = test_subdir.canonicalize().unwrap();
        let actual_canonical = workspace_root.canonicalize().unwrap();
        assert_eq!(actual_canonical, expected_canonical);
    }

    #[test]
    fn test_path_is_absolute_after_resolution() {
        use std::path::PathBuf;
        use tempfile::TempDir;

        // Create a temporary directory for testing
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let temp_path = temp_dir.path();

        // Create a test subdirectory
        let test_subdir = temp_path.join("test-workspace");
        std::fs::create_dir(&test_subdir).expect("Failed to create test subdirectory");

        // Change to the temp directory
        let original_dir = std::env::current_dir().expect("Failed to get current dir");
        std::env::set_current_dir(temp_path).expect("Failed to change directory");

        // Test that relative path gets resolved to absolute
        let workspace_path = Some("test-workspace".to_string());
        let workspace_root = if let Some(ws) = workspace_path {
            let path = PathBuf::from(ws);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .context("Failed to get current directory")
                    .unwrap()
                    .join(&path)
                    .canonicalize()
                    .context(format!(
                        "Failed to resolve workspace path '{}'. Make sure the path exists and is accessible",
                        path.display()
                    ))
                    .unwrap()
            }
        } else {
            std::env::current_dir().unwrap()
        };

        // Restore original directory
        std::env::set_current_dir(original_dir).expect("Failed to restore directory");

        // Critical test: the path should be absolute (required for URI conversion)
        assert!(
            workspace_root.is_absolute(),
            "Resolved path should be absolute for URI conversion: {workspace_root:?}"
        );
        assert!(workspace_root.exists());
    }

    #[test]
    fn test_absolute_path_passthrough() {
        use std::path::PathBuf;
        use tempfile::TempDir;

        // Create a temporary directory for testing
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let absolute_path = temp_dir
            .path()
            .canonicalize()
            .expect("Failed to canonicalize");

        // Test that absolute paths are passed through unchanged
        let workspace_path = Some(absolute_path.to_string_lossy().to_string());
        let workspace_root = if let Some(ws) = workspace_path {
            let path = PathBuf::from(ws);
            // Convert relative paths to absolute paths for URI conversion
            if path.is_absolute() {
                path
            } else {
                // For relative paths, resolve them relative to current directory
                std::env::current_dir()
                    .context("Failed to get current directory")
                    .unwrap()
                    .join(&path)
                    .canonicalize()
                    .context(format!(
                        "Failed to resolve workspace path '{}'. Make sure the path exists and is accessible",
                        path.display()
                    ))
                    .unwrap()
            }
        } else {
            std::env::current_dir().unwrap()
        };

        // Verify the absolute path was preserved
        assert!(workspace_root.is_absolute());
        assert!(workspace_root.exists());
        assert_eq!(workspace_root, absolute_path);
    }
}
