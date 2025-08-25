use anyhow::{Context, Result};
use ast_grep_core::AstGrep;
use ast_grep_language::SupportLang;
use colored::*;
use ignore::WalkBuilder;
use probe_code::path_resolver::resolve_path;
use rayon::prelude::*; // Added import
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Represents a match found by ast-grep
pub struct AstMatch {
    pub file_path: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub column_start: usize,
    pub column_end: usize,
    pub matched_text: String,
}

/// Options for the ast-grep query
pub struct QueryOptions<'a> {
    pub path: &'a Path,
    pub pattern: &'a str,
    pub language: Option<&'a str>,
    pub ignore: &'a [String],
    pub allow_tests: bool,
    pub max_results: Option<usize>,
    #[allow(dead_code)]
    pub format: &'a str,
    pub no_gitignore: bool,
}

/// Convert a language string to the corresponding SupportLang
fn get_language(lang: &str) -> Option<SupportLang> {
    match lang.to_lowercase().as_str() {
        "rust" => Some(SupportLang::Rust),
        "javascript" => Some(SupportLang::JavaScript),
        "typescript" => Some(SupportLang::TypeScript),
        "python" => Some(SupportLang::Python),
        "go" => Some(SupportLang::Go),
        "c" => Some(SupportLang::C),
        "cpp" => Some(SupportLang::Cpp),
        "java" => Some(SupportLang::Java),
        "ruby" => Some(SupportLang::Ruby),
        "php" => Some(SupportLang::Php),
        "swift" => Some(SupportLang::Swift),
        "csharp" => Some(SupportLang::CSharp),
        _ => None,
    }
}

/// Get the file extension for a language
fn get_file_extension(lang: &str) -> Vec<&str> {
    match lang.to_lowercase().as_str() {
        "rust" => vec![".rs"],
        "javascript" => vec![".js", ".jsx", ".mjs"],
        "typescript" => vec![".ts", ".tsx"],
        "python" => vec![".py"],
        "go" => vec![".go"],
        "c" => vec![".c", ".h"],
        "cpp" => vec![".cpp", ".hpp", ".cc", ".hh", ".cxx", ".hxx"],
        "java" => vec![".java"],
        "ruby" => vec![".rb"],
        "php" => vec![".php"],
        "swift" => vec![".swift"],
        "csharp" => vec![".cs"],
        _ => vec![],
    }
}

/// Check if a file should be ignored based on its path
fn should_ignore_file(file_path: &Path, options: &QueryOptions) -> bool {
    let path_str = file_path.to_string_lossy();

    // Skip test files if allow_tests is false
    if !options.allow_tests
        && (path_str.contains("/test/")
            || path_str.contains("/tests/")
            || path_str.contains("_test.")
            || path_str.contains("_spec.")
            || path_str.contains(".test.")
            || path_str.contains(".spec."))
    {
        return true;
    }

    // Skip files that match custom ignore patterns
    for pattern in options.ignore {
        if path_str.contains(pattern) {
            return true;
        }
    }

    false
}

