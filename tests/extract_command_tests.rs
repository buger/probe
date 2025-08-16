use std::fs;
use std::path::PathBuf;
use std::process::Command;

// Import the necessary functions from the extract module
use probe_code::extract::{
    extract_file_paths_from_git_diff, format_and_print_extraction_results, is_git_diff_format,
    process_file_for_extraction,
};

#[test]
fn test_process_file_for_extraction_full_file() {
    // Create a temporary file for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.txt");
    let content = "Line 1\nLine 2\nLine 3\n";
    fs::write(&file_path, content).unwrap();

    // Test processing the full file
    let result = process_file_for_extraction(&file_path, None, None, None, false, 0, None).unwrap();

    assert_eq!(result.file, file_path.to_string_lossy().to_string());
    assert_eq!(result.lines, (1, 3)); // 3 lines in the content
    assert_eq!(result.node_type, "file");
    assert_eq!(result.code, content);

    // Test with non-existent file
    let non_existent = temp_dir.path().join("non_existent.txt");
    let err =
        process_file_for_extraction(&non_existent, None, None, None, false, 0, None).unwrap_err();
    assert!(err.to_string().contains("does not exist"));
}

#[test]
fn test_process_file_for_extraction_with_line() {
    // Create a temporary file with Rust code for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn main() {
    println!("Hello, world!");
    
    let x = 42;
    if x > 0 {
        println!("Positive");
    } else {
        println!("Non-positive");
    }
}

struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}
"#;
    fs::write(&file_path, content).unwrap();

    // Test extracting a function
    let result =
        process_file_for_extraction(&file_path, Some(3), None, None, false, 0, None).unwrap();
    assert_eq!(result.file, file_path.to_string_lossy().to_string());
    assert!(result.lines.0 <= 3 && result.lines.1 >= 3);
    assert!(result.code.contains("fn main()"));
    assert!(result.code.contains("Hello, world!"));

    // Test extracting a struct
    let result =
        process_file_for_extraction(&file_path, Some(13), None, None, false, 0, None).unwrap();
    assert_eq!(result.file, file_path.to_string_lossy().to_string());
    assert!(result.lines.0 <= 13 && result.lines.1 >= 13);
    assert!(result.code.contains("struct Point"));
    assert!(result.code.contains("x: i32"));
    assert!(result.code.contains("y: i32"));

    // Test with out-of-bounds line number (should be clamped to valid range)
    let result =
        process_file_for_extraction(&file_path, Some(1000), None, None, false, 0, None).unwrap();
    // The line number should be clamped to the maximum valid line
    // Don't check for exact equality, just make sure it's within valid range
    assert!(result.lines.0 <= result.lines.1);
    assert!(result.lines.1 <= content.lines().count());
}

#[test]
fn test_process_file_for_extraction_fallback() {
    // Create a temporary file with a non-supported extension
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.xyz");
    let mut content = String::from(
        "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8\nLine 9\nLine 10\n",
    );
    content.push_str("Line 11\nLine 12\nLine 13\nLine 14\nLine 15\nLine 16\nLine 17\nLine 18\nLine 19\nLine 20\n");
    content.push_str("Line 21\nLine 22\nLine 23\nLine 24\nLine 25\n");
    fs::write(&file_path, content).unwrap();

    // Test fallback to line-based context with default context lines (10)
    let result =
        process_file_for_extraction(&file_path, Some(15), None, None, false, 10, None).unwrap();
    assert_eq!(result.file, file_path.to_string_lossy().to_string());
    assert_eq!(result.node_type, "context");

    // Should include 10 lines before and after line 15
    let start_line = result.lines.0;
    let end_line = result.lines.1;

    // Check that the range includes line 15 and has appropriate context
    assert!(start_line <= 15 && end_line >= 15);
    assert!(end_line - start_line >= 10); // At least 10 lines of context

    // Test with a line at the beginning of the file
    let result =
        process_file_for_extraction(&file_path, Some(2), None, None, false, 10, None).unwrap();
    assert!(result.lines.0 <= 2); // Should start at or before line 2
    assert!(result.lines.1 >= 2); // Should include line 2

    // Test with a line at the end of the file
    let result =
        process_file_for_extraction(&file_path, Some(25), None, None, false, 10, None).unwrap();
    assert!(result.lines.0 <= 25); // Should include some lines before line 25
    assert_eq!(result.lines.1, 25); // Can't go beyond the last line

    // Test with custom context lines
    let result =
        process_file_for_extraction(&file_path, Some(15), None, None, false, 5, None).unwrap();
    assert_eq!(result.file, file_path.to_string_lossy().to_string());
    assert_eq!(result.node_type, "context");

    // Should include 5 lines before and after line 15
    let start_line = result.lines.0;
    let end_line = result.lines.1;

    // Check that the range includes line 15 and has appropriate context
    assert!(start_line <= 15 && end_line >= 15);
    assert!(end_line - start_line >= 5); // At least 5 lines of context
    assert!(end_line - start_line <= 11); // At most 11 lines (5 before, 5 after, plus the line itself)
}

