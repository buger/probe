use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::Command;

/// This test reproduces non-deterministic search behavior where the same query
/// returns different results on repeated executions.
///
/// **Issue Description:**
/// The command `probe search "yaml workflow agent multi-agent user input" [path containing "user"]`
/// returns inconsistent results:
/// - Sometimes: Lines 1-3 with content (115 bytes, 33 tokens)
/// - Sometimes: Lines 1-4 with empty content (0 bytes, 0 tokens)
///
/// **Hypothesis:** The issue may be related to filename matching vs content matching,
/// especially when the search path contains the keyword "user" from the query.
#[test]
fn test_search_determinism_with_user_path() {
    let iterations = 50; // Run more times to catch non-determinism
    let mut results: Vec<SearchResult> = Vec::new();

    // Get the path to the compiled binary
    let cargo_target_dir = env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let binary_path = PathBuf::from(&cargo_target_dir)
        .join("release")
        .join("probe");

    // If release binary doesn't exist, try debug
    let binary_path = if binary_path.exists() {
        binary_path
    } else {
        PathBuf::from(&cargo_target_dir).join("debug").join("probe")
    };

    // Path to test fixture - importantly contains "user" keyword
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("user");

    println!("Testing search determinism with {iterations} iterations");
    println!("Binary path: {binary_path:?}");
    println!("Fixture path: {fixture_path:?}");
    println!("Query: \"yaml workflow agent multi-agent user input\"");
    println!("Expected consistent results across all runs\n");

    // Test specifically with --no-merge flag since we've identified it causes non-determinism
    let use_no_merge = true;

    // Run the same search multiple times
    for i in 1..=iterations {
        let mut cmd = Command::new(&binary_path);
        cmd.args([
            "search",
            "yaml workflow agent multi-agent user input",
            fixture_path.to_str().unwrap(),
        ]);

        if use_no_merge {
            cmd.arg("--no-merge");
        }

        let output = cmd.output().expect("Failed to execute probe command");

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("Command failed on iteration {i}: {stderr}");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result = parse_search_output(&stdout);

        println!(
            "Iteration {}: {} lines, {} bytes, {} tokens",
            i, result.line_count, result.byte_count, result.token_count
        );

        results.push(result);
    }

    // Analyze results for consistency
    let mut unique_results: HashMap<String, usize> = HashMap::new();

    for result in &results {
        let key = format!(
            "{}:{}:{}",
            result.line_count, result.byte_count, result.token_count
        );
        *unique_results.entry(key).or_insert(0) += 1;
    }

    println!("\n=== DETERMINISM ANALYSIS ===");
    println!("Unique result patterns found:");
    for (pattern, count) in &unique_results {
        println!("  {pattern}: {count} occurrences");
    }

    // Test assertion: All results should be identical
    if unique_results.len() > 1 {
        println!("\n❌ NON-DETERMINISTIC BEHAVIOR DETECTED!");
        println!("Expected: All {iterations} iterations to return identical results");
        println!(
            "Actual: Found {} different result patterns",
            unique_results.len()
        );

        // Print detailed comparison of first few different results
        let first_result = &results[0];
        for (i, result) in results.iter().enumerate().skip(1) {
            if !results_equal(first_result, result) {
                println!("\nDifference detected:");
                println!(
                    "  Iteration 1: {} lines, {} bytes, {} tokens",
                    first_result.line_count, first_result.byte_count, first_result.token_count
                );
                println!(
                    "  Iteration {}: {} lines, {} bytes, {} tokens",
                    i + 1,
                    result.line_count,
                    result.byte_count,
                    result.token_count
                );
                break;
            }
        }

        panic!(
            "Search results are non-deterministic! Found {} different result patterns. This indicates a bug in the search engine that needs to be fixed.",
            unique_results.len()
        );
    } else {
        println!("\n✅ DETERMINISTIC BEHAVIOR CONFIRMED");
        println!("All {iterations} iterations returned identical results");
        let first_result = &results[0];
        println!(
            "Consistent result: {} lines, {} bytes, {} tokens",
            first_result.line_count, first_result.byte_count, first_result.token_count
        );
    }
}

