//! Functions for processing files and extracting code blocks.
//!
//! This module provides functions for processing files and extracting code blocks
//! based on file paths and optional line numbers.
use anyhow::{Context, Result};
use probe_code::extract::symbol_finder::find_symbol_in_file_with_position;
use probe_code::language::parser::parse_file_for_code_blocks;
use probe_code::lsp_integration::{LspClient, LspConfig};
use probe_code::models::SearchResult;
use probe_code::path_safety;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::runtime::Runtime;

/// Process a single file and extract code blocks
///
/// If a line range is specified, we find all AST blocks overlapping that range,
/// merge them into a bounding block, and return it. If no blocks are found, fallback
/// to the literal lines. If only a single line is specified, do the same but for that line.
/// If a symbol is specified, we delegate to `find_symbol_in_file`.
/// If specific lines are provided, we find AST blocks for each line and merge them.
/// If no lines or symbol are specified, return the entire file.
///
/// This function returns a single SearchResult that includes either the merged AST code
/// or the literal lines as a fallback.
pub fn process_file_for_extraction(
    path: &Path,
    start_line: Option<usize>,
    end_line: Option<usize>,
    symbol: Option<&str>,
    allow_tests: bool,
    context_lines: usize,
    specific_lines: Option<&HashSet<usize>>,
) -> Result<SearchResult> {
    process_file_for_extraction_with_lsp(
        path,
        start_line,
        end_line,
        symbol,
        allow_tests,
        context_lines,
        specific_lines,
        false,
        false, // default to not including stdlib
    )
}

