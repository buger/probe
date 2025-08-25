use crate::lsp_registry::LspServerConfig;
use crate::path_safety;
use crate::socket_path::normalize_executable;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration, Instant};
use tracing::{debug, error, info, warn};
use url::Url;

pub struct LspServer {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    request_id: Arc<Mutex<i64>>,
    project_root: Option<PathBuf>,
    initialized: bool,
    stderr_thread: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    stderr_shutdown: Arc<AtomicBool>,
    // Track server type and opened documents for smart management
    server_name: String,
    opened_documents: Arc<Mutex<HashSet<PathBuf>>>,
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
    /// Return a canonical (real) path if possible, otherwise a best-effort absolute path.
    fn canonicalize_for_uri(p: &Path) -> PathBuf {
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(p)
        };
        std::fs::canonicalize(&abs).unwrap_or(abs)
    }

    fn paths_equal(a: &Path, b: &Path) -> bool {
        let ca = Self::canonicalize_for_uri(a);
        let cb = Self::canonicalize_for_uri(b);
        ca == cb
    }

    fn is_within(child: &Path, base: &Path) -> bool {
        let c = Self::canonicalize_for_uri(child);
        let b = Self::canonicalize_for_uri(base);
        c.starts_with(&b)
    }

    /// Get the PID of the LSP server process
    pub fn get_pid(&self) -> Option<u32> {
        // This needs to be sync since we're calling from async context but Child is not Send
        let child_opt = self.child.try_lock().ok()?;
        child_opt.as_ref().and_then(|child| child.id())
    }

    pub fn spawn_with_workspace(config: &LspServerConfig, workspace_root: &Path) -> Result<Self> {
        // For gopls, use the Go module root if we can find it
        let effective_root = if config.language == crate::language_detector::Language::Go {
            let module_root = Self::find_go_module_root(workspace_root)
                .unwrap_or_else(|| workspace_root.to_path_buf());

            // For gopls, we'll run go mod operations after initialization
            // since we can't use async here
            info!("Will prepare Go module at: {:?}", module_root);

            module_root
        } else {
            workspace_root.to_path_buf()
        };

        Self::spawn_internal(config, Some(&effective_root))
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

    fn spawn_internal(config: &LspServerConfig, workspace_root: Option<&Path>) -> Result<Self> {
        let command = normalize_executable(&config.command);
        info!("Spawning LSP server: {} {:?}", command, config.args);

        // Set working directory - use workspace root if provided
        // This is critical for gopls which needs to run in the Go module root
        let mut child = tokio::process::Command::new(&command);
        if let Some(workspace) = workspace_root {
            info!(
                "Setting working directory for {:?} to: {:?}",
                config.language, workspace
            );
            child.current_dir(workspace);
        } else if config.language == crate::language_detector::Language::Go {
            info!("No workspace provided for Go, using /tmp as fallback");
            child.current_dir("/tmp");
        }

        let mut child = child
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

        // Track stderr thread and shutdown flag
        let stderr_shutdown = Arc::new(AtomicBool::new(false));
        let stderr_thread = if let Some(stderr) = child.stderr.take() {
            let shutdown_flag = stderr_shutdown.clone();
            Some(tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = reader.lines();

                loop {
                    // Check if we should shutdown
                    if shutdown_flag.load(Ordering::Relaxed) {
                        tracing::debug!(target: "lsp_stderr", "Stderr thread shutting down gracefully");
                        break;
                    }

                    match tokio::time::timeout(
                        std::time::Duration::from_millis(100),
                        lines.next_line(),
                    )
                    .await
                    {
                        Ok(Ok(Some(line))) => {
                            // Log stderr output using tracing
                            tracing::warn!(target: "lsp_stderr", "{}", line);
                        }
                        Ok(Ok(None)) => {
                            // EOF reached
                            tracing::debug!(target: "lsp_stderr", "Stderr EOF reached");
                            break;
                        }
                        Ok(Err(e)) => {
                            // Log error and break to avoid infinite loop
                            tracing::error!(target: "lsp_stderr", "Error reading stderr: {}", e);
                            break;
                        }
                        Err(_) => {
                            // Timeout - continue loop to check shutdown flag
                            continue;
                        }
                    }
                }
                tracing::debug!(target: "lsp_stderr", "Stderr reading thread terminated");
            }))
        } else {
            None
        };

        Ok(Self {
            child: Arc::new(Mutex::new(Some(child))),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            request_id: Arc::new(Mutex::new(1)),
            project_root: None,
            initialized: false,
            stderr_thread: Arc::new(Mutex::new(stderr_thread)),
            stderr_shutdown,
            server_name: config.command.clone(),
            opened_documents: Arc::new(Mutex::new(HashSet::new())),
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

        // Initialize with the actual workspace root (canonicalized)
        let absolute_path = if workspace_root.is_absolute() {
            workspace_root.to_path_buf()
        } else {
            std::env::current_dir()?.join(workspace_root)
        };
        let canonical_root = Self::canonicalize_for_uri(&absolute_path);

        let root_uri = Url::from_file_path(&canonical_root).map_err(|_| {
            anyhow!(
                "Failed to convert workspace root to URI: {:?}",
                canonical_root
            )
        })?;

        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri.to_string(),
            "rootPath": workspace_root.to_str(), // Deprecated but some servers still use it
            "workspaceFolders": [{
                "uri": root_uri.to_string(),
                "name": canonical_root.file_name()
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
                "workspace": {
                    "configuration": true,
                    "workspaceFolders": true
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
        self.project_root = Some(canonical_root.clone());
        info!(
            "LSP server initialized for {:?} with workspace {:?}",
            config.language, canonical_root
        );

        // For gopls, perform additional initialization steps
        if self.is_gopls() {
            // Find the actual Go module root (where go.mod is)
            let module_root = Self::find_go_module_root(&canonical_root)
                .unwrap_or_else(|| canonical_root.to_path_buf());

            if !Self::paths_equal(&module_root, &canonical_root) {
                info!(
                    "Using Go module root: {:?} instead of workspace: {:?}",
                    module_root, canonical_root
                );
                self.project_root = Some(Self::canonicalize_for_uri(&module_root));
            }

            // Run go mod download and tidy FIRST
            info!("Preparing Go module dependencies before gopls workspace initialization...");
            if let Err(e) = Self::ensure_go_dependencies(&module_root).await {
                warn!("Failed to ensure Go dependencies: {}", e);
            }

            // Now perform gopls-specific initialization with workspace commands
            if let Err(e) = self.initialize_gopls_workspace(&module_root).await {
                warn!("Gopls workspace initialization had issues: {}", e);
            }
        }

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
                "workspace": {
                    "configuration": true,
                    "workspaceFolders": true
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

        // For rust-analyzer, we need to add linkedProjects to the initialization options
        let mut initialization_options = config.initialization_options.clone();
        if config.language == crate::language_detector::Language::Rust {
            // Find Cargo.toml files in the workspace
            let cargo_toml_path = project_root.join("Cargo.toml");
            if path_safety::exists_no_follow(&cargo_toml_path) {
                debug!(
                    "Found Cargo.toml at {:?}, adding to linkedProjects",
                    cargo_toml_path
                );

                // Merge linkedProjects into existing initialization options
                if let Some(ref mut options) = initialization_options {
                    if let Some(obj) = options.as_object_mut() {
                        obj.insert(
                            "linkedProjects".to_string(),
                            json!([cargo_toml_path.to_string_lossy().to_string()]),
                        );
                    }
                } else {
                    initialization_options = Some(json!({
                        "linkedProjects": [cargo_toml_path.to_string_lossy().to_string()]
                    }));
                }
                info!(
                    "Added linkedProjects for rust-analyzer: {:?}",
                    cargo_toml_path
                );
            } else {
                warn!("No Cargo.toml found in {:?}, rust-analyzer may not recognize files as part of a crate", project_root);
            }
        }

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
                "workspace": {
                    "configuration": true,
                    "workspaceFolders": true
                },
                "window": {
                    "workDoneProgress": true
                },
                "experimental": {
                    "statusNotification": true
                }
            },
            "initializationOptions": initialization_options
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
                                // Handle both string and numeric tokens (gopls uses numeric tokens)
                                let token_str = if let Some(token) = params.get("token") {
                                    if let Some(s) = token.as_str() {
                                        Some(s.to_string())
                                    } else if let Some(n) = token.as_u64() {
                                        Some(n.to_string())
                                    } else {
                                        token.as_i64().map(|n| n.to_string())
                                    }
                                } else {
                                    None
                                };

                                if let Some(token) = token_str {
                                    if let Some(value) = params.get("value") {
                                        if let Some(kind) =
                                            value.get("kind").and_then(|k| k.as_str())
                                        {
                                            // Track progress for debugging
                                            debug!("Progress notification - token: {}, kind: {}, value: {:?}", token, kind, value);

                                            // Check for end of work
                                            if kind == "end" {
                                                // Check for various completion tokens from different language servers
                                                if token.contains("cachePriming")
                                                    || token.contains("Roots Scanned")
                                                    || token.contains("gopls")  // Go-specific progress tokens
                                                    || token.contains("index")  // Generic indexing tokens
                                                    || token.contains("load")
                                                // Loading/analyzing tokens
                                                {
                                                    cache_priming_completed = true;
                                                    debug!(
                                                        "Indexing completed for token: {}",
                                                        token
                                                    );
                                                } else {
                                                    // For gopls numeric tokens, check the work title
                                                    if let Some(title) =
                                                        value.get("title").and_then(|t| t.as_str())
                                                    {
                                                        if title.contains("Loading")
                                                            || title.contains("Indexing")
                                                        {
                                                            cache_priming_completed = true;
                                                            debug!(
                                                                "Gopls indexing completed: {}",
                                                                title
                                                            );
                                                        }
                                                    }
                                                }
                                            }

                                            // Also track begin/report progress for Go
                                            if kind == "begin" {
                                                if let Some(title) =
                                                    value.get("title").and_then(|t| t.as_str())
                                                {
                                                    if title.contains("Loading")
                                                        || title.contains("Indexing")
                                                    {
                                                        debug!("Gopls indexing started: {}", title);
                                                    }
                                                }
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

                        // Handle workspace/configuration requests (important for gopls)
                        if method == "workspace/configuration" {
                            if let Some(id_value) = msg.get("id") {
                                let response_id = if let Some(id_num) = id_value.as_i64() {
                                    id_num
                                } else if let Some(id_str) = id_value.as_str() {
                                    id_str.parse::<i64>().unwrap_or(0)
                                } else {
                                    0
                                };

                                debug!("Received workspace/configuration request from server");
                                // Return empty configurations like OpenCode does - let gopls use defaults
                                let result = if let Some(params) = msg.get("params") {
                                    if let Some(items) =
                                        params.get("items").and_then(|i| i.as_array())
                                    {
                                        // Return an empty object for each configuration item
                                        let configs: Vec<Value> =
                                            items.iter().map(|_| json!({})).collect();
                                        json!(configs)
                                    } else {
                                        json!([{}])
                                    }
                                } else {
                                    json!([{}])
                                };

                                self.send_response(response_id, result).await?;
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
        stdin.write_all(message.as_bytes()).await?;
        stdin.flush().await?;

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
        let bytes_read = stdout.read_line(&mut header).await?;

        if bytes_read == 0 {
            return Err(anyhow!("LSP server closed connection"));
        }

        if !header.starts_with("Content-Length:") {
            return Err(anyhow!("Invalid header: {}", header));
        }

        let len: usize = header["Content-Length:".len()..].trim().parse()?;

        // Skip empty line
        let mut empty_line = String::new();
        stdout.read_line(&mut empty_line).await?;

        let mut body = vec![0; len];
        stdout.read_exact(&mut body).await?;

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

                    // Handle server-initiated requests (like window/workDoneProgress/create)
                    // A message with both 'id' and 'method' is a request, not a response
                    if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                        // This is a request FROM the server (has both id and method)
                        if method == "window/workDoneProgress/create" {
                            if let Some(server_request_id) = msg_id {
                                debug!("Received window/workDoneProgress/create request from server with id: {}", server_request_id);
                                // Send acknowledgment response
                                let response = json!({
                                    "jsonrpc": "2.0",
                                    "id": server_request_id,
                                    "result": null
                                });
                                if let Err(e) = self.send_message(&response).await {
                                    warn!("Failed to acknowledge progress create request: {}", e);
                                }
                            }
                            continue; // This was a server request, not our response
                        }

                        // Handle workspace/configuration requests (critical for gopls)
                        if method == "workspace/configuration" {
                            if let Some(server_request_id) = msg_id {
                                debug!("Received workspace/configuration request from server with id: {}", server_request_id);

                                // Return empty configurations to let gopls use its defaults.
                                // This matches how the VS Code Go extension behaves and avoids
                                // unintentionally restricting workspace discovery via directoryFilters.
                                let result = if let Some(params) = msg.get("params") {
                                    if let Some(items) =
                                        params.get("items").and_then(|i| i.as_array())
                                    {
                                        let configs: Vec<Value> =
                                            items.iter().map(|_| json!({})).collect();
                                        json!(configs)
                                    } else {
                                        json!([{}])
                                    }
                                } else {
                                    json!([{}])
                                };

                                let response = json!({
                                    "jsonrpc": "2.0",
                                    "id": server_request_id,
                                    "result": result
                                });
                                if let Err(e) = self.send_message(&response).await {
                                    warn!("Failed to respond to configuration request: {}", e);
                                }
                            }
                            continue; // This was a server request, not our response
                        }

                        // Any other request from server - just continue waiting
                        if let Some(server_request_id) = msg_id {
                            debug!(
                                "Ignoring server request with ID {} (looking for response to {}), method: {}",
                                server_request_id, id, method
                            );
                        }
                        continue;
                    }

                    if msg_id == Some(id) {
                        // Check if this is actually a response (not a request from the LSP server)
                        if msg.get("method").is_some() {
                            // Should not get here after handling above
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
        let canon = Self::canonicalize_for_uri(file_path);
        let uri =
            Url::from_file_path(&canon).map_err(|_| anyhow!("Failed to convert file path"))?;

        let language_id = self.detect_language_id(&canon);

        debug!(
            "Opening document: uri={}, language={}, content_length={}",
            uri,
            language_id,
            content.len()
        );

        let params = json!({
            "textDocument": {
                "uri": uri.to_string(),
                "languageId": language_id,
                "version": 1,
                "text": content
            }
        });

        // This is a notification, so we just send it and return immediately
        // No need to wait for any response since notifications don't have responses
        self.send_notification("textDocument/didOpen", params).await
    }

    pub async fn close_document(&self, file_path: &Path) -> Result<()> {
        let canon = Self::canonicalize_for_uri(file_path);
        let uri =
            Url::from_file_path(&canon).map_err(|_| anyhow!("Failed to convert file path"))?;

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

    // Helper method to check if this is gopls
    fn is_gopls(&self) -> bool {
        self.server_name == "gopls" || self.server_name.ends_with("/gopls")
    }

    // Helper method to check if this is rust-analyzer
    fn is_rust_analyzer(&self) -> bool {
        self.server_name == "rust-analyzer" || self.server_name.ends_with("/rust-analyzer")
    }

    // Execute workspace command (needed for gopls.tidy and other commands)
    pub async fn execute_command(&self, command: &str, arguments: Vec<Value>) -> Result<Value> {
        let request_id = self.next_request_id().await;
        let params = json!({
            "command": command,
            "arguments": arguments
        });

        debug!(
            "Executing workspace command: {} with args: {:?}",
            command, arguments
        );
        self.send_request("workspace/executeCommand", params, request_id)
            .await?;

        // Give more time for workspace commands
        self.wait_for_response(request_id, Duration::from_secs(30))
            .await
    }

    // Find Go module root by looking for go.mod
    fn find_go_module_root(start_dir: &Path) -> Option<PathBuf> {
        let mut current = start_dir;
        loop {
            if path_safety::exists_no_follow(&current.join("go.mod")) {
                debug!("Found go.mod at {:?}", current);
                return Some(current.to_path_buf());
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => {
                    debug!("No go.mod found in directory tree");
                    return None;
                }
            }
        }
    }

    // Ensure Go dependencies are downloaded before gopls starts
    async fn ensure_go_dependencies(module_root: &Path) -> Result<()> {
        use tokio::process::Command;

        debug!("Running 'go mod download' in {:?}", module_root);

        let output = Command::new("go")
            .args(["mod", "download"])
            .current_dir(module_root)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("go mod download warning: {}", stderr);
            // Don't fail - gopls might still work
        } else {
            debug!("Successfully downloaded Go dependencies");
        }

        // Also run go mod tidy to clean up
        let tidy_output = Command::new("go")
            .args(["mod", "tidy"])
            .current_dir(module_root)
            .output()
            .await?;

        if !tidy_output.status.success() {
            let stderr = String::from_utf8_lossy(&tidy_output.stderr);
            warn!("go mod tidy warning: {}", stderr);
        } else {
            debug!("Successfully tidied Go module");
        }

        Ok(())
    }

    // Simple gopls workspace initialization - following VS Code's minimal approach
    async fn initialize_gopls_workspace(&self, workspace_root: &Path) -> Result<()> {
        info!(
            "Performing gopls workspace initialization at {:?}",
            workspace_root
        );

        // Send basic gopls configuration similar to VS Code
        let config_params = json!({
            "settings": {
                "gopls": {
                    // Essential settings for proper package detection
                    "expandWorkspaceToModule": true,
                    // experimentalWorkspaceModule is deprecated in gopls v0.17+
                    "buildFlags": [],
                    "env": {}
                }
            }
        });

        if let Err(e) = self
            .send_notification("workspace/didChangeConfiguration", config_params)
            .await
        {
            warn!("Failed to send gopls configuration: {}", e);
        } else {
            info!("Sent basic gopls configuration");
        }

        // Allow gopls to naturally discover and index the workspace
        // VS Code doesn't mass-open files during initialization
        info!("Allowing gopls time to naturally index the workspace...");
        tokio::time::sleep(Duration::from_secs(3)).await;

        info!("Gopls workspace initialization complete");
        Ok(())
    }

    // Safely open a file, handling errors gracefully with atomic operation
    async fn open_file_safely(&self, file_path: &Path) -> Result<()> {
        let canonical_path = Self::canonicalize_for_uri(file_path);

        // Use atomic check-and-set to prevent duplicate document opening
        {
            let mut docs = self.opened_documents.lock().await;
            if docs.contains(&canonical_path) {
                debug!(
                    "Document {:?} already opened by another thread",
                    canonical_path
                );
                return Ok(());
            }
            // Mark as opened immediately to prevent race condition
            docs.insert(canonical_path.clone());
        }

        // Read file content and send didOpen notification
        match tokio::fs::read_to_string(&canonical_path).await {
            Ok(content) => {
                if let Err(e) = self.open_document(&canonical_path, &content).await {
                    // Remove from opened set if opening failed
                    let mut docs = self.opened_documents.lock().await;
                    docs.remove(&canonical_path);
                    debug!("Failed to open {:?}: {}", canonical_path, e);
                    return Err(e);
                }
                debug!("Successfully opened document: {:?}", canonical_path);
                Ok(())
            }
            Err(e) => {
                // Remove from opened set if reading failed
                let mut docs = self.opened_documents.lock().await;
                docs.remove(&canonical_path);
                debug!("Failed to read file {:?}: {}", canonical_path, e);
                Err(anyhow!("Failed to read file: {}", e))
            }
        }
    }

    // Helper to check if a document is already opened
    async fn is_document_open(&self, file_path: &Path) -> bool {
        let canonical_path = Self::canonicalize_for_uri(file_path);
        let docs = self.opened_documents.lock().await;
        docs.contains(&canonical_path)
    }

    // Simple document readiness for gopls - VS Code's approach
    async fn ensure_document_ready(&self, file_path: &Path) -> Result<()> {
        let abs_path = if file_path.is_absolute() {
            file_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(file_path)
        };

        // Ensure the module root for this file is part of the workspace (critical for gopls).
        if self.is_gopls() {
            self.ensure_workspace_for_path(&abs_path).await?;
        }

        if !self.is_document_open(&abs_path).await {
            info!("Opening document for LSP analysis: {:?}", abs_path);

            // Use atomic open operation to prevent duplicate DidOpenTextDocument
            self.open_file_safely(&abs_path).await?;

            // For gopls, give it a moment to process the file and establish package context
            if self.is_gopls() {
                info!(
                    "Allowing gopls time to establish package context for {:?}",
                    abs_path
                );
                // Much shorter wait - let gopls work naturally like VS Code does
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        } else {
            // File is already open, just ensure it's current
            debug!("Document {:?} already open", abs_path);
        }
        Ok(())
    }

    // Main call hierarchy method with smart gopls handling
    pub async fn call_hierarchy(&self, file_path: &Path, line: u32, column: u32) -> Result<Value> {
        debug!(target: "lsp_call_hierarchy", "Starting call hierarchy for {:?} at {}:{}", 
            file_path, line, column);

        // For gopls, ensure document is open and ready
        if self.is_gopls() {
            self.ensure_document_ready(file_path).await?;
        }

        // For rust-analyzer, ensure document is open and wait for indexing
        if self.is_rust_analyzer() {
            // Open the document if not already open
            if !self.is_document_open(file_path).await {
                self.open_file_safely(file_path).await?;
                // rust-analyzer needs significant time to index after opening a file
                // especially when the project hasn't been fully indexed yet
                info!("Waiting 10 seconds for rust-analyzer to index the document and build call hierarchy...");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }

        // Try call hierarchy with retry logic for gopls and rust-analyzer
        let max_attempts = if self.is_gopls() || self.is_rust_analyzer() {
            3
        } else {
            1
        };
        let mut last_error = None;

        for attempt in 0..max_attempts {
            if attempt > 0 {
                debug!(
                    "Retrying call hierarchy (attempt {}/{})",
                    attempt + 1,
                    max_attempts
                );
                // Wait progressively longer between retries
                tokio::time::sleep(Duration::from_millis(500 * (attempt + 1) as u64)).await;

                // For gopls, ensure document is really open
                if self.is_gopls() {
                    self.ensure_document_ready(file_path).await?;
                }

                // For rust-analyzer, re-open document on retry
                if self.is_rust_analyzer() {
                    self.open_file_safely(file_path).await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }

            match self
                .perform_call_hierarchy_request(file_path, line, column)
                .await
            {
                Ok(result) => {
                    // Success! Clean up if needed
                    if self.is_gopls() && self.should_auto_close_documents() {
                        // We can optionally close the document later
                        // For now, keep it open for potential future requests
                    }
                    return Ok(result);
                }
                Err(e) => {
                    let error_str = e.to_string();

                    // Enhanced gopls error handling with comprehensive recovery
                    if self.is_gopls()
                        && (error_str.contains("no package metadata")
                            || error_str.contains("no package for file")
                            || error_str.contains("could not find package"))
                    {
                        warn!(
                            "gopls package metadata error for {:?} (attempt {}/{}): {}",
                            file_path,
                            attempt + 1,
                            max_attempts,
                            error_str
                        );
                        last_error = Some(e);

                        // Progressive recovery strategy
                        if attempt == 0 {
                            // First retry: Re-open the document and related files
                            info!("First retry: Re-establishing document context...");
                            // Force re-opening of package context
                            self.ensure_document_ready(file_path).await?;
                        } else if attempt == 1 {
                            // Second retry: Try workspace commands to refresh gopls state
                            info!("Second retry: Refreshing gopls workspace state...");

                            // Try workspace/symbol to force workspace indexing
                            let symbol_id = self.next_request_id().await;
                            if (self
                                .send_request(
                                    "workspace/symbol",
                                    json!({"query": "func"}),
                                    symbol_id,
                                )
                                .await)
                                .is_err()
                            {
                                debug!("Workspace symbol request failed during recovery");
                            }

                            // Try gopls-specific commands if available - use correct commands for v0.17.0
                            if (self.execute_command("gopls.workspace_stats", vec![]).await)
                                .is_err()
                            {
                                debug!("Workspace stats command failed or not available");
                            }

                            // Try gopls.views command which can help refresh workspace state
                            if (self.execute_command("gopls.views", vec![]).await).is_err() {
                                debug!("Views command failed or not available");
                            }

                            // Longer wait for gopls to rebuild metadata
                            tokio::time::sleep(Duration::from_secs(4)).await;
                        } else {
                            // Final retry: Give gopls more time to establish package metadata
                            info!("Final retry: Allowing more time for gopls package indexing...");

                            // Wait longer for gopls to naturally establish package context
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                        continue;
                    }

                    // For other errors or non-gopls servers, fail immediately
                    return Err(e);
                }
            }
        }

        // If we exhausted all retries, provide detailed error information
        let final_error = last_error
            .unwrap_or_else(|| anyhow!("Call hierarchy failed after {} attempts", max_attempts));

        if self.is_gopls() {
            error!(
                "GOPLS CALL HIERARCHY FAILED: {} attempts exhausted for {:?}. \
                This suggests gopls cannot establish package metadata for the file. \
                Ensure the file is part of a valid Go module with go.mod, \
                and the module is properly structured.",
                max_attempts, file_path
            );
        }

        Err(final_error)
    }

    // Helper to decide if we should auto-close documents
    fn should_auto_close_documents(&self) -> bool {
        // For now, keep documents open to avoid repeated open/close cycles
        false
    }

    /// Get text document definition
    pub async fn definition(&self, file_path: &Path, line: u32, column: u32) -> Result<Value> {
        let canon = Self::canonicalize_for_uri(file_path);
        let uri = Url::from_file_path(&canon)
            .map_err(|_| anyhow!("Invalid file path: {:?}", file_path))?;

        let request_id = self.next_request_id().await;
        let params = json!({
            "textDocument": {
                "uri": uri.to_string()
            },
            "position": {
                "line": line,
                "character": column
            }
        });

        self.send_request("textDocument/definition", params, request_id)
            .await?;
        let response = self
            .wait_for_response(request_id, Duration::from_secs(30))
            .await?;

        if let Some(error) = response.get("error") {
            return Err(anyhow!("Definition request failed: {:?}", error));
        }

        Ok(response["result"].clone())
    }

    /// Get text document references
    pub async fn references(
        &self,
        file_path: &Path,
        line: u32,
        column: u32,
        include_declaration: bool,
    ) -> Result<Value> {
        let canon = Self::canonicalize_for_uri(file_path);
        let uri = Url::from_file_path(&canon)
            .map_err(|_| anyhow!("Invalid file path: {:?}", file_path))?;

        let request_id = self.next_request_id().await;
        let params = json!({
            "textDocument": {
                "uri": uri.to_string()
            },
            "position": {
                "line": line,
                "character": column
            },
            "context": {
                "includeDeclaration": include_declaration
            }
        });

        self.send_request("textDocument/references", params, request_id)
            .await?;
        let response = self
            .wait_for_response(request_id, Duration::from_secs(30))
            .await?;

        if let Some(error) = response.get("error") {
            return Err(anyhow!("References request failed: {:?}", error));
        }

        Ok(response["result"].clone())
    }

    /// Get hover information
    pub async fn hover(&self, file_path: &Path, line: u32, column: u32) -> Result<Value> {
        let canon = Self::canonicalize_for_uri(file_path);
        let uri = Url::from_file_path(&canon)
            .map_err(|_| anyhow!("Invalid file path: {:?}", file_path))?;

        let request_id = self.next_request_id().await;
        let params = json!({
            "textDocument": {
                "uri": uri.to_string()
            },
            "position": {
                "line": line,
                "character": column
            }
        });

        self.send_request("textDocument/hover", params, request_id)
            .await?;
        let response = self
            .wait_for_response(request_id, Duration::from_secs(30))
            .await?;

        if let Some(error) = response.get("error") {
            return Err(anyhow!("Hover request failed: {:?}", error));
        }

        Ok(response["result"].clone())
    }

    // The actual call hierarchy request logic (extracted for retry)
    async fn perform_call_hierarchy_request(
        &self,
        file_path: &Path,
        line: u32,
        column: u32,
    ) -> Result<Value> {
        let canon = Self::canonicalize_for_uri(file_path);
        let uri =
            Url::from_file_path(&canon).map_err(|_| anyhow!("Failed to convert file path"))?;

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
            .map_err(|e| anyhow!("Call hierarchy prepare timed out: {}", e))?;

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

    // Ensure a workspace folder exists for the given path's module root (for gopls).
    async fn ensure_workspace_for_path(&self, file_path: &Path) -> Result<()> {
        if !self.is_gopls() {
            return Ok(());
        }

        // Determine module root for the file.
        let start_dir = if file_path.is_dir() {
            file_path.to_path_buf()
        } else {
            file_path.parent().unwrap_or(Path::new("")).to_path_buf()
        };
        let module_root = Self::find_go_module_root(&start_dir).unwrap_or(start_dir);
        if module_root.as_os_str().is_empty() {
            return Ok(());
        }
        let canonical_module = Self::canonicalize_for_uri(&module_root);

        let needs_add = match &self.project_root {
            Some(pr) => {
                // If file/module already within (canonical) project root, no need to add.
                !(Self::is_within(&canonical_module, pr) || Self::is_within(pr, &canonical_module))
            }
            None => true,
        };

        if needs_add {
            let uri = Url::from_directory_path(&canonical_module).map_err(|_| {
                anyhow!(
                    "Failed to create URI for module root: {:?}",
                    canonical_module
                )
            })?;
            let name = canonical_module
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace");
            let params = json!({
                "event": {
                    "added": [{ "uri": uri.to_string(), "name": name }],
                    "removed": []
                }
            });
            info!("Adding workspace folder for gopls: {:?}", canonical_module);
            self.send_notification("workspace/didChangeWorkspaceFolders", params)
                .await?;
            // Give gopls a short moment to incorporate the new view.
            tokio::time::sleep(Duration::from_millis(400)).await;
        }

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        tracing::debug!("Starting LSP server shutdown");

        // Absolute timeout for the entire shutdown process
        const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

        let shutdown_result = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
            let mut child_opt = self.child.lock().await;
            if let Some(ref mut child) = *child_opt {
                // Try graceful shutdown first
                let request_id = self.next_request_id().await;
                match self.send_request("shutdown", json!(null), request_id).await {
                    Ok(_) => {
                        // Wait briefly for shutdown response
                        match tokio::time::timeout(
                            Duration::from_secs(1),
                            self.wait_for_response(request_id, Duration::from_secs(1)),
                        )
                        .await
                        {
                            Ok(response_result) => match response_result {
                                Ok(_) => tracing::debug!("LSP shutdown response received"),
                                Err(e) => {
                                    tracing::warn!("LSP shutdown response error (continuing): {}", e)
                                }
                            },
                            Err(_) => {
                                tracing::warn!("Timeout waiting for LSP shutdown response (continuing)")
                            }
                        }

                        // Send exit notification
                        if let Err(e) = self.send_notification("exit", json!(null)).await {
                            tracing::warn!("Failed to send exit notification to LSP server: {}", e);
                        } else {
                            tracing::debug!("Exit notification sent to LSP server");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to send shutdown request to LSP server: {}", e);
                    }
                }

                // Give the process a moment to shut down gracefully
                tokio::time::sleep(Duration::from_millis(500)).await;

                // Force kill if still running
                match child.try_wait() {
                    Ok(Some(_status)) => {
                        tracing::debug!("LSP process exited gracefully");
                    }
                    Ok(None) => {
                        tracing::warn!("LSP process did not exit gracefully, force killing");
                        if let Err(e) = child.kill().await {
                            tracing::error!("Failed to kill LSP process: {}", e);
                        } else {
                            // Wait for process to actually die (with timeout)
                            // We need to poll try_wait() since wait() is blocking
                            let start = tokio::time::Instant::now();
                            let timeout = Duration::from_secs(5);

                            loop {
                                match child.try_wait() {
                                    Ok(Some(status)) => {
                                        tracing::debug!("LSP process killed with status: {}", status);
                                        break;
                                    }
                                    Ok(None) => {
                                        // Process still running
                                        if start.elapsed() >= timeout {
                                            tracing::error!(
                                                "Timeout waiting for LSP process to die after kill - process may still be running"
                                            );
                                            break;
                                        }
                                        // Sleep briefly before trying again
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                    }
                                    Err(e) => {
                                        tracing::error!("Error waiting for LSP process death: {}", e);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error checking LSP process status: {}", e);
                    }
                }

                // Ensure child is dropped
                *child_opt = None;
            }

            // Signal stderr thread to shutdown
            self.stderr_shutdown.store(true, Ordering::Relaxed);

            // Wait for stderr thread to finish (with timeout to avoid hanging)
            let mut stderr_handle_guard = self.stderr_thread.lock().await;
            if let Some(handle) = stderr_handle_guard.take() {
                drop(stderr_handle_guard); // Release lock before blocking operation

                tracing::debug!("Waiting for stderr thread to finish");
                // Wait with timeout to prevent hanging
                match tokio::time::timeout(Duration::from_secs(3), handle).await {
                    Ok(Ok(_)) => tracing::debug!("Stderr thread joined successfully"),
                    Ok(Err(e)) => tracing::error!("Error joining stderr thread: {:?}", e),
                    Err(_) => {
                        tracing::warn!("Timeout waiting for stderr thread to finish");
                        // Abort the task if it didn't finish
                        // Note: handle is consumed by timeout, so we can't abort here
                    }
                }
            } else {
                tracing::debug!("No stderr thread to cleanup (already cleaned up or never started)");
            }

            Ok::<(), anyhow::Error>(())
        })
        .await;

        match shutdown_result {
            Ok(Ok(())) => {
                tracing::debug!("LSP server shutdown complete");
                Ok(())
            }
            Ok(Err(e)) => {
                tracing::error!("Error during LSP server shutdown: {}", e);
                Err(e)
            }
            Err(_) => {
                tracing::error!(
                    "LSP server shutdown timed out after {} seconds - forcefully terminating",
                    SHUTDOWN_TIMEOUT.as_secs()
                );
                // At this point we've tried our best, return an error
                Err(anyhow::anyhow!(
                    "LSP server shutdown timed out after {} seconds",
                    SHUTDOWN_TIMEOUT.as_secs()
                ))
            }
        }
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

impl Drop for LspServer {
    fn drop(&mut self) {
        tracing::debug!("LspServer Drop implementation called");

        // Signal stderr thread to shutdown immediately - this is atomic and safe
        self.stderr_shutdown.store(true, Ordering::Relaxed);

        // Try to get stderr thread handle without blocking
        if let Ok(mut stderr_handle_guard) = self.stderr_thread.try_lock() {
            if let Some(handle) = stderr_handle_guard.take() {
                drop(stderr_handle_guard); // Release lock before potentially blocking operation

                // Abort the task - this is safe for tokio tasks
                handle.abort();
                tracing::debug!("Stderr task aborted successfully");
            } else {
                tracing::debug!("No stderr thread handle to cleanup (already cleaned up)");
            }
        } else {
            tracing::warn!(
                "Could not acquire stderr thread lock in Drop, thread may still be running"
            );
        }

        // Try to cleanup child process without blocking
        if let Ok(mut child_opt) = self.child.try_lock() {
            if let Some(mut child) = child_opt.take() {
                tracing::debug!("Forcefully killing child process in Drop");
                // Since we're in Drop (not async), we need to spawn a task to handle the async kill
                let child_id = child.id();

                // Spawn a task to handle async kill
                tokio::spawn(async move {
                    if let Err(e) = child.kill().await {
                        tracing::warn!(
                            "Failed to kill child process {} in background task: {}",
                            child_id.unwrap_or(0),
                            e
                        );
                    } else {
                        tracing::debug!(
                            "Child process {} killed successfully in background task",
                            child_id.unwrap_or(0)
                        );
                    }
                });
            } else {
                tracing::debug!("No child process to cleanup (already cleaned up)");
            }
        } else {
            tracing::warn!(
                "Could not acquire child process lock in Drop, process may still be running"
            );
        }

        tracing::debug!("LspServer Drop implementation complete - resources cleanup initiated");
    }
}
