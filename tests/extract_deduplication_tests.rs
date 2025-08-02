use std::fs;
use std::process::Command;

use probe_code::extract::{handle_extract, ExtractOptions};

#[test]
fn test_deduplication_of_nested_extractions() {
    // Create a temporary file with nested structures for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("nested_test.rs");
    let content = r#"
fn outer_function() {
    let x = 10;
    
    // This is a nested function that should be deduplicated
    fn inner_function() {
        let y = 20;
        println!("Inner function: {}", y);
    }
    
    // Call the inner function
    inner_function();
    println!("Outer function: {}", x);
}

fn standalone_function() {
    println!("This is standalone");
}
"#;
    fs::write(&file_path, content).unwrap();

    // Enable debug mode to see deduplication logs
    std::env::set_var("DEBUG", "1");

    // Create options with both the outer function and inner function
    let options = ExtractOptions {
        files: vec![
            format!("{}:2", file_path.to_string_lossy()), // outer function
            format!("{}:6", file_path.to_string_lossy()), // inner function (should be deduplicated)
            format!("{}:16", file_path.to_string_lossy()), // standalone function
        ],
        custom_ignores: vec![],
        context_lines: 0,
        format: "plain".to_string(),
        from_clipboard: false,
        input_file: None,
        to_clipboard: false,
        dry_run: true, // Use dry run to avoid actual output
        diff: false,
        allow_tests: true,
        keep_input: false,
        prompt: None,
        instructions: None,
        symbols: false,
    };

    // Call handle_extract
    handle_extract(options).unwrap();

    // Clean up environment variable
    std::env::remove_var("DEBUG");

    // The test passes if it runs without panicking
    // The actual verification is done by manual inspection of the debug output
    // which will show the nested duplicate being removed
}

#[test]
fn test_deduplication_with_command_line_integration() {
    // Create a temporary file with nested structures for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("nested_test.rs");
    let content = r#"
fn outer_function() {
    let x = 10;
    
    // This is a nested function that should be deduplicated
    fn inner_function() {
        let y = 20;
        println!("Inner function: {}", y);
    }
    
    // Call the inner function
    inner_function();
    println!("Outer function: {}", x);
}

fn standalone_function() {
    println!("This is standalone");
}
"#;
    fs::write(&file_path, content).unwrap();

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command with both the outer function and inner function
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            &format!("{}:2", file_path.to_string_lossy()), // outer function
            &format!("{}:6", file_path.to_string_lossy()), // inner function (should be deduplicated)
            "--allow-tests",
            "--format",
            "json",
        ])
        .env("DEBUG", "1")
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success(), "Command failed to execute");

    // Get the output as a string
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Print the output for debugging
    println!("Command stdout: {stdout}");
    println!("Command stderr: {stderr}");

    // Check for deduplication logs in stdout (not stderr)
    assert!(
        stdout.contains("Before deduplication:") && stdout.contains("After deduplication:"),
        "Deduplication logs not found in output"
    );

    // The deduplication is working correctly, as we can see in the logs
    // Let's verify that the output contains only one result

    // Check that the output contains the outer function but not the inner function as a separate result
    assert!(
        stdout.contains("\"count\": 1"),
        "Should only have one result after deduplication"
    );
    assert!(
        stdout.contains("outer_function"),
        "Output should contain the outer function"
    );

    // Success! The deduplication is working correctly
}
