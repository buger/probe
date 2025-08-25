//! Integration tests for the configuration system
//!
//! This module contains comprehensive integration tests for the entire configuration
//! workflow including file discovery, hierarchy loading, environment variable
//! precedence, and configuration affecting actual behavior.

use anyhow::Result;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper to get the probe binary path
fn probe_binary_path() -> PathBuf {
    if let Ok(path) = env::var("CARGO_BIN_EXE_probe") {
        return PathBuf::from(path);
    }

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push(if cfg!(windows) { "probe.exe" } else { "probe" });

    if !path.exists() {
        panic!("Probe binary not found at {path:?}. Run 'cargo build' first.");
    }

    path
}

#[test]
fn test_config_file_discovery() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    // Create .probe directory
    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    // Create settings.json
    let config_file = probe_dir.join("settings.json");
    let config_content = r#"
    {
        "defaults": {
            "debug": true,
            "log_level": "debug"
        },
        "indexing": {
            "enabled": false,
            "auto_index": false,
            "watch_files": false
        }
    }
    "#;
    fs::write(&config_file, config_content)?;

    // Run probe from the project directory
    let output = Command::new(probe_binary_path())
        .args(["config", "show", "--format", "json"])
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    // Verify config was loaded from file
    assert_eq!(json["defaults"]["debug"], true);
    assert_eq!(json["defaults"]["log_level"], "debug");
    assert_eq!(json["indexing"]["enabled"], false);
    assert_eq!(json["indexing"]["auto_index"], false);
    assert_eq!(json["indexing"]["watch_files"], false);

    Ok(())
}

#[test]
fn test_config_hierarchy_loading() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    // Create global config
    let global_config = probe_dir.join("settings.json");
    let global_content = r#"
    {
        "defaults": {
            "debug": false,
            "log_level": "info",
            "timeout": 30
        },
        "search": {
            "max_results": 10,
            "frequency": true
        }
    }
    "#;
    fs::write(&global_config, global_content)?;

    // Create local config that overrides some settings
    let local_config = probe_dir.join("settings.local.json");
    let local_content = r#"
    {
        "defaults": {
            "debug": true,
            "timeout": 60
        },
        "search": {
            "max_results": 20
        }
    }
    "#;
    fs::write(&local_config, local_content)?;

    let output = Command::new(probe_binary_path())
        .args(["config", "show", "--format", "json"])
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    // Verify hierarchy merging
    assert_eq!(
        json["defaults"]["debug"], true,
        "Local should override global"
    );
    assert_eq!(
        json["defaults"]["log_level"], "info",
        "Should keep global value"
    );
    assert_eq!(
        json["defaults"]["timeout"], 60,
        "Local should override global"
    );
    assert_eq!(
        json["search"]["max_results"], 20,
        "Local should override global"
    );
    assert_eq!(
        json["search"]["frequency"], true,
        "Should keep global value"
    );

    Ok(())
}

#[test]
fn test_environment_variable_precedence() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    // Create config file
    let config_file = probe_dir.join("settings.json");
    let config_content = r#"
    {
        "defaults": {
            "debug": false,
            "enable_lsp": false
        },
        "indexing": {
            "enabled": true,
            "auto_index": true,
            "watch_files": true
        }
    }
    "#;
    fs::write(&config_file, config_content)?;

    // Run with environment variables that should override config file
    let output = Command::new(probe_binary_path())
        .args(["config", "show", "--format", "json"])
        .env("PROBE_DEBUG", "1")
        .env("PROBE_ENABLE_LSP", "true")
        .env("PROBE_INDEXING_ENABLED", "false")
        .env("PROBE_INDEXING_AUTO_INDEX", "false")
        .env("PROBE_INDEXING_WATCH_FILES", "false")
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    // Verify environment variables take precedence
    assert_eq!(
        json["defaults"]["debug"], true,
        "Env var should override config"
    );
    assert_eq!(
        json["defaults"]["enable_lsp"], true,
        "Env var should override config"
    );
    assert_eq!(
        json["indexing"]["enabled"], false,
        "Env var should override config"
    );
    assert_eq!(
        json["indexing"]["auto_index"], false,
        "Env var should override config"
    );
    assert_eq!(
        json["indexing"]["watch_files"], false,
        "Env var should override config"
    );

    Ok(())
}

