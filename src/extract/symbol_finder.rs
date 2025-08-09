//! Functions for finding symbols in files.
//!
//! This module provides functions for finding symbols (functions, structs, classes, etc.)
//! in files using tree-sitter.

use anyhow::Result;
use probe_code::models::SearchResult;
use std::path::Path;

/// Find a symbol (function, struct, class, etc.) in a file by name
///
/// This function searches for a symbol by name in a file and returns the code block
/// containing that symbol. It uses tree-sitter to parse the code and find the symbol.
///
/// # Arguments
///
/// * `path` - The path to the file to search in
/// * `symbol` - The name of the symbol to find
/// * `content` - The content of the file
/// * `allow_tests` - Whether to include test files and test code blocks
/// * `context_lines` - Number of context lines to include
///
/// # Returns
///
/// A SearchResult containing the extracted code block for the symbol, or an error
/// if the symbol couldn't be found.
pub fn find_symbol_in_file(
    path: &Path,
    symbol: &str,
    content: &str,
    _allow_tests: bool,
    context_lines: usize,
) -> Result<SearchResult> {
    let (result, _) =
        find_symbol_in_file_with_position(path, symbol, content, _allow_tests, context_lines)?;
    Ok(result)
}

/// Find the position of a specific identifier within a tree-sitter node
///
/// This function searches for an identifier with the given name within a node
/// and returns its position. This is useful for LSP integration where we need
/// the exact position of the symbol name, not just the start of the node.
///
/// Returns (line, column) in 0-indexed coordinates, or None if not found
fn find_identifier_position_in_node(
    node: tree_sitter::Node,
    identifier_name: &str,
    content: &[u8],
    debug_mode: bool,
) -> Option<(u32, u32)> {
    if debug_mode {
        println!(
            "[DEBUG] Searching for identifier '{}' in node type '{}'",
            identifier_name,
            node.kind()
        );
    }

    // Recursively search for identifier nodes within this node
    fn find_identifier_recursive(
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
            if let Some(pos) = find_identifier_recursive(child, target_name, content, debug_mode) {
                return Some(pos);
            }
        }

        None
    }

    find_identifier_recursive(node, identifier_name, content, debug_mode)
}