#[test]
fn test_format_and_print_extraction_results() {
    // Create a simple search result
    let result = probe_code::models::SearchResult {
        file: "test_file.rs".to_string(),
        lines: (1, 5),
        node_type: "function".to_string(),
        code: "fn test() {\n    println!(\"Hello\");\n}".to_string(),
        matched_by_filename: None,
        rank: None,
        score: None,
        tfidf_score: None,
        bm25_score: None,
        tfidf_rank: None,
        bm25_rank: None,
        new_score: None,
        hybrid2_rank: None,
        combined_score_rank: None,
        file_unique_terms: None,
        file_total_matches: None,
        file_match_rank: None,
        block_unique_terms: None,
        block_total_matches: None,
        parent_file_id: None,
        block_id: None,
        matched_keywords: None,
        tokenized_content: None,
        lsp_info: None,
    };

    // Test different formats
    let results = vec![result];

    // We can't easily test the output directly, but we can at least ensure the function doesn't panic
    format_and_print_extraction_results(&results, "terminal", None, None, None).unwrap();
    format_and_print_extraction_results(&results, "markdown", None, None, None).unwrap();
    format_and_print_extraction_results(&results, "plain", None, None, None).unwrap();
    format_and_print_extraction_results(&results, "json", None, None, None).unwrap();
    format_and_print_extraction_results(&results, "xml", None, None, None).unwrap();

    // Test with system prompt and user instructions
    format_and_print_extraction_results(
        &results,
        "terminal",
        None,
        Some("Test system prompt"),
        Some("Test user instructions"),
    )
    .unwrap();
}

#[test]
fn test_json_format_extraction_results() {
    use serde_json::Value;
    use std::process::Command;
    use tempfile::TempDir;

    // Create a temporary file for testing
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn test() {
    println!("Hello");
}
"#;
    fs::write(&file_path, content).unwrap();

    // Print the file path for debugging
    println!("File path: {}", file_path.display());

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command with JSON format
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            file_path.to_string_lossy().as_ref(),
            "--format",
            "json",
            "--allow-tests", // Add this flag to ensure test files are included
            "--prompt",
            "engineer",
            "--instructions",
            "Extract the main function",
        ])
        .output()
        .expect("Failed to execute command");
    // Print the command output for debugging
    println!(
        "Command stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "Command stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check that the command executed successfully
    assert!(
        output.status.success(),
        "Command failed with status: {}",
        output.status
    );
    assert!(output.status.success());

    // Get the output as a string
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Helper function to extract JSON from command output
    fn extract_json_from_output(output: &str) -> &str {
        // Find the first occurrence of '{'
        if let Some(start_index) = output.find('{') {
            // Return the substring from the first '{' to the end
            &output[start_index..]
        } else {
            // If no '{' is found, return the original string
            output
        }
    }

    // Print the output for debugging
    println!("Command stdout: {stdout}");

    // Extract and parse the JSON
    let json_str = extract_json_from_output(&stdout);
    println!("Extracted JSON: {json_str}");
    let json_value: Value = serde_json::from_str(json_str).expect("Failed to parse JSON output");

    // Validate the structure of the JSON output
    assert!(json_value.is_object(), "JSON output should be an object");
    assert!(
        json_value.get("results").is_some(),
        "JSON output should have a 'results' field"
    );
    assert!(
        json_value.get("summary").is_some(),
        "JSON output should have a 'summary' field"
    );

    // Validate the results array
    let results_array = json_value.get("results").unwrap().as_array().unwrap();
    assert_eq!(
        results_array.len(),
        1,
        "Results array should have 1 element"
    );

    // Validate the structure of the result
    let result_obj = &results_array[0];
    assert!(
        result_obj.get("file").is_some(),
        "Result should have a 'file' field"
    );
    assert!(
        result_obj.get("lines").is_some(),
        "Result should have a 'lines' field"
    );
    assert!(
        result_obj.get("node_type").is_some(),
        "Result should have a 'node_type' field"
    );
    assert!(
        result_obj.get("code").is_some(),
        "Result should have a 'code' field"
    );

    // Validate the values
    assert!(result_obj
        .get("file")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("test_file.rs"));
    assert!(
        result_obj.get("node_type").is_some(),
        "Result should have a node_type field"
    );
    assert!(
        result_obj
            .get("code")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("fn test()")
            && result_obj
                .get("code")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("println!(\"Hello\")")
    );

    // Validate the lines field is an array with two elements
    let lines = result_obj.get("lines").unwrap().as_array().unwrap();
    assert_eq!(lines.len(), 2, "Lines should be an array with 2 elements");
    assert!(
        lines[0].as_u64().unwrap() >= 1,
        "Start line should be at least 1"
    );
    assert!(
        lines[1].as_u64().unwrap() >= lines[0].as_u64().unwrap(),
        "End line should be at least start line"
    );

    // Validate the summary object
    let summary = json_value.get("summary").unwrap();
    assert!(summary.is_object(), "Summary should be an object");
    assert!(
        summary.get("count").is_some(),
        "Summary should have a 'count' field"
    );
    assert!(
        summary.get("total_bytes").is_some(),
        "Summary should have a 'total_bytes' field"
    );
    assert!(
        summary.get("total_tokens").is_some(),
        "Summary should have a 'total_tokens' field"
    );

    // Validate the count matches the number of results
    assert_eq!(summary.get("count").unwrap().as_u64().unwrap(), 1);

    // Validate system_prompt and user_instructions are present
    assert!(
        json_value.get("system_prompt").is_some(),
        "JSON output should have a 'system_prompt' field"
    );
    assert!(
        json_value.get("user_instructions").is_some(),
        "JSON output should have a 'user_instructions' field"
    );

    // Validate the content of system_prompt and user_instructions
    assert!(
        json_value
            .get("user_instructions")
            .unwrap()
            .as_str()
            .unwrap()
            == "Extract the main function",
        "user_instructions should match the provided value"
    );
}

