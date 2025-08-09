use anyhow::{anyhow, Result};
use lsp_daemon::{
    get_default_socket_path, remove_socket_file, CallHierarchyResult, DaemonRequest,
    DaemonResponse, DaemonStatus, IpcStream, Language, LanguageDetector, LanguageInfo, LogEntry,
    MessageCodec,
};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::lsp_integration::types::*;

pub struct LspClient {
    stream: Option<IpcStream>,
    config: LspConfig,
}

impl LspClient {
    /// Create a new LSP client with the given configuration
    pub async fn new(config: LspConfig) -> Result<Self> {
        let use_daemon = config.use_daemon;
        let mut client = Self {
            stream: None,
            config,
        };

        if use_daemon {
            client.connect().await?;
        }

        Ok(client)
    }

    /// Connect to the LSP daemon, auto-starting if necessary
    async fn connect(&mut self) -> Result<()> {
        let socket_path = get_default_socket_path();
        let connection_timeout = Duration::from_millis(self.config.timeout_ms / 3); // Use 1/3 of total timeout for connection

        // Try to connect to existing daemon and check version compatibility
        match timeout(connection_timeout, IpcStream::connect(&socket_path)).await {
            Ok(Ok(stream)) => {
                // Check version compatibility first
                if check_daemon_version_compatibility().await.unwrap_or(false) {
                    info!("Connected to existing LSP daemon with compatible version");
                    self.stream = Some(stream);

                    // Send connect message with timeout
                    let request = DaemonRequest::Connect {
                        client_id: Uuid::new_v4(),
                    };

                    match timeout(connection_timeout, self.send_request(request)).await {
                        Ok(Ok(response)) => {
                            if let DaemonResponse::Connected { daemon_version, .. } = response {
                                debug!("Connected to daemon version: {}", daemon_version);
                            }
                            return Ok(());
                        }
                        Ok(Err(e)) => {
                            warn!("Failed to send connect message: {}", e);
                            self.stream = None;
                        }
                        Err(_) => {
                            warn!("Connect message timed out");
                            self.stream = None;
                        }
                    }
                } else {
                    info!("Daemon version mismatch detected, will restart daemon...");
                    // Close this connection, daemon will be restarted below
                }
            }
            Ok(Err(e)) => {
                debug!("Failed to connect to daemon: {}", e);
            }
            Err(_) => {
                debug!("Connection attempt timed out");
            }
        }

        // Auto-start daemon
        info!("Starting embedded LSP daemon...");
        match timeout(Duration::from_secs(10), start_embedded_daemon_background()).await {
            Ok(Ok(_)) => {
                // Successfully started
            }
            Ok(Err(e)) => {
                return Err(anyhow!("Failed to start LSP daemon: {}", e));
            }
            Err(_) => {
                return Err(anyhow!("Timeout starting LSP daemon"));
            }
        }

        // Wait for daemon to be ready with exponential backoff
        for attempt in 0..10 {
            sleep(Duration::from_millis(100 * 2_u64.pow(attempt))).await;

            match timeout(connection_timeout, IpcStream::connect(&socket_path)).await {
                Ok(Ok(stream)) => {
                    info!("Connected to newly started LSP daemon");
                    self.stream = Some(stream);

                    // Send connect message with timeout
                    let request = DaemonRequest::Connect {
                        client_id: Uuid::new_v4(),
                    };

                    match timeout(connection_timeout, self.send_request(request)).await {
                        Ok(Ok(response)) => {
                            if let DaemonResponse::Connected { daemon_version, .. } = response {
                                debug!("Connected to daemon version: {}", daemon_version);
                            }
                            return Ok(());
                        }
                        Ok(Err(e)) => {
                            warn!("Failed to send connect message to new daemon: {}", e);
                            continue;
                        }
                        Err(_) => {
                            warn!("Connect message to new daemon timed out");
                            continue;
                        }
                    }
                }
                Ok(Err(_)) => {
                    debug!("Connection attempt {} failed", attempt + 1);
                }
                Err(_) => {
                    debug!("Connection attempt {} timed out", attempt + 1);
                }
            }
        }

        Err(anyhow!(
            "Failed to connect to daemon after starting (all attempts timed out)"
        ))
    }

