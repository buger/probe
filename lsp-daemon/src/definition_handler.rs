// LSP Definition handler implementation
use crate::cache_types::{DefinitionInfo, LocationInfo, LspCacheKey, LspOperation, RangeInfo};
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

/// Handle textDocument/definition request with caching
#[allow(clippy::too_many_arguments)]
pub async fn handle_definition(
    definition_cache: &Arc<LspCache<DefinitionInfo>>,
    server_manager: &Arc<SingleServerManager>,
    workspace_resolver: &Arc<Mutex<WorkspaceResolver>>,
    file_path: &Path,
    line: u32,
    column: u32,
    language: Language,
    workspace_hint: Option<PathBuf>,
) -> Result<Vec<LocationInfo>> {
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
        LspOperation::Definition,
        None,
    );

    if let Some(cached) = definition_cache.get(&cache_key).await {
        info!(
            "Definition cache HIT for {:?} at {}:{}",
            absolute_file_path, line, column
        );
        return Ok(cached.locations);
    }

    info!(
        "Definition cache MISS for {:?} at {}:{}",
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
        .definition(&absolute_file_path, line, column)
        .await?;

    // Parse response
    let locations = parse_definition_response(response)?;

    // Cache the result
    let definition_info = DefinitionInfo {
        locations: locations.clone(),
    };
    definition_cache.insert(cache_key, definition_info).await;

    Ok(locations)
}

/// Parse LSP definition response into LocationInfo
fn parse_definition_response(response: serde_json::Value) -> Result<Vec<LocationInfo>> {
    let mut locations = Vec::new();

    // Handle different response formats
    if response.is_null() {
        return Ok(locations);
    }

    // Response can be Location | Location[] | LocationLink[]
    let items = if response.is_array() {
        response.as_array().unwrap().clone()
    } else {
        vec![response]
    };

    for item in items {
        if let Some(uri) = item.get("uri").and_then(|u| u.as_str()) {
            let range = item
                .get("range")
                .ok_or_else(|| anyhow!("Missing range in location"))?;
            let start = range
                .get("start")
                .ok_or_else(|| anyhow!("Missing start in range"))?;
            let end = range
                .get("end")
                .ok_or_else(|| anyhow!("Missing end in range"))?;

            locations.push(LocationInfo {
                uri: uri.to_string(),
                range: RangeInfo {
                    start_line: start.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                    start_character: start.get("character").and_then(|c| c.as_u64()).unwrap_or(0)
                        as u32,
                    end_line: end.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                    end_character: end.get("character").and_then(|c| c.as_u64()).unwrap_or(0)
                        as u32,
                },
            });
        } else if let Some(target_uri) = item.get("targetUri").and_then(|u| u.as_str()) {
            // Handle LocationLink format
            let range = item
                .get("targetRange")
                .ok_or_else(|| anyhow!("Missing targetRange in LocationLink"))?;
            let start = range
                .get("start")
                .ok_or_else(|| anyhow!("Missing start in range"))?;
            let end = range
                .get("end")
                .ok_or_else(|| anyhow!("Missing end in range"))?;

            locations.push(LocationInfo {
                uri: target_uri.to_string(),
                range: RangeInfo {
                    start_line: start.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                    start_character: start.get("character").and_then(|c| c.as_u64()).unwrap_or(0)
                        as u32,
                    end_line: end.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                    end_character: end.get("character").and_then(|c| c.as_u64()).unwrap_or(0)
                        as u32,
                },
            });
        }
    }

    Ok(locations)
}