/// Test specifically for the filename vs content matching hypothesis
#[test]
fn test_user_keyword_filename_vs_content_matching() {
    let cargo_target_dir = env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let binary_path = PathBuf::from(&cargo_target_dir)
        .join("release")
        .join("probe");

    let binary_path = if binary_path.exists() {
        binary_path
    } else {
        PathBuf::from(&cargo_target_dir).join("debug").join("probe")
    };

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("user");

    println!("Testing filename vs content matching hypothesis");
    println!("Path contains 'user' keyword: {fixture_path:?}");
    println!("Query contains 'user' keyword: \"yaml workflow agent multi-agent user input\"");
    println!(
        "File content does NOT contain these keywords (AssemblyInfo.cs has only copyright and assembly info)\n"
    );

    // Run a single search to see what happens
    let output = Command::new(&binary_path)
        .args([
            "search",
            "yaml workflow agent multi-agent user input",
            fixture_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute probe command");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("Command failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = parse_search_output(&stdout);

    println!(
        "Search result: {} lines, {} bytes, {} tokens",
        result.line_count, result.byte_count, result.token_count
    );
    println!("Raw output:\n{stdout}");

    // This test documents the behavior - it may return results due to filename matching
    // even though the content doesn't contain the search terms
    if result.line_count > 0 {
        println!("\n📝 OBSERVATION: Search returned results despite content mismatch");
        println!("This suggests filename/path matching is contributing to results");
    } else {
        println!("\n📝 OBSERVATION: Search returned no results, suggesting content-only matching");
    }
}

/// Test for non-deterministic behavior with different ranking algorithms and concurrent execution
#[test]
fn test_search_determinism_with_multiple_conditions() {
    let cargo_target_dir = env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let binary_path = PathBuf::from(&cargo_target_dir)
        .join("release")
        .join("probe");

    let binary_path = if binary_path.exists() {
        binary_path
    } else {
        PathBuf::from(&cargo_target_dir).join("debug").join("probe")
    };

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("user");

    println!("Testing multiple conditions that might trigger non-determinism");

    // Test different search configurations that might trigger non-determinism
    let search_configs = vec![
        ("default", vec![]),
        ("exact", vec!["--exact"]),
        ("frequency", vec!["--frequency"]),
        ("no-merge", vec!["--no-merge"]),
        ("files-only", vec!["--files-only"]),
        ("exclude-filenames", vec!["--exclude-filenames"]),
    ];

    for (name, config_flags) in search_configs {
        println!("\n--- Testing with {name} configuration ---");
        let mut results = Vec::new();

        // Run multiple times with this configuration
        for i in 1..=10 {
            let mut cmd = Command::new(&binary_path);
            cmd.args([
                "search",
                "yaml workflow agent multi-agent user input",
                fixture_path.to_str().unwrap(),
            ]);

            // Add configuration flags
            for flag in &config_flags {
                cmd.arg(flag);
            }

            let output = cmd.output().expect("Failed to execute probe command");

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                panic!("Command failed on iteration {i} with {name}: {stderr}");
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let result = parse_search_output(&stdout);
            results.push(result);
        }

        // Check for consistency within this configuration
        let first_result = &results[0];
        let mut inconsistent = false;

        for (i, result) in results.iter().enumerate().skip(1) {
            if !results_equal(first_result, result) {
                println!("❌ INCONSISTENCY DETECTED with {name} configuration!");
                println!(
                    "  Iteration 1: {} lines, {} bytes, {} tokens",
                    first_result.line_count, first_result.byte_count, first_result.token_count
                );
                println!(
                    "  Iteration {}: {} lines, {} bytes, {} tokens",
                    i + 1,
                    result.line_count,
                    result.byte_count,
                    result.token_count
                );
                inconsistent = true;
                break;
            }
        }

        if !inconsistent {
            println!(
                "✅ {} configuration is consistent: {} lines, {} bytes, {} tokens",
                name, first_result.line_count, first_result.byte_count, first_result.token_count
            );
        } else {
            // Fail the test if we detect non-deterministic behavior
            panic!(
                "Non-deterministic behavior detected with {name} configuration! This needs to be fixed."
            );
        }
    }
}

/// Test for race conditions by running multiple searches concurrently
#[test]
fn test_search_determinism_concurrent_execution() {
    use std::sync::mpsc;
    use std::thread;

    let cargo_target_dir = env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let binary_path = PathBuf::from(&cargo_target_dir)
        .join("release")
        .join("probe");

    let binary_path = if binary_path.exists() {
        binary_path
    } else {
        PathBuf::from(&cargo_target_dir).join("debug").join("probe")
    };

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("user");

    println!("Testing concurrent execution for race conditions");

    let (tx, rx) = mpsc::channel();
    let thread_count = 10;

    // Spawn multiple threads running the same search
    for thread_id in 0..thread_count {
        let tx = tx.clone();
        let binary_path = binary_path.clone();
        let fixture_path = fixture_path.clone();

        thread::spawn(move || {
            let output = Command::new(&binary_path)
                .args([
                    "search",
                    "yaml workflow agent multi-agent user input",
                    fixture_path.to_str().unwrap(),
                ])
                .output()
                .expect("Failed to execute probe command");

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                panic!("Command failed in thread {thread_id}: {stderr}");
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let result = parse_search_output(&stdout);
            tx.send((thread_id, result)).unwrap();
        });
    }

    // Collect results from all threads
    drop(tx); // Close the sender
    let mut results: Vec<(usize, SearchResult)> = rx.iter().collect();
    results.sort_by_key(|(thread_id, _)| *thread_id);

    println!("Concurrent execution results:");
    for (thread_id, result) in &results {
        println!(
            "  Thread {}: {} lines, {} bytes, {} tokens",
            thread_id, result.line_count, result.byte_count, result.token_count
        );
    }

    // Check for consistency across threads
    let first_result = &results[0].1;
    let mut inconsistent = false;

    for (thread_id, result) in results.iter().skip(1) {
        if !results_equal(first_result, result) {
            println!("❌ RACE CONDITION DETECTED!");
            println!(
                "  Thread 0: {} lines, {} bytes, {} tokens",
                first_result.line_count, first_result.byte_count, first_result.token_count
            );
            println!(
                "  Thread {}: {} lines, {} bytes, {} tokens",
                thread_id, result.line_count, result.byte_count, result.token_count
            );
            inconsistent = true;
            break;
        }
    }

    if !inconsistent {
        println!(
            "✅ Concurrent execution is consistent: {} lines, {} bytes, {} tokens",
            first_result.line_count, first_result.byte_count, first_result.token_count
        );
    }

    // This test documents behavior but doesn't fail - race conditions are hard to reproduce reliably
}

#[derive(Debug, Clone)]
struct SearchResult {
    line_count: u32,
    byte_count: u32,
    token_count: u32,
    #[allow(dead_code)]
    content: String,
}

fn parse_search_output(output: &str) -> SearchResult {
    let mut line_count = 0;
    let mut byte_count = 0;
    let mut token_count = 0;
    let mut content = String::new();

    // Look for summary lines
    for line in output.lines() {
        // New format: "Total bytes returned: X"
        if line.starts_with("Total bytes returned:") {
            if let Some(bytes_str) = line.split(':').nth(1) {
                if let Ok(bytes) = bytes_str.trim().parse::<u32>() {
                    byte_count = bytes;
                }
            }
        }

        // New format: "Total tokens returned: X"
        if line.starts_with("Total tokens returned:") {
            if let Some(tokens_str) = line.split(':').nth(1) {
                if let Ok(tokens) = tokens_str.trim().parse::<u32>() {
                    token_count = tokens;
                }
            }
        }

        // Count lines in code blocks (Lines: X-Y format)
        if line.starts_with("Lines:") {
            if let Some(range_str) = line.strip_prefix("Lines: ") {
                if let Some((start_str, end_str)) = range_str.split_once('-') {
                    if let (Ok(start), Ok(end)) = (start_str.parse::<u32>(), end_str.parse::<u32>())
                    {
                        line_count = end - start + 1;
                    }
                }
            }
        }

        content.push_str(line);
        content.push('\n');
    }

    SearchResult {
        line_count,
        byte_count,
        token_count,
        content,
    }
}

fn results_equal(a: &SearchResult, b: &SearchResult) -> bool {
    a.line_count == b.line_count && a.byte_count == b.byte_count && a.token_count == b.token_count
}
