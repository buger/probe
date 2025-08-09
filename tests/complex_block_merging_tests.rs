use std::fs;
use tempfile::TempDir;

use probe_code::models::SearchResult;
use probe_code::search::block_merging::merge_ranked_blocks;
use probe_code::search::{perform_probe, SearchOptions};

/// Test merging of blocks with different node types
#[test]
fn test_merge_different_node_types() {
    // Create test blocks with different node types
    let block1 = SearchResult {
        file: "mixed_types.rs".to_string(),
        lines: (1, 5),
        node_type: "function".to_string(),
        code:
            "fn test_function() {\n    let x = 1;\n    let y = 2;\n    println!(\"{}\", x + y);\n}"
                .to_string(),
        matched_by_filename: None,
        rank: Some(1),
        score: Some(0.9),
        tfidf_score: Some(0.8),
        bm25_score: Some(0.85),
        tfidf_rank: Some(1),
        bm25_rank: Some(1),
        new_score: Some(0.87),
        hybrid2_rank: Some(1),
        combined_score_rank: Some(1),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(3),
        parent_file_id: None,
        block_id: None,
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };
    let block2 = SearchResult {
    file: "mixed_types.rs".to_string(),
    lines: (6, 10),
    node_type: "comment".to_string(),
    code: "// This is a comment block\n// It explains the function above\n// And provides context\n// For the next function\n// Below".to_string(),
    matched_by_filename: None,
    rank: Some(2),
    score: Some(0.8),
    tfidf_score: Some(0.7),
    bm25_score: Some(0.75),
    tfidf_rank: Some(2),
    bm25_rank: Some(2),
    new_score: Some(0.77),
    hybrid2_rank: Some(2),
    combined_score_rank: Some(2),
    file_unique_terms: Some(3),
    file_total_matches: Some(5),
    file_match_rank: Some(1),
    block_unique_terms: Some(2),
    block_total_matches: Some(2),
    parent_file_id: None,
    block_id: None,
    matched_keywords: None,
    tokenized_content: None,
        lsp_info: None,
};

    let block3 = SearchResult {
        file: "mixed_types.rs".to_string(),
        lines: (11, 15),
        node_type: "function".to_string(),
        code: "fn another_function() {\n    let z = 3;\n    let result = z * 2;\n    println!(\"{}\", result);\n}".to_string(),
        matched_by_filename: None,
        rank: Some(3),
        score: Some(0.7),
        tfidf_score: Some(0.6),
        bm25_score: Some(0.65),
        tfidf_rank: Some(3),
        bm25_rank: Some(3),
        new_score: Some(0.67),
        hybrid2_rank: Some(3),
        combined_score_rank: Some(3),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(2),
        parent_file_id: None,
        block_id: None,
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Create a vector with all blocks
    let blocks = vec![block1, block2, block3];

    // Call the merge_ranked_blocks function
    let merged_blocks = merge_ranked_blocks(blocks, Some(5));

    // Assert that all blocks are merged into one
    assert_eq!(
        merged_blocks.len(),
        1,
        "All adjacent blocks should be merged into one"
    );

    // Check that the merged block has the correct line range
    let merged_block = &merged_blocks[0];
    assert_eq!(
        merged_block.lines,
        (1, 15),
        "Merged block should span from line 1 to 15"
    );

    // Check that the node_type is from the highest-ranked block
    assert_eq!(
        merged_block.node_type, "function",
        "Merged block should have the node_type of the highest-ranked block"
    );

    // Check that the score is the maximum of all blocks
    assert_eq!(
        merged_block.score,
        Some(0.9),
        "Merged score should be the maximum of all blocks"
    );
}

/// Test merging of blocks with gaps between them
#[test]
fn test_merge_with_gaps() {
    // Create test blocks with gaps between them
    let block1 = SearchResult {
        file: "gaps.rs".to_string(),
        lines: (1, 5),
        node_type: "function".to_string(),
        code:
            "fn first_function() {\n    let x = 1;\n    let y = 2;\n    println!(\"{}\", x + y);\n}"
                .to_string(),
        matched_by_filename: None,
        rank: Some(1),
        score: Some(0.9),
        tfidf_score: Some(0.8),
        bm25_score: Some(0.85),
        tfidf_rank: Some(1),
        bm25_rank: Some(1),
        new_score: Some(0.87),
        hybrid2_rank: Some(1),
        combined_score_rank: Some(1),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(3),
        parent_file_id: None,
        block_id: None,
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Gap of 3 lines between block1 and block2
    let block2 = SearchResult {
        file: "gaps.rs".to_string(),
        lines: (9, 13),
        node_type: "function".to_string(),
        code: "fn second_function() {\n    let z = 3;\n    let result = z * 2;\n    println!(\"{}\", result);\n}".to_string(),
        matched_by_filename: None,
        rank: Some(2),
        score: Some(0.8),
        tfidf_score: Some(0.7),
        bm25_score: Some(0.75),
        tfidf_rank: Some(2),
        bm25_rank: Some(2),
        new_score: Some(0.77),
        hybrid2_rank: Some(2),
        combined_score_rank: Some(2),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(2),
        parent_file_id: None,
        block_id: None,
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Gap of 2 lines between block2 and block3
    let block3 = SearchResult {
        file: "gaps.rs".to_string(),
        lines: (16, 20),
        node_type: "function".to_string(),
        code:
            "fn third_function() {\n    let a = 4;\n    let b = 5;\n    println!(\"{}\", a + b);\n}"
                .to_string(),
        matched_by_filename: None,
        rank: Some(3),
        score: Some(0.7),
        tfidf_score: Some(0.6),
        bm25_score: Some(0.65),
        tfidf_rank: Some(3),
        bm25_rank: Some(3),
        new_score: Some(0.67),
        hybrid2_rank: Some(3),
        combined_score_rank: Some(3),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(2),
        parent_file_id: None,
        block_id: None,
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Test with default threshold (5)
    {
        let blocks = vec![block1.clone(), block2.clone(), block3.clone()];
        let merged_blocks = merge_ranked_blocks(blocks, Some(5));

        // With threshold 5, all blocks should be merged
        assert_eq!(
            merged_blocks.len(),
            1,
            "With threshold 5, all blocks should be merged"
        );

        // Check that the merged block has the correct line range
        let merged_block = &merged_blocks[0];
        assert_eq!(
            merged_block.lines,
            (1, 20),
            "Merged block should span from line 1 to 20"
        );
    }

    // Test with smaller threshold (2)
    {
        let blocks = vec![block1.clone(), block2.clone(), block3.clone()];
        let merged_blocks = merge_ranked_blocks(blocks, Some(2));

        // With threshold 2, block1 and block2 should not be merged (gap of 3),
        // but block2 and block3 should be merged (gap of 2)
        assert_eq!(
            merged_blocks.len(),
            2,
            "With threshold 2, we should have 2 merged blocks"
        );

        // Find the blocks
        let first_block = merged_blocks.iter().find(|b| b.lines.0 == 1).unwrap();
        let second_block = merged_blocks.iter().find(|b| b.lines.0 == 9).unwrap();

        // Check line ranges
        assert_eq!(
            first_block.lines,
            (1, 5),
            "First block should span from line 1 to 5"
        );
        assert_eq!(
            second_block.lines,
            (9, 20),
            "Second block should span from line 9 to 20"
        );
    }

    // Test with threshold 0 (no merging)
    {
        let blocks = vec![block1.clone(), block2.clone(), block3.clone()];
        let merged_blocks = merge_ranked_blocks(blocks, Some(0));

        // With threshold 0, no blocks should be merged
        assert_eq!(
            merged_blocks.len(),
            3,
            "With threshold 0, no blocks should be merged"
        );
    }
}

/// Test merging of blocks with overlapping lines
#[test]
fn test_merge_overlapping_blocks() {
    // Create test blocks with overlapping lines
    let block1 = SearchResult {
        file: "overlap.rs".to_string(),
        lines: (1, 7),
        node_type: "function".to_string(),
        code: "fn first_function() {\n    let x = 1;\n    let y = 2;\n    println!(\"{}\", x + y);\n    // Shared lines\n    let shared = true;\n}".to_string(),
        matched_by_filename: None,
        rank: Some(1),
        score: Some(0.9),
        tfidf_score: Some(0.8),
        bm25_score: Some(0.85),
        tfidf_rank: Some(1),
        bm25_rank: Some(1),
        new_score: Some(0.87),
        hybrid2_rank: Some(1),
        combined_score_rank: Some(1),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(3),
        parent_file_id: None,
        block_id: None,
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Overlaps with block1 (lines 5-7 are shared)
    let block2 = SearchResult {
        file: "overlap.rs".to_string(),
        lines: (5, 10),
        node_type: "function".to_string(),
        code: "    // Shared lines\n    let shared = true;\n}\n\nfn second_function() {\n    let z = 3;\n}".to_string(),
        matched_by_filename: None,
        rank: Some(2),
        score: Some(0.8),
        tfidf_score: Some(0.7),
        bm25_score: Some(0.75),
        tfidf_rank: Some(2),
        bm25_rank: Some(2),
        new_score: Some(0.77),
        hybrid2_rank: Some(2),
        combined_score_rank: Some(2),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(2),
        parent_file_id: None,
        block_id: None,
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Create a vector with both blocks
    let blocks = vec![block1, block2];

    // Call the merge_ranked_blocks function
    let merged_blocks = merge_ranked_blocks(blocks, Some(5));

    // Assert that blocks are merged into one
    assert_eq!(
        merged_blocks.len(),
        1,
        "Overlapping blocks should be merged into one"
    );

    // Check that the merged block has the correct line range
    let merged_block = &merged_blocks[0];
    assert_eq!(
        merged_block.lines,
        (1, 10),
        "Merged block should span from line 1 to 10"
    );

    // Check that the score is the maximum of both blocks
    assert_eq!(
        merged_block.score,
        Some(0.9),
        "Merged score should be the maximum of both blocks"
    );
}

/// Test merging of blocks with complex file structure
#[test]
fn test_merge_complex_file_structure() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create a complex test file with multiple functions, classes, and comments
    let complex_file_path = temp_path.join("complex_structure.rs");
    let complex_file_content = r#"
fn first_function() {
    let x = 1;
    let y = 2;
    let result = calculate(x, y);
    println!("{}", result);
}

fn second_function() {
    let z = 3;
    let w = 4;
    let result = calculate(z, w);
    println!("{}", result);
}

// Large gap

fn third_function() {
    let a = 5;
    let b = 6;
    let result = calculate(a, b);
    println!("{}", result);
}
"#;

    // Write file to disk
    fs::write(complex_file_path, complex_file_content).unwrap();

    // Create search queries that exactly match content in the file
    let queries = vec!["result".to_string()]; // This appears in every function and will help us test merging
    let custom_ignores: Vec<String> = vec![];

    // Test with different merge thresholds
    for &threshold in &[2, 5, 10, 20] {
        // Create SearchOptions with the current threshold
        let options = SearchOptions {
            path: temp_path,
            queries: &queries,
            files_only: false,
            custom_ignores: &custom_ignores,
            exclude_filenames: true,
            language: None,
            reranker: "combined",
            frequency_search: false,
            max_results: None,
            max_bytes: None,
            max_tokens: None,
            allow_tests: true,
            no_merge: false,
            merge_threshold: Some(threshold),
            dry_run: false,
            session: None,
            timeout: 30,
            question: None,
            exact: false,
            no_gitignore: false,
        };

        // Run the search
        let search_results = perform_probe(&options).unwrap();

        // Check that we got results
        assert!(
            !search_results.results.is_empty(),
            "Search should return results"
        );

        // Count results for the complex file
        let complex_file_results: Vec<&SearchResult> = search_results
            .results
            .iter()
            .filter(|r| r.file.ends_with("complex_structure.rs"))
            .collect();

        // Print the number of results for debugging
        println!(
            "With threshold {}, got {} results for complex file",
            threshold,
            complex_file_results.len()
        );

        // With a small threshold, we should have more blocks
        // With a large threshold, we should have fewer blocks
        if threshold <= 5 {
            // With threshold <= 5, first and second functions should be merged,
            // but third function should be separate (due to large gap)
            assert!(
                complex_file_results.len() <= 2,
                "With threshold {}, expected at most 2 blocks, got {}",
                threshold,
                complex_file_results.len()
            );
        } else if threshold >= 20 {
            // With threshold >= 20, all functions should be merged
            assert_eq!(
                complex_file_results.len(),
                1,
                "With threshold {}, expected 1 block, got {}",
                threshold,
                complex_file_results.len()
            );
        }
    }
}

/// Test merging of blocks with parent-child relationships
#[test]
fn test_merge_parent_child_blocks() {
    // Create test blocks with parent-child relationships
    let parent_block = SearchResult {
        file: "parent_child.rs".to_string(),
        lines: (1, 10),
        node_type: "class".to_string(),
        code: "struct TestStruct {\n    x: i32,\n    y: i32,\n}\n\nimpl TestStruct {\n    fn new(x: i32, y: i32) -> Self {\n        Self { x, y }\n    }\n}".to_string(),
        matched_by_filename: None,
        rank: Some(2),
        score: Some(0.8),
        tfidf_score: Some(0.7),
        bm25_score: Some(0.75),
        tfidf_rank: Some(2),
        bm25_rank: Some(2),
        new_score: Some(0.77),
        hybrid2_rank: Some(2),
        combined_score_rank: Some(2),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(3),
        parent_file_id: Some("parent_child.rs".to_string()),
        block_id: Some(0),
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Child block (method inside the struct)
    let child_block = SearchResult {
        file: "parent_child.rs".to_string(),
        lines: (7, 9),
        node_type: "function".to_string(),
        code: "    fn new(x: i32, y: i32) -> Self {\n        Self { x, y }\n    }".to_string(),
        matched_by_filename: None,
        rank: Some(1),
        score: Some(0.9),
        tfidf_score: Some(0.8),
        bm25_score: Some(0.85),
        tfidf_rank: Some(1),
        bm25_rank: Some(1),
        new_score: Some(0.87),
        hybrid2_rank: Some(1),
        combined_score_rank: Some(1),
        file_unique_terms: Some(3),
        file_total_matches: Some(5),
        file_match_rank: Some(1),
        block_unique_terms: Some(2),
        block_total_matches: Some(2),
        parent_file_id: Some("parent_child.rs".to_string()),
        block_id: Some(1),
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Create a vector with both blocks
    let blocks = vec![child_block, parent_block];

    // Call the merge_ranked_blocks function
    let merged_blocks = merge_ranked_blocks(blocks, Some(5));

    // Assert that blocks are merged into one
    assert_eq!(
        merged_blocks.len(),
        1,
        "Parent-child blocks should be merged into one"
    );

    // Check that the merged block has the correct line range
    let merged_block = &merged_blocks[0];
    assert_eq!(
        merged_block.lines,
        (1, 10),
        "Merged block should span from line 1 to 10"
    );

    // Check that the score is the maximum of both blocks
    assert_eq!(
        merged_block.score,
        Some(0.9),
        "Merged score should be the maximum of both blocks"
    );

    // Check that the node_type is from the highest-ranked block
    assert_eq!(
        merged_block.node_type, "function",
        "Merged block should have the node_type of the highest-ranked block"
    );
}
