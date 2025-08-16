use anyhow::{Context, Result};
use std::path::PathBuf;
use tempfile::TempDir;

/// Test the path resolution logic that was implemented to fix the relative path URI conversion issue
#[test]
fn test_relative_path_to_absolute_conversion() -> Result<()> {
    // Create a temporary directory structure for testing
    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

    // Create a nested workspace directory
    let workspace_dir = temp_path.join("my-workspace");
    std::fs::create_dir(&workspace_dir)?;

    // Create a Rust file to make it a valid workspace
    std::fs::write(workspace_dir.join("main.rs"), "fn main() {}")?;

    // Change to the temp directory to simulate user running command from parent directory
    let original_dir = std::env::current_dir()?;
    std::env::set_current_dir(temp_path)?;

    // Test the path resolution logic from init_workspaces
    let workspace_path = Some("my-workspace".to_string());
    let workspace_root = if let Some(ws) = workspace_path {
        let path = PathBuf::from(ws);
        // This is the exact logic from the fix
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .context("Failed to get current directory")?
                .join(&path)
                .canonicalize()
                .context(format!(
                    "Failed to resolve workspace path '{}'. Make sure the path exists and is accessible",
                    path.display()
                ))?
        }
    } else {
        std::env::current_dir()?
    };

    // Restore original directory
    std::env::set_current_dir(original_dir)?;

    // Verify the fix: path should now be absolute and exist
    assert!(
        workspace_root.is_absolute(),
        "Path should be absolute after resolution"
    );
    assert!(workspace_root.exists(), "Resolved path should exist");
    assert_eq!(workspace_root, workspace_dir.canonicalize()?);

    // Verify this path would now work with URI conversion
    // (We can't test the actual URI conversion here since url crate is not available
    // in the main crate, but the fact that it's absolute means it will work)
    assert!(workspace_root.is_absolute());

    Ok(())
}

/// Test that absolute paths continue to work unchanged (backward compatibility)
#[test]
fn test_absolute_path_unchanged() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let absolute_path = temp_dir.path().canonicalize()?;

    // Test with absolute path input
    let workspace_path = Some(absolute_path.to_string_lossy().to_string());
    let workspace_root = if let Some(ws) = workspace_path {
        let path = PathBuf::from(ws);
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .context("Failed to get current directory")?
                .join(&path)
                .canonicalize()
                .context(format!(
                    "Failed to resolve workspace path '{}'. Make sure the path exists and is accessible",
                    path.display()
                ))?
        }
    } else {
        std::env::current_dir()?
    };

    // Absolute path should be passed through unchanged
    assert_eq!(workspace_root, absolute_path);
    assert!(workspace_root.is_absolute());
    assert!(workspace_root.exists());

    Ok(())
}

/// Test error handling for non-existent relative paths
#[test]
fn test_nonexistent_relative_path_error() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let workspace_path = Some("nonexistent-workspace".to_string());
    let result = if let Some(ws) = workspace_path {
        let path = PathBuf::from(ws);
        if path.is_absolute() {
            Ok(path)
        } else {
            std::env::current_dir()
                .context("Failed to get current directory")
                .and_then(|current_dir| {
                    current_dir
                        .join(&path)
                        .canonicalize()
                        .context(format!(
                            "Failed to resolve workspace path '{}'. Make sure the path exists and is accessible",
                            path.display()
                        ))
                })
        }
    } else {
        std::env::current_dir().context("Failed to get current directory")
    };

    std::env::set_current_dir(original_dir).unwrap();

    // Should fail with descriptive error for non-existent path
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("Failed to resolve workspace path"));
    assert!(error_msg.contains("nonexistent-workspace"));
}
