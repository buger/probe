use anyhow::Result;
use probe_code::language::parser::parse_file_for_code_blocks;
use std::collections::HashSet;
use std::fs;
use tempfile::NamedTempFile;

/// Integration test to verify that comments in test functions are properly merged
/// with their parent function context, even when allow_tests is false (the default).
///
/// This test recreates the specific issue reported where single-line comments
/// in test functions were not being extended to include their parent context.
#[test]
fn test_comment_context_with_test_functions() -> Result<()> {
    let rust_code = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn test_tokenize() {
        let tokens = tokenize("stemming word");
        assert!(tokens.contains(&"stem".to_string())); // stemmed "stemming"
        assert!(tokens.contains(&"token".to_string())); // test token
    }
    
    #[test]
    fn test_parsing() {
        let result = parse("test");
        assert!(result.is_ok()); // should parse correctly
    }
}

pub fn regular_function() {
    let x = 42; // regular comment
}
"#;

    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    fs::write(&temp_file, rust_code).expect("Failed to write test content");

    // Test with allow_tests = false (the default behavior that was causing the issue)
    let lines_to_search: HashSet<usize> = (0..25).collect();
    let blocks_disallow_tests = parse_file_for_code_blocks(
        rust_code,
        "rs",
        &lines_to_search,
        false, // allow_tests = false - this was causing comments to not get context
        None,
    )?;

    // Comments should still be merged with their parent functions even when allow_tests=false
    let comment_blocks: Vec<_> = blocks_disallow_tests
        .iter()
        .filter(|block| {
            // Find blocks that likely contain the "stemmed" comment
            block.start_row <= 7 && block.end_row >= 7 // Line with "stemmed" comment
        })
        .collect();

    assert!(
        !comment_blocks.is_empty(),
        "Should find blocks containing the comment line with allow_tests=false"
    );

    // The comment should be part of a function block, not a standalone comment
    let function_block = comment_blocks
        .iter()
        .find(|block| block.node_type.contains("function") || block.node_type == "function_item");

    assert!(
        function_block.is_some(),
        "Comment should be merged with function context even when allow_tests=false, found blocks: {:?}",
        comment_blocks
            .iter()
            .map(|b| &b.node_type)
            .collect::<Vec<_>>()
    );

    if let Some(func_block) = function_block {
        // The function block should span multiple lines (the entire test function)
        assert!(
            func_block.end_row > func_block.start_row + 1,
            "Function block should span multiple lines, got {}-{}",
            func_block.start_row + 1,
            func_block.end_row + 1
        );
    }

    Ok(())
}

/// Test that regular function comments (non-test) work correctly
#[test]
fn test_regular_function_comment_context() -> Result<()> {
    let rust_code = r#"
pub fn tokenize_and_stem(keyword: &str) -> Vec<String> {
    let stemmer = get_stemmer();
    let tokens = split_camel_case(keyword);
    
    if tokens.len() > 1 {
        tokens
            .into_iter()
            .map(|part| stemmer.stem(&part).to_string()) // stemmed parts
            .collect()
    } else {
        vec![stemmer.stem(keyword).to_string()] // stemmed keyword
    }
}
"#;

    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    fs::write(&temp_file, rust_code).expect("Failed to write test content");

    let lines_to_search: HashSet<usize> = (0..15).collect();
    let blocks = parse_file_for_code_blocks(
        rust_code,
        "rs",
        &lines_to_search,
        false, // allow_tests = false
        None,
    )?;

    // Find blocks containing the "stemmed" comments
    let comment_blocks: Vec<_> = blocks
        .iter()
        .filter(|block| {
            // Should include the comment lines
            (block.start_row <= 8 && block.end_row >= 8) ||  // "stemmed parts"
            (block.start_row <= 10 && block.end_row >= 10) // "stemmed keyword"
        })
        .collect();

    assert!(
        !comment_blocks.is_empty(),
        "Should find blocks containing the comments"
    );

    // Comments should be merged with the function
    let function_blocks: Vec<_> = comment_blocks
        .iter()
        .filter(|block| block.node_type.contains("function") || block.node_type == "function_item")
        .collect();

    assert!(
        !function_blocks.is_empty(),
        "Comments should be merged with function context, found: {:?}",
        comment_blocks
            .iter()
            .map(|b| &b.node_type)
            .collect::<Vec<_>>()
    );

    Ok(())
}

/// Test that the --allow-tests flag behavior is preserved:
/// - Comments should always get their parent context regardless of allow_tests
/// - Non-comment test functions should be filtered based on allow_tests
#[test]
fn test_allow_tests_flag_behavior() -> Result<()> {
    let rust_code = r#"
pub fn regular_function() {
    let x = 42; // regular comment in non-test
}

#[test]
fn test_example() {
    assert_eq!(2 + 2, 4); // comment in test function
}
"#;

    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    fs::write(&temp_file, rust_code).expect("Failed to write test content");

    let lines_to_search: HashSet<usize> = (0..10).collect();

    // Test with allow_tests = false (default)
    let blocks_no_tests = parse_file_for_code_blocks(
        rust_code,
        "rs",
        &lines_to_search,
        false, // allow_tests = false
        None,
    )?;

    // Test with allow_tests = true
    let blocks_with_tests = parse_file_for_code_blocks(
        rust_code,
        "rs",
        &lines_to_search,
        true, // allow_tests = true
        None,
    )?;

    // Both should have the regular function
    let regular_func_no_tests = blocks_no_tests
        .iter()
        .find(|b| b.node_type.contains("function") && b.start_row <= 2 && b.end_row >= 2);
    let regular_func_with_tests = blocks_with_tests
        .iter()
        .find(|b| b.node_type.contains("function") && b.start_row <= 2 && b.end_row >= 2);

    assert!(
        regular_func_no_tests.is_some(),
        "Regular function should be present without allow_tests"
    );
    assert!(
        regular_func_with_tests.is_some(),
        "Regular function should be present with allow_tests"
    );

    // Comments in test functions should get their context regardless of allow_tests
    // (This was the bug we fixed - comments were losing context when allow_tests=false)
    let test_comment_no_tests = blocks_no_tests
        .iter()
        .find(|b| b.start_row <= 6 && b.end_row >= 6); // Line with test comment
    let test_comment_with_tests = blocks_with_tests
        .iter()
        .find(|b| b.start_row <= 6 && b.end_row >= 6); // Line with test comment

    assert!(
        test_comment_no_tests.is_some(),
        "Test comment should have context block even when allow_tests=false"
    );
    assert!(
        test_comment_with_tests.is_some(),
        "Test comment should have context block when allow_tests=true"
    );

    // The key fix: comments should get merged with parent context regardless of allow_tests
    if let (Some(block_no_tests), Some(block_with_tests)) =
        (test_comment_no_tests, test_comment_with_tests)
    {
        // Both should span multiple lines (merged with function context), not just the comment line
        assert!(
            block_no_tests.end_row > block_no_tests.start_row,
            "Comment should be merged with function context when allow_tests=false, got lines {}-{}",
            block_no_tests.start_row + 1,
            block_no_tests.end_row + 1
        );
        assert!(
            block_with_tests.end_row > block_with_tests.start_row,
            "Comment should be merged with function context when allow_tests=true, got lines {}-{}",
            block_with_tests.start_row + 1,
            block_with_tests.end_row + 1
        );
    }

    Ok(())
}
