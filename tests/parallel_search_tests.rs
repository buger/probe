use probe_code::search::search_runner::{perform_probe, search_with_structured_patterns};
use probe_code::search::SearchOptions;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::tempdir;

// Helper function to create test files
fn create_test_files(dir: &Path, count: usize, lines_per_file: usize) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for i in 0..count {
        let file_path = dir.join(format!("test_file_{i}.rs"));

        // Create file content with multiple functions and searchable terms
        let mut content = format!("// Test file {i}\n\n");

        for j in 0..lines_per_file / 10 {
            content.push_str(&format!(
                "/// Function documentation for function_{}_{}
fn function_{}_{}() {{
    // Initialize variables
    let search_term_alpha = {};
    let search_term_beta = {};
    
    // Process data
    println!(\"Processing data with search_term_gamma\");
    
    // Return result
    search_term_alpha + search_term_beta
}}

",
                i,
                j,
                i,
                j,
                j * 2,
                j * 3
            ));
        }

        fs::write(&file_path, content).unwrap();
        paths.push(file_path);
    }

    paths
}

#[test]
fn test_parallel_file_search() {
    // Create a temporary directory for test files
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create multiple test files (50 files with 100 lines each)
    let file_count = 50;
    let lines_per_file = 100;
    let _file_paths = create_test_files(base_path, file_count, lines_per_file);

    // Create search options
    let queries = vec![
        "search_term_alpha".to_string(),
        "search_term_beta".to_string(),
    ];
    let custom_ignores = Vec::new();

    let options = SearchOptions {
        path: base_path,
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: false,
        language: None,
        reranker: "hybrid",
        frequency_search: false,
        max_results: Some(100),
        max_bytes: Some(1_000_000),
        max_tokens: Some(100_000),
        allow_tests: true,
        no_merge: false,
        merge_threshold: Some(5),
        dry_run: false,
        session: None,
        timeout: 30,
        question: None,
        exact: false,
        no_gitignore: false,
        lsp: false,
    };

    // Measure search time
    let start_time = Instant::now();
    let result = perform_probe(&options);
    let duration = start_time.elapsed();

    // Verify search results
    assert!(result.is_ok(), "Search should succeed");
    let search_results = result.unwrap();

    // Ensure we found matches
    assert!(
        !search_results.results.is_empty(),
        "Search should find matches"
    );

    // Print performance information
    println!("Parallel search completed in {duration:?} for {file_count} files");
    println!("Found {} results", search_results.results.len());
}

#[test]
fn test_structured_patterns_search() {
    // Create a temporary directory for test files
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create test files
    let file_count = 20;
    let lines_per_file = 50;
    let _file_paths = create_test_files(base_path, file_count, lines_per_file);

    // Create a simple query plan and patterns for testing
    let query_plan = probe_code::search::query::create_query_plan(
        "search_term_alpha OR search_term_beta",
        false,
    )
    .unwrap();

    // Create patterns from the query plan
    let patterns = probe_code::search::query::create_structured_patterns(&query_plan);

    // Create empty custom ignores
    let custom_ignores: Vec<String> = Vec::new();

    // Measure search time
    let start_time = Instant::now();
    let result = search_with_structured_patterns(
        base_path,
        &query_plan,
        &patterns,
        &custom_ignores,
        true,
        None,
        false,
    );
    let duration = start_time.elapsed();

    // Verify search results
    assert!(result.is_ok(), "Structured pattern search should succeed");
    let file_term_maps = result.unwrap();

    // Ensure we found matches
    assert!(!file_term_maps.is_empty(), "Search should find matches");

    // Print performance information
    println!("Parallel structured pattern search completed in {duration:?} for {file_count} files");
    println!("Found matches in {} files", file_term_maps.len());
}