/// Find a symbol in a file and return both the SearchResult and position information
///
/// Returns a tuple of (SearchResult, Option<(line, column)>) where the position
/// is the exact location of the symbol in the file (0-indexed)
pub fn find_symbol_in_file_with_position(
    path: &Path,
    symbol: &str,
    content: &str,
    _allow_tests: bool,
    context_lines: usize,
) -> Result<(SearchResult, Option<(u32, u32)>)> {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    // Check if the symbol contains a dot, indicating a nested symbol path
    let symbol_parts: Vec<&str> = symbol.split('.').collect();
    let is_nested_symbol = symbol_parts.len() > 1;

    // For nested symbols, we'll use the AST-based approach directly
    // The find_symbol_node function already handles nested symbols

    if debug_mode {
        println!("\n[DEBUG] ===== Symbol Search =====");
        if is_nested_symbol {
            println!("[DEBUG] Searching for nested symbol '{symbol}' in file {path:?}");
            println!(
                "[DEBUG] Symbol parts: {:?} (parent: '{}', child: '{}')",
                symbol_parts,
                symbol_parts[0],
                symbol_parts.last().unwrap_or(&"")
            );
        } else {
            println!("[DEBUG] Searching for symbol '{symbol}' in file {path:?}");
        }
        println!(
            "[DEBUG] Content size: {content_len} bytes",
            content_len = content.len()
        );
        println!(
            "[DEBUG] Line count: {line_count}",
            line_count = content.lines().count()
        );
    }

    // Get the file extension to determine the language
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    if debug_mode {
        println!("[DEBUG] File extension: {extension}");
    }

    // Get the language implementation for this extension
    let language_impl = crate::language::factory::get_language_impl(extension)
        .ok_or_else(|| anyhow::anyhow!("Unsupported language extension: {}", extension))?;

    if debug_mode {
        println!("[DEBUG] Language detected: {extension}");
        println!("[DEBUG] Using tree-sitter to parse file");
    }

    // Parse the file with tree-sitter using pooled parser for better performance
    let mut parser = crate::language::get_pooled_parser(extension)
        .map_err(|e| anyhow::anyhow!("Failed to get pooled parser: {}", e))?;

    let tree = parser
        .parse(content.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse file"))?;

    // Return parser to pool for reuse
    crate::language::return_pooled_parser(extension, parser);

    let root_node = tree.root_node();

    if debug_mode {
        println!("[DEBUG] File parsed successfully");
        println!(
            "[DEBUG] Root node type: {root_node_kind}",
            root_node_kind = root_node.kind()
        );
        println!(
            "[DEBUG] Root node range: {}:{} - {}:{}",
            root_node.start_position().row + 1,
            root_node.start_position().column + 1,
            root_node.end_position().row + 1,
            root_node.end_position().column + 1
        );
        println!("[DEBUG] Searching for symbol '{symbol}' in AST");
    }

    // Function to recursively search for a node with the given symbol name
    fn find_symbol_node<'a>(
        node: tree_sitter::Node<'a>,
        symbol_parts: &[&str],
        language_impl: &dyn crate::language::language_trait::LanguageImpl,
        content: &'a [u8],
        debug_mode: bool,
    ) -> Option<tree_sitter::Node<'a>> {
        // If we're looking for a nested symbol (e.g., "Class.method"), we need to:
        // 1. First find the parent symbol (e.g., "Class")
        // 2. Then search within that node for the child symbol (e.g., "method")
        let current_symbol = symbol_parts[0];
        let is_nested = symbol_parts.len() > 1;

        // Check if this node is an acceptable parent (function, struct, class, etc.)
        if language_impl.is_acceptable_parent(&node) {
            if debug_mode {
                println!(
                    "[DEBUG] Checking node type '{}' at {}:{} for symbol '{}'",
                    node.kind(),
                    node.start_position().row + 1,
                    node.start_position().column + 1,
                    current_symbol
                );
            }

            // Try to extract the name of this node
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier"
                    || child.kind() == "field_identifier"
                    || child.kind() == "type_identifier"
                    || child.kind() == "function_declarator"
                {
                    // Get the text of this identifier
                    if let Ok(name) = child.utf8_text(content) {
                        if debug_mode {
                            println!(
                                "[DEBUG] Found identifier: '{name}' (looking for '{current_symbol}')"
                            );
                        }

                        if name == current_symbol {
                            if is_nested {
                                // If this is a nested symbol, we found the parent
                                // Now we need to search for the child within this node
                                if debug_mode {
                                    println!(
                                        "[DEBUG] Found parent symbol '{}' in node type '{}', now searching for child '{}'",
                                        current_symbol,
                                        node.kind(),
                                        symbol_parts[1]
                                    );
                                }

                                // First, check if there's a direct method definition with this name
                                let mut direct_method_cursor = node.walk();
                                for direct_child in node.children(&mut direct_method_cursor) {
                                    if direct_child.kind() == "method_definition" {
                                        let mut method_cursor = direct_child.walk();
                                        for method_child in
                                            direct_child.children(&mut method_cursor)
                                        {
                                            if method_child.kind() == "property_identifier" {
                                                if let Ok(method_name) =
                                                    method_child.utf8_text(content)
                                                {
                                                    if debug_mode {
                                                        println!(
                                                            "[DEBUG] Found direct method: '{}' (looking for '{}')",
                                                            method_name, symbol_parts[1]
                                                        );
                                                    }

                                                    if method_name == symbol_parts[1] {
                                                        if debug_mode {
                                                            println!(
                                                                "[DEBUG] Found child symbol '{}' as direct method_definition",
                                                                symbol_parts[1]
                                                            );
                                                            println!(
                                                                "[DEBUG] Symbol location: {}:{} - {}:{}",
                                                                direct_child.start_position().row + 1,
                                                                direct_child.start_position().column + 1,
                                                                direct_child.end_position().row + 1,
                                                                direct_child.end_position().column + 1
                                                            );
                                                        }
                                                        return Some(direct_child);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Look for any node that might contain the child symbol
                                let mut child_cursor = node.walk();
                                for child_node in node.children(&mut child_cursor) {
                                    if debug_mode {
                                        println!(
                                            "[DEBUG] Checking child node type '{}' for symbol '{}'",
                                            child_node.kind(),
                                            symbol_parts[1]
                                        );
                                    }

                                    // Check if this node is the child symbol we're looking for
                                    if language_impl.is_acceptable_parent(&child_node) {
                                        // Try to extract the name of this node
                                        let mut subcursor = child_node.walk();
                                        for subchild in child_node.children(&mut subcursor) {
                                            if subchild.kind() == "identifier"
                                                || subchild.kind() == "property_identifier"
                                                || subchild.kind() == "field_identifier"
                                                || subchild.kind() == "type_identifier"
                                                || subchild.kind() == "method_definition"
                                            {
                                                // For method_definition, we need to get the property_identifier child
                                                if subchild.kind() == "method_definition" {
                                                    let mut method_cursor = subchild.walk();
                                                    for method_child in
                                                        subchild.children(&mut method_cursor)
                                                    {
                                                        if method_child.kind()
                                                            == "property_identifier"
                                                        {
                                                            if let Ok(method_name) =
                                                                method_child.utf8_text(content)
                                                            {
                                                                if debug_mode {
                                                                    println!(
                                                                        "[DEBUG] Found method: '{}' (looking for '{}')",
                                                                        method_name, symbol_parts[1]
                                                                    );
                                                                }

                                                                if method_name == symbol_parts[1] {
                                                                    if debug_mode {
                                                                        println!(
                                                                            "[DEBUG] Found child symbol '{}' in method_definition",
                                                                            symbol_parts[1]
                                                                        );
                                                                        println!(
                                                                            "[DEBUG] Symbol location: {}:{} - {}:{}",
                                                                            subchild.start_position().row + 1,
                                                                            subchild.start_position().column + 1,
                                                                            subchild.end_position().row + 1,
                                                                            subchild.end_position().column + 1
                                                                        );
                                                                    }
                                                                    return Some(subchild);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    continue;
                                                }
                                                if let Ok(name) = subchild.utf8_text(content) {
                                                    if debug_mode {
                                                        println!(
                                                            "[DEBUG] Found identifier: '{}' (looking for '{}')",
                                                            name, symbol_parts[1]
                                                        );
                                                    }

                                                    if name == symbol_parts[1] {
                                                        if debug_mode {
                                                            println!(
                                                                "[DEBUG] Found child symbol '{}' in node type '{}'",
                                                                symbol_parts[1],
                                                                child_node.kind()
                                                            );
                                                            println!(
                                                                "[DEBUG] Symbol location: {}:{} - {}:{}",
                                                                child_node.start_position().row + 1,
                                                                child_node.start_position().column + 1,
                                                                child_node.end_position().row + 1,
                                                                child_node.end_position().column + 1
                                                            );
                                                        }
                                                        return Some(child_node);
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Recursively search in this child node
                                    if let Some(found) = find_symbol_node(
                                        child_node,
                                        &symbol_parts[1..],
                                        language_impl,
                                        content,
                                        debug_mode,
                                    ) {
                                        return Some(found);
                                    }
                                }
                            } else {
                                // If this is a simple symbol, we found it
                                if debug_mode {
                                    println!(
                                        "[DEBUG] Found symbol '{}' in node type '{}'",
                                        current_symbol,
                                        node.kind()
                                    );
                                    println!(
                                        "[DEBUG] Symbol location: {}:{} - {}:{}",
                                        node.start_position().row + 1,
                                        node.start_position().column + 1,
                                        node.end_position().row + 1,
                                        node.end_position().column + 1
                                    );
                                }
                                return Some(node);
                            }
                        }
                    }

                    // For function_declarator, we need to look deeper
                    if child.kind() == "function_declarator" {
                        if debug_mode {
                            println!("[DEBUG] Checking function_declarator for symbol");
                        }

                        let mut subcursor = child.walk();
                        for subchild in child.children(&mut subcursor) {
                            if subchild.kind() == "identifier" {
                                if let Ok(name) = subchild.utf8_text(content) {
                                    if debug_mode {
                                        println!("[DEBUG] Found function identifier: '{name}' (looking for '{current_symbol}')");
                                    }

                                    if name == current_symbol {
                                        if is_nested {
                                            // If this is a nested symbol, we found the parent
                                            // Now we need to search for the child within this node
                                            if debug_mode {
                                                println!(
                                                    "[DEBUG] Found parent symbol '{}' in function_declarator, now searching for child '{}'",
                                                    current_symbol,
                                                    symbol_parts[1]
                                                );
                                            }

                                            // Recursively search for the child symbol within this node
                                            let mut child_cursor = node.walk();
                                            for child_node in node.children(&mut child_cursor) {
                                                if let Some(found) = find_symbol_node(
                                                    child_node,
                                                    &symbol_parts[1..],
                                                    language_impl,
                                                    content,
                                                    debug_mode,
                                                ) {
                                                    return Some(found);
                                                }
                                            }
                                        } else {
                                            // If this is a simple symbol, we found it
                                            if debug_mode {
                                                println!(
                                                    "[DEBUG] Found symbol '{current_symbol}' in function_declarator"
                                                );
                                                println!(
                                                    "[DEBUG] Symbol location: {}:{} - {}:{}",
                                                    node.start_position().row + 1,
                                                    node.start_position().column + 1,
                                                    node.end_position().row + 1,
                                                    node.end_position().column + 1
                                                );
                                            }
                                            return Some(node);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Recursively search in children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) =
                find_symbol_node(child, symbol_parts, language_impl, content, debug_mode)
            {
                return Some(found);
            }
        }

        None
    }

    // Search for the symbol in the AST
    if let Some(found_node) = find_symbol_node(
        root_node,
        &symbol_parts,
        language_impl.as_ref(),
        content.as_bytes(),
        debug_mode,
    ) {
        let node_start_line = found_node.start_position().row + 1;
        let node_end_line = found_node.end_position().row + 1;

        if debug_mode {
            println!("\n[DEBUG] ===== Symbol Found =====");
            println!("[DEBUG] Found symbol '{symbol}' at lines {node_start_line}-{node_end_line}");
            println!(
                "[DEBUG] Node type: {node_kind}",
                node_kind = found_node.kind()
            );
            println!(
                "[DEBUG] Node range: {}:{} - {}:{}",
                found_node.start_position().row + 1,
                found_node.start_position().column + 1,
                found_node.end_position().row + 1,
                found_node.end_position().column + 1
            );
        }

        // Extract the code block
        let node_text = &content[found_node.start_byte()..found_node.end_byte()];

        if debug_mode {
            println!(
                "[DEBUG] Extracted code size: {code_size} bytes",
                code_size = node_text.len()
            );
            println!(
                "[DEBUG] Extracted code lines: {line_count}",
                line_count = node_text.lines().count()
            );
        }

        // Tokenize the content
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let node_text_str = node_text.to_string();
        let tokenized_content =
            crate::ranking::preprocess_text_with_filename(&node_text_str, &filename);

        // Extract position information from the found node
        // Instead of using the node's start position (which might be the "fn" keyword),
        // find the actual identifier position within the node
        let (symbol_line, symbol_column) = find_identifier_position_in_node(
            found_node,
            symbol_parts[symbol_parts.len() - 1], // Use the last part for nested symbols
            content.as_bytes(),
            debug_mode,
        )
        .unwrap_or((
            found_node.start_position().row as u32,
            found_node.start_position().column as u32,
        ));

        if debug_mode {
            println!("[DEBUG] Symbol position: line {symbol_line}, column {symbol_column}");
        }

        let search_result = SearchResult {
            file: path.to_string_lossy().to_string(),
            lines: (node_start_line, node_end_line),
            node_type: found_node.kind().to_string(),
            code: node_text_str,
            matched_by_filename: None,
            rank: None,
            score: None,
            tfidf_score: None,
            bm25_score: None,
            tfidf_rank: None,
            bm25_rank: None,
            new_score: None,
            hybrid2_rank: None,
            combined_score_rank: None,
            file_unique_terms: None,
            file_total_matches: None,
            file_match_rank: None,
            block_unique_terms: None,
            block_total_matches: None,
            parent_file_id: None,
            block_id: None,
            matched_keywords: None,
            tokenized_content: Some(tokenized_content),
            lsp_info: None,
        };

        return Ok((search_result, Some((symbol_line, symbol_column))));
    }

    // If we couldn't find the symbol using tree-sitter, try a simple text search as fallback
    if debug_mode {
        println!("\n[DEBUG] ===== Symbol Not Found in AST =====");
        println!("[DEBUG] Symbol '{symbol}' not found in AST");
        println!("[DEBUG] Trying text search fallback");
    }

    // Simple text search for the symbol
    let lines: Vec<&str> = content.lines().collect();
    let mut found_line = None;

    // For nested symbols, we'll try to find lines that contain all parts
    // This is a simple fallback and may not be as accurate as AST parsing
    let search_terms = if is_nested_symbol {
        // For nested symbols, we'll look for lines containing all parts
        if debug_mode {
            println!(
                "[DEBUG] Using fallback search for nested symbol: looking for lines containing all parts"
            );
        }
        symbol_parts.to_vec()
    } else {
        vec![symbol]
    };

    if debug_mode {
        println!(
            "[DEBUG] Performing text search for '{:?}' across {} lines",
            search_terms,
            lines.len()
        );
    }

    for (i, line) in lines.iter().enumerate() {
        // Check if the line contains all search terms
        let found = search_terms.iter().all(|term| line.contains(term));

        if found {
            found_line = Some(i + 1); // 1-indexed line number
            if debug_mode {
                println!(
                    "[DEBUG] Found symbol '{}' in line {}: '{}'",
                    symbol,
                    i + 1,
                    line.trim()
                );
            }
            break;
        }
    }

    if let Some(line_num) = found_line {
        if debug_mode {
            println!("\n[DEBUG] ===== Symbol Found via Text Search =====");
            println!("[DEBUG] Found symbol '{symbol}' using text search at line {line_num}");
        }

        // Extract context around the line
        let start_line = line_num.saturating_sub(context_lines);
        let end_line = std::cmp::min(line_num + context_lines, lines.len());

        if debug_mode {
            println!("[DEBUG] Extracting context around line {line_num}");
            println!("[DEBUG] Context lines: {context_lines}");
            println!("[DEBUG] Extracting lines {start_line}-{end_line}");
        }

        // Adjust start_line to be at least 1 (1-indexed)
        let start_idx = if start_line > 0 { start_line - 1 } else { 0 };

        let context = lines[start_idx..end_line].join("\n");

        if debug_mode {
            println!(
                "[DEBUG] Extracted {line_count} lines of code",
                line_count = end_line - start_line
            );
            println!(
                "[DEBUG] Content size: {content_size} bytes",
                content_size = context.len()
            );
        }

        // Tokenize the content
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let tokenized_content = crate::ranking::preprocess_text_with_filename(&context, &filename);

        let search_result = SearchResult {
            file: path.to_string_lossy().to_string(),
            lines: (start_line, end_line),
            node_type: "text_search".to_string(),
            code: context,
            matched_by_filename: None,
            rank: None,
            score: None,
            tfidf_score: None,
            bm25_score: None,
            tfidf_rank: None,
            bm25_rank: None,
            new_score: None,
            hybrid2_rank: None,
            combined_score_rank: None,
            file_unique_terms: None,
            file_total_matches: None,
            file_match_rank: None,
            block_unique_terms: None,
            block_total_matches: None,
            parent_file_id: None,
            block_id: None,
            matched_keywords: None,
            tokenized_content: Some(tokenized_content),
            lsp_info: None,
        };

        // For text search fallback, we don't have precise position information
        // We could estimate it, but it's less reliable than tree-sitter positions
        return Ok((search_result, None));
    }

    // If we get here, we couldn't find the symbol
    if debug_mode {
        println!("\n[DEBUG] ===== Symbol Not Found =====");
        println!("[DEBUG] Symbol '{symbol}' not found in file {path:?}");
        println!("[DEBUG] Neither AST parsing nor text search found the symbol");
    }

    Err(anyhow::anyhow!(
        "Symbol '{}' not found in file {:?}",
        symbol,
        path
    ))
}
