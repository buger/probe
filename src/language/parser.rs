use anyhow::{Context, Result};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Parser as TSParser};

use probe_code::language::factory::get_language_impl;
use probe_code::language::language_trait::LanguageImpl;
use probe_code::language::tree_cache;
use probe_code::models::CodeBlock;

/// Node type priority for deterministic selection when multiple important types match same content
/// Higher index = higher priority (more specific types should win)
const NODE_TYPE_PRIORITY: &[&str] = &[
    "compilation_unit", // Lowest priority - most general
    "function_declaration",
    "method_declaration",
    "function_item",
    "impl_item",
    "type_declaration",
    "struct_item",
    "class",             // Class keyword node should have high priority
    "class_declaration", // Class declarations should have high priority
    "global_attribute",  // Highest priority - most specific
];

/// Select the highest priority node type from a list of candidates
/// This ensures deterministic behavior when multiple important node types match the same content
#[allow(dead_code)]
fn select_priority_node_type<'a>(node_types: &'a [&'a str]) -> &'a str {
    // Find the node type with the highest priority (rightmost in the array)
    let mut best_priority = -1;
    let mut best_type = *node_types.first().unwrap_or(&"unknown");

    for &node_type in node_types {
        if let Some(priority) = NODE_TYPE_PRIORITY.iter().position(|&t| t == node_type) {
            if priority as i32 > best_priority {
                best_priority = priority as i32;
                best_type = node_type;
            }
        }
    }

    // If no priority found, use lexicographic ordering for complete determinism
    if best_priority == -1 {
        node_types.iter().min().unwrap_or(&"unknown")
    } else {
        best_type
    }
}

// Define a static cache for sparse line maps
static LINE_MAP_CACHE: Lazy<DashMap<String, SparseLineMap>> = Lazy::new(DashMap::new);

/// Sparse line map that only stores mappings for lines that are actually needed
/// This dramatically reduces memory usage and construction time compared to dense Vec approach
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct SparseLineMap {
    /// Only stores mappings for lines that have been processed
    mappings: HashMap<usize, CachedNodeInfo>,
    /// Track which line ranges have been populated to avoid redundant work
    populated_ranges: Vec<(usize, usize)>,
    /// Base offset used when this sparse map was created (for cache key consistency)
    base_offset: usize,
}

impl SparseLineMap {
    fn new(base_offset: usize) -> Self {
        Self {
            mappings: HashMap::new(),
            populated_ranges: Vec::new(),
            base_offset,
        }
    }

    /// Insert a mapping for a specific line
    fn insert(&mut self, line: usize, info: CachedNodeInfo) {
        self.mappings.insert(line, info);
    }

    /// Get mapping for a specific line
    fn get(&self, line: usize) -> Option<&CachedNodeInfo> {
        self.mappings.get(&line)
    }

    /// Mark a range as populated
    fn mark_populated(&mut self, start: usize, end: usize) {
        self.populated_ranges.push((start, end));
    }

    /// Check if a line is within any populated range
    #[allow(dead_code)]
    fn is_populated(&self, line: usize) -> bool {
        self.populated_ranges
            .iter()
            .any(|&(start, end)| line >= start && end >= line)
    }

    /// Get total number of mappings stored
    fn len(&self) -> usize {
        self.mappings.len()
    }
}

/// Calculate a deterministic hash of the content for cache validation
///
/// This uses a fast, deterministic hash function to ensure consistent cache keys
/// across program runs, fixing the inconsistent search results issue caused by
/// DefaultHasher's non-deterministic behavior.
///
/// The hash function is based on FNV-1a algorithm which is fast and provides
/// good distribution while being deterministic.
fn calculate_content_hash(content: &str) -> u64 {
    // FNV-1a hash algorithm - fast and deterministic
    // Constants for 64-bit FNV-1a
    const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in content.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// A version of NodeInfo without lifetimes for caching
#[derive(Clone, Debug)]
struct CachedNodeInfo {
    // Original node info
    start_byte: usize,
    end_byte: usize,
    start_row: usize,
    end_row: usize,
    node_kind: String,
    is_comment: bool,
    is_test: bool,                     // Test status of the original node
    original_node_is_acceptable: bool, // Was the original node an acceptable parent type?
    // Context node info (if any)
    context_node_bytes: Option<(usize, usize)>,
    context_node_rows: Option<(usize, usize)>,
    context_node_kind: Option<String>,
    context_node_is_test: Option<bool>, // Test status of the context node (if any)
    // specificity: usize, // Specificity of the original node assignment - REMOVED (unused)
    // Parent function info (if applicable)
    parent_node_type: Option<String>,
    parent_start_row: Option<usize>,
    parent_end_row: Option<usize>,
    // Representative node info - REMOVED (unused)
    // representative_start_byte: usize,
    // representative_end_byte: usize,
    // representative_start_row: usize,
    // representative_end_row: usize,
    // representative_node_kind: String,
    // is_merged_comment: bool, // Flag if this represents a merged comment+context block - REMOVED (unused)
}

impl CachedNodeInfo {
    /// Create a CachedNodeInfo from a NodeInfo and determine the representative node
    fn from_node_info(
        info: &NodeInfo<'_>,
        language_impl: &dyn LanguageImpl,
        content: &[u8],
        allow_tests: bool,
    ) -> Self {
        // Determine the representative node based on the same logic used in the live processing path
        let mut rep_node = info.node; // Default to self
                                      // let mut is_merged = false; // Removed unused variable

        if info.is_comment {
            if let Some(ctx) = info.context_node {
                if !allow_tests && !language_impl.is_test_node(&ctx, content) {
                    rep_node = ctx; // Context represents the merged block
                                    // is_merged = true; // Removed assignment to unused variable
                }
            }
        } else if !info.is_test {
            if let Some(ctx) = info.context_node {
                if !allow_tests && !language_impl.is_test_node(&ctx, content) {
                    rep_node = ctx; // Use context ancestor
                }
            }
        }
        // If info.is_test is true, rep_node remains info.node

        // Get parent function info if applicable (e.g., for struct_type nodes)
        let parent_info = if rep_node.kind() == "struct_type" {
            language_impl
                .find_parent_function(rep_node)
                .map(|parent_node| {
                    let parent_type = parent_node.kind().to_string();
                    let parent_start = parent_node.start_position().row;
                    let parent_end = parent_node.end_position().row;
                    (parent_type, parent_start, parent_end)
                })
        } else {
            None
        };

        // Check if original node is acceptable
        let original_acceptable = language_impl.is_acceptable_parent(&info.node);

        // Check if context node is a test node
        let context_test = info
            .context_node
            .map(|ctx| language_impl.is_test_node(&ctx, content));

        CachedNodeInfo {
            // Original node details
            start_byte: info.node.start_byte(),
            end_byte: info.node.end_byte(),
            start_row: info.node.start_position().row,
            end_row: info.node.end_position().row,
            node_kind: info.node.kind().to_string(),
            is_comment: info.is_comment,
            is_test: info.is_test, // Original node test status
            original_node_is_acceptable: original_acceptable,
            // Context node details
            context_node_bytes: info.context_node.map(|n| (n.start_byte(), n.end_byte())),
            context_node_rows: info
                .context_node
                .map(|n| (n.start_position().row, n.end_position().row)),
            context_node_kind: info.context_node.map(|n| n.kind().to_string()),
            context_node_is_test: context_test, // Context node test status
            // specificity: info.specificity, // Original node specificity - REMOVED (unused)
            // Parent function info
            parent_node_type: parent_info.as_ref().map(|(t, _, _)| t.clone()),
            parent_start_row: parent_info.as_ref().map(|(_, s, _)| *s),
            parent_end_row: parent_info.as_ref().map(|(_, _, e)| *e),
            // Representative node details - REMOVED (unused)
            // representative_start_byte: rep_node.start_byte(),
            // representative_end_byte: rep_node.end_byte(),
            // representative_start_row: rep_node.start_position().row,
            // representative_end_row: rep_node.end_position().row,
            // representative_node_kind: rep_node.kind().to_string(),
            // is_merged_comment: is_merged,
        }
    }
}

/// Structure to hold node information for a specific line
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct NodeInfo<'a> {
    node: Node<'a>,
    is_comment: bool,
    context_node: Option<Node<'a>>, // Represents the nearest acceptable ancestor if node itself isn't one
    is_test: bool,
    specificity: usize,
}

/// Helper function to determine if we should update the line map for a given line
#[allow(dead_code)]
fn should_update_line_map<'a>(
    line_map: &[Option<NodeInfo<'a>>],
    line: usize,
    node: Node<'a>,
    is_comment: bool,
    context_node: Option<Node<'a>>,
    specificity: usize,
) -> bool {
    match &line_map[line] {
        None => true, // No existing node, always update
        Some(current) => {
            // Special case: If current node is a comment with context, and new node is the context,
            // don't replace it (preserve the comment+context relationship)
            if current.is_comment && current.context_node.is_some() {
                if let Some(ctx) = current.context_node {
                    if ctx.id() == node.id() {
                        return false;
                    }
                }
            }

            // Special case: If new node is a comment with context, and current node is the context,
            // replace it (comment with context is more specific)
            if is_comment && context_node.is_some() {
                if let Some(ctx) = context_node {
                    if ctx.id() == current.node.id() {
                        return true;
                    }
                }
            }

            // Otherwise use specificity to decide
            specificity < current.specificity
        }
    }
}

/// Gets the previous sibling of a node in the AST
fn find_prev_sibling(node: Node<'_>) -> Option<Node<'_>> {
    let parent = node.parent()?;

    let mut cursor = parent.walk();
    let mut prev_child = None;

    for child in parent.children(&mut cursor) {
        if child.id() == node.id() {
            return prev_child;
        }
        prev_child = Some(child);
    }

    None // No previous sibling found
}

/// Find first acceptable node in a subtree
fn find_acceptable_child<'a>(node: Node<'a>, language_impl: &dyn LanguageImpl) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if language_impl.is_acceptable_parent(&child) {
            return Some(child);
        }

        // Recursive search
        if let Some(acceptable) = find_acceptable_child(child, language_impl) {
            return Some(acceptable);
        }
    }

    None // No acceptable child found
}

