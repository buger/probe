use anyhow::Result;
use probe_code::language::factory::get_language_impl;
use probe_code::language::parser_pool::{get_pooled_parser, return_pooled_parser};
use probe_code::lsp_integration::{LspClient, LspConfig};
use probe_code::models::SearchResult;
use rayon::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

// Type alias for the cache key
type CacheKey = (String, String, u32, u32);
// Type alias for the cache value
type CacheValue = Arc<serde_json::Value>;
// Type alias for the cache map
type CacheMap = HashMap<CacheKey, CacheValue>;

// Global cache for LSP results to avoid redundant calls
lazy_static::lazy_static! {
    static ref LSP_CACHE: Arc<Mutex<CacheMap>> =
        Arc::new(Mutex::new(HashMap::new()));
}

/// Enrich search results with LSP information
/// This function processes search results in parallel and adds LSP information
/// for functions, methods, and other symbols found in the code blocks.
pub fn enrich_results_with_lsp(results: &mut Vec<SearchResult>, debug_mode: bool) -> Result<()> {
    if debug_mode {
        println!(
            "[DEBUG] Starting LSP enrichment for {} results",
            results.len()
        );
    }

    // Process results in parallel
    results.par_iter_mut().for_each(|result| {
        // Skip if we already have LSP info
        if result.lsp_info.is_some() {
            return;
        }

        // Try to extract symbol information from the code block with precise position
        if let Some(symbol_info) = extract_symbol_from_code_block_with_position(result, debug_mode)
        {
            // Check cache first
            let cache_key = (
                result.file.clone(),
                symbol_info.name.clone(),
                symbol_info.line,
                symbol_info.column,
            );

            let cached_value = {
                if let Ok(cache) = LSP_CACHE.lock() {
                    cache.get(&cache_key).cloned()
                } else {
                    None
                }
            };

            if let Some(cached) = cached_value {
                if debug_mode {
                    println!(
                        "[DEBUG] Using cached LSP info for {} at {}:{}:{}",
                        symbol_info.name, result.file, symbol_info.line, symbol_info.column
                    );
                }
                result.lsp_info = Some((*cached).clone());
            } else {
                // Get LSP information for the symbol
                let lsp_info = get_lsp_info_for_result(
                    &result.file,
                    &symbol_info.name,
                    symbol_info.line,
                    symbol_info.column,
                    debug_mode,
                );

                // Cache the result if successful
                if let Some(ref info) = lsp_info {
                    if let Ok(mut cache) = LSP_CACHE.lock() {
                        cache.insert(cache_key, Arc::new(info.clone()));
                    }
                }

                result.lsp_info = lsp_info;
            }
        }
    });

    if debug_mode {
        let enriched_count = results.iter().filter(|r| r.lsp_info.is_some()).count();
        println!(
            "[DEBUG] LSP enrichment complete: {}/{} results enriched",
            enriched_count,
            results.len()
        );

        // Print cache statistics
        if let Ok(cache) = LSP_CACHE.lock() {
            println!("[DEBUG] LSP cache size: {} entries", cache.len());
        }
    }

    Ok(())
}

/// Information about a symbol extracted from a code block
struct SymbolInfo {
    name: String,
    line: u32,
    column: u32,
}