/// Perform an ast-grep query on a single file
fn query_file(file_path: &Path, options: &QueryOptions) -> Result<Vec<AstMatch>> {
    // Get the file extension
    let file_ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // If language is provided, check if the file has the correct extension
    if let Some(language) = options.language {
        let extensions = get_file_extension(language);
        let has_matching_ext = extensions
            .iter()
            .any(|&ext| file_path.to_string_lossy().ends_with(ext));

        if !has_matching_ext {
            return Ok(vec![]);
        }
    }

    // Read the file content
    let content = fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    // Get the language for ast-grep
    let lang = if let Some(language) = options.language {
        // If language is specified, use it
        match get_language(language) {
            Some(lang) => lang,
            None => return Ok(vec![]),
        }
    } else {
        // If language is not specified, try to infer from file extension
        let inferred_lang = match file_ext {
            "rs" => Some(SupportLang::Rust),
            "js" | "jsx" | "mjs" => Some(SupportLang::JavaScript),
            "ts" | "tsx" => Some(SupportLang::TypeScript),
            "py" => Some(SupportLang::Python),
            "go" => Some(SupportLang::Go),
            "c" | "h" => Some(SupportLang::C),
            "cpp" | "hpp" | "cc" | "hh" | "cxx" | "hxx" => Some(SupportLang::Cpp),
            "java" => Some(SupportLang::Java),
            "rb" => Some(SupportLang::Ruby),
            "php" => Some(SupportLang::Php),
            "swift" => Some(SupportLang::Swift),
            "cs" => Some(SupportLang::CSharp),
            _ => None, // Unsupported extension
        };

        match inferred_lang {
            Some(lang) => lang,
            None => return Ok(vec![]), // Skip files with unsupported extensions
        }
    };

    // Create the document and grep instance
    let grep = AstGrep::new(&content, lang);

    // Create the pattern and find all matches
    let matches = match std::panic::catch_unwind(|| {
        grep.root().find_all(options.pattern).collect::<Vec<_>>()
    }) {
        Ok(matches) => matches,
        Err(_) => {
            // Only print error if language is explicitly specified
            // This suppresses errors during auto-detection
            if options.language.is_some() {
                eprintln!(
                    "Error parsing pattern: '{}' is not a valid ast-grep pattern",
                    options.pattern
                );
            }
            return Ok(vec![]);
        }
    };

    // Convert matches to AstMatch structs
    let mut ast_matches = Vec::new();
    for node in matches {
        let range = node.range();

        // Convert byte offsets to line and column numbers
        let mut line_start = 1;
        let mut column_start = 1;
        let mut line_end = 1;
        let mut column_end = 1;

        let mut current_line = 1;
        let mut current_column = 1;

        for (i, c) in content.char_indices() {
            if i == range.start {
                line_start = current_line;
                column_start = current_column;
            }
            if i == range.end {
                line_end = current_line;
                column_end = current_column;
                break;
            }

            if c == '\n' {
                current_line += 1;
                current_column = 1;
            } else {
                current_column += 1;
            }
        }

        ast_matches.push(AstMatch {
            file_path: file_path.to_path_buf(),
            line_start,
            line_end,
            column_start,
            column_end,
            matched_text: node.text().to_string(),
        });
    }

    Ok(ast_matches)
}

pub fn perform_query(options: &QueryOptions) -> Result<Vec<AstMatch>> {
    // Suppress panic output if language is not specified
    let suppress_output = options.language.is_none();

    // Set a custom panic hook to suppress panic messages if needed
    let original_hook = if suppress_output {
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {
            // Do nothing, effectively suppressing the panic message
        }));
        Some(original_hook)
    } else {
        None
    };

    // Resolve the path if it's a special format (e.g., "go:github.com/user/repo")
    let resolved_path = if let Some(path_str) = options.path.to_str() {
        match resolve_path(path_str) {
            Ok(resolved_path) => {
                if std::env::var("PROBE_DEBUG").unwrap_or_default() == "1" {
                    println!(
                        "DEBUG: Resolved path '{}' to '{}'",
                        path_str,
                        resolved_path.display()
                    );
                }
                resolved_path
            }
            Err(err) => {
                if std::env::var("PROBE_DEBUG").unwrap_or_default() == "1" {
                    println!("DEBUG: Failed to resolve path '{path_str}': {err}");
                }
                // Fall back to the original path
                options.path.to_path_buf()
            }
        }
    } else {
        // If we can't convert the path to a string, use it as is
        options.path.to_path_buf()
    };

    // Collect file paths using WalkBuilder to conditionally respect gitignore
    let mut builder = WalkBuilder::new(&resolved_path);

    // Configure gitignore handling based on the no_gitignore option
    if !options.no_gitignore {
        builder.git_ignore(true);
        builder.git_global(true);
        builder.git_exclude(true);
    } else {
        builder.git_ignore(false);
        builder.git_global(false);
        builder.git_exclude(false);
    }

    let file_paths: Vec<PathBuf> = builder
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_some_and(|ft| ft.is_file()))
        .filter(|entry| !should_ignore_file(entry.path(), options))
        .map(|entry| entry.path().to_path_buf())
        .collect();

    // Process files in parallel
    let all_matches: Vec<AstMatch> = file_paths
        .par_iter()
        .flat_map(|path| {
            std::panic::catch_unwind(|| query_file(path, options))
                .unwrap_or_else(|_| {
                    // Panic was caught, return empty results
                    Ok(vec![])
                })
                .unwrap_or_else(|_| {
                    // Error was caught, return empty results
                    vec![]
                })
        })
        .collect();

    // Restore the original panic hook if we changed it
    if let Some(hook) = original_hook {
        std::panic::set_hook(hook);
    }

    // Apply max_results limit
    let mut all_matches = all_matches;
    if let Some(max) = options.max_results {
        all_matches.truncate(max);
    }

    Ok(all_matches)
}

