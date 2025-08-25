use anyhow::Result;
use std::path::Path;

use probe_code::models::SearchResult;
use probe_code::search::query::QueryPlan;
use probe_code::search::search_tokens::sum_tokens_with_deduplication;

/// Function to format and print search results according to the specified format
pub fn format_and_print_search_results(
    results: &[SearchResult],
    dry_run: bool,
    format: &str,
    query_plan: Option<&QueryPlan>,
) {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // Count valid results (with non-empty file names)
    let valid_results: Vec<&SearchResult> = results.iter().filter(|r| !r.file.is_empty()).collect();

    // Check if terminal supports colors and if output is being piped
    let use_color = match format {
        "color" => colored::control::SHOULD_COLORIZE.should_colorize(),
        _ => false,
    };

    // Handle different output formats
    match format {
        "color" if use_color => {
            format_and_print_color_results(&valid_results, dry_run, query_plan, debug_mode);
        }
        "json" => {
            if let Err(e) = format_and_print_json_results(&valid_results) {
                eprintln!("Error formatting JSON: {e}");
            }
            return; // Skip the summary output at the end
        }
        "xml" => {
            if let Err(e) = format_and_print_xml_results(&valid_results) {
                eprintln!("Error formatting XML: {e}");
            }
            return; // Skip the summary output at the end
        }
        _ => {
            // Default format (terminal)
            for result in &valid_results {
                let file_path = Path::new(&result.file);
                let extension = file_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("");
                let is_full_file = result.node_type == "file";

                if dry_run {
                    // In dry-run mode, only print file names and line numbers
                    if is_full_file {
                        println!("File: {}", result.file);
                    } else {
                        println!(
                            "File: {}, Lines: {}-{}",
                            result.file, result.lines.0, result.lines.1
                        );
                    }
                } else {
                    // Normal mode with full content
                    if is_full_file {
                        println!("File: {}", result.file);
                        println!("```{extension}");
                        println!("{}", result.code);
                        println!("```");
                    } else {
                        println!("File: {}", result.file);
                        println!(
                            "Lines: {start}-{end}",
                            start = result.lines.0,
                            end = result.lines.1
                        );
                        println!("```{extension}");
                        println!("{code}", code = result.code);
                        println!("```");
                    }

                    // Display LSP information if available (for both full file and partial results)
                    if let Some(lsp_info) = &result.lsp_info {
                        if let Some(obj) = lsp_info.as_object() {
                            if let Some(symbols) = obj.get("symbols").and_then(|v| v.as_array()) {
                                // Merged LSP information with multiple symbols
                                println!("Merged LSP Information:");
                                println!("  Symbols: {}", symbols.len());

                                for sym in symbols {
                                    if let Some(sobj) = sym.as_object() {
                                        let name = sobj
                                            .get("symbol")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown");
                                        let node_type = sobj
                                            .get("node_type")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let (start, end) = sobj
                                            .get("range")
                                            .and_then(|r| r.get("lines"))
                                            .and_then(|arr| arr.as_array())
                                            .and_then(|a| {
                                                if a.len() == 2 {
                                                    Some((
                                                        a[0].as_u64().unwrap_or(0),
                                                        a[1].as_u64().unwrap_or(0),
                                                    ))
                                                } else {
                                                    None
                                                }
                                            })
                                            .unwrap_or((0, 0));

                                        println!("  • Symbol: {name}");
                                        if start > 0 || end > 0 {
                                            println!("    Lines: {start}-{end}");
                                        }
                                        if !node_type.is_empty() {
                                            println!("    Type: {node_type}");
                                        }

                                        if let Some(ch) =
                                            sobj.get("call_hierarchy").and_then(|v| v.as_object())
                                        {
                                            if let Some(incoming) =
                                                ch.get("incoming_calls").and_then(|v| v.as_array())
                                            {
                                                if !incoming.is_empty() {
                                                    println!("    Incoming Calls:");
                                                    for call in incoming {
                                                        if let Some(call_obj) = call.as_object() {
                                                            let name = call_obj
                                                                .get("name")
                                                                .and_then(|v| v.as_str())
                                                                .unwrap_or("unknown");
                                                            let file_path = call_obj
                                                                .get("file_path")
                                                                .and_then(|v| v.as_str())
                                                                .unwrap_or("");
                                                            let line = call_obj
                                                                .get("line")
                                                                .and_then(|v| v.as_u64())
                                                                .unwrap_or(0);
                                                            let file_path = file_path
                                                                .strip_prefix("file://")
                                                                .unwrap_or(file_path);
                                                            println!("      - {name} ({file_path}:{line})");
                                                        }
                                                    }
                                                }
                                            }
                                            if let Some(outgoing) =
                                                ch.get("outgoing_calls").and_then(|v| v.as_array())
                                            {
                                                if !outgoing.is_empty() {
                                                    println!("    Outgoing Calls:");
                                                    for call in outgoing {
                                                        if let Some(call_obj) = call.as_object() {
                                                            let name = call_obj
                                                                .get("name")
                                                                .and_then(|v| v.as_str())
                                                                .unwrap_or("unknown");
                                                            let file_path = call_obj
                                                                .get("file_path")
                                                                .and_then(|v| v.as_str())
                                                                .unwrap_or("");
                                                            let line = call_obj
                                                                .get("line")
                                                                .and_then(|v| v.as_u64())
                                                                .unwrap_or(0);
                                                            let file_path = file_path
                                                                .strip_prefix("file://")
                                                                .unwrap_or(file_path);
                                                            println!("      - {name} ({file_path}:{line})");
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        if let Some(refs_count) =
                                            sobj.get("references_count").and_then(|v| v.as_u64())
                                        {
                                            if refs_count > 0 {
                                                println!("    References: {refs_count}");
                                            }
                                        }
                                        // Small gap between symbols
                                        println!();
                                    }
                                }
                            } else {
                                // Single-symbol legacy shape (backward compatible)
                                println!("LSP Information:");

                                // Display call hierarchy if available
                                if let Some(call_hierarchy) =
                                    obj.get("call_hierarchy").and_then(|v| v.as_object())
                                {
                                    // Incoming calls
                                    if let Some(incoming) = call_hierarchy
                                        .get("incoming_calls")
                                        .and_then(|v| v.as_array())
                                    {
                                        if !incoming.is_empty() {
                                            println!("  Incoming Calls:");
                                            for call in incoming {
                                                if let Some(call_obj) = call.as_object() {
                                                    let name = call_obj
                                                        .get("name")
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("unknown");
                                                    let file_path = call_obj
                                                        .get("file_path")
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("");
                                                    let line = call_obj
                                                        .get("line")
                                                        .and_then(|v| v.as_u64())
                                                        .unwrap_or(0);
                                                    let file_path = file_path
                                                        .strip_prefix("file://")
                                                        .unwrap_or(file_path);
                                                    println!("    - {name} ({file_path}:{line})");
                                                }
                                            }
                                        }
                                    }

                                    // Outgoing calls
                                    if let Some(outgoing) = call_hierarchy
                                        .get("outgoing_calls")
                                        .and_then(|v| v.as_array())
                                    {
                                        if !outgoing.is_empty() {
                                            println!("  Outgoing Calls:");
                                            for call in outgoing {
                                                if let Some(call_obj) = call.as_object() {
                                                    let name = call_obj
                                                        .get("name")
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("unknown");
                                                    let file_path = call_obj
                                                        .get("file_path")
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("");
                                                    let line = call_obj
                                                        .get("line")
                                                        .and_then(|v| v.as_u64())
                                                        .unwrap_or(0);
                                                    let file_path = file_path
                                                        .strip_prefix("file://")
                                                        .unwrap_or(file_path);
                                                    println!("    - {name} ({file_path}:{line})");
                                                }
                                            }
                                        }
                                    }
                                }

                                // Display references count if available
                                if let Some(refs_count) =
                                    obj.get("references_count").and_then(|v| v.as_u64())
                                {
                                    if refs_count > 0 {
                                        println!("  References: {refs_count}");
                                    }
                                }
                            }
                        }
                    }
                }
                if debug_mode {
                    if let Some(rank) = result.rank {
                        // Add a display order field to show the actual ordering of results
                        println!(
                            "Display Order: {}",
                            results
                                .iter()
                                .position(|r| r.file == result.file && r.lines == result.lines)
                                .unwrap_or(0)
                                + 1
                        );

                        println!("Rank: {rank}");

                        if let Some(score) = result.score {
                            println!("Combined Score: {score:.4}");
                        }

                        // Display the combined score rank if available, otherwise calculate it
                        if let Some(combined_rank) = result.combined_score_rank {
                            println!("Combined Score Rank: {combined_rank}");
                        } else {
                            // Fall back to the old behavior if the field isn't set
                            println!("Combined Score Rank: {rank}");
                        }

                        if let Some(tfidf_score) = result.tfidf_score {
                            println!("TF-IDF Score: {tfidf_score:.4}");
                        }

                        if let Some(tfidf_rank) = result.tfidf_rank {
                            println!("TF-IDF Rank: {tfidf_rank}");
                        }

                        if let Some(bm25_score) = result.bm25_score {
                            // Check if this is actually a BERT score by looking at the rank field
                            // When BERT reranking is used, both score and bm25_score are set to BERT score
                            let is_bert_score =
                                result.score == result.bm25_score && result.score.is_some();
                            if is_bert_score {
                                println!("BERT Score: {bm25_score:.4}");
                            } else {
                                println!("BM25 Score: {bm25_score:.4}");
                            }
                        }

                        if let Some(bm25_rank) = result.bm25_rank {
                            println!("BM25 Rank: {bm25_rank}");
                        }

                        // Display Hybrid 2 score and rank with more prominence
                        if let Some(new_score) = result.new_score {
                            println!("Hybrid 2 Score: {new_score:.4}");
                        }

                        if let Some(hybrid2_rank) = result.hybrid2_rank {
                            println!("Hybrid 2 Rank: {hybrid2_rank}");
                        } else if result.new_score.is_some() {
                            println!("Hybrid 2 Rank: N/A");
                        }

                        if let Some(file_unique_terms) = result.file_unique_terms {
                            println!("File Unique Terms: {file_unique_terms}");
                        }

                        if let Some(file_total_matches) = result.file_total_matches {
                            println!("File Total Matches: {file_total_matches}");
                        }

                        if let Some(file_match_rank) = result.file_match_rank {
                            println!("File Match Rank: {file_match_rank}");
                        }

                        if let Some(block_unique_terms) = result.block_unique_terms {
                            println!("Block Unique Terms: {block_unique_terms}");
                        }

                        if let Some(block_total_matches) = result.block_total_matches {
                            println!("Block Total Matches: {block_total_matches}");
                        }

                        println!("Type: {}", result.node_type);
                    }
                }
            }
        }
    }

    println!("Found {count} search results", count = valid_results.len());

    let total_bytes: usize = valid_results.iter().map(|r| r.code.len()).sum();

    // BATCH TOKENIZATION WITH DEDUPLICATION OPTIMIZATION:
    // Use batch processing with content deduplication for improved performance
    // when multiple identical code blocks need tokenization (common in search results)
    let code_blocks: Vec<&str> = valid_results.iter().map(|r| r.code.as_str()).collect();
    let total_tokens: usize = sum_tokens_with_deduplication(&code_blocks);
    println!("Total bytes returned: {total_bytes}");
    println!("Total tokens returned: {total_tokens}");
}

/// Format and print search results with color highlighting for matching words
fn format_and_print_color_results(
    results: &[&SearchResult],
    dry_run: bool,
    query_plan: Option<&QueryPlan>,
    debug_mode: bool,
) {
    use colored::*;
    use regex::Regex;

    if results.is_empty() {
        println!("No results found.");
        return;
    }

    // Print a header with the number of results
    println!("{}", format!("Found {} results", results.len()).bold());
    println!();

    // Print the results
    for (index, result) in results.iter().enumerate() {
        // Get file extension
        let file_path = Path::new(&result.file);
        let extension = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        // Check if this is a full file or partial file
        let is_full_file = result.node_type == "file";

        // Print result number
        println!(
            "{} {}",
            "Result".bold().blue(),
            format!("#{}", index + 1).bold().blue()
        );

        // Print the file path and node info with color
        if is_full_file {
            println!(
                "{label} {file}",
                label = "File:".bold().green().yellow(),
                file = result.file
            );
        } else {
            println!(
                "{} {} ({})",
                "File:".bold().green(),
                result.file.yellow(),
                result.node_type.cyan()
            );
            println!(
                "{} {}-{}",
                "Lines:".bold().green(),
                result.lines.0,
                result.lines.1
            );
        }

        // Print additional debug information if in debug mode
        if debug_mode {
            // Print the same debug info that would be shown in standard mode
            if let Some(keywords) = &result.matched_keywords {
                println!("{} {keywords:?}", "Matched Keywords:".bold().green());
            }
            if let Some(score) = result.score {
                println!("{} {score:.4}", "Score:".bold().green());
            }
            if let Some(query_plan) = query_plan {
                println!("{} {query_plan:?}", "Query Plan:".bold().green());
            }
        }

        if dry_run {
            // In dry-run mode, don't print the content
            continue;
        }

        // Determine the language for syntax highlighting
        let language = match extension {
            "rs" => "rust",
            "py" => "python",
            "js" => "javascript",
            "ts" => "typescript",
            "go" => "go",
            "c" | "h" => "c",
            "cpp" | "cc" | "cxx" | "hpp" => "cpp",
            "java" => "java",
            "rb" => "ruby",
            "php" => "php",
            "sh" => "bash",
            "md" => "markdown",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "html" => "html",
            "css" => "css",
            "sql" => "sql",
            "kt" | "kts" => "kotlin",
            "swift" => "swift",
            "scala" => "scala",
            "dart" => "dart",
            "ex" | "exs" => "elixir",
            "hs" => "haskell",
            "clj" => "clojure",
            "lua" => "lua",
            "r" => "r",
            "pl" | "pm" => "perl",
            "proto" => "protobuf",
            _ => "",
        };

        println!("{label}", label = "Code:".bold().magenta());

        // Print the code with syntax highlighting
        if !language.is_empty() {
            println!("{code_block}", code_block = format!("```{language}").cyan());
        } else {
            println!("{code_block}", code_block = "```".cyan());
        }

        // Generate patterns from the matched keywords in the search result
        let mut patterns = Vec::new();

        // Use the matched keywords from the search result if available
        if let Some(keywords) = &result.matched_keywords {
            for keyword in keywords {
                // Create a case-insensitive regex for the keyword with word boundaries
                if let Ok(regex) = Regex::new(&format!(r"(?i){}", regex::escape(keyword))) {
                    patterns.push(regex);
                }

                // Also try to match camelCase/PascalCase variations
                if let Ok(regex) = Regex::new(&format!(r"(?i){}", regex::escape(keyword))) {
                    patterns.push(regex);
                }
            }
        }

        // If no patterns were generated, add some default patterns
        if patterns.is_empty() {
            // Use lazily initialized static regexes to avoid recompilation
            lazy_static::lazy_static! {
                static ref STRUCT_REGEX: Regex = Regex::new(r"(?i)struct").unwrap();
                static ref SEARCH_REGEX: Regex = Regex::new(r"(?i)search").unwrap();
                static ref RESULT_REGEX: Regex = Regex::new(r"(?i)result").unwrap();
            }

            patterns.push(STRUCT_REGEX.clone());
            patterns.push(SEARCH_REGEX.clone());
            patterns.push(RESULT_REGEX.clone());
        }

        // Process the code line by line with inline highlighting
        for line in result.code.lines() {
            let mut output_line = String::new();
            let mut last_end = 0;
            let mut matches = Vec::new();

            // Collect all matches from all patterns
            for pattern in &patterns {
                for mat in pattern.find_iter(line) {
                    matches.push((mat.start(), mat.end()));
                }
            }

            // Sort matches by start position
            matches.sort_by_key(|&(start, _)| start);

            // Merge overlapping matches
            let mut merged_matches = Vec::new();
            for (start, end) in matches {
                if let Some((_, prev_end)) = merged_matches.last() {
                    if start <= *prev_end {
                        // Overlapping match, extend the previous one
                        let last_idx = merged_matches.len() - 1;
                        merged_matches[last_idx].1 = end.max(*prev_end);
                        continue;
                    }
                }
                merged_matches.push((start, end));
            }

            // Build the highlighted line
            for &(start, end) in &merged_matches {
                // Add text before the match
                if start > last_end {
                    output_line.push_str(&line[last_end..start]);
                }

                // Add the highlighted match
                let matched_text = &line[start..end];
                output_line.push_str(&matched_text.yellow().bold().to_string());

                last_end = end;
            }

            // Add any remaining text
            if last_end < line.len() {
                output_line.push_str(&line[last_end..]);
            }

            // Print the line (highlighted or original if no matches)
            if !merged_matches.is_empty() {
                println!("{output_line}");
            } else {
                println!("{line}");
            }
        }

        println!();

        // Display LSP information if available
        if let Some(lsp_info) = &result.lsp_info {
            if debug_mode {
                eprintln!("DEBUG: Found LSP info for result");
            }
            // Parse the JSON structure directly
            if let Some(obj) = lsp_info.as_object() {
                if let Some(symbols) = obj.get("symbols").and_then(|v| v.as_array()) {
                    // Merged LSP information with multiple symbols
                    println!("{}", "Merged LSP Information:".bold().blue());
                    println!("  {} {}", "Symbols:".bold(), symbols.len());

                    for sym in symbols {
                        if let Some(sobj) = sym.as_object() {
                            let name = sobj
                                .get("symbol")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let node_type =
                                sobj.get("node_type").and_then(|v| v.as_str()).unwrap_or("");

                            // Optional range display
                            let (start, end) = sobj
                                .get("range")
                                .and_then(|r| r.get("lines"))
                                .and_then(|arr| arr.as_array())
                                .and_then(|a| {
                                    if a.len() == 2 {
                                        Some((
                                            a[0].as_u64().unwrap_or(0),
                                            a[1].as_u64().unwrap_or(0),
                                        ))
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or((0, 0));

                            println!(
                                "  • {} {}{}",
                                "Symbol:".bold(),
                                name,
                                if start > 0 || end > 0 {
                                    format!("   Lines: {start}-{end}")
                                } else {
                                    "".to_string()
                                }
                            );

                            if !node_type.is_empty() {
                                println!("    Type: {node_type}");
                            }

                            if let Some(ch) = sobj.get("call_hierarchy").and_then(|v| v.as_object())
                            {
                                // Incoming calls
                                if let Some(incoming) =
                                    ch.get("incoming_calls").and_then(|v| v.as_array())
                                {
                                    if !incoming.is_empty() {
                                        println!("    {}", "Incoming Calls:".bold());
                                        for call in incoming {
                                            if let Some(call_obj) = call.as_object() {
                                                let name = call_obj
                                                    .get("name")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("unknown");
                                                let file_path = call_obj
                                                    .get("file_path")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                let line = call_obj
                                                    .get("line")
                                                    .and_then(|v| v.as_u64())
                                                    .unwrap_or(0);
                                                let file_path = file_path
                                                    .strip_prefix("file://")
                                                    .unwrap_or(file_path);
                                                println!("      - {name} ({file_path}:{line})");
                                            }
                                        }
                                    }
                                }
                                // Outgoing calls
                                if let Some(outgoing) =
                                    ch.get("outgoing_calls").and_then(|v| v.as_array())
                                {
                                    if !outgoing.is_empty() {
                                        println!("    {}", "Outgoing Calls:".bold());
                                        for call in outgoing {
                                            if let Some(call_obj) = call.as_object() {
                                                let name = call_obj
                                                    .get("name")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("unknown");
                                                let file_path = call_obj
                                                    .get("file_path")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                let line = call_obj
                                                    .get("line")
                                                    .and_then(|v| v.as_u64())
                                                    .unwrap_or(0);
                                                let file_path = file_path
                                                    .strip_prefix("file://")
                                                    .unwrap_or(file_path);
                                                println!("      - {name} ({file_path}:{line})");
                                            }
                                        }
                                    }
                                }
                            }

                            if let Some(refs_count) =
                                sobj.get("references_count").and_then(|v| v.as_u64())
                            {
                                if refs_count > 0 {
                                    println!("    {}: {}", "References".bold(), refs_count);
                                }
                            }

                            // Small gap between symbols
                            println!();
                        }
                    }
                } else {
                    // Single-symbol legacy shape (backward compatible)
                    println!("{}", "LSP Information:".bold().blue());

                    // Display call hierarchy if available
                    if let Some(call_hierarchy) =
                        obj.get("call_hierarchy").and_then(|v| v.as_object())
                    {
                        // Incoming calls
                        if let Some(incoming) = call_hierarchy
                            .get("incoming_calls")
                            .and_then(|v| v.as_array())
                        {
                            if !incoming.is_empty() {
                                println!("  {}", "Incoming Calls:".bold());
                                for call in incoming {
                                    if let Some(call_obj) = call.as_object() {
                                        let name = call_obj
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown");
                                        let file_path = call_obj
                                            .get("file_path")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let line = call_obj
                                            .get("line")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let file_path =
                                            file_path.strip_prefix("file://").unwrap_or(file_path);
                                        println!("    - {name} ({file_path}:{line})");
                                    }
                                }
                            }
                        }

                        // Outgoing calls
                        if let Some(outgoing) = call_hierarchy
                            .get("outgoing_calls")
                            .and_then(|v| v.as_array())
                        {
                            if !outgoing.is_empty() {
                                println!("  {}", "Outgoing Calls:".bold());
                                for call in outgoing {
                                    if let Some(call_obj) = call.as_object() {
                                        let name = call_obj
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown");
                                        let file_path = call_obj
                                            .get("file_path")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let line = call_obj
                                            .get("line")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let file_path =
                                            file_path.strip_prefix("file://").unwrap_or(file_path);
                                        println!("    - {name} ({file_path}:{line})");
                                    }
                                }
                            }
                        }
                    }

                    // Display references count if available
                    if let Some(refs_count) = obj.get("references_count").and_then(|v| v.as_u64()) {
                        if refs_count > 0 {
                            println!("  {}: {}", "References".bold(), refs_count);
                        }
                    }
                }

                println!();
            }
        }

        // Print a separator between results
        if index < results.len() - 1 {
            println!();
            println!("{separator}", separator = "─".repeat(50).cyan());
            println!();
        }

        if debug_mode {
            if let Some(rank) = result.rank {
                // Add a display order field to show the actual ordering of results
                println!(
                    "Display Order: {}",
                    results
                        .iter()
                        .position(|r| r.file == result.file && r.lines == result.lines)
                        .unwrap_or(0)
                        + 1
                );

                println!("Rank: {rank}");

                if let Some(score) = result.score {
                    println!("Combined Score: {score:.4}");
                }

                // Display the combined score rank if available, otherwise calculate it
                if let Some(combined_rank) = result.combined_score_rank {
                    println!("Combined Score Rank: {combined_rank}");
                } else {
                    // Fall back to the old behavior if the field isn't set
                    println!("Combined Score Rank: {rank}");
                }

                if let Some(tfidf_score) = result.tfidf_score {
                    println!("TF-IDF Score: {tfidf_score:.4}");
                }

                if let Some(tfidf_rank) = result.tfidf_rank {
                    println!("TF-IDF Rank: {tfidf_rank}");
                }

                if let Some(bm25_score) = result.bm25_score {
                    // Check if this is actually a BERT score by looking at the rank field
                    // When BERT reranking is used, both score and bm25_score are set to BERT score
                    let is_bert_score = result.score == result.bm25_score && result.score.is_some();
                    if is_bert_score {
                        println!("BERT Score: {bm25_score:.4}");
                    } else {
                        println!("BM25 Score: {bm25_score:.4}");
                    }
                }

                if let Some(bm25_rank) = result.bm25_rank {
                    println!("BM25 Rank: {bm25_rank}");
                }

                // Display Hybrid 2 score and rank with more prominence
                if let Some(new_score) = result.new_score {
                    println!("Hybrid 2 Score: {new_score:.4}");
                }

                if let Some(hybrid2_rank) = result.hybrid2_rank {
                    println!("Hybrid 2 Rank: {hybrid2_rank}");
                } else if result.new_score.is_some() {
                    println!("Hybrid 2 Rank: N/A");
                }

                if let Some(file_unique_terms) = result.file_unique_terms {
                    println!("File Unique Terms: {file_unique_terms}");
                }

                if let Some(file_total_matches) = result.file_total_matches {
                    println!("File Total Matches: {file_total_matches}");
                }

                if let Some(file_match_rank) = result.file_match_rank {
                    println!("File Match Rank: {file_match_rank}");
                }

                if let Some(block_unique_terms) = result.block_unique_terms {
                    println!("Block Unique Terms: {block_unique_terms}");
                }

                if let Some(block_total_matches) = result.block_total_matches {
                    println!("Block Total Matches: {block_total_matches}");
                }

                println!("Type: {}", result.node_type);
            }
        }
    }
}

/// Helper function to escape XML special characters
fn escape_xml(s: &str) -> String {
    s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&apos;")
}

/// Format and print search results in JSON format
fn format_and_print_json_results(results: &[&SearchResult]) -> Result<()> {
    // Create a simplified version of the results for JSON output
    #[derive(serde::Serialize)]
    struct JsonResult<'a> {
        file: &'a str,
        lines: [usize; 2],
        node_type: &'a str,
        code: &'a str,
        // Include other relevant fields
        matched_keywords: Option<&'a Vec<String>>,
        score: Option<f64>,
        tfidf_score: Option<f64>,
        bm25_score: Option<f64>,
        file_unique_terms: Option<usize>,
        file_total_matches: Option<usize>,
        block_unique_terms: Option<usize>,
        block_total_matches: Option<usize>,
        lsp_info: Option<&'a serde_json::Value>,
    }

    let json_results: Vec<JsonResult> = results
        .iter()
        .map(|r| {
            if std::env::var("PROBE_DEBUG").is_ok() {
                if r.lsp_info.is_some() {
                    eprintln!(
                        "DEBUG format_json: Result has LSP info for file {} at lines {:?}",
                        r.file, r.lines
                    );
                } else {
                    eprintln!(
                        "DEBUG format_json: Result has NO LSP info for file {} at lines {:?}",
                        r.file, r.lines
                    );
                }
            }
            JsonResult {
                file: &r.file,
                lines: [r.lines.0, r.lines.1],
                node_type: &r.node_type,
                code: &r.code,
                matched_keywords: r.matched_keywords.as_ref(),
                score: r.score,
                tfidf_score: r.tfidf_score,
                bm25_score: r.bm25_score,
                file_unique_terms: r.file_unique_terms,
                file_total_matches: r.file_total_matches,
                block_unique_terms: r.block_unique_terms,
                block_total_matches: r.block_total_matches,
                lsp_info: r.lsp_info.as_ref(),
            }
        })
        .collect();

    // BATCH TOKENIZATION WITH DEDUPLICATION OPTIMIZATION for JSON output:
    // Process all code blocks in batch to leverage content deduplication
    let code_blocks: Vec<&str> = results.iter().map(|r| r.code.as_str()).collect();
    let total_tokens = sum_tokens_with_deduplication(&code_blocks);

    // Create a wrapper object with results and summary
    let wrapper = serde_json::json!({
        "results": json_results,
        "summary": {
            "count": results.len(),
            "total_bytes": results.iter().map(|r| r.code.len()).sum::<usize>(),
            "total_tokens": total_tokens,
        },
        "version": probe_code::version::get_version()
    });

    println!("{json}", json = serde_json::to_string_pretty(&wrapper)?);
    Ok(())
}

/// Format and print search results in XML format
fn format_and_print_xml_results(results: &[&SearchResult]) -> Result<()> {
    println!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    println!("<probe_results>");

    for result in results {
        println!("  <result>");
        println!("    <file>{file}</file>", file = escape_xml(&result.file));
        println!(
            "    <lines>{start}-{end}</lines>",
            start = result.lines.0,
            end = result.lines.1
        );
        println!(
            "    <node_type>{}</node_type>",
            escape_xml(&result.node_type)
        );

        if let Some(keywords) = &result.matched_keywords {
            println!("    <matched_keywords>");
            for keyword in keywords {
                println!(
                    "      <keyword>{keyword}</keyword>",
                    keyword = escape_xml(keyword)
                );
            }
            println!("    </matched_keywords>");
        }

        if let Some(score) = result.score {
            println!("    <score>{score:.4}</score>");
        }

        if let Some(tfidf_score) = result.tfidf_score {
            println!("    <tfidf_score>{tfidf_score:.4}</tfidf_score>");
        }

        if let Some(bm25_score) = result.bm25_score {
            println!("    <bm25_score>{bm25_score:.4}</bm25_score>");
        }

        if let Some(file_unique_terms) = result.file_unique_terms {
            println!("    <file_unique_terms>{file_unique_terms}</file_unique_terms>");
        }

        if let Some(file_total_matches) = result.file_total_matches {
            println!("    <file_total_matches>{file_total_matches}</file_total_matches>");
        }

        if let Some(block_unique_terms) = result.block_unique_terms {
            println!("    <block_unique_terms>{block_unique_terms}</block_unique_terms>");
        }

        if let Some(block_total_matches) = result.block_total_matches {
            println!("    <block_total_matches>{block_total_matches}</block_total_matches>");
        }

        println!("    <code><![CDATA[{code}]]></code>", code = result.code);
        println!("  </result>");
    }

    // Add summary section
    println!("  <summary>");
    println!("    <count>{}</count>", results.len());
    println!(
        "    <total_bytes>{total_bytes}</total_bytes>",
        total_bytes = results.iter().map(|r| r.code.len()).sum::<usize>()
    );
    // BATCH TOKENIZATION WITH DEDUPLICATION OPTIMIZATION for XML output:
    // Process all code blocks in batch to leverage content deduplication
    let code_blocks: Vec<&str> = results.iter().map(|r| r.code.as_str()).collect();
    let total_tokens = sum_tokens_with_deduplication(&code_blocks);

    println!("    <total_tokens>{total_tokens}</total_tokens>");
    println!("  </summary>");

    println!(
        "  <version>{}</version>",
        probe_code::version::get_version()
    );

    println!("</probe_results>");
    Ok(())
}