/// Extract symbol information from a code block with precise position using tree-sitter
/// This function parses the code block to find the exact position of the symbol.
fn extract_symbol_from_code_block_with_position(
    result: &SearchResult,
    debug_mode: bool,
) -> Option<SymbolInfo> {
    // For function and method node types, try to extract the name
    let is_function_like = matches!(
        result.node_type.as_str(),
        "function_item"
            | "function_definition"
            | "method_definition"
            | "function_declaration"
            | "method_declaration"
            | "function"
            | "method"
            | "class_definition"
            | "struct_item"
            | "impl_item"
            | "trait_item"
            | "file"
            | "import"
            | "code" // Also check common fallback node types
    );

    if debug_mode {
        println!(
            "[DEBUG] Checking node_type '{}' for symbol extraction (is_function_like: {})",
            result.node_type, is_function_like
        );
    }

    // For non-function-like node types, still try to extract if the code looks like it contains a function
    if !is_function_like {
        // Check if the code block contains function-like patterns
        let code_contains_function = result.code.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("pub fn ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("def ")
                || trimmed.starts_with("function ")
                || trimmed.starts_with("func ")
                || trimmed.starts_with("class ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("impl ")
        });

        if !code_contains_function {
            if debug_mode {
                println!(
                    "[DEBUG] Skipping non-function-like block with node_type: {}",
                    result.node_type
                );
            }
            return None;
        }

        if debug_mode {
            println!(
                "[DEBUG] Found function-like code in block with node_type: {}",
                result.node_type
            );
        }
    }

    // Find the first line that looks like a function/method/class definition
    // Skip doc comments (///, //), attributes (#[...]), and regular comments
    let mut function_line = None;
    for line in result.code.lines() {
        let trimmed = line.trim();
        // Skip comments and attributes
        if trimmed.starts_with("///")
            || trimmed.starts_with("//")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("#!")
            || trimmed.is_empty()
        {
            continue;
        }
        // Check if this line looks like a function/method/class definition
        if trimmed.starts_with("pub fn ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("pub async fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
            || trimmed.starts_with("function ")
            || trimmed.starts_with("func ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("pub struct ")
            || trimmed.starts_with("impl ")
            || trimmed.starts_with("pub impl ")
            || trimmed.starts_with("trait ")
            || trimmed.starts_with("pub trait ")
            || trimmed.starts_with("interface ")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("pub type ")
            || trimmed.starts_with("const ")
            || trimmed.starts_with("pub const ")
            || trimmed.starts_with("static ")
            || trimmed.starts_with("pub static ")
            || trimmed.starts_with("let ")
            || trimmed.starts_with("var ")
            || trimmed.starts_with("export ")
            || trimmed.starts_with("public ")
            || trimmed.starts_with("private ")
            || trimmed.starts_with("protected ")
        {
            function_line = Some(line);
            break;
        }
    }

    let first_line = function_line?;

    if debug_mode {
        println!("[DEBUG] First function line of code block: '{first_line}'");
    }

    // Try to extract symbol name based on common patterns
    let symbol_name = extract_symbol_name_from_line(first_line, &result.node_type, debug_mode)?;

    if debug_mode {
        println!("[DEBUG] Extracted symbol name: '{symbol_name}'");
    }

    // Now find the precise column position using tree-sitter
    let file_path = Path::new(&result.file);
    let extension = file_path.extension()?.to_str()?;

    // Try to parse with tree-sitter for precise position
    if let Some((precise_line, precise_column)) = find_symbol_position_with_tree_sitter(
        &result.code,
        &symbol_name,
        extension,
        result.lines.0,
        debug_mode,
    ) {
        if debug_mode {
            println!(
                "[DEBUG] Found precise position for '{}' at {}:{}:{} using tree-sitter",
                symbol_name, result.file, precise_line, precise_column
            );
        }
        return Some(SymbolInfo {
            name: symbol_name,
            line: precise_line,
            column: precise_column,
        });
    }

    // Fallback to text-based column detection if tree-sitter fails
    let column = find_symbol_column_in_line(first_line, &symbol_name);

    if debug_mode {
        println!(
            "[DEBUG] Extracted symbol '{}' from {} at {}:{} (text-based)",
            symbol_name, result.file, result.lines.0, column
        );
    }

    Some(SymbolInfo {
        name: symbol_name,
        line: (result.lines.0 - 1) as u32, // Convert to 0-indexed
        column,
    })
}

/// Find the exact column position of a symbol in a line of text
fn find_symbol_column_in_line(line: &str, symbol_name: &str) -> u32 {
    if let Some(pos) = line.find(symbol_name) {
        pos as u32
    } else {
        0
    }
}

/// Find symbol position using tree-sitter parsing for maximum accuracy
fn find_symbol_position_with_tree_sitter(
    code: &str,
    symbol_name: &str,
    file_extension: &str,
    base_line: usize,
    debug_mode: bool,
) -> Option<(u32, u32)> {
    // Get the language implementation based on file extension
    let language_impl = get_language_impl(file_extension)?;

    // Get a parser from the pool
    let mut parser = get_pooled_parser(file_extension).ok()?;
    parser
        .set_language(&language_impl.get_tree_sitter_language())
        .ok()?;

    // Parse the code block
    let tree = parser.parse(code, None)?;
    let root_node = tree.root_node();

    // Find the identifier position within the parsed tree
    let position =
        find_identifier_position_in_tree(root_node, symbol_name, code.as_bytes(), debug_mode);

    // Return the parser to the pool
    return_pooled_parser(file_extension, parser);

    // Adjust the line number to be relative to the file, not the code block
    position.map(|(line, column)| ((base_line - 1 + line as usize) as u32, column))
}

/// Recursively search for an identifier in the tree-sitter AST
fn find_identifier_position_in_tree(
    node: tree_sitter::Node,
    target_name: &str,
    content: &[u8],
    debug_mode: bool,
) -> Option<(u32, u32)> {
    // Check if this node is an identifier and matches our target
    if node.kind() == "identifier"
        || node.kind() == "field_identifier"
        || node.kind() == "type_identifier"
    {
        if let Ok(name) = node.utf8_text(content) {
            if debug_mode {
                println!(
                    "[DEBUG] Found identifier '{}' at {}:{} (looking for '{}')",
                    name,
                    node.start_position().row,
                    node.start_position().column,
                    target_name
                );
            }
            if name == target_name {
                return Some((
                    node.start_position().row as u32,
                    node.start_position().column as u32,
                ));
            }
        }
    }

    // Search in children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(pos) = find_identifier_position_in_tree(child, target_name, content, debug_mode)
        {
            return Some(pos);
        }
    }

    None
}

/// Extract symbol name from a line of code based on node type
fn extract_symbol_name_from_line(line: &str, node_type: &str, debug_mode: bool) -> Option<String> {
    // Remove the opening brace if present at the end
    let line = if line.trim_end().ends_with('{') {
        &line[..line.rfind('{').unwrap()]
    } else {
        line
    };

    let trimmed = line.trim();

    // Common patterns for different languages
    match node_type {
        "function_item"
        | "function_definition"
        | "function_declaration"
        | "function"
        | "file"
        | "import"
        | "code" => {
            // Handle various function patterns

            // Rust: pub fn function_name, async fn function_name, pub async fn function_name
            if let Some(pos) = trimmed.find("fn ") {
                let after_fn = &trimmed[pos + 3..];
                if debug_mode {
                    println!("[DEBUG] Text after 'fn ': '{after_fn}'");
                }
                return extract_name_after_keyword(after_fn);
            }

            // Python: def function_name, async def function_name
            if let Some(pos) = trimmed.find("def ") {
                return extract_name_after_keyword(&trimmed[pos + 4..]);
            }

            // JavaScript: function function_name, async function function_name
            // Also handle: export function, export async function
            if let Some(pos) = trimmed.find("function ") {
                return extract_name_after_keyword(&trimmed[pos + 9..]);
            }

            // Go: func function_name, func (r Receiver) function_name
            if let Some(pos) = trimmed.find("func ") {
                let after_func = &trimmed[pos + 5..];
                // Skip receiver if present
                if after_func.starts_with('(') {
                    if let Some(end_paren) = after_func.find(')') {
                        return extract_name_after_keyword(&after_func[end_paren + 1..]);
                    }
                }
                return extract_name_after_keyword(after_func);
            }

            // C/C++: Handle various return types
            // e.g., int function_name, void function_name, static int function_name
            // Look for common patterns where identifier is followed by parenthesis
            if let Some(paren_pos) = trimmed.find('(') {
                // Walk back from the parenthesis to find the function name
                let before_paren = &trimmed[..paren_pos];
                if let Some(name) = before_paren.split_whitespace().last() {
                    // Remove any pointer/reference symbols
                    let clean_name = name.trim_start_matches('*').trim_start_matches('&');
                    if !clean_name.is_empty() && clean_name.chars().next().unwrap().is_alphabetic()
                    {
                        return Some(clean_name.to_string());
                    }
                }
            }
        }
        "method_definition" | "method_declaration" | "method" => {
            // Similar patterns but also handle class methods
            if let Some(pos) = trimmed.find("fn ") {
                return extract_name_after_keyword(&trimmed[pos + 3..]);
            }
            if let Some(pos) = trimmed.find("def ") {
                return extract_name_after_keyword(&trimmed[pos + 4..]);
            }
        }
        "class_definition" => {
            if let Some(pos) = trimmed.find("class ") {
                return extract_name_after_keyword(&trimmed[pos + 6..]);
            }
        }
        "struct_item" => {
            if let Some(pos) = trimmed.find("struct ") {
                return extract_name_after_keyword(&trimmed[pos + 7..]);
            }
        }
        "impl_item" => {
            if let Some(pos) = trimmed.find("impl ") {
                // For impl blocks, extract the type being implemented
                let after_impl = &trimmed[pos + 5..];
                // Handle "impl Trait for Type" and "impl Type"
                if let Some(for_pos) = after_impl.find(" for ") {
                    return extract_name_after_keyword(&after_impl[for_pos + 5..]);
                } else {
                    return extract_name_after_keyword(after_impl);
                }
            }
        }
        "trait_item" => {
            if let Some(pos) = trimmed.find("trait ") {
                return extract_name_after_keyword(&trimmed[pos + 6..]);
            }
        }
        _ => {}
    }

    None
}

/// Extract a name/identifier after a keyword
fn extract_name_after_keyword(text: &str) -> Option<String> {
    let trimmed = text.trim_start();

    // Find the position of the first non-identifier character
    // This should handle: function_name(params) -> "function_name"
    let end_pos = trimmed
        .char_indices()
        .find(|(_, c)| !c.is_alphanumeric() && *c != '_')
        .map(|(i, _)| i)
        .unwrap_or(trimmed.len());

    if end_pos > 0 {
        let name = trimmed[..end_pos].trim();
        // Validate that it's a valid identifier
        if !name.is_empty()
            && (name.chars().next().unwrap().is_alphabetic() || name.starts_with('_'))
        {
            return Some(name.to_string());
        }
    }

    None
}

/// Get LSP information for a search result
fn get_lsp_info_for_result(
    file_path: &str,
    symbol_name: &str,
    line: u32,
    column: u32,
    debug_mode: bool,
) -> Option<serde_json::Value> {
    // Clone the strings to avoid lifetime issues
    let file_path_owned = file_path.to_string();
    let symbol_name_owned = symbol_name.to_string();
    let symbol_name_for_error = symbol_name.to_string();

    // Use a separate thread with its own runtime to avoid blocking
    match std::thread::spawn(move || {
        let rt = Runtime::new().ok()?;
        let path = Path::new(&file_path_owned);

        rt.block_on(async {
            get_lsp_info_async(path, &symbol_name_owned, line, column, debug_mode).await
        })
    })
    .join()
    {
        Ok(result) => result,
        Err(_) => {
            if debug_mode {
                println!("[DEBUG] LSP thread panicked for symbol: {symbol_name_for_error}");
            }
            None
        }
    }
}

/// Async function to get LSP information
async fn get_lsp_info_async(
    file_path: &Path,
    symbol_name: &str,
    line: u32,
    column: u32,
    debug_mode: bool,
) -> Option<serde_json::Value> {
    if debug_mode {
        println!(
            "[DEBUG] Getting LSP info for {} at {}:{}:{}",
            symbol_name,
            file_path.display(),
            line,
            column
        );
    }

    // Find workspace root
    let workspace_hint = find_workspace_root(file_path).map(|p| p.to_string_lossy().to_string());

    let config = LspConfig {
        use_daemon: true,
        workspace_hint: workspace_hint.clone(),
        timeout_ms: 30000, // 30 seconds timeout for search results
    };

    // Try to create LSP client - this will start the server if needed
    // Use regular new() instead of new_non_blocking() to ensure server starts
    let mut client = match LspClient::new(config).await {
        Ok(client) => {
            if debug_mode {
                println!("[DEBUG] LSP client connected successfully");
            }
            client
        }
        Err(e) => {
            // Failed to create client or start server
            if debug_mode {
                println!("[DEBUG] Failed to create LSP client: {e}");
            }
            eprintln!("Warning: LSP enrichment unavailable: {e}");
            return None;
        }
    };

    // Try to get symbol info with shorter timeout for search
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.get_symbol_info(file_path, symbol_name, line, column),
    )
    .await
    {
        Ok(Ok(Some(info))) => {
            // Create a simplified JSON structure for search results
            let references_count = info.references.len();

            let lsp_data = json!({
                "symbol": symbol_name,
                "call_hierarchy": info.call_hierarchy,
                "references_count": references_count,
            });

            if debug_mode {
                println!("[DEBUG] Got LSP info for {symbol_name}: {lsp_data:?}");
            }

            Some(lsp_data)
        }
        Ok(Ok(None)) => {
            if debug_mode {
                println!("[DEBUG] No LSP info available for {symbol_name}");
            }
            None
        }
        Ok(Err(e)) => {
            if debug_mode {
                println!("[DEBUG] LSP query failed for {symbol_name}: {e}");
            }
            None
        }
        Err(_) => {
            if debug_mode {
                println!("[DEBUG] LSP query timed out for {symbol_name}");
            }
            None
        }
    }
}

/// Find the workspace root by looking for project markers
fn find_workspace_root(file_path: &Path) -> Option<&Path> {
    let mut current = file_path.parent()?;

    // Look for common project root markers
    let markers = [
        "Cargo.toml",
        "package.json",
        "go.mod",
        "pyproject.toml",
        "setup.py",
        ".git",
        "tsconfig.json",
        "composer.json",
    ];

    while current.parent().is_some() {
        for marker in &markers {
            if current.join(marker).exists() {
                return Some(current);
            }
        }
        current = current.parent()?;
    }

    None
}
