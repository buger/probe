//! Extract command functionality for extracting code blocks from files.
//!
//! This module provides functions for extracting code blocks from files based on file paths
//! and optional line numbers. When a line number is specified, it uses tree-sitter to find
//! the closest suitable parent node (function, struct, class, etc.) for that line.

mod file_paths;
mod formatter;
mod processor;
mod prompts;
pub mod symbol_finder;

// Re-export public functions
#[allow(unused_imports)]
pub use file_paths::{
    extract_file_paths_from_git_diff, extract_file_paths_from_text, is_git_diff_format,
    parse_file_with_line,
};
#[allow(unused_imports)]
pub use formatter::{
    format_and_print_extraction_results, format_extraction_dry_run, format_extraction_results,
};
#[allow(unused_imports)]
pub use processor::process_file_for_extraction;
#[allow(unused_imports)]
pub use prompts::PromptTemplate;

use anyhow::Result;
use probe_code::extract::file_paths::{set_custom_ignores, FilePathInfo};
use probe_code::models::SearchResult;
use std::collections::HashSet;
use std::io::Read;
#[allow(unused_imports)]
use std::path::PathBuf;

/// Options for the extract command
pub struct ExtractOptions {
    /// Files to extract from
    pub files: Vec<String>,
    /// Custom patterns to ignore
    pub custom_ignores: Vec<String>,
    /// Number of context lines to include
    pub context_lines: usize,
    /// Output format
    pub format: String,
    /// Whether to read from clipboard
    pub from_clipboard: bool,
    /// Path to input file to read from
    pub input_file: Option<String>,
    /// Whether to write to clipboard
    pub to_clipboard: bool,
    /// Whether to perform a dry run
    pub dry_run: bool,
    /// Whether to parse input as git diff format
    pub diff: bool,
    /// Whether to allow test files and test code blocks
    pub allow_tests: bool,
    /// Whether to keep and display the original input content
    pub keep_input: bool,
    /// Optional prompt template for LLM models
    pub prompt: Option<prompts::PromptTemplate>,
    /// Optional user instructions for LLM models
    pub instructions: Option<String>,
    /// Whether to ignore .gitignore files
    pub no_gitignore: bool,
    /// Whether to enable LSP integration for call hierarchy and reference graphs
    pub lsp: bool,
}