/// Process a single file and extract code blocks with optional LSP integration
///
/// This is an enhanced version of the extraction function that optionally
/// queries LSP servers for additional symbol information like call hierarchy
/// and references when LSP is enabled.
#[allow(clippy::too_many_arguments)]
pub fn process_file_for_extraction_with_lsp(
    path: &Path,
    start_line: Option<usize>,
    end_line: Option<usize>,
    symbol: Option<&str>,
    allow_tests: bool,
    context_lines: usize,
    specific_lines: Option<&HashSet<usize>>,
    enable_lsp: bool,
    include_stdlib: bool,
) -> Result<SearchResult> {
    // Check if debug mode is enabled
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        eprintln!("\n[DEBUG] ===== Processing File for Extraction =====");
        eprintln!("[DEBUG] File path: {path:?}");
        eprintln!("[DEBUG] Start line: {start_line:?}");
        eprintln!("[DEBUG] End line: {end_line:?}");
        eprintln!("[DEBUG] Symbol: {symbol:?}");
        eprintln!("[DEBUG] Allow tests: {allow_tests}");
        eprintln!("[DEBUG] Context lines: {context_lines}");
        eprintln!("[DEBUG] Specific lines: {specific_lines:?}");
        eprintln!("[DEBUG] LSP enabled: {enable_lsp}");
    }

    // Check if the file exists (using safe no-follow check)
    if !path_safety::exists_no_follow(path) {
        if debug_mode {
            eprintln!("[DEBUG] Error: File does not exist");
        }
        return Err(anyhow::anyhow!("File does not exist: {:?}", path));
    }

    // Read the file content
    let content = fs::read_to_string(path).context(format!("Failed to read file: {path:?}"))?;
    let lines: Vec<&str> = content.lines().collect();

    if debug_mode {
        eprintln!("[DEBUG] File read successfully");
        eprintln!("[DEBUG] File size: {} bytes", content.len());
        eprintln!("[DEBUG] Line count: {}", lines.len());
    }

    // If we have a symbol, find it in the file
    if let Some(symbol_name) = symbol {
        if debug_mode {
            eprintln!("[DEBUG] Looking for symbol: {symbol_name}");
        }

        // Find the symbol in the file first and get position information
        let (mut result, symbol_position) = find_symbol_in_file_with_position(
            path,
            symbol_name,
            &content,
            allow_tests,
            context_lines,
        )?;

        // Add LSP information if enabled
        if enable_lsp {
            if debug_mode {
                println!("[DEBUG] LSP enabled, attempting to get symbol info for: {symbol_name}");
            }
            // Only attempt LSP if we have position information from tree-sitter
            if let Some((line, column)) = symbol_position {
                if debug_mode {
                    println!(
                        "[DEBUG] Using position from tree-sitter: line {line}, column {column}"
                    );
                }
                // Ensure we use an absolute path for workspace detection
                let abs_path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    std::env::current_dir().unwrap_or_default().join(path)
                };
                result.lsp_info = get_lsp_symbol_info_sync(
                    &abs_path,
                    symbol_name,
                    line,
                    column,
                    debug_mode,
                    include_stdlib,
                );
                // Don't add error placeholder - let formatter handle missing LSP info gracefully
            } else if debug_mode {
                println!(
                    "[DEBUG] No position information available from tree-sitter, skipping LSP"
                );
            }
        }

        return Ok(result);
    }

    // If we have a line range (start_line, end_line), gather AST blocks overlapping that range.
    if let (Some(start), Some(end)) = (start_line, end_line) {
        if debug_mode {
            eprintln!("[DEBUG] Extracting line range: {start}-{end} (with AST merging)");
        }

        // Clamp line numbers to valid ranges instead of failing
        // Bound start to 1..lines.len()
        let mut clamped_start = start.clamp(1, lines.len());

        // Bound end to clamped_start..lines.len()
        let mut clamped_end = end.clamp(clamped_start, lines.len());

        // If the start is still larger than the total lines, we know there's literally nothing to extract
        if clamped_start > lines.len() {
            clamped_start = lines.len();
        }

        // If the end is zero or ends up less than the start, just clamp it to the start
        if clamped_end < clamped_start {
            clamped_end = clamped_start;
        }

        if debug_mode && (clamped_start != start || clamped_end != end) {
            eprintln!(
                "[DEBUG] Requested lines {start}-{end} out of range; clamping to {clamped_start}-{clamped_end}"
            );
        }

        // Use the clamped values for the rest of the function
        let start = clamped_start;
        let end = clamped_end;

        // Parse AST for all lines in [start, end]
        let mut needed_lines = HashSet::new();
        for l in start..=end {
            needed_lines.insert(l);
        }

        // If specific_lines is provided, add those lines too
        if let Some(lines_set) = specific_lines {
            for &line in lines_set {
                needed_lines.insert(line);
            }
        }

        let code_blocks_result = parse_file_for_code_blocks(
            &content,
            file_extension(path),
            &needed_lines,
            allow_tests,
            None,
        );

        match code_blocks_result {
            Ok(blocks) if !blocks.is_empty() => {
                // Merge them into a bounding block
                // i.e. from min(block.start_row) to max(block.end_row)
                let min_start = blocks.iter().map(|b| b.start_row).min().unwrap_or(0);
                let max_end = blocks.iter().map(|b| b.end_row).max().unwrap_or(0);

                // Ensure max_end is within bounds of the file
                let max_end = std::cmp::min(max_end, lines.len() - 1);

                // Ensure min_start is not greater than max_end
                let min_start = std::cmp::min(min_start, max_end);

                // lines in the file are 0-indexed internally, so we add 1 for final display
                let merged_start = min_start + 1;
                let merged_end = max_end + 1;

                if debug_mode {
                    eprintln!(
                        "[DEBUG] Found {} overlapping AST blocks, merging into lines {}-{}",
                        blocks.len(),
                        merged_start,
                        merged_end
                    );
                }

                let merged_content = lines[min_start..=max_end].join("\n");

                // Tokenize the content
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let tokenized_content =
                    crate::ranking::preprocess_text_with_filename(&merged_content, &filename);

                Ok(SearchResult {
                    file: path.to_string_lossy().to_string(),
                    lines: (merged_start, merged_end),
                    node_type: "merged_ast_range".to_string(),
                    code: merged_content,
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
                })
            }
            _ => {
                // Fallback to literal extraction of lines [start..end]
                if debug_mode {
                    eprintln!(
                        "[DEBUG] No AST blocks found for the range {start}-{end}, falling back to literal lines"
                    );
                }
                let start_idx = start - 1;
                let end_idx = end;
                let range_content = lines[start_idx..end_idx].join("\n");
                // Tokenize the content
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let tokenized_content =
                    crate::ranking::preprocess_text_with_filename(&range_content, &filename);

                Ok(SearchResult {
                    file: path.to_string_lossy().to_string(),
                    lines: (start, end),
                    node_type: "range".to_string(),
                    code: range_content,
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
                })
            }
        }
    }
    // Single line extraction
    else if let Some(line_num) = start_line {
        if debug_mode {
            eprintln!("[DEBUG] Single line extraction requested: line {line_num}");
        }
        // Clamp line number to valid range instead of failing
        let clamped_line_num = line_num.clamp(1, lines.len());

        if debug_mode && clamped_line_num != line_num {
            eprintln!(
                "[DEBUG] Requested line {line_num} out of bounds; clamping to {clamped_line_num}"
            );
        }

        // Use the clamped value for the rest of the function
        let line_num = clamped_line_num;

        // We'll parse the AST for just this line
        let mut needed_lines = HashSet::new();
        needed_lines.insert(line_num);

        // If specific_lines is provided, add those lines too
        if let Some(lines_set) = specific_lines {
            for &line in lines_set {
                needed_lines.insert(line);
            }
        }

        match parse_file_for_code_blocks(
            &content,
            file_extension(path),
            &needed_lines,
            allow_tests,
            None,
        ) {
            Ok(blocks) if !blocks.is_empty() => {
                // Merge them into a bounding block (in most cases it should only be one block,
                // but let's be safe if multiple overlap)
                let min_start = blocks.iter().map(|b| b.start_row).min().unwrap_or(0);
                let max_end = blocks.iter().map(|b| b.end_row).max().unwrap_or(0);

                // Ensure max_end is within bounds of the file
                let max_end = std::cmp::min(max_end, lines.len() - 1);

                // Ensure min_start is not greater than max_end
                let min_start = std::cmp::min(min_start, max_end);

                let merged_start = min_start + 1;
                let merged_end = max_end + 1;

                if debug_mode {
                    eprintln!(
                        "[DEBUG] Found {} AST blocks for line {}, merging into lines {}-{}",
                        blocks.len(),
                        line_num,
                        merged_start,
                        merged_end
                    );
                }
                let merged_content = lines[min_start..=max_end].join("\n");

                // Tokenize the content
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let tokenized_content =
                    crate::ranking::preprocess_text_with_filename(&merged_content, &filename);

                // Attempt to get LSP information for line-based extraction
                let lsp_info = if enable_lsp {
                    if debug_mode {
                        println!("[DEBUG] LSP enabled for line extraction, finding function declaration for line {line_num}");
                    }
                    // Ensure we use an absolute path for workspace detection
                    let abs_path = if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        std::env::current_dir().unwrap_or_default().join(path)
                    };

                    // Use tree-sitter to find the function declaration that contains this line
                    if let Some((symbol_name, decl_line, decl_column)) =
                        find_function_declaration_at_line(
                            &content,
                            &abs_path,
                            line_num as u32,
                            debug_mode,
                        )
                    {
                        if debug_mode {
                            println!(
                                "[DEBUG] Found enclosing function '{symbol_name}' at line {decl_line}, column {decl_column}"
                            );
                        }
                        get_lsp_symbol_info_sync(
                            &abs_path,
                            &symbol_name,
                            decl_line,
                            decl_column,
                            debug_mode,
                            include_stdlib,
                        )
                    } else {
                        if debug_mode {
                            println!(
                                "[DEBUG] No enclosing function found for line {line_num}, using fallback"
                            );
                        }
                        // Fallback to original behavior if no function declaration found
                        get_lsp_symbol_info_sync(
                            &abs_path,
                            "",
                            line_num as u32,
                            0,
                            debug_mode,
                            include_stdlib,
                        )
                    }
                } else {
                    None
                };
                // Don't add error placeholder - let formatter handle missing LSP info gracefully

                Ok(SearchResult {
                    file: path.to_string_lossy().to_string(),
                    lines: (merged_start, merged_end),
                    node_type: "merged_ast_line".to_string(),
                    code: merged_content,
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
                    lsp_info,
                })
            }
            _ => {
                // If no AST block found, fallback to the line + context
                if debug_mode {
                    eprintln!(
                        "[DEBUG] No AST blocks found for line {line_num}, using context-based fallback"
                    );
                }

                // Extract context
                let file_line_count = lines.len();
                let start_ctx = if line_num <= context_lines {
                    1
                } else {
                    line_num - context_lines
                };
                let end_ctx = std::cmp::min(line_num + context_lines, file_line_count);

                let start_idx = start_ctx - 1;
                let end_idx = end_ctx;

                let context_code = lines[start_idx..end_idx].join("\n");

                // Tokenize the content
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let tokenized_content =
                    crate::ranking::preprocess_text_with_filename(&context_code, &filename);

                // Attempt to get LSP information for line-based extraction fallback
                let lsp_info = if enable_lsp {
                    if debug_mode {
                        println!("[DEBUG] LSP enabled for line fallback extraction, finding function declaration for line {line_num}");
                    }
                    // Ensure we use an absolute path for workspace detection
                    let abs_path = if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        std::env::current_dir().unwrap_or_default().join(path)
                    };

                    // Use tree-sitter to find the function declaration that contains this line
                    if let Some((symbol_name, decl_line, decl_column)) =
                        find_function_declaration_at_line(
                            &content,
                            &abs_path,
                            line_num as u32,
                            debug_mode,
                        )
                    {
                        if debug_mode {
                            println!("[DEBUG] Found enclosing function '{symbol_name}' at line {decl_line}, column {decl_column} (fallback)");
                        }
                        get_lsp_symbol_info_sync(
                            &abs_path,
                            &symbol_name,
                            decl_line,
                            decl_column,
                            debug_mode,
                            include_stdlib,
                        )
                    } else {
                        if debug_mode {
                            println!("[DEBUG] No enclosing function found for line {line_num} (fallback), using original position");
                        }
                        // Fallback to original behavior if no function declaration found
                        get_lsp_symbol_info_sync(
                            &abs_path,
                            "",
                            line_num as u32,
                            0,
                            debug_mode,
                            include_stdlib,
                        )
                    }
                } else {
                    None
                };
                // Don't add error placeholder - let formatter handle missing LSP info gracefully

                Ok(SearchResult {
                    file: path.to_string_lossy().to_string(),
                    lines: (start_ctx, end_ctx),
                    node_type: "context".to_string(),
                    code: context_code,
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
                    lsp_info,
                })
            }
        }
    } else if let Some(lines_set) = specific_lines {
        // We have specific lines to extract
        if debug_mode {
            eprintln!("[DEBUG] Extracting specific lines: {lines_set:?}");
        }

        if lines_set.is_empty() {
            if debug_mode {
                eprintln!("[DEBUG] No specific lines provided, returning entire file content");
            }

            // Tokenize the content
            let filename = path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let tokenized_content =
                crate::ranking::preprocess_text_with_filename(&content, &filename);

            return Ok(SearchResult {
                file: path.to_string_lossy().to_string(),
                lines: (1, lines.len()),
                node_type: "file".to_string(),
                code: content,
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
            });
        }

        // Clamp specific lines to valid range instead of failing
        let mut clamped_lines = HashSet::new();
        let mut any_clamped = false;

        for &line in lines_set {
            if line == 0 || line > lines.len() {
                if line > 0 {
                    // Only add lines that are > 0 (clamp to max)
                    clamped_lines.insert(line.min(lines.len()));
                }
                any_clamped = true;
            } else {
                clamped_lines.insert(line);
            }
        }

        if debug_mode && any_clamped {
            eprintln!(
                "[DEBUG] Some requested lines were out of bounds; clamping to valid range 1-{}",
                lines.len()
            );
        }

        // Use the clamped set for the rest of the function
        let lines_set = &clamped_lines;

        // Parse AST for all specified lines
        let code_blocks_result = parse_file_for_code_blocks(
            &content,
            file_extension(path),
            lines_set,
            allow_tests,
            None,
        );

        match code_blocks_result {
            Ok(blocks) if !blocks.is_empty() => {
                // Merge them into a bounding block
                let min_start = blocks.iter().map(|b| b.start_row).min().unwrap_or(0);
                let max_end = blocks.iter().map(|b| b.end_row).max().unwrap_or(0);

                // Ensure max_end is within bounds of the file
                let max_end = std::cmp::min(max_end, lines.len() - 1);

                // Ensure min_start is not greater than max_end
                let min_start = std::cmp::min(min_start, max_end);

                // lines in the file are 0-indexed internally, so we add 1 for final display
                let merged_start = min_start + 1;
                let merged_end = max_end + 1;

                if debug_mode {
                    eprintln!(
                        "[DEBUG] Found {} AST blocks for specific lines, merging into lines {}-{}",
                        blocks.len(),
                        merged_start,
                        merged_end
                    );
                }

                let merged_content = lines[min_start..=max_end].join("\n");

                // Tokenize the content
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let tokenized_content =
                    crate::ranking::preprocess_text_with_filename(&merged_content, &filename);

                Ok(SearchResult {
                    file: path.to_string_lossy().to_string(),
                    lines: (merged_start, merged_end),
                    node_type: "merged_ast_specific_lines".to_string(),
                    code: merged_content,
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
                })
            }
            _ => {
                // Fallback to literal extraction of the specific lines
                if debug_mode {
                    eprintln!(
                        "[DEBUG] No AST blocks found for specific lines, falling back to literal lines"
                    );
                }

                // Get the min and max line numbers
                let min_line = *lines_set.iter().min().unwrap_or(&1);
                let max_line = *lines_set.iter().max().unwrap_or(&lines.len());

                // Add some context around the lines
                let start = if min_line <= context_lines {
                    1
                } else {
                    min_line - context_lines
                };
                let end = std::cmp::min(max_line + context_lines, lines.len());

                let start_idx = start - 1;
                let end_idx = end;
                let range_content = lines[start_idx..end_idx].join("\n");

                // Tokenize the content
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let tokenized_content =
                    crate::ranking::preprocess_text_with_filename(&range_content, &filename);

                Ok(SearchResult {
                    file: path.to_string_lossy().to_string(),
                    lines: (start, end),
                    node_type: "specific_lines".to_string(),
                    code: range_content,
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
                })
            }
        }
    } else {
        // No line specified, return the entire file
        if debug_mode {
            eprintln!("[DEBUG] No line or range specified, returning entire file content");
        }

        // Tokenize the content
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let tokenized_content = crate::ranking::preprocess_text_with_filename(&content, &filename);

        Ok(SearchResult {
            file: path.to_string_lossy().to_string(),
            lines: (1, lines.len()),
            node_type: "file".to_string(),
            code: content,
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
        })
    }
}

