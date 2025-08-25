use lru::LruCache;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

use probe_code::search::elastic_query::Expr;
use probe_code::search::query::QueryPlan;
use probe_code::search::{perform_probe, SearchOptions};

/// Create test files with different content for testing queries
fn create_test_files(temp_dir: &Path) {
    // File with "keywordAlpha" and "keywordBeta"
    let file1_path = temp_dir.join("file1.rs");
    let file1_content = r#"
// This file contains keywordAlpha and keywordBeta
fn test_function() {
    // This is keywordAlpha
    let x = 1;
    
    // This is keywordBeta
    let y = 2;
    
    println!("Result: {}", x + y);
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

    // File with "keywordBeta" and "keywordGamma"
    let file3_path = temp_dir.join("file3.rs");
    let file3_content = r#"
// This file contains keywordBeta and keywordGamma
fn third_function() {
    // This is keywordBeta
    let y = 2;
    
    // This is keywordGamma
    let z = 3;
    
    println!("Result: {}", y + z);
}
"#;

    // File with all keywords
    let file4_path = temp_dir.join("file4.rs");
    let file4_content = r#"
// This file contains keywordAlpha, keywordBeta, and keywordGamma
fn all_keywords_function() {
    // This is keywordAlpha
    let x = 1;
    
    // This is keywordBeta
    let y = 2;
    
    // This is keywordGamma
    let z = 3;
    
    println!("Result: {}", x + y + z);
}
"#;

    // Write files to disk
    fs::write(file1_path, file1_content).unwrap();
    fs::write(file2_path, file2_content).unwrap();
    fs::write(file3_path, file3_content).unwrap();
    fs::write(file4_path, file4_content).unwrap();
}

/// Test a query with a required term: key OR word OR keyword
#[test]
fn test_required_term_query() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with different content
    create_test_files(temp_path);

    // Create search query with explicit OR syntax that matches the actual content
    // Since we've removed any_term parameter and changed the default to AND,
    // we need to use explicit OR syntax to maintain the original behavior
    let queries = vec!["keywordAlpha OR keywordBeta OR keywordGamma".to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Print the query for debugging
    println!("Testing query: {queries:?}");

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

    // Print the temp_path for debugging
    println!("Temp path: {temp_path:?}");

    // List files in the temp directory
    println!("Files in temp directory:");
    for entry in std::fs::read_dir(temp_path).unwrap() {
        let entry = entry.unwrap();
        println!("  {:?}", entry.path());
    }

    // Run the search
    let search_results = perform_probe(&options).unwrap();

    // Debug output
    println!("Search results: {} items", search_results.results.len());
    for result in &search_results.results {
        println!("  File: {}", result.file);
    }

    // Check that we got results
    assert!(
        !search_results.results.is_empty(),
        "Search should return results"
    );

    // We should only find files with keywordAlpha
    let file_names: Vec<&str> = search_results
        .results
        .iter()
        .map(|r| r.file.as_str())
        .collect();

    // Debug output
    println!("Found {} results", search_results.results.len());
    for result in &search_results.results {
        println!("File: {}", result.file);
    }

    // Check that we found files with keywordAlpha OR keywordBeta OR keywordGamma
    assert!(
        file_names.iter().any(|&name| name.contains("file1")),
        "Should find file1 which contains keywordAlpha OR keywordBeta"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file2")),
        "Should find file2 which contains keywordAlpha OR keywordGamma"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file3")),
        "Should find file3 which contains keywordBeta OR keywordGamma"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file4")),
        "Should find file4 which contains keywordAlpha, keywordBeta, and keywordGamma"
    );
}

/// Test a query with an excluded term: -keywordGamma
#[test]
fn test_excluded_term_query() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with different content
    create_test_files(temp_path);

    // Create search query with an excluded term
    let queries = vec!["(key OR word OR keyword) -keywordGamma".to_string()];
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

    // We should find files with (key OR word OR keyword) -keywordGamma
    let file_names: Vec<&str> = search_results
        .results
        .iter()
        .map(|r| r.file.as_str())
        .collect();

    // Debug output
    println!(
        "Excluded term query results: {} items",
        search_results.results.len()
    );
    for result in &search_results.results {
        println!("  File: {}", result.file);
    }

    // Check that we found file1 (has key OR word OR keyword but not keywordGamma)
    assert!(
        file_names.iter().any(|&name| name.contains("file1")),
        "Should find file1 which contains key OR word OR keyword but not keywordGamma"
    );

    // Check that we don't find file2, file3, and file4 (they have keywordGamma)
    assert!(
        !file_names.iter().any(|&name| name.contains("file2")),
        "Should not find file2 which contains keywordGamma"
    );

    assert!(
        !file_names.iter().any(|&name| name.contains("file3")),
        "Should not find file3 which contains keywordGamma"
    );

    assert!(
        !file_names.iter().any(|&name| name.contains("file4")),
        "Should not find file4 which contains keywordGamma"
    );
}

