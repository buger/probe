use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::language::{merge_code_blocks, parse_file_for_code_blocks};
use crate::models::SearchResult;
use crate::ranking::preprocess_text;
use crate::search::file_search::get_filename_matched_queries_compat;

/// Function to check if a code block should be included based on term matches
fn filter_code_block(
    block_lines: (usize, usize),
    term_matches: &HashMap<usize, HashSet<usize>>,
    any_term: bool,
    num_queries: usize,
    filename_matched_queries: &HashSet<usize>, // New parameter for filename matches
    debug_mode: bool,                          // Added debug_mode parameter
) -> bool {
    // Note: For large files with many blocks, performance could be improved by
    // pre-computing term matches per line range instead of scanning term_matches
    // for each block. This optimization should be considered if performance
    // becomes an issue.

    let mut matched_queries = HashSet::new();

    // Check which queries have matches within the block's line range
    for (query_idx, lines) in term_matches {
        if lines
            .iter()
            .any(|&l| l >= block_lines.0 && l <= block_lines.1)
        {
            matched_queries.insert(*query_idx);
        }
    }

    // Determine if the block should be included
    let should_include = if any_term {
        // Any term mode: include if any term matches in content
        // (we don't use filename matches in any_term mode to maintain precision)
        !matched_queries.is_empty()
    } else {
        // All terms mode: include if all queries are matched either in content or filename
        (0..num_queries)
            .all(|i| filename_matched_queries.contains(&i) || matched_queries.contains(&i))
    };

    // Add debug logging
    if debug_mode {
        println!(
            "DEBUG: Considering block at lines {}-{}",
            block_lines.0, block_lines.1
        );
        println!("DEBUG: Matched queries (indices): {:?}", matched_queries);
        println!("DEBUG: Total matched queries: {}", matched_queries.len());
        if any_term {
            println!("DEBUG: Any-term mode: Include if any term matches");
        } else {
            println!("DEBUG: All-terms mode: Include if all {} queries matched (including filename matches: {:?})", 
                     num_queries, filename_matched_queries);
        }
        println!(
            "DEBUG: Block included: {} (Reason: {})",
            should_include,
            if should_include {
                "Matched criteria"
            } else {
                "Insufficient matches"
            }
        );
    }

    should_include
}

/// Function to process a file that was matched by filename
pub fn process_file_by_filename(
    path: &Path, 
    queries_terms: &[Vec<(String, String)>],
    preprocessed_queries: Option<&[Vec<String>]>, // Optional preprocessed query terms for optimization
) -> Result<SearchResult> {
    // Read the file content
    let content = fs::read_to_string(path).context(format!("Failed to read file: {:?}", path))?;
    
    // Get the filename for matching
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    
    // Use get_filename_matched_queries_compat to determine matched terms
    let matched_terms = get_filename_matched_queries_compat(&filename, queries_terms);
    
    // Create a SearchResult with filename match information
    let mut search_result = SearchResult {
        file: path.to_string_lossy().to_string(),
        lines: (1, content.lines().count()),
        node_type: "file".to_string(),
        code: content.clone(), // Clone content here to avoid the move
        matched_by_filename: Some(true),
        rank: None,
        score: None,
        tfidf_score: None,
        bm25_score: None,
        tfidf_rank: None,
        bm25_rank: None,
        new_score: None,
        file_unique_terms: Some(matched_terms.len()),
        file_total_matches: Some(0),
        file_match_rank: None,
        block_unique_terms: Some(matched_terms.len()),
        block_total_matches: Some(0),
    };

    // Use preprocessed query terms if available
    if let Some(preprocessed) = preprocessed_queries {
        let query_terms: Vec<String> = preprocessed.iter().flat_map(|terms| terms.iter().cloned()).collect();
        let unique_query_terms: HashSet<String> = query_terms.into_iter().collect();
        let block_terms = preprocess_text(&content, false);
        let block_unique_terms = if block_terms.is_empty() || unique_query_terms.is_empty() {
            0
        } else {
            block_terms.iter()
                .filter(|t| unique_query_terms.contains(*t))
                .collect::<HashSet<&String>>()
                .len()
        };
        let block_total_matches = if block_terms.is_empty() || unique_query_terms.is_empty() {
            0
        } else {
            block_terms.iter()
                .filter(|t| unique_query_terms.contains(*t))
                .count()
        };
        search_result.file_unique_terms = Some(block_unique_terms);
        search_result.file_total_matches = Some(block_total_matches);
    }

    Ok(search_result)
}