/// Helper to get LSP information for a symbol at a specific position
async fn get_lsp_symbol_info(
    file_path: &Path,
    symbol_name: &str,
    line: u32,
    column: u32,
    debug_mode: bool,
    include_stdlib: bool,
) -> Option<serde_json::Value> {
    if debug_mode {
        println!("[DEBUG] Attempting to get LSP info for symbol: {symbol_name}");
        println!("[DEBUG] File path for workspace detection: {file_path:?}");
    }

    // Create non-blocking LSP client that doesn't wait for server to be ready
    // Find the actual workspace root by looking for project markers
    let workspace_root_result = find_workspace_root(file_path);
    if debug_mode {
        println!("[DEBUG] find_workspace_root returned: {workspace_root_result:?}");
    }
    let workspace_hint = workspace_root_result
        .map(|p| {
            if debug_mode {
                println!("[DEBUG] Found workspace root via find_workspace_root: {p:?}");
            }
            p.to_string_lossy().to_string()
        })
        .or_else(|| {
            // Fallback: for Go files, use the current working directory if we can't find a project root
            if file_path.extension().and_then(|ext| ext.to_str()) == Some("go") {
                let cwd = std::env::current_dir().ok();
                if debug_mode {
                    println!(
                        "[DEBUG] Using current working directory fallback for Go file: {cwd:?}"
                    );
                }
                cwd.map(|p| p.to_string_lossy().to_string())
            } else {
                if debug_mode {
                    println!("[DEBUG] Not a Go file, no workspace fallback applied");
                }
                None
            }
        });
    // Determine whether to include stdlib frames. Default: filter out stdlib.
    // This can be controlled by the CLI flag or the PROBE_INCLUDE_STDLIB environment variable.
    let final_include_stdlib = include_stdlib
        || std::env::var("PROBE_INCLUDE_STDLIB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

    let config = LspConfig {
        use_daemon: true,
        workspace_hint: workspace_hint.clone(),
        timeout_ms: 90000, // 90 seconds timeout for complex projects with rust-analyzer
        include_stdlib: final_include_stdlib,
    };

    if debug_mode {
        println!(
            "[DEBUG] LSP config: timeout={}ms, workspace_hint={:?}, include_stdlib={}",
            config.timeout_ms, config.workspace_hint, config.include_stdlib
        );
    }

    // Try non-blocking client creation with brief backoff (helps on cold CI runners)
    let mut attempts = 0u32;
    let max_attempts = 3u32;
    let mut client = loop {
        if let Some(c) = LspClient::new_non_blocking(config.clone()).await {
            if debug_mode {
                println!(
                    "[DEBUG] LSP client connected successfully (attempt #{})",
                    attempts + 1
                );
            }
            break c;
        }
        attempts += 1;
        if attempts >= max_attempts {
            if debug_mode {
                eprintln!(
                    "[DEBUG] LSP server not ready after {attempts} attempts for symbol: {symbol_name}"
                );
            }
            // Don't print error in non-debug mode - LSP is optional enhancement
            return None;
        }
        if debug_mode {
            println!("[DEBUG] LSP not ready (attempt #{attempts}), retrying shortly…");
        }
        // Shorter delay to avoid breaking non-blocking test expectations
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    };

    // Check if LSP is supported for this file
    if !client.is_supported(file_path) {
        if debug_mode {
            println!("[DEBUG] LSP not supported for file: {file_path:?}");
        }
        return None;
    }

    // Get symbol information with retries
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 2;

    while attempts < MAX_ATTEMPTS {
        attempts += 1;
        if debug_mode && attempts > 1 {
            println!("[DEBUG] LSP attempt {attempts} of {MAX_ATTEMPTS}");
        }

        match client
            .get_symbol_info(file_path, symbol_name, line, column)
            .await
        {
            Ok(Some(symbol_info)) => {
                if debug_mode {
                    println!("[DEBUG] Successfully retrieved LSP info for symbol: {symbol_name}");
                    if let Some(ref call_hierarchy) = symbol_info.call_hierarchy {
                        println!(
                            "[DEBUG] Call hierarchy - incoming calls: {}, outgoing calls: {}",
                            call_hierarchy.incoming_calls.len(),
                            call_hierarchy.outgoing_calls.len()
                        );
                    }
                }

                // Convert to JSON for storage
                match serde_json::to_value(&symbol_info) {
                    Ok(json) => return Some(json),
                    Err(e) => {
                        if debug_mode {
                            println!("[DEBUG] Failed to serialize LSP info to JSON: {e}");
                        }
                        return None;
                    }
                }
            }
            Ok(None) => {
                if debug_mode {
                    println!(
                        "[DEBUG] No LSP info available for symbol: {symbol_name} (attempt {attempts})"
                    );
                }
                if attempts < MAX_ATTEMPTS {
                    // Wait a bit before retry
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    continue;
                }
                return None;
            }
            Err(e) => {
                if debug_mode {
                    println!(
                        "[DEBUG] LSP query failed for symbol {symbol_name} (attempt {attempts}): {e}"
                    );
                }
                if attempts < MAX_ATTEMPTS {
                    // Wait a bit before retry
                    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
                    continue;
                }
                return None;
            }
        }
    }

    None
}

/// Find the function declaration that contains the given line using tree-sitter
/// Returns (symbol_name, declaration_line, declaration_column) if found
fn find_function_declaration_at_line(
    content: &str,
    file_path: &Path,
    target_line: u32,
    debug_mode: bool,
) -> Option<(String, u32, u32)> {
    use crate::language::factory::get_language_impl;
    use tree_sitter::Parser as TSParser;

    let extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    let language_impl = get_language_impl(extension)?;

    if debug_mode {
        println!("[DEBUG] Finding function declaration at line {target_line} in {extension} file");
    }

    let language = language_impl.get_tree_sitter_language();
    let mut parser = TSParser::new();
    if parser.set_language(&language).is_err() {
        if debug_mode {
            println!("[DEBUG] Failed to set language for function declaration search");
        }
        return None;
    }

    let tree = parser.parse(content, None)?;
    let root_node = tree.root_node();
    let source = content.as_bytes();

    // Convert 1-based line to 0-based for tree-sitter
    let target_line_zero_based = target_line.saturating_sub(1) as usize;

    if debug_mode {
        println!("[DEBUG] Searching for function declaration containing line {target_line} (0-based: {target_line_zero_based})");
    }

    // Recursively search for function declarations that contain the target line
    find_enclosing_function_node(
        root_node,
        target_line_zero_based,
        source,
        extension,
        debug_mode,
    )
}

/// Recursively search tree-sitter nodes to find the function declaration containing the target line
fn find_enclosing_function_node(
    node: tree_sitter::Node,
    target_line: usize,
    source: &[u8],
    extension: &str,
    debug_mode: bool,
) -> Option<(String, u32, u32)> {
    let node_start_line = node.start_position().row;
    let node_end_line = node.end_position().row;

    // Skip nodes that don't contain our target line
    if target_line < node_start_line || target_line > node_end_line {
        return None;
    }

    // Check if this node represents a function/method declaration
    let is_function_like = match extension {
        "rs" => matches!(node.kind(), "function_item" | "impl_item"),
        "go" => matches!(node.kind(), "function_declaration" | "method_declaration"),
        "js" | "jsx" => matches!(
            node.kind(),
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "function"
        ),
        "ts" | "tsx" => matches!(
            node.kind(),
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "function"
                | "method_signature"
        ),
        "py" => matches!(node.kind(), "function_definition"),
        "java" => matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ),
        "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hxx" | "h++" => {
            matches!(node.kind(), "function_definition" | "function_declarator")
        }
        "c" | "h" => matches!(node.kind(), "function_definition" | "function_declarator"),
        _ => node.kind().contains("function"),
    };

    if is_function_like {
        if debug_mode {
            println!(
                "[DEBUG] Found {} node at lines {}-{}, extracting identifier",
                node.kind(),
                node_start_line + 1,
                node_end_line + 1
            );
        }

        // Extract the identifier from this function-like node
        if let Some((symbol_name, line, column)) =
            extract_function_identifier_with_position(node, source, extension, debug_mode)
        {
            if debug_mode {
                println!(
                    "[DEBUG] Extracted function '{}' at line {}, column {}",
                    symbol_name,
                    line + 1,
                    column
                );
            }
            return Some((symbol_name, line as u32, column as u32));
        }
    }

    // Recursively check child nodes (depth-first to find the most specific enclosing function)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(result) =
            find_enclosing_function_node(child, target_line, source, extension, debug_mode)
        {
            return Some(result);
        }
    }

    None
}

