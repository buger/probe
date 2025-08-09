use anyhow::{anyhow, Result};
use lsp_daemon::start_daemon_background;
use lsp_daemon::{get_default_socket_path, IpcStream};
use lsp_daemon::{
    CallHierarchyResult, DaemonRequest, DaemonResponse, DaemonStatus, LanguageInfo, MessageCodec,
};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, timeout};
use tracing::{debug, info};
use uuid::Uuid;

pub struct LspClient {
    stream: Option<IpcStream>,
    auto_start_daemon: bool,
}

impl LspClient {
    pub async fn new(auto_start: bool) -> Result<Self> {
        let mut client = Self {
            stream: None,
            auto_start_daemon: auto_start,
        };

        client.connect().await?;
        Ok(client)
    }

    pub async fn connect(&mut self) -> Result<()> {
        let socket_path = get_default_socket_path();

        // Try to connect to existing daemon
        match IpcStream::connect(&socket_path).await {
            Ok(stream) => {
                info!("Connected to existing daemon");
                self.stream = Some(stream);

                // Send connect message
                let request = DaemonRequest::Connect {
                    client_id: Uuid::new_v4(),
                };
                let response = self.send_request(request).await?;

                if let DaemonResponse::Connected { daemon_version, .. } = response {
                    debug!("Connected to daemon version: {}", daemon_version);
                }

                return Ok(());
            }
            Err(e) => {
                debug!("Failed to connect to daemon: {}", e);
            }
        }

        // Auto-start daemon if enabled
        if self.auto_start_daemon {
            info!("Starting daemon...");
            start_daemon_background().await?;

            // Wait for daemon to be ready with exponential backoff
            for attempt in 0..10 {
                sleep(Duration::from_millis(100 * 2_u64.pow(attempt))).await;

                if let Ok(stream) = IpcStream::connect(&socket_path).await {
                    info!("Connected to newly started daemon");
                    self.stream = Some(stream);

                    // Send connect message
                    let request = DaemonRequest::Connect {
                        client_id: Uuid::new_v4(),
                    };
                    let response = self.send_request(request).await?;

                    if let DaemonResponse::Connected { daemon_version, .. } = response {
                        debug!("Connected to daemon version: {}", daemon_version);
                    }

                    return Ok(());
                }
            }

            return Err(anyhow!("Failed to connect to daemon after starting"));
        }

        Err(anyhow!("Daemon not running and auto-start disabled"))
    }

    async fn send_request(&mut self, request: DaemonRequest) -> Result<DaemonResponse> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| anyhow!("Not connected to daemon"))?;

        // Encode and send request
        let encoded = MessageCodec::encode(&request)?;
        stream.write_all(&encoded).await?;
        stream.flush().await?;

        // Read response with timeout
        let mut buffer = vec![0; 65536];
        let n = timeout(Duration::from_secs(90), stream.read(&mut buffer)).await??; // Increased for rust-analyzer

        if n == 0 {
            return Err(anyhow!("Connection closed by daemon"));
        }

        // Decode response
        let response = MessageCodec::decode_response(&buffer[..n])?;

        // Check for errors
        if let DaemonResponse::Error { error, .. } = &response {
            return Err(anyhow!("Daemon error: {}", error));
        }

        Ok(response)
    }

    pub async fn call_hierarchy(
        &mut self,
        file_path: &Path,
        line: u32,
        column: u32,
    ) -> Result<CallHierarchyResult> {
        let request = DaemonRequest::CallHierarchy {
            request_id: Uuid::new_v4(),
            file_path: file_path.to_path_buf(),
            line,
            column,
            workspace_hint: None,
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::CallHierarchy { result, .. } => Ok(result),
            DaemonResponse::Error { error, .. } => Err(anyhow!("Call hierarchy failed: {}", error)),
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

    pub async fn get_status(&mut self) -> Result<DaemonStatus> {
        let request = DaemonRequest::Status {
            request_id: Uuid::new_v4(),
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::Status { status, .. } => Ok(status),
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

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

    pub async fn shutdown_daemon(&mut self) -> Result<()> {
        let request = DaemonRequest::Shutdown {
            request_id: Uuid::new_v4(),
        };

        let response = self.send_request(request).await?;

        match response {
            DaemonResponse::Shutdown { .. } => {
                info!("Daemon shutdown acknowledged");
                self.stream = None;
                Ok(())
            }
            _ => Err(anyhow!("Unexpected response type")),
        }
    }

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
}

// Fallback implementation for direct LSP communication (without daemon)
pub struct DirectLspClient;

impl DirectLspClient {
    pub async fn call_hierarchy(file_path: &Path, pattern: &str) -> Result<CallHierarchyResult> {
        eprintln!(
            "DirectLspClient::call_hierarchy called with file: {:?}, pattern: {}",
            file_path, pattern
        );
        use lsp_daemon::lsp_registry::LspRegistry;
        use lsp_daemon::lsp_server::LspServer;
        use lsp_daemon::parse_call_hierarchy_from_lsp;
        use lsp_daemon::{Language, LanguageDetector};
        use std::fs;

        // Detect language
        let detector = LanguageDetector::new();
        let language = detector.detect(file_path)?;

        if language == Language::Unknown {
            return Err(anyhow!("Unknown language for file: {:?}", file_path));
        }

        // Get LSP server config
        let registry = LspRegistry::new()?;
        let config = registry
            .get(language)
            .ok_or_else(|| anyhow!("No LSP server configured for {:?}", language))?;

        // Spawn and initialize server
        let mut server = LspServer::spawn(config)?;
        eprintln!("About to call initialize...");
        server.initialize(config).await?;
        eprintln!("Initialization complete, proceeding immediately with call hierarchy...");

        // Read file content
        let content = fs::read_to_string(file_path)?;

        // Find pattern position
        let (line, column) = find_pattern_position(&content, pattern)
            .ok_or_else(|| anyhow!("Pattern '{}' not found in file", pattern))?;

        eprintln!(
            "Found pattern '{}' at line {}, column {}",
            pattern, line, column
        );

        // Open document
        server.open_document(file_path, &content).await?;
        eprintln!("Document opened, requesting call hierarchy...");

        // Get call hierarchy
        let result = server.call_hierarchy(file_path, line, column).await?;
        eprintln!("Call hierarchy received!");

        // Close document and shutdown
        server.close_document(file_path).await?;
        server.shutdown().await?;

        // Parse result
        parse_call_hierarchy_from_lsp(&result)
    }
}

fn find_pattern_position(content: &str, pattern: &str) -> Option<(u32, u32)> {
    for (line_idx, line) in content.lines().enumerate() {
        if let Some(col_idx) = line.find(pattern) {
            let char_col = line[..col_idx].chars().count() as u32;
            return Some((line_idx as u32, char_col));
        }
    }
    None
}