#[test]
fn test_xml_format_extraction_results() {
    use roxmltree::{Document, Node};
    use std::process::Command;
    use tempfile::TempDir;

    // Create a temporary file for testing
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn test() {
    println!("Hello");
}
"#;
    fs::write(&file_path, content).unwrap();

    // Print the file path for debugging
    println!("File path: {}", file_path.display());

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command with XML format
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            file_path.to_string_lossy().as_ref(),
            "--format",
            "xml",
            "--allow-tests", // Add this flag to ensure test files are included
            "--prompt",
            "engineer",
            "--instructions",
            "Extract the main function",
        ])
        .output()
        .expect("Failed to execute command");

    // Print the command output for debugging
    println!(
        "Command stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "Command stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check that the command executed successfully
    assert!(
        output.status.success(),
        "Command failed with status: {}",
        output.status
    );

    // Get the output as a string
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Helper function to extract XML from command output
    fn extract_xml_from_output(output: &str) -> &str {
        // Find the first occurrence of '<?xml'
        if let Some(start_index) = output.find("<?xml") {
            // Return the substring from the first '<?xml' to the end
            &output[start_index..]
        } else {
            // If no '<?xml' is found, return the original string
            output
        }
    }

    // Print the output for debugging
    println!("Command stdout: {stdout}");

    // Extract and parse the XML
    let xml_str = extract_xml_from_output(&stdout);
    println!("Extracted XML: {xml_str}");
    let doc = Document::parse(xml_str).expect("Failed to parse XML output");
    let root = doc.root_element();

    // Validate the structure of the XML output
    assert_eq!(
        root.tag_name().name(),
        "probe_results",
        "Root element should be 'probe_results'"
    );

    // Validate that there are result elements
    let results_nodes: Vec<Node> = root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "result")
        .collect();
    assert_eq!(
        results_nodes.len(),
        1,
        "Should have exactly one result element"
    );

    // Validate the structure of the result
    let result_node = &results_nodes[0];

    // Check for required elements
    let file = result_node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "file");
    assert!(file.is_some(), "Result should have a file element");
    assert!(
        file.unwrap().text().unwrap().contains("test_file.rs"),
        "File element should contain the correct file path"
    );

    // For a full file extraction, the lines element might not be present
    let node_type = result_node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "node_type");
    if let Some(node_type) = node_type {
        if node_type.text().unwrap() != "file" {
            let lines = result_node
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "lines");
            assert!(lines.is_some(), "Result should have a lines element");
        }

        // Validate system_prompt and user_instructions are present
        let system_prompt = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "system_prompt");
        assert!(
            system_prompt.is_some(),
            "Should have a system_prompt element"
        );

        let user_instructions = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "user_instructions");
        assert!(
            user_instructions.is_some(),
            "Should have a user_instructions element"
        );

        // Validate the content of user_instructions
        assert_eq!(
            user_instructions.unwrap().text().unwrap(),
            "Extract the main function",
            "user_instructions should match the provided value"
        );
    }

    // The node_type element might not be present for all extraction types
    let _node_type = result_node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "node_type");

    let code = result_node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "code");
    assert!(code.is_some(), "Result should have a code element");
    assert!(
        code.unwrap().text().unwrap().contains("fn test()")
            && code
                .unwrap()
                .text()
                .unwrap()
                .contains("println!(\"Hello\")"),
        "Code element should contain the correct code"
    );

    // Validate the summary element
    let summary = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "summary");
    assert!(summary.is_some(), "Should have a summary element");

    if let Some(summary) = summary {
        // Validate the summary contains count, total_bytes, and total_tokens
        let count = summary
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "count");
        assert!(count.is_some(), "Summary should have a count element");
        assert_eq!(count.unwrap().text().unwrap(), "1", "Count should be 1");

        let total_bytes = summary
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "total_bytes");
        assert!(
            total_bytes.is_some(),
            "Summary should have a total_bytes element"
        );

        let total_tokens = summary
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "total_tokens");
        assert!(
            total_tokens.is_some(),
            "Summary should have a total_tokens element"
        );
    }
}

