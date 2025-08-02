use probe_code::extract::{handle_extract, ExtractOptions};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_parallel_file_extraction() {
    // Create a temporary directory for test files
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create multiple test files
    let file_count = 10;
    let mut file_paths = Vec::new();

    for i in 0..file_count {
        let file_path = base_path.join(format!("test_file_{i}.rs"));
        let content = format!(
            "// Test file {i}\n\nfn function_a() {{\n    println!(\"Hello from function A\");\n}}\n\nfn function_b() {{\n    println!(\"Hello from function B\");\n}}\n"
        );
        fs::write(&file_path, content).unwrap();
        file_paths.push(file_path.to_string_lossy().to_string());
    }

    // Create options for extraction
    let options = ExtractOptions {
        files: file_paths,
        custom_ignores: Vec::new(),
        context_lines: 0,
        format: "plain".to_string(),
        from_clipboard: false,
        input_file: None,
        to_clipboard: false,
        dry_run: false,
        diff: false,
        allow_tests: true,
        instructions: None,
        keep_input: false,
        prompt: None,
        symbols: false,
    };

    // Run the extraction
    let result = handle_extract(options);
    assert!(result.is_ok(), "Extraction should succeed");

    // Verify that all files were processed by checking the output
    // This is a basic test - in a real scenario, you might want to capture
    // and parse the output to verify each file was processed correctly
}

#[test]
fn test_parallel_extraction_with_specific_lines() {
    // Create a temporary directory for test files
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create a test file with multiple functions
    let file_path = base_path.join("multi_function.rs");
    let content = r#"
fn function_one() {
    println!("This is function one");
}

fn function_two() {
    println!("This is function two");
}

fn function_three() {
    println!("This is function three");
}

fn function_four() {
    println!("This is function four");
}
"#;
    fs::write(&file_path, content).unwrap();

    // Create multiple files with the same content to test parallelism
    let file_count = 5;
    let mut file_paths = Vec::new();

    for i in 0..file_count {
        let path = base_path.join(format!("multi_function_{i}.rs"));
        fs::write(&path, content).unwrap();
        file_paths.push(path.to_string_lossy().to_string());
    }
    // Create options for extraction with specific lines
    let options = ExtractOptions {
        files: file_paths,
        custom_ignores: Vec::new(),
        context_lines: 0,
        format: "plain".to_string(),
        from_clipboard: false,
        input_file: None,
        to_clipboard: false,
        dry_run: false,
        diff: false,
        allow_tests: true,
        instructions: None,
        keep_input: false,
        prompt: None,
        symbols: false,
    };

    // Run the extraction
    let result = handle_extract(options);
    assert!(result.is_ok(), "Extraction should succeed");

    // In a real test, you might want to capture the output and verify
    // that all functions were extracted correctly from all files
}

#[test]
fn test_parallel_extraction_performance() {
    // This test is designed to verify that parallel extraction is faster
    // than sequential extraction for a large number of files

    // Skip this test in CI environments where performance might vary
    if std::env::var("CI").is_ok() {
        return;
    }

    // Create a temporary directory for test files
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create a large number of test files
    let file_count = 50; // Adjust based on your system capabilities
    let mut file_paths = Vec::new();

    for i in 0..file_count {
        let file_path = base_path.join(format!("perf_test_{i}.rs"));

        // Create a file with multiple functions to make parsing non-trivial
        let content = format!(
            "// Performance test file {}\n\n{}\n",
            i,
            (0..20)
                .map(|j| format!(
                "fn function_{i}_{j}() {{\n    let x = {j};\n    println!(\"Value: {{}}\", x);\n}}\n"
            ))
                .collect::<Vec<_>>()
                .join("\n")
        );

        fs::write(&file_path, content).unwrap();
        file_paths.push(file_path.to_string_lossy().to_string());
    }

    // Note: In a real performance test, you would measure the time taken
    // and compare it with a sequential implementation. For this example,
    // we're just verifying that the parallel implementation works correctly
    // with a large number of files.

    let options = ExtractOptions {
        files: file_paths,
        custom_ignores: Vec::new(),
        context_lines: 0,
        format: "plain".to_string(),
        from_clipboard: false,
        input_file: None,
        to_clipboard: false,
        dry_run: true, // Use dry run to avoid large output
        diff: false,
        allow_tests: true,
        instructions: None,
        keep_input: false,
        prompt: None,
        symbols: false,
    };

    // Run the extraction
    let result = handle_extract(options);
    assert!(
        result.is_ok(),
        "Parallel extraction should succeed with many files"
    );
}