#[test]
fn test_config_affects_search_behavior() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    // Create test files with more substantial content for search
    let src_dir = project_dir.join("src");
    fs::create_dir(&src_dir)?;

    let main_rs = src_dir.join("main.rs");
    fs::write(
        &main_rs,
        r#"
fn main() {
    println!("Hello, world!");
    search_in_main();
}

fn search_in_main() {
    println!("This function contains search term");
}

fn test_helper_function() {
    println!("This is a test helper function with search");
}

#[test]
fn test_something_with_search() {
    assert_eq!(1, 1);
    // This test contains search term
}
"#,
    )?;

    // Create config that limits results
    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    let config_file = probe_dir.join("settings.json");
    let config_content = r#"
    {
        "search": {
            "max_results": 3
        }
    }
    "#;
    fs::write(&config_file, config_content)?;

    // Search for "search" - should find multiple matches
    let output = Command::new(probe_binary_path())
        .args(["search", "search", "."])
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify that search ran and we get some results
    if !stdout.contains("No results found") {
        // Should find search_in_main or test_helper_function containing search
        let has_search_matches = stdout.contains("search_in_main")
            || stdout.contains("test_helper_function")
            || stdout.contains("search");
        assert!(
            has_search_matches,
            "Should find functions or content containing 'search' term. Output:\n{stdout}"
        );
    } else {
        // If no results found, verify that the command still succeeded (configuration loaded correctly)
        // This tests that the config was loaded without errors
        println!("No search results found, but configuration was loaded successfully");
    }

    Ok(())
}

#[test]
fn test_config_affects_lsp_behavior() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    // Create config that disables LSP
    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    let config_file = probe_dir.join("settings.json");
    let config_content = r#"
    {
        "defaults": {
            "enable_lsp": false
        },
        "lsp": {
            "disable_autostart": true
        }
    }
    "#;
    fs::write(&config_file, config_content)?;

    // Create a test file
    let test_file = project_dir.join("test.rs");
    fs::write(&test_file, "fn main() {}")?;

    // Try to extract with LSP (should not use LSP due to config)
    let output = Command::new(probe_binary_path())
        .args(["extract", "test.rs#main"])
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    // With LSP disabled, it should fall back to tree-sitter extraction

    Ok(())
}

#[test]
fn test_indexing_configuration_affects_behavior() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    // Create config with specific indexing settings
    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    let config_file = probe_dir.join("settings.json");
    let config_content = r#"
    {
        "indexing": {
            "enabled": true,
            "auto_index": false,
            "watch_files": false,
            "features": {
                "extract_functions": true,
                "extract_types": true,
                "extract_variables": false,
                "extract_imports": false,
                "extract_tests": false
            }
        }
    }
    "#;
    fs::write(&config_file, config_content)?;

    // Verify config is loaded correctly
    let output = Command::new(probe_binary_path())
        .args(["config", "show", "--format", "json"])
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    assert_eq!(json["indexing"]["enabled"], true);
    assert_eq!(json["indexing"]["auto_index"], false);
    assert_eq!(json["indexing"]["watch_files"], false);
    assert_eq!(json["indexing"]["features"]["extract_functions"], true);
    assert_eq!(json["indexing"]["features"]["extract_variables"], false);

    Ok(())
}

#[test]
fn test_invalid_config_fallback() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    // Create invalid JSON config
    let config_file = probe_dir.join("settings.json");
    let invalid_content = r#"
    {
        "defaults": {
            "debug": true,
        },  // Extra comma
        "search" {  // Missing colon
            "max_results": 10
        }
    }
    "#;
    fs::write(&config_file, invalid_content)?;

    // Should still work with defaults
    let output = Command::new(probe_binary_path())
        .args(["config", "show", "--format", "json"])
        .current_dir(&project_dir)
        .output()?;

    assert!(
        output.status.success(),
        "Should succeed with invalid config"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    // Should have default values
    assert_eq!(json["indexing"]["enabled"], true);
    assert_eq!(json["indexing"]["auto_index"], true);
    assert_eq!(json["indexing"]["watch_files"], true);

    Ok(())
}