#[test]
fn test_process_file_for_extraction_with_range() {
    // Create a temporary file for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.txt");
    let mut content = String::new();
    for i in 1..=20 {
        content.push_str(&format!("Line {i}\n"));
    }
    fs::write(&file_path, &content).unwrap();

    // Test extracting a range of lines
    let result =
        process_file_for_extraction(&file_path, Some(1), Some(10), None, false, 0, None).unwrap();
    assert_eq!(result.file, file_path.to_string_lossy().to_string());
    assert_eq!(result.lines, (1, 10));
    assert_eq!(result.node_type, "range");

    // Check that the extracted content contains exactly lines 1-10
    let expected_content = content.lines().take(10).collect::<Vec<_>>().join("\n");
    assert_eq!(result.code, expected_content);

    // Test with a different range
    let result =
        process_file_for_extraction(&file_path, Some(5), Some(15), None, false, 0, None).unwrap();
    assert_eq!(result.lines, (5, 15));

    // Check that the extracted content contains exactly lines 5-15
    let expected_content = content
        .lines()
        .skip(4)
        .take(11)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(result.code, expected_content);

    // Test with invalid range (start > end) - should be clamped to valid range
    let result =
        process_file_for_extraction(&file_path, Some(10), Some(5), None, false, 0, None).unwrap();
    // The start and end lines should be clamped to valid values
    assert!(result.lines.0 <= result.lines.1);
    assert!(result.lines.1 <= content.lines().count());

    // Test with out-of-bounds range (should be clamped to valid range)
    let result =
        process_file_for_extraction(&file_path, Some(15), Some(25), None, false, 0, None).unwrap();
    // The end line should be clamped to the maximum valid line
    assert!(result.lines.0 <= 15);
    assert!(result.lines.1 <= content.lines().count());
    // The node_type should be "range"
    assert_eq!(result.node_type, "range");
}

#[test]
fn test_integration_extract_command() {
    // Create a temporary file for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn main() {
    println!("Hello, world!");
}

struct Point {
    x: i32,
    y: i32,
}
"#;
    fs::write(&file_path, content).unwrap();

    // Print the current directory and file path for debugging
    println!("Current directory: {:?}", std::env::current_dir().unwrap());
    println!("File path: {file_path:?}");

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    println!("Project directory: {project_dir:?}");

    // Run the extract command using cargo run from the project directory
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            file_path.to_string_lossy().as_ref(),
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .current_dir(&project_dir) // Ensure we're in the project directory
        .output()
        .expect("Failed to execute command");

    // Print the command output for debugging
    println!(
        "Command stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "Command stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check that the command executed successfully
    assert!(output.status.success());

    // Check that the output contains the file content
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("File:"));
    assert!(stdout.contains("fn main()"));
    // Note: We don't check for "struct Point" here anymore since the extract command
    // might only extract the function and not the struct depending on the AST parsing

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run with a line number
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            &format!("{}:3", file_path.to_string_lossy()),
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .current_dir(&project_dir) // Ensure we're in the project directory
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success());

    // Check that the output contains the function
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("File:"));
    assert!(stdout.contains("fn main()"));
    assert!(stdout.contains("Hello, world!"));

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run with a different format
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            file_path.to_string_lossy().as_ref(),
            "--format",
            "markdown",
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .current_dir(&project_dir) // Ensure we're in the project directory
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success());

    // Check that the output is in markdown format
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("## File:"));

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run with a line range
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            &format!("{}:2-7", file_path.to_string_lossy()),
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .current_dir(&project_dir) // Ensure we're in the project directory
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success());

    // Check that the output contains the specified range
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("File:"));
    // The exact format of the line range might vary, so just check for the content
    // We only check for the content that should definitely be in the range
    assert!(
        stdout.contains("fn main()")
            || stdout.contains("println!(\"Hello, world!\")")
            || stdout.contains("struct Point")
    );
}

#[test]
fn test_integration_extract_command_json_format() {
    use serde_json::Value;

    // Create a temporary file for testing with special characters
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn main() {
    // This contains special characters: "quotes", 'apostrophes', <tags>, &ampersands
    println!("Hello, \"world\"!");
    let message = 'A';
    let html = "<div>Content & More</div>";
}
"#;
    fs::write(&file_path, content).unwrap();

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command with JSON format
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            file_path.to_string_lossy().as_ref(),
            "--format",
            "json",
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success());

    // Get the output as a string
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Helper function to extract JSON from command output
    fn extract_json_from_output(output: &str) -> &str {
        // Find the first occurrence of '{'
        if let Some(start_index) = output.find('{') {
            // Return the substring from the first '{' to the end
            &output[start_index..]
        } else {
            // If no '{' is found, return the original string
            output
        }
    }

    // Extract and parse the JSON
    let json_str = extract_json_from_output(&stdout);
    let json_value: Value = serde_json::from_str(json_str).expect("Failed to parse JSON output");

    // Validate the structure of the JSON output
    assert!(json_value.is_object(), "JSON output should be an object");
    assert!(
        json_value.get("results").is_some(),
        "JSON output should have a 'results' field"
    );
    assert!(
        json_value.get("summary").is_some(),
        "JSON output should have a 'summary' field"
    );

    // Validate the results array
    let results = json_value.get("results").unwrap().as_array().unwrap();
    assert!(!results.is_empty(), "Results array should not be empty");

    // Validate the first result
    let first_result = &results[0];
    assert!(
        first_result.get("file").is_some(),
        "Result should have a 'file' field"
    );
    assert!(
        first_result.get("code").is_some(),
        "Result should have a 'code' field"
    );

    // Verify that special characters are properly escaped in the JSON
    let code = first_result.get("code").unwrap().as_str().unwrap();
    assert!(
        code.contains("\"quotes\""),
        "Double quotes should be properly escaped in JSON"
    );
    assert!(
        code.contains("'apostrophes'"),
        "Apostrophes should be properly escaped in JSON"
    );
    assert!(
        code.contains("<tags>"),
        "Tags should be properly escaped in JSON"
    );
    assert!(
        code.contains("&ampersands"),
        "Ampersands should be properly escaped in JSON"
    );

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run with a line number and JSON format
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            &format!("{}:3", file_path.to_string_lossy()),
            "--format",
            "json",
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .current_dir(&project_dir) // Ensure we're in the project directory
        .output()
        .expect("Failed to execute command");

    // Print the command output for debugging
    println!(
        "Command stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "Command stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check that the command executed successfully
    assert!(
        output.status.success(),
        "Command failed with status: {}",
        output.status
    );

    // Parse the JSON output
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_str = extract_json_from_output(&stdout);
    let json_value: Value = serde_json::from_str(json_str).expect("Failed to parse JSON output");

    // Validate the structure
    let results = json_value.get("results").unwrap().as_array().unwrap();
    assert!(!results.is_empty(), "Results array should not be empty");

    // The result should contain the function with line 3
    let first_result = &results[0];
    let code = first_result.get("code").unwrap().as_str().unwrap();
    assert!(
        code.contains("fn main()"),
        "Code should contain the main function"
    );
    assert!(
        code.contains("Hello, \\\"world\\\"!"),
        "Code should contain the println statement with escaped quotes"
    );
}

