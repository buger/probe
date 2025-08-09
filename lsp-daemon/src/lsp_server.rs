use crate::lsp_registry::LspServerConfig;
use crate::socket_path::normalize_executable;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration, Instant};
use tracing::{debug, info, warn};
use url::Url;

pub struct LspServer {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Box<dyn Write + Send>>>,
    stdout: Arc<Mutex<Box<dyn BufRead + Send>>>,
    request_id: Arc<Mutex<i64>>,
    project_root: Option<PathBuf>,
    initialized: bool,
}

impl std::fmt::Debug for LspServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspServer")
            .field("project_root", &self.project_root)
            .field("initialized", &self.initialized)
            .finish()
    }
}

impl LspServer {
    pub fn spawn_with_workspace(
        config: &LspServerConfig,
        workspace_root: &PathBuf,
    ) -> Result<Self> {
        Self::spawn_internal(config, Some(workspace_root))
    }

    pub fn spawn(config: &LspServerConfig) -> Result<Self> {
        info!(
            "Starting LSP server for {:?}: {} {}",
            config.language,
            config.command,
            config.args.join(" ")
        );
        Self::spawn_internal(config, None)
    }

    fn spawn_internal(config: &LspServerConfig, workspace_root: Option<&PathBuf>) -> Result<Self> {
        let command = normalize_executable(&config.command);
        info!("Spawning LSP server: {} {:?}", command, config.args);

        // Set working directory - use workspace root if provided
        let mut cmd = Command::new(&command);
        if let Some(workspace) = workspace_root {
            cmd.current_dir(workspace);
        } else if config.language == crate::language_detector::Language::Go {
            cmd.current_dir("/tmp");
        }

        let mut child = cmd
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()) // Capture stderr for debugging
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn {}: {}", command, e))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to get stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to get stdout"))?;

        // Spawn a thread to log stderr output (using std thread since ChildStderr isn't async)
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            // Log stderr output using tracing
                            tracing::warn!(target: "lsp_stderr", "{}", line);
                        }
                        Err(e) => {
                            // Log error and break to avoid infinite loop
                            tracing::error!(target: "lsp_stderr", "Error reading stderr: {}", e);
                            break;
                        }
                    }
                }
            });
        }

        Ok(Self {
            child: Arc::new(Mutex::new(Some(child))),
            stdin: Arc::new(Mutex::new(Box::new(stdin) as Box<dyn Write + Send>)),
            stdout: Arc::new(Mutex::new(
                Box::new(BufReader::new(stdout)) as Box<dyn BufRead + Send>
            )),
            request_id: Arc::new(Mutex::new(1)),
            project_root: None,
            initialized: false,
        })
    }

    pub async fn initialize(&mut self, config: &LspServerConfig) -> Result<()> {
        self.initialize_internal(config, None).await
    }

    /// Initialize server with a specific workspace
    pub async fn initialize_with_workspace(
        &mut self,
        config: &LspServerConfig,
        workspace_root: &Path,
    ) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        let request_id = self.next_request_id().await;

        // Initialize with the actual workspace root
        let absolute_path = if workspace_root.is_absolute() {
            workspace_root.to_path_buf()
        } else {
            std::env::current_dir()?.join(workspace_root)
        };

        let root_uri = Url::from_file_path(&absolute_path).map_err(|_| {
            anyhow!(
                "Failed to convert workspace root to URI: {:?}",
                absolute_path
            )
        })?;

        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri.to_string(),
            "workspaceFolders": [{
                "uri": root_uri.to_string(),
                "name": workspace_root.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("workspace")
            }],
            "capabilities": {
                "textDocument": {
                    "callHierarchy": {
                        "dynamicRegistration": false
                    },
                    "definition": {
                        "dynamicRegistration": false
                    },
                    "references": {
                        "dynamicRegistration": false
                    },
                    "hover": {
                        "dynamicRegistration": false
                    },
                    "completion": {
                        "dynamicRegistration": false,
                        "completionItem": {
                            "snippetSupport": true
                        }
                    }
                },
                "window": {
                    "workDoneProgress": true
                },
                "experimental": {
                    "statusNotification": true
                }
            },
            "initializationOptions": config.initialization_options
        });

        self.send_request("initialize", init_params, request_id)
            .await?;

        // Wait for initialize response with reduced timeout
        debug!("Waiting for initialize response with timeout 10s...");
        let response = self
            .wait_for_response(request_id, Duration::from_secs(10))
            .await?;
        debug!("Received initialize response!");

        if response.get("error").is_some() {
            return Err(anyhow!("Initialize failed: {:?}", response["error"]));
        }

        // Send initialized notification
        debug!("Sending initialized notification...");
        self.send_notification("initialized", json!({})).await?;
        debug!("Initialized notification sent!");

        self.initialized = true;
        self.project_root = Some(workspace_root.to_path_buf());
        info!(
            "LSP server initialized for {:?} with workspace {:?}",
            config.language, workspace_root
        );

        Ok(())
    }

    /// Initialize server with empty workspaceFolders array for multi-workspace support
    pub async fn initialize_empty(&mut self, config: &LspServerConfig) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        // Use tmp directory as fallback root for Go, current directory otherwise
        let _fallback_root = if config.language == crate::language_detector::Language::Go {
            PathBuf::from("/tmp")
        } else {
            std::env::current_dir()?
        };

        let request_id = self.next_request_id().await;

        // Initialize with a default workspace like OpenCode does
        // We can add more workspaces dynamically later
        let root_uri = Url::from_file_path(&_fallback_root)
            .map_err(|_| anyhow!("Failed to convert fallback root to URI"))?;

        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri.to_string(), // Provide a root URI like OpenCode
            "workspaceFolders": [{  // Start with one workspace like OpenCode
                "uri": root_uri.to_string(),
                "name": "default"
            }],
            "capabilities": {
                "textDocument": {
                    "callHierarchy": {
                        "dynamicRegistration": false
                    },
                    "definition": {
                        "dynamicRegistration": false
                    },
                    "references": {
                        "dynamicRegistration": false
                    },
                    "hover": {
                        "dynamicRegistration": false
                    },
                    "completion": {
                        "dynamicRegistration": false,
                        "completionItem": {
                            "snippetSupport": true
                        }
                    }
                },
                "window": {
                    "workDoneProgress": true
                },
                "experimental": {
                    "statusNotification": true
                }
            },
            "initializationOptions": config.initialization_options
        });

        self.send_request("initialize", init_params, request_id)
            .await?;

        // Wait for initialize response with reduced timeout
        debug!("Waiting for initialize response with timeout 10s...");
        let response = self
            .wait_for_response(
                request_id,
                Duration::from_secs(10), // Reduced from 300s to 10s
            )
            .await?;
        debug!("Received initialize response!");

        if response.get("error").is_some() {
            return Err(anyhow!("Initialize failed: {:?}", response["error"]));
        }

        // Send initialized notification
        debug!("Sending initialized notification...");
        self.send_notification("initialized", json!({})).await?;
        debug!("Initialized notification sent!");

        self.initialized = true;
        info!(
            "LSP server initialized for {:?} with empty workspace folders",
            config.language
        );

        Ok(())
    }

    async fn initialize_internal(
        &mut self,
        config: &LspServerConfig,
        workspace_root: Option<&PathBuf>,
    ) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        // Use provided workspace root, or fallback to current directory or /tmp for Go
        let project_root = if let Some(workspace) = workspace_root {
            workspace.clone()
        } else if config.language == crate::language_detector::Language::Go {
            PathBuf::from("/tmp")
        } else {
            std::env::current_dir()?
        };
        self.project_root = Some(project_root.clone());

        let request_id = self.next_request_id().await;

        let workspace_uri = Url::from_directory_path(&project_root)
            .map_err(|_| anyhow!("Failed to convert path"))?;

        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": workspace_uri.to_string(),
            "workspaceFolders": [{ // Include workspaceFolders in initialization
                "uri": workspace_uri.to_string(),
                "name": project_root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("workspace")
                    .to_string()
            }],
            "capabilities": {
                "textDocument": {
                    "callHierarchy": {
                        "dynamicRegistration": false
                    },
                    "definition": {
                        "dynamicRegistration": false
                    },
                    "references": {
                        "dynamicRegistration": false
                    },
                    "hover": {
                        "dynamicRegistration": false
                    },
                    "completion": {
                        "dynamicRegistration": false,
                        "completionItem": {
                            "snippetSupport": true
                        }
                    }
                },
                "window": {
                    "workDoneProgress": true
                },
                "experimental": {
                    "statusNotification": true
                }
            },
            "initializationOptions": config.initialization_options
        });

        self.send_request("initialize", init_params, request_id)
            .await?;

        // Wait for initialize response
        debug!(
            "Waiting for initialize response with timeout {}s...",
            config.initialization_timeout_secs
        );
        let response = self
            .wait_for_response(
                request_id,
                Duration::from_secs(config.initialization_timeout_secs),
            )
            .await?;
        debug!("Received initialize response!");

        if response.get("error").is_some() {
            return Err(anyhow!("Initialize failed: {:?}", response["error"]));
        }

        // Send initialized notification
        debug!("Sending initialized notification...");
        self.send_notification("initialized", json!({})).await?;
        debug!("Initialized notification sent!");

        self.initialized = true;
        info!("LSP server initialized for {:?}", config.language);

        Ok(())
    }

    pub async fn wait_until_ready(&mut self) -> Result<()> {
        // This method monitors LSP server messages to determine when it's ready
        // Similar to the original implementation but adapted for async

        eprintln!("[DEBUG] Starting wait_until_ready...");
        let start = Instant::now();
        let max_wait = Duration::from_secs(180); // Reduced to 3 minutes to detect stuck indexing faster
        let required_silence = Duration::from_secs(3); // Longer silence period
        let progress_stall_timeout = Duration::from_secs(60); // Detect if progress stalls for 60 seconds

        let mut cache_priming_completed = false;
        let mut silence_start: Option<Instant> = None;
        let mut last_progress_time = Instant::now();
        let mut last_progress_percentage: Option<u32> = None;

        eprintln!("[DEBUG] Starting message reading loop...");
        while start.elapsed() < max_wait {
            // Try to read a message with timeout
            match self.read_message_timeout(Duration::from_millis(100)).await {
                Ok(Some(msg)) => {
                    silence_start = None;

                    if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                        // Handle progress notifications
                        if method == "$/progress" {
                            if let Some(params) = msg.get("params") {
                                if let Some(token) = params.get("token").and_then(|t| t.as_str()) {
                                    if let Some(value) = params.get("value") {
                                        if let Some(kind) =
                                            value.get("kind").and_then(|k| k.as_str())
                                        {
                                            if kind == "end"
                                                && (token.contains("cachePriming")
                                                    || token.contains("Roots Scanned"))
                                            {
                                                cache_priming_completed = true;
                                                debug!("Indexing completed for token: {}", token);
                                            }

                                            // Monitor progress to detect stalled indexing
                                            if kind == "report" {
                                                let current_percentage = value
                                                    .get("percentage")
                                                    .and_then(|p| p.as_u64())
                                                    .map(|p| p as u32);
                                                if let Some(percentage) = current_percentage {
                                                    if let Some(last_pct) = last_progress_percentage
                                                    {
                                                        if percentage > last_pct {
                                                            // Progress made, update timestamp
                                                            last_progress_time = Instant::now();
                                                            debug!(
                                                                "Indexing progress: {}%",
                                                                percentage
                                                            );
                                                        }
                                                    } else {
                                                        // First progress report
                                                        last_progress_time = Instant::now();
                                                    }
                                                    last_progress_percentage = Some(percentage);

                                                    // Check for stalled progress
                                                    if last_progress_time.elapsed()
                                                        > progress_stall_timeout
                                                    {
                                                        debug!("Indexing appears to be stalled at {}% for {:?}", percentage, last_progress_time.elapsed());
                                                        if percentage >= 80 {
                                                            // If we're at 80%+ and stalled, consider it "good enough"
                                                            debug!("Proceeding with partial indexing ({}%)", percentage);
                                                            cache_priming_completed = true;
                                                        } else {
                                                            return Err(anyhow!("rust-analyzer indexing stalled at {}% for {:?}", percentage, last_progress_time.elapsed()));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Handle status notifications
                        if method == "rust-analyzer/status" {
                            if let Some(params) = msg.get("params") {
                                if let Some(status) = params.as_str() {
                                    if status == "ready" {
                                        debug!("LSP server reports ready");
                                        if cache_priming_completed {
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        }

                        // Respond to window/workDoneProgress/create requests
                        if method == "window/workDoneProgress/create" {
                            if let Some(id_value) = msg.get("id") {
                                // Handle various ID types (integer, string, null)
                                let response_id = if let Some(id_num) = id_value.as_i64() {
                                    id_num
                                } else if let Some(id_str) = id_value.as_str() {
                                    // Try to parse string as number, or use hash as fallback
                                    id_str.parse::<i64>().unwrap_or_else(|_| {
                                        warn!("Non-numeric ID received: {}, using 0", id_str);
                                        0
                                    })
                                } else {
                                    warn!(
                                        "Unexpected ID type in LSP request: {:?}, using 0",
                                        id_value
                                    );
                                    0
                                };

                                self.send_response(response_id, json!(null)).await?;
                            }
                        }
                    }
                }
                Ok(None) | Err(_) => {
                    // No message available
                    if silence_start.is_none() {
                        silence_start = Some(Instant::now());
                        if cache_priming_completed {
                            debug!("Cache priming complete, starting silence timer...");
                        }
                    }

                    if let Some(silence_time) = silence_start {
                        let elapsed = silence_time.elapsed();
                        if cache_priming_completed && elapsed >= required_silence {
                            debug!(
                                "Server ready after cache priming and {}s silence period",
                                elapsed.as_secs()
                            );
                            return Ok(());
                        }
                    }
                }
            }

            // Small delay before next iteration
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // If we've waited long enough, assume ready
        warn!("LSP server readiness timeout, proceeding anyway");
        Ok(())
    }

    async fn next_request_id(&self) -> i64 {
        let mut id = self.request_id.lock().await;
        let current = *id;
        *id += 1;
        current
    }

    async fn send_message(&self, msg: &Value) -> Result<()> {
        let bytes = msg.to_string();
        let message = format!("Content-Length: {}\r\n\r\n{}", bytes.len(), bytes);

        // Log outgoing message
        info!(target: "lsp_protocol", ">>> TO LSP: {}", 
            serde_json::to_string(&msg).unwrap_or_else(|_| msg.to_string()));

        // Simplified approach - just acquire the lock and write directly
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(message.as_bytes())?;
        stdin.flush()?;

        Ok(())
    }

    pub async fn send_request(&self, method: &str, params: Value, id: i64) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.send_message(&msg).await
    }

    pub async fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        self.send_message(&msg).await
    }

    async fn send_response(&self, id: i64, result: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        });

        self.send_message(&msg).await
    }

    async fn read_message(&self) -> Result<Value> {
        let mut stdout = self.stdout.lock().await;

        let mut header = String::new();
        let bytes_read = stdout.read_line(&mut header)?;

        if bytes_read == 0 {
            return Err(anyhow!("LSP server closed connection"));
        }

        if !header.starts_with("Content-Length:") {
            return Err(anyhow!("Invalid header: {}", header));
        }

        let len: usize = header["Content-Length:".len()..].trim().parse()?;

        // Skip empty line
        let mut empty_line = String::new();
        stdout.read_line(&mut empty_line)?;

        let mut body = vec![0; len];
        stdout.read_exact(&mut body)?;

        let msg: Value = serde_json::from_slice(&body)?;

        // Log incoming message
        info!(target: "lsp_protocol", "<<< FROM LSP: {}", 
            serde_json::to_string(&msg).unwrap_or_else(|_| msg.to_string()));

        Ok(msg)
    }

    async fn read_message_timeout(&self, duration: Duration) -> Result<Option<Value>> {
        match timeout(duration, self.read_message()).await {
            Ok(Ok(msg)) => Ok(Some(msg)),
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(None), // Timeout
        }
    }

    async fn wait_for_response(&self, id: i64, timeout_duration: Duration) -> Result<Value> {
        let start = Instant::now();
        let mut message_count = 0;
        let mut last_progress_log = Instant::now();

        while start.elapsed() < timeout_duration {
            // Log progress every 10 seconds during long waits
            if last_progress_log.elapsed() > Duration::from_secs(10) {
                debug!(
                    "Still waiting for response ID {} (elapsed: {:?}, messages seen: {})",
                    id,
                    start.elapsed(),
                    message_count
                );
                last_progress_log = Instant::now();
            }

            match self.read_message_timeout(Duration::from_millis(500)).await {
                Ok(Some(msg)) => {
                    message_count += 1;
                    let msg_id = msg.get("id").and_then(|i| i.as_i64());

                    // Log what kind of message we got
                    if let Some(_method) = msg.get("method").and_then(|m| m.as_str()) {
                        // Skip progress notifications in release mode
                    } else {
                        debug!(
                            "Got message with ID: {:?}, looking for {} (message #{})",
                            msg_id, id, message_count
                        );
                    }

                    if msg_id == Some(id) {
                        // Check if this is actually a response (not a request from the LSP server)
                        if msg.get("method").is_some() {
                            debug!(
                                "Ignoring request (not response) with ID {} - method: {:?}",
                                id,
                                msg.get("method")
                            );
                            // This is a request FROM the LSP server, not a response TO our request
                            continue;
                        }
                        debug!(
                            "Found matching response for ID {}! (took {:?}, saw {} messages)",
                            id,
                            start.elapsed(),
                            message_count
                        );
                        return Ok(msg);
                    }
                }
                Ok(None) => {
                    // Timeout on single read - this is normal, just continue
                }
                Err(e) => {
                    debug!("Error reading message: {}", e);
                    return Err(e);
                }
            }
        }

        debug!(
            "TIMEOUT: No response received for request ID {} after {:?} (saw {} total messages)",
            id, timeout_duration, message_count
        );
        Err(anyhow!(
            "Timeout waiting for response to request {} after {:?}",
            id,
            timeout_duration
        ))
    }

    pub async fn open_document(&self, file_path: &Path, content: &str) -> Result<()> {
        let uri =
            Url::from_file_path(file_path).map_err(|_| anyhow!("Failed to convert file path"))?;

        let params = json!({
            "textDocument": {
                "uri": uri.to_string(),
                "languageId": self.detect_language_id(file_path),
                "version": 1,
                "text": content
            }
        });

        // This is a notification, so we just send it and return immediately
        // No need to wait for any response since notifications don't have responses
        self.send_notification("textDocument/didOpen", params).await
    }

    pub async fn close_document(&self, file_path: &Path) -> Result<()> {
        let uri =
            Url::from_file_path(file_path).map_err(|_| anyhow!("Failed to convert file path"))?;

        let params = json!({
            "textDocument": {
                "uri": uri.to_string()
            }
        });

        self.send_notification("textDocument/didClose", params)
            .await
    }

    pub async fn test_readiness(&self, file_path: &Path, line: u32, column: u32) -> Result<bool> {
        let uri =
            Url::from_file_path(file_path).map_err(|_| anyhow!("Failed to convert file path"))?;

        let request_id = self.next_request_id().await;
        let params = json!({
            "textDocument": { "uri": uri.to_string() },
            "position": { "line": line, "character": column }
        });

        self.send_request("textDocument/hover", params, request_id)
            .await?;

        // Use a shorter timeout for readiness check
        match self
            .wait_for_response(request_id, Duration::from_secs(10))
            .await
        {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    pub async fn call_hierarchy(&self, file_path: &Path, line: u32, column: u32) -> Result<Value> {
        debug!(target: "lsp_call_hierarchy", "Starting call hierarchy for {:?} at {}:{}", 
            file_path, line, column);

        let uri =
            Url::from_file_path(file_path).map_err(|_| anyhow!("Failed to convert file path"))?;

        let request_id = self.next_request_id().await;

        // Prepare call hierarchy
        let params = json!({
            "textDocument": { "uri": uri.to_string() },
            "position": { "line": line, "character": column }
        });

        self.send_request("textDocument/prepareCallHierarchy", params, request_id)
            .await?;
        let response = self
            .wait_for_response(request_id, Duration::from_secs(60))
            .await
            .map_err(|e| {
                anyhow!(
                    "Call hierarchy prepare timed out - rust-analyzer may still be indexing: {}",
                    e
                )
            })?;

        if let Some(error) = response.get("error") {
            return Err(anyhow!("Call hierarchy prepare failed: {:?}", error));
        }

        // Handle null result (rust-analyzer returns null when no items found)
        let result = &response["result"];
        if result.is_null() {
            return Ok(json!({
                "incoming": [],
                "outgoing": []
            }));
        }

        let items = match result.as_array() {
            Some(array) => array,
            None => {
                return Ok(json!({
                    "incoming": [],
                    "outgoing": []
                }));
            }
        };

        if items.is_empty() {
            return Ok(json!({
                "incoming": [],
                "outgoing": []
            }));
        }

        let item = &items[0];

        // Get incoming calls
        let incoming_request_id = self.next_request_id().await;

        self.send_request(
            "callHierarchy/incomingCalls",
            json!({ "item": item }),
            incoming_request_id,
        )
        .await?;

        let incoming_response = self
            .wait_for_response(incoming_request_id, Duration::from_secs(60))
            .await?;

        // Get outgoing calls
        let outgoing_request_id = self.next_request_id().await;

        self.send_request(
            "callHierarchy/outgoingCalls",
            json!({ "item": item }),
            outgoing_request_id,
        )
        .await?;

        let outgoing_response = match self
            .wait_for_response(outgoing_request_id, Duration::from_secs(60))
            .await
        {
            Ok(response) => {
                // Check if there's an error in the response
                if let Some(_error) = response.get("error") {
                    // Return empty result instead of failing
                    json!({
                        "result": []
                    })
                } else {
                    response
                }
            }
            Err(_e) => {
                // Return empty result instead of failing
                json!({
                    "result": []
                })
            }
        };

        let result = json!({
            "item": item,
            "incoming": incoming_response["result"],
            "outgoing": outgoing_response["result"]
        });

        Ok(result)
    }

    pub async fn shutdown(&self) -> Result<()> {
        let mut child_opt = self.child.lock().await;
        if let Some(ref mut child) = *child_opt {
            // Try graceful shutdown first
            let request_id = self.next_request_id().await;
            if self
                .send_request("shutdown", json!(null), request_id)
                .await
                .is_ok()
            {
                // Wait briefly for shutdown response
                let _ = tokio::time::timeout(
                    Duration::from_secs(1),
                    self.wait_for_response(request_id, Duration::from_secs(1)),
                )
                .await;

                // Send exit notification
                let _ = self.send_notification("exit", json!(null)).await;
            }

            // Give the process a moment to shut down gracefully
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Force kill if still running
            match child.try_wait() {
                Ok(Some(_status)) => {
                    // Process exited gracefully
                }
                Ok(None) => {
                    if let Err(_e) = child.kill() {
                        // Failed to kill process
                    } else {
                        // Wait for process to actually die
                        let _ = tokio::time::timeout(Duration::from_secs(2), async {
                            let _ = child.wait();
                        })
                        .await;
                    }
                }
                Err(_e) => {
                    // Error checking process status
                }
            }

            // Ensure child is dropped
            *child_opt = None;
        }
        Ok(())
    }

    fn detect_language_id(&self, file_path: &Path) -> &str {
        match file_path.extension().and_then(|e| e.to_str()) {
            Some("rs") => "rust",
            Some("ts") | Some("tsx") => "typescript",
            Some("js") | Some("jsx") => "javascript",
            Some("py") => "python",
            Some("go") => "go",
            Some("java") => "java",
            Some("c") | Some("h") => "c",
            Some("cpp") | Some("cxx") | Some("cc") | Some("hpp") => "cpp",
            Some("cs") => "csharp",
            Some("rb") => "ruby",
            Some("php") => "php",
            Some("swift") => "swift",
            Some("kt") | Some("kts") => "kotlin",
            Some("scala") | Some("sc") => "scala",
            Some("hs") => "haskell",
            Some("ex") | Some("exs") => "elixir",
            Some("clj") | Some("cljs") => "clojure",
            Some("lua") => "lua",
            Some("zig") => "zig",
            _ => "plaintext",
        }
    }
}
