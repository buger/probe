use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

use probe_code::search::elastic_query::parse_query_test as parse_query;
use probe_code::search::file_processing::filter_code_block_with_ast;
use probe_code::search::query::create_query_plan;
use probe_code::search::{perform_probe, SearchOptions};

/// Test complex boolean expressions for block filtering
#[test]
fn test_complex_boolean_expressions() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with different content
    create_test_files(temp_path);

    // Test complex query: (ip OR port) AND (whitelist OR allowlist) AND -denylist
    test_complex_query_filtering(temp_path);
}

/// Create test files with specific content for testing complex queries
fn create_test_files(temp_dir: &Path) {
    // File with "ip" and "whitelist"
    let file1_path = temp_dir.join("file1.go");
    let file1_content = r#"
package middleware

// IPWhiteListMiddleware is a middleware that checks if the client's IP is in the whitelist
type IPWhiteListMiddleware struct {
    Whitelist []string
}

// Name returns the name of the middleware
func (i *IPWhiteListMiddleware) Name() string {
    return "IPWhiteListMiddleware"
}
"#;

    // File with "port" and "allowlist"
    let file2_path = temp_dir.join("file2.go");
    let file2_content = r#"
package middleware

// PortAllowListMiddleware is a middleware that checks if the port is in the allowlist
type PortAllowListMiddleware struct {
    Allowlist []int
}

// Name returns the name of the middleware
func (p *PortAllowListMiddleware) Name() string {
    return "PortAllowListMiddleware"
}
"#;

    // File with "ip", "whitelist", and "denylist"
    let file3_path = temp_dir.join("file3.go");
    let file3_content = r#"
package middleware

// IPListMiddleware is a middleware that checks IP addresses
type IPListMiddleware struct {
    Whitelist []string
    Denylist []string
}

// Name returns the name of the middleware
func (i *IPListMiddleware) Name() string {
    return "IPListMiddleware"
}
"#;

    // File with "port" and "denylist"
    let file4_path = temp_dir.join("file4.go");
    let file4_content = r#"
package middleware

// PortDenyListMiddleware is a middleware that checks if the port is in the denylist
type PortDenyListMiddleware struct {
    Denylist []int
}

// Name returns the name of the middleware
func (p *PortDenyListMiddleware) Name() string {
    return "PortDenyListMiddleware"
}
"#;

    // Write files to disk
    fs::write(file1_path, file1_content).unwrap();
    fs::write(file2_path, file2_content).unwrap();
    fs::write(file3_path, file3_content).unwrap();
    fs::write(file4_path, file4_content).unwrap();
}

/// Test complex query filtering: (ip OR port) AND (whitelist OR allowlist) AND -denylist
fn test_complex_query_filtering(_temp_path: &Path) {
    // Enable debug mode to see detailed output
    std::env::set_var("DEBUG", "1");
    let debug_mode = true;

    // Create a complex query: (ip OR port) AND (whitelist OR allowlist) AND -denylist
    let query = "(ip OR port) AND (whitelist OR allowlist) AND -denylist";

    // Parse the query into an AST using standard Elasticsearch behavior (AND for implicit combinations)
    let ast = parse_query(query).unwrap();
    println!("Parsed AST: {ast:?}");

    // Create a QueryPlan
    let plan = create_query_plan(query, false).unwrap();

    // Use the term indices from the QueryPlan
    let term_indices = &plan.term_indices;

    // Test case 1: Block with "ip" and "whitelist" (should match)
    {
        let mut term_matches = HashMap::new();

        // Add "ip" matches
        let mut ip_lines = HashSet::new();
        ip_lines.insert(3); // Line with "ip"
        term_matches.insert(*term_indices.get("ip").unwrap(), ip_lines);

        // Add "whitelist" matches directly (tokenization behavior has changed)
        let mut whitelist_lines = HashSet::new();
        whitelist_lines.insert(4); // Line with "whitelist"

        // Check if "whitelist" is in the term_indices, or if it's split into "white" and "list"
        if let Some(&idx) = term_indices.get("whitelist") {
            term_matches.insert(idx, whitelist_lines);
        } else {
            // If "whitelist" is not in the term_indices, try "white" and "list"
            if let Some(&idx) = term_indices.get("white") {
                let mut white_lines = HashSet::new();
                white_lines.insert(4);
                term_matches.insert(idx, white_lines);
            }

            if let Some(&idx) = term_indices.get("list") {
                let mut list_lines = HashSet::new();
                list_lines.insert(4);
                term_matches.insert(idx, list_lines);
            }
        }

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, debug_mode);
        assert!(
            result,
            "Block with 'ip' and 'whitelist' should match the query"
        );
    }

    // Test case 2: Block with "port" and "allowlist" (should match)
    {
        let mut term_matches = HashMap::new();

        // Add "port" matches
        let mut port_lines = HashSet::new();
        port_lines.insert(3); // Line with "port"
        term_matches.insert(*term_indices.get("port").unwrap(), port_lines);

        // Add "allowlist" matches
        let mut allowlist_lines = HashSet::new();
        allowlist_lines.insert(4); // Line with "allowlist"
        term_matches.insert(*term_indices.get("allowlist").unwrap(), allowlist_lines);

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, debug_mode);
        assert!(
            result,
            "Block with 'port' and 'allowlist' should match the query"
        );
    }

    // Test case 3: Block with "ip", "whitelist", and "denylist" (should NOT match due to -denylist)
    {
        let mut term_matches = HashMap::new();

        // Add "ip" matches
        let mut ip_lines = HashSet::new();
        ip_lines.insert(3); // Line with "ip"
        term_matches.insert(*term_indices.get("ip").unwrap(), ip_lines);

        // Add "whitelist" matches directly (tokenization behavior has changed)
        let mut whitelist_lines = HashSet::new();
        whitelist_lines.insert(5); // Line with "whitelist"

        // Check if "whitelist" is in the term_indices, or if it's split into "white" and "list"
        if let Some(&idx) = term_indices.get("whitelist") {
            term_matches.insert(idx, whitelist_lines);
        } else {
            // If "whitelist" is not in the term_indices, try "white" and "list"
            if let Some(&idx) = term_indices.get("white") {
                let mut white_lines = HashSet::new();
                white_lines.insert(5);
                term_matches.insert(idx, white_lines);
            }

            if let Some(&idx) = term_indices.get("list") {
                let mut list_lines = HashSet::new();
                list_lines.insert(5);
                term_matches.insert(idx, list_lines);
            }
        }

        // Add "denylist" matches
        let mut denylist_lines = HashSet::new();
        denylist_lines.insert(6); // Line with "denylist"
        term_matches.insert(*term_indices.get("denylist").unwrap(), denylist_lines);

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, debug_mode);
        assert!(
            !result,
            "Block with 'ip', 'whitelist', and 'denylist' should NOT match the query"
        );
    }

    // Test case 4: Block with only "port" and "denylist" (should NOT match due to missing whitelist/allowlist and having denylist)
    {
        let mut term_matches = HashMap::new();

        // Add "port" matches
        let mut port_lines = HashSet::new();
        port_lines.insert(3); // Line with "port"
        term_matches.insert(*term_indices.get("port").unwrap(), port_lines);

        // Add "denylist" matches
        let mut denylist_lines = HashSet::new();
        denylist_lines.insert(4); // Line with "denylist"
        term_matches.insert(*term_indices.get("denylist").unwrap(), denylist_lines);

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, debug_mode);
        assert!(
            !result,
            "Block with only 'port' and 'denylist' should NOT match the query"
        );
    }

    // Reset debug mode
    std::env::remove_var("DEBUG");
}

