// LSP Hover handler implementation
use crate::cache_types::{HoverInfo, LspCacheKey, LspOperation, RangeInfo};
use crate::hash_utils::md5_hex_file;
use crate::language_detector::Language;
use crate::lsp_cache::LspCache;
use crate::server_manager::SingleServerManager;
use crate::workspace_resolver::WorkspaceResolver;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Handle textDocument/hover request with caching
#[allow(clippy::too_many_arguments)]
pub async fn handle_hover(
    hover_cache: &Arc<LspCache<HoverInfo>>,
    server_manager: &Arc<SingleServerManager>,
    workspace_resolver: &Arc<Mutex<WorkspaceResolver>>,
    file_path: &Path,
    line: u32,
    column: u32,
    language: Language,
    workspace_hint: Option<PathBuf>,
) -> Result<Option<HoverInfo>> {
    // Get absolute path
    let absolute_file_path = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());

    // Check cache first
    let content_md5 = md5_hex_file(&absolute_file_path)?;
    let cache_key = LspCacheKey::new(
        absolute_file_path.clone(),
        line,
        column,
        content_md5.clone(),
        LspOperation::Hover,
        None,
    );

    if let Some(cached) = hover_cache.get(&cache_key).await {
        info!(
            "Hover cache HIT for {:?} at {}:{}",
            absolute_file_path, line, column
        );
        return Ok(Some(cached));
    }

    info!(
        "Hover cache MISS for {:?} at {}:{}",
        absolute_file_path, line, column
    );

    // Resolve workspace
    let workspace_root = {
        let mut resolver = workspace_resolver.lock().await;
        resolver.resolve_workspace(&absolute_file_path, workspace_hint)?
    };

    // Get or create server
    let server_instance = server_manager
        .ensure_workspace_registered(language, workspace_root.clone())
        .await?;

    let server = server_instance.lock().await;

    // Send request to LSP server
    let response = server
        .server
        .hover(&absolute_file_path, line, column)
        .await?;

    // Parse response
    let hover_info = parse_hover_response(response)?;

    // Cache the result if we got something
    if let Some(ref info) = hover_info {
        hover_cache.insert(cache_key, info.clone()).await;
    }

    Ok(hover_info)
}

/// Parse LSP hover response into HoverInfo
fn parse_hover_response(response: serde_json::Value) -> Result<Option<HoverInfo>> {
    if response.is_null() {
        return Ok(None);
    }

    let contents = response
        .get("contents")
        .ok_or_else(|| anyhow!("Missing contents in hover response"))?;

    // Parse contents - can be string, MarkedString, or MarkedString[]
    let mut content_parts = Vec::new();

    if let Some(s) = contents.as_str() {
        content_parts.push(s.to_string());
    } else if let Some(arr) = contents.as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                content_parts.push(s.to_string());
            } else if let Some(value) = item.get("value").and_then(|v| v.as_str()) {
                // MarkedString with language
                let lang = item.get("language").and_then(|l| l.as_str()).unwrap_or("");
                content_parts.push(format!("```{lang}\n{value}\n```"));
            }
        }
    } else if let Some(value) = contents.get("value").and_then(|v| v.as_str()) {
        // Single MarkedString
        let lang = contents
            .get("language")
            .and_then(|l| l.as_str())
            .unwrap_or("");
        content_parts.push(format!("```{lang}\n{value}\n```"));
    } else if contents.get("kind").and_then(|k| k.as_str()) == Some("markdown") {
        // MarkupContent
        if let Some(value) = contents.get("value").and_then(|v| v.as_str()) {
            content_parts.push(value.to_string());
        }
    }

    if content_parts.is_empty() {
        return Ok(None);
    }

    // Parse range if present
    let range = response.get("range").and_then(|r| {
        let start = r.get("start")?;
        let end = r.get("end")?;

        Some(RangeInfo {
            start_line: start.get("line").and_then(|l| l.as_u64())? as u32,
            start_character: start.get("character").and_then(|c| c.as_u64())? as u32,
            end_line: end.get("line").and_then(|l| l.as_u64())? as u32,
            end_character: end.get("character").and_then(|c| c.as_u64())? as u32,
        })
    });

    Ok(Some(HoverInfo {
        contents: if content_parts.is_empty() {
            None
        } else {
            Some(content_parts.join("\n\n"))
        },
        range,
    }))
}
