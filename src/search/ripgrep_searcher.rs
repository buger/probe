use anyhow::{Context, Result};
use regex::RegexSet;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// High-performance RegexSet-based searcher for fast file pattern matching
/// This provides optimal performance through:
/// - Pre-compiled RegexSet for multi-pattern efficiency  
/// - Simple file I/O without unnecessary abstraction layers
/// - Parallel processing capabilities
/// - Thread-safe design for concurrent access
pub struct RipgrepSearcher {
    debug_mode: bool,
    regex_set: RegexSet,
}

impl RipgrepSearcher {
    /// Create a new RipgrepSearcher with optimized settings
    pub fn new(patterns: &[String], _enable_simd: bool) -> Result<Self> {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

        if debug_mode {
            println!(
                "DEBUG: Creating fast RegexSet searcher with {} patterns",
                patterns.len()
            );
        }

        // Pre-compile RegexSet for efficient pattern matching
        let case_insensitive_patterns: Vec<String> =
            patterns.iter().map(|p| format!("(?i){p}")).collect();
        let regex_set = RegexSet::new(&case_insensitive_patterns)
            .context("Failed to build RegexSet during initialization")?;

        Ok(RipgrepSearcher {
            debug_mode,
            regex_set,
        })
    }

    /// Search a single file and return term matches with line numbers
    /// This uses a fast RegexSet-based approach for maximum performance
    pub fn search_file(
        &self,
        file_path: &Path,
        pattern_to_terms: &[HashSet<usize>],
    ) -> Result<HashMap<usize, HashSet<usize>>> {
        let start_time = Instant::now();
        let mut term_map = HashMap::new();

        if self.debug_mode {
            println!("DEBUG: Searching file with optimized RegexSet: {file_path:?}");
        }

        // Define a reasonable maximum file size (same as old implementation)
        const MAX_FILE_SIZE: u64 = 1024 * 1024; // 1MB

        // Skip symlinks and junctions entirely to avoid stack overflow
        use crate::path_safety;

        if path_safety::is_symlink_or_junction(file_path) {
            if self.debug_mode {
                println!("DEBUG: Skipping symlink/junction: {file_path:?}");
            }
            return Err(anyhow::anyhow!("Skipping symlink/junction"));
        }

        // Use the path as-is in CI to avoid any resolution issues
        let resolved_path = if path_safety::is_ci_environment() {
            file_path.to_path_buf()
        } else {
            // Only canonicalize if not in CI and not a symlink
            match std::fs::canonicalize(file_path) {
                Ok(path) => path,
                Err(_) => file_path.to_path_buf(), // Fall back to original path
            }
        };

        // Get file metadata to check size and file type
        let metadata = match std::fs::metadata(&resolved_path) {
            Ok(meta) => meta,
            Err(e) => {
                if self.debug_mode {
                    println!("DEBUG: Error getting metadata for {resolved_path:?}: {e:?}");
                }
                return Err(anyhow::anyhow!("Failed to get file metadata: {}", e));
            }
        };

        // Check if the file is too large
        if metadata.len() > MAX_FILE_SIZE {
            if self.debug_mode {
                println!(
                    "DEBUG: Skipping file {:?} - file too large ({} bytes > {} bytes limit)",
                    resolved_path,
                    metadata.len(),
                    MAX_FILE_SIZE
                );
            }
            return Err(anyhow::anyhow!(
                "File too large: {} bytes (limit: {} bytes)",
                metadata.len(),
                MAX_FILE_SIZE
            ));
        }

        // Read the file content with proper error handling (simple and fast)
        let content = match std::fs::read_to_string(&resolved_path) {
            Ok(content) => content,
            Err(e) => {
                if self.debug_mode {
                    println!(
                        "DEBUG: Error reading file {:?}: {:?} (size: {} bytes)",
                        resolved_path,
                        e,
                        metadata.len()
                    );
                }
                return Err(anyhow::anyhow!("Failed to read file: {}", e));
            }
        };

        // Process each line (fast in-memory processing)
        for (line_number, line) in content.lines().enumerate() {
            // Skip lines that are too long
            if line.len() > 2000 {
                if self.debug_mode {
                    println!(
                        "DEBUG: Skipping line {} in file {:?} - line too long ({} characters)",
                        line_number + 1,
                        file_path,
                        line.len()
                    );
                }
                continue;
            }

            // Use pre-compiled RegexSet for efficient multi-pattern matching
            let matches = self.regex_set.matches(line);
            if matches.matched_any() {
                // For each matched pattern, map to corresponding term indices
                for pattern_idx in matches.iter() {
                    if let Some(term_set) = pattern_to_terms.get(pattern_idx) {
                        for &term_idx in term_set {
                            term_map
                                .entry(term_idx)
                                .or_insert_with(HashSet::new)
                                .insert(line_number + 1); // Convert to 1-based line numbers
                        }
                    }
                }
            }
        }

        if self.debug_mode {
            let duration = start_time.elapsed();
            println!(
                "DEBUG: Fast RegexSet search completed for {:?} in {:?} - found {} unique terms",
                file_path,
                duration,
                term_map.len()
            );
        }

        Ok(term_map)
    }

