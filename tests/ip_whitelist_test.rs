use probe_code::search::{perform_probe, SearchOptions};
use std::path::PathBuf;

#[test]
fn test_ip_whitelist_stemming() {
    // Path to our test file
    let file_path = PathBuf::from("tests/mocks/test_ip_whitelist.go");

    // Create search query
    let queries = vec!["ip whitelisting".to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Create SearchOptions
    let options = SearchOptions {
        path: file_path.parent().unwrap().parent().unwrap(), // Use the tests directory
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: true,
        language: None,
        reranker: "hybrid",
        frequency_search: true, // Use frequency search to get detailed term stats
        exact: false,
        max_results: None,
        max_bytes: None,
        max_tokens: None,
        allow_tests: true,
        no_merge: true,
        merge_threshold: None,
        dry_run: false,
        session: None,
        timeout: 30,
        question: None,
        no_gitignore: false,
        lsp: false,
    };

    // Enable debug mode to see the actual terms
    std::env::set_var("DEBUG", "1");

    // Search for the terms
    let search_results = perform_probe(&options).expect("Failed to perform search");

    // Reset debug mode
    std::env::remove_var("DEBUG");

    // Should find matches
    assert!(!search_results.results.is_empty(), "Should find matches");

    // Find the result for our test file
    let test_file_result = search_results
        .results
        .iter()
        .find(|r| r.file.contains("test_ip_whitelist.go"));

    assert!(
        test_file_result.is_some(),
        "Should find the test_ip_whitelist.go file"
    );

    // Check the block_unique_terms and block_total_matches
    if let Some(result) = test_file_result {
        // Check block_unique_terms
        if let Some(block_unique_terms) = result.block_unique_terms {
            // With compound word splitting, "whitelist" becomes "white" and "list"
            // So we expect 3 unique terms: "ip", "white", and "list"
            assert!(
                block_unique_terms >= 1,
                "Expected at least 1 unique term, got {block_unique_terms}"
            );
        } else {
            panic!("block_unique_terms should be set");
        }

        // Check block_total_matches
        if let Some(block_total_matches) = result.block_total_matches {
            // With compound word splitting, we expect at least 1 match
            assert!(
                block_total_matches >= 1,
                "Expected at least 1 total match, got {block_total_matches}"
            );
        } else {
            panic!("block_total_matches should be set");
        }

        // Print the result for debugging
        println!("Result for test_ip_whitelist.go:");
        println!("  block_unique_terms: {:?}", result.block_unique_terms);
        println!("  block_total_matches: {:?}", result.block_total_matches);
        println!("  code: {}", result.code);
    }
}

#[test]
fn test_negative_terms_exclude_files() {
    // Path to our test file
    let file_path = PathBuf::from("tests/mocks/test_ip_whitelist.go");

    // Create search query with a negative term
    let queries = vec!["(+ip) -whitelist".to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Create SearchOptions
    let options = SearchOptions {
        path: file_path.parent().unwrap().parent().unwrap(), // Use the tests directory
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: true,
        language: None,
        reranker: "hybrid",
        frequency_search: true,
        exact: false,
        max_results: None,
        max_bytes: None,
        max_tokens: None,
        allow_tests: true,
        no_merge: true,
        merge_threshold: None,
        dry_run: false,
        session: None,
        timeout: 30,
        question: None,
        no_gitignore: false,
        lsp: false,
    };

    // Enable debug mode to see the actual terms
    std::env::set_var("DEBUG", "1");

    // Search for the terms
    let search_results = perform_probe(&options).expect("Failed to perform search");

    // Reset debug mode
    std::env::remove_var("DEBUG");

    // Find the result for our test file
    let test_file_result = search_results
        .results
        .iter()
        .find(|r| r.file.contains("test_ip_whitelist.go"));

    // The test file should NOT be in the results because it contains "whitelist"
    assert!(
        test_file_result.is_none(),
        "test_ip_whitelist.go should be excluded from results because it contains 'whitelist'"
    );

    println!("✓ Files containing negative terms are properly excluded from results");
}
