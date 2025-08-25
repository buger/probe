use probe_code::models::SearchResult;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Merges ranked search results that are adjacent or overlapping
///
/// This function should be called AFTER ranking and limiting to merge blocks
/// that come from the same file and are adjacent or overlapping.
///
/// # Arguments
/// * `results` - A vector of already ranked and limited SearchResult objects
/// * `threshold` - Maximum number of lines between blocks to consider them adjacent (default: 5)
///
/// # Returns
/// A new vector of SearchResult objects with adjacent blocks merged
pub fn merge_ranked_blocks(
    results: Vec<SearchResult>,
    threshold: Option<usize>,
) -> Vec<SearchResult> {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
    let threshold = threshold.unwrap_or(5); // Default to 5 lines if not specified

    if results.is_empty() {
        return results;
    }

    if debug_mode {
        println!(
            "DEBUG: Starting post-rank merging of {} results with threshold {}",
            results.len(),
            threshold
        );
    }

    // Store the original count before we move results
    let original_count = results.len();

    // Group results by file - use BTreeMap for deterministic iteration order
    let mut file_blocks: BTreeMap<String, Vec<SearchResult>> = BTreeMap::new();

    for result in results {
        file_blocks
            .entry(result.file.clone())
            .or_default()
            .push(result);
    }

    let mut merged_results = Vec::new();

    // Process each file's blocks
    for (file_path, mut blocks) in file_blocks {
        if debug_mode {
            println!(
                "DEBUG: Processing {} blocks from file: {}",
                blocks.len(),
                file_path
            );
        }

        // If file only has one block, no need to merge
        if blocks.len() == 1 {
            merged_results.push(blocks.remove(0));
            continue;
        }

        // Sort blocks by start line for merging
        blocks.sort_by_key(|block| block.lines.0);

        // Sort blocks by start line
        blocks.sort_by_key(|block| block.lines.0);

        // Keep track of blocks we've already processed
        let mut processed_indices = std::collections::HashSet::new();
        let mut merged_blocks = Vec::new();

        // Process each block
        for i in 0..blocks.len() {
            if processed_indices.contains(&i) {
                continue;
            }

            // Start with the current block
            let mut current_block = blocks[i].clone();
            processed_indices.insert(i);

            // Keep track of which blocks we're merging in this group
            let mut merged_indices = vec![i];
            let mut changed = true;

            // Keep trying to merge blocks until no more merges are possible
            while changed {
                changed = false;

                // Try to merge with any remaining unprocessed block
                for (j, next_block) in blocks.iter().enumerate() {
                    if processed_indices.contains(&j) {
                        continue;
                    }

                    if should_merge_blocks(&current_block, next_block, threshold) {
                        if debug_mode {
                            println!(
                                "DEBUG: Merging blocks - current: {}-{}, next: {}-{}",
                                current_block.lines.0,
                                current_block.lines.1,
                                next_block.lines.0,
                                next_block.lines.1
                            );
                        }

                        // Merge the blocks
                        let merged_start = current_block.lines.0.min(next_block.lines.0);
                        let merged_end = current_block.lines.1.max(next_block.lines.1);
                        // Capture original ranges *before* mutating current_block
                        let current_range = current_block.lines;
                        let next_range = next_block.lines;
                        let merged_code = merge_block_content(&current_block, next_block);

                        // Use node type from the highest-ranked block
                        let merged_node_type = if current_block.rank.unwrap_or(usize::MAX)
                            <= next_block.rank.unwrap_or(usize::MAX)
                        {
                            current_block.node_type.clone()
                        } else {
                            next_block.node_type.clone()
                        };

                        // Combine scores and term statistics
                        let merged_score = merge_scores(&current_block, next_block);
                        let merged_term_stats = merge_term_statistics(&current_block, next_block);

                        // Update the current block
                        current_block.lines = (merged_start, merged_end);
                        current_block.code = merged_code;
                        current_block.node_type = merged_node_type.clone();
                        current_block.score = merged_score.0;
                        current_block.tfidf_score = merged_score.1;
                        current_block.bm25_score = merged_score.2;
                        current_block.new_score = merged_score.3;
                        current_block.block_unique_terms = merged_term_stats.0;
                        current_block.block_total_matches = merged_term_stats.1;

                        // Merge LSP information (preserve per-symbol data)
                        current_block.lsp_info = merge_lsp_info(
                            current_block.lsp_info.take(),
                            next_block.lsp_info.clone(),
                            current_range,
                            next_range,
                            &current_block.node_type,
                            &next_block.node_type,
                        );

                        // Mark this block as processed
                        processed_indices.insert(j);
                        merged_indices.push(j);
                        changed = true;
                    }
                }
            }

            // Add the merged block to results
            merged_blocks.push(current_block);
        }

        // Add the merged blocks to the final results
        merged_results.extend(merged_blocks);
    }

    if debug_mode {
        println!(
            "DEBUG: Post-rank merging complete. Merged {} blocks into {} blocks",
            original_count,
            merged_results.len()
        );
    }

    merged_results
}