/// Finds the immediate next node that follows a given node in the AST
fn find_immediate_next_node(node: Node<'_>) -> Option<Node<'_>> {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // First try direct next sibling
    if let Some(next) = node.next_sibling() {
        if debug_mode {
            eprintln!(
                "DEBUG: Found immediate next sibling: type='{}', lines={}-{}",
                next.kind(),
                next.start_position().row + 1,
                next.end_position().row + 1
            );
        }
        return Some(next);
    }

    // If no direct sibling, check parent's next sibling
    if let Some(parent) = node.parent() {
        if let Some(next_parent) = parent.next_sibling() {
            if debug_mode {
                eprintln!(
                    "DEBUG: Found parent's next sibling: type='{}', lines={}-{}",
                    next_parent.kind(),
                    next_parent.start_position().row + 1,
                    next_parent.end_position().row + 1
                );
            }
            return Some(next_parent);
        }
    }

    if debug_mode {
        eprintln!("DEBUG: No immediate next node found");
    }
    None
}

/// Helper function to find the context node for a comment.
/// This is a comprehensive implementation that handles all comment context finding strategies.
fn find_comment_context_node<'a>(
    comment_node: Node<'a>,
    language_impl: &dyn LanguageImpl,
    debug_mode: bool,
) -> Option<Node<'a>> {
    let start_row = comment_node.start_position().row;

    if debug_mode {
        eprintln!(
            "DEBUG: Finding context for comment at lines {}-{}: {}",
            comment_node.start_position().row + 1,
            comment_node.end_position().row + 1,
            comment_node.kind()
        );
    }

    // Strategy 1: Try to find next non-comment sibling first (most common case for doc comments)
    let mut current_sibling = comment_node.next_sibling();

    // Skip over any comment siblings to find the next non-comment sibling
    while let Some(sibling) = current_sibling {
        if sibling.kind() == "comment"
            || sibling.kind() == "line_comment"
            || sibling.kind() == "block_comment"
            || sibling.kind() == "doc_comment"
            || sibling.kind() == "//"
        {
            // This is another comment, move to the next sibling
            current_sibling = sibling.next_sibling();
            continue;
        }

        // Found a non-comment sibling
        if language_impl.is_acceptable_parent(&sibling) {
            if debug_mode {
                eprintln!(
                    "DEBUG: Found next non-comment sibling for comment at line {}: type='{}', lines={}-{}",
                    start_row + 1,
                    sibling.kind(),
                    sibling.start_position().row + 1,
                    sibling.end_position().row + 1
                );
            }
            return Some(sibling);
        } else {
            // If next sibling isn't acceptable, check its children
            if let Some(child) = find_acceptable_child(sibling, language_impl) {
                if debug_mode {
                    eprintln!(
                        "DEBUG: Found acceptable child in next non-comment sibling for comment at line {}: type='{}', lines={}-{}",
                        start_row + 1,
                        child.kind(),
                        child.start_position().row + 1,
                        child.end_position().row + 1
                    );
                }
                return Some(child);
            }
        }

        // If we get here, this non-comment sibling wasn't acceptable, try the next one
        current_sibling = sibling.next_sibling();
    }

    // Strategy 2: If no acceptable next sibling, try previous sibling (for trailing comments)
    // But only if the comment is at the end of a block or if there's no next sibling
    // This helps ensure comments are associated with the code that follows them when possible
    let has_next_sibling = comment_node.next_sibling().is_some();

    if !has_next_sibling {
        if let Some(prev_sibling) = find_prev_sibling(comment_node) {
            if language_impl.is_acceptable_parent(&prev_sibling) {
                if debug_mode {
                    eprintln!(
                        "DEBUG: Found previous sibling for comment at line {}: type='{}', lines={}-{}",
                        start_row + 1,
                        prev_sibling.kind(),
                        prev_sibling.start_position().row + 1,
                        prev_sibling.end_position().row + 1
                    );
                }
                return Some(prev_sibling);
            } else {
                // If previous sibling isn't acceptable, check its children
                if let Some(child) = find_acceptable_child(prev_sibling, language_impl) {
                    if debug_mode {
                        eprintln!(
                            "DEBUG: Found acceptable child in previous sibling for comment at line {}: type='{}', lines={}-{}",
                            start_row + 1,
                            child.kind(),
                            child.start_position().row + 1,
                            child.end_position().row + 1
                        );
                    }
                    return Some(child);
                }
            }
        }
    }

    // Strategy 3: Check parent chain
    let mut current = comment_node;
    while let Some(parent) = current.parent() {
        if language_impl.is_acceptable_parent(&parent) {
            if debug_mode {
                eprintln!(
                    "DEBUG: Found parent for comment at line {}: type='{}', lines={}-{}",
                    start_row + 1,
                    parent.kind(),
                    parent.start_position().row + 1,
                    parent.end_position().row + 1
                );
            }
            return Some(parent);
        }
        current = parent;
    }

    // Strategy 4: Look for any immediate next node
    if let Some(next_node) = find_immediate_next_node(comment_node) {
        if language_impl.is_acceptable_parent(&next_node) {
            if debug_mode {
                eprintln!(
                    "DEBUG: Using immediate next acceptable node: type='{}', lines={}-{}",
                    next_node.kind(),
                    next_node.start_position().row + 1,
                    next_node.end_position().row + 1
                );
            }
            return Some(next_node);
        }

        // Look for acceptable child in the next node
        if let Some(child) = find_acceptable_child(next_node, language_impl) {
            if debug_mode {
                eprintln!(
                    "DEBUG: Found acceptable child in next node: type='{}', lines={}-{}",
                    child.kind(),
                    child.start_position().row + 1,
                    child.end_position().row + 1
                );
            }
            return Some(child);
        }
    }

    if debug_mode {
        eprintln!("DEBUG: No related node found for the comment");
    }
    None
}

