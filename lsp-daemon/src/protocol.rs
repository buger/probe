use crate::language_detector::Language;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    Connect {
        client_id: Uuid,
    },
    // Workspace management
    InitializeWorkspace {
        request_id: Uuid,
        workspace_root: PathBuf,
        language: Option<Language>,
    },
    ListWorkspaces {
        request_id: Uuid,
    },
    // Health check endpoint for monitoring
    HealthCheck {
        request_id: Uuid,
    },
    // Analysis requests with optional workspace hints
    CallHierarchy {
        request_id: Uuid,
        file_path: PathBuf,
        line: u32,
        column: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_hint: Option<PathBuf>,
    },
    Definition {
        request_id: Uuid,
        file_path: PathBuf,
        line: u32,
        column: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_hint: Option<PathBuf>,
    },
    References {
        request_id: Uuid,
        file_path: PathBuf,
        line: u32,
        column: u32,
        include_declaration: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_hint: Option<PathBuf>,
    },
    Hover {
        request_id: Uuid,
        file_path: PathBuf,
        line: u32,
        column: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_hint: Option<PathBuf>,
    },
    Completion {
        request_id: Uuid,
        file_path: PathBuf,
        line: u32,
        column: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_hint: Option<PathBuf>,
    },
    // System requests
    Status {
        request_id: Uuid,
    },
    ListLanguages {
        request_id: Uuid,
    },
    Shutdown {
        request_id: Uuid,
    },
    Ping {
        request_id: Uuid,
    },
    GetLogs {
        request_id: Uuid,
        lines: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    Connected {
        request_id: Uuid,
        daemon_version: String,
    },
    // Workspace responses
    WorkspaceInitialized {
        request_id: Uuid,
        workspace_root: PathBuf,
        language: Language,
        lsp_server: String,
    },
    WorkspaceList {
        request_id: Uuid,
        workspaces: Vec<WorkspaceInfo>,
    },
    // Analysis responses
    CallHierarchy {
        request_id: Uuid,
        result: CallHierarchyResult,
    },
    Definition {
        request_id: Uuid,
        locations: Vec<Location>,
    },
    References {
        request_id: Uuid,
        locations: Vec<Location>,
    },
    Hover {
        request_id: Uuid,
        content: Option<HoverContent>,
    },
    Completion {
        request_id: Uuid,
        items: Vec<CompletionItem>,
    },
    // System responses
    Status {
        request_id: Uuid,
        status: DaemonStatus,
    },
    LanguageList {
        request_id: Uuid,
        languages: Vec<LanguageInfo>,
    },
    Shutdown {
        request_id: Uuid,
    },
    Pong {
        request_id: Uuid,
    },
    HealthCheck {
        request_id: Uuid,
        healthy: bool,
        uptime_seconds: u64,
        total_requests: usize,
        active_connections: usize,
        active_servers: usize,
        memory_usage_mb: f64,
    },
    Logs {
        request_id: Uuid,
        entries: Vec<LogEntry>,
    },
    Error {
        request_id: Uuid,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyResult {
    pub item: CallHierarchyItem,
    pub incoming: Vec<CallHierarchyCall>,
    pub outgoing: Vec<CallHierarchyCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: String,
    pub uri: String,
    pub range: Range,
    pub selection_range: Range,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyCall {
    pub from: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverContent {
    pub contents: String,
    pub range: Option<Range>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub kind: Option<CompletionItemKind>,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CompletionItemKind {
    Text = 1,
    Method = 2,
    Function = 3,
    Constructor = 4,
    Field = 5,
    Variable = 6,
    Class = 7,
    Interface = 8,
    Module = 9,
    Property = 10,
    Unit = 11,
    Value = 12,
    Enum = 13,
    Keyword = 14,
    Snippet = 15,
    Color = 16,
    File = 17,
    Reference = 18,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub uptime_secs: u64,
    pub pools: Vec<PoolStatus>,
    pub total_requests: u64,
    pub active_connections: usize,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub git_hash: String,
    #[serde(default)]
    pub build_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolStatus {
    pub language: Language,
    pub ready_servers: usize,
    pub busy_servers: usize,
    pub total_servers: usize,
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default)]
    pub uptime_secs: u64,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageInfo {
    pub language: Language,
    pub lsp_server: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub root: PathBuf,
    pub language: Language,
    pub server_status: ServerStatus,
    pub file_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerStatus {
    Initializing,
    Ready,
    Busy,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Trace => write!(f, "TRACE"),
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

pub struct MessageCodec;

impl MessageCodec {
    pub fn encode(msg: &DaemonRequest) -> Result<Vec<u8>> {
        let json = serde_json::to_string(msg)?;
        let bytes = json.as_bytes();

        // Simple length-prefixed encoding
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        encoded.extend_from_slice(bytes);

        Ok(encoded)
    }

    pub fn encode_response(msg: &DaemonResponse) -> Result<Vec<u8>> {
        let json = serde_json::to_string(msg)?;
        let bytes = json.as_bytes();

        let mut encoded = Vec::new();
        encoded.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        encoded.extend_from_slice(bytes);

        Ok(encoded)
    }

    pub fn decode_request(bytes: &[u8]) -> Result<DaemonRequest> {
        // Maximum message size: 10MB (must match daemon.rs)
        const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

        if bytes.len() < 4 {
            return Err(anyhow::anyhow!("Message too short"));
        }

        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;

        // Validate message size to prevent excessive memory allocation
        if len > MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!(
                "Message size {} exceeds maximum allowed size of {} bytes",
                len,
                MAX_MESSAGE_SIZE
            ));
        }

        if bytes.len() < 4 + len {
            return Err(anyhow::anyhow!("Incomplete message"));
        }

        let json_bytes = &bytes[4..4 + len];
        let request = serde_json::from_slice(json_bytes)?;

        Ok(request)
    }

    pub fn decode_response(bytes: &[u8]) -> Result<DaemonResponse> {
        // Maximum message size: 10MB (must match daemon.rs)
        const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

        if bytes.len() < 4 {
            return Err(anyhow::anyhow!("Message too short"));
        }

        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;

        // Validate message size to prevent excessive memory allocation
        if len > MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!(
                "Message size {} exceeds maximum allowed size of {} bytes",
                len,
                MAX_MESSAGE_SIZE
            ));
        }

        if bytes.len() < 4 + len {
            return Err(anyhow::anyhow!("Incomplete message"));
        }

        let json_bytes = &bytes[4..4 + len];
        let response = serde_json::from_slice(json_bytes)?;

        Ok(response)
    }
}

// Helper function to convert from serde_json::Value to our types
pub fn parse_call_hierarchy_from_lsp(value: &Value) -> Result<CallHierarchyResult> {
    // Handle case where rust-analyzer returns empty call hierarchy (no item)
    let item = match value.get("item") {
        Some(item) => item,
        None => {
            // Return empty call hierarchy result
            return Ok(CallHierarchyResult {
                item: CallHierarchyItem {
                    name: "unknown".to_string(),
                    kind: "unknown".to_string(),
                    uri: "".to_string(),
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                    selection_range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                },
                incoming: vec![],
                outgoing: vec![],
            });
        }
    };

    let incoming = value
        .get("incoming")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| parse_call_hierarchy_call(v).ok())
                .collect()
        })
        .unwrap_or_default();

    let outgoing = value
        .get("outgoing")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| parse_call_hierarchy_call(v).ok())
                .collect()
        })
        .unwrap_or_default();

    Ok(CallHierarchyResult {
        item: parse_call_hierarchy_item(item)?,
        incoming,
        outgoing,
    })
}