/// Helper function to determine if two blocks should be merged
///
/// # Arguments
/// * `block1` - First search result
/// * `block2` - Second search result
/// * `threshold` - Maximum number of lines between blocks to consider them adjacent
///
/// # Returns
/// `true` if blocks should be merged, `false` otherwise
pub fn should_merge_blocks(block1: &SearchResult, block2: &SearchResult, threshold: usize) -> bool {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // Check if both blocks have parent_file_id, and if they match
    if let (Some(file_id1), Some(file_id2)) = (&block1.parent_file_id, &block2.parent_file_id) {
        if file_id1 != file_id2 {
            if debug_mode {
                println!("DEBUG: Blocks not merged - different parent file IDs");
            }
            return false;
        }
    } else {
        // If blocks don't have parent_file_id, check if they're from the same file
        if block1.file != block2.file {
            if debug_mode {
                println!("DEBUG: Blocks not merged - different files");
            }
            return false;
        }
    }

    // Get line ranges
    let (start1, end1) = block1.lines;
    let (start2, end2) = block2.lines;

    // Blocks should be merged if:
    // 1. They overlap
    // 2. Or they are within the threshold distance
    // 3. Or one is a comment and is adjacent to a function

    // Check for overlap first
    let overlapping = start1 <= end2 && start2 <= end1;

    // If not overlapping, check the gap size
    let distance = if overlapping {
        0
    } else if start2 > end1 {
        start2 - end1 - 1 // Subtract 1 because we want the gap size
    } else {
        start1 - end2 - 1
    };

    let comment_with_function = (block1.node_type.contains("comment")
        && is_function_like(&block2.node_type))
        || (block2.node_type.contains("comment") && is_function_like(&block1.node_type));

    let should_merge = overlapping
        || distance <= threshold
        || (comment_with_function && distance <= threshold * 2);

    if debug_mode {
        println!("DEBUG: Considering merging blocks - Block1: type='{}' lines {}-{}, Block2: type='{}' lines {}-{}, threshold: {}",
                 block1.node_type, start1, end1, block2.node_type, start2, end2, threshold);
        println!(
            "DEBUG: Should merge: {should_merge} (distance: {distance}, threshold: {threshold})"
        );
    }

    should_merge
}

/// Helper function to check if a node type represents a function-like construct
fn is_function_like(node_type: &str) -> bool {
    node_type.contains("function")
        || node_type.contains("method")
        || node_type.contains("fn")
        || node_type.contains("func")
}