/// Helper function to escape XML special characters
fn escape_xml(s: &str) -> String {
    s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&apos;")
}

/// Format and print the query results
pub fn format_and_print_query_results(matches: &[AstMatch], format: &str) -> Result<()> {
    match format {
        "color" | "terminal" => {
            for m in matches {
                println!(
                    "{}",
                    format!(
                        "{}:{}:{}",
                        m.file_path.display(),
                        m.line_start,
                        m.column_start
                    )
                    .cyan()
                );
                println!("{}", m.matched_text.trim());
                println!();
            }
        }
        "plain" => {
            for m in matches {
                println!(
                    "{}:{}:{}",
                    m.file_path.display(),
                    m.line_start,
                    m.column_start
                );
                println!("{}", m.matched_text.trim());
                println!();
            }
        }
        "markdown" => {
            for m in matches {
                println!(
                    "**{}:{}:{}**",
                    m.file_path.display(),
                    m.line_start,
                    m.column_start
                );

                // Determine language for code block
                let lang = m
                    .file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");

                println!("```{lang}");
                println!("{}", m.matched_text.trim());
                println!("```");
                println!();
            }
        }
        "json" => {
            // BATCH TOKENIZATION WITH DEDUPLICATION OPTIMIZATION for query JSON output:
            // Process all matched text in batch to leverage content deduplication
            use probe_code::search::search_tokens::sum_tokens_with_deduplication;
            let matched_texts: Vec<&str> =
                matches.iter().map(|m| m.matched_text.as_str()).collect();
            let total_tokens = sum_tokens_with_deduplication(&matched_texts);

            // Create standardized results
            let json_matches_standardized: Vec<_> = matches
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "file": m.file_path.to_string_lossy(),
                        "lines": [m.line_start, m.line_end],
                        "node_type": "match",
                        "content": m.matched_text,
                        "column_start": m.column_start,
                        "column_end": m.column_end
                    })
                })
                .collect();

            // Create the wrapper object
            let wrapper = serde_json::json!({
                "results": json_matches_standardized,
                "summary": {
                    "count": matches.len(),
                    "total_bytes": matches.iter().map(|m| m.matched_text.len()).sum::<usize>(),
                    "total_tokens": total_tokens
                },
                "version": probe_code::version::get_version()
            });

            println!("{}", serde_json::to_string_pretty(&wrapper)?);
        }
        "xml" => {
            println!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
            println!("<probe_results>");

            for m in matches {
                println!("  <result>");
                println!(
                    "    <file>{}</file>",
                    escape_xml(&m.file_path.to_string_lossy())
                );
                println!("    <lines>{}-{}</lines>", m.line_start, m.line_end);
                println!("    <node_type>match</node_type>");
                println!("    <column_start>{}</column_start>", m.column_start);
                println!("    <column_end>{}</column_end>", m.column_end);
                println!("    <code><![CDATA[{}]]></code>", m.matched_text.trim());
                println!("  </result>");
            }

            // Add summary section
            println!("  <summary>");
            println!("    <count>{}</count>", matches.len());
            println!(
                "    <total_bytes>{}",
                matches.iter().map(|m| m.matched_text.len()).sum::<usize>()
            );

            // BATCH TOKENIZATION WITH DEDUPLICATION OPTIMIZATION for query XML output:
            // Process all matched text in batch to leverage content deduplication
            use probe_code::search::search_tokens::sum_tokens_with_deduplication;
            let matched_texts: Vec<&str> =
                matches.iter().map(|m| m.matched_text.as_str()).collect();
            let total_tokens = sum_tokens_with_deduplication(&matched_texts);

            println!("    <total_tokens>{total_tokens}");
            println!("  </summary>");

            println!(
                "  <version>{}</version>",
                probe_code::version::get_version()
            );

            println!("</probe_results>");
        }
        _ => {
            // Default to color format
            format_and_print_query_results(matches, "color")?;
        }
    }

    Ok(())
}