/// Handle the extract command
pub fn handle_extract(options: ExtractOptions) -> Result<()> {
    use arboard::Clipboard;
    use colored::*;

    // Check if debug mode is enabled
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("\n[DEBUG] ===== Extract Command Started =====");
        println!("[DEBUG] Files to process: {files:?}", files = options.files);
        println!(
            "[DEBUG] Custom ignores: {custom_ignores:?}",
            custom_ignores = options.custom_ignores
        );
        println!(
            "[DEBUG] Context lines: {context_lines}",
            context_lines = options.context_lines
        );
        println!("[DEBUG] Output format: {format}", format = options.format);
        println!(
            "[DEBUG] Read from clipboard: {from_clipboard}",
            from_clipboard = options.from_clipboard
        );
        println!(
            "[DEBUG] Write to clipboard: {to_clipboard}",
            to_clipboard = options.to_clipboard
        );
        println!("[DEBUG] Dry run: {dry_run}", dry_run = options.dry_run);
        println!("[DEBUG] Parse as git diff: {diff}", diff = options.diff);
        println!(
            "[DEBUG] Allow tests: {allow_tests}",
            allow_tests = options.allow_tests
        );
        println!(
            "[DEBUG] Prompt template: {prompt:?}",
            prompt = options.prompt
        );
        println!(
            "[DEBUG] Instructions: {instructions:?}",
            instructions = options.instructions
        );
    }

    // Set custom ignore patterns
    set_custom_ignores(&options.custom_ignores);

    let mut file_paths: Vec<FilePathInfo> = Vec::new();

    // Store the original input if the keep_input flag is set
    let mut original_input: Option<String> = None;

    if options.from_clipboard {
        // Read from clipboard
        println!("{}", "Reading from clipboard...".bold().blue());
        let mut clipboard = Clipboard::new()?;
        let buffer = clipboard.get_text()?;

        // Store the original input if keep_input is true
        if options.keep_input {
            original_input = Some(buffer.clone());
            if debug_mode {
                println!(
                    "[DEBUG] Stored original clipboard input: {} bytes",
                    original_input.as_ref().map_or(0, |s| s.len())
                );
            }
        }

        if debug_mode {
            println!(
                "[DEBUG] Reading from clipboard, content length: {} bytes",
                buffer.len()
            );
        }

        // Auto-detect git diff format or use explicit flag
        let is_diff_format = options.diff || is_git_diff_format(&buffer);

        if is_diff_format {
            // Parse as git diff format
            if debug_mode {
                println!("[DEBUG] Parsing clipboard content as git diff format");
            }
            file_paths = extract_file_paths_from_git_diff(&buffer, options.allow_tests);
        } else {
            // Parse as regular text
            file_paths = file_paths::extract_file_paths_from_text(&buffer, options.allow_tests);
        }

        if debug_mode {
            println!(
                "[DEBUG] Extracted {} file paths from clipboard",
                file_paths.len()
            );
            for (path, start, end, symbol, lines) in &file_paths {
                println!(
                    "[DEBUG]   - {:?} (lines: {:?}-{:?}, symbol: {:?}, specific lines: {:?})",
                    path,
                    start,
                    end,
                    symbol,
                    lines.as_ref().map(|l| l.len())
                );
            }
        }

        if file_paths.is_empty() {
            println!("{}", "No file paths found in clipboard.".yellow().bold());
            return Ok(());
        }
    } else if let Some(input_file_path) = &options.input_file {
        // Read from input file
        println!(
            "{}",
            format!("Reading from file: {input_file_path}...")
                .bold()
                .blue()
        );

        // Check if the file exists
        let input_path = std::path::Path::new(input_file_path);
        if !input_path.exists() {
            return Err(anyhow::anyhow!(
                "Input file does not exist: {}",
                input_file_path
            ));
        }

        // Read the file content
        let buffer = std::fs::read_to_string(input_path)?;

        // Store the original input if keep_input is true
        if options.keep_input {
            original_input = Some(buffer.clone());
            if debug_mode {
                println!(
                    "[DEBUG] Stored original file input: {} bytes",
                    original_input.as_ref().map_or(0, |s| s.len())
                );
            }
        }

        if debug_mode {
            println!(
                "[DEBUG] Reading from file, content length: {} bytes",
                buffer.len()
            );
        }

        // Auto-detect git diff format or use explicit flag
        let is_diff_format = options.diff || is_git_diff_format(&buffer);

        if is_diff_format {
            // Parse as git diff format
            if debug_mode {
                println!("[DEBUG] Parsing file content as git diff format");
            }
            file_paths = extract_file_paths_from_git_diff(&buffer, options.allow_tests);
        } else {
            // Parse as regular text
            file_paths = file_paths::extract_file_paths_from_text(&buffer, options.allow_tests);
        }

        if debug_mode {
            println!(
                "[DEBUG] Extracted {} file paths from input file",
                file_paths.len()
            );
            for (path, start, end, symbol, lines) in &file_paths {
                println!(
                    "[DEBUG]   - {:?} (lines: {:?}-{:?}, symbol: {:?}, specific lines: {:?})",
                    path,
                    start,
                    end,
                    symbol,
                    lines.as_ref().map(|l| l.len())
                );
            }
        }

        if file_paths.is_empty() {
            println!(
                "{}",
                format!("No file paths found in input file: {input_file_path}")
                    .yellow()
                    .bold()
            );
            return Ok(());
        }
    } else if options.files.is_empty() {
        // Check if stdin is available (not a terminal)
        let is_stdin_available = !atty::is(atty::Stream::Stdin);

        if is_stdin_available {
            // Read from stdin
            println!("{}", "Reading from stdin...".bold().blue());
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;

            // Store the original input if keep_input is true
            if options.keep_input {
                original_input = Some(buffer.clone());
                if debug_mode {
                    println!(
                        "[DEBUG] Stored original stdin input: {} bytes",
                        original_input.as_ref().map_or(0, |s| s.len())
                    );
                }
            }

            if debug_mode {
                println!(
                    "[DEBUG] Reading from stdin, content length: {} bytes",
                    buffer.len()
                );
            }

            // Auto-detect git diff format or use explicit flag
            let is_diff_format = options.diff || is_git_diff_format(&buffer);

            if is_diff_format {
                // Parse as git diff format
                if debug_mode {
                    println!("[DEBUG] Parsing stdin content as git diff format");
                }
                file_paths = extract_file_paths_from_git_diff(&buffer, options.allow_tests);
            } else {
                // Parse as regular text
                file_paths = file_paths::extract_file_paths_from_text(&buffer, options.allow_tests);
            }
        } else {
            // No arguments and no stdin, show help
            println!(
                "{}",
                "No files specified and no stdin input detected."
                    .yellow()
                    .bold()
            );
            println!("{}", "Use --help for usage information.".blue());
            return Ok(());
        }

        if debug_mode {
            println!(
                "[DEBUG] Extracted {} file paths from stdin",
                file_paths.len()
            );
            for (path, start, end, symbol, lines) in &file_paths {
                println!(
                    "[DEBUG]   - {:?} (lines: {:?}-{:?}, symbol: {:?}, specific lines: {:?})",
                    path,
                    start,
                    end,
                    symbol,
                    lines.as_ref().map(|l| l.len())
                );
            }
        }

        if file_paths.is_empty() {
            println!("{}", "No file paths found in stdin.".yellow().bold());
            return Ok(());
        }
    } else {
        // Parse command-line arguments
        if debug_mode {
            println!("[DEBUG] Parsing command-line arguments");
        }

        // Store the original input if keep_input is true
        if options.keep_input {
            original_input = Some(options.files.join(" "));
            if debug_mode {
                println!(
                    "[DEBUG] Stored original command-line input: {}",
                    original_input.as_ref().unwrap_or(&String::new())
                );
            }
        }

        for file in &options.files {
            if debug_mode {
                println!("[DEBUG] Parsing file argument: {file}");
            }

            let paths = file_paths::parse_file_with_line(file, options.allow_tests);

            if debug_mode {
                println!(
                    "[DEBUG] Parsed {} paths from argument '{}'",
                    paths.len(),
                    file
                );
                for (path, start, end, symbol, lines) in &paths {
                    println!(
                        "[DEBUG]   - {:?} (lines: {:?}-{:?}, symbol: {:?}, specific lines: {:?})",
                        path,
                        start,
                        end,
                        symbol,
                        lines.as_ref().map(|l| l.len())
                    );
                }
            }

            file_paths.extend(paths);
        }
    }

    // Only print file information for non-JSON/XML formats
    if options.format != "json" && options.format != "xml" {
        println!("{text}", text = "Files to extract:".bold().green());

        for (path, start_line, end_line, symbol, lines) in &file_paths {
            if let (Some(start), Some(end)) = (start_line, end_line) {
                println!(
                    "  {path} (lines {start}-{end})",
                    path = path.display(),
                    start = start,
                    end = end
                );
            } else if let Some(line_num) = start_line {
                println!(
                    "  {path} (line {line_num})",
                    path = path.display(),
                    line_num = line_num
                );
            } else if let Some(sym) = symbol {
                println!("  {path} (symbol: {sym})", path = path.display());
            } else if let Some(lines_set) = lines {
                println!(
                    "  {path} (specific lines: {count} lines)",
                    path = path.display(),
                    count = lines_set.len()
                );
            } else {
                println!("  {path}", path = path.display());
            }
        }

        if options.context_lines > 0 {
            println!(
                "Context lines: {context_lines}",
                context_lines = options.context_lines
            );
        }

        if options.dry_run {
            println!(
                "{text}",
                text = "Dry run (file names and lines only)".yellow()
            );
        }

        println!("Format: {format}", format = options.format);
        println!();
    }

    // Process prompt template and instructions if provided
    let system_prompt = if let Some(prompt_template) = &options.prompt {
        if debug_mode {
            println!("[DEBUG] Processing prompt template: {prompt_template:?}");
        }
        match prompt_template.get_content() {
            Ok(content) => {
                if debug_mode {
                    println!(
                        "[DEBUG] Loaded prompt template content ({} bytes)",
                        content.len()
                    );
                }
                Some(content)
            }
            Err(e) => {
                eprintln!(
                    "{text}",
                    text = format!("Error loading prompt template: {e}").red()
                );
                if debug_mode {
                    println!("[DEBUG] Error loading prompt template: {e}");
                }
                None
            }
        }
    } else {
        None
    };

    // Process files in parallel using Rayon
    use rayon::prelude::*;
    use std::sync::{Arc, Mutex};

    // Create thread-safe containers for results and errors
    let results_mutex = Arc::new(Mutex::new(Vec::<SearchResult>::new()));
    let errors_mutex = Arc::new(Mutex::new(Vec::<String>::new()));

    // Create a struct to hold all parameters for parallel processing
    struct FileProcessingParams {
        path: std::path::PathBuf,
        start_line: Option<usize>,
        end_line: Option<usize>,
        symbol: Option<String>,
        specific_lines: Option<HashSet<usize>>,
        allow_tests: bool,
        context_lines: usize,
        debug_mode: bool,
        format: String,
        #[allow(dead_code)]
        lsp: bool,

        #[allow(dead_code)]
        original_input: Option<String>,
        #[allow(dead_code)]
        system_prompt: Option<String>,
        #[allow(dead_code)]
        user_instructions: Option<String>,
    }

    // Collect all file parameters
    let file_params: Vec<FileProcessingParams> = file_paths
        .into_iter()
        .map(
            |(path, start_line, end_line, symbol, specific_lines)| FileProcessingParams {
                path,
                start_line,
                end_line,
                symbol,
                specific_lines,
                allow_tests: options.allow_tests,
                context_lines: options.context_lines,
                debug_mode,
                format: options.format.clone(),
                lsp: options.lsp,
                original_input: original_input.clone(),
                system_prompt: system_prompt.clone(),
                user_instructions: options.instructions.clone(),
            },
        )
        .collect();

    // Process files in parallel
    file_params.par_iter().for_each(|params| {
        if params.debug_mode {
            println!("\n[DEBUG] Processing file: {:?}", params.path);
            println!("[DEBUG] Start line: {:?}", params.start_line);
            println!("[DEBUG] End line: {:?}", params.end_line);
            println!("[DEBUG] Symbol: {:?}", params.symbol);
            println!(
                "[DEBUG] Specific lines: {:?}",
                params.specific_lines.as_ref().map(|l| l.len())
            );

            // Check if file exists
            if params.path.exists() {
                println!("[DEBUG] File exists: Yes");

                // Get file extension and language
                if let Some(ext) = params.path.extension().and_then(|e| e.to_str()) {
                    let language = formatter::get_language_from_extension(ext);
                    println!("[DEBUG] File extension: {ext}");
                    println!(
                        "[DEBUG] Detected language: {}",
                        if language.is_empty() {
                            "unknown"
                        } else {
                            language
                        }
                    );
                } else {
                    println!("[DEBUG] File has no extension");
                }
            } else {
                println!("[DEBUG] File exists: No");
            }
        }

        // The allow_tests check is now handled in the file path extraction functions
        // We only need to check if this is a test file for debugging purposes
        if params.debug_mode && crate::language::is_test_file(&params.path) && !params.allow_tests {
            println!("[DEBUG] Test file detected: {:?}", params.path);
        }

        match processor::process_file_for_extraction_with_lsp(
            &params.path,
            params.start_line,
            params.end_line,
            params.symbol.as_deref(),
            params.allow_tests,
            params.context_lines,
            params.specific_lines.as_ref(),
            params.lsp,
        ) {
            Ok(result) => {
                if params.debug_mode {
                    println!("[DEBUG] Successfully extracted code from {:?}", params.path);
                    println!("[DEBUG] Extracted lines: {:?}", result.lines);
                    println!("[DEBUG] Node type: {}", result.node_type);
                    println!("[DEBUG] Code length: {} bytes", result.code.len());
                    println!(
                        "[DEBUG] Estimated tokens: {}",
                        crate::search::search_tokens::count_tokens(&result.code)
                    );
                }

                // Thread-safe addition to results
                let mut results = results_mutex.lock().unwrap();
                results.push(result);
            }
            Err(e) => {
                let error_msg = format!(
                    "Error processing file {path:?}: {e}",
                    path = params.path,
                    e = e
                );
                if params.debug_mode {
                    println!("[DEBUG] Error: {error_msg}");
                }
                // Only print error messages for non-JSON/XML formats
                if params.format != "json" && params.format != "xml" {
                    eprintln!("{}", error_msg.red());
                }
                // Thread-safe addition to errors
                let mut errors = errors_mutex.lock().unwrap();
                errors.push(error_msg);
            }
        }
    });
    // Move results and errors from the mutex containers
    let mut results = Arc::try_unwrap(results_mutex)
        .expect("Failed to unwrap results mutex")
        .into_inner()
        .expect("Failed to get inner results");

    let errors = Arc::try_unwrap(errors_mutex)
        .expect("Failed to unwrap errors mutex")
        .into_inner()
        .expect("Failed to get inner errors");

    // Deduplicate results based on file path and line range
    if debug_mode {
        println!(
            "[DEBUG] Before deduplication: {len} results",
            len = results.len()
        );
    }

    // First, sort results by file path and then by line range size (largest first)
    // This ensures that parent blocks (like classes) are processed before nested blocks (like methods)
    results.sort_by(|a, b| {
        let a_file = &a.file;
        let b_file = &b.file;

        // First compare by file path
        if a_file != b_file {
            return a_file.cmp(b_file);
        }

        // Then compare by range size (largest first)
        let a_range_size = a.lines.1 - a.lines.0;
        let b_range_size = b.lines.1 - b.lines.0;
        b_range_size.cmp(&a_range_size)
    });

    if debug_mode {
        println!("[DEBUG] Sorted results by file path and range size");
        for (i, result) in results.iter().enumerate() {
            println!(
                "[DEBUG] Result {}: {} (lines {}-{}, size: {})",
                i,
                result.file,
                result.lines.0,
                result.lines.1,
                result.lines.1 - result.lines.0
            );
        }
    }

    // Now deduplicate, keeping track of which results to retain
    let mut to_retain = vec![true; results.len()];

    // Use a HashSet to track exact duplicates
    let mut seen_exact = HashSet::new();

    for i in 0..results.len() {
        if !to_retain[i] {
            continue; // Skip already marked for removal
        }

        let result_i = &results[i];
        let file_i = &result_i.file;
        let start_i = result_i.lines.0;
        let end_i = result_i.lines.1;

        // Check for exact duplicates first
        let key = format!("{file_i}:{start_i}:{end_i}");
        if !seen_exact.insert(key) {
            to_retain[i] = false;
            if debug_mode {
                println!("[DEBUG] Removing exact duplicate: {file_i} (lines {start_i}-{end_i})");
            }
            continue;
        }

        // Then check for nested duplicates
        for j in i + 1..results.len() {
            if !to_retain[j] {
                continue; // Skip already marked for removal
            }

            let result_j = &results[j];
            let file_j = &result_j.file;
            let start_j = result_j.lines.0;
            let end_j = result_j.lines.1;

            // Only compare results from the same file
            if file_i != file_j {
                continue;
            }

            // Check if result_j is contained within result_i
            if start_j >= start_i && end_j <= end_i {
                to_retain[j] = false;
                if debug_mode {
                    println!("[DEBUG] Removing nested duplicate: {file_j} (lines {start_j}-{end_j}) contained within (lines {start_i}-{end_i})");
                }
            }
        }
    }

    // Apply the retention filter
    let original_len = results.len();
    let mut new_results = Vec::with_capacity(original_len);

    for i in 0..original_len {
        if to_retain[i] {
            new_results.push(results[i].clone());
        }
    }

    results = new_results;

    if debug_mode {
        println!(
            "[DEBUG] After deduplication: {len} results",
            len = results.len()
        );
    }

    if debug_mode {
        println!("\n[DEBUG] ===== Extraction Summary =====");
        println!("[DEBUG] Total results: {}", results.len());
        println!("[DEBUG] Total errors: {}", errors.len());
        println!("[DEBUG] Output format: {}", options.format);
        println!("[DEBUG] Dry run: {}", options.dry_run);
    }

    // Format the results
    let res = {
        // Temporarily disable colors if writing to clipboard
        let colors_enabled = if options.to_clipboard {
            let was_enabled = colored::control::SHOULD_COLORIZE.should_colorize();
            colored::control::set_override(false);
            was_enabled
        } else {
            false
        };

        // Format the results
        let result = if options.dry_run {
            formatter::format_extraction_dry_run(
                &results,
                &options.format,
                original_input.as_deref(),
                system_prompt.as_deref(),
                options.instructions.as_deref(),
            )
        } else {
            formatter::format_extraction_results(
                &results,
                &options.format,
                original_input.as_deref(),
                system_prompt.as_deref(),
                options.instructions.as_deref(),
            )
        };

        // Restore color settings if they were changed
        if options.to_clipboard && colors_enabled {
            colored::control::set_override(true);
        }

        result
    };
    match res {
        Ok(formatted_output) => {
            if options.to_clipboard {
                // Write to clipboard
                let mut clipboard = Clipboard::new()?;
                clipboard.set_text(&formatted_output)?;
                println!("{}", "Results copied to clipboard.".green().bold());

                if debug_mode {
                    println!(
                        "[DEBUG] Wrote {} bytes to clipboard",
                        formatted_output.len()
                    );
                }
            } else {
                // Print to stdout
                println!("{formatted_output}");
            }
        }
        Err(e) => {
            // Only print error messages for non-JSON/XML formats
            if options.format != "json" && options.format != "xml" {
                eprintln!("{}", format!("Error formatting results: {e}").red());
            }
            if debug_mode {
                println!("[DEBUG] Error formatting results: {e}");
            }
        }
    }

    // Print summary of errors if any (only for non-JSON/XML formats)
    if !errors.is_empty() && options.format != "json" && options.format != "xml" {
        println!();
        println!(
            "{} {} {}",
            "Encountered".red().bold(),
            errors.len(),
            if errors.len() == 1 { "error" } else { "errors" }
        );
    }

    if debug_mode {
        println!("[DEBUG] ===== Extract Command Completed =====");
    }

    Ok(())
}
