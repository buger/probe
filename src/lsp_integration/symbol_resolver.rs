//! Symbol Resolver for LSP Integration
//!
//! This module provides functionality to parse location specifications in multiple formats:
//! - `file.rs:42:10` - line:column format
//! - `file.rs#symbol_name` - symbol reference format
//!
//! The resolver uses tree-sitter AST parsing to find symbol positions when using the #symbol syntax,
//! leveraging existing code from the extract module.

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::extract::symbol_finder::find_symbol_in_file_with_position;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLocation {
    /// The file path (absolute)
    pub file_path: PathBuf,
    /// Line number (0-based)
    pub line: u32,
    /// Column number (0-based)
    pub column: u32,
}

impl ResolvedLocation {
    /// Create a new ResolvedLocation
    pub fn new(file_path: PathBuf, line: u32, column: u32) -> Self {
        Self {
            file_path,
            line,
            column,
        }
    }

    /// Convert 1-based line and column to 0-based
    pub fn from_one_based(file_path: PathBuf, line: u32, column: u32) -> Self {
        Self {
            file_path,
            line: line.saturating_sub(1),
            column: column.saturating_sub(1),
        }
    }

    /// Get line number as 1-based (for display)
    pub fn line_one_based(&self) -> u32 {
        self.line + 1
    }

    /// Get column number as 1-based (for display)
    pub fn column_one_based(&self) -> u32 {
        self.column + 1
    }
}

/// Main entry point for resolving location specifications
///
/// Supports two formats:
/// 1. `file.rs:42:10` - line:column format (1-based, converted to 0-based internally)
/// 2. `file.rs#symbol_name` - symbol reference format (uses tree-sitter to find position)
///
/// Returns the resolved location with 0-based line and column numbers.
pub fn resolve_location(spec: &str) -> Result<ResolvedLocation> {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("[DEBUG] Resolving location spec: '{spec}'");
    }

    // Check if this is a symbol reference (contains '#')
    if spec.contains('#') {
        resolve_symbol_location(spec)
    } else {
        // Try to parse as line:column format
        parse_line_column_spec(spec)
    }
}

/// Parse a location specification in the format `file.rs:line:column`
///
/// Both line and column are expected to be 1-based and will be converted to 0-based internally.
/// If only line is provided (no column), column defaults to 0.
fn parse_line_column_spec(spec: &str) -> Result<ResolvedLocation> {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("[DEBUG] Parsing line:column spec: '{spec}'");
    }

    // Check if this is a Windows absolute path (e.g., C:\path\file.rs:42:10)
    let is_windows_path = spec.len() >= 3
        && spec.chars().nth(1) == Some(':')
        && spec
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
        && (spec.chars().nth(2) == Some('\\') || spec.chars().nth(2) == Some('/'));

    if is_windows_path {
        // For Windows paths, we need to be careful about splitting on ':'
        // Find the last two ':' characters for line:column
        let parts: Vec<&str> = spec.rsplitn(3, ':').collect();
        if parts.len() >= 2 {
            // parts[0] = column (or line if no column)
            // parts[1] = line (or file if no column)
            // parts[2..] = file path components (reversed)

            let column_str = parts[0];
            let line_str = parts[1];
            let file_part = if parts.len() > 2 {
                // Reconstruct the file path (reverse the split)
                let mut file_parts = parts[2..].to_vec();
                file_parts.reverse();
                file_parts.join(":")
            } else {
                return Err(anyhow::anyhow!("Invalid Windows path format: '{}'", spec));
            };

            if let (Ok(column), Ok(line)) = (column_str.parse::<u32>(), line_str.parse::<u32>()) {
                let file_path = resolve_file_path(&file_part)?;
                return Ok(ResolvedLocation::from_one_based(file_path, line, column));
            } else if let Ok(line) = column_str.parse::<u32>() {
                // Only line was provided, column is in line_str position
                let file_path = resolve_file_path(&format!("{file_part}:{line_str}"))?;
                return Ok(ResolvedLocation::from_one_based(file_path, line, 0));
            }
        }

        return Err(anyhow::anyhow!(
            "Failed to parse Windows path with line:column: '{}'",
            spec
        ));
    }

    // For non-Windows paths, split on ':' from the right
    let parts: Vec<&str> = spec.rsplitn(3, ':').collect();

    match parts.len() {
        3 => {
            // file:line:column format
            let column_str = parts[0];
            let line_str = parts[1];
            let file_part = parts[2];

            let line = line_str
                .parse::<u32>()
                .with_context(|| format!("Invalid line number: '{line_str}'"))?;
            let column = column_str
                .parse::<u32>()
                .with_context(|| format!("Invalid column number: '{column_str}'"))?;

            let file_path = resolve_file_path(file_part)?;

            if debug_mode {
                println!(
                    "[DEBUG] Parsed file: '{}', line: {}, column: {}",
                    file_path.display(),
                    line,
                    column
                );
            }

            Ok(ResolvedLocation::from_one_based(file_path, line, column))
        }
        2 => {
            // file:line format (column defaults to 0)
            let line_str = parts[0];
            let file_part = parts[1];

            let line = line_str
                .parse::<u32>()
                .with_context(|| format!("Invalid line number: '{line_str}'"))?;

            let file_path = resolve_file_path(file_part)?;

            if debug_mode {
                println!(
                    "[DEBUG] Parsed file: '{}', line: {} (column defaulted to 0)",
                    file_path.display(),
                    line
                );
            }

            Ok(ResolvedLocation::from_one_based(file_path, line, 0))
        }
        _ => Err(anyhow::anyhow!(
            "Invalid line:column format: '{}'. Expected 'file:line' or 'file:line:column'",
            spec
        )),
    }
}

