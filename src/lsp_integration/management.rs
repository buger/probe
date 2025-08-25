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

        // Quick health check with timeout
        let ping_result = tokio::time::timeout(Duration::from_secs(5), client.ping()).await;
        if ping_result.is_err() || ping_result.unwrap().is_err() {
            // If ping fails or times out, restart daemon
            let _ = client.shutdown_daemon().await;
            tokio::time::sleep(Duration::from_millis(500)).await;

            Self::auto_start_daemon().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Skip workspace initialization for basic operations to avoid hanging
        // Workspace initialization will happen on-demand when LSP features are actually used
        // This prevents test timeouts and improves startup time for simple search operations

        Ok(())
    }

    /// Auto-start the LSP daemon in background mode
    async fn auto_start_daemon() -> Result<()> {
        // Delegate to the single, lock-protected spawn path in client.rs
        crate::lsp_integration::client::start_embedded_daemon_background().await?;

        // Wait for daemon to be ready (up to 30 seconds)
        let socket_path = lsp_daemon::get_default_socket_path();
        for _ in 0..60 {
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
    #[allow(dead_code)] // Keep for potential future use
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
            LspSubcommands::Call { command } => Self::handle_call_command(command).await,
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
            include_stdlib: false,
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
            include_stdlib: false,
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
            include_stdlib: false,
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
            include_stdlib: false,
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
            include_stdlib: false,
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
        _log_level: String,
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
        let log_level = std::env::var("PROBE_LOG_LEVEL").unwrap_or_default();
        if log_level == "debug" || log_level == "trace" {
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

        // Create and start daemon using async constructor
        let daemon = LspDaemon::new_async(socket_path.clone()).await?;

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
            include_stdlib: false,
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
            CacheSubcommands::Stats {
                detailed: _,
                git: _,
            } => {
                let stats = client.cache_stats().await?;

                match format {
                    "json" => {
                        println!("{}", serde_json::to_string_pretty(&stats)?);
                    }
                    _ => {
                        println!("{}", "LSP Cache Statistics".bold().green());
                        println!("  {} {}", "Total Entries:".bold(), stats.total_entries);
                        println!(
                            "  {} {}",
                            "Total Size:".bold(),
                            format_bytes(stats.total_size_bytes as usize)
                        );
                        println!(
                            "  {} {}",
                            "Disk Size:".bold(),
                            format_bytes(stats.disk_size_bytes as usize)
                        );
                        println!("  {} {:.2}%", "Hit Rate:".bold(), stats.hit_rate * 100.0);
                        println!("  {} {:.2}%", "Miss Rate:".bold(), stats.miss_rate * 100.0);

                        // Memory usage breakdown
                        println!(
                            "  {} {}",
                            "In-Memory Cache:".bold(),
                            format_bytes(stats.memory_usage.in_memory_cache_bytes as usize)
                        );
                        println!(
                            "  {} {}",
                            "Persistent Cache:".bold(),
                            format_bytes(stats.memory_usage.persistent_cache_bytes as usize)
                        );

                        // Show some top files and languages if available
                        if !stats.entries_per_file.is_empty() {
                            println!("\n{}", "Top Files by Entry Count:".bold());
                            let mut file_entries: Vec<_> = stats.entries_per_file.iter().collect();
                            file_entries.sort_by(|a, b| b.1.cmp(a.1));
                            for (file_path, count) in file_entries.iter().take(5) {
                                println!(
                                    "  {} {}: {}",
                                    "•".cyan(),
                                    file_path.file_name().unwrap_or_default().to_string_lossy(),
                                    count.to_string().green()
                                );
                            }
                        }

                        if !stats.entries_per_language.is_empty() {
                            println!("\n{}", "Entries by Language:".bold());
                            for (language, count) in &stats.entries_per_language {
                                println!(
                                    "  {} {}: {}",
                                    "•".cyan(),
                                    language,
                                    count.to_string().green()
                                );
                            }
                        }
                    }
                }
            }
            CacheSubcommands::Clear {
                older_than,
                file,
                commit,
                all,
            } => {
                let result = client
                    .cache_clear(*older_than, file.clone(), commit.clone(), *all)
                    .await?;

                match format {
                    "json" => {
                        let json_output = json!({
                            "entries_removed": result.entries_removed,
                            "duration_ms": result.duration_ms
                        });
                        println!("{}", serde_json::to_string_pretty(&json_output)?);
                    }
                    _ => {
                        if result.entries_removed == 0 {
                            println!("{}", "No cache entries to clear".yellow());
                        } else {
                            println!(
                                "{} {} in {} ms",
                                "Cleared".bold().green(),
                                format!("{} entries", result.entries_removed).cyan(),
                                result.duration_ms
                            );
                        }
                    }
                }
            }
            CacheSubcommands::Export {
                output,
                current_branch,
                compress,
            } => {
                client
                    .cache_export(output.clone(), *current_branch, *compress)
                    .await?;

                match format {
                    "json" => {
                        let json_output = json!({
                            "output_path": output,
                            "current_branch_only": current_branch,
                            "compressed": compress,
                            "success": true
                        });
                        println!("{}", serde_json::to_string_pretty(&json_output)?);
                    }
                    _ => {
                        println!(
                            "{} exported to {}",
                            "Cache".bold().green(),
                            output.display().to_string().cyan()
                        );
                        if *current_branch {
                            println!("  {} Current branch entries only", "•".green());
                        }
                        if *compress {
                            println!("  {} Compressed format", "•".green());
                        }
                    }
                }
            }
            CacheSubcommands::Import { input, merge } => {
                // TODO: Implement cache import
                println!("Cache import not yet implemented");
                println!("Input file: {}", input.display());
                println!("Merge mode: {merge}");
            }
            CacheSubcommands::Compact {
                clean_expired,
                target_size_mb,
            } => {
                // TODO: Implement cache compaction
                println!("Cache compact not yet implemented");
                println!("Clean expired: {clean_expired}");
                if let Some(size) = target_size_mb {
                    println!("Target size: {size} MB");
                }
            }
            CacheSubcommands::List { detailed, format } => {
                Self::handle_workspace_cache_list(&mut client, *detailed, format).await?
            }
            CacheSubcommands::Info { workspace, format } => {
                Self::handle_workspace_cache_info(&mut client, workspace.as_ref(), format).await?
            }
            CacheSubcommands::ClearWorkspace {
                workspace,
                force,
                format,
            } => {
                Self::handle_workspace_cache_clear(&mut client, workspace.as_ref(), *force, format)
                    .await?
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

        // Create indexing config using defaults and override specific fields
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
            ..Default::default()
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
                cache_call_hierarchy,
                cache_definitions,
                cache_references,
                cache_hover,
                cache_document_symbols,
                cache_during_indexing,
                preload_common_symbols,
                max_cache_entries_per_operation,
                lsp_operation_timeout_ms,
                lsp_priority_operations,
                lsp_disabled_operations,
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

                // Update LSP caching configuration fields
                if let Some(cache_ch) = cache_call_hierarchy {
                    config.cache_call_hierarchy = Some(*cache_ch);
                }
                if let Some(cache_def) = cache_definitions {
                    config.cache_definitions = Some(*cache_def);
                }
                if let Some(cache_ref) = cache_references {
                    config.cache_references = Some(*cache_ref);
                }
                if let Some(cache_hover_val) = cache_hover {
                    config.cache_hover = Some(*cache_hover_val);
                }
                if let Some(cache_doc) = cache_document_symbols {
                    config.cache_document_symbols = Some(*cache_doc);
                }
                if let Some(cache_during) = cache_during_indexing {
                    config.cache_during_indexing = Some(*cache_during);
                }
                if let Some(preload) = preload_common_symbols {
                    config.preload_common_symbols = Some(*preload);
                }
                if let Some(max_entries) = max_cache_entries_per_operation {
                    config.max_cache_entries_per_operation = Some(*max_entries);
                }
                if let Some(timeout_ms) = lsp_operation_timeout_ms {
                    config.lsp_operation_timeout_ms = Some(*timeout_ms);
                }
                if let Some(priority_ops) = lsp_priority_operations {
                    config.lsp_priority_operations = priority_ops
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                if let Some(disabled_ops) = lsp_disabled_operations {
                    config.lsp_disabled_operations = disabled_ops
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
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
                    // Use saturating_sub to avoid underflow
                    let elapsed_secs = now.saturating_sub(started_at);
                    let elapsed_friendly =
                        Self::format_duration(std::time::Duration::from_secs(elapsed_secs));
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

                // LSP Caching Configuration section
                println!("\n{}", "LSP Caching Configuration".bold().magenta());

                let cache_ch = config
                    .cache_call_hierarchy
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "Default".to_string());
                println!("  {}: {}", "Cache Call Hierarchy".bold(), cache_ch);

                let cache_def = config
                    .cache_definitions
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "Default".to_string());
                println!("  {}: {}", "Cache Definitions".bold(), cache_def);

                let cache_ref = config
                    .cache_references
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "Default".to_string());
                println!("  {}: {}", "Cache References".bold(), cache_ref);

                let cache_hover = config
                    .cache_hover
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "Default".to_string());
                println!("  {}: {}", "Cache Hover".bold(), cache_hover);

                let cache_doc = config
                    .cache_document_symbols
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "Default".to_string());
                println!("  {}: {}", "Cache Document Symbols".bold(), cache_doc);

                let cache_during = config
                    .cache_during_indexing
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "Default".to_string());
                println!("  {}: {}", "Cache During Indexing".bold(), cache_during);

                let preload = config
                    .preload_common_symbols
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "Default".to_string());
                println!("  {}: {}", "Preload Common Symbols".bold(), preload);

                let max_entries = config
                    .max_cache_entries_per_operation
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "Default".to_string());
                println!(
                    "  {}: {}",
                    "Max Cache Entries Per Operation".bold(),
                    max_entries
                );

                let timeout = config
                    .lsp_operation_timeout_ms
                    .map(|n| format!("{n}ms"))
                    .unwrap_or_else(|| "Default".to_string());
                println!("  {}: {}", "LSP Operation Timeout".bold(), timeout);

                if !config.lsp_priority_operations.is_empty() {
                    println!(
                        "  {}: {}",
                        "Priority Operations".bold(),
                        config.lsp_priority_operations.join(", ")
                    );
                } else {
                    println!("  {}: Default", "Priority Operations".bold());
                }

                if !config.lsp_disabled_operations.is_empty() {
                    println!(
                        "  {}: {}",
                        "Disabled Operations".bold(),
                        config.lsp_disabled_operations.join(", ")
                    );
                } else {
                    println!("  {}: None", "Disabled Operations".bold());
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

    /// Handle workspace cache list command
    async fn handle_workspace_cache_list(
        client: &mut LspClient,
        detailed: bool,
        format: &str,
    ) -> Result<()> {
        let workspaces = client.list_workspace_caches().await?;

        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(&workspaces)?);
            }
            _ => {
                if workspaces.is_empty() {
                    println!("{}", "No workspace caches found".yellow());
                    return Ok(());
                }

                println!("{}", "Workspace Caches".bold().green());
                println!();

                for workspace in &workspaces {
                    println!(
                        "{}",
                        format!("Workspace: {}", workspace.workspace_id).bold()
                    );
                    println!(
                        "  {} {}",
                        "Path:".bold(),
                        workspace.workspace_root.display()
                    );
                    println!(
                        "  {} {}",
                        "Cache Dir:".bold(),
                        workspace.cache_path.display()
                    );
                    println!(
                        "  {} {}",
                        "Size:".bold(),
                        format_bytes(workspace.size_bytes as usize)
                    );
                    println!("  {} {}", "Files:".bold(), workspace.file_count);
                    println!("  {} {}", "Last Accessed:".bold(), workspace.last_accessed);

                    if detailed {
                        println!("  {} {}", "Created:".bold(), workspace.created_at);
                        println!(
                            "  {} {}",
                            "Cache Directory:".bold(),
                            workspace.cache_path.display()
                        );
                    }

                    println!();
                }

                // Summary
                let total_size: u64 = workspaces.iter().map(|w| w.size_bytes).sum();
                let total_entries: u64 = workspaces.iter().map(|w| w.file_count as u64).sum();
                let open_count = workspaces.len(); // All listed workspaces are accessible

                println!("{}", "Summary".bold().cyan());
                println!("  {} {}", "Total Workspaces:".bold(), workspaces.len());
                println!("  {} {}", "Open in Memory:".bold(), open_count);
                println!(
                    "  {} {}",
                    "Total Size:".bold(),
                    format_bytes(total_size as usize)
                );
                println!("  {} {}", "Total Entries:".bold(), total_entries);
            }
        }

        Ok(())
    }

    /// Handle workspace cache info command
    async fn handle_workspace_cache_info(
        client: &mut LspClient,
        workspace_path: Option<&std::path::PathBuf>,
        format: &str,
    ) -> Result<()> {
        let info = client
            .get_workspace_cache_info(workspace_path.cloned())
            .await?;

        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(&info)?);
            }
            _ => {
                match info {
                    Some(workspaces_info) => {
                        if workspaces_info.is_empty() {
                            println!("{}", "No workspace cache information available".yellow());
                            return Ok(());
                        }

                        for workspace_info in &workspaces_info {
                            println!(
                                "{}",
                                format!(
                                    "Workspace Cache Information: {}",
                                    workspace_info.workspace_id
                                )
                                .bold()
                                .green()
                            );
                            println!();

                            // Basic information
                            println!("{}", "Basic Information".bold().cyan());
                            println!(
                                "  {} {}",
                                "Workspace ID:".bold(),
                                workspace_info.workspace_id
                            );
                            println!(
                                "  {} {}",
                                "Workspace Root:".bold(),
                                workspace_info.workspace_root.display()
                            );
                            println!(
                                "  {} {}",
                                "Cache Directory:".bold(),
                                workspace_info.cache_path.display()
                            );
                            println!();

                            // Access statistics
                            println!("{}", "Access Statistics".bold().cyan());
                            println!(
                                "  {} {}",
                                "Last Accessed:".bold(),
                                workspace_info.last_accessed
                            );
                            println!("  {} {}", "Created At:".bold(), workspace_info.created_at);
                            println!();

                            // Cache content statistics
                            println!("{}", "Cache Content".bold().cyan());
                            println!("  {} {}", "Entry Count:".bold(), workspace_info.file_count);
                            println!(
                                "  {} {}",
                                "Size in Memory:".bold(),
                                format_bytes(workspace_info.size_bytes as usize)
                            );
                            println!(
                                "  {} {}",
                                "Size on Disk:".bold(),
                                format_bytes(workspace_info.disk_size_bytes as usize)
                            );
                            println!(
                                "  {} {}",
                                "Files Indexed:".bold(),
                                workspace_info.files_indexed
                            );

                            if !workspace_info.languages.is_empty() {
                                println!(
                                    "  {} {}",
                                    "Languages:".bold(),
                                    workspace_info.languages.join(", ")
                                );
                            }
                            println!();

                            // Performance statistics
                            if let Some(ref cache_stats) = workspace_info.cache_stats {
                                println!("{}", "Performance".bold().cyan());
                                println!(
                                    "  {} {:.1}%",
                                    "Hit Rate:".bold(),
                                    cache_stats.hit_rate * 100.0
                                );
                                println!(
                                    "  {} {:.1}%",
                                    "Miss Rate:".bold(),
                                    cache_stats.miss_rate * 100.0
                                );
                                println!();
                            }

                            // Router statistics
                            if let Some(ref router_stats) = workspace_info.router_stats {
                                println!("{}", "Router Stats".bold().cyan());
                                println!(
                                    "  {} {}",
                                    "Access Count:".bold(),
                                    router_stats.access_count
                                );
                                println!(
                                    "  {} {} / {}",
                                    "Open Caches:".bold(),
                                    router_stats.current_open_caches,
                                    router_stats.max_open_caches
                                );
                            }

                            if workspaces_info.len() > 1 {
                                println!();
                                println!("{}", "─".repeat(60).dimmed());
                                println!();
                            }
                        }
                    }
                    None => {
                        if workspace_path.is_some() {
                            println!(
                                "{}",
                                "No cache information found for the specified workspace".yellow()
                            );
                        } else {
                            println!("{}", "No workspace cache information available".yellow());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle workspace cache clear command
    async fn handle_workspace_cache_clear(
        client: &mut LspClient,
        workspace_path: Option<&std::path::PathBuf>,
        force: bool,
        format: &str,
    ) -> Result<()> {
        // Confirmation prompt for destructive operations
        if !force && std::env::var("PROBE_BATCH").unwrap_or_default() != "1" {
            use std::io::{self, Write};

            if workspace_path.is_some() {
                print!("Are you sure you want to clear the workspace cache? [y/N]: ");
            } else {
                print!("Are you sure you want to clear ALL workspace caches? [y/N]: ");
            }
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
                println!("{}", "Operation cancelled".yellow());
                return Ok(());
            }
        }

        let result = client
            .clear_workspace_cache(workspace_path.cloned())
            .await?;

        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            _ => {
                if result.cleared_workspaces.is_empty() {
                    println!("{}", "No workspace caches to clear".yellow());
                    return Ok(());
                }

                println!("{}", "Cache Clear Results".bold().green());
                println!();

                for workspace_result in &result.cleared_workspaces {
                    if workspace_result.success {
                        println!("{} {}", "✓".green(), workspace_result.workspace_id.bold());
                        println!(
                            "  {} {}",
                            "Path:".bold(),
                            workspace_result.workspace_root.display()
                        );
                        println!(
                            "  {} {}",
                            "Files Removed:".bold(),
                            workspace_result.files_removed
                        );
                        println!(
                            "  {} {}",
                            "Space Reclaimed:".bold(),
                            format_bytes(workspace_result.size_freed_bytes as usize)
                        );
                    } else {
                        println!("{} {}", "✗".red(), workspace_result.workspace_id.bold());
                        println!(
                            "  {} {}",
                            "Path:".bold(),
                            workspace_result.workspace_root.display()
                        );
                        if let Some(ref error) = workspace_result.error {
                            println!("  {} {}", "Error:".bold().red(), error);
                        }
                    }
                    println!();
                }

                // Summary
                println!("{}", "Summary".bold().cyan());
                println!(
                    "  {} {}",
                    "Workspaces Cleared:".bold(),
                    result.cleared_workspaces.len()
                );
                println!(
                    "  {} {}",
                    "Total Files Removed:".bold(),
                    result.total_files_removed
                );
                println!(
                    "  {} {}",
                    "Total Space Reclaimed:".bold(),
                    format_bytes(result.total_size_freed_bytes as usize)
                );
                if !result.errors.is_empty() {
                    println!("  {} {}", "Errors:".bold().red(), result.errors.len());
                }
            }
        }

        Ok(())
    }

    /// Handle LSP call commands
    async fn handle_call_command(command: &crate::lsp_integration::LspCallCommands) -> Result<()> {
        use crate::lsp_integration::LspCallCommands;

        // Ensure daemon is ready
        Self::ensure_ready().await?;

        // Create client
        let config = LspConfig::default();
        let mut client = LspClient::new(config).await?;

        match command {
            LspCallCommands::Definition { location, format } => {
                let resolved = crate::lsp_integration::symbol_resolver::resolve_location(location)?;
                let results = client
                    .call_definition(&resolved.file_path, resolved.line, resolved.column)
                    .await?;
                Self::display_locations(&results, "Definition", format).await
            }
            LspCallCommands::References {
                location,
                include_declaration,
                format,
            } => {
                let resolved = crate::lsp_integration::symbol_resolver::resolve_location(location)?;
                let results = client
                    .call_references(
                        &resolved.file_path,
                        resolved.line,
                        resolved.column,
                        *include_declaration,
                    )
                    .await?;
                Self::display_locations(&results, "References", format).await
            }
            LspCallCommands::Hover { location, format } => {
                let resolved = crate::lsp_integration::symbol_resolver::resolve_location(location)?;
                let result = client
                    .call_hover(&resolved.file_path, resolved.line, resolved.column)
                    .await?;
                Self::display_hover_info(&result, format).await
            }
            LspCallCommands::DocumentSymbols { file, format } => {
                let results = client.call_document_symbols(file).await?;
                Self::display_document_symbols(&results, format).await
            }
            LspCallCommands::WorkspaceSymbols {
                query,
                max_results,
                format,
            } => {
                let results = client.call_workspace_symbols(query, *max_results).await?;
                Self::display_workspace_symbols(&results, format).await
            }
            LspCallCommands::CallHierarchy { location, format } => {
                let resolved = crate::lsp_integration::symbol_resolver::resolve_location(location)?;
                let result = client
                    .get_call_hierarchy(&resolved.file_path, resolved.line, resolved.column)
                    .await?;
                Self::display_call_hierarchy(&result, format).await
            }
            LspCallCommands::Implementations { location, format } => {
                let resolved = crate::lsp_integration::symbol_resolver::resolve_location(location)?;
                let results = client
                    .call_implementations(&resolved.file_path, resolved.line, resolved.column)
                    .await?;
                Self::display_locations(&results, "Implementations", format).await
            }
            LspCallCommands::TypeDefinition { location, format } => {
                let resolved = crate::lsp_integration::symbol_resolver::resolve_location(location)?;
                let results = client
                    .call_type_definition(&resolved.file_path, resolved.line, resolved.column)
                    .await?;
                Self::display_locations(&results, "Type Definition", format).await
            }
        }
    }

    /// Display location results (for definition, references, implementations, type definition)
    async fn display_locations(
        locations: &[lsp_daemon::protocol::Location],
        command_name: &str,
        format: &str,
    ) -> Result<()> {
        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(locations)?);
            }
            "plain" => {
                for location in locations {
                    println!(
                        "{}:{}:{}",
                        location.uri,
                        location.range.start.line + 1,
                        location.range.start.character + 1
                    );
                }
            }
            _ => {
                // Terminal format
                if locations.is_empty() {
                    println!(
                        "{}",
                        format!("No {} found", command_name.to_lowercase()).yellow()
                    );
                    return Ok(());
                }

                println!("{}", format!("{command_name} Results:").bold().green());
                println!();

                for (i, location) in locations.iter().enumerate() {
                    let file_path = location
                        .uri
                        .strip_prefix("file://")
                        .unwrap_or(&location.uri);

                    println!(
                        "{} {}:{}:{}",
                        format!("{}.", i + 1).dimmed(),
                        file_path.cyan(),
                        (location.range.start.line + 1).to_string().yellow(),
                        (location.range.start.character + 1).to_string().yellow()
                    );

                    // Show a snippet of code if possible
                    if let Ok(content) = std::fs::read_to_string(file_path) {
                        let lines: Vec<&str> = content.lines().collect();
                        let line_idx = location.range.start.line as usize;
                        if line_idx < lines.len() {
                            let line = lines[line_idx].trim();
                            if !line.is_empty() {
                                println!("   {}", line.dimmed());
                            }
                        }
                    }
                    println!();
                }

                let count = locations.len();
                println!(
                    "{} {}",
                    format!(
                        "Found {} {}",
                        count,
                        if count == 1 { "result" } else { "results" }
                    )
                    .green(),
                    format!("for {}", command_name.to_lowercase()).dimmed()
                );
            }
        }
        Ok(())
    }

    /// Display hover information
    async fn display_hover_info(
        hover: &Option<lsp_daemon::protocol::HoverContent>,
        format: &str,
    ) -> Result<()> {
        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(hover)?);
            }
            "plain" => {
                if let Some(hover_content) = hover {
                    println!("{}", hover_content.contents);
                } else {
                    println!("No hover information available");
                }
            }
            _ => {
                // Terminal format
                match hover {
                    Some(hover_content) => {
                        println!("{}", "Hover Information:".bold().green());
                        println!();

                        // Format the hover contents nicely
                        let contents = &hover_content.contents;
                        if contents.starts_with("```") && contents.ends_with("```") {
                            // It's a markdown code block, display with syntax highlighting
                            let lines: Vec<&str> = contents.lines().collect();
                            if lines.len() > 2 {
                                println!("{}", lines[0].dimmed()); // Language marker
                                for line in &lines[1..lines.len() - 1] {
                                    println!("  {line}");
                                }
                                println!("{}", lines.last().unwrap().dimmed()); // Closing ```
                            } else {
                                println!("{contents}");
                            }
                        } else {
                            // Regular text, just display it
                            for line in contents.lines() {
                                if line.trim().is_empty() {
                                    println!();
                                } else {
                                    println!("  {line}");
                                }
                            }
                        }

                        if let Some(ref range) = hover_content.range {
                            println!();
                            println!(
                                "{} {}:{}-{}:{}",
                                "Range:".bold(),
                                (range.start.line + 1).to_string().yellow(),
                                (range.start.character + 1).to_string().yellow(),
                                (range.end.line + 1).to_string().yellow(),
                                (range.end.character + 1).to_string().yellow()
                            );
                        }
                    }
                    None => {
                        println!("{}", "No hover information available".yellow());
                    }
                }
            }
        }
        Ok(())
    }

    /// Display document symbols
    async fn display_document_symbols(
        symbols: &[lsp_daemon::protocol::DocumentSymbol],
        format: &str,
    ) -> Result<()> {
        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(symbols)?);
            }
            "plain" => {
                Self::print_document_symbols_plain(symbols, 0);
            }
            _ => {
                // Terminal format
                if symbols.is_empty() {
                    println!("{}", "No symbols found in document".yellow());
                    return Ok(());
                }

                println!("{}", "Document Symbols:".bold().green());
                println!();

                Self::print_document_symbols_tree(symbols, 0);

                let total_count = Self::count_document_symbols(symbols);
                println!();
                println!(
                    "{} {}",
                    format!(
                        "Found {} {}",
                        total_count,
                        if total_count == 1 {
                            "symbol"
                        } else {
                            "symbols"
                        }
                    )
                    .green(),
                    "in document".dimmed()
                );
            }
        }
        Ok(())
    }

    /// Print document symbols in plain format (one per line)
    fn print_document_symbols_plain(
        symbols: &[lsp_daemon::protocol::DocumentSymbol],
        _depth: usize,
    ) {
        for symbol in symbols {
            println!(
                "{}:{}:{} {} {}",
                symbol.range.start.line + 1,
                symbol.range.start.character + 1,
                symbol.range.end.line + 1,
                Self::format_symbol_kind(&symbol.kind),
                symbol.name
            );

            if let Some(ref children) = symbol.children {
                Self::print_document_symbols_plain(children, _depth + 1);
            }
        }
    }

    /// Print document symbols in tree format
    fn print_document_symbols_tree(symbols: &[lsp_daemon::protocol::DocumentSymbol], depth: usize) {
        for (i, symbol) in symbols.iter().enumerate() {
            let is_last = i == symbols.len() - 1;
            let prefix = if depth == 0 {
                "".to_string()
            } else {
                let mut p = "  ".repeat(depth - 1);
                p.push_str(if is_last { "└─ " } else { "├─ " });
                p
            };

            let kind_str = Self::format_symbol_kind(&symbol.kind);
            let location_str = format!(
                "{}:{}",
                symbol.range.start.line + 1,
                symbol.range.start.character + 1
            )
            .dimmed();

            println!(
                "{}{} {} {}",
                prefix,
                kind_str.bold(),
                symbol.name.cyan(),
                location_str
            );

            if let Some(ref detail) = symbol.detail {
                if !detail.is_empty() {
                    let detail_prefix = if depth == 0 {
                        "  ".to_string()
                    } else {
                        let mut p = "  ".repeat(depth);
                        p.push_str("   ");
                        p
                    };
                    println!("{}{}", detail_prefix, detail.dimmed());
                }
            }

            if let Some(ref children) = symbol.children {
                Self::print_document_symbols_tree(children, depth + 1);
            }
        }
    }

    /// Count total symbols including children
    fn count_document_symbols(symbols: &[lsp_daemon::protocol::DocumentSymbol]) -> usize {
        let mut count = symbols.len();
        for symbol in symbols {
            if let Some(ref children) = symbol.children {
                count += Self::count_document_symbols(children);
            }
        }
        count
    }

    /// Display workspace symbols
    async fn display_workspace_symbols(
        symbols: &[lsp_daemon::protocol::SymbolInformation],
        format: &str,
    ) -> Result<()> {
        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(symbols)?);
            }
            "plain" => {
                for symbol in symbols {
                    let file_path = symbol
                        .location
                        .uri
                        .strip_prefix("file://")
                        .unwrap_or(&symbol.location.uri);
                    println!(
                        "{}:{}:{} {} {}",
                        file_path,
                        symbol.location.range.start.line + 1,
                        symbol.location.range.start.character + 1,
                        Self::format_symbol_kind(&symbol.kind),
                        symbol.name
                    );
                }
            }
            _ => {
                // Terminal format
                if symbols.is_empty() {
                    println!("{}", "No workspace symbols found".yellow());
                    return Ok(());
                }

                println!("{}", "Workspace Symbols:".bold().green());
                println!();

                // Group symbols by file for better readability
                use std::collections::HashMap;
                let mut symbols_by_file: HashMap<
                    String,
                    Vec<&lsp_daemon::protocol::SymbolInformation>,
                > = HashMap::new();

                for symbol in symbols {
                    let file_path = symbol
                        .location
                        .uri
                        .strip_prefix("file://")
                        .unwrap_or(&symbol.location.uri)
                        .to_string();
                    symbols_by_file.entry(file_path).or_default().push(symbol);
                }

                let mut files: Vec<_> = symbols_by_file.keys().collect();
                files.sort();

                for file_path in files {
                    if let Some(file_symbols) = symbols_by_file.get(file_path) {
                        println!("{}", file_path.bold());

                        for symbol in file_symbols {
                            let kind_str = Self::format_symbol_kind(&symbol.kind);
                            let location_str = format!(
                                "{}:{}",
                                symbol.location.range.start.line + 1,
                                symbol.location.range.start.character + 1
                            )
                            .dimmed();

                            print!(
                                "  {} {} {}",
                                kind_str.bold(),
                                symbol.name.cyan(),
                                location_str
                            );

                            if let Some(ref container) = symbol.container_name {
                                print!(" {}", format!("in {container}").dimmed());
                            }

                            println!();
                        }
                        println!();
                    }
                }

                let count = symbols.len();
                println!(
                    "{} {}",
                    format!(
                        "Found {} {}",
                        count,
                        if count == 1 { "symbol" } else { "symbols" }
                    )
                    .green(),
                    "in workspace".dimmed()
                );
            }
        }
        Ok(())
    }

    /// Display call hierarchy information
    async fn display_call_hierarchy(
        hierarchy: &crate::lsp_integration::types::CallHierarchyInfo,
        format: &str,
    ) -> Result<()> {
        match format {
            "json" => {
                println!("{}", serde_json::to_string_pretty(hierarchy)?);
            }
            "plain" => {
                // Plain format: just list all the calls
                for caller in &hierarchy.incoming_calls {
                    println!("INCOMING: {}", caller.name);
                }
                for callee in &hierarchy.outgoing_calls {
                    println!("OUTGOING: {}", callee.name);
                }
            }
            _ => {
                // Terminal format
                println!("{}", "Call Hierarchy:".bold().green());
                println!();

                if !hierarchy.incoming_calls.is_empty() {
                    println!(
                        "{} ({} calls)",
                        "Incoming Calls:".bold().blue(),
                        hierarchy.incoming_calls.len()
                    );
                    for (i, call) in hierarchy.incoming_calls.iter().enumerate() {
                        println!(
                            "  {}. {} {} {}",
                            (i + 1).to_string().dimmed(),
                            call.symbol_kind.dimmed(),
                            call.name.cyan(),
                            format!("({}:{})", call.line + 1, call.column + 1).dimmed()
                        );
                        println!("     {}", call.file_path.dimmed());
                    }
                    println!();
                }

                if !hierarchy.outgoing_calls.is_empty() {
                    println!(
                        "{} ({} calls)",
                        "Outgoing Calls:".bold().yellow(),
                        hierarchy.outgoing_calls.len()
                    );
                    for (i, call) in hierarchy.outgoing_calls.iter().enumerate() {
                        println!(
                            "  {}. {} {} {}",
                            (i + 1).to_string().dimmed(),
                            call.symbol_kind.dimmed(),
                            call.name.cyan(),
                            format!("({}:{})", call.line + 1, call.column + 1).dimmed()
                        );
                        println!("     {}", call.file_path.dimmed());
                    }
                    println!();
                }

                if hierarchy.incoming_calls.is_empty() && hierarchy.outgoing_calls.is_empty() {
                    println!("{}", "No call hierarchy information found".yellow());
                }
            }
        }
        Ok(())
    }

    /// Format symbol kind as a readable string with icon
    fn format_symbol_kind(kind: &lsp_daemon::protocol::SymbolKind) -> String {
        use lsp_daemon::protocol::SymbolKind;

        match kind {
            SymbolKind::File => "📄 File".to_string(),
            SymbolKind::Module => "📦 Module".to_string(),
            SymbolKind::Namespace => "🏠 Namespace".to_string(),
            SymbolKind::Package => "📦 Package".to_string(),
            SymbolKind::Class => "🏛️  Class".to_string(),
            SymbolKind::Method => "⚡ Method".to_string(),
            SymbolKind::Property => "🔧 Property".to_string(),
            SymbolKind::Field => "🔹 Field".to_string(),
            SymbolKind::Constructor => "🏗️  Constructor".to_string(),
            SymbolKind::Enum => "🔢 Enum".to_string(),
            SymbolKind::Interface => "🔌 Interface".to_string(),
            SymbolKind::Function => "⚙️  Function".to_string(),
            SymbolKind::Variable => "📋 Variable".to_string(),
            SymbolKind::Constant => "🔒 Constant".to_string(),
            SymbolKind::String => "📝 String".to_string(),
            SymbolKind::Number => "🔢 Number".to_string(),
            SymbolKind::Boolean => "✅ Boolean".to_string(),
            SymbolKind::Array => "📚 Array".to_string(),
            SymbolKind::Object => "🧩 Object".to_string(),
            SymbolKind::Key => "🔑 Key".to_string(),
            SymbolKind::Null => "⭕ Null".to_string(),
            SymbolKind::EnumMember => "🔸 EnumMember".to_string(),
            SymbolKind::Struct => "🏗️  Struct".to_string(),
            SymbolKind::Event => "⚡ Event".to_string(),
            SymbolKind::Operator => "➕ Operator".to_string(),
            SymbolKind::TypeParameter => "🏷️  TypeParameter".to_string(),
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
    #[cfg(not(target_os = "windows"))] // Skip on Windows due to path resolution issues in CI
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
        // workspace_root is already canonicalized from line 2949
        assert_eq!(workspace_root, expected_canonical);
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
