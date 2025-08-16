use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

use probe_code::search::elastic_query::parse_query_test as parse_query;
use probe_code::search::file_processing::filter_code_block_with_ast;
use probe_code::search::query::create_query_plan;
use probe_code::search::{perform_probe, SearchOptions};

/// Test stemming and compound word handling in block filtering with complex queries
#[test]
fn test_stemming_in_complex_queries() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create test files with content for stemming test
    create_test_files(temp_path);

    // Test various stemming scenarios
    test_stemming_with_and_query(temp_path);
    test_stemming_with_or_query(temp_path);
    test_stemming_with_complex_query(temp_path);
    test_compound_word_splitting(temp_path);
    test_negative_compound_word_in_existing_tests(temp_path);
}

/// Create test files with content for stemming tests
fn create_test_files(temp_dir: &Path) {
    // File with stemming variations: "ip", "ips", "whitelist", "whitelisting"
    let file1_path = temp_dir.join("stemming_variations.go");
    let file1_content = r#"
package middleware

// IPWhiteListMiddleware handles IP whitelisting
// It processes IPs against the whitelist
type IPWhiteListMiddleware struct {
    Whitelist []string
}

// Process implements whitelisting for IPs
func (i *IPWhiteListMiddleware) Process() {
    // Implementation for IP whitelisting
}
"#;

    // File with compound words: "whitelist", "firewall", "network"
    let file2_path = temp_dir.join("compound_words.go");
    let file2_content = r#"
package security

// FirewallConfig configures the network firewall
// It includes white list settings for the network
type FirewallConfig struct {
    NetworkWhitelist []string
    FirewallRules []Rule
}

// Rule represents a firewall rule
type Rule struct {
    Name string
    Action string
}
"#;

    // Write files to disk
    fs::write(file1_path, file1_content).unwrap();
    fs::write(file2_path, file2_content).unwrap();
}

/// Test stemming with AND query: "ips AND whitelisting"
fn test_stemming_with_and_query(temp_path: &Path) {
    // Enable debug mode
    std::env::set_var("DEBUG", "1");
    println!("\n=== Testing stemming with AND query: ips AND whitelisting ===");

    // Create the query
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

    // Check that we found the file with stemming variations
    let found_file = search_results
        .results
        .iter()
        .any(|r| r.file.contains("stemming_variations.go"));
    assert!(
        found_file,
        "Should find stemming_variations.go with stemmed terms"
    );

    // Now test the filter_code_block_with_ast function directly
    // Create a QueryPlan
    let plan = create_query_plan(query, false).unwrap();

    // Get the term indices from the QueryPlan
    let term_indices = &plan.term_indices;

    // Create term matches
    let mut term_matches = HashMap::new();

    // Add "ip" matches
    let mut ip_lines = HashSet::new();
    ip_lines.insert(4); // Line with "IPs"
    term_matches.insert(*term_indices.get("ip").unwrap(), ip_lines);

    // Add "whitelist" matches
    let mut whitelist_lines = HashSet::new();
    whitelist_lines.insert(5); // Line with "whitelist"
    term_matches.insert(*term_indices.get("whitelist").unwrap(), whitelist_lines);

    // Block lines
    let block_lines = (1, 10);

    // Test filtering
    let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, true);
    assert!(result, "Block with stemmed terms 'ip' and 'whitelist' should match the query 'ips AND whitelisting'");
    println!("✓ Block with stemmed terms matches the AND query");
}

/// Test stemming with OR query: "ips OR whitelisting"
fn test_stemming_with_or_query(temp_path: &Path) {
    println!("\n=== Testing stemming with OR query: ips OR whitelisting ===");

    // Create the query
    let query = "ips OR whitelisting";
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
        // Use any term mode
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

    // Check that we found both files
    let found_file1 = search_results
        .results
        .iter()
        .any(|r| r.file.contains("stemming_variations.go"));
    assert!(
        found_file1,
        "Should find stemming_variations.go with stemmed terms"
    );

    // Now test the filter_code_block_with_ast function directly
    // Create a QueryPlan
    let plan = create_query_plan(query, false).unwrap();

    // Get the term indices from the QueryPlan
    let term_indices = &plan.term_indices;

    // Test case 1: Block with only "ip" (should match OR query)
    {
        let mut term_matches = HashMap::new();

        // Add only "ip" matches
        let mut ip_lines = HashSet::new();
        ip_lines.insert(4);
        term_matches.insert(*term_indices.get("ip").unwrap(), ip_lines);

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, true);
        assert!(
            result,
            "Block with only stemmed term 'ip' should match the OR query"
        );
        println!("✓ Block with only stemmed term 'ip' matches the OR query");
    }

    // Test case 2: Block with only "whitelist" (should match OR query)
    {
        let mut term_matches = HashMap::new();

        // Add only "whitelist" matches
        let mut whitelist_lines = HashSet::new();
        whitelist_lines.insert(5);
        term_matches.insert(*term_indices.get("whitelist").unwrap(), whitelist_lines);

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, true);
        assert!(
            result,
            "Block with only stemmed term 'whitelist' should match the OR query"
        );
        println!("✓ Block with only stemmed term 'whitelist' matches the OR query");
    }
}

