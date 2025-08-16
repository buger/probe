use crate::language_detector::Language;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

/// Shared limit for length-prefixed messages (also used by daemon).
pub const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

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
    InitWorkspaces {
        request_id: Uuid,
        workspace_root: PathBuf,
        languages: Option<Vec<Language>>,
        recursive: bool,
        enable_watchdog: bool,
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
    DocumentSymbols {
        request_id: Uuid,
        file_path: PathBuf,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_hint: Option<PathBuf>,
    },
    WorkspaceSymbols {
        request_id: Uuid,
        query: String,
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
    WorkspacesInitialized {
        request_id: Uuid,
        initialized: Vec<InitializedWorkspace>,
        errors: Vec<String>,
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
        #[serde(default)]
        lsp_server_health: Vec<LspServerHealthInfo>,
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
pub struct LspServerHealthInfo {
    pub language: Language,
    pub is_healthy: bool,
    pub consecutive_failures: u32,
    pub circuit_breaker_open: bool,
    pub last_check_ms: u64,
    pub response_time_ms: u64,
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
    #[serde(default)]
    pub health_status: String,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub circuit_breaker_open: bool,
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
pub struct InitializedWorkspace {
    pub workspace_root: PathBuf,
    pub language: Language,
    pub lsp_server: String,
    pub status: String,
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
        // Maximum message size is shared with the daemon (see MAX_MESSAGE_SIZE).

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
        // Maximum message size is shared with the daemon (see MAX_MESSAGE_SIZE).

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

// Small helper to build a default/empty CallHierarchyItem
fn default_call_hierarchy_item() -> CallHierarchyItem {
    CallHierarchyItem {
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
    }
}

// Helper function to convert from serde_json::Value to our types
pub fn parse_call_hierarchy_from_lsp(value: &Value) -> Result<CallHierarchyResult> {
    // Accept alternative shapes: when LSP returns an array (prepare call result),
    // take the first element as the root item and leave incoming/outgoing empty.
    if let Some(arr) = value.as_array() {
        if let Some(first) = arr.first() {
            return Ok(CallHierarchyResult {
                item: parse_call_hierarchy_item(first)?,
                incoming: vec![],
                outgoing: vec![],
            });
        } else {
            return Ok(CallHierarchyResult {
                item: default_call_hierarchy_item(),
                incoming: vec![],
                outgoing: vec![],
            });
        }
    }
    // Handle case where rust-analyzer returns empty call hierarchy (no item)
    let item = match value.get("item") {
        Some(item) => item,
        None => {
            // Return empty call hierarchy result
            return Ok(CallHierarchyResult {
                item: default_call_hierarchy_item(),
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
        // Accept numeric or string kinds
        kind: match value.get("kind") {
            Some(kv) => {
                if let Some(num) = kv.as_u64() {
                    num.to_string()
                } else {
                    kv.as_str().unwrap_or("unknown").to_string()
                }
            }
            None => "unknown".to_string(),
        },
        // Accept targetUri as a fallback
        uri: value
            .get("uri")
            .or_else(|| value.get("targetUri"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        range: parse_range(value.get("range").unwrap_or(&json!({})))?,
        selection_range: parse_range(
            value
                .get("selectionRange")
                .or_else(|| value.get("range"))
                .unwrap_or(&json!({})),
        )?,
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
        .or_else(|| value.get("toRanges"))
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
                message: format!("Large message {i} with lots of content that makes the overall response quite big"),
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

    #[test]
    fn test_message_codec_large_request() {
        // Create a large request (GetLogs), encode and decode it
        let request = DaemonRequest::GetLogs {
            request_id: Uuid::new_v4(),
            lines: 1000,
        };
        let encoded = MessageCodec::encode(&request).expect("encode");
        let decoded = MessageCodec::decode_request(&encoded).expect("decode");
        match decoded {
            DaemonRequest::GetLogs { lines, .. } => assert_eq!(lines, 1000),
            _ => panic!("expected GetLogs"),
        }
    }

    #[test]
    fn test_parse_call_hierarchy_accepts_string_kind_and_to_ranges() {
        let v = serde_json::json!({
            "item": {
                "name": "root",
                "kind": "Function",
                "uri": "file:///root.rs",
                "range": { "start": {"line":1, "character":2}, "end": {"line":1, "character":10} },
                "selectionRange": { "start": {"line":1, "character":2}, "end": {"line":1, "character":10} }
            },
            "incoming": [{
                "from": {
                    "name": "caller",
                    "kind": "Method",
                    "uri": "file:///caller.rs",
                    "range": { "start": {"line":0, "character":0}, "end": {"line":0, "character":1} },
                    "selectionRange": { "start": {"line":0, "character":0}, "end": {"line":0, "character":1} }
                },
                "fromRanges": [ { "start": {"line":0, "character":0}, "end": {"line":0, "character":1} } ]
            }],
            "outgoing": [{
                "to": {
                    "name": "callee",
                    "kind": 12,
                    "targetUri": "file:///callee.rs",
                    "range": { "start": {"line":2, "character":0}, "end": {"line":2, "character":1} },
                    "selectionRange": { "start": {"line":2, "character":0}, "end": {"line":2, "character":1} }
                },
                "toRanges": [ { "start": {"line":2, "character":0}, "end": {"line":2, "character":1} } ]
            }]
        });
        let result = parse_call_hierarchy_from_lsp(&v).expect("parse ok");
        assert_eq!(result.item.kind, "Function");
        assert_eq!(result.incoming.len(), 1);
        assert_eq!(result.outgoing.len(), 1);
        assert_eq!(result.outgoing[0].from.kind, "12");
        assert_eq!(result.outgoing[0].from.uri, "file:///callee.rs");
        assert_eq!(result.outgoing[0].from_ranges.len(), 1);
    }

    #[test]
    fn test_parse_call_hierarchy_array_item_defaults() {
        let v = serde_json::json!([{
            "name": "root",
            "kind": 3,
            "uri": "file:///root.rs",
            "range": { "start": {"line":3, "character":0}, "end": {"line":3, "character":5} }
        }]);
        let result = parse_call_hierarchy_from_lsp(&v).expect("parse");
        assert_eq!(result.item.name, "root");
        assert!(result.incoming.is_empty());
        assert!(result.outgoing.is_empty());
    }
}
