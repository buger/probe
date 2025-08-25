//! Path safety utilities for LSP daemon to avoid junction point cycles on Windows CI
//!
//! This module provides safe alternatives to canonicalize() and other path operations
//! that can trigger stack overflow when encountering Windows junction points.

use std::fs;
use std::path::{Path, PathBuf};

/// Safely canonicalize a path, avoiding junction point cycles on Windows CI
pub fn safe_canonicalize(path: &Path) -> PathBuf {
    // On Windows CI, avoid canonicalize() completely to prevent junction point traversal
    #[cfg(target_os = "windows")]
    {
        if std::env::var("CI").is_ok() {
            // In CI, just convert to absolute path without following junctions
            if path.is_absolute() {
                return path.to_path_buf();
            } else {
                // Make relative paths absolute using current directory
                return std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path);
            }
        }
    }

    // On non-Windows or non-CI, use regular canonicalize with fallback
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}

/// Check if a path exists without following symlinks/junctions
/// This is safe from junction point cycles that cause stack overflow
pub fn exists_no_follow(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Get metadata without following symlinks/junctions
/// Returns None if the path doesn't exist or can't be accessed
pub fn metadata_no_follow(path: &Path) -> Option<fs::Metadata> {
    fs::symlink_metadata(path).ok()
}

/// Check if a path is a symlink or junction
/// On Windows, this checks for reparse points which include junctions
pub fn is_symlink_or_junction(path: &Path) -> bool {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return true;
        }

        // On Windows, also check for reparse points (junctions)
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return true;
            }
        }
    }
    false
}

/// Check if we're in a CI environment where we should be extra cautious
pub fn is_ci_environment() -> bool {
    std::env::var("CI").is_ok()
}

/// Safe check if a path exists and is a file (without following symlinks)
pub fn is_file_no_follow(path: &Path) -> bool {
    metadata_no_follow(path)
        .map(|m| m.is_file())
        .unwrap_or(false)
}

/// Safe check if a path exists and is a directory (without following symlinks)
pub fn is_dir_no_follow(path: &Path) -> bool {
    metadata_no_follow(path)
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

/// Safely read directory entries without following junctions or symlinks
/// Returns an iterator over DirEntry that skips junction points and symlinks
pub fn safe_read_dir(dir: &Path) -> Result<impl Iterator<Item = fs::DirEntry>, std::io::Error> {
    let entries = fs::read_dir(dir)?;
    Ok(entries.filter_map(|entry| {
        match entry {
            Ok(entry) => {
                let path = entry.path();

                // Skip symlinks and junction points to avoid cycles
                if is_symlink_or_junction(&path) {
                    return None;
                }

                Some(entry)
            }
            Err(_) => None,
        }
    }))
}

/// Check if directory contains files with given extensions (safely)
/// Returns true if any file with the given extensions exists in the directory
pub fn has_files_with_extension(dir: &Path, extensions: &[&str]) -> bool {
    match safe_read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                let path = entry.path();
                if is_file_no_follow(&path) {
                    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                        if extensions.contains(&ext) {
                            return true;
                        }
                    }
                }
            }
            false
        }
        Err(_) => false,
    }
}
