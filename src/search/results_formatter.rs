use std::path::Path;
use probe_code::models::SearchResult;
use colored::*;

/// Function to format and print search results according to the specified format
pub fn format_and_print_search_results(results: &[SearchResult], dry_run: bool) {
    // Check if debug mode is enabled
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
    
    // Check if colors should be disabled
    let no_color = std::env::var("NO_COLOR").is_ok();
    
    // Filter out results with empty file paths
    let valid_results: Vec<&SearchResult> = results.iter().filter(|r| !r.file.is_empty()).collect();
    
    if !valid_results.is_empty() {
        if dry_run {
            // More compact header for dry-run mode
            println!("{}", format!("Found {len} results:", len = valid_results.len()).bold());
        } else {
            // Full header for normal mode
            println!("{}", "╭─────────────────────────────────────────────────╮".cyan());
            println!("{} {} {}", "│".cyan(), format!("Found {len} results", len = valid_results.len()).bold(), "│".cyan());
            println!("{}", "╰─────────────────────────────────────────────────╯".cyan());
            println!();
        }
    }

    if dry_run {
        // In dry-run mode, only print file names and line numbers in a compact format
        for (index, result) in valid_results.iter().enumerate() {
            let is_full_file = result.node_type == "file";
            
            if is_full_file {
                println!("{} {}: {}",
                    format!("#{}", index + 1).bold().blue(),
                    "File".bold().green(),
                    result.file.yellow());
            } else {
                // Get line numbers if available
                let line_info = if let Some(lines) = &result.lines {
                    format!("Lines: {start}-{end}", start = lines.0, end = lines.1)
                } else {
                    "".to_string()
                };
                
                println!("{} {}: {} {} ({})",
                    format!("#{}", index + 1).bold().blue(),
                    "File".bold().green(),
                    result.file.yellow(),
                    line_info.cyan(),
                    result.node_type.cyan());
            }
        }
    } else {
        // Normal mode with full content
        for (index, result) in valid_results.iter().enumerate() {
            // Get file extension
            let file_path = Path::new(&result.file);
            let extension = file_path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("");

            // Check if this is a full file or partial file
            let is_full_file = result.node_type == "file";

            // Print result number
            println!("{} {}", "Result".bold().blue(), format!("#{num}", num = index + 1).bold().blue());
            
            // Print the file path and node info with color
            if is_full_file {
                println!("{} {}", "File:".bold().green(), result.file.yellow());
            } else {
                println!("{} {} ({})", "File:".bold().green(), result.file.yellow(), result.node_type.cyan());
                if !result.node_path.is_empty() {
                    println!("{} {}", "Node:".bold().green(), result.node_path.cyan());
                }
            }

            // Print the score if available and in debug mode
            if debug_mode {
                if let Some(score) = result.score {
                    println!("{} {score:.6}", "Score:".dimmed());
                }
                if let Some(tfidf_score) = result.tfidf_score {
                    println!("{} {tfidf_score:.6}", "TF-IDF Score:".dimmed());
                }
                if let Some(bm25_score) = result.bm25_score {
                    // Check if this is actually a BERT score
                    let is_bert_score = result.score == result.bm25_score && result.score.is_some();
                    if is_bert_score {
                        println!("{} {bm25_score:.6}", "BERT Score:".dimmed());
                    } else {
                        println!("{} {bm25_score:.6}", "BM25 Score:".dimmed());
                    }
                }
                if let Some(content_matches) = &result.content_matches {
                    let content_matches_str = content_matches.join(", ");
                    println!("{} {content_matches_str}", "Content matches:".dimmed());
                }
                if let Some(filename_matches) = &result.filename_matches {
                    let filename_matches_str = filename_matches.join(", ");
                    println!("{} {filename_matches_str}", "Filename matches:".dimmed());
                }
                if let Some(unique_terms) = result.unique_terms {
                    println!("{} {unique_terms}", "Unique terms matched:".dimmed());
                }
                if let Some(total_matches) = result.total_matches {
                    println!("{} {total_matches}", "Total term matches:".dimmed());
                }
                
                // Display block-level match statistics in debug mode
                if let Some(block_unique_terms) = result.block_unique_terms {
                    println!("{} {block_unique_terms}", "Block unique terms matched:".dimmed());
                }
                if let Some(block_total_matches) = result.block_total_matches {
                    println!("{} {block_total_matches}", "Block total term matches:".dimmed());
                }
            }

            // Print the content with syntax highlighting if available
            if !result.content.is_empty() {
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

                println!("{}", "Code:".bold().magenta());
                
                // Print the content with or without syntax highlighting
                if !language.is_empty() {
                    println!("{}", format!("```{language}").cyan());
                    println!("{result.content}");
                    println!("{}", "```".cyan());
                } else {
                    println!("{}", "```".cyan());
                    println!("{result.content}");
                    println!("{}", "```".cyan());
                }
            }

            // Print a separator between results
            if index < valid_results.len() - 1 {
                println!();
                println!("{}", "─".repeat(50).cyan());
                println!();
            }
        }
    }
}