/// Resolve a symbol location specification in the format `file.rs#symbol_name`
///
/// Uses tree-sitter to find the symbol position in the file.
fn resolve_symbol_location(spec: &str) -> Result<ResolvedLocation> {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("[DEBUG] Resolving symbol location: '{spec}'");
    }

    let (file_part, symbol) = spec.split_once('#').ok_or_else(|| {
        anyhow::anyhow!("Invalid symbol format: '{}'. Expected 'file#symbol'", spec)
    })?;

    if symbol.is_empty() {
        return Err(anyhow::anyhow!("Empty symbol name in: '{}'", spec));
    }

    let file_path = resolve_file_path(file_part)?;

    if debug_mode {
        println!(
            "[DEBUG] Looking for symbol '{}' in file '{}'",
            symbol,
            file_path.display()
        );
    }

    // Read the file content
    let content = std::fs::read_to_string(&file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    // Use the existing symbol finder to locate the symbol
    let (search_result, position) = find_symbol_in_file_with_position(
        &file_path, symbol, &content, true, // allow_tests
        0,    // context_lines (not used for position finding)
    )?;

    if let Some((line, column)) = position {
        if debug_mode {
            println!("[DEBUG] Found symbol '{symbol}' at line {line}, column {column}");
        }

        // find_symbol_in_file_with_position returns 0-based positions
        Ok(ResolvedLocation::new(file_path, line, column))
    } else {
        // If we get a search result but no exact position, use the start of the search result
        if debug_mode {
            println!(
                "[DEBUG] No exact position found, using search result lines: {}-{}",
                search_result.lines.0, search_result.lines.1
            );
        }

        // search_result.lines are 1-based, convert to 0-based
        let line = search_result.lines.0.saturating_sub(1) as u32;
        Ok(ResolvedLocation::new(file_path, line, 0))
    }
}

