use anyhow::{anyhow, Context, Result};
use colored::*;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

use crate::lsp_integration::client::LspClient;
use crate::lsp_integration::types::*;
use crate::lsp_integration::LspSubcommands;
use lsp_daemon::{LogEntry, LogLevel, LspDaemon};

pub struct LspManager;

impl LspManager {
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

        let mut client =
            match tokio::time::timeout(Duration::from_secs(15), LspClient::new(config)).await {
                Ok(Ok(client)) => client,
                Ok(Err(e)) => return Err(anyhow!("Failed to connect to daemon: {}", e)),
                Err(_) => return Err(anyhow!("Timeout connecting to daemon after 15 seconds")),
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
                println!("  {} {}", "Uptime:".bold(), format_duration(status.uptime));
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
                            let uptime =
                                format_duration(std::time::Duration::from_secs(pool.uptime_secs));
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
            // Follow mode - poll for new logs
            println!(
                "{}",
                "Following LSP daemon log (Ctrl+C to stop)..."
                    .green()
                    .bold()
            );
            println!("{}", "─".repeat(60).dimmed());

            // First show the last N lines
            let entries = match client.get_logs(lines).await {
                Ok(entries) => {
                    for entry in &entries {
                        Self::print_log_entry(entry);
                    }
                    entries
                }
                Err(e) => {
                    println!("{} Failed to get logs: {}", "❌".red(), e);
                    return Ok(());
                }
            };

            // Keep track of the last timestamp to avoid duplicates
            let mut last_timestamp = entries.last().map(|e| e.timestamp.clone());

            // Poll for new logs every 500ms
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;

                match client.get_logs(100).await {
                    Ok(new_entries) => {
                        // Show only new entries after the last timestamp
                        let mut found_last = last_timestamp.is_none();
                        for entry in &new_entries {
                            if found_last {
                                Self::print_log_entry(entry);
                                last_timestamp = Some(entry.timestamp.clone());
                            } else if Some(&entry.timestamp) == last_timestamp.as_ref() {
                                found_last = true;
                            }
                        }
                    }
                    Err(_) => {
                        // Daemon might have been shutdown
                        break;
                    }
                }
            }
        } else {
            // Show last N lines
            match client.get_logs(lines).await {
                Ok(entries) => {
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
                Err(e) => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m");
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