    /// Send a request to the daemon and wait for response
    async fn send_request(&mut self, request: DaemonRequest) -> Result<DaemonResponse> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow!("Not connected to daemon"))?;

        // Encode and send request
        let encoded = MessageCodec::encode(&request)?;
        stream.write_all(&encoded).await?;
        stream.flush().await?;

        // Read response with timeout using proper message framing
        let timeout_duration = Duration::from_millis(self.config.timeout_ms);

        // Read message length (4 bytes)
        let mut length_buf = [0u8; 4];
        timeout(timeout_duration, stream.read_exact(&mut length_buf)).await??;
        let message_len = u32::from_be_bytes(length_buf) as usize;

        // Ensure we don't try to read unreasonably large messages (10MB limit)
        if message_len > 10 * 1024 * 1024 {
            return Err(anyhow!("Message too large: {} bytes", message_len));
        }

        // Read the complete message body
        let mut message_buf = vec![0u8; message_len];
        timeout(timeout_duration, stream.read_exact(&mut message_buf)).await??;

        // Reconstruct the complete message with length prefix for decoding
        let mut complete_message = Vec::with_capacity(4 + message_len);
        complete_message.extend_from_slice(&length_buf);
        complete_message.extend_from_slice(&message_buf);

        // Decode response
        let response = MessageCodec::decode_response(&complete_message)?;

        // Check for errors
        if let DaemonResponse::Error { error, .. } = &response {
            return Err(anyhow!("Daemon error: {}", error));
        }

        Ok(response)
    }

    /// Get enhanced symbol information including call hierarchy and references
    pub async fn get_symbol_info(
        &mut self,
        file_path: &Path,
        symbol_name: &str,
        line: u32,
        column: u32,
    ) -> Result<Option<EnhancedSymbolInfo>> {
        if !self.config.use_daemon || self.stream.is_none() {
            return Ok(None);
        }

        // Get call hierarchy information
        let call_hierarchy = match self.get_call_hierarchy(file_path, line, column).await {
            Ok(hierarchy) => Some(hierarchy),
            Err(e) => {
                warn!("Failed to get call hierarchy: {}", e);
                None
            }
        };

        // For now, we focus on call hierarchy. References and other info can be added later
        Ok(Some(EnhancedSymbolInfo {
            name: symbol_name.to_string(),
            file_path: file_path.to_string_lossy().to_string(),
            line,
            column,
            symbol_kind: "unknown".to_string(), // Will be determined by tree-sitter
            call_hierarchy,
            references: Vec::new(), // TODO: implement references
            documentation: None,    // TODO: implement hover info
            type_info: None,        // TODO: implement type info
        }))
    }

    /// Get call hierarchy for a symbol
    async fn get_call_hierarchy(
        &mut self,
        file_path: &Path,
        line: u32,
        column: u32,
    ) -> Result<CallHierarchyInfo> {
        let request = DaemonRequest::CallHierarchy {
            request_id: Uuid::new_v4(),
            file_path: file_path.to_path_buf(),
            line,
            column,
            workspace_hint: self
                .config
                .workspace_hint
                .as_ref()
                .map(std::path::PathBuf::from),
        };

        // Add timeout for call hierarchy request - this can be slow due to rust-analyzer
        let call_timeout = Duration::from_millis(self.config.timeout_ms);
        let response = timeout(call_timeout, self.send_request(request))
            .await
            .map_err(|_| {
                anyhow!(
                    "Call hierarchy request timed out after {}ms",
                    self.config.timeout_ms
                )
            })??;

        match response {
            DaemonResponse::CallHierarchy { result, .. } => {
                let converted = convert_call_hierarchy_result(result);
                Ok(converted)
            }
            DaemonResponse::Error { error, .. } => Err(anyhow!("Call hierarchy failed: {}", error)),
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

    /// Get daemon status
    pub async fn get_status(&mut self) -> Result<LspDaemonStatus> {
        let request = DaemonRequest::Status {
            request_id: Uuid::new_v4(),
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::Status { status, .. } => Ok(convert_daemon_status(status)),
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

    /// List available language servers
    pub async fn list_languages(&mut self) -> Result<Vec<LanguageInfo>> {
        let request = DaemonRequest::ListLanguages {
            request_id: Uuid::new_v4(),
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::LanguageList { languages, .. } => Ok(languages),
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

    /// Get log entries from the daemon
    pub async fn get_logs(&mut self, lines: usize) -> Result<Vec<LogEntry>> {
        let request = DaemonRequest::GetLogs {
            request_id: Uuid::new_v4(),
            lines,
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::Logs { entries, .. } => Ok(entries),
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

    /// Shutdown the daemon
    pub async fn shutdown_daemon(&mut self) -> Result<()> {
        let request = DaemonRequest::Shutdown {
            request_id: Uuid::new_v4(),
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::Shutdown { .. } => {
                info!("LSP daemon shutdown acknowledged");
                self.stream = None;
                Ok(())
            }
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

    /// Ping the daemon for health check
    pub async fn ping(&mut self) -> Result<()> {
        let request = DaemonRequest::Ping {
            request_id: Uuid::new_v4(),
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::Pong { .. } => Ok(()),
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

    /// Check if LSP is available for the given file
    pub fn is_supported(&self, file_path: &Path) -> bool {
        let detector = LanguageDetector::new();
        if let Ok(language) = detector.detect(file_path) {
            language != Language::Unknown
        } else {
            false
        }
    }
}

/// Get current probe binary version info
fn get_probe_version_info() -> (String, String, String) {
    (
        env!("CARGO_PKG_VERSION").to_string(),
        env!("GIT_HASH").to_string(),
        env!("BUILD_DATE").to_string(),
    )
}

/// Check if daemon version matches probe binary version
async fn check_daemon_version_compatibility() -> Result<bool> {
    let socket_path = get_default_socket_path();

    // Try to connect to existing daemon
    match IpcStream::connect(&socket_path).await {
        Ok(mut stream) => {
            // Send status request to get daemon version
            let request = DaemonRequest::Status {
                request_id: Uuid::new_v4(),
            };

            let encoded = MessageCodec::encode(&request)?;
            stream.write_all(&encoded).await?;

            // Read response
            let mut length_buf = [0u8; 4];
            stream.read_exact(&mut length_buf).await?;
            let length = u32::from_be_bytes(length_buf) as usize;

            let mut response_buf = vec![0u8; length];
            stream.read_exact(&mut response_buf).await?;

            let response =
                MessageCodec::decode_response(&[&length_buf[..], &response_buf[..]].concat())?;

            if let DaemonResponse::Status { status, .. } = response {
                let (probe_version, probe_git_hash, probe_build_date) = get_probe_version_info();

                debug!(
                    "Probe version: {}, git: {}, build: {}",
                    probe_version, probe_git_hash, probe_build_date
                );
                debug!(
                    "Daemon version: {}, git: {}, build: {}",
                    status.version, status.git_hash, status.build_date
                );

                // Check if versions match
                let version_matches = !status.version.is_empty()
                    && !status.git_hash.is_empty()
                    && status.git_hash == probe_git_hash;

                if !version_matches {
                    info!(
                        "Version mismatch detected - Probe: {} ({}), Daemon: {} ({})",
                        probe_version, probe_git_hash, status.version, status.git_hash
                    );
                }

                Ok(version_matches)
            } else {
                // If we can't get status, assume incompatible
                Ok(false)
            }
        }
        Err(_) => {
            // No daemon running, no version conflict
            Ok(true)
        }
    }
}

/// Shutdown existing daemon gracefully
async fn shutdown_existing_daemon() -> Result<()> {
    let socket_path = get_default_socket_path();

    match IpcStream::connect(&socket_path).await {
        Ok(mut stream) => {
            // Send shutdown request
            let request = DaemonRequest::Shutdown {
                request_id: Uuid::new_v4(),
            };

            let encoded = MessageCodec::encode(&request)?;
            stream.write_all(&encoded).await?;

            info!("Sent shutdown request to existing daemon");

            // Give daemon time to shutdown
            sleep(Duration::from_millis(500)).await;
            Ok(())
        }
        Err(_) => {
            // No daemon running
            Ok(())
        }
    }
}

/// Start embedded LSP daemon in the background using probe binary
async fn start_embedded_daemon_background() -> Result<()> {
    let socket_path = get_default_socket_path();

    // Check version compatibility if daemon is running
    if IpcStream::connect(&socket_path).await.is_ok() {
        if check_daemon_version_compatibility().await.unwrap_or(false) {
            debug!("Daemon already running with compatible version");
            return Ok(());
        } else {
            info!("Daemon version mismatch detected, restarting daemon...");
            shutdown_existing_daemon().await?;
        }
    }

    // Clean up any stale socket
    remove_socket_file(&socket_path)?;

    // Get current executable path (probe binary)
    let probe_binary = std::env::current_exe()
        .map_err(|e| anyhow!("Failed to get current executable path: {}", e))?;

    debug!(
        "Starting embedded daemon using probe binary: {:?}",
        probe_binary
    );

    // Start daemon using "probe lsp start" command
    // Environment variables are inherited by default
    std::process::Command::new(&probe_binary)
        .args(["lsp", "start"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn embedded daemon: {}", e))?;

    info!("Started embedded daemon in background");
    Ok(())
}

/// Convert lsp-daemon CallHierarchyResult to our CallHierarchyInfo
fn convert_call_hierarchy_result(result: CallHierarchyResult) -> CallHierarchyInfo {
    let incoming_calls = result
        .incoming
        .into_iter()
        .map(|call| CallInfo {
            name: call.from.name,
            file_path: call.from.uri,
            line: call.from.range.start.line,
            column: call.from.range.start.character,
            symbol_kind: call.from.kind,
        })
        .collect();

    let outgoing_calls = result
        .outgoing
        .into_iter()
        .map(|call| CallInfo {
            name: call.from.name,
            file_path: call.from.uri,
            line: call.from.range.start.line,
            column: call.from.range.start.character,
            symbol_kind: call.from.kind,
        })
        .collect();

    CallHierarchyInfo {
        incoming_calls,
        outgoing_calls,
    }
}

/// Convert lsp-daemon DaemonStatus to our LspDaemonStatus
fn convert_daemon_status(status: DaemonStatus) -> LspDaemonStatus {
    use std::collections::HashMap;

    let language_pools = status
        .pools
        .into_iter()
        .map(|pool| {
            let pool_status = LanguagePoolStatus {
                language: format!("{:?}", pool.language), // Convert Language enum to string
                ready_servers: pool.ready_servers,
                busy_servers: pool.busy_servers,
                total_servers: pool.total_servers,
                available: pool.ready_servers > 0,
                workspaces: pool.workspaces,
                uptime_secs: pool.uptime_secs,
                status: pool.status,
            };
            (format!("{:?}", pool.language), pool_status)
        })
        .collect::<HashMap<String, LanguagePoolStatus>>();

    LspDaemonStatus {
        uptime: std::time::Duration::from_secs(status.uptime_secs),
        total_requests: status.total_requests,
        active_connections: status.active_connections,
        language_pools,
        version: status.version.clone(),
        git_hash: status.git_hash.clone(),
        build_date: status.build_date.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_is_supported() {
        let config = LspConfig::default();
        let client = LspClient {
            stream: None,
            config,
        };

        // Test supported file types
        assert!(client.is_supported(&PathBuf::from("test.rs")));
        assert!(client.is_supported(&PathBuf::from("test.py")));
        assert!(client.is_supported(&PathBuf::from("test.js")));

        // Test unsupported file type
        assert!(!client.is_supported(&PathBuf::from("test.txt")));
    }
}