/// Test stemming with complex query: "(ips OR ports) AND (whitelisting OR security) AND -blocking"
fn test_stemming_with_complex_query(_temp_path: &Path) {
    println!("\n=== Testing stemming with complex query: (ips OR ports) AND (whitelisting OR security) AND -blocking ===");

    // Create the query
    let query = "(ips OR ports) AND (whitelisting OR security) AND -blocking";

    // Parse the query into an AST
    // Using standard Elasticsearch behavior (AND for implicit combinations)
    let ast = parse_query(query).unwrap();
    println!("Parsed AST: {ast:?}");

    // Create a QueryPlan
    let plan = create_query_plan(query, false).unwrap();

    // Get the term indices from the QueryPlan
    let term_indices = &plan.term_indices;

    // Test case 1: Block with "ip" and "whitelist" (should match)
    {
        let mut term_matches = HashMap::new();

        // Add "ip" matches
        let mut ip_lines = HashSet::new();
        ip_lines.insert(3);
        term_matches.insert(*term_indices.get("ip").unwrap(), ip_lines);

        // Add "whitelist" matches
        let mut whitelist_lines = HashSet::new();
        whitelist_lines.insert(4);
        term_matches.insert(*term_indices.get("whitelist").unwrap(), whitelist_lines);

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, true);
        assert!(
            result,
            "Block with stemmed terms 'ip' and 'whitelist' should match the complex query"
        );
        println!("✓ Block with stemmed terms 'ip' and 'whitelist' matches the complex query");
    }

    // Test case 2: Block with "port" and "security" (should match)
    {
        let mut term_matches = HashMap::new();

        // Add "port" matches
        let mut port_lines = HashSet::new();
        port_lines.insert(3);
        term_matches.insert(*term_indices.get("port").unwrap(), port_lines);

        // Add "secur" matches (stemmed form of "security")
        let mut security_lines = HashSet::new();
        security_lines.insert(4);
        term_matches.insert(*term_indices.get("secur").unwrap(), security_lines);

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, true);
        assert!(
            result,
            "Block with stemmed terms 'port' and 'security' should match the complex query"
        );
        println!("✓ Block with stemmed terms 'port' and 'security' matches the complex query");
    }

    // Test case 3: Block with "ip", "whitelist", and "block" (should NOT match due to -blocking)
    {
        let mut term_matches = HashMap::new();

        // Add "ip" matches
        let mut ip_lines = HashSet::new();
        ip_lines.insert(3);
        term_matches.insert(*term_indices.get("ip").unwrap(), ip_lines);

        // Add "whitelist" matches
        let mut whitelist_lines = HashSet::new();
        whitelist_lines.insert(4);
        term_matches.insert(*term_indices.get("whitelist").unwrap(), whitelist_lines);

        // Add "blocking" matches (the exact term used in the query)
        let mut block_lines = HashSet::new();
        block_lines.insert(5);
        term_matches.insert(*term_indices.get("blocking").unwrap(), block_lines);

        // Block lines
        let block_lines = (1, 10);

        // Test filtering
        let result = filter_code_block_with_ast(block_lines, &term_matches, &plan, true);
        assert!(
            !result,
            "Block with 'ip', 'whitelist', and 'block' should NOT match due to -blocking"
        );
        println!("✓ Block with 'ip', 'whitelist', and 'block' doesn't match due to -blocking");
    }
}

/// Test compound word splitting: "networkfirewall"
fn test_compound_word_splitting(temp_path: &Path) {
    println!("\n=== Testing compound word splitting: networkfirewall ===");

    // Create the query
    let query = "networkfirewall";
    let queries = vec![query.to_string()];
    let custom_ignores: Vec<String> = vec![];

    // Create SearchOptions with stemming and compound word splitting enabled
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
        "Search should return results with compound word splitting"
    );

    // Check that we found the file with "network" and "firewall"
    let found_file = search_results
        .results
        .iter()
        .any(|r| r.file.contains("compound_words.go"));
    assert!(
        found_file,
        "Should find compound_words.go with split compound word"
    );

    // Now test with a more complex query: "network AND firewall"
    let complex_query = "network AND firewall";
    let complex_queries = vec![complex_query.to_string()];

    let complex_options = SearchOptions {
        path: temp_path,
        queries: &complex_queries,
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
        // Use all terms mode
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
    let complex_results = perform_probe(&complex_options).unwrap();

    // Check that we got results
    assert!(
        !complex_results.results.is_empty(),
        "Search should return results with AND query on compound words"
    );

    // Check that we found the file with both "network" and "firewall"
    let found_complex_file = complex_results
        .results
        .iter()
        .any(|r| r.file.contains("compound_words.go"));
    assert!(
        found_complex_file,
        "Should find compound_words.go with AND query on compound words"
    );

    // Reset debug mode
    std::env::remove_var("DEBUG");
}

/// Test negative compound word in existing tests: "-networkfirewall"
fn test_negative_compound_word_in_existing_tests(temp_path: &Path) {
    println!("\n=== Testing negative compound word in existing tests: -networkfirewall ===");

    // Enable debug mode
    std::env::set_var("DEBUG", "1");

    // Create the query
    let query = "-networkfirewall";
    let queries = vec![query.to_string()];
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

    // Check that we didn't find the file with "network" and "firewall"
    let found_file = search_results
        .results
        .iter()
        .any(|r| r.file.contains("compound_words.go"));
    assert!(
        !found_file,
        "Should NOT find compound_words.go with negative compound word"
    );
    println!("✓ Negative compound word properly excludes files with compound parts");

    // Test excluded terms extraction directly using QueryPlan
    let plan = create_query_plan(query, false).unwrap();

    // Check that the original term is in the excluded_terms set
    assert!(
        plan.excluded_terms.contains("networkfirewall"),
        "Original term 'networkfirewall' should be in excluded_terms"
    );

    // We don't split excluded terms, so compound parts should not be in the excluded terms set
    assert!(
        !plan.excluded_terms.contains("network"),
        "Compound part 'network' should not be in excluded_terms"
    );
    assert!(
        !plan.excluded_terms.contains("firewall"),
        "Compound part 'firewall' should not be in excluded_terms"
    );
    println!("✓ Excluded terms extraction properly handles compound words");

    // Reset debug mode
    std::env::remove_var("DEBUG");
}
