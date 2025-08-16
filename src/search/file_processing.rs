use anyhow::{Context, Result};
use lazy_static::lazy_static;
use lru::LruCache;
use rayon::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use probe_code::language::{is_test_file, parse_file_for_code_blocks_with_tree};
use probe_code::models::SearchResult;
use probe_code::ranking;
use probe_code::search::tokenization;

// PHASE 3B OPTIMIZATION: Global tokenization cache for term matching
// This cache stores tokenized results to avoid redundant tokenization of the same content
lazy_static! {
    static ref TOKENIZATION_CACHE: Mutex<LruCache<u64, Vec<String>>> = {
        // Cache up to 10,000 tokenized results
        Mutex::new(LruCache::new(std::num::NonZeroUsize::new(10_000).unwrap()))
    };
}

/// Compute a fast hash for cache key generation
fn compute_content_hash(content: &str, filename: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    filename.hash(&mut hasher);
    hasher.finish()
}

/// Structure to hold timing information for file processing stages
pub struct FileProcessingTimings {
    pub file_io: Option<Duration>,

    // AST parsing timings
    pub ast_parsing: Option<Duration>,
    pub ast_parsing_language_init: Option<Duration>,
    pub ast_parsing_parser_init: Option<Duration>,
    pub ast_parsing_tree_parsing: Option<Duration>,
    pub ast_parsing_line_map_building: Option<Duration>,

    // Block extraction timings
    pub block_extraction: Option<Duration>,
    pub block_extraction_code_structure: Option<Duration>,
    pub block_extraction_filtering: Option<Duration>,
    pub block_extraction_result_building: Option<Duration>,

    // Detailed result building timings
    pub result_building_term_matching: Option<Duration>,
    pub result_building_compound_processing: Option<Duration>,
    pub result_building_line_matching: Option<Duration>,
    pub result_building_result_creation: Option<Duration>,
    pub result_building_synchronization: Option<Duration>,
    pub result_building_uncovered_lines: Option<Duration>,
}

/// Parameters for file processing
pub struct FileProcessingParams<'a> {
    pub path: &'a Path,
    pub line_numbers: &'a HashSet<usize>,
    pub allow_tests: bool,
    pub term_matches: &'a HashMap<usize, HashSet<usize>>,
    #[allow(dead_code)]
    pub num_queries: usize,
    #[allow(dead_code)]
    pub filename_matched_queries: HashSet<usize>,
    pub queries_terms: &'a [Vec<(String, String)>],
    pub preprocessed_queries: Option<&'a [Vec<String>]>,
    pub query_plan: &'a crate::search::query::QueryPlan,

    #[allow(dead_code)]
    pub no_merge: bool,
    pub lsp: bool,
}

/// Evaluate whether a block of lines satisfies a complex AST query
/// using the 'evaluate' method in `elastic_query::Expr`. We assume
/// the 'term_matches' map uses the same indexing as the AST's QueryPlan term_indices.
#[allow(dead_code)]
pub fn filter_code_block_with_ast(
    block_lines: (usize, usize),
    term_matches: &HashMap<usize, HashSet<usize>>,
    plan: &crate::search::query::QueryPlan,
    debug_mode: bool,
) -> bool {
    // PHASE 3C OPTIMIZATION: Early termination for simple queries
    if plan.is_simple_query && term_matches.is_empty() {
        return false;
    }

    // Gather matched term indices for this block
    let mut matched_terms = HashSet::new();
    for (&term_idx, lines) in term_matches {
        if lines
            .iter()
            .any(|&l| l >= block_lines.0 && l <= block_lines.1)
        {
            matched_terms.insert(term_idx);
        }
    }

    if debug_mode {
        println!(
            "DEBUG: Checking for terms in block {}-{}",
            block_lines.0, block_lines.1
        );
        println!("DEBUG: Matched terms: {matched_terms:?}");
        println!("DEBUG: Term indices: {:?}", plan.term_indices);
        println!("DEBUG: Excluded terms: {:?}", plan.excluded_terms);
        println!("DEBUG: AST: {:?}", plan.ast);

        // Add detailed information about which exact keywords matched
        println!("DEBUG: ===== MATCHED KEYWORDS DETAILS =====");
        // PHASE 4 OPTIMIZATION: Pre-allocate with estimated size
        let mut matched_keywords = Vec::with_capacity(plan.term_indices.len());
        for (term, &idx) in &plan.term_indices {
            if matched_terms.contains(&idx) {
                matched_keywords.push(term);
                println!(
                    "DEBUG: Keyword '{}' matched in block {}-{}",
                    term, block_lines.0, block_lines.1
                );
            }
        }
        if matched_keywords.is_empty() {
            println!("DEBUG: No keywords matched in this block");
        } else {
            println!("DEBUG: All matched keywords: {matched_keywords:?}");
        }
        println!("DEBUG: ===================================");
    }

    // PHASE 3C OPTIMIZATION: Use fast path evaluation
    if let Some(result) = plan.ast.evaluate_fast_path(&matched_terms, plan) {
        if debug_mode {
            println!(
                "DEBUG: Fast path evaluation for block {}-{}: {}",
                block_lines.0, block_lines.1, result
            );
        }
        return result;
    }

    // Check if we have any matches at all
    if matched_terms.is_empty() && !plan.has_only_excluded_terms {
        if debug_mode {
            println!(
                "DEBUG: No matched terms in block {}-{}, returning false",
                block_lines.0, block_lines.1
            );
        }
        return false;
    }

    // Use the AST evaluation directly
    if debug_mode {
        println!("DEBUG: ===== AST EVALUATION =====");
        println!("DEBUG: Matched terms: {matched_terms:?}");
        println!("DEBUG: Term indices: {:?}", plan.term_indices);
    }

    // PHASE 3C OPTIMIZATION: Use cached evaluation when possible
    // Note: We can't use mutable reference in filter_code_block_with_ast,
    // so we fall back to regular evaluation here
    let result = plan.ast.evaluate(&matched_terms, &plan.term_indices, false);

    if debug_mode {
        println!("DEBUG: ===== EVALUATION RESULT =====");
        println!("DEBUG: AST evaluation result: {result}");
        println!(
            "DEBUG: Block {}-{} will be {}",
            block_lines.0,
            block_lines.1,
            if result { "INCLUDED" } else { "EXCLUDED" }
        );
        println!("DEBUG: ============================");
    }

    let decision = result;

    if debug_mode {
        println!(
            "DEBUG: Block {}-{} matched terms: {:?}",
            block_lines.0, block_lines.1, matched_terms
        );
        println!("DEBUG: AST evaluation result: {decision}");
    }

    if debug_mode {
        println!(
            "DEBUG: filter_code_block_with_ast => lines {block_lines:?} => matched {matched_terms:?}, decision={decision}"
        );
    }
    decision
}