/// Build a sparse line map that only processes nodes intersecting with requested lines plus buffer
/// This is the core optimization that avoids expensive full-tree traversal
#[allow(clippy::too_many_arguments)]
fn build_sparse_line_map<'a>(
    root_node: Node<'a>,
    line_numbers: &HashSet<usize>,
    language_impl: &dyn LanguageImpl,
    content: &[u8],
    allow_tests: bool,
    debug_mode: bool,
) -> SparseLineMap {
    // Create buffer zones around requested lines for context
    const BUFFER_SIZE: usize = 10;
    let mut target_ranges: Vec<(usize, usize)> = Vec::new();

    // Convert line numbers to ranges with buffer
    let mut sorted_lines: Vec<usize> = line_numbers.iter().cloned().collect();
    sorted_lines.sort();

    if debug_mode {
        eprintln!(
            "DEBUG: SPARSE OPTIMIZATION - Building sparse line map for {} lines",
            sorted_lines.len()
        );
    }

    // Build contiguous ranges with buffer
    if !sorted_lines.is_empty() {
        let mut range_start = sorted_lines[0].saturating_sub(BUFFER_SIZE);
        let mut range_end = sorted_lines[0] + BUFFER_SIZE;

        for &line in &sorted_lines[1..] {
            let buffered_start = line.saturating_sub(BUFFER_SIZE);
            let buffered_end = line + BUFFER_SIZE;

            if buffered_start <= range_end + BUFFER_SIZE {
                range_end = buffered_end;
            } else {
                target_ranges.push((range_start, range_end));
                range_start = buffered_start;
                range_end = buffered_end;
            }
        }
        target_ranges.push((range_start, range_end));
    }

    let base_offset = target_ranges.first().map(|(start, _)| *start).unwrap_or(0);
    let mut sparse_map = SparseLineMap::new(base_offset);

    if debug_mode {
        eprintln!(
            "DEBUG: SPARSE OPTIMIZATION - Target ranges: {target_ranges:?}, base_offset: {base_offset}"
        );
    }

    // Process only nodes that intersect with target ranges
    process_node_sparse(
        root_node,
        &mut sparse_map,
        language_impl,
        content,
        allow_tests,
        debug_mode,
        None, // Initial ancestor context is None
        &target_ranges,
    );

    // Mark all target ranges as populated
    for &(start, end) in &target_ranges {
        sparse_map.mark_populated(start, end);
    }

    if debug_mode {
        eprintln!("DEBUG: SPARSE OPTIMIZATION - Built sparse line map with {} mappings (vs full file approach)", sparse_map.len());
    }

    sparse_map
}

/// Process a node for sparse line map construction - only processes nodes intersecting target ranges
#[allow(clippy::too_many_arguments)]
fn process_node_sparse<'a>(
    node: Node<'a>,
    sparse_map: &mut SparseLineMap,
    language_impl: &dyn LanguageImpl,
    content: &[u8],
    allow_tests: bool,
    debug_mode: bool,
    current_ancestor: Option<Node<'a>>,
    target_ranges: &[(usize, usize)],
) {
    let start_row = node.start_position().row;
    let end_row = node.end_position().row;

    // OPTIMIZATION: Early filtering - skip nodes that don't intersect with any target ranges
    let mut intersects_target = false;
    for &(range_start, range_end) in target_ranges {
        if start_row <= range_end && end_row >= range_start {
            intersects_target = true;
            break;
        }
    }

    if !intersects_target {
        if debug_mode {
            eprintln!(
                "DEBUG: SPARSE - Skipping node '{}' at lines {}-{} (no intersection)",
                node.kind(),
                start_row + 1,
                end_row + 1
            );
        }
        return; // Skip this entire subtree
    }

    // Process this node since it intersects with target ranges
    let is_comment = node.kind() == "comment"
        || node.kind() == "line_comment"
        || node.kind() == "block_comment"
        || node.kind() == "doc_comment"
        || node.kind() == "//";

    let is_test = !allow_tests && language_impl.is_test_node(&node, content);
    let line_coverage = end_row.saturating_sub(start_row) + 1;
    let byte_coverage = node.end_byte().saturating_sub(node.start_byte());
    let specificity = line_coverage * 1000 + (byte_coverage / 100);

    // Determine context node
    let context_node = if is_comment {
        find_comment_context_node(node, language_impl, debug_mode)
    } else if !language_impl.is_acceptable_parent(&node) {
        current_ancestor
    } else {
        None
    };

    // Store mappings for lines within target ranges
    for line in start_row..=end_row {
        // Check if this line is within any target range
        let mut line_in_target = false;
        for &(range_start, range_end) in target_ranges {
            if line >= range_start && line <= range_end {
                line_in_target = true;
                break;
            }
        }

        if line_in_target {
            // Check if we should update using the original logic
            let should_update = if let Some(existing) = sparse_map.get(line) {
                // Recreate the original should_update_line_map logic
                // Special case: If current mapping is a comment with context, and new node is the context, don't replace
                if existing.is_comment && existing.context_node_kind.is_some() {
                    // This gets complex to check without the actual nodes, so we'll use specificity
                    specificity
                        < (existing.end_row.saturating_sub(existing.start_row) + 1) * 1000
                            + (existing.end_byte.saturating_sub(existing.start_byte) / 100)
                } else if is_comment && context_node.is_some() {
                    // If new node is a comment with context, it's more specific
                    true
                } else {
                    specificity
                        < (existing.end_row.saturating_sub(existing.start_row) + 1) * 1000
                            + (existing.end_byte.saturating_sub(existing.start_byte) / 100)
                }
            } else {
                true // No existing mapping
            };

            if should_update {
                let cached_info = CachedNodeInfo::from_node_info(
                    &NodeInfo {
                        node,
                        is_comment,
                        context_node,
                        is_test,
                        specificity,
                    },
                    language_impl,
                    content,
                    allow_tests,
                );
                sparse_map.insert(line, cached_info);

                if debug_mode {
                    eprintln!(
                        "DEBUG: SPARSE - Stored mapping for line {}: type='{}', is_comment={}, context={:?}",
                        line + 1,
                        node.kind(),
                        is_comment,
                        context_node.map(|n| n.kind())
                    );
                }
            }
        }
    }

    // Determine ancestor for children
    let next_ancestor = if language_impl.is_acceptable_parent(&node) {
        Some(node)
    } else {
        current_ancestor
    };

    // Process children recursively
    let mut cursor = node.walk();
    let mut children: Vec<Node> = node.children(&mut cursor).collect();
    children.sort_by_key(|child| (child.start_byte(), child.end_byte()));

    for child in children {
        process_node_sparse(
            child,
            sparse_map,
            language_impl,
            content,
            allow_tests,
            debug_mode,
            next_ancestor,
            target_ranges,
        );
    }
}

