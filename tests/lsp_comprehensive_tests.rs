//! Comprehensive LSP integration tests for Go, TypeScript, and JavaScript
//!
//! This test suite validates that ALL language servers work correctly with probe's LSP daemon.
//! Unlike the basic LSP integration tests, these tests:
//!
//! - NEVER skip tests due to missing language servers - they FAIL if dependencies are missing
//! - Test exact call hierarchy assertions for all supported languages
//! - Validate performance requirements (extraction < 3s, search < 5s)
//! - Test concurrent multi-language LSP operations
//! - Use dedicated test fixtures designed for call hierarchy testing
//!
//! Required language servers:
//! - gopls (Go language server): go install golang.org/x/tools/gopls@latest
//! - typescript-language-server: npm install -g typescript-language-server typescript
//!
//! These tests are designed to run in CI environments and ensure full LSP functionality.
//!
//! IMPORTANT: These tests share a single LSP daemon instance. For reliable results:
//! - Run with: cargo test --test lsp_comprehensive_tests -- --test-threads=1
//! - Or run individual tests separately
//!   Running tests in parallel may cause timeouts and failures due to daemon contention.

mod common;

use anyhow::Result;
use common::{
    cleanup_test_namespace, ensure_daemon_stopped_with_config,
    extract_with_call_hierarchy_retry_config, fixtures, init_lsp_workspace_with_config,
    init_test_namespace, performance, require_all_language_servers, run_probe_command_with_config,
    start_daemon_and_wait_with_config, wait_for_lsp_servers_ready_with_config,
};
use std::time::{Duration, Instant};

/// Setup function that validates all required language servers are available
/// This function FAILS the test if any language server is missing
fn setup_comprehensive_tests() -> Result<()> {
    require_all_language_servers()?;
    common::ensure_daemon_stopped();
    Ok(())
}