    /// Search multiple files in parallel using fast RegexSet-based approach
    pub fn search_files_parallel(
        &self,
        file_paths: &[PathBuf],
        pattern_to_terms: &[HashSet<usize>],
    ) -> Result<HashMap<PathBuf, HashMap<usize, HashSet<usize>>>> {
        use rayon::prelude::*;

        let start_time = Instant::now();

        if self.debug_mode {
            println!(
                "DEBUG: Starting parallel RegexSet search on {} files",
                file_paths.len()
            );
        }

        // Sort file paths for deterministic processing order to fix non-deterministic behavior
        // This ensures that parallel processing always processes files in the same order
        let mut sorted_file_paths = file_paths.to_vec();
        sorted_file_paths.sort();

        // Use par_iter().filter_map() for parallel processing
        // The searcher instance is thread-safe, so we can reuse it
        let results: Vec<(PathBuf, HashMap<usize, HashSet<usize>>)> = sorted_file_paths
            .par_iter()
            .filter_map(|file_path| {
                // Reuse the shared searcher instance - it's thread-safe
                // The search_file method uses only simple file I/O and RegexSet matching
                match self.search_file(file_path, pattern_to_terms) {
                    Ok(term_map) => {
                        if !term_map.is_empty() {
                            Some((file_path.clone(), term_map))
                        } else {
                            None
                        }
                    }
                    Err(e) => {
                        if self.debug_mode {
                            println!("DEBUG: Error searching file {file_path:?}: {e}");
                        }
                        None
                    }
                }
            })
            .collect();

        // Convert to HashMap via BTreeMap for deterministic ordering
        let sorted_results: std::collections::BTreeMap<PathBuf, HashMap<usize, HashSet<usize>>> =
            results.into_iter().collect();
        let final_results: HashMap<PathBuf, HashMap<usize, HashSet<usize>>> =
            sorted_results.into_iter().collect();

        if self.debug_mode {
            let duration = start_time.elapsed();
            println!(
                "DEBUG: Parallel RegexSet search completed in {:?} - found matches in {} files",
                duration,
                final_results.len()
            );
        }

        Ok(final_results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_ripgrep_searcher_basic() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        fs::write(&file_path, "fn main() {\n    println!(\"hello world\");\n}").unwrap();

        let patterns = vec!["fn main".to_string()];
        let searcher = RipgrepSearcher::new(&patterns, true).unwrap();

        let pattern_to_terms = vec![{
            let mut set = HashSet::new();
            set.insert(0);
            set
        }];

        let result = searcher.search_file(&file_path, &pattern_to_terms).unwrap();

        assert!(!result.is_empty());
        assert!(result.contains_key(&0));
        assert!(result[&0].contains(&1)); // Line 1 should match
    }

    #[test]
    fn test_ripgrep_searcher_parallel() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("test1.rs");
        let file2 = dir.path().join("test2.rs");

        fs::write(&file1, "fn test1() {}").unwrap();
        fs::write(&file2, "fn test2() {}").unwrap();

        let patterns = vec!["fn test".to_string()];
        let searcher = RipgrepSearcher::new(&patterns, true).unwrap();

        let files = vec![file1.clone(), file2.clone()];
        let pattern_to_terms = vec![{
            let mut set = HashSet::new();
            set.insert(0);
            set
        }];

        let results = searcher
            .search_files_parallel(&files, &pattern_to_terms)
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.contains_key(&file1));
        assert!(results.contains_key(&file2));
    }
}