/// Function to process a file with line numbers and return SearchResult structs
pub fn process_file_with_results(
    path: &Path,
    line_numbers: &HashSet<usize>,
    allow_tests: bool,
    term_matches: Option<&HashMap<usize, HashSet<usize>>>, // Query index to line numbers
    any_term: bool, // Whether to include code blocks that match any term (true) or all terms (false)
    num_queries: usize, // Total number of queries being searched
    filename_matched_queries: HashSet<usize>, // Query indices that match the filename
    queries_terms: &[Vec<(String, String)>], // The query terms for calculating block matches
    preprocessed_queries: Option<&[Vec<String>]>, // Optional preprocessed query terms for optimization
) -> Result<Vec<SearchResult>> {
    // Read the file content
    let content = fs::read_to_string(path).context(format!("Failed to read file: {:?}", path))?;

    // Get the file extension
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    // Split the content into lines for context processing
    let lines: Vec<&str> = content.lines().collect();

    // Create SearchResult structs for each match
    let mut results = Vec::new();

    // Track which line numbers have been covered
    let mut covered_lines = HashSet::new();

    // Debug mode
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("DEBUG: Processing file with results: {:?}", path);
        println!("DEBUG:   Matched line numbers: {:?}", line_numbers);
        println!("DEBUG:   File extension: {}", extension);
        println!("DEBUG:   Total lines in file: {}", lines.len());

        // Log filename matches if present
        if !filename_matched_queries.is_empty() {
            println!(
                "DEBUG: Filename '{}' matched queries (indices): {:?}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                filename_matched_queries
            );
        }
    }

    // First try to use AST parsing
    if let Ok(code_blocks) =
        parse_file_for_code_blocks(&content, extension, line_numbers, allow_tests, term_matches)
    {
        if debug_mode {
            println!("DEBUG: AST parsing successful");
            println!("DEBUG:   Found {} code blocks", code_blocks.len());
        }

        // Merge overlapping code blocks
        let merged_blocks = merge_code_blocks(code_blocks);

        if debug_mode {
            println!("DEBUG:   After merging: {} blocks", merged_blocks.len());

            for (i, block) in merged_blocks.iter().enumerate() {
                println!(
                    "DEBUG:   Block {}: type={}, lines={}-{}",
                    i + 1,
                    block.node_type,
                    block.start_row + 1,
                    block.end_row + 1
                );
            }
        }

        // Process all blocks found by AST parsing
        for block in merged_blocks {
            // Get the line start and end based on AST
            let start_line = block.start_row + 1; // Convert to 1-based line numbers
            let end_line = block.end_row + 1;

            // Extract the full code for this block
            let full_code = if start_line > 0 && end_line <= lines.len() {
                lines[start_line - 1..end_line].join("\n")
            } else {
                "".to_string()
            };

            // Calculate block term matches
            let block_terms = preprocess_text(&full_code, false);
            
            // Use preprocessed query terms if available, otherwise generate them
            let query_terms: Vec<String> = if let Some(preprocessed) = preprocessed_queries {
                preprocessed.iter().flat_map(|terms| terms.iter().cloned()).collect()
            } else {
                queries_terms.iter()
                    .flat_map(|terms| terms.iter().map(|(_, stemmed)| stemmed.clone()))
                    .collect()
            };
            
            let unique_query_terms: HashSet<String> = query_terms.into_iter().collect();
            
            // Calculate unique terms matched in the block
            let block_unique_terms = if block_terms.is_empty() || unique_query_terms.is_empty() {
                0
            } else {
                block_terms.iter()
                    .filter(|t| unique_query_terms.contains(*t))
                    .collect::<HashSet<&String>>()
                    .len()
            };
            
            // Calculate total matches in the block
            let block_total_matches = if block_terms.is_empty() || unique_query_terms.is_empty() {
                0
            } else {
                block_terms.iter()
                    .filter(|t| unique_query_terms.contains(*t))
                    .count()
            };

            if debug_mode {
                println!(
                    "DEBUG: Block at {}-{} has {} unique term matches and {} total matches",
                    start_line, end_line, block_unique_terms, block_total_matches
                );
            }

            // Mark all lines in this block as covered
            for line_num in start_line..=end_line {
                covered_lines.insert(line_num);
            }

            // Apply term filtering if term_matches is provided
            let should_include = if let Some(term_matches_map) = term_matches {
                // Use the filter_code_block function with the filename_matched_queries parameter
                filter_code_block(
                    (start_line, end_line),
                    term_matches_map,
                    any_term,
                    num_queries,
                    &filename_matched_queries,
                    debug_mode,
                )
            } else {
                // If no term_matches provided, include all blocks
                true
            };

            if debug_mode {
                println!(
                    "DEBUG: Filtered code block at {}-{}: included={}",
                    start_line, end_line, should_include
                );
                println!(
                    "DEBUG: Block at {}-{} filtered: included={}",
                    start_line, end_line, should_include
                );
            }

            // Add to results only if it passes the filter
            if should_include {
                results.push(SearchResult {
                    file: path.to_string_lossy().to_string(),
                    lines: (start_line, end_line),
                    node_type: "file".to_string(),
                    code: full_code.clone(),
                    matched_by_filename: None,
                    rank: None,
                    score: None,
                    tfidf_score: None,
                    bm25_score: None,
                    tfidf_rank: None,
                    bm25_rank: None,
                    new_score: None,
                    file_unique_terms: Some(block_unique_terms),
                    file_total_matches: Some(block_total_matches),
                    file_match_rank: None,
                    block_unique_terms: Some(block_unique_terms),
                    block_total_matches: Some(block_total_matches),
                });
            }
        }
    } else if debug_mode {
        println!("DEBUG: AST parsing failed, using line-based context only");
    }

    // Check for any line numbers that weren't covered
    for &line_num in line_numbers {
        if !covered_lines.contains(&line_num) {
            if debug_mode {
                println!(
                    "DEBUG: Line {} not covered, using fallback context",
                    line_num
                );
                if line_num <= lines.len() {
                    println!("DEBUG:   Line content: '{}'", lines[line_num - 1].trim());
                }
            }

            // Skip fallback context for test files if allow_tests is false
            if !allow_tests && crate::language::is_test_file(path) {
                if debug_mode {
                    println!("DEBUG: Skipping fallback context for test file: {:?}", path);
                }
                continue;
            }

            // Check if the line is in a test function/module by examining its content
            if !allow_tests && line_num <= lines.len() {
                let line_content = lines[line_num - 1];
                // Simple heuristic check for test functions/modules
                if line_content.contains("fn test_")
                    || line_content.contains("#[test]")
                    || line_content.contains("#[cfg(test)]")
                    || line_content.contains("mod tests")
                {
                    if debug_mode {
                        println!(
                            "DEBUG: Skipping fallback context for test code: '{}'",
                            line_content.trim()
                        );
                    }
                    continue;
                }
            }

            // Fallback: Get context around the line (20 lines before and after)
            let context_start = line_num.saturating_sub(20); // Expanded from 10
            let context_end = std::cmp::min(line_num + 20, lines.len());

            // Skip if we don't have enough context
            if context_start >= context_end {
                continue;
            }

            // Extract the context lines - ensure context_start is at least 1 to avoid underflow
            let context_code = if context_start > 0 {
                lines[context_start - 1..context_end].join("\n")
            } else {
                lines[0..context_end].join("\n")
            };

            // Calculate block term matches
            let block_terms = preprocess_text(&context_code, false);
            
            // Use preprocessed query terms if available, otherwise generate them
            let query_terms: Vec<String> = if let Some(preprocessed) = preprocessed_queries {
                preprocessed.iter().flat_map(|terms| terms.iter().cloned()).collect()
            } else {
                queries_terms.iter()
                    .flat_map(|terms| terms.iter().map(|(_, stemmed)| stemmed.clone()))
                    .collect()
            };
            
            let unique_query_terms: HashSet<String> = query_terms.into_iter().collect();
            
            // Calculate unique terms matched in the block
            let block_unique_terms = if block_terms.is_empty() || unique_query_terms.is_empty() {
                0
            } else {
                block_terms.iter()
                    .filter(|t| unique_query_terms.contains(*t))
                    .collect::<HashSet<&String>>()
                    .len()
            };
            
            // Calculate total matches in the block
            let block_total_matches = if block_terms.is_empty() || unique_query_terms.is_empty() {
                0
            } else {
                block_terms.iter()
                    .filter(|t| unique_query_terms.contains(*t))
                    .count()
            };

            if debug_mode {
                println!(
                    "DEBUG: Context block at {}-{} has {} unique term matches and {} total matches",
                    context_start, context_end, block_unique_terms, block_total_matches
                );
                println!(
                    "DEBUG: Fallback context at lines {}-{}",
                    context_start, context_end
                );
            }

            // Apply term filtering if term_matches is provided
            let should_include = if let Some(term_matches_map) = term_matches {
                // Use the filter_code_block function with the filename_matched_queries parameter
                filter_code_block(
                    (context_start, context_end),
                    term_matches_map,
                    any_term,
                    num_queries,
                    &filename_matched_queries,
                    debug_mode,
                )
            } else {
                // If no term_matches provided, include all blocks
                true
            };

            if debug_mode {
                println!(
                    "DEBUG: Filtered context block at {}-{}: included={}",
                    context_start, context_end, should_include
                );
                println!(
                    "DEBUG: Context at {}-{} filtered: included={}",
                    context_start, context_end, should_include
                );
            }

            // Add to results only if it passes the filter
            if should_include {
                results.push(SearchResult {
                    file: path.to_string_lossy().to_string(),
                    lines: (context_start, context_end),
                    node_type: "context".to_string(), // Mark as context-based result
                    code: context_code.clone(), // Clone context_code here to avoid the move
                    matched_by_filename: None,
                    rank: None,
                    score: None,
                    tfidf_score: None,
                    bm25_score: None,
                    tfidf_rank: None,
                    bm25_rank: None,
                    new_score: None,
                    file_unique_terms: None,
                    file_total_matches: None,
                    file_match_rank: None,
                    block_unique_terms: Some(block_unique_terms),
                    block_total_matches: Some(block_total_matches),
                });
            }

            // Mark these lines as covered (even if we don't include the result)
            // This prevents duplicate processing of the same lines
            for line in context_start..=context_end {
                covered_lines.insert(line);
            }
        }
    }

    // Define a function to determine if we should return the full file
    fn should_return_full_file(coverage_percentage: f64, total_lines: usize) -> bool {
        total_lines >= 5 && coverage_percentage >= 99.0
    }

    // Calculate coverage percentage with safeguards for division by zero
    let total_lines = lines.len();
    let covered_line_count = covered_lines.len();
    let coverage_percentage = if total_lines > 0 {
        (covered_line_count as f64 / total_lines as f64) * 100.0
    } else {
        0.0
    };

    if debug_mode {
        println!(
            "DEBUG: File coverage: {}/{} lines ({:.2}%)",
            covered_line_count, total_lines, coverage_percentage
        );
    }

    // Check if we should return the full file based on coverage and minimum line count
    if false && should_return_full_file(coverage_percentage, total_lines) {
        if debug_mode {
            println!("DEBUG: Coverage exceeds 80%, returning entire file");
        }

        // Clear the previous results and return the entire file
        results.clear();

        // Calculate block term matches for the entire file
        let block_terms = preprocess_text(&content, false);
        
        // Use preprocessed query terms if available, otherwise generate them
        let query_terms: Vec<String> = if let Some(preprocessed) = preprocessed_queries {
            preprocessed.iter().flat_map(|terms| terms.iter().cloned()).collect()
        } else {
            queries_terms.iter()
                .flat_map(|terms| terms.iter().map(|(_, stemmed)| stemmed.clone()))
                .collect()
        };
        
        let unique_query_terms: HashSet<String> = query_terms.into_iter().collect();
        
        // Calculate unique terms matched in the file
        let block_unique_terms = if block_terms.is_empty() || unique_query_terms.is_empty() {
            0
        } else {
            block_terms.iter()
                .filter(|t| unique_query_terms.contains(*t))
                .collect::<HashSet<&String>>()
                .len()
        };
        
        // Calculate total matches in the file
        let block_total_matches = if block_terms.is_empty() || unique_query_terms.is_empty() {
            0
        } else {
            block_terms.iter()
                .filter(|t| unique_query_terms.contains(*t))
                .count()
        };

        if debug_mode {
            println!(
                "DEBUG: Full file has {} unique term matches and {} total matches",
                block_unique_terms, block_total_matches
            );
        }

        results.push(SearchResult {
            file: path.to_string_lossy().to_string(),
            lines: (1, total_lines),
            node_type: "file".to_string(), // Mark as full file result
            code: content.clone(), // Clone content here to avoid the move
            matched_by_filename: None,
            rank: None,
            score: None,
            tfidf_score: None,
            bm25_score: None,
            tfidf_rank: None,
            bm25_rank: None,
            new_score: None,
            file_unique_terms: None,
            file_total_matches: None,
            file_match_rank: None,
            block_unique_terms: Some(block_unique_terms),
            block_total_matches: Some(block_total_matches),
        });
    }

    // Log debug information outside the conditional block
    if debug_mode {
        println!(
            "DEBUG: Generated {} search results for file {:?}",
            results.len(),
            path
        );
    }

    Ok(results)
}