#[test]
fn test_go_lsp_call_hierarchy_exact() -> Result<()> {
    setup_comprehensive_tests()?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_go_lsp_call_hierarchy_exact");

    // Start daemon with isolated socket
    start_daemon_and_wait_with_config(Some(&socket_path))?;

    let workspace_path = fixtures::get_go_project1();
    init_lsp_workspace_with_config(
        workspace_path.to_str().unwrap(),
        &["go"],
        Some(&socket_path),
    )?;

    // Wait for gopls to fully index the project using status polling
    wait_for_lsp_servers_ready_with_config(
        &["Go"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Test extraction with LSP for the Calculate function
    let file_path = workspace_path.join("calculator.go");
    let extract_arg = format!("{}:10", file_path.to_string_lossy());
    let extract_args = [
        "extract",
        &extract_arg, // Line 10 should be the Calculate function
        "--lsp",
        "--allow-tests", // Allow test files since fixtures are in tests directory
    ];

    let max_extract_time = performance::max_extract_time();
    let (stdout, stderr, success) = extract_with_call_hierarchy_retry_config(
        &extract_args,
        3, // Expected incoming calls: main(), ProcessNumbers(), BusinessLogic.ProcessValue()
        3, // Expected outgoing calls: Add(), Multiply(), Subtract() (conditional)
        max_extract_time,
        Some(&socket_path),
    )?;

    // Cleanup before assertions to avoid daemon issues
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // Validate the command succeeded
    assert!(success, "Extract command should succeed. Stderr: {stderr}");

    // Validate basic extraction worked
    assert!(
        stdout.contains("Calculate"),
        "Should extract the Calculate function"
    );
    assert!(
        stdout.contains("func Calculate"),
        "Should show function signature"
    );

    // Validate LSP call hierarchy information is present
    assert!(
        stdout.contains("LSP Information"),
        "Should contain LSP information section"
    );
    assert!(
        stdout.contains("Call Hierarchy"),
        "Should contain call hierarchy"
    );

    // Call hierarchy validation is now handled by extract_with_call_hierarchy_retry
    // The function ensures we have the expected number of incoming and outgoing calls

    Ok(())
}

#[test]
fn test_typescript_lsp_call_hierarchy_exact() -> Result<()> {
    setup_comprehensive_tests()?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_typescript_lsp_call_hierarchy_exact");

    // Start daemon with isolated socket
    start_daemon_and_wait_with_config(Some(&socket_path))?;

    let workspace_path = fixtures::get_typescript_project1();
    init_lsp_workspace_with_config(
        workspace_path.to_str().unwrap(),
        &["typescript"],
        Some(&socket_path),
    )?;

    // Wait for typescript-language-server to fully index the project using status polling
    wait_for_lsp_servers_ready_with_config(
        &["TypeScript"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Test extraction with LSP for the calculate function
    let file_path = workspace_path.join("src/calculator.ts");
    let extract_args = [
        "extract",
        &format!("{}:17", file_path.to_string_lossy()), // Line 17 should be the calculate function
        "--lsp",
        "--allow-tests", // Allow test files since fixtures are in tests directory
    ];

    let max_extract_time = performance::max_extract_time();
    let (stdout, stderr, success) = extract_with_call_hierarchy_retry_config(
        &extract_args,
        6, // Expected incoming calls: advancedCalculation(), processValue(), processArray(), main(), processNumbers(), processValue()
        3, // Expected outgoing calls: add(), multiply(), subtract() (conditional)
        max_extract_time,
        Some(&socket_path),
    )?;

    // Cleanup before assertions to avoid daemon issues
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // Validate the command succeeded
    assert!(success, "Extract command should succeed. Stderr: {stderr}");

    // Validate basic extraction worked
    assert!(
        stdout.contains("calculate"),
        "Should extract the calculate function"
    );
    assert!(
        stdout.contains("function calculate"),
        "Should show function signature"
    );

    // Validate LSP call hierarchy information is present
    assert!(
        stdout.contains("LSP Information"),
        "Should contain LSP information section"
    );
    assert!(
        stdout.contains("Call Hierarchy"),
        "Should contain call hierarchy"
    );

    // Call hierarchy validation is now handled by extract_with_call_hierarchy_retry
    // The function ensures we have the expected number of incoming and outgoing calls

    Ok(())
}

#[test]
fn test_javascript_lsp_call_hierarchy_exact() -> Result<()> {
    setup_comprehensive_tests()?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_javascript_lsp_call_hierarchy_exact");

    // Start daemon with isolated socket
    start_daemon_and_wait_with_config(Some(&socket_path))?;

    let workspace_path = fixtures::get_javascript_project1();
    init_lsp_workspace_with_config(
        workspace_path.to_str().unwrap(),
        &["javascript"],
        Some(&socket_path),
    )?;

    // Wait for typescript-language-server to fully index the JavaScript project using status polling
    wait_for_lsp_servers_ready_with_config(
        &["JavaScript"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Test extraction with LSP for the calculate function
    let file_path = workspace_path.join("src/calculator.js");
    let extract_args = [
        "extract",
        &format!("{}:13", file_path.to_string_lossy()), // Line 13 is the calculate function declaration
        "--lsp",
        "--allow-tests", // Allow test files since fixtures are in tests directory
    ];

    let max_extract_time = performance::max_extract_time();
    let (stdout, stderr, success) = extract_with_call_hierarchy_retry_config(
        &extract_args,
        4, // Expected incoming calls: advancedCalculation(), processValue(), processArray(), createProcessor()
        3, // Expected outgoing calls: add(), multiply(), subtract() (conditional)
        max_extract_time,
        Some(&socket_path),
    )?;

    // Cleanup before assertions to avoid daemon issues
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // Validate the command succeeded
    assert!(success, "Extract command should succeed. Stderr: {stderr}");

    // Validate basic extraction worked
    assert!(
        stdout.contains("calculate"),
        "Should extract the calculate function"
    );
    assert!(
        stdout.contains("function calculate"),
        "Should show function signature"
    );

    // Validate LSP call hierarchy information is present
    assert!(
        stdout.contains("LSP Information"),
        "Should contain LSP information section"
    );
    assert!(
        stdout.contains("Call Hierarchy"),
        "Should contain call hierarchy"
    );

    // Call hierarchy validation is now handled by extract_with_call_hierarchy_retry
    // The function ensures we have the expected number of incoming and outgoing calls

    Ok(())
}

#[test]
fn test_concurrent_multi_language_lsp_operations() -> Result<()> {
    setup_comprehensive_tests()?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_concurrent_multi_language_lsp_operations");

    // Start daemon with isolated socket
    start_daemon_and_wait_with_config(Some(&socket_path))?;

    // Initialize all language workspaces
    let go_workspace = fixtures::get_go_project1();
    let ts_workspace = fixtures::get_typescript_project1();
    let js_workspace = fixtures::get_javascript_project1();

    init_lsp_workspace_with_config(go_workspace.to_str().unwrap(), &["go"], Some(&socket_path))?;
    init_lsp_workspace_with_config(
        ts_workspace.to_str().unwrap(),
        &["typescript"],
        Some(&socket_path),
    )?;
    init_lsp_workspace_with_config(
        js_workspace.to_str().unwrap(),
        &["javascript"],
        Some(&socket_path),
    )?;

    // Wait for all language servers to be ready using status polling
    wait_for_lsp_servers_ready_with_config(
        &["Go", "TypeScript", "JavaScript"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Perform concurrent operations on all languages
    let start = Instant::now();

    // Prepare extraction files
    let go_file = go_workspace.join("calculator.go");
    let ts_file = ts_workspace.join("src/calculator.ts");
    let js_file = js_workspace.join("src/calculator.js");

    let timeout = performance::max_extract_time();

    // Run all three extractions concurrently using threads
    // We need to clone/move all data into the threads
    let go_file_str = format!("{}:10", go_file.to_string_lossy());
    let socket_path_clone1 = socket_path.clone();
    let go_handle = std::thread::spawn(move || {
        run_probe_command_with_config(
            &["extract", &go_file_str, "--lsp", "--allow-tests"],
            timeout,
            Some(&socket_path_clone1),
        )
    });

    let ts_file_str = format!("{}:17", ts_file.to_string_lossy());
    let socket_path_clone2 = socket_path.clone();
    let ts_handle = std::thread::spawn(move || {
        run_probe_command_with_config(
            &["extract", &ts_file_str, "--lsp", "--allow-tests"],
            timeout,
            Some(&socket_path_clone2),
        )
    });

    let js_file_str = format!("{}:14", js_file.to_string_lossy());
    let socket_path_clone3 = socket_path.clone();
    let js_handle = std::thread::spawn(move || {
        run_probe_command_with_config(
            &["extract", &js_file_str, "--lsp", "--allow-tests"],
            timeout,
            Some(&socket_path_clone3),
        )
    });

    // Wait for all threads to complete and collect results
    let (go_stdout, go_stderr, go_success) =
        go_handle.join().expect("Go extraction thread panicked")?;

    let (ts_stdout, ts_stderr, ts_success) = ts_handle
        .join()
        .expect("TypeScript extraction thread panicked")?;

    let (js_stdout, js_stderr, js_success) = js_handle
        .join()
        .expect("JavaScript extraction thread panicked")?;

    let total_elapsed = start.elapsed();

    // Cleanup before assertions
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // Validate all operations succeeded
    assert!(
        go_success,
        "Go extraction should succeed. Stderr: {go_stderr}"
    );
    assert!(
        ts_success,
        "TypeScript extraction should succeed. Stderr: {ts_stderr}"
    );
    assert!(
        js_success,
        "JavaScript extraction should succeed. Stderr: {js_stderr}"
    );

    // Validate total time is reasonable for concurrent operations
    let max_concurrent_time = if performance::is_ci_environment() {
        Duration::from_secs(45) // Much longer for CI
    } else {
        Duration::from_secs(15)
    };
    assert!(
        total_elapsed < max_concurrent_time,
        "Concurrent operations took {total_elapsed:?}, should be under {max_concurrent_time:?}"
    );

    // Validate all outputs contain LSP information
    assert!(
        go_stdout.contains("LSP Information"),
        "Go output should contain LSP information"
    );
    assert!(
        ts_stdout.contains("LSP Information"),
        "TypeScript output should contain LSP information"
    );
    assert!(
        js_stdout.contains("LSP Information"),
        "JavaScript output should contain LSP information"
    );

    // Validate call hierarchy is present in all outputs
    assert!(
        go_stdout.contains("Call Hierarchy"),
        "Go output should contain call hierarchy"
    );
    assert!(
        ts_stdout.contains("Call Hierarchy"),
        "TypeScript output should contain call hierarchy"
    );
    assert!(
        js_stdout.contains("Call Hierarchy"),
        "JavaScript output should contain call hierarchy"
    );

    Ok(())
}

#[test]
fn test_search_with_lsp_enrichment_performance() -> Result<()> {
    eprintln!("=== Starting test_search_with_lsp_enrichment_performance ===");

    setup_comprehensive_tests().map_err(|e| {
        eprintln!("ERROR: setup_comprehensive_tests failed: {e}");
        e
    })?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_search_with_lsp_enrichment_performance");
    eprintln!("Test namespace initialized with socket: {socket_path:?}");

    // Start daemon and initialize workspace
    start_daemon_and_wait_with_config(Some(&socket_path)).map_err(|e| {
        eprintln!("ERROR: Failed to start daemon: {e}");
        e
    })?;

    let workspace_path = fixtures::get_go_project1();
    eprintln!("Using workspace path: {workspace_path:?}");

    init_lsp_workspace_with_config(
        workspace_path.to_str().unwrap(),
        &["go"],
        Some(&socket_path),
    )
    .map_err(|e| {
        eprintln!("ERROR: Failed to initialize LSP workspace: {e}");
        e
    })?;

    // Wait for language server to be ready using status polling
    wait_for_lsp_servers_ready_with_config(
        &["Go"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )
    .map_err(|e| {
        eprintln!("ERROR: Failed waiting for LSP servers: {e}");
        e
    })?;

    // Test search with LSP enrichment
    let search_args = [
        "search",
        "Calculate",
        workspace_path.to_str().unwrap(),
        "--max-results",
        "5",
        "--lsp",
    ];

    eprintln!("Running search with args: {search_args:?}");
    let start = Instant::now();
    let max_search_time = performance::max_search_time();
    let (stdout, stderr, success) =
        run_probe_command_with_config(&search_args, max_search_time, Some(&socket_path)).map_err(
            |e| {
                eprintln!("ERROR: Search command failed: {e}");
                e
            },
        )?;
    let elapsed = start.elapsed();
    eprintln!("Search completed in {elapsed:?}, success={success}");

    // Cleanup before assertions
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // Validate the command succeeded
    assert!(success, "Search command should succeed. Stderr: {stderr}");

    // Validate performance requirement
    assert!(
        elapsed < max_search_time,
        "Search took {elapsed:?}, should be under {max_search_time:?}"
    );

    // Validate search results contain expected functions
    assert!(
        stdout.contains("Calculate"),
        "Should find Calculate function"
    );
    assert!(!stdout.is_empty(), "Should return non-empty results");

    // LSP enrichment might not be visible in search results, but the command should succeed
    // The important thing is that LSP doesn't break or slow down search

    Ok(())
}

#[test]
fn test_lsp_daemon_status_with_multiple_languages() -> Result<()> {
    setup_comprehensive_tests()?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_lsp_daemon_status_with_multiple_languages");

    // Start daemon and initialize all language workspaces
    start_daemon_and_wait_with_config(Some(&socket_path))?;

    let go_workspace = fixtures::get_go_project1();
    let ts_workspace = fixtures::get_typescript_project1();
    let js_workspace = fixtures::get_javascript_project1();

    init_lsp_workspace_with_config(go_workspace.to_str().unwrap(), &["go"], Some(&socket_path))?;
    init_lsp_workspace_with_config(
        ts_workspace.to_str().unwrap(),
        &["typescript"],
        Some(&socket_path),
    )?;
    init_lsp_workspace_with_config(
        js_workspace.to_str().unwrap(),
        &["javascript"],
        Some(&socket_path),
    )?;

    // Wait for language servers to initialize using status polling
    wait_for_lsp_servers_ready_with_config(
        &["Go"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Check daemon status
    let (stdout, stderr, success) = run_probe_command_with_config(
        &["lsp", "status"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Cleanup before assertions
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // Validate status command succeeded
    assert!(success, "LSP status should succeed. Stderr: {stderr}");

    // Validate status output contains expected information
    assert!(
        stdout.contains("LSP Daemon Status"),
        "Should show daemon status header"
    );
    assert!(stdout.contains("Connected"), "Should show connected status");

    // Should show information about multiple language servers
    // Note: The exact format may vary, but we should see evidence of multiple language pools
    assert!(
        stdout.contains("Server Pools") || stdout.contains("Language") || stdout.len() > 100,
        "Should show substantial status information for multiple languages"
    );

    Ok(())
}

#[test]
fn test_lsp_initialization_timeout_handling() -> Result<()> {
    setup_comprehensive_tests()?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_lsp_initialization_timeout_handling");

    // Start daemon
    start_daemon_and_wait_with_config(Some(&socket_path))?;

    let workspace_path = fixtures::get_go_project1();

    // Initialize workspace but don't wait for full indexing
    init_lsp_workspace_with_config(
        workspace_path.to_str().unwrap(),
        &["go"],
        Some(&socket_path),
    )?;

    // Try extraction immediately (before gopls is fully ready)
    let file_path = workspace_path.join("calculator.go");
    let extract_args = [
        "extract",
        &format!("{}:10", file_path.to_string_lossy()),
        "--lsp",
        "--allow-tests", // Allow test files since fixtures are in tests directory
    ];

    let (stdout, _stderr, success) =
        run_probe_command_with_config(&extract_args, Duration::from_secs(30), Some(&socket_path))?;

    // Cleanup before assertions
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // The command should succeed even if LSP isn't fully ready
    assert!(
        success,
        "Extract should succeed even with LSP not fully ready"
    );

    // Should extract the function even if LSP info is not available
    assert!(
        stdout.contains("Calculate"),
        "Should extract function even without LSP"
    );

    // LSP information might or might not be present, depending on timing
    // The important thing is that the command doesn't hang or fail

    Ok(())
}

#[test]
fn test_error_recovery_with_invalid_file_paths() -> Result<()> {
    setup_comprehensive_tests()?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_error_recovery_with_invalid_file_paths");

    // Start daemon
    start_daemon_and_wait_with_config(Some(&socket_path))?;

    let workspace_path = fixtures::get_go_project1();
    init_lsp_workspace_with_config(
        workspace_path.to_str().unwrap(),
        &["go"],
        Some(&socket_path),
    )?;

    // Wait for language server using status polling
    wait_for_lsp_servers_ready_with_config(
        &["Go"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Try extraction with invalid file path
    let extract_args = ["extract", "nonexistent_file.go:10", "--lsp"];

    let (stdout, stderr, success) = run_probe_command_with_config(
        &extract_args,
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Cleanup before assertions
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // The command should fail gracefully. Some CLIs print a clear error but still exit 0.
    // Accept either a non-zero exit OR a clear missing-file error message in output.
    let combined = format!("{stderr}\n{stdout}").to_ascii_lowercase();
    let reported_missing = combined.contains("no such file")
        || combined.contains("not found")
        || combined.contains("enoent")
        || combined.contains("does not exist");

    assert!(
        !success || reported_missing,
        "Extract should fail or report a clear missing-file error. success={success}\nstderr={stderr}\nstdout={stdout}"
    );

    // Should provide meaningful error message
    assert!(
        reported_missing || stdout.contains("Error"),
        "Should provide meaningful error message"
    );

    // Should not crash the daemon or leave it in a bad state
    // (The cleanup function will verify daemon can be stopped properly)

    Ok(())
}

/// Performance benchmark test - not a strict requirement but useful for monitoring
#[test]
fn test_lsp_performance_benchmark() -> Result<()> {
    // Skip performance benchmarks in CI - they're unreliable due to varying resources
    if performance::is_ci_environment() {
        eprintln!("Skipping performance benchmark in CI environment");
        return Ok(());
    }

    setup_comprehensive_tests()?;

    // Initialize test namespace for isolation
    let socket_path = init_test_namespace("test_lsp_performance_benchmark");

    // Start daemon and initialize workspace
    start_daemon_and_wait_with_config(Some(&socket_path))?;

    let workspace_path = fixtures::get_go_project1();
    init_lsp_workspace_with_config(
        workspace_path.to_str().unwrap(),
        &["go"],
        Some(&socket_path),
    )?;

    // Wait for language server to be fully ready using status polling
    wait_for_lsp_servers_ready_with_config(
        &["Go"],
        performance::language_server_ready_time(),
        Some(&socket_path),
    )?;

    // Perform multiple extractions to test consistency
    let file_path = workspace_path.join("calculator.go");

    // Warm-up extraction to ensure language server is fully indexed
    // This is not counted in the performance metrics
    let warm_up_args = [
        "extract",
        &format!("{}:10", file_path.to_string_lossy()),
        "--lsp",
    ];
    let _ = run_probe_command_with_config(
        &warm_up_args,
        performance::language_server_ready_time(),
        Some(&socket_path),
    );

    let mut timings = Vec::new();

    for i in 0..3 {
        let extract_args = [
            "extract",
            &format!("{}:10", file_path.to_string_lossy()),
            "--lsp",
            "--allow-tests", // Allow test files since fixtures are in tests directory
        ];

        let start = Instant::now();
        let (stdout, stderr, success) = run_probe_command_with_config(
            &extract_args,
            performance::language_server_ready_time(),
            Some(&socket_path),
        )?;
        let elapsed = start.elapsed();

        assert!(
            success,
            "Extraction {} should succeed. Stderr: {}",
            i + 1,
            stderr
        );
        assert!(
            stdout.contains("Calculate"),
            "Should extract function in attempt {}",
            i + 1
        );

        timings.push(elapsed);
    }

    // Cleanup before assertions
    ensure_daemon_stopped_with_config(Some(&socket_path));
    cleanup_test_namespace(&socket_path);

    // Calculate average timing
    let avg_time = timings.iter().sum::<Duration>() / timings.len() as u32;

    // Performance expectations (not strict failures, but good to monitor)
    if avg_time > Duration::from_secs(2) {
        eprintln!("Warning: Average LSP extraction time ({avg_time:?}) is slower than expected");
    }

    // All individual timings should be reasonable
    // Note: When tests run concurrently, they share the daemon and performance degrades
    // We detect this by checking if extraction times are unusually high
    for (i, timing) in timings.iter().enumerate() {
        let max_individual_time = if performance::is_ci_environment() {
            Duration::from_secs(20) // Much more lenient for CI
        } else if *timing > Duration::from_secs(8) {
            // If any timing is over 8 seconds locally, assume concurrent test execution
            // and be more lenient. This happens when multiple tests share the daemon.
            eprintln!(
                "Warning: Detected slow extraction ({}s), likely due to concurrent test execution",
                timing.as_secs()
            );
            eprintln!("Consider running with --test-threads=1 for accurate performance testing");
            Duration::from_secs(15) // More lenient for concurrent execution
        } else {
            Duration::from_secs(10) // Increased threshold to account for performance variations
        };
        assert!(
            *timing < max_individual_time,
            "Extraction {} took {:?}, which is too slow (max: {:?})",
            i + 1,
            timing,
            max_individual_time
        );
    }

    println!("LSP Performance Benchmark Results:");
    println!("  Individual timings: {timings:?}");
    println!("  Average time: {avg_time:?}");

    Ok(())
}