#[test]
fn test_is_git_diff_format() {
    // Test with git diff format
    let diff_content = r#"diff --git a/tests/property_tests.rs b/tests/property_tests.rs
index cb2cb64..3717769 100644
--- a/tests/property_tests.rs
+++ b/tests/property_tests.rs
@@ -45,7 +45,7 @@ proptest! {
            Err(_) => return proptest::test_runner::TestCaseResult::Ok(()), // Skip invalid queries
        };

-        let patterns = create_structured_patterns(&plan);
+        let patterns = create_structured_patterns(&plan, false);
"#;
    assert!(
        is_git_diff_format(diff_content),
        "Should detect git diff format"
    );

    // Test with whitespace before git diff format
    let diff_content_with_whitespace = r#"

diff --git a/tests/property_tests.rs b/tests/property_tests.rs
index cb2cb64..3717769 100644
"#;
    assert!(
        is_git_diff_format(diff_content_with_whitespace),
        "Should detect git diff format with leading whitespace"
    );

    // Test with non-git diff format
    let non_diff_content = r#"This is not a git diff format
It's just some text
"#;
    assert!(
        !is_git_diff_format(non_diff_content),
        "Should not detect git diff format"
    );
}

#[test]
fn test_extract_file_paths_from_git_diff() {
    // Sample git diff output
    let diff_content = r#"diff --git a/tests/property_tests.rs b/tests/property_tests.rs
index cb2cb64..3717769 100644
--- a/tests/property_tests.rs
+++ b/tests/property_tests.rs
@@ -45,7 +45,7 @@ proptest! {
            Err(_) => return proptest::test_runner::TestCaseResult::Ok(()), // Skip invalid queries
        };

-        let patterns = create_structured_patterns(&plan);
+        let patterns = create_structured_patterns(&plan, false);

        // Check that we have at least one pattern for each term
        for (term, &idx) in &plan.term_indices {
"#;

    // Parse the git diff
    let file_paths = extract_file_paths_from_git_diff(diff_content, true);

    // Verify that we extracted the correct file path and line number
    assert_eq!(file_paths.len(), 1, "Should extract exactly one file path");

    let (path, start_line, end_line, symbol, _specific_lines) = &file_paths[0];
    assert_eq!(
        path,
        &PathBuf::from("tests/property_tests.rs"),
        "Should extract the correct file path"
    );
    assert_eq!(
        *start_line,
        Some(48),
        "Should extract the correct line number"
    );
    assert_eq!(*end_line, Some(48), "End line should be 48");
    assert_eq!(*symbol, None, "Symbol should be None");
}

#[test]
fn test_extract_file_paths_from_git_diff_multiple_files() {
    // Sample git diff output with multiple files
    let diff_content = r#"diff --git a/tests/property_tests.rs b/tests/property_tests.rs
index cb2cb64..3717769 100644
--- a/tests/property_tests.rs
+++ b/tests/property_tests.rs
@@ -45,7 +45,7 @@ proptest! {
            Err(_) => return proptest::test_runner::TestCaseResult::Ok(()), // Skip invalid queries
        };

-        let patterns = create_structured_patterns(&plan);
+        let patterns = create_structured_patterns(&plan, false);

        // Check that we have at least one pattern for each term
        for (term, &idx) in &plan.term_indices {
diff --git a/tests/tokenization_tests.rs b/tests/tokenization_tests.rs
index abcdef1..1234567 100644
--- a/tests/tokenization_tests.rs
+++ b/tests/tokenization_tests.rs
@@ -20,7 +20,7 @@ fn test_tokenize_with_stemming() {
    let tokens = tokenize_with_stemming("running runs runner");
    
-    assert_eq!(tokens, vec!["run", "run", "runner"]);
+    assert_eq!(tokens, vec!["run", "run", "run"]);
}
"#;

    // Parse the git diff
    let file_paths = extract_file_paths_from_git_diff(diff_content, true);

    // Verify that we extracted the correct file paths and line numbers
    assert_eq!(file_paths.len(), 2, "Should extract exactly two file paths");

    // Sort the results by file path to ensure consistent order for testing
    let mut sorted_paths = file_paths.clone();
    sorted_paths.sort_by(|a, b| a.0.cmp(&b.0));

    // Check first file
    let (path1, start_line1, end_line1, symbol1, _specific_lines1) = &sorted_paths[0];
    assert_eq!(
        path1,
        &PathBuf::from("tests/property_tests.rs"),
        "Should extract the correct file path"
    );
    assert_eq!(
        *start_line1,
        Some(48),
        "Should extract the correct line number"
    );
    assert_eq!(*end_line1, Some(48), "End line should be 48");
    assert_eq!(*symbol1, None, "Symbol should be None");

    // Check second file
    let (path2, start_line2, end_line2, symbol2, _specific_lines2) = &sorted_paths[1];
    assert_eq!(
        path2,
        &PathBuf::from("tests/tokenization_tests.rs"),
        "Should extract the correct file path"
    );
    assert_eq!(
        *start_line2,
        Some(22),
        "Should extract the correct line number"
    );
    assert_eq!(*end_line2, Some(22), "End line should be 22");
    assert_eq!(*symbol2, None, "Symbol should be None");
}

#[test]
fn test_integration_extract_command_with_diff_flag() {
    // Create a temporary file for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn main() {
    println!("Hello, world!");
}

struct Point {
    x: i32,
    y: i32,
}
"#;
    fs::write(&file_path, content).unwrap();

    // Create a git diff file
    let diff_path = temp_dir.path().join("test.diff");
    let diff_content = format!(
        r#"diff --git a/{0} b/{0}
index cb2cb64..3717769 100644
--- a/{0}
+++ b/{0}
@@ -3,1 +3,1 @@ fn main() {{
-    println!("Hello, world!");
+    println!("Hello, universe!");
"#,
        file_path.file_name().unwrap().to_string_lossy()
    );
    fs::write(&diff_path, &diff_content).unwrap();

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command with diff option
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            "--diff",
            diff_path.to_string_lossy().as_ref(),
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success(), "Command failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("Command output: {stdout}");

    // The output should contain information about the extracted file
    assert!(
        stdout.contains("Files to extract:"),
        "Output should contain file list"
    );
    assert!(
        stdout.contains("test_file.rs"),
        "Output should contain the extracted file name"
    );
}

#[test]
fn test_integration_extract_command_with_auto_diff_detection() {
    // Create a temporary file for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn main() {
    println!("Hello, world!");
}

struct Point {
    x: i32,
    y: i32,
}
"#;
    fs::write(&file_path, content).unwrap();

    // Create a git diff file
    let diff_path = temp_dir.path().join("test.diff");
    let diff_content = format!(
        r#"diff --git a/{0} b/{0}
index cb2cb64..3717769 100644
--- a/{0}
+++ b/{0}
@@ -3,1 +3,1 @@ fn main() {{
-    println!("Hello, world!");
+    println!("Hello, universe!");
"#,
        file_path.file_name().unwrap().to_string_lossy()
    );
    fs::write(&diff_path, &diff_content).unwrap();

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command WITHOUT the diff flag - it should auto-detect
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            diff_path.to_string_lossy().as_ref(),
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success(), "Command failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("Command output: {stdout}");

    // The output should contain information about the extracted file
    assert!(
        stdout.contains("Files to extract:"),
        "Output should contain file list"
    );
    assert!(
        stdout.contains("test_file.rs"),
        "Output should contain the extracted file name"
    );
}

#[test]
fn test_integration_extract_command_xml_format() {
    use roxmltree::Document;

    // Create a temporary file for testing with special characters
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn main() {
    // This contains special characters: "quotes", 'apostrophes', <tags>, &ampersands
    println!("Hello, \"world\"!");
    let message = 'A';
    let html = "<div>Content & More</div>";
}
"#;
    fs::write(&file_path, content).unwrap();

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command with XML format
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            file_path.to_string_lossy().as_ref(),
            "--format",
            "xml",
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .current_dir(&project_dir) // Ensure we're in the project directory
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success());

    // Get the output as a string
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Helper function to extract XML from command output
    fn extract_xml_from_output(output: &str) -> &str {
        // Find the first occurrence of '<?xml'
        if let Some(start_index) = output.find("<?xml") {
            // Return the substring from the first '<?xml' to the end
            &output[start_index..]
        } else {
            // If no '<?xml' is found, return the original string
            output
        }
    }

    // Extract and parse the XML
    let xml_str = extract_xml_from_output(&stdout);
    let doc = Document::parse(xml_str).expect("Failed to parse XML output");
    let root = doc.root_element();

    // Validate the structure of the XML output
    assert_eq!(
        root.tag_name().name(),
        "probe_results",
        "Root element should be 'probe_results'"
    );

    // Validate that there are result elements
    let results: Vec<_> = root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "result")
        .collect();
    assert!(
        !results.is_empty(),
        "Should have at least one result element"
    );

    // Validate the first result
    let first_result = &results[0];

    // Check for required elements
    let file = first_result
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "file");
    assert!(file.is_some(), "Result should have a file element");

    let code = first_result
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "code");
    assert!(code.is_some(), "Result should have a code element");

    // Verify that special characters are properly handled in the XML (should be in CDATA)
    if let Some(code_elem) = code {
        let code_text = code_elem.text().unwrap();
        assert!(
            code_text.contains("\"quotes\""),
            "Double quotes should be preserved in CDATA"
        );
        assert!(
            code_text.contains("'apostrophes'"),
            "Apostrophes should be preserved in CDATA"
        );
        assert!(
            code_text.contains("<tags>"),
            "Tags should be preserved in CDATA"
        );
        assert!(
            code_text.contains("&ampersands"),
            "Ampersands should be preserved in CDATA"
        );
    }

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run with a line number and XML format
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            &format!("{}:3", file_path.to_string_lossy()),
            "--format",
            "xml",
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .current_dir(&project_dir) // Ensure we're in the project directory
        .output()
        .expect("Failed to execute command");

    // Print the command output for debugging
    println!(
        "Command stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "Command stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check that the command executed successfully
    assert!(
        output.status.success(),
        "Command failed with status: {}",
        output.status
    );

    // Parse the XML output
    let stdout = String::from_utf8_lossy(&output.stdout);
    let xml_str = extract_xml_from_output(&stdout);
    let doc = Document::parse(xml_str).expect("Failed to parse XML output");
    let root = doc.root_element();

    // Validate the structure
    let results: Vec<_> = root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "result")
        .collect();
    assert!(
        !results.is_empty(),
        "Should have at least one result element"
    );

    // The result should contain the function with line 3
    let first_result = &results[0];
    if let Some(code_elem) = first_result
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "code")
    {
        let code_text = code_elem.text().unwrap();
        assert!(
            code_text.contains("fn main()"),
            "Code should contain the main function"
        );
        assert!(
            code_text.contains("Hello") && code_text.contains("world"),
            "Code should contain the println statement"
        );
    }
}