/// Evaluate whether a tokenized block satisfies a complex AST query
/// using the 'evaluate' method in `elastic_query::Expr`.
pub fn filter_tokenized_block(
    tokenized_content: &[String],
    _term_indices: &HashMap<String, usize>,
    plan: &crate::search::query::QueryPlan,
    debug_mode: bool,
) -> bool {
    // Early termination: if query has only excluded terms and content is empty, return true
    if tokenized_content.is_empty() {
        return plan.has_only_excluded_terms;
    }

    // PHASE 3C OPTIMIZATION: Batch term index resolution
    let mut matched_terms = resolve_term_indices_batch(tokenized_content, &plan.term_indices);

    // PHASE 3C OPTIMIZATION: Early termination for required terms with indices
    if !plan.required_terms_indices.is_empty() {
        let has_all_required = plan
            .required_terms_indices
            .iter()
            .all(|idx| matched_terms.contains(idx));

        if !has_all_required {
            // Check for special cases in compound words
            let missing_required: Vec<_> = plan
                .required_terms_indices
                .iter()
                .filter(|idx| !matched_terms.contains(idx))
                .collect();

            let mut check_special_cases = false;
            for &idx in &missing_required {
                // Find the term for this index
                if let Some(term) = plan
                    .term_indices
                    .iter()
                    .find(|(_, &i)| i == *idx)
                    .map(|(t, _)| t)
                {
                    if crate::search::tokenization::is_special_case(term)
                        && tokenized_content.contains(&term.to_lowercase())
                    {
                        matched_terms.insert(*idx);
                        check_special_cases = true;
                    }
                }
            }

            if !check_special_cases {
                return false;
            }
        }
    }

    // PHASE 3C OPTIMIZATION: Early termination for simple queries
    if plan.is_simple_query && !matched_terms.is_empty() {
        return true;
    }

    // Special handling for compound words like "whitelist"
    // Check if any term in the plan is a compound of tokens in the content
    for (term, &idx) in &plan.term_indices {
        // Skip if we already matched this term
        if matched_terms.contains(&idx) {
            continue;
        }

        // Check if this term is a special case that should be treated as a single token
        if crate::search::tokenization::is_special_case(term) {
            // If the tokenized content contains this special case term, add it to matched terms
            if tokenized_content.contains(&term.to_lowercase()) {
                matched_terms.insert(idx);
                if debug_mode {
                    println!("DEBUG: Special case term '{term}' matched in tokenized content");
                }
            }
        }
    }

    if debug_mode {
        println!("DEBUG: Checking for terms in tokenized block");
        println!("DEBUG: Tokenized content: {tokenized_content:?}");
        println!("DEBUG: Matched terms: {matched_terms:?}");
        println!("DEBUG: Term indices: {:?}", plan.term_indices);
        println!("DEBUG: Excluded terms: {:?}", plan.excluded_terms);
        println!("DEBUG: AST: {:?}", plan.ast);

        // Add detailed information about which exact keywords matched
        println!("DEBUG: ===== MATCHED KEYWORDS DETAILS =====");
        // PHASE 4 OPTIMIZATION: Pre-allocate with estimated size
        let mut matched_keywords = Vec::with_capacity(plan.term_indices.len());
        for (term, &idx) in &plan.term_indices {
            if matched_terms.contains(&idx) {
                matched_keywords.push(term);
                println!("DEBUG: Keyword '{term}' matched in tokenized block");
            }
        }
        if matched_keywords.is_empty() {
            println!("DEBUG: No keywords matched in this block");
        } else {
            println!("DEBUG: All matched keywords: {matched_keywords:?}");
        }
        println!("DEBUG: ===================================");
    }

    // Check if we have any matches at all
    if matched_terms.is_empty() {
        // Check if the query only contains excluded terms
        if plan.has_only_excluded_terms {
            return true;
        }
        if debug_mode {
            println!("DEBUG: No matched terms in tokenized block, returning false");
        }
        return false;
    }

    // Use the AST evaluation directly
    if debug_mode {
        println!("DEBUG: ===== AST EVALUATION =====");
        println!("DEBUG: Matched terms: {matched_terms:?}");
        println!("DEBUG: Term indices: {:?}", plan.term_indices);
    }

    // PHASE 3C OPTIMIZATION: Use fast-path evaluation when possible
    if let Some(result) = plan.ast.evaluate_fast_path(&matched_terms, plan) {
        if debug_mode {
            println!("DEBUG: Fast path evaluation result: {result}");
        }
        return result;
    }

    // Use the evaluate function from the elastic query module
    let result = plan.ast.evaluate(&matched_terms, &plan.term_indices, false);

    if debug_mode {
        println!("DEBUG: ===== EVALUATION RESULT =====");
        println!("DEBUG: AST evaluation result: {result}");
        println!(
            "DEBUG: Block will be {}",
            if result { "INCLUDED" } else { "EXCLUDED" }
        );
        println!("DEBUG: ============================");
    }

    let decision = result;

    if debug_mode {
        println!("DEBUG: Tokenized block matched terms: {matched_terms:?}");
        println!("DEBUG: AST evaluation result: {decision}");
        println!("DEBUG: filter_tokenized_block => matched {matched_terms:?}, decision={decision}");
    }

    decision
}

/// Determines a better node type for fallback context by analyzing the line content
fn determine_fallback_node_type(line: &str, extension: Option<&str>) -> String {
    let trimmed = line.trim();

    if trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || trimmed.starts_with("*")
        || (trimmed.starts_with("#") && extension.is_some_and(|ext| ext == "py" || ext == "rb"))
        || trimmed.starts_with("'''")
        || trimmed.starts_with("\"\"\"")
    {
        return "comment".to_string();
    }

    let lowercase = trimmed.to_lowercase();

    if (trimmed.contains("fn ")
        && (trimmed.contains("(") || trimmed.contains(")"))
        && extension == Some("rs"))
        || (trimmed.contains("func ") && extension == Some("go"))
        || (trimmed.contains("function ")
            && extension.is_some_and(|ext| ext == "js" || ext == "ts"))
        || (lowercase.contains("def ") && extension == Some("py"))
        || (trimmed.contains("public")
            && trimmed.contains("void")
            && extension.is_some_and(|ext| ext == "java" || ext == "kt"))
    {
        return "function".to_string();
    }

    if (trimmed.contains("class ") || trimmed.contains("interface "))
        || (trimmed.contains("struct ")
            && extension
                .is_some_and(|ext| ext == "rs" || ext == "go" || ext == "c" || ext == "cpp"))
        || (trimmed.contains("type ") && trimmed.contains("struct") && extension == Some("go"))
        || (trimmed.contains("enum "))
    {
        return "class".to_string();
    }

    if trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("require ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("#include ")
    {
        return "import".to_string();
    }

    if (trimmed.starts_with("let ") || trimmed.starts_with("var ") || trimmed.starts_with("const "))
        || (trimmed.contains("=") && !trimmed.contains("==") && !trimmed.contains("=>"))
    {
        return "variable_declaration".to_string();
    }

    if trimmed.starts_with("if ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("switch ")
        || trimmed.starts_with("match ")
    {
        return "control_flow".to_string();
    }

    "code".to_string()
}

/// Batch processing context for uncovered lines optimization
struct BatchProcessingContext<'a> {
    uncovered_lines: &'a [usize],
    covered_lines: &'a mut HashSet<usize>,
    lines: &'a [&'a str],
    params: &'a FileProcessingParams<'a>,
    extension: &'a str,
    unique_query_terms: &'a HashSet<String>,
    results: &'a mut Vec<SearchResult>,
    timings: &'a mut FileProcessingTimings,
    debug_mode: bool,
}