fn parse_call_hierarchy_item(value: &Value) -> Result<CallHierarchyItem> {
    Ok(CallHierarchyItem {
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        kind: value
            .get("kind")
            .and_then(|v| v.as_u64())
            .map(|k| k.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        uri: value
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        range: parse_range(value.get("range").unwrap_or(&json!({})))?,
        selection_range: parse_range(value.get("selectionRange").unwrap_or(&json!({})))?,
    })
}

fn parse_call_hierarchy_call(value: &Value) -> Result<CallHierarchyCall> {
    // For incoming calls, use "from" field
    // For outgoing calls, use "to" field (rename it to "from" for consistency)
    let from = value
        .get("from")
        .or_else(|| value.get("to"))
        .ok_or_else(|| anyhow::anyhow!("Missing 'from' or 'to' in call"))?;

    let from_ranges = value
        .get("fromRanges")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|r| parse_range(r).ok()).collect())
        .unwrap_or_default();

    Ok(CallHierarchyCall {
        from: parse_call_hierarchy_item(from)?,
        from_ranges,
    })
}

fn parse_range(value: &Value) -> Result<Range> {
    let default_pos = json!({});
    let start = value.get("start").unwrap_or(&default_pos);
    let end = value.get("end").unwrap_or(&default_pos);

    Ok(Range {
        start: Position {
            line: start.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            character: start.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        },
        end: Position {
            line: end.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            character: end.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        },
    })
}

use serde_json::json;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_message_codec_large_response() {
        // Create a large response with many log entries
        let mut large_log_entries = Vec::new();
        for i in 0..100 {
            large_log_entries.push(LogEntry {
                timestamp: format!("2024-01-01 12:00:{:02}.000 UTC", i % 60),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Large message {} with lots of content that makes the overall response quite big", i),
                file: Some("test.rs".to_string()),
                line: Some(i),
            });
        }

        let response = DaemonResponse::Logs {
            request_id: Uuid::new_v4(),
            entries: large_log_entries,
        };

        // Encode the response
        let encoded =
            MessageCodec::encode_response(&response).expect("Failed to encode large response");

        // Ensure it's properly encoded with length prefix
        assert!(encoded.len() >= 4);
        let expected_len = encoded.len() - 4;
        let actual_len =
            u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize;
        assert_eq!(actual_len, expected_len);

        // Decode it back
        let decoded =
            MessageCodec::decode_response(&encoded).expect("Failed to decode large response");

        match decoded {
            DaemonResponse::Logs { entries, .. } => {
                assert_eq!(entries.len(), 100);
                assert_eq!(entries[0].message, "Large message 0 with lots of content that makes the overall response quite big");
            }
            _ => panic!("Expected Logs response"),
        }
    }

    #[test]
    fn test_incomplete_message_detection() {
        // Create a normal response
        let response = DaemonResponse::Pong {
            request_id: Uuid::new_v4(),
        };

        let encoded = MessageCodec::encode_response(&response).expect("Failed to encode");

        // Test with truncated message (missing some bytes)
        let truncated = &encoded[..encoded.len() - 5];
        let result = MessageCodec::decode_response(truncated);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Incomplete message"));
    }

    #[test]
    fn test_message_too_short() {
        // Test with message shorter than 4 bytes
        let short_message = vec![1, 2];
        let result = MessageCodec::decode_response(&short_message);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Message too short"));
    }
}