/// Helper function to merge the content of two blocks
///
/// # Arguments
/// * `block1` - First search result
/// * `block2` - Second search result
///
/// # Returns
/// The merged code content
fn merge_block_content(block1: &SearchResult, block2: &SearchResult) -> String {
    // Extract line ranges
    let (start1, end1) = block1.lines;
    let (start2, end2) = block2.lines;

    // Calculate the merged range
    let merged_start = start1.min(start2);
    let merged_end = end1.max(end2);

    // If the blocks are already complete, we can use simpler logic
    if start1 == merged_start && end1 == merged_end {
        return block1.code.clone();
    }

    if start2 == merged_start && end2 == merged_end {
        return block2.code.clone();
    }

    // We need to extract the merged content from the file
    // For simplicity, we'll use the content from the blocks we have
    // This is not perfect, as we might be missing some lines in between,
    // but it's a reasonable approximation without loading the file again

    // Convert content to lines
    let lines1: Vec<&str> = block1.code.lines().collect();
    let lines2: Vec<&str> = block2.code.lines().collect();

    // Map lines to their absolute positions in the file
    // Use BTreeMap for deterministic iteration order to fix non-deterministic merging behavior
    let mut line_map: BTreeMap<usize, String> = BTreeMap::new();

    for (i, line) in lines1.iter().enumerate() {
        let abs_pos = start1 + i;
        line_map.insert(abs_pos, line.to_string());
    }

    for (i, line) in lines2.iter().enumerate() {
        let abs_pos = start2 + i;
        line_map.entry(abs_pos).or_insert_with(|| line.to_string());
    }

    // Build the merged content from the line map
    let mut merged_lines = Vec::new();
    let mut current_line = merged_start;
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // Try to open the file to fill small gaps
    let file_path = Path::new(&block1.file);
    let file_result = File::open(file_path);
    let file_content_available = file_result.is_ok();
    let _reader = file_result.map(BufReader::new).ok(); // Used for debugging purposes only

    if debug_mode {
        println!(
            "DEBUG: Current working directory: {:?}",
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("unknown"))
        );
        println!(
            "DEBUG: Attempting to read file: {:?}",
            file_path
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(file_path))
        );
        println!("DEBUG: File exists: {}", file_path.exists());
        println!("DEBUG: File can be opened: {file_content_available}");
    }

    while current_line <= merged_end {
        if let Some(line_content) = line_map.get(&current_line) {
            merged_lines.push(line_content.clone());
            current_line += 1;
        } else {
            // This is a gap in our knowledge - find the entire gap range
            let gap_start = current_line;
            let mut gap_end = current_line;

            // Find the end of the gap
            while gap_end < merged_end && !line_map.contains_key(&(gap_end + 1)) {
                gap_end += 1;
            }

            let gap_size = gap_end - gap_start + 1;

            // For small gaps (less than 10 lines), try to read the actual content
            if gap_size < 10 {
                if file_content_available {
                    if debug_mode {
                        println!(
                            "DEBUG: Attempting to fill small gap from line {} to {} from file {}",
                            gap_start, gap_end, block1.file
                        );
                    }

                    // Read the file content directly instead of using a reader clone
                    // which might have its position already moved forward
                    let file_result = File::open(Path::new(&block1.file));

                    if let Ok(file) = file_result {
                        let reader = BufReader::new(file);

                        if debug_mode {
                            println!("DEBUG: Created fresh file reader for gap");
                        }

                        // Read the file line by line
                        let mut lines_read = Vec::new();
                        let mut current_line_in_file = 1;

                        for line_content in reader.lines().map_while(Result::ok) {
                            if current_line_in_file >= gap_start && current_line_in_file <= gap_end
                            {
                                lines_read.push(line_content);
                            }

                            current_line_in_file += 1;

                            if current_line_in_file > gap_end {
                                break;
                            }
                        }

                        // Add the actual content for the gap
                        if !lines_read.is_empty() {
                            if debug_mode {
                                println!(
                                    "DEBUG: Successfully read {} lines for gap",
                                    lines_read.len()
                                );
                            }
                            merged_lines.extend(lines_read);
                            current_line = gap_end + 1;
                            continue;
                        } else if debug_mode {
                            println!("DEBUG: No lines were read for the gap (empty lines)");
                        }
                    } else if debug_mode {
                        println!("DEBUG: Could not create fresh file reader");
                    }
                } else if debug_mode {
                    println!("DEBUG: File content not available for {}", block1.file);
                }

                // For small gaps where we couldn't read the file or no lines were read,
                // include a special placeholder indicating we want to include these lines
                merged_lines.push(format!(
                    "... lines {gap_start}-{gap_end} should be included ..."
                ));
            } else {
                // Add a more informative placeholder showing how many lines were skipped for larger gaps
                merged_lines.push(format!("... lines {gap_start}-{gap_end} skipped..."));
            }

            // Move past the gap
            current_line = gap_end + 1;
        }
    }

    merged_lines.join("\n")
}