/// Handle the query command
#[allow(clippy::too_many_arguments)]
pub fn handle_query(
    pattern: &str,
    path: &Path,
    language: Option<&str>,
    ignore: &[String],
    allow_tests: bool,
    max_results: Option<usize>,
    format: &str,
    no_gitignore: bool,
) -> Result<()> {
    // Print version at the start for text-based formats
    if format != "json" && format != "xml" {
        println!("Probe version: {}", probe_code::version::get_version());
    }

    // Only print information for non-JSON/XML formats
    if format != "json" && format != "xml" {
        println!("{} {}", "Pattern:".bold().green(), pattern);
        println!("{} {}", "Path:".bold().green(), path.display());

        // Print language if provided, otherwise show auto-detect
        if let Some(lang) = language {
            println!("{} {}", "Language:".bold().green(), lang);
        } else {
            println!("{} auto-detect", "Language:".bold().green());
        }

        // Show advanced options if they differ from defaults
        let mut advanced_options = Vec::<String>::new();
        if allow_tests {
            advanced_options.push("Including tests".to_string());
        }
        if no_gitignore {
            advanced_options.push("Ignoring .gitignore".to_string());
        }
        if let Some(max) = max_results {
            advanced_options.push(format!("Max results: {max}"));
        }

        if !advanced_options.is_empty() {
            println!(
                "{} {}",
                "Options:".bold().green(),
                advanced_options.join(", ")
            );
        }
    }

    let start_time = Instant::now();

    let options = QueryOptions {
        path,
        pattern,
        language,
        ignore,
        allow_tests,
        max_results,
        format,
        no_gitignore,
    };

    let matches = perform_query(&options)?;

    // Calculate search time
    let duration = start_time.elapsed();

    if matches.is_empty() {
        // For JSON and XML formats, still call format_and_print_query_results
        if format == "json" || format == "xml" {
            format_and_print_query_results(&matches, format)?;
        } else {
            // For other formats, print the "No results found" message
            println!("{}", "No results found.".yellow().bold());
            println!("Search completed in {duration:.2?}");
        }
    } else {
        // For non-JSON/XML formats, print search time
        if format != "json" && format != "xml" {
            println!("Found {} matches in {:.2?}", matches.len(), duration);
            println!();
        }

        format_and_print_query_results(&matches, format)?;

        // Skip summary for JSON and XML formats
        if format != "json" && format != "xml" {
            // Calculate and display total bytes and tokens
            let total_bytes: usize = matches.iter().map(|m| m.matched_text.len()).sum();
            let total_tokens: usize = matches
                .iter()
                .map(|m| {
                    // Import the count_tokens function locally to avoid unused import warning
                    use probe_code::search::search_tokens::count_tokens;
                    count_tokens(&m.matched_text)
                })
                .sum();

            println!("Total bytes returned: {total_bytes}");
            println!("Total tokens returned: {total_tokens}");
        }
    }

    Ok(())
}
