use anyhow::Result;
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
        }
    }

    /// Show daemon status
    async fn show_status(
        use_daemon: bool,
        workspace_hint: Option<String>,
        format: &str,
    ) -> Result<()> {
        // Check if we're being run via cargo and warn about potential conflicts
        if std::env::current_exe()
            .map(|path| path.to_string_lossy().contains("cargo"))
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
            timeout_ms: 240000, // Increased to 4 minutes for full rust-analyzer indexing (90s) + call hierarchy (60s)
        };

        let mut client = LspClient::new(config).await?;
        let status = client.get_status().await?;

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
                                "    {} {}",
                                "Workspaces:".bold(),
                                pool.workspaces.len().to_string().cyan()
                            );
                            for workspace in &pool.workspaces {
                                if let Some(name) = std::path::Path::new(workspace).file_name() {
                                    println!("      • {}", name.to_string_lossy().dimmed());
                                }
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
            timeout_ms: 30000, // Increased for rust-analyzer
        };

        let start_time = std::time::Instant::now();
        let mut client = LspClient::new(config).await?;

        client.ping().await?;
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

        println!("🚀 Starting embedded LSP daemon...");
        println!("   Socket: {socket_path}");
        println!("   Log Level: {log_level}");

        if foreground {
            println!("   Mode: Foreground");
        } else {
            println!("   Mode: Background");
        }

        // Create and start daemon
        let daemon = LspDaemon::new(socket_path)?;

        if foreground {
            println!("✓ LSP daemon started in foreground mode");
            daemon.run().await?;
        } else {
            println!("✓ LSP daemon started in background mode");
            // For background mode, we would typically daemonize the process
            // For now, just run in foreground since we're embedded
            daemon.run().await?;
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
}