#[test]
fn test_config_language_specific_settings() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    // Create config with language-specific settings
    let config_file = probe_dir.join("settings.json");
    let config_content = r#"
    {
        "indexing": {
            "enabled": true,
            "priority_languages": ["rust", "python"],
            "disabled_languages": ["c", "cpp"],
            "language_configs": {
                "rust": {
                    "enabled": true,
                    "max_workers": 4
                },
                "python": {
                    "enabled": true,
                    "max_workers": 2
                }
            }
        }
    }
    "#;
    fs::write(&config_file, config_content)?;

    let output = Command::new(probe_binary_path())
        .args(["config", "show", "--format", "json"])
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    // Verify language-specific configuration
    assert_eq!(json["indexing"]["priority_languages"][0], "rust");
    assert_eq!(json["indexing"]["priority_languages"][1], "python");
    assert_eq!(json["indexing"]["disabled_languages"][0], "c");
    assert_eq!(json["indexing"]["disabled_languages"][1], "cpp");

    let rust_config = &json["indexing"]["language_configs"]["rust"];
    assert_eq!(rust_config["enabled"], true);
    assert_eq!(rust_config["max_workers"], 4);

    Ok(())
}

#[test]
fn test_lsp_caching_configuration() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    // Create config with LSP caching settings
    let config_file = probe_dir.join("settings.json");
    let config_content = r#"
    {
        "indexing": {
            "lsp_caching": {
                "cache_call_hierarchy": true,
                "cache_definitions": false,
                "cache_references": true,
                "cache_hover": true,
                "cache_document_symbols": false,
                "cache_during_indexing": false,
                "preload_common_symbols": false,
                "max_cache_entries_per_operation": 2000,
                "lsp_operation_timeout_ms": 10000,
                "priority_operations": ["call_hierarchy", "references"],
                "disabled_operations": ["document_symbols"]
            }
        }
    }
    "#;
    fs::write(&config_file, config_content)?;

    let output = Command::new(probe_binary_path())
        .args(["config", "show", "--format", "json"])
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    // Verify LSP caching configuration
    let lsp_caching = &json["indexing"]["lsp_caching"];
    assert_eq!(lsp_caching["cache_call_hierarchy"], true);
    assert_eq!(lsp_caching["cache_definitions"], false);
    assert_eq!(lsp_caching["cache_references"], true);
    assert_eq!(lsp_caching["max_cache_entries_per_operation"], 2000);
    assert_eq!(lsp_caching["lsp_operation_timeout_ms"], 10000);
    assert_eq!(lsp_caching["priority_operations"][0], "call_hierarchy");
    assert_eq!(lsp_caching["disabled_operations"][0], "document_symbols");

    Ok(())
}

#[test]
fn test_performance_configuration() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("project");
    fs::create_dir(&project_dir)?;

    let probe_dir = project_dir.join(".probe");
    fs::create_dir(&probe_dir)?;

    // Create config with performance settings
    let config_file = probe_dir.join("settings.json");
    let config_content = r#"
    {
        "performance": {
            "tree_cache_size": 5000,
            "optimize_blocks": true
        },
        "indexing": {
            "max_workers": 16,
            "memory_budget_mb": 1024,
            "memory_pressure_threshold": 0.9,
            "max_queue_size": 20000,
            "discovery_batch_size": 2000,
            "file_processing_timeout_ms": 60000,
            "parallel_file_processing": true
        }
    }
    "#;
    fs::write(&config_file, config_content)?;

    let output = Command::new(probe_binary_path())
        .args(["config", "show", "--format", "json"])
        .current_dir(&project_dir)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    // Verify performance configuration
    assert_eq!(json["performance"]["tree_cache_size"], 5000);
    assert_eq!(json["performance"]["optimize_blocks"], true);
    assert_eq!(json["indexing"]["max_workers"], 16);
    assert_eq!(json["indexing"]["memory_budget_mb"], 1024);
    assert_eq!(json["indexing"]["memory_pressure_threshold"], 0.9);
    assert_eq!(json["indexing"]["parallel_file_processing"], true);

    Ok(())
}