/// Process a node and its children in a single pass, building a comprehensive line-to-node map.
/// This version passes the nearest acceptable ancestor context down the tree.
/// OPTIMIZATION: Added line range filtering to skip AST nodes that don't intersect with requested lines.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
fn process_node<'a>(
    node: Node<'a>,
    line_map: &mut Vec<Option<NodeInfo<'a>>>,
    _extension: &str, // Keep extension if needed by language_impl methods, otherwise remove
    language_impl: &dyn LanguageImpl,
    content: &[u8],
    allow_tests: bool,
    debug_mode: bool,
    current_ancestor: Option<Node<'a>>, // The nearest acceptable ancestor found so far
    target_line_ranges: &[(usize, usize)], // OPTIMIZATION: Ranges of lines we actually need to process
    line_map_base_offset: usize,           // OPTIMIZATION: Base offset for the line_map indexing
) {
    let start_row = node.start_position().row;
    let end_row = node.end_position().row;

    // OPTIMIZATION: Early filtering - skip nodes that don't intersect with any target line ranges
    let mut intersects_target = false;
    for &(range_start, range_end) in target_line_ranges {
        // Check if node's line range [start_row, end_row] intersects with target range [range_start, range_end]
        if start_row <= range_end && end_row >= range_start {
            intersects_target = true;
            break;
        }
    }

    if !intersects_target {
        if debug_mode {
            eprintln!(
                "DEBUG: AST Node filtering - skipping node '{}' at lines {}-{} (no intersection with target ranges)",
                node.kind(),
                start_row + 1,
                end_row + 1
            );
        }
        return; // Skip this entire subtree - no intersection with requested lines
    }

    // Skip nodes that are outside the line_map bounds (adjusted for optimization)
    let adjusted_start_row = start_row.saturating_sub(line_map_base_offset);
    let adjusted_end_row = end_row.saturating_sub(line_map_base_offset);
    if adjusted_start_row >= line_map.len() {
        return;
    }

    // Determine node type and test status
    let is_comment = node.kind() == "comment"
        || node.kind() == "line_comment"
        || node.kind() == "block_comment"
        || node.kind() == "doc_comment"
        || node.kind() == "//"; // Example for some languages

    // Check if the node itself represents test code
    let is_test = !allow_tests && language_impl.is_test_node(&node, content);

    // Calculate node specificity (smaller is more specific)
    let line_coverage = end_row.saturating_sub(start_row) + 1;
    let byte_coverage = node.end_byte().saturating_sub(node.start_byte());
    let specificity = line_coverage * 1000 + (byte_coverage / 100); // Example specificity calculation

    // Determine the context_node for this node
    let context_node = if is_comment {
        // For comments, find the related code node (e.g., the function it documents)
        // This function might still need to look up/around, but doesn't use the ancestor cache.
        find_comment_context_node(node, language_impl, debug_mode)
    } else {
        // For non-comments, if the node itself isn't an acceptable block boundary,
        // use the ancestor context passed down. Otherwise, it defines its own context (None).
        if !language_impl.is_acceptable_parent(&node) {
            current_ancestor
        } else {
            None // This node is an acceptable parent, it starts a new context.
        }
    };

    // Update the line map for each line covered by this node (using adjusted indexing)
    // Ensure end_row does not exceed line_map bounds
    let effective_end_row = std::cmp::min(adjusted_end_row, line_map.len().saturating_sub(1));
    for adjusted_line in adjusted_start_row..=effective_end_row {
        // Convert back to original line number for should_update_line_map call
        let original_line = adjusted_line + line_map_base_offset;

        // Skip if original line is outside target ranges (additional safety check)
        let mut line_in_target = false;
        for &(range_start, range_end) in target_line_ranges {
            if original_line >= range_start && original_line <= range_end {
                line_in_target = true;
                break;
            }
        }
        if !line_in_target {
            continue;
        }

        // Determine if this node is a better fit for the line than the current entry
        let should_update = should_update_line_map(
            line_map,
            adjusted_line,
            node,
            is_comment,
            context_node,
            specificity,
        );

        if should_update {
            // Store info about the node covering this line
            line_map[adjusted_line] = Some(NodeInfo {
                node,
                is_comment,
                context_node, // Store the determined context (parent ancestor or None)
                is_test,      // Store the test status of this specific node
                specificity,
            });
        }
    }

    // Determine the ancestor context to pass down to children
    let next_ancestor = if language_impl.is_acceptable_parent(&node) {
        // If this node is an acceptable parent, it becomes the context for its children
        Some(node)
    } else {
        // Otherwise, children inherit the same context as this node
        current_ancestor
    };

    // Process children recursively (depth-first traversal)
    // IMPORTANT: Collect and sort children to ensure deterministic processing order
    // This fixes the non-deterministic search results issue caused by inconsistent
    // node traversal order across program runs.
    let mut cursor = node.walk();
    let mut children: Vec<Node> = node.children(&mut cursor).collect();

    // Sort children by their byte positions to ensure consistent processing order
    children.sort_by_key(|child| (child.start_byte(), child.end_byte()));

    for child in children {
        process_node(
            child,
            line_map,
            _extension,
            language_impl,
            content,
            allow_tests,
            debug_mode,
            next_ancestor,        // Pass the determined ancestor context down
            target_line_ranges,   // OPTIMIZATION: Pass target ranges for filtering
            line_map_base_offset, // OPTIMIZATION: Pass base offset for indexing
        );
    }
}