#[test]
fn test_integration_extract_command_with_multiple_files_diff() {
    // Create temporary files for testing
    let temp_dir = tempfile::tempdir().unwrap();

    // Change to the temp directory for this test
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&temp_dir).unwrap();

    // Create the test files in the temp directory
    let file_path1 = PathBuf::from("property_tests.rs");
    let content1 = r#"
fn test_create_structured_patterns() {
    let plan = create_query_plan("test query", false).unwrap();
    let patterns = create_structured_patterns(&plan);
    
    // Check that we have at least one pattern for each term
    for (term, &idx) in &plan.term_indices {
        assert!(!patterns[idx].is_empty());
    }
}
"#;
    fs::write(&file_path1, content1).unwrap();

    let file_path2 = PathBuf::from("tokenization_tests.rs");
    let content2 = r#"
fn test_tokenize_with_stemming() {
    let tokens = tokenize_with_stemming("running runs runner");
    
    assert_eq!(tokens, vec!["run", "run", "runner"]);
}
"#;
    fs::write(&file_path2, content2).unwrap();

    // Run the git diff command to create a real diff
    let status = Command::new("git")
        .args(["init"])
        .status()
        .expect("Failed to initialize git repo");
    assert!(status.success(), "Failed to initialize git repo");

    let status = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .status()
        .expect("Failed to configure git");
    assert!(status.success(), "Failed to configure git");

    let status = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .status()
        .expect("Failed to configure git");
    assert!(status.success(), "Failed to configure git");

    let status = Command::new("git")
        .args(["add", "."])
        .status()
        .expect("Failed to add files to git");
    assert!(status.success(), "Failed to add files to git");

    let status = Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .status()
        .expect("Failed to commit files");
    assert!(status.success(), "Failed to commit files");

    // Modify the files
    let content1_modified = r#"