/// Resolve a file path, handling relative paths and path normalization
fn resolve_file_path(file_part: &str) -> Result<PathBuf> {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("[DEBUG] Resolving file path: '{file_part}'");
    }

    let path = PathBuf::from(file_part);

    // If the path is absolute and exists, use it directly
    if path.is_absolute() {
        if path.exists() {
            return Ok(path);
        } else {
            return Err(anyhow::anyhow!("File not found: '{}'", path.display()));
        }
    }

    // For relative paths, try to resolve from the current directory
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;

    let resolved = current_dir.join(&path);

    if resolved.exists() {
        if debug_mode {
            println!(
                "[DEBUG] Resolved relative path to: '{}'",
                resolved.display()
            );
        }
        Ok(resolved)
    } else {
        // Try using the path_resolver module's resolve_path function as fallback
        match probe_code::path_resolver::resolve_path(file_part) {
            Ok(resolved_path) => {
                if debug_mode {
                    println!(
                        "[DEBUG] Resolved via path_resolver to: '{}'",
                        resolved_path.display()
                    );
                }
                Ok(resolved_path)
            }
            Err(_) => Err(anyhow::anyhow!(
                "File not found: '{}' (tried relative to current directory: '{}')",
                file_part,
                resolved.display()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_line_column_spec() {
        // Create a temporary file for testing
        let temp_file = NamedTempFile::new().unwrap();
        let temp_path = temp_file.path().to_string_lossy();

        // Test file:line:column format
        let spec = format!("{temp_path}:10:5");
        let result = parse_line_column_spec(&spec).unwrap();
        assert_eq!(result.line, 9); // 0-based
        assert_eq!(result.column, 4); // 0-based

        // Test file:line format (column should default to 0)
        let spec = format!("{temp_path}:10");
        let result = parse_line_column_spec(&spec).unwrap();
        assert_eq!(result.line, 9); // 0-based
        assert_eq!(result.column, 0); // default
    }

    #[test]
    fn test_parse_line_column_spec_windows_path() {
        // Test Windows path parsing (without actually creating the file on non-Windows)
        if cfg!(windows) {
            let spec = r"C:\path\to\file.rs:10:5";
            // This would work on Windows with a real file
            // For this test, we just ensure it doesn't panic and gives the right error
            let result = parse_line_column_spec(spec);
            assert!(result.is_err()); // Expected to fail since file doesn't exist
        }
    }

    #[test]
    fn test_resolve_location_line_column() {
        // Create a temporary file
        let temp_file = NamedTempFile::new().unwrap();
        let temp_path = temp_file.path().to_string_lossy();

        let spec = format!("{temp_path}:5:10");
        let result = resolve_location(&spec).unwrap();

        assert_eq!(result.line, 4); // 0-based
        assert_eq!(result.column, 9); // 0-based
        assert_eq!(result.line_one_based(), 5); // 1-based for display
        assert_eq!(result.column_one_based(), 10); // 1-based for display
    }

    #[test]
    fn test_resolve_location_invalid_format() {
        // Test invalid formats
        assert!(resolve_location("").is_err());
        assert!(resolve_location("file").is_err());
        assert!(resolve_location("file#").is_err());
        assert!(resolve_location("file:abc:def").is_err());
    }

    #[test]
    fn test_symbol_location_with_rust_code() {
        // Create a temporary Rust file with some code
        let temp_file = NamedTempFile::new().unwrap();
        let rust_code = r#"
pub struct TestStruct {
    field: i32,
}

impl TestStruct {
    pub fn test_method(&self) -> i32 {
        self.field
    }
}

pub fn test_function() -> i32 {
    42
}
"#;
        fs::write(temp_file.path(), rust_code).unwrap();

        let spec = format!("{}#test_function", temp_file.path().to_string_lossy());
        let result = resolve_location(&spec);

        // Should successfully find the function
        let location = match result {
            Ok(loc) => loc,
            Err(e) => {
                panic!("Failed to resolve location '{spec}': {e}");
            }
        };

        // The function should be found somewhere in the file
        // Just verify that we got a valid location
        assert!(location.file_path.exists());
        // And that we have some meaningful line number (could be 0 for first line)
        // We just care that it's a reasonable number
        assert!(location.line < 1000); // Arbitrary reasonable upper bound
    }

    #[test]
    fn test_symbol_location_nested_symbol() {
        // Create a temporary Rust file with nested symbols
        let temp_file = NamedTempFile::new().unwrap();
        let rust_code = r#"
pub struct TestStruct {
    field: i32,
}

impl TestStruct {
    pub fn test_method(&self) -> i32 {
        self.field
    }
}
"#;
        fs::write(temp_file.path(), rust_code).unwrap();

        let spec = format!(
            "{}#TestStruct.test_method",
            temp_file.path().to_string_lossy()
        );
        let result = resolve_location(&spec);

        // Should successfully find the nested method
        let location = match result {
            Ok(loc) => loc,
            Err(e) => {
                panic!("Failed to resolve location '{spec}': {e}");
            }
        };

        // The method should be found somewhere in the file
        // Just verify that we got a valid location
        assert!(location.file_path.exists());
        // And that we have some meaningful line number
        assert!(location.line < 1000); // Arbitrary reasonable upper bound
    }

    #[test]
    fn test_resolved_location_creation() {
        let path = PathBuf::from("/test/file.rs");

        // Test zero-based constructor
        let loc = ResolvedLocation::new(path.clone(), 5, 10);
        assert_eq!(loc.line, 5);
        assert_eq!(loc.column, 10);
        assert_eq!(loc.line_one_based(), 6);
        assert_eq!(loc.column_one_based(), 11);

        // Test one-based constructor
        let loc = ResolvedLocation::from_one_based(path, 6, 11);
        assert_eq!(loc.line, 5);
        assert_eq!(loc.column, 10);
        assert_eq!(loc.line_one_based(), 6);
        assert_eq!(loc.column_one_based(), 11);
    }
}