/// Process a sparse line map to extract code blocks
/// This function works with the new sparse line map format for better performance
fn process_sparse_line_map(
    sparse_line_map: &SparseLineMap,
    line_numbers: &HashSet<usize>,
    _language_impl: &dyn LanguageImpl,
    _content: &str,
    allow_tests: bool,
    debug_mode: bool,
) -> Result<Vec<CodeBlock>> {
    let mut code_blocks: Vec<CodeBlock> = Vec::new();
    let mut seen_block_spans: HashSet<(usize, usize)> = HashSet::new();

    // Sort line numbers for deterministic processing order
    let mut sorted_lines: Vec<usize> = line_numbers.iter().cloned().collect();
    sorted_lines.sort();

    for line in sorted_lines {
        let line_idx = line.saturating_sub(1); // Adjust for 0-based indexing

        if debug_mode {
            eprintln!("DEBUG: Processing line {line} from sparse cache");
        }

        if let Some(info) = sparse_line_map.get(line_idx) {
            if debug_mode {
                eprintln!(
                    "DEBUG: Found sparse cached node info for line {}: original_type='{}', original_lines={}-{}, is_comment={}, is_test={}, context_kind={:?}, context_lines={:?}, context_is_test={:?}",
                    line,
                    info.node_kind,
                    info.start_row + 1,
                    info.end_row + 1,
                    info.is_comment,
                    info.is_test,
                    info.context_node_kind,
                    info.context_node_rows.map(|(s, e)| (s + 1, e + 1)),
                    info.context_node_is_test
                );
            }

            // Use the same logic as the dense cached line map processing
            // ... (this will be the same block processing logic)

            // Determine which block to potentially create based on cached info
            let mut potential_block: Option<CodeBlock> = None;
            let mut block_key: Option<(usize, usize)> = None;

            // Handle Comments
            if info.is_comment {
                if debug_mode {
                    eprintln!("DEBUG: Sparse Cache: Handling comment node at line {line}");
                }
                // Check for context node
                if let (Some(ctx_rows), Some(ctx_bytes), Some(ctx_kind), Some(ctx_is_test)) = (
                    info.context_node_rows,
                    info.context_node_bytes,
                    &info.context_node_kind,
                    info.context_node_is_test,
                ) {
                    // For comments, always use parent context regardless of test status
                    // For non-comments, respect the allow_tests setting for the context node
                    let should_use_context = info.is_comment || allow_tests || !ctx_is_test;

                    if !should_use_context {
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Sparse Cache: Skipping test context node at lines {}-{}, type: {}",
                                ctx_rows.0 + 1,
                                ctx_rows.1 + 1,
                                ctx_kind
                            );
                        }
                    } else {
                        // Create a merged block
                        let merged_start_row = std::cmp::min(info.start_row, ctx_rows.0);
                        let merged_end_row = std::cmp::max(info.end_row, ctx_rows.1);
                        let merged_start_byte = std::cmp::min(info.start_byte, ctx_bytes.0);
                        let merged_end_byte = std::cmp::max(info.end_byte, ctx_bytes.1);

                        block_key = Some((merged_start_row, merged_end_row));
                        if !seen_block_spans.contains(&block_key.unwrap()) {
                            potential_block = Some(CodeBlock {
                                start_row: merged_start_row,
                                end_row: merged_end_row,
                                start_byte: merged_start_byte,
                                end_byte: merged_end_byte,
                                node_type: ctx_kind.clone(),
                                parent_node_type: None,
                                parent_start_row: None,
                                parent_end_row: None,
                            });
                            if debug_mode {
                                eprintln!(
                                    "DEBUG: Sparse Cache: Potential merged block (comment + context) at lines {}-{}, type: {}",
                                    merged_start_row + 1, merged_end_row + 1, ctx_kind
                                );
                            }
                        }
                        if seen_block_spans.contains(&block_key.unwrap())
                            || potential_block.is_some()
                        {
                            seen_block_spans.insert(block_key.unwrap());
                            seen_block_spans.insert((info.start_row, info.end_row));
                            if let Some(block) = potential_block {
                                // Filter out complete test function blocks when allow_tests=false
                                // Only filter if this is a complete test function block (merged from comment + full test context)
                                // and NOT just a comment that happens to have test context
                                let should_filter = !allow_tests
                                    && ctx_is_test
                                    && block.node_type == "function_item"
                                    && block.start_row == ctx_rows.0
                                    && block.end_row == ctx_rows.1
                                    && (block.end_row - block.start_row) > 10; // Only filter large function blocks, not small comments

                                if should_filter {
                                    if debug_mode {
                                        eprintln!(
                                            "DEBUG: Sparse Cache: Filtering out complete test function block at lines {}-{}, type: {}",
                                            block.start_row + 1, block.end_row + 1, block.node_type
                                        );
                                    }
                                    // Skip adding this block
                                } else {
                                    code_blocks.push(block);
                                }
                            }
                            continue;
                        }
                    }
                }

                // Add individual comment if not merged
                if potential_block.is_none() {
                    block_key = Some((info.start_row, info.end_row));
                    if !seen_block_spans.contains(&block_key.unwrap()) {
                        potential_block = Some(CodeBlock {
                            start_row: info.start_row,
                            end_row: info.end_row,
                            start_byte: info.start_byte,
                            end_byte: info.end_byte,
                            node_type: info.node_kind.clone(),
                            parent_node_type: None,
                            parent_start_row: None,
                            parent_end_row: None,
                        });
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Sparse Cache: Potential individual comment block at lines {}-{}",
                                info.start_row + 1,
                                info.end_row + 1
                            );
                        }
                    }
                }
            }
            // Handle Non-Comments
            else {
                // Skip original test nodes if not allowed
                if !allow_tests && info.is_test {
                    if debug_mode {
                        eprintln!(
                            "DEBUG: Sparse Cache: Skipping original test node at lines {}-{}",
                            info.start_row + 1,
                            info.end_row + 1
                        );
                    }
                    continue;
                }

                // Check for context node (ancestor)
                if let (Some(ctx_rows), Some(ctx_bytes), Some(ctx_kind), Some(ctx_is_test)) = (
                    info.context_node_rows,
                    info.context_node_bytes,
                    &info.context_node_kind,
                    info.context_node_is_test,
                ) {
                    if !allow_tests && ctx_is_test {
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Sparse Cache: Skipping test context node (ancestor) at lines {}-{}",
                                ctx_rows.0 + 1, ctx_rows.1 + 1
                            );
                        }
                    } else {
                        // Use context node
                        block_key = Some((ctx_rows.0, ctx_rows.1));
                        if !seen_block_spans.contains(&block_key.unwrap()) {
                            potential_block = Some(CodeBlock {
                                start_row: ctx_rows.0,
                                end_row: ctx_rows.1,
                                start_byte: ctx_bytes.0,
                                end_byte: ctx_bytes.1,
                                node_type: ctx_kind.clone(),
                                parent_node_type: info.parent_node_type.clone(),
                                parent_start_row: info.parent_start_row,
                                parent_end_row: info.parent_end_row,
                            });
                            if debug_mode {
                                eprintln!(
                                    "DEBUG: Sparse Cache: Potential context node (ancestor) block at lines {}-{}",
                                    ctx_rows.0 + 1, ctx_rows.1 + 1
                                );
                            }
                        }
                        if seen_block_spans.contains(&block_key.unwrap())
                            || potential_block.is_some()
                        {
                            seen_block_spans.insert(block_key.unwrap());
                            if let Some(block) = potential_block {
                                code_blocks.push(block);
                            }
                            continue;
                        }
                    }
                }

                // Check if original node itself is acceptable
                if potential_block.is_none() && info.original_node_is_acceptable {
                    block_key = Some((info.start_row, info.end_row));
                    if !seen_block_spans.contains(&block_key.unwrap()) {
                        potential_block = Some(CodeBlock {
                            start_row: info.start_row,
                            end_row: info.end_row,
                            start_byte: info.start_byte,
                            end_byte: info.end_byte,
                            node_type: info.node_kind.clone(),
                            parent_node_type: info.parent_node_type.clone(),
                            parent_start_row: info.parent_start_row,
                            parent_end_row: info.parent_end_row,
                        });
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Sparse Cache: Potential acceptable original node block at lines {}-{}",
                                info.start_row + 1, info.end_row + 1
                            );
                        }
                    }
                }
            }

            // Add the potential block if one was determined
            if let (Some(block), Some(key)) = (potential_block, block_key) {
                if !seen_block_spans.contains(&key) {
                    seen_block_spans.insert(key);
                    if debug_mode {
                        eprintln!(
                            "DEBUG: Sparse Cache: Added new block at lines {}-{}, type: {}",
                            key.0 + 1,
                            key.1 + 1,
                            block.node_type
                        );
                    }
                    code_blocks.push(block);
                }
            }
        } else if debug_mode {
            eprintln!("DEBUG: Sparse Cache: No cached node info found for line {line}");
        }
    }

    // Apply same deduplication logic as original
    code_blocks.sort_by_key(|block| block.start_row);
    let mut final_code_blocks: Vec<CodeBlock> = Vec::new();

    // Add comments first
    for block in code_blocks
        .iter()
        .filter(|b| b.node_type.contains("comment") || b.node_type == "/*" || b.node_type == "*/")
    {
        final_code_blocks.push(block.clone());
    }

    // Add non-comments with deduplication
    for block in code_blocks
        .iter()
        .filter(|b| !b.node_type.contains("comment") && b.node_type != "/*" && b.node_type != "*/")
    {
        let mut should_add = true;
        let mut blocks_to_remove: Vec<usize> = Vec::new();

        let important_block_types = [
            "function_declaration",
            "method_declaration",
            "function_item",
            "impl_item",
            "type_declaration",
            "struct_item",
            "block_comment",
            "compilation_unit",
            "global_attribute",
        ];
        let is_important = important_block_types.contains(&block.node_type.as_str());

        for (idx, prev_block) in final_code_blocks.iter().enumerate() {
            if prev_block.node_type.contains("comment")
                || prev_block.node_type == "/*"
                || prev_block.node_type == "*/"
            {
                continue;
            }

            let prev_is_important = important_block_types.contains(&prev_block.node_type.as_str());

            if block.start_row <= prev_block.end_row && block.end_row >= prev_block.start_row {
                if block.start_row >= prev_block.start_row && block.end_row <= prev_block.end_row {
                    if is_important && !prev_is_important {
                        // Keep both
                    } else if !is_important && prev_is_important {
                        should_add = false;
                        break;
                    } else {
                        // Use priority-based selection
                        let current_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == block.node_type.as_str());
                        let prev_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == prev_block.node_type.as_str());

                        match (current_priority, prev_priority) {
                            (Some(cur_pri), Some(prev_pri)) => {
                                if cur_pri > prev_pri {
                                    blocks_to_remove.push(idx);
                                } else {
                                    should_add = false;
                                    break;
                                }
                            }
                            _ => {
                                blocks_to_remove.push(idx);
                            }
                        }
                    }
                } else if prev_block.start_row >= block.start_row
                    && prev_block.end_row <= block.end_row
                {
                    // Similar logic for other containment case
                    if is_important && !prev_is_important {
                        // Keep both
                    } else if !is_important && prev_is_important {
                        should_add = false;
                        break;
                    } else {
                        let current_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == block.node_type.as_str());
                        let prev_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == prev_block.node_type.as_str());

                        match (current_priority, prev_priority) {
                            (Some(cur_pri), Some(prev_pri)) => {
                                if cur_pri > prev_pri {
                                    blocks_to_remove.push(idx);
                                } else {
                                    should_add = false;
                                    break;
                                }
                            }
                            _ => {
                                should_add = false;
                                break;
                            }
                        }
                    }
                } else {
                    // Partial overlap - skip current block
                    should_add = false;
                    break;
                }
            }
        }

        // Remove blocks marked for removal
        for idx in blocks_to_remove.iter().rev() {
            final_code_blocks.remove(*idx);
        }

        if should_add {
            final_code_blocks.push(block.clone());
        }
    }

    final_code_blocks.sort_by_key(|block| block.start_row);
    Ok(final_code_blocks)
}

