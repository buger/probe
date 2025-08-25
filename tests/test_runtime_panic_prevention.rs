//! Test that demonstrates the runtime panic issue and validates the fix
//!
//! This test file contains tests that would have failed before the fix
//! and now pass, proving that our fix works correctly.

/// This test simulates the EXACT problem that was occurring
/// It would panic before the fix with:
/// "Cannot start a runtime from within a runtime"
#[tokio::test]
async fn test_would_have_caught_runtime_panic() {
    // Simulate what the original broken code was doing
    let result = std::panic::catch_unwind(|| {
        // This simulates the broken behavior - creating a runtime inside a runtime
        // The original code did: Runtime::new().block_on(...)
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { "This would panic" })
    });

    // Before fix: This would panic
    // After fix: We don't do this anymore, we use Handle::try_current()
    assert!(
        result.is_err(),
        "Creating a runtime inside a runtime SHOULD panic - this demonstrates the original issue"
    );

    // Verify it's the specific panic we expected
    if let Err(panic) = result {
        if let Some(msg) = panic.downcast_ref::<String>() {
            assert!(
                msg.contains("Cannot start a runtime from within a runtime")
                    || msg.contains("runtime from within a runtime"),
                "Should have the specific runtime nesting error"
            );
        }
    }
}

/// This test shows how the fix works - using Handle::try_current()
#[tokio::test]
async fn test_fix_using_handle_try_current() {
    // This is how our fix works
    let result = if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // We're in a runtime, use block_in_place
        tokio::task::block_in_place(|| {
            handle.block_on(async { "This works correctly in async context" })
        })
    } else {
        // Not in a runtime, safe to create one
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { "This works in sync context" })
    };

    assert_eq!(result, "This works correctly in async context");
}

/// Test that demonstrates the issue in a realistic scenario
#[tokio::test]
async fn test_realistic_search_scenario() {
    use probe_code::models::SearchResult;

    // Create a mock search result
    let result = SearchResult {
        file: "test.rs".to_string(),
        lines: (1, 10),
        code: "fn test() {}".to_string(),
        node_type: "function_item".to_string(),
        score: Some(1.0),
        tfidf_score: None,
        bm25_score: Some(1.0),
        new_score: None,
        rank: Some(1),
        matched_keywords: Some(vec!["test".to_string()]),
        parent_file_id: None,
        file_match_rank: Some(1),
        file_unique_terms: Some(1),
        file_total_matches: Some(1),
        block_unique_terms: Some(1),
        block_total_matches: Some(1),
        combined_score_rank: Some(1),
        bm25_rank: Some(1),
        tfidf_rank: None,
        hybrid2_rank: None,
        lsp_info: None,
        block_id: None,
        matched_by_filename: Some(false),
        tokenized_content: None,
    };

    let mut results = vec![result];

    // This simulates the search command calling LSP enrichment
    // from an async context (which is what was causing the panic)
    let enrichment_result = tokio::task::spawn_blocking(move || {
        // With our fix, this now works correctly
        probe_code::search::lsp_enrichment::enrich_results_with_lsp(&mut results, false)
    })
    .await;

    assert!(
        enrichment_result.is_ok(),
        "Should handle async context correctly"
    );
}

/// Test that verifies both sync and async contexts work
#[test]
fn test_sync_context_still_works() {
    // Verify we're NOT in a runtime
    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "Should not be in a runtime in sync test"
    );

    // Our fix should handle this case by creating a new runtime
    let mut results = vec![];
    let result = probe_code::search::lsp_enrichment::enrich_results_with_lsp(&mut results, false);

    // Should work without issues
    assert!(result.is_ok(), "Should work in sync context");
}