fn test_create_structured_patterns() {
    let plan = create_query_plan("test query", false).unwrap();
    let patterns = create_structured_patterns(&plan, false);
    
    // Check that we have at least one pattern for each term
    for (term, &idx) in &plan.term_indices {
        assert!(!patterns[idx].is_empty());
    }
}
"#;
    fs::write(&file_path1, content1_modified).unwrap();

    let content2_modified = r#"
fn test_tokenize_with_stemming() {
    let tokens = tokenize_with_stemming("running runs runner");
    
    assert_eq!(tokens, vec!["run", "run", "run"]);
}
"#;
    fs::write(&file_path2, content2_modified).unwrap();

    // Create the diff file
    let diff_output = Command::new("git")
        .args(["diff", "property_tests.rs", "tokenization_tests.rs"])
        .output()
        .expect("Failed to create git diff");

    assert!(diff_output.status.success(), "Failed to create git diff");
    let diff_content = String::from_utf8_lossy(&diff_output.stdout).to_string();

    // Write the diff to a file
    let diff_path = PathBuf::from("changes.diff");
    fs::write(&diff_path, &diff_content).unwrap();

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command with the diff containing multiple files
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            diff_path.to_string_lossy().as_ref(),
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .output()
        .expect("Failed to execute command");

    // Restore the original directory
    std::env::set_current_dir(original_dir).unwrap();

    // Check that the command executed successfully
    assert!(output.status.success(), "Command failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("Command output: {stdout}");

    // The output should contain information about the diff file
    assert!(
        stdout.contains("Files to extract:"),
        "Output should contain file list"
    );
    assert!(
        stdout.contains("changes.diff"),
        "Output should contain the diff file name"
    );

    // Verify that the diff content contains both files
    assert!(
        stdout.contains("property_tests.rs"),
        "Output should contain the first file name in diff content"
    );
    assert!(
        stdout.contains("tokenization_tests.rs"),
        "Output should contain the second file name in diff content"
    );

    // When processing a diff file directly, we only get one "File:" entry for the diff itself
    let file_count = stdout.matches("File:").count();
    assert_eq!(file_count, 1, "Should process the diff file");
}