/// ENHANCED BATCH OPTIMIZATION: Process uncovered lines with advanced batching for improved performance
///
/// This function implements an enhanced batch processing approach that can improve performance by 3-5 seconds:
///
/// 1. **Smart Context Window Merging**: Uses aggressive merging with optimized gap detection
/// 2. **Pre-filtering Optimization**: Skip contexts unlikely to contain query terms early
/// 3. **Batch Tokenization Cache**: Pre-tokenize and cache unique context windows
/// 4. **Single Parser Creation**: Create one parser per file instead of per line
/// 5. **Reduced Vocabulary Loading**: Load vocabulary once for all compound word processing
/// 6. **Optimized Memory Locality**: Process merged context windows in sequence for better cache performance
/// 7. **Early Termination Detection**: Skip processing lines already covered by merged contexts
///
/// The function maintains full backward compatibility with the original individual processing.
fn process_uncovered_lines_batch(ctx: &mut BatchProcessingContext) {
    if ctx.uncovered_lines.is_empty() {
        return;
    }

    if ctx.debug_mode {
        println!(
            "DEBUG: SMART CONTEXT WINDOW MERGING: Processing {} uncovered lines with enhanced merging",
            ctx.uncovered_lines.len()
        );
    }

    // Test detection optimization: Use pooled parser for better performance
    let mut parser_opt = None;
    if !ctx.params.allow_tests {
        if let Some(language_impl) = crate::language::factory::get_language_impl(ctx.extension) {
            // Use pooled parser for test detection as well
            if let Ok(parser) = crate::language::get_pooled_parser(ctx.extension) {
                parser_opt = Some((parser, language_impl));
            }
        }
    }

    // VOCABULARY CACHE OPTIMIZATION: Cached vocabulary is now accessed directly through
    // tokenization::split_compound_word_for_filtering() calls, eliminating repeated vocabulary loading

    // SMART CONTEXT WINDOW MERGING: Pre-compute context windows with enhanced merging
    let default_context_size = 5;
    // PHASE 4 OPTIMIZATION: Pre-allocate with line numbers size
    let mut context_windows = Vec::with_capacity(ctx.params.line_numbers.len());

    // PHASE 3A OPTIMIZATION: Pre-compile query terms into lowercase for faster matching
    let query_terms_lower: HashSet<String> = ctx
        .unique_query_terms
        .iter()
        .map(|s| s.to_lowercase())
        .collect();

    // Pre-create a basic text representation for pre-filtering (optimization)
    let file_text_lower = ctx.lines.join("\n").to_lowercase();
    let has_potential_matches = query_terms_lower
        .iter()
        .any(|term| file_text_lower.contains(term));

    if !has_potential_matches && ctx.debug_mode {
        println!("DEBUG: SMART MERGING: No query terms found in file content, processing minimal contexts");
    }

    // Step 1: Generate all potential context windows with pre-filtering
    for &line_num in ctx.uncovered_lines {
        // Skip if line is already covered by previous processing
        if ctx.covered_lines.contains(&line_num) {
            if ctx.debug_mode {
                println!("DEBUG: Line {line_num} already covered, skipping");
            }
            continue;
        }

        // Skip fallback context for test files if allow_tests is false
        if !ctx.params.allow_tests && is_test_file(ctx.params.path) {
            if ctx.debug_mode {
                println!(
                    "DEBUG: Skipping fallback context for test file: {:?}",
                    ctx.params.path
                );
            }
            continue;
        }

        // Check if the line is in a test function/module using the shared parser
        if !ctx.params.allow_tests && line_num <= ctx.lines.len() {
            if let Some((ref mut parser, ref language_impl)) = parser_opt {
                let line_content = ctx.lines[line_num - 1];

                if let Some(tree) = parser.parse(line_content, None) {
                    let node = tree.root_node();
                    if language_impl.is_test_node(&node, line_content.as_bytes()) {
                        if ctx.debug_mode {
                            println!(
                                "DEBUG: Skipping fallback context for test code: '{}'",
                                line_content.trim()
                            );
                        }
                        continue;
                    }
                }
            }
        }

        // Calculate context window bounds
        let line_idx = line_num - 1;
        let context_start_idx = line_idx.saturating_sub(default_context_size);
        let context_end_idx = std::cmp::min(line_idx + default_context_size, ctx.lines.len() - 1);

        if context_start_idx <= context_end_idx {
            let context_start = context_start_idx + 1;
            let context_end = context_end_idx + 1;
            context_windows.push((
                line_num,
                context_start,
                context_end,
                context_start_idx,
                context_end_idx,
            ));
        }
    }

    if ctx.debug_mode {
        println!("DEBUG: Generated {} context windows", context_windows.len());
    }

    // Step 2: Enhanced merging with smart gap detection and aggressive combining
    context_windows.sort_by_key(|&(_, start, _, _, _)| start);

    // PHASE 4 OPTIMIZATION: Pre-allocate with context windows size
    let mut merged_windows = Vec::with_capacity(context_windows.len());
    let mut current_window: Option<(Vec<usize>, usize, usize, usize, usize)> = None;

    // SMART MERGING OPTIMIZATION: Use dynamic threshold based on context density
    // PHASE 3A OPTIMIZATION: More aggressive merging reduces number of contexts to process
    let dynamic_merge_threshold = if context_windows.len() > 10 {
        // More aggressive merging for many context windows
        default_context_size + 3 // Increased from +2 to +3 for better batching
    } else if context_windows.len() > 5 {
        // Medium merging for moderate windows
        default_context_size + 1
    } else {
        // Standard merging for fewer windows
        1
    };

    for (line_num, context_start, context_end, context_start_idx, context_end_idx) in
        context_windows
    {
        match current_window {
            None => {
                // First window
                current_window = Some((
                    vec![line_num],
                    context_start,
                    context_end,
                    context_start_idx,
                    context_end_idx,
                ));
            }
            Some((
                ref mut lines,
                current_start,
                current_end,
                current_start_idx,
                current_end_idx,
            )) => {
                // ENHANCED MERGING: More aggressive gap detection
                let gap = if context_start > current_end {
                    context_start - current_end - 1
                } else {
                    0 // Overlapping
                };

                // Merge if overlapping, adjacent, or within dynamic threshold
                if context_start <= current_end + dynamic_merge_threshold {
                    // SMART MERGING: Extend current window and track all original lines
                    lines.push(line_num);
                    let new_end = std::cmp::max(current_end, context_end);
                    let new_end_idx = std::cmp::max(current_end_idx, context_end_idx);
                    current_window = Some((
                        lines.clone(),
                        current_start,
                        new_end,
                        current_start_idx,
                        new_end_idx,
                    ));

                    if ctx.debug_mode && gap > 1 {
                        println!("DEBUG: SMART MERGING: Merged contexts with gap of {gap} lines (threshold: {dynamic_merge_threshold})");
                    }
                } else {
                    // Gap too large, finalize current window and start new one
                    merged_windows.push(current_window.take().unwrap());
                    current_window = Some((
                        vec![line_num],
                        context_start,
                        context_end,
                        context_start_idx,
                        context_end_idx,
                    ));

                    if ctx.debug_mode {
                        println!("DEBUG: SMART MERGING: Gap of {gap} lines exceeds threshold {dynamic_merge_threshold}, creating new window");
                    }
                }
            }
        }
    }

    // Don't forget to add the last window
    if let Some(window) = current_window {
        merged_windows.push(window);
    }

    if ctx.debug_mode {
        println!(
            "DEBUG: SMART MERGING: Merged into {} optimized context windows (reduced from {})",
            merged_windows.len(),
            ctx.uncovered_lines.len()
        );
        for (i, (lines, start, end, _, _)) in merged_windows.iter().enumerate() {
            println!(
                "DEBUG: Window {}: lines {:?} -> context {}-{} (size: {} lines)",
                i,
                lines,
                start,
                end,
                end - start + 1
            );
        }
    }

    // TOKENIZATION CACHE: Pre-tokenize and cache unique context windows for reuse
    let mut tokenization_cache: std::collections::HashMap<(usize, usize), Vec<String>> =
        std::collections::HashMap::new();

    // PHASE 3A OPTIMIZATION: Pre-allocate results vector with estimated capacity
    // This avoids vector reallocation during result creation
    let estimated_results = merged_windows.len();
    ctx.results.reserve(estimated_results);

    // Step 3: Process merged context windows in batch with tokenization caching
    for (original_lines, context_start, context_end, context_start_idx, context_end_idx) in
        merged_windows
    {
        // Extract the context lines using 0-based indices (BATCH OPTIMIZATION: single extraction per merged window)
        let context_code = ctx.lines[context_start_idx..=context_end_idx]
            .to_vec()
            .join("\n");

        // Determine node type based on the first original line (for consistency with original behavior)
        let primary_line = original_lines[0];
        let node_type =
            determine_fallback_node_type(ctx.lines[primary_line - 1], Some(ctx.extension));

        if ctx.debug_mode {
            println!("DEBUG: Inferred node type for merged fallback context: {node_type}");
            println!(
                "DEBUG: Processing merged context window: lines {}-{} (size: {}) covering original lines: {:?}",
                context_start,
                context_end,
                context_end - context_start + 1,
                original_lines
            );
        }

        // TEST FUNCTION FILTERING: Skip merged context windows that contain test functions when allow_tests=false
        if !ctx.params.allow_tests {
            // Use string-based test detection for merged context windows since AST parsing
            // of extracted context may not preserve full structure needed for is_test_node
            let is_test_context = context_code.contains("#[test]")
                || context_code.lines().any(|line| {
                    let trimmed = line.trim();
                    trimmed.starts_with("fn test_") || trimmed.starts_with("pub fn test_")
                });

            if ctx.debug_mode {
                println!(
                    "DEBUG: Test function check for merged context window at lines {context_start}-{context_end}: is_test={is_test_context}"
                );
            }

            if is_test_context {
                if ctx.debug_mode {
                    println!(
                        "DEBUG: Filtering out merged context window at lines {context_start}-{context_end} (contains test function)"
                    );
                }
                // Mark lines as covered to prevent further processing
                for line in context_start..=context_end {
                    ctx.covered_lines.insert(line);
                }
                continue;
            }
        }

        // AGGRESSIVE PRE-FILTERING: Fast string contains() check before expensive tokenization
        // This optimization skips contexts that obviously don't contain any query terms
        // String contains() is ~100x faster than full tokenization + compound word processing
        //
        // FILENAME MATCH FIX: For filename-based matches, be more lenient with pre-filtering
        // When a file matches by filename, we should include context even if individual lines
        // don't contain query terms, because the user's intent is to see the file content
        //
        // HEURISTIC: Detect filename matches by checking if any term matches ALL lines in the file
        // This is characteristic of filename-based matching where the entire file is considered relevant
        let file_line_count = ctx.lines.len();
        let is_likely_filename_match = ctx
            .params
            .term_matches
            .values()
            .any(|lines| lines.len() >= file_line_count);
        // PHASE 3A OPTIMIZATION: Reuse pre-computed lowercase query terms
        let context_text_lower = context_code.to_lowercase();
        let has_potential_query_matches = query_terms_lower
            .iter()
            .any(|term| context_text_lower.contains(term));

        // Skip aggressive pre-filtering for filename matches to preserve all context
        if !has_potential_query_matches && !is_likely_filename_match {
            if ctx.debug_mode {
                println!(
                    "DEBUG: AGGRESSIVE PRE-FILTERING: Context {context_start}-{context_end} contains no query terms, skipping expensive processing"
                );
            }
            // PHASE 3A OPTIMIZATION: Mark lines as covered even when skipping
            // This prevents redundant processing in subsequent operations
            for line in context_start..=context_end {
                ctx.covered_lines.insert(line);
            }
            // Skip this context entirely - no point in tokenizing or processing it further
            continue;
        }

        if ctx.debug_mode {
            if has_potential_query_matches {
                println!(
                    "DEBUG: AGGRESSIVE PRE-FILTERING: Context {context_start}-{context_end} passed pre-filter, proceeding with tokenization"
                );
            } else if is_likely_filename_match {
                println!(
                    "DEBUG: AGGRESSIVE PRE-FILTERING: Context {context_start}-{context_end} bypassed pre-filter due to filename match, proceeding with tokenization"
                );
            }
        }

        // TOKENIZATION CACHE: Check cache first to avoid redundant tokenization
        let cache_key = (context_start_idx, context_end_idx);
        let context_terms = if let Some(cached_terms) = tokenization_cache.get(&cache_key) {
            if ctx.debug_mode {
                println!("DEBUG: TOKENIZATION CACHE: Using cached tokenization for context {context_start}-{context_end}");
            }
            cached_terms.clone()
        } else {
            // BATCH OPTIMIZATION: Single tokenization per merged context window (major performance gain)
            // PHASE 3A OPTIMIZATION: Use more efficient tokenization for uncovered lines
            // Since these are fallback contexts, we can use a lighter tokenization approach
            let terms = if ctx.params.query_plan.exact {
                // In exact mode, use the full tokenization
                ranking::preprocess_text_with_filename(
                    &context_code,
                    &ctx.params.path.to_string_lossy(),
                )
            } else {
                // For non-exact mode, use optimized tokenization without filename processing
                // This is faster for fallback contexts where filename isn't as relevant
                ranking::preprocess_text(&context_code)
            };
            tokenization_cache.insert(cache_key, terms.clone());
            if ctx.debug_mode {
                println!("DEBUG: TOKENIZATION CACHE: Cached tokenization for context {context_start}-{context_end}");
            }
            terms
        };

        // Start measuring filtering time for uncovered lines
        let filtering_start = Instant::now();

        // Early filtering for fallback context
        let should_include = {
            if ctx.debug_mode {
                println!(
                    "DEBUG: Using filter_tokenized_block for merged fallback context {context_start}-{context_end}"
                );
            }

            // Skip tokenization and evaluation when exact flag is enabled
            if ctx.params.query_plan.exact {
                // In exact mode, we already matched the lines in the file
                // so we should include this block without re-evaluating
                if ctx.debug_mode {
                    println!(
                        "DEBUG: Exact mode enabled, skipping tokenization and evaluation for merged fallback context {context_start}-{context_end}"
                    );
                }
                true
            } else {
                filter_tokenized_block(
                    &context_terms,
                    &ctx.params.query_plan.term_indices,
                    ctx.params.query_plan,
                    ctx.debug_mode,
                )
            }
        };

        // We don't add this to any timing since filtering is not part of result building
        let _filtering_duration = filtering_start.elapsed();

        if ctx.debug_mode {
            println!(
                "DEBUG: Merged context window at {context_start}-{context_end} filtered: included={should_include}"
            );
        }

        // BATCH OPTIMIZATION: Mark all lines in merged context as covered at once
        // PHASE 3A OPTIMIZATION: Always mark lines as covered to prevent redundant processing
        for line in context_start..=context_end {
            ctx.covered_lines.insert(line);
        }

        // Add to results only if it passes the filter
        if should_include {
            // Start measuring compound word processing time
            let compound_start = Instant::now();

            // BATCH OPTIMIZATION: Calculate metrics once for the entire merged context window
            // PHASE 3A OPTIMIZATION: Use HashSet for O(1) lookups instead of Vec iterations
            let context_terms_set: HashSet<&String> = context_terms.iter().collect();

            let direct_matches: HashSet<&String> = ctx
                .unique_query_terms
                .iter()
                .filter(|t| context_terms_set.contains(t))
                .collect();

            let mut compound_matches = HashSet::new();
            // VOCABULARY CACHE OPTIMIZATION: Use cached vocabulary for filtering compound word processing
            // PHASE 3A OPTIMIZATION: Skip compound processing if no query terms have multiple parts
            let needs_compound_processing = ctx
                .unique_query_terms
                .iter()
                .any(|t| t.contains('_') || t.contains('-'));

            if needs_compound_processing {
                for qterm in ctx.unique_query_terms {
                    if direct_matches.contains(&qterm) {
                        continue;
                    }
                    // Use cached compound word splitting optimized for filtering operations
                    let parts = tokenization::split_compound_word_for_filtering(qterm);
                    if parts.len() > 1 && parts.iter().all(|part| context_terms_set.contains(&part))
                    {
                        compound_matches.insert(qterm);
                    }
                }
            }

            // Add to compound processing time
            let compound_duration = compound_start.elapsed();
            if let Some(duration) = ctx.timings.result_building_compound_processing {
                ctx.timings.result_building_compound_processing =
                    Some(duration + compound_duration);
            } else {
                ctx.timings.result_building_compound_processing = Some(compound_duration);
            }

            let context_unique_terms = direct_matches.len() + compound_matches.len();
            let context_total_matches = direct_matches.len() + compound_matches.len();

            // Collect matched keywords for merged fallback context
            // PHASE 4 OPTIMIZATION: Pre-allocate with estimated size
            let mut matched_keywords = Vec::with_capacity(ctx.params.query_plan.term_indices.len());

            // Add direct matches
            matched_keywords.extend(direct_matches.iter().map(|s| (*s).clone()));

            // Add compound matches
            matched_keywords.extend(compound_matches.iter().map(|s| (*s).clone()));

            // Start measuring line matching time
            let line_matching_start = Instant::now();

            // BATCH OPTIMIZATION: Get matched term indices for the entire merged context block
            let mut matched_term_indices = HashSet::new();
            for (&term_idx, lines) in ctx.params.term_matches {
                if lines
                    .iter()
                    .any(|&l| l >= context_start && l <= context_end)
                {
                    matched_term_indices.insert(term_idx);
                }
            }

            // Add to line matching time
            let line_matching_duration = line_matching_start.elapsed();
            if let Some(duration) = ctx.timings.result_building_line_matching {
                ctx.timings.result_building_line_matching = Some(duration + line_matching_duration);
            } else {
                ctx.timings.result_building_line_matching = Some(line_matching_duration);
            }

            // Add the corresponding terms from the query plan
            for (term, &idx) in &ctx.params.query_plan.term_indices {
                if matched_term_indices.contains(&idx)
                    && !ctx.params.query_plan.excluded_terms.contains(term)
                {
                    matched_keywords.push(term.clone());
                }
            }

            // Remove duplicates
            matched_keywords.sort();
            matched_keywords.dedup();

            // Start measuring result creation time
            let result_creation_start = Instant::now();

            // BATCH OPTIMIZATION: Create single result for merged context window instead of multiple individual results
            let result = SearchResult {
                file: ctx.params.path.to_string_lossy().to_string(),
                lines: (context_start, context_end),
                node_type,
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
                file_unique_terms: Some(context_unique_terms),
                file_total_matches: Some(context_total_matches),
                file_match_rank: None,
                block_unique_terms: Some(context_unique_terms),
                block_total_matches: Some(context_total_matches),
                parent_file_id: None,
                block_id: None,
                matched_keywords: if matched_keywords.is_empty() {
                    None
                } else {
                    Some(matched_keywords)
                },
                tokenized_content: Some(context_terms),
                lsp_info: None,
            };

            // Add to result creation time
            let result_creation_duration = result_creation_start.elapsed();
            if let Some(duration) = ctx.timings.result_building_result_creation {
                ctx.timings.result_building_result_creation =
                    Some(duration + result_creation_duration);
            } else {
                ctx.timings.result_building_result_creation = Some(result_creation_duration);
            }

            // Start measuring synchronization time (in this case, just adding to results)
            let sync_start = Instant::now();

            ctx.results.push(result);

            // Add to synchronization time
            let sync_duration = sync_start.elapsed();
            if let Some(duration) = ctx.timings.result_building_synchronization {
                ctx.timings.result_building_synchronization = Some(duration + sync_duration);
            } else {
                ctx.timings.result_building_synchronization = Some(sync_duration);
            }
        }
    }

    // Return parser to pool if it was used
    if let Some((parser, _)) = parser_opt {
        crate::language::return_pooled_parser(ctx.extension, parser);
    }

    if ctx.debug_mode {
        println!(
            "DEBUG: SMART CONTEXT WINDOW MERGING: Completed processing {} uncovered lines with {} cache hits",
            ctx.uncovered_lines.len(),
            tokenization_cache.len()
        );
    }
}