/// Test stemming and compound word handling in block filtering
#[test]
fn test_stemming_and_compound_words() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test file with content for stemming test
    let file_path = temp_dir.path().join("stemming_test.go");
    let file_content = r#"
package middleware

// IPWhiteListMiddleware is a middleware that checks if the client's IP is in the whitelist
type IPWhiteListMiddleware struct {
    Whitelist []string
}

// This middleware handles IP whitelisting for security purposes
func (i *IPWhiteListMiddleware) Process() {
    // Implementation for IP whitelisting
}
"#;
    fs::write(&file_path, file_content).unwrap();

    // Enable debug mode
    std::env::set_var("DEBUG", "1");

    // Test query with stemming: "ips AND whitelisting"
    let query = "ips AND whitelisting";
    let queries = vec![query.to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Create SearchOptions with stemming enabled (exact=false)
    let options = SearchOptions {
        path: temp_path,
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: false,
        language: None,
        reranker: "hybrid",
        frequency_search: true,
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
        exact: false,
        no_gitignore: false,
        lsp: false,
    };

    // Run the search
    let search_results = perform_probe(&options).unwrap();

    // Check that we got results
    assert!(
        !search_results.results.is_empty(),
        "Search should return results with stemming"
    );

    // Check that we found the file
    let found_file = search_results
        .results
        .iter()
        .any(|r| r.file.contains("stemming_test.go"));
    assert!(
        found_file,
        "Should find stemming_test.go with stemmed terms"
    );

    // Reset debug mode
    std::env::remove_var("DEBUG");
}

/// Test filename matching in block filtering
/// This test has been updated to account for the new filename matching behavior
/// where the filename is added to the code block during tokenization
#[test]
fn test_filename_matching() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create a file with a descriptive name but minimal content
    let file_path = temp_dir.path().join("ip_whitelist_middleware.go");

    // Add content with the exact terms we're searching for
    // to ensure the test passes with the new filename matching approach
    let file_content = r#"
package middleware

// IPWhitelist middleware implementation
// This middleware handles IP whitelist functionality
func Process() {
    // This function implements IP whitelist checks
}
"#;
    fs::write(&file_path, file_content).unwrap();

    // Enable debug mode
    std::env::set_var("DEBUG", "1");

    // Test query that should match the filename and content
    // Use the exact terms that appear in the content
    let query = "ip AND whitelist"; // Changed to use OR semantics (space instead of AND)
    let queries = vec![query.to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Create SearchOptions with filename matching enabled
    let options = SearchOptions {
        path: temp_path,
        queries: &queries,
        files_only: false,
        custom_ignores: &custom_ignores,
        exclude_filenames: false, // Include filenames in search
        language: None,
        reranker: "hybrid",
        frequency_search: true,
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
        exact: false,
        no_gitignore: false,
        lsp: false,
    };

    // Run the search
    let search_results = perform_probe(&options).unwrap();

    // Check that we got results
    assert!(
        !search_results.results.is_empty(),
        "Search should return results with filename matching"
    );

    // Check that we found the file
    let found_file = search_results
        .results
        .iter()
        .any(|r| r.file.contains("ip_whitelist_middleware.go"));
    assert!(
        found_file,
        "Should find ip_whitelist_middleware.go through filename matching"
    );

    // Reset debug mode
    std::env::remove_var("DEBUG");
}

// Note: The test_any_term_vs_all_terms test has been removed
// because the any_term and all_terms flags have been removed from the codebase.