/// Helper function to merge scores from two blocks
///
/// # Arguments
/// * `block1` - First search result
/// * `block2` - Second search result
///
/// # Returns
/// Tuple of (score, tfidf_score, bm25_score, new_score) for the merged block
fn merge_scores(
    block1: &SearchResult,
    block2: &SearchResult,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    // For each score, take the maximum of the two blocks' scores
    let score = match (block1.score, block2.score) {
        (Some(s1), Some(s2)) => Some(s1.max(s2)),
        (Some(s), None) | (None, Some(s)) => Some(s),
        _ => None,
    };

    let tfidf_score = match (block1.tfidf_score, block2.tfidf_score) {
        (Some(s1), Some(s2)) => Some(s1.max(s2)),
        (Some(s), None) | (None, Some(s)) => Some(s),
        _ => None,
    };

    let bm25_score = match (block1.bm25_score, block2.bm25_score) {
        (Some(s1), Some(s2)) => Some(s1.max(s2)),
        (Some(s), None) | (None, Some(s)) => Some(s),
        _ => None,
    };

    let new_score = match (block1.new_score, block2.new_score) {
        (Some(s1), Some(s2)) => Some(s1.max(s2)),
        (Some(s), None) | (None, Some(s)) => Some(s),
        _ => None,
    };

    (score, tfidf_score, bm25_score, new_score)
}

/// Helper function to merge term statistics from two blocks
///
/// # Arguments
/// * `block1` - First search result
/// * `block2` - Second search result
///
/// # Returns
/// Tuple of (block_unique_terms, block_total_matches) for the merged block
fn merge_term_statistics(
    block1: &SearchResult,
    block2: &SearchResult,
) -> (Option<usize>, Option<usize>) {
    // For unique terms, we take the maximum (since merging blocks shouldn't reduce matched terms)
    let unique_terms = match (block1.block_unique_terms, block2.block_unique_terms) {
        (Some(t1), Some(t2)) => Some(t1.max(t2)),
        (Some(t), None) | (None, Some(t)) => Some(t),
        _ => None,
    };

    // For total matches, we sum them (this might overcount if same terms appear in both blocks)
    // A more accurate approach would require re-processing the merged content
    let total_matches = match (block1.block_total_matches, block2.block_total_matches) {
        (Some(t1), Some(t2)) => Some(t1 + t2),
        (Some(t), None) | (None, Some(t)) => Some(t),
        _ => None,
    };

    (unique_terms, total_matches)
}

/// Merge two LSP JSON blobs into a single JSON value.
/// Rules:
/// - If only one side is Some, return it unchanged (keeps single-object shape).
/// - If both are Some, normalize to `{ merged: true, symbols: [ ... ] }`.
fn merge_lsp_info(
    left: Option<Value>,
    right: Option<Value>,
    left_lines: (usize, usize),
    right_lines: (usize, usize),
    left_node_type: &str,
    right_node_type: &str,
) -> Option<Value> {
    match (left, right) {
        (None, None) => None,
        (Some(l), None) => Some(l),
        (None, Some(r)) => Some(r),
        (Some(l), Some(r)) => {
            let mut symbols = Vec::new();
            symbols.extend(normalize_symbols(&l, left_lines, left_node_type));
            symbols.extend(normalize_symbols(&r, right_lines, right_node_type));

            // Dedup symbols by (symbol, range)
            let mut seen = HashSet::<(String, u64, u64)>::new();
            symbols.retain(|s| {
                if let Some(o) = s.as_object() {
                    let name = o
                        .get("symbol")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let (start, end) = extract_lines_from_symbol(o).unwrap_or((0, 0));
                    seen.insert((name, start as u64, end as u64))
                } else {
                    true
                }
            });

            // Dedup calls within each symbol
            for s in symbols.iter_mut() {
                if let Some(obj) = s.as_object_mut() {
                    if let Some(ch) = obj
                        .get_mut("call_hierarchy")
                        .and_then(|v| v.as_object_mut())
                    {
                        dedup_calls_array(ch, "incoming_calls");
                        dedup_calls_array(ch, "outgoing_calls");
                    }
                }
            }

            Some(json!({ "merged": true, "symbols": symbols }))
        }
    }
}