/// Process a cached line map to extract code blocks (LEGACY - for compatibility)
#[allow(dead_code)]
fn process_cached_line_map(
    cached_line_map: &[Option<CachedNodeInfo>],
    line_numbers: &HashSet<usize>,
    _language_impl: &dyn LanguageImpl, // Not used directly, logic relies on cached info
    _content: &str,                    // Not used directly, logic relies on cached info
    allow_tests: bool,
    debug_mode: bool,
) -> Result<Vec<CodeBlock>> {
    let mut code_blocks: Vec<CodeBlock> = Vec::new();
    // Use a HashSet to track the start/end rows of blocks already added
    let mut seen_block_spans: HashSet<(usize, usize)> = HashSet::new();

    // Process each line number using the cached map
    // Sort line numbers for deterministic processing order to fix non-deterministic behavior
    let mut sorted_lines: Vec<usize> = line_numbers.iter().cloned().collect();
    sorted_lines.sort();

    for line in sorted_lines {
        let line_idx = line.saturating_sub(1); // Adjust for 0-based indexing

        if debug_mode {
            eprintln!("DEBUG: Processing line {line} from cache");
        }

        if line_idx >= cached_line_map.len() {
            if debug_mode {
                eprintln!("DEBUG: Line {line} is out of bounds (Cache)");
            }
            continue;
        }

        if let Some(info) = &cached_line_map[line_idx] {
            if debug_mode {
                eprintln!(
                    "DEBUG: Found cached node info for line {}: original_type='{}', original_lines={}-{}, is_comment={}, is_test={}, context_kind={:?}, context_lines={:?}",
                    line,
                    info.node_kind,
                    info.start_row + 1,
                    info.end_row + 1,
                    info.is_comment,
                    info.is_test,
                    info.context_node_kind,
                    info.context_node_rows.map(|(s, e)| (s + 1, e + 1))
                );
            }

            // Determine which block to potentially create based on cached info
            let mut potential_block: Option<CodeBlock> = None;
            let mut block_key: Option<(usize, usize)> = None; // Key for seen_block_spans

            // --- Replicate Cache Miss Logic using CachedNodeInfo ---

            // 1. Handle Comments
            if info.is_comment {
                if debug_mode {
                    eprintln!("DEBUG: Cache: Handling comment node at line {line}");
                }
                // Check for context node
                if let (Some(ctx_rows), Some(ctx_bytes), Some(ctx_kind), Some(ctx_is_test)) = (
                    info.context_node_rows,
                    info.context_node_bytes,
                    &info.context_node_kind,
                    info.context_node_is_test,
                ) {
                    // Check test status of the context node
                    if !allow_tests && ctx_is_test {
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Cache: Skipping test context node at lines {}-{}, type: {}",
                                ctx_rows.0 + 1,
                                ctx_rows.1 + 1,
                                ctx_kind
                            );
                        }
                        // Fall through to potentially add individual comment if context is skipped
                    } else {
                        // Create a merged block
                        let merged_start_row = std::cmp::min(info.start_row, ctx_rows.0);
                        let merged_end_row = std::cmp::max(info.end_row, ctx_rows.1);
                        let merged_start_byte = std::cmp::min(info.start_byte, ctx_bytes.0);
                        let merged_end_byte = std::cmp::max(info.end_byte, ctx_bytes.1);

                        block_key = Some((merged_start_row, merged_end_row));
                        if !seen_block_spans.contains(&block_key.unwrap()) {
                            potential_block = Some(CodeBlock {
                                start_row: merged_start_row,
                                end_row: merged_end_row,
                                start_byte: merged_start_byte,
                                end_byte: merged_end_byte,
                                node_type: ctx_kind.clone(), // Use context kind for merged block
                                parent_node_type: None, // Consistent with original miss path logic
                                parent_start_row: None,
                                parent_end_row: None,
                            });
                            if debug_mode {
                                eprintln!(
                                    "DEBUG: Cache: Potential merged block (comment + context) at lines {}-{}, type: {}",
                                    merged_start_row + 1, merged_end_row + 1, ctx_kind
                                );
                            }
                        }
                        // If we created a merged block, we don't add the individual comment later
                        // So we continue the outer loop here if the block_key was already seen or if we created a potential block
                        if seen_block_spans.contains(&block_key.unwrap())
                            || potential_block.is_some()
                        {
                            if seen_block_spans.contains(&block_key.unwrap()) && debug_mode {
                                eprintln!(
                                    "DEBUG: Cache: Merged block span {}-{} already seen",
                                    block_key.unwrap().0 + 1,
                                    block_key.unwrap().1 + 1
                                );
                            }
                            // Add the key even if block wasn't added, to prevent reprocessing context
                            seen_block_spans.insert(block_key.unwrap());
                            // Also mark original comment span as seen if merged
                            seen_block_spans.insert((info.start_row, info.end_row));
                            if let Some(block) = potential_block {
                                code_blocks.push(block);
                            }
                            continue; // Move to next line number
                        }
                    }
                }

                // Add individual comment if not merged or if context was skipped
                if potential_block.is_none() {
                    block_key = Some((info.start_row, info.end_row));
                    if !seen_block_spans.contains(&block_key.unwrap()) {
                        potential_block = Some(CodeBlock {
                            start_row: info.start_row,
                            end_row: info.end_row,
                            start_byte: info.start_byte,
                            end_byte: info.end_byte,
                            node_type: info.node_kind.clone(),
                            parent_node_type: None,
                            parent_start_row: None,
                            parent_end_row: None,
                        });
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Cache: Potential individual comment block at lines {}-{}",
                                info.start_row + 1,
                                info.end_row + 1
                            );
                        }
                    } else if debug_mode {
                        eprintln!(
                            "DEBUG: Cache: Individual comment span {}-{} already seen",
                            block_key.unwrap().0 + 1,
                            block_key.unwrap().1 + 1
                        );
                    }
                }
            }
            // 2. Handle Non-Comments
            else {
                // Skip original test nodes if not allowed
                if !allow_tests && info.is_test {
                    if debug_mode {
                        eprintln!(
                            "DEBUG: Cache: Skipping original test node at lines {}-{}",
                            info.start_row + 1,
                            info.end_row + 1
                        );
                    }
                    continue; // Move to next line number
                }

                // Check for context node (ancestor)
                if let (Some(ctx_rows), Some(ctx_bytes), Some(ctx_kind), Some(ctx_is_test)) = (
                    info.context_node_rows,
                    info.context_node_bytes,
                    &info.context_node_kind,
                    info.context_node_is_test,
                ) {
                    // Check test status of the context node
                    if !allow_tests && ctx_is_test {
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Cache: Skipping test context node (ancestor) at lines {}-{}",
                                ctx_rows.0 + 1, ctx_rows.1 + 1
                            );
                        }
                        // Fall through to check original node if context is skipped
                    } else {
                        // Use context node
                        block_key = Some((ctx_rows.0, ctx_rows.1));
                        if !seen_block_spans.contains(&block_key.unwrap()) {
                            potential_block = Some(CodeBlock {
                                start_row: ctx_rows.0,
                                end_row: ctx_rows.1,
                                start_byte: ctx_bytes.0,
                                end_byte: ctx_bytes.1,
                                node_type: ctx_kind.clone(),
                                // Parent info comes from CachedNodeInfo, which derived it based on the representative node (potentially the context)
                                parent_node_type: info.parent_node_type.clone(),
                                parent_start_row: info.parent_start_row,
                                parent_end_row: info.parent_end_row,
                            });
                            if debug_mode {
                                eprintln!(
                                    "DEBUG: Cache: Potential context node (ancestor) block at lines {}-{}",
                                    ctx_rows.0 + 1, ctx_rows.1 + 1
                                );
                            }
                        } else if debug_mode {
                            eprintln!(
                                "DEBUG: Cache: Context node span {}-{} already seen",
                                block_key.unwrap().0 + 1,
                                block_key.unwrap().1 + 1
                            );
                        }
                        // If we used the context node (or it was already seen), skip checking the original node
                        if seen_block_spans.contains(&block_key.unwrap())
                            || potential_block.is_some()
                        {
                            seen_block_spans.insert(block_key.unwrap()); // Mark context as seen
                            if let Some(block) = potential_block {
                                code_blocks.push(block);
                            }
                            continue; // Move to next line number
                        }
                    }
                }

                // Check if original node itself is acceptable (and wasn't skipped as test)
                // This check happens if there was no context node, or if the context node was skipped (e.g., test)
                if potential_block.is_none() && info.original_node_is_acceptable {
                    block_key = Some((info.start_row, info.end_row));
                    if !seen_block_spans.contains(&block_key.unwrap()) {
                        potential_block = Some(CodeBlock {
                            start_row: info.start_row,
                            end_row: info.end_row,
                            start_byte: info.start_byte,
                            end_byte: info.end_byte,
                            node_type: info.node_kind.clone(),
                            // Parent info comes from CachedNodeInfo, derived based on representative node (original node in this case)
                            parent_node_type: info.parent_node_type.clone(),
                            parent_start_row: info.parent_start_row,
                            parent_end_row: info.parent_end_row,
                        });
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Cache: Potential acceptable original node block at lines {}-{}",
                                info.start_row + 1, info.end_row + 1
                            );
                        }
                    } else if debug_mode {
                        eprintln!(
                            "DEBUG: Cache: Original acceptable node span {}-{} already seen",
                            block_key.unwrap().0 + 1,
                            block_key.unwrap().1 + 1
                        );
                    }
                }
            }

            // Add the potential block if one was determined, applying priority-based selection for overlapping spans
            if let (Some(block), Some(key)) = (potential_block, block_key) {
                if seen_block_spans.contains(&key) {
                    // Span already seen - check if current block has higher priority
                    if let Some(existing_idx) = code_blocks
                        .iter()
                        .position(|b| (b.start_row, b.end_row) == key)
                    {
                        let existing_node_type = code_blocks[existing_idx].node_type.clone();

                        let current_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == block.node_type.as_str());
                        let existing_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == existing_node_type.as_str());

                        match (current_priority, existing_priority) {
                            (Some(cur_pri), Some(exist_pri)) if cur_pri > exist_pri => {
                                // Replace with higher priority node
                                if debug_mode {
                                    eprintln!(
                                        "DEBUG: Cache: Replaced block at lines {}-{} with higher priority type: {} > {}",
                                        key.0 + 1, key.1 + 1, block.node_type, existing_node_type
                                    );
                                }
                                code_blocks[existing_idx] = block;
                            }
                            _ => {
                                // Keep existing (higher or equal priority)
                                if debug_mode {
                                    eprintln!(
                                        "DEBUG: Cache: Keeping existing block at lines {}-{} with priority: {} >= {}",
                                        key.0 + 1, key.1 + 1,
                                        existing_node_type, block.node_type
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // New span - add it
                    seen_block_spans.insert(key);
                    if debug_mode {
                        eprintln!(
                            "DEBUG: Cache: Added new block at lines {}-{}, type: {}",
                            key.0 + 1,
                            key.1 + 1,
                            block.node_type
                        );
                    }
                    code_blocks.push(block);
                }
            }
        } else if debug_mode {
            eprintln!("DEBUG: Cache: No cached node info found for line {line}");
        }
    }

    // Removed extra closing brace that was here

    // Sort the blocks generated from the cache
    code_blocks.sort_by_key(|block| block.start_row);

    // --- Apply the exact same deduplication logic as the cache miss path ---
    let mut final_code_blocks: Vec<CodeBlock> = Vec::new();

    // Add comments first
    for block in code_blocks
        .iter()
        .filter(|b| b.node_type.contains("comment") || b.node_type == "/*" || b.node_type == "*/")
    {
        final_code_blocks.push(block.clone());
    }

    // Add non-comments, using the improved deduplication logic
    for block in code_blocks
        .iter() // Use iter() here as we pushed clones earlier
        .filter(|b| !b.node_type.contains("comment") && b.node_type != "/*" && b.node_type != "*/")
    {
        let mut should_add = true;
        let mut blocks_to_remove: Vec<usize> = Vec::new();

        // Define important block types that should be preserved
        let important_block_types = [
            "function_declaration",
            "method_declaration",
            "function_item",
            "impl_item",
            "type_declaration",
            "struct_item",
            "block_comment", // Keep this? Seems odd for non-comment filter but matches original
            "compilation_unit", // Root-level AST node - critical for content extraction
            "global_attribute", // Assembly-level attributes - critical for C# code
        ];
        let is_important = important_block_types.contains(&block.node_type.as_str());

        // Check if this block overlaps with any of the previous blocks in final_code_blocks
        for (idx, prev_block) in final_code_blocks.iter().enumerate() {
            if prev_block.node_type.contains("comment")
                || prev_block.node_type == "/*"
                || prev_block.node_type == "*/"
            {
                continue; // Skip comments already added
            }

            let prev_is_important = important_block_types.contains(&prev_block.node_type.as_str());

            // Check if blocks overlap
            if block.start_row <= prev_block.end_row && block.end_row >= prev_block.start_row {
                // Case 1: Current block is contained within previous block
                if block.start_row >= prev_block.start_row && block.end_row <= prev_block.end_row {
                    if debug_mode {
                        eprintln!(
                            "DEBUG: Cache Dedupe: Current block contained: type='{}', lines={}-{} (in type='{}', lines={}-{})",
                            block.node_type, block.start_row + 1, block.end_row + 1,
                            prev_block.node_type, prev_block.start_row + 1, prev_block.end_row + 1
                        );
                    }
                    if is_important && !prev_is_important {
                        if debug_mode {
                            eprintln!("DEBUG: Cache Dedupe: Keeping important contained block");
                        }
                        // Keep both - don't remove, don't skip add
                    } else if !is_important && prev_is_important {
                        if debug_mode {
                            eprintln!(
                                "DEBUG: Cache Dedupe: Skipping non-important contained block"
                            );
                        }
                        should_add = false;
                        break;
                    } else {
                        // Both important or both not - use priority-based selection for determinism
                        let current_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == block.node_type.as_str());
                        let prev_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == prev_block.node_type.as_str());

                        match (current_priority, prev_priority) {
                            (Some(cur_pri), Some(prev_pri)) => {
                                if cur_pri > prev_pri {
                                    // Current block has higher priority - keep it, remove previous
                                    if debug_mode {
                                        eprintln!("DEBUG: Cache Dedupe: Replacing block with higher priority type: {} > {}", 
                                                block.node_type, prev_block.node_type);
                                    }
                                    blocks_to_remove.push(idx);
                                } else {
                                    // Previous block has higher or equal priority - keep previous, skip current
                                    if debug_mode {
                                        eprintln!("DEBUG: Cache Dedupe: Skipping block in favor of higher priority type: {} >= {}", 
                                                prev_block.node_type, block.node_type);
                                    }
                                    should_add = false;
                                    break;
                                }
                            }
                            _ => {
                                // Fallback: prefer contained (current) block for consistency
                                if debug_mode {
                                    eprintln!("DEBUG: Cache Dedupe: Replacing outer block with contained block (no priority)");
                                }
                                blocks_to_remove.push(idx);
                            }
                        }
                    }
                }
                // Case 2: Previous block is contained within current block
                else if prev_block.start_row >= block.start_row
                    && prev_block.end_row <= block.end_row
                {
                    if debug_mode {
                        eprintln!(
                            "DEBUG: Cache Dedupe: Previous block contained: type='{}', lines={}-{} (contains type='{}', lines={}-{})",
                            block.node_type, block.start_row + 1, block.end_row + 1,
                            prev_block.node_type, prev_block.start_row + 1, prev_block.end_row + 1
                        );
                    }
                    if is_important && !prev_is_important {
                        if debug_mode {
                            eprintln!("DEBUG: Cache Dedupe: Keeping important outer block");
                        }
                        // Keep both - don't skip add, continue checking
                    } else if !is_important && prev_is_important {
                        if debug_mode {
                            eprintln!("DEBUG: Cache Dedupe: Skipping non-important outer block");
                        }
                        should_add = false;
                        break;
                    } else {
                        // Both important or both not - use priority-based selection for determinism
                        let current_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == block.node_type.as_str());
                        let prev_priority = NODE_TYPE_PRIORITY
                            .iter()
                            .position(|&t| t == prev_block.node_type.as_str());

                        match (current_priority, prev_priority) {
                            (Some(cur_pri), Some(prev_pri)) => {
                                if cur_pri > prev_pri {
                                    // Current block has higher priority - keep it, remove previous
                                    if debug_mode {
                                        eprintln!("DEBUG: Cache Dedupe: Replacing contained block with higher priority type: {} > {}", 
                                                block.node_type, prev_block.node_type);
                                    }
                                    blocks_to_remove.push(idx);
                                } else {
                                    // Previous block has higher or equal priority - keep previous, skip current
                                    if debug_mode {
                                        eprintln!("DEBUG: Cache Dedupe: Skipping outer block in favor of higher priority contained type: {} >= {}", 
                                                prev_block.node_type, block.node_type);
                                    }
                                    should_add = false;
                                    break;
                                }
                            }
                            _ => {
                                // Fallback: prefer contained (previous) block for consistency
                                if debug_mode {
                                    eprintln!("DEBUG: Cache Dedupe: Skipping outer block (already have contained, no priority)");
                                }
                                should_add = false;
                                break;
                            }
                        }
                    }
                }
                // Case 3: Blocks partially overlap
                else {
                    if debug_mode {
                        eprintln!(
                            "DEBUG: Cache Dedupe: Partial overlap: type='{}', lines={}-{} (overlaps type='{}', lines={}-{})",
                            block.node_type, block.start_row + 1, block.end_row + 1,
                            prev_block.node_type, prev_block.start_row + 1, prev_block.end_row + 1
                        );
                    }
                    // Skip current block in case of partial overlap (consistent with miss path)
                    should_add = false;
                    break;
                }
            }
        }

        // Remove blocks marked for removal (in reverse order)
        for idx in blocks_to_remove.iter().rev() {
            final_code_blocks.remove(*idx);
        }

        // Add the current block if it wasn't skipped
        if should_add {
            final_code_blocks.push(block.clone());
        }
    }

    // Final sort to maintain correct order after deduplication
    final_code_blocks.sort_by_key(|block| block.start_row);
    Ok(final_code_blocks)
} // Added missing closing brace for process_cached_line_map
  // Removed unexpected closing brace that was here

