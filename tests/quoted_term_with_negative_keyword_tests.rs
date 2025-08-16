use std::fs;
use std::path::Path;
use tempfile::TempDir;

use probe_code::search::{perform_probe, SearchOptions};

/// Create test files with different content for testing queries
fn create_test_files(temp_dir: &Path) {
    // File with "keywordAlpha" only
    let file1_path = temp_dir.join("file1.rs");
    let file1_content = r#"
// This file contains keywordAlpha only
fn test_function() {
    // This is keywordAlpha
    let x = 1;
    
    println!("Result: {}", x);
}
"#;

    // File with "keywordAlpha" and "keywordGamma"
    let file2_path = temp_dir.join("file2.rs");
    let file2_content = r#"
// This file contains keywordAlpha and keywordGamma
fn another_function() {
    // This is keywordAlpha
    let x = 1;
    
    // This is keywordGamma
    let z = 3;
    
    println!("Result: {}", x + z);
}
"#;

    // Write files to disk
    fs::write(file1_path, file1_content).unwrap();
    fs::write(file2_path, file2_content).unwrap();
}

/// Test a query with a quoted term and a negative keyword: "keywordAlpha" -keywordGamma
#[test]
fn test_quoted_term_with_negative_keyword() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with different content
    create_test_files(temp_path);

    // Create search query with a quoted term and a negative keyword
    let queries = vec!["\"keywordAlpha\" -keywordGamma".to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Print the test files for debugging
    println!("Test files created in: {temp_path:?}");
    for entry in std::fs::read_dir(temp_path).unwrap() {
        let entry = entry.unwrap();
        println!("  {:?}", entry.path());

        // Print file content for debugging
        let content = std::fs::read_to_string(entry.path()).unwrap();
        println!(
            "  Content of {:?}:\n{}",
            entry.path().file_name().unwrap(),
            content
        );
    }

    // Create SearchOptions
    let options = SearchOptions {
        path: temp_path,
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: false,
        language: None,
        reranker: "hybrid",
        frequency_search: false,
        max_results: None,
        max_bytes: None,
        max_tokens: None,
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

    // Print the query for debugging
    println!("Executing search with query: {queries:?}");
    println!(
        "Path: {:?}, frequency_search: {}",
        options.path, options.frequency_search
    );

    // Run the search
    let search_results = perform_probe(&options).unwrap();

    // Check that we got results
    assert!(
        !search_results.results.is_empty(),
        "Search should return results"
    );

    // We should find files with "keywordAlpha" but not "keywordGamma"
    let file_names: Vec<&str> = search_results
        .results
        .iter()
        .map(|r| r.file.as_str())
        .collect();

    // Debug output
    println!(
        "Quoted term with negative keyword query results: {} items",
        search_results.results.len()
    );
    for result in &search_results.results {
        println!("  File: {}", result.file);
    }

    // Check that we found file1 (has "keywordAlpha" but not "keywordGamma")
    assert!(
        file_names.iter().any(|&name| name.contains("file1")),
        "Should find file1 which contains \"keywordAlpha\" but not \"keywordGamma\""
    );

    // Check that we don't find file2 (it has "keywordGamma")
    assert!(
        !file_names.iter().any(|&name| name.contains("file2")),
        "Should not find file2 which contains \"keywordGamma\""
    );
}

/// Test a query with a negative quoted term: -"keywordGamma"
#[test]
fn test_negative_quoted_term() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with different content
    create_test_files(temp_path);

    // Create search query with a negative quoted term
    let queries = vec!["\"keywordalpha\" -\"keywordgamma\"".to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Create SearchOptions
    let options = SearchOptions {
        path: temp_path,
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: false,
        language: None,
        reranker: "hybrid",
        frequency_search: false,
        max_results: None,
        max_bytes: None,
        max_tokens: None,
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

    // Print the query for debugging
    println!("Executing search with query: {queries:?}");

    // Run the search
    let search_results = perform_probe(&options).unwrap();

    // Check that we got results
    assert!(
        !search_results.results.is_empty(),
        "Search should return results"
    );

    // We should find files with "keywordAlpha" but not "keywordGamma"
    let file_names: Vec<&str> = search_results
        .results
        .iter()
        .map(|r| r.file.as_str())
        .collect();

    // Debug output
    println!(
        "Negative quoted term query results: {} items",
        search_results.results.len()
    );
    for result in &search_results.results {
        println!("  File: {}", result.file);
    }

    // Check that we found file1 (has "keywordAlpha" but not "keywordGamma")
    assert!(
        file_names.iter().any(|&name| name.contains("file1")),
        "Should find file1 which contains \"keywordAlpha\" but not \"keywordGamma\""
    );

    // Check that we don't find file2 (it has "keywordGamma")
    assert!(
        !file_names.iter().any(|&name| name.contains("file2")),
        "Should not find file2 which contains \"keywordGamma\""
    );
}