fn normalize_symbols(v: &Value, lines: (usize, usize), node_type: &str) -> Vec<Value> {
    if let Some(obj) = v.as_object() {
        // Already merged shape
        if let Some(arr) = obj.get("symbols").and_then(|s| s.as_array()) {
            return arr
                .iter()
                .map(|sym| {
                    let mut s = sym.clone();
                    ensure_symbol_range_and_type(&mut s, lines, node_type);
                    s
                })
                .collect();
        }

        // Single-symbol shape -> wrap into a symbol object
        let mut sym = Map::new();
        if let Some(symbol) = obj.get("symbol").cloned() {
            sym.insert("symbol".to_string(), symbol);
        } else {
            sym.insert("symbol".to_string(), Value::String("unknown".to_string()));
        }
        if let Some(ch) = obj.get("call_hierarchy").cloned() {
            sym.insert("call_hierarchy".to_string(), ch);
        }
        if let Some(rc) = obj.get("references_count").cloned() {
            sym.insert("references_count".to_string(), rc);
        }
        sym.insert(
            "node_type".to_string(),
            Value::String(node_type.to_string()),
        );
        sym.insert("range".to_string(), json!({ "lines": [lines.0, lines.1] }));
        return vec![Value::Object(sym)];
    }
    Vec::new()
}

fn ensure_symbol_range_and_type(sym: &mut Value, lines: (usize, usize), node_type: &str) {
    if let Some(map) = sym.as_object_mut() {
        if !map.contains_key("range") {
            map.insert("range".to_string(), json!({ "lines": [lines.0, lines.1] }));
        }
        if !map.contains_key("node_type") {
            map.insert(
                "node_type".to_string(),
                Value::String(node_type.to_string()),
            );
        }
    }
}

fn extract_lines_from_symbol(obj: &Map<String, Value>) -> Option<(usize, usize)> {
    let arr = obj.get("range")?.get("lines")?.as_array()?;
    if arr.len() == 2 {
        let start = arr[0].as_u64()? as usize;
        let end = arr[1].as_u64()? as usize;
        Some((start, end))
    } else {
        None
    }
}