/// Test a query with OR: keywordAlpha OR keywordBeta
#[test]
fn test_or_query() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with different content
    create_test_files(temp_path);

    // Create search query with explicit OR syntax
    // Make sure to use uppercase OR to ensure it's recognized as an operator
    let queries = vec!["keywordAlpha OR keywordBeta".to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Create SearchOptions
    let options = SearchOptions {
        path: temp_path,
        queries: &queries,
        files_only: true, // Use files_only to ensure we find all matching files
        custom_ignores: &custom_ignores,
        exclude_filenames: false,
        language: None,
        reranker: "hybrid",
        frequency_search: true, // Enable frequency search to improve matching
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

    // We should find files with keywordAlpha OR keywordBeta
    let file_names: Vec<&str> = search_results
        .results
        .iter()
        .map(|r| r.file.as_str())
        .collect();

    // Debug output to see what files were found
    println!("Found files with 'keywordAlpha OR keywordBeta':");
    for name in &file_names {
        println!("  {name}");
    }

    // Check that we found files with keywordAlpha OR keywordBeta
    assert!(
        file_names.iter().any(|&name| name.contains("file1")),
        "Should find file1 which contains keywordAlpha and keywordBeta"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file2")),
        "Should find file2 which contains keywordAlpha"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file3")),
        "Should find file3 which contains keywordBeta"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file4")),
        "Should find file4 which contains keywordAlpha and keywordBeta"
    );
}

/// Test a complex query with OR syntax
#[test]
fn test_complex_query_or() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with different content
    create_test_files(temp_path);

    // Test with explicit OR syntax
    // "keywordAlpha OR keywordBeta" means files with keywordAlpha OR keywordBeta
    let queries = vec!["keywordAlpha OR keywordBeta".to_string()];
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
        frequency_search: true, // Enable frequency search to improve matching
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

    let file_names: Vec<&str> = search_results
        .results
        .iter()
        .map(|r| r.file.as_str())
        .collect();

    // Debug output to see what files were found
    println!("Found files with 'keywordAlpha OR keywordBeta':");
    for name in &file_names {
        println!("  {name}");
    }

    // With explicit OR syntax, should find all files with keywordAlpha OR keywordBeta
    assert!(
        file_names.iter().any(|&name| name.contains("file1")),
        "Should find file1 which has keywordAlpha and keywordBeta"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file2")),
        "Should find file2 which has keywordAlpha"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file3")),
        "Should find file3 which has keywordBeta"
    );
    assert!(
        file_names.iter().any(|&name| name.contains("file4")),
        "Should find file4 which has keywordAlpha and keywordBeta"
    );
}

/// Test a complex query with explicit OR syntax
#[test]
fn test_complex_query_exclusion() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with different content
    create_test_files(temp_path);

    // Test with exclusion
    let queries = vec!["\"keywordAlpha\" -keywordGamma".to_string()];
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
    println!(
        "Path: {:?}, frequency_search: {}",
        options.path, options.frequency_search
    );

    // Run the search
    let search_results = perform_probe(&options).unwrap();

    // Check that we got results
    assert!(
        !search_results.results.is_empty(),
        "Search should return results for query: {queries:?}"
    );

    let file_names: Vec<&str> = search_results
        .results
        .iter()
        .map(|r| r.file.as_str())
        .collect();

    // Debug output to see what files were found
    println!("Found files with 'keywordAlpha -keywordGamma':");
    for name in &file_names {
        println!("  {name}");
    }

    // Should find file1 (has keywordAlpha and no keywordGamma)
    assert!(
        file_names.iter().any(|&name| name.contains("file1")),
        "Should find file1 which has keywordAlpha but no keywordGamma"
    );

    // Should NOT find file2 (has keywordAlpha but also has keywordGamma)
    assert!(
        !file_names.iter().any(|&name| name.contains("file2")),
        "Should not find file2 which has keywordGamma"
    );

    // Should NOT find file4 (has keywordAlpha but also has keywordGamma)
    assert!(
        !file_names.iter().any(|&name| name.contains("file4")),
        "Should not find file4 which has keywordGamma"
    );
}

/// Test underscore handling in queries
#[test]
fn test_underscore_handling_integration() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create a test file with the tokenized terms
    let file_path = temp_path.join("underscore_test.rs");
    let file_content = r#"
