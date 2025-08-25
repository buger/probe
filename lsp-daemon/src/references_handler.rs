// LSP References handler implementation
use crate::cache_types::{LocationInfo, LspCacheKey, LspOperation, RangeInfo, ReferencesInfo};
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

/// Handle textDocument/references request with caching
#[allow(clippy::too_many_arguments)]
pub async fn handle_references(
    references_cache: &Arc<LspCache<ReferencesInfo>>,
    server_manager: &Arc<SingleServerManager>,
    workspace_resolver: &Arc<Mutex<WorkspaceResolver>>,
    file_path: &Path,
    line: u32,
    column: u32,
    include_declaration: bool,
    language: Language,
    workspace_hint: Option<PathBuf>,
) -> Result<Vec<LocationInfo>> {
    // Get absolute path
    let absolute_file_path = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());

    // Create cache key with include_declaration flag
    let content_md5 = md5_hex_file(&absolute_file_path)?;
    let cache_key = LspCacheKey::new(
        absolute_file_path.clone(),
        line,
        column,
        content_md5.clone(),
        LspOperation::References,
        if include_declaration {
            Some("include_decl".to_string())
        } else {
            None
        },
    );

    if let Some(cached) = references_cache.get(&cache_key).await {
        info!(
            "References cache HIT for {:?} at {}:{} (include_decl: {})",
            absolute_file_path, line, column, include_declaration
        );
        return Ok(cached.locations);
    }

    info!(
        "References cache MISS for {:?} at {}:{} (include_decl: {})",
        absolute_file_path, line, column, include_declaration
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
        .references(&absolute_file_path, line, column, include_declaration)
        .await?;

    // Parse response
    let locations = parse_references_response(response)?;

    // Cache the result
    let references_info = ReferencesInfo {
        locations: locations.clone(),
        include_declaration,
    };
    references_cache.insert(cache_key, references_info).await;

    Ok(locations)
}

/// Parse LSP references response into LocationInfo
fn parse_references_response(response: serde_json::Value) -> Result<Vec<LocationInfo>> {
    let mut locations = Vec::new();

    // Response should be Location[] or null
    if response.is_null() {
        return Ok(locations);
    }

    let items = response
        .as_array()
        .ok_or_else(|| anyhow!("Expected array in references response"))?;

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
        }
    }

    Ok(locations)
}