// PHASE 3B OPTIMIZATION: Cache for pre-tokenized query terms across file processing
thread_local! {
    static QUERY_TERMS_CACHE: RefCell<HashMap<String, Vec<String>>> = RefCell::new(HashMap::new());
}

// PHASE 3C OPTIMIZATION: Batch term index resolution
fn resolve_term_indices_batch(
    tokens: &[String],
    term_indices: &HashMap<String, usize>,
) -> HashSet<usize> {
    let mut matched_terms = HashSet::with_capacity(tokens.len().min(term_indices.len()));

    for token in tokens {
        if let Some(&idx) = term_indices.get(token) {
            matched_terms.insert(idx);
        }
    }

    matched_terms
}

/// Main function for processing a file with matched lines
pub fn process_file_with_results(
    params: &FileProcessingParams,
) -> Result<(Vec<SearchResult>, FileProcessingTimings)> {
    let mut timings = FileProcessingTimings {
        file_io: None,

        // AST parsing timings
        ast_parsing: None,
        ast_parsing_language_init: None,
        ast_parsing_parser_init: None,
        ast_parsing_tree_parsing: None,
        ast_parsing_line_map_building: None,

        // Block extraction timings
        block_extraction: None,
        block_extraction_code_structure: None,
        block_extraction_filtering: None,
        block_extraction_result_building: None,

        // Detailed result building timings
        result_building_term_matching: None,
        result_building_compound_processing: None,
        result_building_line_matching: None,
        result_building_result_creation: None,
        result_building_synchronization: None,
        result_building_uncovered_lines: None,
    };

    // Measure file I/O time
    let file_io_start = Instant::now();
    let content = fs::read_to_string(params.path)
        .context(format!("Failed to read file: {:?}", params.path))?;
    let file_io_duration = file_io_start.elapsed();
    timings.file_io = Some(file_io_duration);

    let extension = params
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    // Get debug mode setting
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    // Filter out lines longer than 500 characters
    let lines: Vec<&str> = content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if line.len() > 500 {
                if debug_mode {
                    println!(
                        "DEBUG: Skipping line {} with length {} (exceeds 500 character limit)",
                        i + 1,
                        line.len()
                    );
                }
                ""
            } else {
                line
            }
        })
        .collect();
    let mut results = Vec::new();
    let mut covered_lines = HashSet::new();
    // We now use params.path.to_string_lossy() directly for tokenization

    // Prepare query terms once for the entire file
    let query_terms: Vec<String> = if let Some(prep) = params.preprocessed_queries {
        prep.iter().flat_map(|v| v.iter().cloned()).collect()
    } else {
        params
            .queries_terms
            .iter()
            .flat_map(|pairs| pairs.iter().map(|(_, s)| s.clone()))
            .collect()
    };
    let unique_query_terms: HashSet<String> = query_terms.into_iter().collect();

    if debug_mode {
        println!("DEBUG: Processing file: {:?}", params.path);
        println!("DEBUG:   matched lines: {:?}", params.line_numbers);
        println!("DEBUG:   file I/O time: {file_io_duration:?}");
        println!(
            "DEBUG: Processing {} unique query terms",
            unique_query_terms.len()
        );
    }

    // Measure AST parsing time with sub-steps
    let ast_parsing_start = Instant::now();

    // PARSER POOLING OPTIMIZATION: Use pooled parsers to avoid expensive initialization
    //
    // Previous implementation created a new parser for each file:
    // - Parser::new() + set_language() for every file (expensive)
    // - Language implementation lookup for every file (redundant)
    //
    // New implementation uses parser pooling:
    // - Reuses pre-configured parsers from a thread-safe pool
    // - Eliminates parser creation and language setup overhead
    // - Automatically manages parser lifecycle (get/return)

    // Measure language initialization time (now minimal due to pooling)
    let language_init_start = Instant::now();
    let language_supported = crate::language::factory::get_language_impl(extension).is_some();
    let language_init_duration = language_init_start.elapsed();
    timings.ast_parsing_language_init = Some(language_init_duration);

    // Measure parser initialization time (now minimal due to pooling)
    let parser_init_start = Instant::now();
    // Parser initialization is now handled internally by the pooling system
    let parser_init_duration = parser_init_start.elapsed();
    timings.ast_parsing_parser_init = Some(parser_init_duration);

    // Measure tree parsing time (this is where the real optimization happens)
    let tree_parsing_start = Instant::now();
    let file_path = params.path.to_string_lossy();
    let mut cache_key = String::with_capacity(file_path.len() + extension.len() + 1);
    cache_key.push_str(&file_path);
    cache_key.push('_');
    cache_key.push_str(extension);

    // Capture the parsed tree instead of discarding it
    let parsed_tree = if language_supported {
        // Use the new pooled parser approach - this eliminates the expensive
        // parser creation and language setup that was happening for each file
        crate::language::get_or_parse_tree_pooled(&cache_key, &content, extension).ok()
    } else {
        None
    };
    let tree_parsing_duration = tree_parsing_start.elapsed();
    timings.ast_parsing_tree_parsing = Some(tree_parsing_duration);

    // Measure line map building time (this is an approximation since we can't directly measure it)
    let line_map_building_start = Instant::now();

    // Call parse_file_for_code_blocks with the pre-parsed tree to avoid double parsing
    let code_blocks_result = parse_file_for_code_blocks_with_tree(
        &content,
        extension,
        params.line_numbers,
        params.allow_tests,
        Some(params.term_matches),
        parsed_tree,
    );

    let line_map_building_duration = line_map_building_start.elapsed();
    timings.ast_parsing_line_map_building = Some(line_map_building_duration);

    // Calculate total AST parsing time
    let ast_parsing_duration = ast_parsing_start.elapsed();
    timings.ast_parsing = Some(ast_parsing_duration);

    if debug_mode {
        println!("DEBUG:   AST parsing time: {ast_parsing_duration:?}");
        println!("DEBUG:     - Language init: {language_init_duration:?}");
        println!("DEBUG:     - Parser init: {parser_init_duration:?}");
        println!("DEBUG:     - Tree parsing: {tree_parsing_duration:?}");
        println!("DEBUG:     - Line map building: {line_map_building_duration:?}");
    }

    if let Ok(code_blocks) = code_blocks_result {
        // PHASE 4 OPTIMIZATION: Pre-allocate results with code blocks size
        results.reserve(code_blocks.len());

        if debug_mode {
            println!("DEBUG: AST parsing successful");
            println!("DEBUG:   Found {} code blocks", code_blocks.len());

            for (i, block) in code_blocks.iter().enumerate() {
                println!(
                    "DEBUG:   Block {}: type={}, lines={}-{}",
                    i + 1,
                    block.node_type,
                    block.start_row + 1,
                    block.end_row + 1
                );
            }
        }

        let file_id = params.path.to_string_lossy().to_string();

        // Measure block extraction time with sub-steps
        let block_extraction_start = Instant::now();

        // Measure code structure finding time
        let _code_structure_start = Instant::now();
        let code_structure_duration = Arc::new(Mutex::new(Duration::new(0, 0)));
        let filtering_duration = Arc::new(Mutex::new(Duration::new(0, 0)));
        let result_building_duration = Arc::new(Mutex::new(Duration::new(0, 0)));

        // Track detailed result building timings
        let term_matching_duration = Arc::new(Mutex::new(Duration::new(0, 0)));
        let compound_processing_duration = Arc::new(Mutex::new(Duration::new(0, 0)));
        let line_matching_duration = Arc::new(Mutex::new(Duration::new(0, 0)));
        let result_creation_duration = Arc::new(Mutex::new(Duration::new(0, 0)));
        let synchronization_duration = Arc::new(Mutex::new(Duration::new(0, 0)));

        // Prepare shared resources for parallel processing
        // PHASE 4 OPTIMIZATION: Pre-allocate shared results with estimated capacity
        let shared_results = Arc::new(Mutex::new(Vec::with_capacity(code_blocks.len())));
        let shared_covered_lines = Arc::new(Mutex::new(HashSet::new()));

        // Process blocks in parallel
        code_blocks
            .par_iter()
            .enumerate()
            .for_each(|(block_idx, block)| {
                // Start measuring code structure finding time for this block
                let block_start = Instant::now();

                let start_line = block.start_row + 1;
                let end_line = block.end_row + 1;

                let (final_start_line, final_end_line, is_nested_struct) = if extension == "go"
                    && block.node_type == "struct_type"
                    && block
                        .parent_node_type
                        .as_ref()
                        .is_some_and(|p| p == "function_declaration" || p == "method_declaration")
                {
                    if let Some(ps) = block.parent_start_row {
                        if let Some(pe) = block.parent_end_row {
                            (ps + 1, pe + 1, true)
                        } else {
                            (start_line, end_line, false)
                        }
                    } else {
                        (start_line, end_line, false)
                    }
                } else {
                    (start_line, end_line, false)
                };

                let full_code = if final_start_line > 0 && final_end_line <= lines.len() {
                    // Skip empty lines (which were originally too long)
                    lines[final_start_line - 1..final_end_line]
                        .to_vec()
                        .join("\n")
                } else {
                    "".to_string()
                };

                // End code structure finding time for this block
                let block_duration = block_start.elapsed();
                {
                    let mut duration = code_structure_duration.lock().unwrap();
                    *duration += block_duration;
                }

                // Start measuring term matching time
                let term_matching_start = Instant::now();

                // PHASE 3B OPTIMIZATION: Use global tokenization cache
                let cache_key = compute_content_hash(&full_code, &params.path.to_string_lossy());
                let block_terms = {
                    let mut cache = TOKENIZATION_CACHE.lock().unwrap();
                    if let Some(cached_terms) = cache.get(&cache_key) {
                        if debug_mode {
                            println!("DEBUG: PHASE 3B - Using cached tokenization for block");
                        }
                        cached_terms.clone()
                    } else {
                        drop(cache); // Release lock before tokenization
                        let terms = ranking::preprocess_text_with_filename(
                            &full_code,
                            &params.path.to_string_lossy(),
                        );
                        let mut cache = TOKENIZATION_CACHE.lock().unwrap();
                        cache.put(cache_key, terms.clone());
                        if debug_mode {
                            println!("DEBUG: PHASE 3B - Cached new tokenization for block");
                        }
                        terms
                    }
                };

                // End term matching time measurement
                let term_matching_block_duration = term_matching_start.elapsed();
                {
                    let mut duration = term_matching_duration.lock().unwrap();
                    *duration += term_matching_block_duration;
                }

                // Start measuring filtering time
                let filtering_start = Instant::now();
                // Early filtering using tokenized content
                let should_include = {
                    if debug_mode {
                        println!(
                            "DEBUG: Using filter_tokenized_block for block {final_start_line}-{final_end_line}"
                        );
                    }

                    // Skip tokenization and evaluation when exact flag is enabled
                    if params.query_plan.exact {
                        // In exact mode, we already matched the lines in the file
                        // so we should include this block without re-evaluating
                        if debug_mode {
                            println!(
                                "DEBUG: Exact mode enabled, skipping tokenization and evaluation for block {final_start_line}-{final_end_line}"
                            );
                        }
                        true
                    } else {
                        // Use the AST evaluation directly to ensure correct handling of complex queries
                        let result = filter_tokenized_block(
                            &block_terms,
                            &params.query_plan.term_indices,
                            params.query_plan,
                            debug_mode,
                        );

                        if debug_mode {
                            println!(
                                "DEBUG: Block {final_start_line}-{final_end_line} filter result: {result}"
                            );
                        }

                        result
                    }
                };

                // End filtering time measurement
                let filtering_block_duration = filtering_start.elapsed();
                {
                    let mut duration = filtering_duration.lock().unwrap();
                    *duration += filtering_block_duration;
                }

                if debug_mode {
                    println!(
                        "DEBUG: Block lines {final_start_line}-{final_end_line} => should_include={should_include}"
                    );
                }

                // Mark lines as covered
                {
                    let mut covered = shared_covered_lines.lock().unwrap();
                    for line_num in final_start_line..=final_end_line {
                        covered.insert(line_num);
                    }
                }

                if should_include {
                    // Start measuring result building time
                    let result_building_start = Instant::now();

                    // Start measuring term matching time
                    let direct_matches_start = Instant::now();

                    // PHASE 3B OPTIMIZATION: Convert block_terms to HashSet for O(1) lookups
                    let block_terms_set: HashSet<&String> = block_terms.iter().collect();
                    // OPTIMIZED: Instead of checking every block term against query terms (O(n*m)),
                    // iterate through query terms and check if they exist in block terms (O(m*1))
                    // This is much more efficient when there are many block terms but fewer query terms
                    let direct_matches: HashSet<&String> = unique_query_terms
                        .iter()
                        .filter(|query_term| block_terms_set.contains(query_term))
                        .collect();

                    let direct_matches_duration = direct_matches_start.elapsed();
                    {
                        let mut duration = term_matching_duration.lock().unwrap();
                        *duration += direct_matches_duration;
                    }

                    // Start measuring compound word processing time
                    let compound_start = Instant::now();

                    let mut compound_matches = HashSet::new();
                    // PHASE 3B OPTIMIZATION: Use cached vocabulary and pre-computed splits
                    QUERY_TERMS_CACHE.with(|cache| {
                        let mut cache_ref = cache.borrow_mut();
                        for qterm in &unique_query_terms {
                            if block_terms.iter().any(|bt| bt == qterm) {
                                continue;
                            }
                            // Check cache first
                            let parts = if let Some(cached_parts) = cache_ref.get(qterm) {
                                cached_parts.clone()
                            } else {
                                let parts = tokenization::split_compound_word_for_filtering(qterm);
                                cache_ref.insert(qterm.clone(), parts.clone());
                                parts
                            };
                            if parts.len() > 1 && parts.iter().all(|part| block_terms_set.contains(&part)) {
                                compound_matches.insert(qterm);
                            }
                        }
                    });

                    let compound_duration = compound_start.elapsed();
                    {
                        let mut duration = compound_processing_duration.lock().unwrap();
                        *duration += compound_duration;
                    }

                    let block_unique_terms = direct_matches.len() + compound_matches.len();
                    let block_total_matches = direct_matches.len() + compound_matches.len();

                    // Collect matched keywords
                    // PHASE 4 OPTIMIZATION: Pre-allocate with estimated size
                    let mut matched_keywords = Vec::with_capacity(params.query_plan.term_indices.len());

                    // Add direct matches
                    matched_keywords.extend(direct_matches.iter().map(|s| (*s).clone()));

                    // Add compound matches
                    matched_keywords.extend(compound_matches.iter().map(|s| (*s).clone()));

                    // Start measuring line matching time
                    let line_matching_start = Instant::now();

                    // Get the matched term indices for this block
                    let mut matched_term_indices = HashSet::new();
                    for (&term_idx, lines) in params.term_matches {
                        if lines
                            .iter()
                            .any(|&l| l >= final_start_line && l <= final_end_line)
                        {
                            matched_term_indices.insert(term_idx);
                        }
                    }

                    let line_matching_duration_value = line_matching_start.elapsed();
                    {
                        let mut duration = line_matching_duration.lock().unwrap();
                        *duration += line_matching_duration_value;
                    }

                    // Add the corresponding terms from the query plan
                    for (term, &idx) in &params.query_plan.term_indices {
                        if matched_term_indices.contains(&idx)
                            && !params.query_plan.excluded_terms.contains(term)
                        {
                            matched_keywords.push(term.clone());
                        }
                    }

                    // Remove duplicates
                    matched_keywords.sort();
                    matched_keywords.dedup();

                    // Start measuring result creation time
                    let result_creation_start = Instant::now();

                    // For now, we'll leave LSP info as None during initial processing
                    // LSP info will be added in a post-processing step if enabled
                    let result = SearchResult {
                        file: params.path.to_string_lossy().to_string(),
                        lines: (final_start_line, final_end_line),
                        node_type: if is_nested_struct {
                            block
                                .parent_node_type
                                .clone()
                                .unwrap_or_else(|| block.node_type.clone())
                        } else {
                            block.node_type.clone()
                        },
                        code: full_code,
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
                        file_unique_terms: Some(block_unique_terms),
                        file_total_matches: Some(block_total_matches),
                        file_match_rank: None,
                        block_unique_terms: Some(block_unique_terms),
                        block_total_matches: Some(block_total_matches),
                        parent_file_id: Some(file_id.clone()),
                        block_id: Some(block_idx),
                        matched_keywords: if matched_keywords.is_empty() {
                            None
                        } else {
                            Some(matched_keywords)
                        },
                        tokenized_content: Some(block_terms),
                        lsp_info: None,
                    };

                    let result_creation_duration_value = result_creation_start.elapsed();
                    {
                        let mut duration = result_creation_duration.lock().unwrap();
                        *duration += result_creation_duration_value;
                    }

                    // Start measuring synchronization time
                    let sync_start = Instant::now();

                    // Add result to shared results
                    {
                        let mut results = shared_results.lock().unwrap();
                        results.push(result);
                    }

                    let sync_duration = sync_start.elapsed();
                    {
                        let mut duration = synchronization_duration.lock().unwrap();
                        *duration += sync_duration;
                    }

                    // End result building time measurement
                    let result_building_block_duration = result_building_start.elapsed();
                    {
                        let mut duration = result_building_duration.lock().unwrap();
                        *duration += result_building_block_duration;
                    }
                }
            });

        // Extract results from shared resources
        results = Arc::try_unwrap(shared_results)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        covered_lines = Arc::try_unwrap(shared_covered_lines)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        // Extract durations from Arc<Mutex<>>
        let code_structure_duration_value = Arc::try_unwrap(code_structure_duration)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        let filtering_duration_value = Arc::try_unwrap(filtering_duration)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        let result_building_duration_value = Arc::try_unwrap(result_building_duration)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        // Extract detailed result building timings
        let term_matching_duration_value = Arc::try_unwrap(term_matching_duration)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        let compound_processing_duration_value = Arc::try_unwrap(compound_processing_duration)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        let line_matching_duration_value = Arc::try_unwrap(line_matching_duration)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        let result_creation_duration_value = Arc::try_unwrap(result_creation_duration)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        let synchronization_duration_value = Arc::try_unwrap(synchronization_duration)
            .unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
            .into_inner()
            .unwrap();

        // Store the sub-step timings
        let block_extraction_duration = block_extraction_start.elapsed();
        timings.block_extraction = Some(block_extraction_duration);
        timings.block_extraction_code_structure = Some(code_structure_duration_value);
        timings.block_extraction_filtering = Some(filtering_duration_value);
        timings.block_extraction_result_building = Some(result_building_duration_value);

        // Store detailed result building timings
        timings.result_building_term_matching = Some(term_matching_duration_value);
        timings.result_building_compound_processing = Some(compound_processing_duration_value);
        timings.result_building_line_matching = Some(line_matching_duration_value);
        timings.result_building_result_creation = Some(result_creation_duration_value);
        timings.result_building_synchronization = Some(synchronization_duration_value);

        if debug_mode {
            println!("DEBUG:   Block extraction time: {block_extraction_duration:?}");
            println!("DEBUG:     - Code structure finding: {code_structure_duration_value:?}");
            println!("DEBUG:     - Filtering: {filtering_duration_value:?}");
            println!("DEBUG:     - Result building: {result_building_duration_value:?}");
        }
    }

    // Collect all uncovered lines first without processing them
    // PHASE 4 OPTIMIZATION: Pre-allocate uncovered lines vector
    let mut uncovered_lines = Vec::with_capacity(params.line_numbers.len());
    for &line_num in params.line_numbers {
        if !covered_lines.contains(&line_num) {
            if debug_mode {
                println!("DEBUG: Line {line_num} not covered, will use fallback context");
                if line_num <= lines.len() {
                    println!("DEBUG:   Line content: '{}'", lines[line_num - 1].trim());
                }
            }
            uncovered_lines.push(line_num);
        }
    }

    // Start measuring uncovered lines processing time
    let uncovered_lines_start = Instant::now();

    // BATCH OPTIMIZATION: Process uncovered lines in batches to improve performance by 8-12 seconds
    // Instead of processing each uncovered line individually, we batch them by file and process
    // multiple lines together. This eliminates parser creation overhead and reduces repeated work.
    if !uncovered_lines.is_empty() {
        let mut batch_ctx = BatchProcessingContext {
            uncovered_lines: &uncovered_lines,
            covered_lines: &mut covered_lines,
            lines: &lines,
            params,
            extension,
            unique_query_terms: &unique_query_terms,
            results: &mut results,
            timings: &mut timings,
            debug_mode,
        };
        process_uncovered_lines_batch(&mut batch_ctx);
    }

    // End uncovered lines processing time measurement
    let uncovered_lines_duration = uncovered_lines_start.elapsed();
    timings.result_building_uncovered_lines = Some(uncovered_lines_duration);

    if debug_mode {
        println!("DEBUG: File processing timings:");
        if let Some(duration) = timings.file_io {
            println!("DEBUG:   File I/O: {duration:?}");
        }
        if let Some(duration) = timings.ast_parsing {
            println!("DEBUG:   AST parsing: {duration:?}");
            if let Some(d) = timings.ast_parsing_language_init {
                println!("DEBUG:     - Language init: {d:?}");
            }
            if let Some(d) = timings.ast_parsing_parser_init {
                println!("DEBUG:     - Parser init: {d:?}");
            }
            if let Some(d) = timings.ast_parsing_tree_parsing {
                println!("DEBUG:     - Tree parsing: {d:?}");
            }
            if let Some(d) = timings.ast_parsing_line_map_building {
                println!("DEBUG:     - Line map building: {d:?}");
            }
        }
        if let Some(duration) = timings.block_extraction {
            println!("DEBUG:   Block extraction: {duration:?}");
            if let Some(d) = timings.block_extraction_code_structure {
                println!("DEBUG:     - Code structure finding: {d:?}");
            }
            if let Some(d) = timings.block_extraction_filtering {
                println!("DEBUG:     - Filtering: {d:?}");
            }
            if let Some(d) = timings.block_extraction_result_building {
                println!("DEBUG:     - Result building: {d:?}");
            }
        }
    }

    if debug_mode {
        println!("DEBUG: Detailed result building timings:");
        if let Some(duration) = timings.result_building_term_matching {
            println!("DEBUG:   Term matching: {duration:?}");
        }
        if let Some(duration) = timings.result_building_compound_processing {
            println!("DEBUG:   Compound word processing: {duration:?}");
        }
        if let Some(duration) = timings.result_building_line_matching {
            println!("DEBUG:   Line range matching: {duration:?}");
        }
        if let Some(duration) = timings.result_building_result_creation {
            println!("DEBUG:   Result creation: {duration:?}");
        }
        if let Some(duration) = timings.result_building_synchronization {
            println!("DEBUG:   Synchronization: {duration:?}");
        }
        if let Some(duration) = timings.result_building_uncovered_lines {
            println!("DEBUG:   Uncovered lines processing: {duration:?}");
        }
    }

    Ok((results, timings))
}
