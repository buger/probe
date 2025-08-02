//! Tests for the `--symbols` flag in the extract command
//!
//! This test suite verifies that the `--symbols` flag correctly extracts symbols
//! (functions, structs, methods) from a Go file.

use std::path::Path;
use std::process::Command;

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper function to run the probe command
    fn run_probe(args: &[&str]) -> std::process::Output {
        // Try running with cargo run first
        let output = Command::new("cargo")
            .args(["run", "--"])
            .args(args)
            .output();

        // If cargo run fails, try running the binary directly
        match output {
            Ok(o) => o,
            Err(_) => {
                // Try with the binary in target/debug
                Command::new("target/debug/probe")
                    .args(args)
                    .output()
                    .expect("Failed to execute probe command")
            }
        }
    }

    /// Test that the `--symbols` flag correctly extracts symbols from a Go file
    #[test]
    fn test_extract_symbols_basic() {
        // Path to the test file
        let test_file = Path::new("test_data/test_nested_struct.go");
        
        // Ensure the test file exists
        assert!(test_file.exists(), "Test file does not exist: {:?}", test_file);

        // Run the extract command with the --symbols flag
        let output = run_probe(&["extract", "--symbols", test_file.to_str().unwrap()]);

        // Check that the command succeeded
        assert!(output.status.success(), 
            "Extract command failed: {}", 
            String::from_utf8_lossy(&output.stderr));

        // Get the output as a string
        let output_str = String::from_utf8_lossy(&output.stdout);
        
        // Print the output for debugging
        println!("Extract command output:\n{}", output_str);

        // Verify that the output contains the expected symbols
        
        // Check for struct symbols in the output
        assert!(output_str.contains("Person struct"), "Output missing Person struct");
        assert!(output_str.contains("Address struct"), "Output missing Address struct");
        assert!(output_str.contains("Employee struct"), "Output missing Employee struct");
        
        // Check for method symbols in the output
        assert!(output_str.contains("func (p Person) SayHello"), "Output missing SayHello method");
        assert!(output_str.contains("func (a Address) GetFullAddress"), "Output missing GetFullAddress method");
        assert!(output_str.contains("func (e Employee) DisplayInfo"), "Output missing DisplayInfo method");
        assert!(output_str.contains("func (e Employee) GetSalaryDetails"), "Output missing GetSalaryDetails method");
        assert!(output_str.contains("func (e Employee) CalculateBonus"), "Output missing CalculateBonus method");
        
        // Check for function symbols in the output
        assert!(output_str.contains("func NestedFunction"), "Output missing NestedFunction function");
        assert!(output_str.contains("func main"), "Output missing main function");

        // Verify line numbers are included
        assert!(output_str.contains("Lines:"), "Output missing line numbers");
    }

    /// Test that the `--symbols` flag works with the plain output format
    #[test]
    fn test_extract_symbols_plain_format() {
        // Path to the test file
        let test_file = Path::new("test_data/test_nested_struct.go");
        
        // Run the extract command with the --symbols flag and plain format
        let output = run_probe(&["extract", "--symbols", "--format", "plain", test_file.to_str().unwrap()]);

        // Check that the command succeeded
        assert!(output.status.success(), 
            "Extract command failed: {}", 
            String::from_utf8_lossy(&output.stderr));

        // Get the output as a string
        let output_str = String::from_utf8_lossy(&output.stdout);
        
        // Verify that the output contains "Format: plain"
        assert!(output_str.contains("Format: plain"), "Output missing 'Format: plain' indicator");
        
        // Check for the same symbols as in the basic test
        assert!(output_str.contains("Person struct"), "Output missing Person struct");
        assert!(output_str.contains("func main"), "Output missing main function");
    }

    /// Test that the `--symbols` flag works with the JSON output format
    #[test]
    fn test_extract_symbols_json_format() {
        // Path to the test file
        let test_file = Path::new("test_data/test_nested_struct.go");
        
        // Run the extract command with the --symbols flag and JSON format
        let output = run_probe(&["extract", "--symbols", "--format", "json", test_file.to_str().unwrap()]);

        // Check that the command succeeded
        assert!(output.status.success(), 
            "Extract command failed: {}", 
            String::from_utf8_lossy(&output.stderr));

        // Get the output as a string
        let output_str = String::from_utf8_lossy(&output.stdout);
        
        // Verify that the output is in JSON format
        assert!(output_str.trim().contains("\"results\""), "JSON output missing results field");
        
        // Check for expected JSON structure
        assert!(output_str.contains("\"node_type\""), "JSON output missing node_type field");
        assert!(output_str.contains("\"code\""), "JSON output missing code field");
        assert!(output_str.contains("\"file\""), "JSON output missing file field");
        assert!(output_str.contains("\"lines\""), "JSON output missing lines field");
        
        // Check for specific symbols in the extracted code
        assert!(output_str.contains("Person struct"), "JSON output missing Person struct in code");
        assert!(output_str.contains("func main"), "JSON output missing main function in code");
    }

    /// Test that the `--symbols` flag works with the --allow-tests option
    #[test]
    fn test_extract_symbols_with_allow_tests() {
        // Path to the test file
        let test_file = Path::new("test_data/test_nested_struct.go");
        
        // Run the extract command with the --symbols flag and --allow-tests
        let output = run_probe(&["extract", "--symbols", "--allow-tests", test_file.to_str().unwrap()]);

        // Check that the command succeeded
        assert!(output.status.success(), 
            "Extract command failed: {}", 
            String::from_utf8_lossy(&output.stderr));

        // The output should be the same as without --allow-tests since our test file doesn't contain tests
        // But we're verifying the flag combination works
        let output_str = String::from_utf8_lossy(&output.stdout);
        
        // Check for the same symbols as in the basic test
        assert!(output_str.contains("Person struct"), "Output missing Person struct");
        assert!(output_str.contains("func main"), "Output missing main function");
    }

    /// Test the behavior when the file doesn't exist
    #[test]
    fn test_extract_symbols_nonexistent_file() {
        // Path to a nonexistent file
        let test_file = "test_data/nonexistent_file.go";
        
        // Run the extract command with the --symbols flag
        let output = run_probe(&["extract", "--symbols", test_file]);

        // The command should fail with a non-zero exit code
        assert!(!output.status.success(), "Command unexpectedly succeeded for nonexistent file");
        
        // The error message should mention that the file doesn't exist
        let error_msg = String::from_utf8_lossy(&output.stderr);
        assert!(error_msg.contains("exist") || error_msg.contains("found"), 
            "Error message doesn't indicate file not found: {}", error_msg);
    }

    /// Test the behavior when the path is a directory
    #[test]
    fn test_extract_symbols_directory() {
        // Path to a directory
        let test_dir = "test_data";
        
        // Run the extract command with the --symbols flag
        let output = run_probe(&["extract", "--symbols", test_dir]);

        // The command should fail with a non-zero exit code
        assert!(!output.status.success(), "Command unexpectedly succeeded for directory");
        
        // The error message should mention that directories are not supported
        let error_msg = String::from_utf8_lossy(&output.stderr);
        assert!(error_msg.contains("director") || error_msg.contains("not a file") || error_msg.contains("not support"), 
            "Error message doesn't indicate directory not supported: {}", error_msg);
    }
}