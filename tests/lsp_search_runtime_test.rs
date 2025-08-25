//! Test specifically for LSP search runtime issues
//!
//! This test is designed to catch the runtime panic that occurs when
//! search with LSP enrichment is called from an async context.

mod common;

use anyhow::Result;
use common::LspTestGuard;
use probe_code::models::SearchResult;
use probe_code::search::lsp_enrichment::enrich_results_with_lsp;
use std::env;
use std::path::PathBuf;

/// Test that would have caught the runtime panic issue
/// This simulates the exact conditions that caused the panic:
/// - Running in an async runtime context (like the search command does)
/// - Calling enrich_results_with_lsp which was creating a new runtime
#[tokio::test]
async fn test_lsp_enrichment_in_async_context() -> Result<()> {
    let _guard = LspTestGuard::new("test_lsp_enrichment_in_async_context");

    // Create a sample search result
    let mut results = vec![create_test_search_result()];

    // This is the key test - calling from async context
    // Before the fix, this would panic with:
    // "Cannot start a runtime from within a runtime"
    let enrichment_result =
        tokio::task::spawn_blocking(move || enrich_results_with_lsp(&mut results, false)).await?;

    // The function should complete without panic
    assert!(
        enrichment_result.is_ok(),
        "LSP enrichment should not panic in async context"
    );

    Ok(())
}

/// Test the same functionality but from sync context (how unit tests run)
#[test]
fn test_lsp_enrichment_in_sync_context() -> Result<()> {
    let _guard = LspTestGuard::new("test_lsp_enrichment_in_sync_context");

    // Create a sample search result
    let mut results = vec![create_test_search_result()];

    // This should work in sync context (and always has)
    let result = enrich_results_with_lsp(&mut results, false);

    // Should complete without issues
    assert!(result.is_ok(), "LSP enrichment should work in sync context");

    Ok(())
}

/// Test that the runtime detection works correctly
#[test]
fn test_runtime_detection() {
    let _guard = LspTestGuard::new("test_runtime_detection");

    // Outside of async context
    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "Should not detect runtime in sync context"
    );

    // Inside async context
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        assert!(
            tokio::runtime::Handle::try_current().is_ok(),
            "Should detect runtime in async context"
        );
    });
}

/// Test that we can handle nested async operations correctly
#[tokio::test]
async fn test_nested_async_operations() -> Result<()> {
    let _guard = LspTestGuard::new("test_nested_async_operations");

    // Simulate what happens in the search command
    let handle = tokio::spawn(async {
        // This is like the search_runner being in async context
        let mut results = vec![create_test_search_result()];

        // Use spawn_blocking to run the sync code that needs runtime
        tokio::task::spawn_blocking(move || enrich_results_with_lsp(&mut results, false)).await
    });

    let result = handle.await?;
    assert!(result.is_ok(), "Nested async operations should work");

    Ok(())
}

/// Helper to create a test search result
fn create_test_search_result() -> SearchResult {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("src/main.rs");

    SearchResult {
        file: path.to_string_lossy().to_string(),
        lines: (10, 20),
        code: r#"fn main() {
    println!("Hello, world!");
}"#
        .to_string(),
        node_type: "function_item".to_string(),
        score: Some(1.0),
        tfidf_score: None,
        bm25_score: Some(1.0),
        new_score: None,
        rank: Some(1),
        matched_keywords: Some(vec!["main".to_string()]),
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
    }
}

/// Integration test that runs the actual search command with LSP
/// This is the most realistic test - it runs the full pipeline
#[test]
#[ignore] // Ignore by default as it requires LSP daemon
fn test_search_command_with_lsp_integration() -> Result<()> {
    let _guard = LspTestGuard::new("test_search_command_with_lsp_integration");

    use std::process::Command;

    // Build the project first to ensure binary exists
    let build_output = Command::new("cargo")
        .args(["build", "--release"])
        .output()?;

    if !build_output.status.success() {
        eprintln!(
            "Build failed: {}",
            String::from_utf8_lossy(&build_output.stderr)
        );
        return Err(anyhow::anyhow!("Failed to build project"));
    }

    // Run the actual search command with LSP
    let output = Command::new("./target/release/probe")
        .args(["search", "main", "src", "--lsp", "--max-results", "1"])
        .env("RUST_BACKTRACE", "1") // Capture panic backtrace if it occurs
        .output()?;

    // Check for the specific panic message that was occurring
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Cannot start a runtime from within a runtime"),
        "Should not have runtime panic: {stderr}"
    );

    // Command should succeed
    assert!(
        output.status.success(),
        "Search command should succeed. Stderr: {stderr}"
    );

    Ok(())
}
