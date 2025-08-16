use anyhow::{anyhow, Result};
use lsp_daemon::{
    get_default_socket_path, protocol::InitializedWorkspace, remove_socket_file,
    CallHierarchyResult, DaemonRequest, DaemonResponse, DaemonStatus, IpcStream, Language,
    LanguageDetector, LanguageInfo, LogEntry, MessageCodec,
};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, timeout};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::lsp_integration::types::*;

#[derive(Debug)]
enum DaemonHealth {
    Healthy,
    VersionMismatch,
    Unhealthy,
}

/// Resolve the socket path with optional override.
/// If PROBE_LSP_SOCKET_PATH is set, we use it; otherwise fall back to the default.
fn effective_socket_path() -> String {
    if let Ok(p) = std::env::var("PROBE_LSP_SOCKET_PATH") {
        return p;
    }
    // Default (typically under TMPDIR)
    get_default_socket_path()
}

pub struct LspClient {
    stream: Option<IpcStream>,
    config: LspConfig,
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Ensure the stream is properly closed when the client is dropped
        if let Some(mut stream) = self.stream.take() {
            // Try to flush and properly close the stream
            // We use block_on here since Drop is not async
            // Using std::thread::spawn to avoid runtime issues during drop
            let _ = std::thread::spawn(move || {
                futures::executor::block_on(async move {
                    // Best effort - ignore errors since we're dropping anyway
                    let _ = stream.flush().await;
                    let _ = stream.shutdown().await;
                });
            })
            .join();
            debug!("LspClient dropped, connection closed");
        }
    }
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

    /// Create a non-blocking client that doesn't wait for LSP server to be ready
    /// Returns None if LSP is not available or still initializing
    pub async fn new_non_blocking(config: LspConfig) -> Option<Self> {
        let use_daemon = config.use_daemon;
        let mut client = Self {
            stream: None,
            config,
        };

        if use_daemon {
            // Try quick connection without auto-start or waiting
            if client.try_connect_no_wait().await.is_err() {
                return None;
            }
        }

        Some(client)
    }

    /// Try to connect without waiting for server to be ready
    /// This is used for non-blocking operations
    async fn try_connect_no_wait(&mut self) -> Result<()> {
        let socket_path = effective_socket_path();

        // Very short timeout - just check if daemon is there
        let quick_timeout = Duration::from_millis(100);

        match timeout(quick_timeout, IpcStream::connect(&socket_path)).await {
            Ok(Ok(stream)) => {
                self.stream = Some(stream);

                // Send connect message with short timeout
                let request = DaemonRequest::Connect {
                    client_id: Uuid::new_v4(),
                };

                match timeout(quick_timeout, self.send_request(request)).await {
                    Ok(Ok(response)) => {
                        if let DaemonResponse::Connected { daemon_version, .. } = response {
                            debug!("Quick connect to daemon version: {}", daemon_version);
                        }
                        Ok(())
                    }
                    Ok(Err(e)) => {
                        debug!("LSP daemon not ready: {}", e);
                        self.stream = None;
                        Err(anyhow!("LSP daemon not ready"))
                    }
                    Err(_) => {
                        debug!("LSP daemon connection timed out");
                        self.stream = None;
                        Err(anyhow!("LSP daemon not available"))
                    }
                }
            }
            Ok(Err(e)) => {
                debug!("No LSP daemon running: {}", e);
                // Try to start daemon in background but don't wait
                let _ = start_embedded_daemon_background().await;
                info!("LSP daemon starting in background, skipping LSP operations");
                Err(anyhow!("LSP daemon not available (starting in background)"))
            }
            Err(_) => {
                debug!("Quick connection check timed out");
                Err(anyhow!("LSP daemon not available"))
            }
        }
    }

    /// Connect to the LSP daemon, auto-starting if necessary
    async fn connect(&mut self) -> Result<()> {
        let socket_path = effective_socket_path();
        // Use shorter timeout for initial connection attempt
        let connection_timeout = Duration::from_secs(5);

        debug!("Attempting to connect to LSP daemon at: {}", socket_path);

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

                    match timeout(connection_timeout, self.send_request_internal(request)).await {
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

                    match timeout(connection_timeout, self.send_request_internal(request)).await {
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

    /// Send a request to the daemon with retry logic for connection issues
    async fn send_request_with_retry(&mut self, request: DaemonRequest) -> Result<DaemonResponse> {
        const MAX_RETRIES: u32 = 3;
        let mut last_error = None;

        for retry in 0..MAX_RETRIES {
            match self.send_request_internal(request.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    let error_msg = e.to_string();
                    let is_retryable = error_msg.contains("early eof")
                        || error_msg.contains("Failed to read message length")
                        || error_msg.contains("Connection refused")
                        || error_msg.contains("connection reset")
                        || error_msg.contains("broken pipe");

                    if !is_retryable {
                        return Err(e);
                    }

                    warn!(
                        "LSP request failed with retryable error (attempt {}): {}",
                        retry + 1,
                        e
                    );
                    last_error = Some(e);

                    if retry < MAX_RETRIES - 1 {
                        // Reconnect before retry
                        self.stream = None;
                        tokio::time::sleep(Duration::from_millis(500 * (retry + 1) as u64)).await;

                        if let Err(conn_err) = self.connect().await {
                            warn!("Failed to reconnect for retry: {}", conn_err);
                            continue;
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("All retry attempts failed")))
    }

    /// Send a request to the daemon and wait for response (public interface with retry)
    async fn send_request(&mut self, request: DaemonRequest) -> Result<DaemonResponse> {
        self.send_request_with_retry(request).await
    }

    /// Send a request to the daemon and wait for response (internal implementation)
    async fn send_request_internal(&mut self, request: DaemonRequest) -> Result<DaemonResponse> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow!("Not connected to daemon"))?;

        debug!("Sending request: {:?}", request);

        // Encode and send request
        let encoded = MessageCodec::encode(&request)?;
        stream.write_all(&encoded).await?;
        stream.flush().await?;

        // Read response with timeout using proper message framing
        let timeout_duration = Duration::from_millis(self.config.timeout_ms);
        debug!(
            "Waiting for response with timeout: {}ms",
            self.config.timeout_ms
        );

        // Read message length (4 bytes)
        let mut length_buf = [0u8; 4];
        match timeout(timeout_duration, stream.read_exact(&mut length_buf)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                error!("Failed to read message length: {}", e);
                return Err(anyhow!("Failed to read message length: {}", e));
            }
            Err(_) => {
                error!(
                    "Timeout reading message length after {}ms",
                    self.config.timeout_ms
                );
                let sp = effective_socket_path();
                return Err(anyhow!(
                    "Timeout connecting to daemon after {}ms (socket: {})",
                    self.config.timeout_ms,
                    sp
                ));
            }
        }
        let message_len = u32::from_be_bytes(length_buf) as usize;
        debug!("Message length: {} bytes", message_len);

        // Ensure we don't try to read unreasonably large messages (10MB limit)
        if message_len > 10 * 1024 * 1024 {
            return Err(anyhow!("Message too large: {} bytes", message_len));
        }

        // Read the complete message body
        let mut message_buf = vec![0u8; message_len];
        match timeout(timeout_duration, stream.read_exact(&mut message_buf)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                error!(
                    "Failed to read message body of {} bytes: {}",
                    message_len, e
                );
                return Err(anyhow!("Failed to read message body: {}", e));
            }
            Err(_) => {
                error!(
                    "Timeout reading message body of {} bytes after {}ms",
                    message_len, self.config.timeout_ms
                );
                let sp = effective_socket_path();
                return Err(anyhow!(
                    "Timeout waiting for daemon response after {}ms (socket: {})",
                    self.config.timeout_ms,
                    sp
                ));
            }
        }

        // Reconstruct the complete message with length prefix for decoding
        let mut complete_message = Vec::with_capacity(4 + message_len);
        complete_message.extend_from_slice(&length_buf);
        complete_message.extend_from_slice(&message_buf);

        // Decode response
        let response = MessageCodec::decode_response(&complete_message)?;
        debug!("Received response: {:?}", response);

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
        debug!(
            "Getting call hierarchy for {:?} at {}:{}",
            file_path, line, column
        );

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

        debug!("Sending CallHierarchy request to daemon");

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

        debug!("Received response from daemon");

        match response {
            DaemonResponse::CallHierarchy { result, .. } => {
                debug!("Call hierarchy response received successfully");
                let converted = convert_call_hierarchy_result(result);
                Ok(converted)
            }
            DaemonResponse::Error { error, .. } => {
                debug!("Call hierarchy failed: {}", error);
                Err(anyhow!("Call hierarchy failed: {}", error))
            }
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

    /// Initialize workspaces
    pub async fn init_workspaces(
        &mut self,
        workspace_root: PathBuf,
        languages: Option<Vec<String>>,
        recursive: bool,
        enable_watchdog: bool,
    ) -> Result<(Vec<InitializedWorkspace>, Vec<String>)> {
        // Convert language strings to Language enum
        let languages = languages.map(|langs| {
            langs
                .into_iter()
                .filter_map(|lang| {
                    let lang_lower = lang.to_lowercase();
                    match lang_lower.as_str() {
                        "rust" => Some(Language::Rust),
                        "typescript" | "ts" => Some(Language::TypeScript),
                        "javascript" | "js" => Some(Language::JavaScript),
                        "python" | "py" => Some(Language::Python),
                        "go" => Some(Language::Go),
                        "java" => Some(Language::Java),
                        "c" => Some(Language::C),
                        "cpp" | "c++" => Some(Language::Cpp),
                        "csharp" | "c#" => Some(Language::CSharp),
                        "ruby" | "rb" => Some(Language::Ruby),
                        "php" => Some(Language::Php),
                        "swift" => Some(Language::Swift),
                        "kotlin" | "kt" => Some(Language::Kotlin),
                        "scala" => Some(Language::Scala),
                        "haskell" | "hs" => Some(Language::Haskell),
                        "elixir" | "ex" => Some(Language::Elixir),
                        "clojure" | "clj" => Some(Language::Clojure),
                        "lua" => Some(Language::Lua),
                        "zig" => Some(Language::Zig),
                        _ => None,
                    }
                })
                .collect()
        });

        let request = DaemonRequest::InitWorkspaces {
            request_id: Uuid::new_v4(),
            workspace_root,
            languages,
            recursive,
            enable_watchdog,
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::WorkspacesInitialized {
                initialized,
                errors,
                ..
            } => Ok((initialized, errors)),
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

/// Check daemon health and version compatibility
async fn check_daemon_health() -> Result<DaemonHealth> {
    let socket_path = effective_socket_path();

    // Try to connect to existing daemon
    let mut stream = match timeout(Duration::from_secs(2), IpcStream::connect(&socket_path)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Err(anyhow!("Failed to connect to daemon: {}", e)),
        Err(_) => return Err(anyhow!("Connection timeout")),
    };

    // Send ping request to check health
    let ping_request = DaemonRequest::Ping {
        request_id: Uuid::new_v4(),
    };

    let encoded = MessageCodec::encode(&ping_request)?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;

    // Read ping response with timeout
    let mut length_buf = [0u8; 4];
    match timeout(Duration::from_secs(2), stream.read_exact(&mut length_buf)).await {
        Ok(Ok(_)) => {}
        _ => return Ok(DaemonHealth::Unhealthy),
    }

    let length = u32::from_be_bytes(length_buf) as usize;
    let mut response_buf = vec![0u8; length];
    match timeout(Duration::from_secs(2), stream.read_exact(&mut response_buf)).await {
        Ok(Ok(_)) => {}
        _ => return Ok(DaemonHealth::Unhealthy),
    }

    let response = MessageCodec::decode_response(&[&length_buf[..], &response_buf[..]].concat())?;

    // Check if we got a pong response
    match response {
        DaemonResponse::Pong { .. } => {
            // Daemon is responsive, now check version
            if check_daemon_version_compatibility().await.unwrap_or(false) {
                Ok(DaemonHealth::Healthy)
            } else {
                Ok(DaemonHealth::VersionMismatch)
            }
        }
        _ => Ok(DaemonHealth::Unhealthy),
    }
}

/// Check if daemon version matches probe binary version
async fn check_daemon_version_compatibility() -> Result<bool> {
    let socket_path = effective_socket_path();

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
    let socket_path = effective_socket_path();

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

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::Instant;

/// Wrapper for client startup lock file that cleans up on drop
struct ClientStartupLock {
    _file: File,
    path: String,
}

impl Drop for ClientStartupLock {
    fn drop(&mut self) {
        // Clean up the lock file when dropped
        let _ = std::fs::remove_file(&self.path);
        debug!("Released client startup lock");
    }
}

/// Global path for client startup coordination lock
fn get_client_lock_path() -> String {
    // Use platform-appropriate temp directory
    let temp_dir = std::env::temp_dir();
    temp_dir
        .join("probe-lsp-client-start.lock")
        .to_string_lossy()
        .to_string()
}

/// Start embedded LSP daemon in the background using probe binary
async fn start_embedded_daemon_background() -> Result<()> {
    let socket_path = effective_socket_path();

    // Use file-based locking for cross-process coordination
    let _lock = acquire_client_startup_lock()?;

    // Double-check after acquiring the lock - another process might have started the daemon
    match check_daemon_health().await {
        Ok(DaemonHealth::Healthy) => {
            debug!("Daemon already running and healthy (after acquiring lock)");
            return Ok(());
        }
        Ok(DaemonHealth::VersionMismatch) => {
            info!("Daemon version mismatch detected, restarting daemon...");
            shutdown_existing_daemon().await?;
        }
        Ok(DaemonHealth::Unhealthy) => {
            warn!("Daemon is unhealthy, restarting...");
            shutdown_existing_daemon().await?;
        }
        Err(_) => {
            // No daemon running or can't connect
            debug!("No daemon running, will start new one");
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
    let mut cmd = std::process::Command::new(&probe_binary);
    cmd.args(["lsp", "start"])
        .stdin(std::process::Stdio::null());
    // In CI or when debugging, inherit stdout/stderr so early failures (bind/lock) are visible.
    // Enable by setting LSP_VERBOSE_SPAWN=1 in the environment.
    if std::env::var("LSP_VERBOSE_SPAWN").ok().as_deref() == Some("1") {
        cmd.stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
    } else {
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    cmd.spawn()
        .map_err(|e| anyhow!("Failed to spawn embedded daemon: {}", e))?;

    info!("Started embedded daemon in background");

    // Lock will be automatically released when _lock goes out of scope

    Ok(())
}

/// Acquire a file-based lock for client startup coordination
fn acquire_client_startup_lock() -> Result<ClientStartupLock> {
    let lock_path = get_client_lock_path();
    let start_time = Instant::now();
    let max_wait = Duration::from_secs(10);

    loop {
        // Try to create the lock file exclusively
        match OpenOptions::new()
            .write(true)
            .create_new(true) // Atomic creation - fails if file exists
            .open(&lock_path)
        {
            Ok(mut file) => {
                // Write our PID to help with debugging
                let _ = writeln!(file, "{}", std::process::id());
                debug!("Acquired client startup lock");
                return Ok(ClientStartupLock {
                    _file: file,
                    path: lock_path,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Another client is starting the daemon
                if start_time.elapsed() > max_wait {
                    // Clean up potentially stale lock
                    let _ = std::fs::remove_file(&lock_path);
                    return Err(anyhow!("Timeout waiting for client startup lock"));
                }

                debug!("Another client is starting daemon, waiting...");
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(anyhow!("Failed to acquire client startup lock: {}", e)),
        }
    }
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