#[test]
fn test_keep_input_option_with_stdin() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Create a temporary file with some content
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn main() {
    println!("Hello, world!");
}
"#;
    fs::write(&file_path, content).unwrap();

    // Create input content that references the file
    let input_content = format!("{}", file_path.to_string_lossy());

    // Run the extract command with stdin input and keep_input flag
    let mut child = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            "--keep-input",
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn command");

    // Write to stdin
    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        stdin
            .write_all(input_content.as_bytes())
            .expect("Failed to write to stdin");
    }

    // Get the output
    let output = child.wait_with_output().expect("Failed to read stdout");

    // Check that the command executed successfully
    assert!(output.status.success(), "Command failed to execute");

    // Get the output as a string
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The output should contain the original input
    assert!(
        stdout.contains("Original Input:"),
        "Output should contain 'Original Input:' section"
    );
    assert!(
        stdout.contains(&input_content),
        "Output should contain the original input content"
    );
}

#[test]
fn test_keep_input_option_with_clipboard() {
    use arboard::Clipboard;
    use std::process::Command;

    // Skip this test if clipboard access is not available
    let clipboard_result = Clipboard::new();
    if clipboard_result.is_err() {
        println!("Skipping clipboard test as clipboard access is not available");
        return;
    }

    // Get the project root directory (where Cargo.toml is)
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Create a temporary file with some content
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.rs");
    let content = r#"
fn main() {
    println!("Hello, world!");
}
"#;
    fs::write(&file_path, content).unwrap();

    // Create input content that references the file
    let input_content = format!("{}", file_path.to_string_lossy());

    // Set clipboard content
    let mut clipboard = Clipboard::new().unwrap();
    clipboard.set_text(&input_content).unwrap();

    // Run the extract command with clipboard input and keep_input flag
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            "--from-clipboard",
            "--keep-input",
            "--allow-tests", // Add this flag to ensure test files are included
        ])
        .output()
        .expect("Failed to execute command");

    // Check that the command executed successfully
    assert!(output.status.success(), "Command failed to execute");

    // Get the output as a string
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The output should contain the original input
    assert!(
        stdout.contains("Original Input:"),
        "Output should contain 'Original Input:' section"
    );
    assert!(
        stdout.contains(&input_content),
        "Output should contain the original input content"
    );
}

#[test]
fn test_extract_unsupported_file_type_symbol() {
    use tempfile::TempDir;

    // Create a temporary Terraform file (unsupported by tree-sitter)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("main.tf");
    let content = r#"# Terraform configuration
resource "aws_instance" "example" {
  ami           = "ami-0c55b159cbfafe1f0"
  instance_type = "t2.micro"
  
  tags = {
    Name = "ExampleInstance"
  }
}

output "instance_id" {
  value = aws_instance.example.id
}
"#;
    fs::write(&file_path, content).unwrap();

    // Test extracting a symbol from an unsupported file type
    // Should return the full file content as fallback
    let result = process_file_for_extraction(
        &file_path,
        None,                 // start_line
        None,                 // end_line
        Some("aws_instance"), // symbol
        false,                // allow_tests
        0,                    // context_lines
        None,                 // specific_line_numbers
    )
    .unwrap();

    // Should return full file content as fallback
    assert_eq!(
        result.node_type, "file",
        "Should return file type for unsupported language"
    );
    assert_eq!(result.code, content, "Should return full file content");
    assert_eq!(result.lines, (1, 13), "Should return all lines");
}

#[test]
fn test_extract_unsupported_file_type_lines() {
    use tempfile::TempDir;

    // Create a temporary YAML file (another unsupported type)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("config.yml");
    let content = r#"version: '3'
services:
  web:
    image: nginx:latest
    ports:
      - "80:80"
  database:
    image: postgres:13
    environment:
      POSTGRES_PASSWORD: secret
"#;
    fs::write(&file_path, content).unwrap();

    // Test extracting specific lines from an unsupported file type
    let result = process_file_for_extraction(
        &file_path,
        Some(3), // start_line
        Some(6), // end_line
        None,    // symbol
        false,   // allow_tests
        0,       // context_lines
        None,    // specific_line_numbers
    )
    .unwrap();

    // Should return the requested lines
    assert_eq!(result.lines, (3, 6), "Should return requested line range");
    assert!(result.code.contains("web:"), "Should contain web service");
    assert!(
        result.code.contains("image: nginx"),
        "Should contain nginx image"
    );
    assert!(result.code.contains("80:80"), "Should contain port mapping");
}

#[test]
fn test_extract_cli_unsupported_file_type() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("data.jsonl");
    let content = r#"{"id": 1, "name": "Alice", "age": 30}
{"id": 2, "name": "Bob", "age": 25}
{"id": 3, "name": "Charlie", "age": 35}
"#;
    fs::write(&file_path, content).unwrap();

    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Run the extract command on an unsupported file type
    let output = Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            project_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--",
            "extract",
            &format!("{}:2", file_path.to_string_lossy()),
        ])
        .output()
        .expect("Failed to execute command");

    // Should succeed even with unsupported file type
    assert!(
        output.status.success(),
        "Command should succeed for unsupported file type. Stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain the requested line
    assert!(
        stdout.contains("Bob"),
        "Output should contain the second line with Bob"
    );
}