// This file contains key word score
fn test_function() {
    // This has key, word, and score
    let x = 1;
    
    // This also has key word score
    let y = 2;
    
    println!("Result: {}", x + y);
}
"#;
    fs::write(file_path, file_content).unwrap();

    // Test query with underscore using explicit OR syntax
    // Since we've removed any_term parameter and changed the default to AND,
    // we need to use explicit OR syntax to maintain the original behavior
    let queries = vec!["key OR word OR score".to_string()];
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

    // Run the search
    let search_results = perform_probe(&options).unwrap();

    // Check that we got results
    assert!(
        !search_results.results.is_empty(),
        "Search should return results"
    );

    let file_names: Vec<&str> = search_results
        .results
        .iter()
        .map(|r| r.file.as_str())
        .collect();

    // Debug output to see what files were found
    println!("Found files with 'key OR word OR score':");
    for name in &file_names {
        println!("  {name}");
    }

    // Should find the file with at least one of the terms: key, word, or score
    assert!(
        file_names
            .iter()
            .any(|&name| name.contains("underscore_test")),
        "Should find underscore_test.rs which contains at least one of the terms: key, word, or score"
    );

    // Verify that the code in the results contains at least one of the tokenized terms
    for result in &search_results.results {
        println!("Result code: {}", result.code);
        // The search should find files containing at least one of the tokenized terms
        assert!(
            result.code.contains("key")
                || result.code.contains("word")
                || result.code.contains("score"),
            "Result code should contain at least one of the terms: 'key', 'word', or 'score'"
        );
    }
}

/// Test the direct usage of filter_code_block_with_ast
#[test]
fn test_filter_code_block_with_ast() {
    // This test directly tests the filter_code_block_with_ast function
    // by creating a mock QueryPlan and term matches

    // Create a simple AST: keywordAlpha AND -keywordBeta
    let ast = Expr::And(
        Box::new(Expr::Term {
            keywords: vec!["keywordAlpha".to_string()],
            field: None,
            required: false,
            excluded: false,
            exact: false,
        }),
        Box::new(Expr::Term {
            keywords: vec!["keywordBeta".to_string()],
            field: None,
            required: false,
            excluded: true,
            exact: false,
        }),
    );

    // Create a term indices map
    let mut term_indices = HashMap::new();
    term_indices.insert("keywordAlpha".to_string(), 0);
    term_indices.insert("keywordBeta".to_string(), 1);

    // Create a QueryPlan
    let has_required_anywhere = ast.has_required_term();
    let has_only_excluded_terms = ast.is_only_excluded_terms();
    let required_terms_indices = HashSet::new();
    let evaluation_cache = Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(1000).unwrap())));

    let plan = QueryPlan {
        ast,
        term_indices,
        excluded_terms: {
            let mut set = HashSet::new();
            set.insert("keywordBeta".to_string());
            set
        },
        exact: false,
        is_simple_query: false,
        required_terms: HashSet::new(),
        has_required_anywhere,
        required_terms_indices,
        has_only_excluded_terms,
        evaluation_cache,
    };

    // Create term matches for a block
    let mut term_matches = HashMap::new();

    // Block with only keywordAlpha
    let mut lines1 = HashSet::new();
    lines1.insert(1);
    lines1.insert(2);
    term_matches.insert(0, lines1);

    // Test the function with a block that should match
    let block_lines = (1, 5);
    let debug_mode = false;

    // Import the function from probe crate
    use probe_code::search::file_processing::filter_code_block_with_ast;

    // The block should match because it has keywordAlpha but not keywordBeta
    assert!(
        filter_code_block_with_ast(block_lines, &term_matches, &plan, debug_mode),
        "Block should match because it has keywordAlpha but not keywordBeta"
    );

    // Now add keywordBeta to the block
    let mut lines2 = HashSet::new();
    lines2.insert(3);
    lines2.insert(4);
    term_matches.insert(1, lines2);

    // The block should not match because it now has keywordBeta (which is excluded)
    assert!(
        !filter_code_block_with_ast(block_lines, &term_matches, &plan, debug_mode),
        "Block should not match because it has keywordBeta which is excluded"
    );
}

