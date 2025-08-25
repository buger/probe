//! Path safety utilities to avoid following symlinks/junctions on Windows
//! This module provides safe alternatives to common path operations that
//! could trigger stack overflow when encountering junction point cycles.

use std::fs;
use std::path::Path;

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