/// Extract the identifier name and its precise position from a function-like tree-sitter node
fn extract_function_identifier_with_position(
    node: tree_sitter::Node,
    source: &[u8],
    extension: &str,
    debug_mode: bool,
) -> Option<(String, usize, usize)> {
    let mut cursor = node.walk();

    // Language-specific identifier extraction
    for child in node.children(&mut cursor) {
        let child_kind = child.kind();

        if debug_mode {
            println!(
                "[DEBUG] Examining child node: {} at line {}",
                child_kind,
                child.start_position().row + 1
            );
        }

        // Check for identifier nodes based on language
        let is_identifier = match extension {
            "rs" => child_kind == "identifier",
            "go" => child_kind == "identifier",
            "js" | "jsx" | "ts" | "tsx" => {
                matches!(child_kind, "identifier" | "property_identifier")
            }
            "py" => child_kind == "identifier",
            "java" => child_kind == "identifier",
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hxx" | "h++" | "c" | "h" => {
                child_kind == "identifier"
            }
            _ => child_kind == "identifier",
        };

        if is_identifier {
            if let Ok(name) = child.utf8_text(source) {
                if debug_mode {
                    println!(
                        "[DEBUG] Found identifier: '{}' at position ({}, {})",
                        name,
                        child.start_position().row,
                        child.start_position().column
                    );
                }
                // Skip common non-function identifiers
                if !matches!(name, "function" | "fn" | "def" | "func" | "method") {
                    return Some((
                        name.to_string(),
                        child.start_position().row,
                        child.start_position().column,
                    ));
                }
            }
        }

        // For some languages, we might need to look deeper
        if matches!(extension, "js" | "jsx" | "ts" | "tsx") && child_kind == "property_identifier" {
            if let Ok(name) = child.utf8_text(source) {
                if debug_mode {
                    println!(
                        "[DEBUG] Found property identifier: '{}' at position ({}, {})",
                        name,
                        child.start_position().row,
                        child.start_position().column
                    );
                }
                return Some((
                    name.to_string(),
                    child.start_position().row,
                    child.start_position().column,
                ));
            }
        }

        // Recursively check for nested identifiers (e.g., for complex function signatures)
        if let Some(result) =
            extract_function_identifier_with_position(child, source, extension, debug_mode)
        {
            return Some(result);
        }
    }

    None
}