/// Test the direct usage of filter_tokenized_block
#[test]
fn test_filter_tokenized_block() {
    // This test directly tests the filter_tokenized_block function
    // by creating a mock QueryPlan and tokenized content

    // Create a simple AST: keywordAlpha AND -keywordBeta
    let ast = Expr::And(
        Box::new(Expr::Term {
            keywords: vec!["keywordAlpha".to_string()],
            field: None,
            required: false,
            excluded: false,
            exact: false,
        }),
        Box::new(Expr::Term {
            keywords: vec!["keywordBeta".to_string()],
            field: None,
            required: false,
            excluded: true,
            exact: false,
        }),
    );

    // Create a term indices map
    let mut term_indices = HashMap::new();
    term_indices.insert("keywordAlpha".to_string(), 0);
    term_indices.insert("keywordBeta".to_string(), 1);

    // Create a QueryPlan
    let has_required_anywhere = ast.has_required_term();
    let has_only_excluded_terms = ast.is_only_excluded_terms();
    let required_terms_indices = HashSet::new();
    let evaluation_cache = Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(1000).unwrap())));

    let plan = QueryPlan {
        ast,
        term_indices: term_indices.clone(),
        excluded_terms: {
            let mut set = HashSet::new();
            set.insert("keywordBeta".to_string());
            set
        },
        exact: false,
        is_simple_query: false,
        required_terms: HashSet::new(),
        has_required_anywhere,
        required_terms_indices,
        has_only_excluded_terms,
        evaluation_cache,
    };

    // Import the function from probe crate
    use probe_code::search::file_processing::filter_tokenized_block;

    // Test case 1: Tokenized content with only keywordAlpha
    let tokenized_content = vec!["keywordAlpha".to_string()];
    let debug_mode = false;

    // The block should match because it has keywordAlpha but not keywordBeta
    assert!(
        filter_tokenized_block(&tokenized_content, &term_indices, &plan, debug_mode),
        "Block should match because it has keywordAlpha but not keywordBeta"
    );

    // Test case 2: Tokenized content with both keywordAlpha and keywordBeta
    let tokenized_content = vec!["keywordAlpha".to_string(), "keywordBeta".to_string()];

    // The block should not match because it has keywordBeta (which is excluded)
    assert!(
        !filter_tokenized_block(&tokenized_content, &term_indices, &plan, debug_mode),
        "Block should not match because it has keywordBeta which is excluded"
    );

    // Test case 3: Tokenized content with neither keyword
    let tokenized_content = vec!["other".to_string()];

    // The block should not match because it doesn't have keywordAlpha
    assert!(
        !filter_tokenized_block(&tokenized_content, &term_indices, &plan, debug_mode),
        "Block should not match because it doesn't have keywordAlpha"
    );

    // Test case 4: Empty tokenized content
    let tokenized_content: Vec<String> = vec![];

    // The block should not match because it doesn't have keywordAlpha
    assert!(
        !filter_tokenized_block(&tokenized_content, &term_indices, &plan, debug_mode),
        "Empty block should not match"
    );

    // Test case 5: Test with OR expression
    // Create a simple AST: keywordAlpha OR keywordGamma
    let ast_or = Expr::Or(
        Box::new(Expr::Term {
            keywords: vec!["keywordAlpha".to_string()],
            field: None,
            required: false,
            excluded: false,
            exact: false,
        }),
        Box::new(Expr::Term {
            keywords: vec!["keywordGamma".to_string()],
            field: None,
            required: false,
            excluded: false,
            exact: false,
        }),
    );

    // Create a term indices map
    let mut term_indices_or = HashMap::new();
    term_indices_or.insert("keywordAlpha".to_string(), 0);
    term_indices_or.insert("keywordGamma".to_string(), 2);

    // Create a QueryPlan
    let has_required_anywhere = ast_or.has_required_term();
    let has_only_excluded_terms = ast_or.is_only_excluded_terms();
    let required_terms_indices = HashSet::new();
    let evaluation_cache = Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(1000).unwrap())));

    let plan_or = QueryPlan {
        ast: ast_or,
        term_indices: term_indices_or.clone(),
        excluded_terms: HashSet::new(),
        exact: false,
        is_simple_query: false,
        required_terms: HashSet::new(),
        has_required_anywhere,
        required_terms_indices,
        has_only_excluded_terms,
        evaluation_cache,
    };

    // Test with only keywordGamma
    let tokenized_content = vec!["keywordGamma".to_string()];

    // The block should match because it has keywordGamma (part of OR expression)
    assert!(
        filter_tokenized_block(&tokenized_content, &term_indices_or, &plan_or, debug_mode),
        "Block should match because it has keywordGamma (part of OR expression)"
    );

    // Test with both keywordAlpha and keywordGamma
    let tokenized_content = vec!["keywordAlpha".to_string(), "keywordGamma".to_string()];

    // The block should match because it has both keywords in OR expression
    assert!(
        filter_tokenized_block(&tokenized_content, &term_indices_or, &plan_or, debug_mode),
        "Block should match because it has both keywords in OR expression"
    );
}
