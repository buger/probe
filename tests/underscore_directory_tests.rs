use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;
use assert_cmd::Command;

// Helper function to create test files
fn create_test_file(path: &PathBuf, content: &str) {
    let mut file = File::create(path).expect("Failed to create test file");
    file.write_all(content.as_bytes())
        .expect("Failed to write test content");
}

// Helper function to create a test directory structure with underscores
fn create_underscore_directory_structure(root_dir: &TempDir) -> PathBuf {
    // Create parent directory
    let parent_dir = root_dir.path().join("parent_dir");
    fs::create_dir(&parent_dir).expect("Failed to create parent_dir");

    // Create docs_packages directory (with underscore)
    let docs_packages_dir = parent_dir.join("docs_packages");
    fs::create_dir(&docs_packages_dir).expect("Failed to create docs_packages directory");

    // Create helloKitty directory
    let hello_kitty_dir = docs_packages_dir.join("helloKitty");
    fs::create_dir(&hello_kitty_dir).expect("Failed to create helloKitty directory");

    // Create a file with the search term
    let file_path = hello_kitty_dir.join("dog.txt");
    create_test_file(&file_path, "bad kitty");

    parent_dir
}

#[cfg(windows)]
#[test]
fn test_underscore_directory_search() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parent_dir = create_underscore_directory_structure(&temp_dir);

    // Test 1: Search directly in the helloKitty directory (should work)
    let hello_kitty_path = parent_dir.join("docs_packages").join("helloKitty");
    let result1 = Command::cargo_bin("probe")
        .expect("Failed to find binary")
        .args(&[
            "search",
            "bad kitty",
            hello_kitty_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    
    let output1 = result1.get_output();
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    
    println!("Search in helloKitty directory output: {}", stdout1);
    assert!(
        stdout1.contains("Found"),
        "Search in helloKitty directory should find matches"
    );
    assert!(
        stdout1.contains("dog.txt"),
        "Search in helloKitty directory should find dog.txt"
    );

    // Test 2: Search from the parent directory (should work)
    let result2 = Command::cargo_bin("probe")
        .expect("Failed to find binary")
        .args(&[
            "search",
            "bad kitty",
            parent_dir.to_str().unwrap(),
        ])
        .assert()
        .success();
    
    let output2 = result2.get_output();
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    
    println!("Search from parent directory output: {}", stdout2);
    assert!(
        stdout2.contains("Found"),
        "Search from parent directory should find matches"
    );
    assert!(
        stdout2.contains("dog.txt"),
        "Search from parent directory should find dog.txt"
    );

    // Test 3: Search in the docs_packages directory (should fail or return no results)
    let docs_packages_path = parent_dir.join("docs_packages");
    let result3 = Command::cargo_bin("probe")
        .expect("Failed to find binary")
        .args(&[
            "search",
            "bad kitty",
            docs_packages_path.to_str().unwrap(),
        ])
        .assert()
        .success(); // The command itself should succeed even if no results are found
    
    let output3 = result3.get_output();
    let stdout3 = String::from_utf8_lossy(&output3.stdout);
    
    println!("Search in docs_packages directory output: {}", stdout3);
    
    // Check if the search failed to find results
    // This could be either "No results found" or some other indication of failure
    assert!(
        !stdout3.contains("dog.txt") || stdout3.contains("No results found") || stdout3.contains("0 search results"),
        "Search in docs_packages directory should fail to find dog.txt due to underscore issue"
    );
}