#[test]
fn test_ast_parallel_processing() {
    // Create a temporary directory for test files
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create a single large file with many top-level nodes to test AST parallelization
    let file_path = base_path.join("large_file.rs");

    // Create file content with many top-level functions
    let mut content = "// Large test file with many top-level nodes\n\n".to_string();

    for i in 0..100 {
        content.push_str(&format!(
            "/// Function documentation for function_{}
fn function_{}() {{
    // Function body with search terms
    let search_term_alpha = {};
    let search_term_beta = {};
    println!(\"Processing with search_term_gamma\");
}}

",
            i,
            i,
            i * 2,
            i * 3
        ));
    }

    fs::write(&file_path, content).unwrap();

    // Create search options targeting the large file
    let queries = vec![
        "search_term_alpha".to_string(),
        "search_term_beta".to_string(),
    ];
    let custom_ignores = Vec::new();

    let options = SearchOptions {
        path: base_path,
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: false,
        language: None,
        reranker: "hybrid",
        frequency_search: false,
        max_results: Some(100),
        max_bytes: Some(1_000_000),
        max_tokens: Some(100_000),
        allow_tests: true,
        no_merge: false,
        merge_threshold: Some(5),
        dry_run: false,
        session: None,
        timeout: 30,
        question: None,
        exact: false,
        no_gitignore: false,
        lsp: false,
    };

    // Measure search time
    let start_time = Instant::now();
    let result = perform_probe(&options);
    let duration = start_time.elapsed();

    // Verify search results
    assert!(
        result.is_ok(),
        "AST parallel processing search should succeed"
    );
    let search_results = result.unwrap();

    // Ensure we found matches
    assert!(
        !search_results.results.is_empty(),
        "Search should find matches"
    );

    // Print performance information
    println!("AST parallel processing completed in {duration:?}");
    println!("Found {} results", search_results.results.len());
}

#[test]
fn test_block_parallel_processing() {
    // Create a temporary directory for test files
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create a file with many code blocks to test block parallelization
    let file_path = base_path.join("multi_block.rs");

    // Create file content with a large number of functions and nested blocks
    let mut content = "// Test file with many code blocks\n\n".to_string();

    for i in 0..50 {
        content.push_str(&format!(
            "/// Function with multiple blocks
fn function_with_blocks_{}() {{
    // Block 1
    {{
        let search_term_alpha = {};
        println!(\"Block 1\");
    }}
    
    // Block 2
    {{
        let search_term_beta = {};
        println!(\"Block 2\");
    }}
    
    // Block 3
    {{
        let search_term_gamma = {};
        println!(\"Block 3\");
    }}
    
    // Block 4
    if true {{
        let search_term_delta = {};
        println!(\"Block 4\");
    }}
}}

",
            i,
            i,
            i * 2,
            i * 3,
            i * 4
        ));
    }

    fs::write(&file_path, content).unwrap();

    // Create search options targeting the multi-block file
    let queries = vec![
        "search_term_alpha".to_string(),
        "search_term_beta".to_string(),
    ];
    let custom_ignores = Vec::new();

    let options = SearchOptions {
        path: base_path,
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: false,
        language: None,
        reranker: "hybrid",
        frequency_search: false,
        max_results: Some(100),
        max_bytes: Some(1_000_000),
        max_tokens: Some(100_000),
        allow_tests: true,
        no_merge: false,
        merge_threshold: Some(5),
        dry_run: false,
        session: None,
        timeout: 30,
        question: None,
        exact: false,
        no_gitignore: false,
        lsp: false,
    };

    // Measure search time
    let start_time = Instant::now();
    let result = perform_probe(&options);
    let duration = start_time.elapsed();

    // Verify search results
    assert!(
        result.is_ok(),
        "Block parallel processing search should succeed"
    );
    let search_results = result.unwrap();

    // Ensure we found matches
    assert!(
        !search_results.results.is_empty(),
        "Search should find matches"
    );

    // Print performance information
    println!("Block parallel processing completed in {duration:?}");
    println!("Found {} results", search_results.results.len());
}
