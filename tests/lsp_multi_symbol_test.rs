//! Integration test for multi-symbol LSP extraction in merged blocks
//!
//! This test verifies that when multiple functions are merged into a single block,
//! the LSP enrichment correctly extracts all symbols and retrieves their call hierarchy.

use anyhow::Result;
use probe_code::models::SearchResult;
use probe_code::search::lsp_enrichment::enrich_results_with_lsp;
use serde_json::json;
use std::env;
use std::path::PathBuf;

/// Create a mock search result with multiple functions in the code block
fn create_merged_block_result() -> SearchResult {
    SearchResult {
        file: test_file_path(),
        lines: (3, 19), // Lines containing Add, Multiply, and Subtract functions
        code: r#"// Add performs addition of two integers
// This function should show incoming calls from Calculate
func Add(x, y int) int {
    return x + y
}

// Multiply performs multiplication of two integers  
// This function should show incoming calls from Calculate
func Multiply(x, y int) int {
    return x * y
}

// Subtract performs subtraction of two integers
// This function should show incoming calls from Calculate
func Subtract(x, y int) int {
    return x - y
}"#
        .to_string(),
        node_type: "function_declaration".to_string(),
        score: Some(1.0),
        tfidf_score: None,
        bm25_score: Some(1.0),
        new_score: None,
        rank: Some(1),
        matched_keywords: Some(vec![
            "Add".to_string(),
            "Multiply".to_string(),
            "Subtract".to_string(),
        ]),
        parent_file_id: None,
        file_match_rank: Some(1),
        file_unique_terms: Some(3),
        file_total_matches: Some(3),
        block_unique_terms: Some(3),
        block_total_matches: Some(3),
        combined_score_rank: Some(1),
        bm25_rank: Some(1),
        tfidf_rank: None,
        hybrid2_rank: None,
        lsp_info: None, // This should be populated by enrichment
        block_id: None,
        matched_by_filename: Some(false),
        tokenized_content: None,
    }
}

fn test_file_path() -> String {
    // Use the actual test fixture path
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/go/project1/utils.go");
    path.to_string_lossy().to_string()
}

#[test]
#[ignore] // Ignore by default as it requires LSP daemon to be running
fn test_multi_symbol_lsp_extraction() -> Result<()> {
    // Create a merged block containing multiple functions
    let mut results = vec![create_merged_block_result()];

    // Enable debug mode to see what's happening
    let debug_mode = env::var("DEBUG").unwrap_or_default() == "1";

    // Enrich with LSP information
    enrich_results_with_lsp(&mut results, debug_mode)?;

    // Verify that LSP info was added
    assert!(
        results[0].lsp_info.is_some(),
        "LSP info should be populated"
    );

    let lsp_info = results[0].lsp_info.as_ref().unwrap();

    // Check if it's a merged structure
    if let Some(merged) = lsp_info.get("merged") {
        assert_eq!(merged, &json!(true), "Should be marked as merged");

        // Check symbols array
        let symbols = lsp_info
            .get("symbols")
            .expect("Merged LSP info should have symbols array")
            .as_array()
            .expect("Symbols should be an array");

        // We expect 3 symbols: Add, Multiply, Subtract
        assert_eq!(symbols.len(), 3, "Should have extracted 3 symbols");

        // Verify each symbol has the expected fields
        let symbol_names: Vec<String> = symbols
            .iter()
            .filter_map(|s| s.get("symbol").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .collect();

        assert!(
            symbol_names.contains(&"Add".to_string()),
            "Should contain Add function"
        );
        assert!(
            symbol_names.contains(&"Multiply".to_string()),
            "Should contain Multiply function"
        );
        assert!(
            symbol_names.contains(&"Subtract".to_string()),
            "Should contain Subtract function"
        );

        // Check that each symbol has call hierarchy
        for symbol in symbols {
            assert!(
                symbol.get("call_hierarchy").is_some(),
                "Each symbol should have call hierarchy"
            );
            assert!(
                symbol.get("node_type").is_some(),
                "Each symbol should have node_type"
            );
            assert!(
                symbol.get("range").is_some(),
                "Each symbol should have range"
            );
        }

        println!("✓ Multi-symbol LSP extraction test passed!");
        println!("  Found {} symbols in merged block", symbols.len());
        for name in &symbol_names {
            println!("    - {name}");
        }
    } else {
        // If not merged, it should at least have one symbol
        assert!(
            lsp_info.get("symbol").is_some(),
            "Should have at least one symbol"
        );
        println!(
            "⚠ LSP info was not merged, but has single symbol: {}",
            lsp_info.get("symbol").unwrap()
        );
    }

    Ok(())
}

#[test]
fn test_tree_sitter_symbol_detection() -> Result<()> {
    use probe_code::language::factory::get_language_impl;
    use probe_code::language::parser_pool::{get_pooled_parser, return_pooled_parser};

    let code = r#"func Add(x, y int) int {
    return x + y
}

func Multiply(x, y int) int {
    return x * y
}"#;

    // Get Go language implementation
    let lang = get_language_impl("go").expect("Go language should be available");

    // Get parser from pool
    let mut parser = get_pooled_parser("go")?;
    parser.set_language(&lang.get_tree_sitter_language())?;

    // Parse the code
    let tree = parser.parse(code, None).expect("Should parse Go code");
    let root = tree.root_node();

    // Find all function nodes
    let mut function_count = 0;

    fn count_functions(node: tree_sitter::Node, content: &[u8], count: &mut usize) {
        if node.kind() == "function_declaration" {
            *count += 1;

            // Find the function name
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    if let Ok(name) = child.utf8_text(content) {
                        println!("Found function: {name}");
                    }
                    break;
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            count_functions(child, content, count);
        }
    }

    count_functions(root, code.as_bytes(), &mut function_count);

    // Return parser to pool
    return_pooled_parser("go", parser);

    assert_eq!(function_count, 2, "Should find 2 functions in the code");

    Ok(())
}