/// Function to parse a file and extract code blocks for the given line numbers
pub fn parse_file_for_code_blocks(
    content: &str,
    extension: &str,
    line_numbers: &HashSet<usize>,
    allow_tests: bool,
    _term_matches: Option<&HashMap<usize, HashSet<usize>>>, // Query index to line numbers
) -> Result<Vec<CodeBlock>> {
    parse_file_for_code_blocks_with_tree(
        content,
        extension,
        line_numbers,
        allow_tests,
        _term_matches,
        None,
    )
}

/// Function to parse a file and extract code blocks with an optional pre-parsed tree
pub fn parse_file_for_code_blocks_with_tree(
    content: &str,
    extension: &str,
    line_numbers: &HashSet<usize>,
    allow_tests: bool,
    _term_matches: Option<&HashMap<usize, HashSet<usize>>>, // Query index to line numbers
    pre_parsed_tree: Option<tree_sitter::Tree>,
) -> Result<Vec<CodeBlock>> {
    // Check for debug mode
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // Get the appropriate language implementation
    let language_impl = match get_language_impl(extension) {
        Some(lang) => lang,
        None => {
            // For unsupported languages, return empty blocks to trigger fallback to literal extraction
            if debug_mode {
                eprintln!("DEBUG: File extension '{extension}' not supported for AST parsing, returning empty blocks for fallback");
            }
            return Ok(Vec::new());
        }
    };

    // Calculate content hash for cache key
    let content_hash = calculate_content_hash(content);
    let cache_key = format!("{extension}_{content_hash}_{allow_tests}");

    // Check if we have a cached sparse line map
    if let Some(cached_entry) = LINE_MAP_CACHE.get(&cache_key) {
        if debug_mode {
            eprintln!("DEBUG: Sparse cache hit for line_map key: {cache_key}");
        }

        // Process the sparse cached line map
        return process_sparse_line_map(
            cached_entry.value(),
            line_numbers,
            language_impl.as_ref(),
            content,
            allow_tests,
            debug_mode,
        );
    }

    if debug_mode {
        eprintln!(
            "DEBUG: Sparse cache miss for line_map key: {cache_key}. Building sparse line map..."
        );
    }

    // Get the tree - either use pre-parsed or parse it
    let tree = if let Some(pre_parsed) = pre_parsed_tree {
        if debug_mode {
            eprintln!("DEBUG: Using pre-parsed tree, skipping redundant parsing");
        }
        pre_parsed
    } else {
        // Get the tree-sitter language
        let language = language_impl.get_tree_sitter_language();

        // Parse the file
        let mut parser = TSParser::new();
        parser.set_language(&language)?;

        // Use the tree cache to get or parse the tree
        let tree_cache_key = format!("file_{extension}");
        tree_cache::get_or_parse_tree(&tree_cache_key, content, &mut parser)
            .context("Failed to parse the file")?
    };

    let root_node = tree.root_node();

    if debug_mode {
        eprintln!("DEBUG: SPARSE OPTIMIZATION - Parsing file with extension: {extension}");
        eprintln!(
            "DEBUG: SPARSE OPTIMIZATION - Root node type: {}",
            root_node.kind()
        );
    }

    // OPTIMIZATION: Build sparse line map that only processes intersecting nodes
    let sparse_line_map = build_sparse_line_map(
        root_node,
        line_numbers,
        language_impl.as_ref(),
        content.as_bytes(),
        allow_tests,
        debug_mode,
    );

    if debug_mode {
        eprintln!(
            "DEBUG: SPARSE OPTIMIZATION - Sparse line map built with {} entries",
            sparse_line_map.len()
        );
    }

    // Generate code blocks from the sparse line map
    let code_blocks = process_sparse_line_map(
        &sparse_line_map,
        line_numbers,
        language_impl.as_ref(),
        content,
        allow_tests,
        debug_mode,
    )?;
    // Store the sparse line map in cache for future requests
    LINE_MAP_CACHE.insert(cache_key.clone(), sparse_line_map);
    if debug_mode {
        eprintln!("DEBUG: SPARSE OPTIMIZATION - Stored sparse line map in cache key: {cache_key}");
    }

    Ok(code_blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_parse_file_unsupported_language_returns_empty() {
        // Test that parsing an unsupported language returns empty blocks instead of error
        let content = r#"
        # Terraform configuration
        resource "aws_instance" "example" {
          ami           = "ami-0c55b159cbfafe1f0"
          instance_type = "t2.micro"
        }
        "#;

        let mut line_numbers = HashSet::new();
        line_numbers.insert(3); // Line with 'resource'

        // .tf extension is not supported by tree-sitter
        let result = parse_file_for_code_blocks(
            content,
            "tf", // Terraform extension, not supported
            &line_numbers,
            false, // allow_tests
            None,  // term_matches
        );

        // Should return Ok with empty vector, not an error
        assert!(result.is_ok(), "Should not error for unsupported language");
        let blocks = result.unwrap();
        assert_eq!(
            blocks.len(),
            0,
            "Should return empty blocks for unsupported language"
        );
    }

    #[test]
    fn test_parse_file_supported_language_returns_blocks() {
        // Test that parsing a supported language returns appropriate blocks
        let content = r#"fn main() {
    println!("Hello, world!");
}

fn test_function() {
    assert_eq!(1, 1);
}"#;

        let mut line_numbers = HashSet::new();
        line_numbers.insert(2); // Line inside main function

        let result = parse_file_for_code_blocks(
            content,
            "rs", // Rust extension, supported
            &line_numbers,
            false, // allow_tests
            None,  // term_matches
        );

        assert!(result.is_ok(), "Should succeed for supported language");
        let blocks = result.unwrap();
        assert!(
            !blocks.is_empty(),
            "Should return code blocks for supported language"
        );

        // Check that we found a function block
        let has_function = blocks
            .iter()
            .any(|b| b.node_type == "function_item" || b.node_type == "function");
        assert!(has_function, "Should find a function block");
    }
}