/// Helper to get LSP information synchronously using spawn_blocking
fn get_lsp_symbol_info_sync(
    file_path: &Path,
    symbol_name: &str,
    line: u32,
    column: u32,
    debug_mode: bool,
    include_stdlib: bool,
) -> Option<serde_json::Value> {
    // Use spawn_blocking to run the async LSP code from within an async context
    let file_path = file_path.to_path_buf();
    let symbol_name = symbol_name.to_string();
    let symbol_name_for_error = symbol_name.clone();

    match std::thread::spawn(move || {
        // Create a new runtime in a separate thread
        let rt = match Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                if debug_mode {
                    println!("[DEBUG] Failed to create async runtime for LSP: {e}");
                }
                return None;
            }
        };

        // Use different timeouts for CI vs local environments
        let timeout_duration = if std::env::var("PROBE_CI").is_ok() {
            std::time::Duration::from_secs(30) // Much longer timeout in CI
        } else {
            std::time::Duration::from_secs(10) // Standard timeout locally
        };
        match rt.block_on(async {
            tokio::time::timeout(
                timeout_duration,
                get_lsp_symbol_info(
                    &file_path,
                    &symbol_name,
                    line,
                    column,
                    debug_mode,
                    include_stdlib,
                ),
            )
            .await
        }) {
            Ok(result) => result,
            Err(_) => {
                // Timeout is expected for non-blocking, just log at debug level
                if debug_mode {
                    println!(
                        "[DEBUG] LSP query timed out for symbol: {symbol_name} (non-blocking mode)"
                    );
                }
                None
            }
        }
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

/// Helper to get file extension as a &str
fn file_extension(path: &Path) -> &str {
    path.extension().and_then(|ext| ext.to_str()).unwrap_or("")
}

/// Find the workspace root by walking up the directory tree looking for project markers
fn find_workspace_root(file_path: &Path) -> Option<PathBuf> {
    // On Windows, avoid canonicalize() which can trigger stack overflow with junction points
    // On other platforms, canonicalize for better symlink handling
    #[cfg(target_os = "windows")]
    let start_path = file_path.to_path_buf();

    #[cfg(not(target_os = "windows"))]
    let start_path = file_path.canonicalize().ok()?;

    let mut current = start_path.parent()?;

    // Use safe path operations to avoid following symlinks/junctions
    use crate::path_safety::exists_no_follow;

    loop {
        // Check for Cargo.toml (Rust projects)
        if exists_no_follow(&current.join("Cargo.toml")) {
            return Some(current.to_path_buf());
        }

        // Check for package.json (Node.js projects)
        if exists_no_follow(&current.join("package.json")) {
            return Some(current.to_path_buf());
        }

        // Check for go.mod (Go projects)
        if exists_no_follow(&current.join("go.mod")) {
            return Some(current.to_path_buf());
        }

        // Check for pom.xml or build.gradle (Java projects)
        if exists_no_follow(&current.join("pom.xml"))
            || exists_no_follow(&current.join("build.gradle"))
        {
            return Some(current.to_path_buf());
        }

        // Check for .git directory (Git repository root)
        if exists_no_follow(&current.join(".git")) {
            return Some(current.to_path_buf());
        }

        // Move up one directory
        match current.parent() {
            Some(parent) => current = parent,
            None => break, // Reached filesystem root
        }
    }

    // Fallback to the file's parent directory
    file_path.parent().map(|p| p.to_path_buf())
}