fn dedup_calls_array(parent: &mut Map<String, Value>, key: &str) {
    if let Some(arr) = parent.get_mut(key).and_then(|v| v.as_array_mut()) {
        let mut seen = HashSet::<(String, String, u64)>::new();
        let mut out = Vec::with_capacity(arr.len());
        for c in arr.drain(..) {
            if let Some(co) = c.as_object() {
                let name = co
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let path = co
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let line = co.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
                if seen.insert((name, path, line)) {
                    out.push(Value::Object(co.clone()));
                }
            } else {
                out.push(c);
            }
        }
        *arr = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_merge_lsp_info_both_none() {
        let result = merge_lsp_info(None, None, (1, 5), (6, 10), "function", "function");
        assert!(result.is_none());
    }

    #[test]
    fn test_merge_lsp_info_left_only() {
        let left = Some(json!({
            "symbol": "foo",
            "call_hierarchy": {
                "incoming_calls": [{"name": "bar", "line": 10}],
                "outgoing_calls": []
            }
        }));

        let result = merge_lsp_info(left.clone(), None, (1, 5), (6, 10), "function", "function");
        assert_eq!(result, left);
    }

    #[test]
    fn test_merge_lsp_info_right_only() {
        let right = Some(json!({
            "symbol": "baz",
            "call_hierarchy": {
                "incoming_calls": [],
                "outgoing_calls": [{"name": "qux", "line": 20}]
            }
        }));

        let result = merge_lsp_info(None, right.clone(), (1, 5), (6, 10), "function", "function");
        assert_eq!(result, right);
    }

    #[test]
    fn test_merge_lsp_info_both_present() {
        let left = Some(json!({
            "symbol": "foo",
            "call_hierarchy": {
                "incoming_calls": [
                    {"name": "caller1", "file_path": "file1.rs", "line": 10}
                ],
                "outgoing_calls": []
            },
            "references_count": 5
        }));

        let right = Some(json!({
            "symbol": "bar",
            "call_hierarchy": {
                "incoming_calls": [
                    {"name": "caller2", "file_path": "file2.rs", "line": 20}
                ],
                "outgoing_calls": [
                    {"name": "callee1", "file_path": "file3.rs", "line": 30}
                ]
            },
            "references_count": 3
        }));

        let result = merge_lsp_info(
            left,
            right,
            (1, 5),
            (6, 10),
            "function_declaration",
            "function_declaration",
        );

        assert!(result.is_some());
        let result_json = result.unwrap();

        // Check merged structure
        assert_eq!(result_json["merged"], true);
        assert!(result_json["symbols"].is_array());

        let symbols = result_json["symbols"].as_array().unwrap();
        assert_eq!(symbols.len(), 2);

        // Check first symbol
        assert_eq!(symbols[0]["symbol"], "foo");
        assert_eq!(symbols[0]["node_type"], "function_declaration");
        assert_eq!(symbols[0]["range"]["lines"], json!([1, 5]));

        // Check second symbol
        assert_eq!(symbols[1]["symbol"], "bar");
        assert_eq!(symbols[1]["node_type"], "function_declaration");
        assert_eq!(symbols[1]["range"]["lines"], json!([6, 10]));
    }

    #[test]
    fn test_merge_lsp_info_deduplicates_calls() {
        let left = Some(json!({
            "symbol": "foo",
            "call_hierarchy": {
                "incoming_calls": [
                    {"name": "caller1", "file_path": "file1.rs", "line": 10},
                    {"name": "caller1", "file_path": "file1.rs", "line": 10} // Duplicate
                ],
                "outgoing_calls": []
            }
        }));

        let right = Some(json!({
            "symbol": "bar",
            "call_hierarchy": {
                "incoming_calls": [
                    {"name": "caller1", "file_path": "file1.rs", "line": 10} // Same as left
                ],
                "outgoing_calls": []
            }
        }));

        let result = merge_lsp_info(left, right, (1, 5), (6, 10), "function", "function");
        assert!(result.is_some());

        let result_json = result.unwrap();
        let symbols = result_json["symbols"].as_array().unwrap();

        // First symbol should have deduplicated calls
        let first_incoming = symbols[0]["call_hierarchy"]["incoming_calls"]
            .as_array()
            .unwrap();
        assert_eq!(first_incoming.len(), 1); // Deduplicated from 2 to 1

        // Second symbol keeps its own calls
        let second_incoming = symbols[1]["call_hierarchy"]["incoming_calls"]
            .as_array()
            .unwrap();
        assert_eq!(second_incoming.len(), 1);
    }

    #[test]
    fn test_merge_lsp_info_already_merged() {
        let left = Some(json!({
            "merged": true,
            "symbols": [
                {
                    "symbol": "foo",
                    "call_hierarchy": {"incoming_calls": [], "outgoing_calls": []},
                    "node_type": "function",
                    "range": {"lines": [1, 5]}
                }
            ]
        }));

        let right = Some(json!({
            "symbol": "bar",
            "call_hierarchy": {"incoming_calls": [], "outgoing_calls": []}
        }));

        let result = merge_lsp_info(left, right, (1, 10), (11, 15), "function", "function");
        assert!(result.is_some());

        let result_json = result.unwrap();
        assert_eq!(result_json["merged"], true);

        let symbols = result_json["symbols"].as_array().unwrap();
        assert_eq!(symbols.len(), 2); // Should have both foo and bar
        assert_eq!(symbols[0]["symbol"], "foo");
        assert_eq!(symbols[1]["symbol"], "bar");
    }
}
