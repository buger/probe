use anyhow::Result;
// Removed unused random imports - now using deterministic session ID generation
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{create_dir_all, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use probe_code::models::SearchResult;

/// Generate a deterministic hash for a query string
/// This is used to create a unique identifier for each query
/// Uses ahash with a fixed seed for consistent, fast cache keys across program runs
pub fn hash_query(query: &str) -> String {
    use ahash::RandomState;

    // Use ahash with fixed seed for deterministic, fast hashing
    // This provides 15-20x performance improvement over SHA-256 while maintaining determinism
    let build_hasher = RandomState::with_seeds(
        0x123456789abcdef,
        0xfedcba9876543210,
        0x1111111111111111,
        0x2222222222222222,
    );
    format!("{:x}", build_hasher.hash_one(query))
}

/// Structure to hold cache data for a session
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionCache {
    /// Session identifier
    pub session_id: String,
    /// Query hash for this cache
    pub query_hash: String,
    /// Set of block identifiers that have been seen in this session
    /// Format: "file.rs:23-45" (file path with start-end line numbers)
    pub block_identifiers: HashSet<String>,
    /// Map of file paths to their MD5 hashes for cache invalidation
    /// Key: normalized file path, Value: MD5 hash of file contents
    #[serde(default)]
    pub file_md5_hashes: HashMap<String, String>,
}

/// Calculate MD5 hash of a file's contents
pub fn calculate_file_md5(file_path: &Path) -> Result<String> {
    let mut file = File::open(file_path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    let digest = md5::compute(&contents);
    Ok(format!("{digest:x}"))
}

// Test comment to trigger MD5 change

impl SessionCache {
    /// Create a new session cache with the given ID and query hash
    pub fn new(session_id: String, query_hash: String) -> Self {
        Self {
            session_id,
            query_hash,
            block_identifiers: HashSet::new(),
            file_md5_hashes: HashMap::new(),
        }
    }

    /// Load a session cache from disk and validate file MD5 hashes
    pub fn load(session_id: &str, query_hash: &str) -> Result<Self> {
        let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
        let cache_path = Self::get_cache_path(session_id, query_hash);

        // If the cache file doesn't exist, create a new empty cache
        if !cache_path.exists() {
            if debug_mode {
                println!("DEBUG: Cache file does not exist at {cache_path:?}, creating new cache");
            }
            return Ok(Self::new(session_id.to_string(), query_hash.to_string()));
        }

        if debug_mode {
            println!("DEBUG: Loading cache from {cache_path:?}");
        }

        // Read the cache file
        let mut file = match File::open(&cache_path) {
            Ok(f) => f,
            Err(e) => {
                if debug_mode {
                    println!("DEBUG: Error opening cache file: {e}");
                }
                return Ok(Self::new(session_id.to_string(), query_hash.to_string()));
            }
        };

        let mut contents = String::new();
        if let Err(e) = file.read_to_string(&mut contents) {
            if debug_mode {
                println!("DEBUG: Error reading cache file: {e}");
            }
            return Ok(Self::new(session_id.to_string(), query_hash.to_string()));
        }

        // Parse the JSON
        let mut cache: SessionCache = match serde_json::from_str(&contents) {
            Ok(cache) => cache,
            Err(e) => {
                if debug_mode {
                    println!("DEBUG: Error parsing cache JSON: {e}");
                }
                return Ok(Self::new(session_id.to_string(), query_hash.to_string()));
            }
        };

        if debug_mode {
            println!(
                "DEBUG: Successfully loaded cache with {} entries and {} file hashes",
                cache.block_identifiers.len(),
                cache.file_md5_hashes.len()
            );
        }

        // Validate and invalidate cache entries based on file MD5 changes
        cache.validate_and_invalidate_cache(debug_mode)?;

        Ok(cache)
    }

    /// Save the session cache to disk
    pub fn save(&self) -> Result<()> {
        let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
        let cache_path = Self::get_cache_path(&self.session_id, &self.query_hash);

        if debug_mode {
            println!(
                "DEBUG: Saving cache with {} entries to {:?}",
                self.block_identifiers.len(),
                cache_path
            );
        }

        // Ensure the cache directory exists
        if let Some(parent) = cache_path.parent() {
            if let Err(e) = create_dir_all(parent) {
                if debug_mode {
                    println!("DEBUG: Error creating cache directory: {e}");
                }
                return Err(e.into());
            }
        }

        // Serialize the cache to JSON
        let json = match serde_json::to_string_pretty(self) {
            Ok(j) => j,
            Err(e) => {
                if debug_mode {
                    println!("DEBUG: Error serializing cache to JSON: {e}");
                }
                return Err(e.into());
            }
        };

        // Write to the cache file
        match File::create(&cache_path) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(json.as_bytes()) {
                    if debug_mode {
                        println!("DEBUG: Error writing to cache file: {e}");
                    }
                    return Err(e.into());
                }
            }
            Err(e) => {
                if debug_mode {
                    println!("DEBUG: Error creating cache file: {e}");
                }
                return Err(e.into());
            }
        }

        if debug_mode {
            println!("DEBUG: Successfully saved cache to disk");
        }

        Ok(())
    }

    /// Check if a block identifier is in the cache
    pub fn is_cached(&self, block_id: &str) -> bool {
        self.block_identifiers.contains(block_id)
    }

    /// Add a block identifier to the cache
    pub fn add_to_cache(&mut self, block_id: String) {
        self.block_identifiers.insert(block_id);
    }

    /// Validate cache entries against current file MD5 hashes and invalidate if changed
    pub fn validate_and_invalidate_cache(&mut self, debug_mode: bool) -> Result<()> {
        let mut invalidated_files = HashSet::new();
        let mut blocks_to_remove = HashSet::new();

        // Check each cached file's MD5 hash
        for (file_path, cached_md5) in &self.file_md5_hashes {
            let current_path = Path::new(file_path);

            // Skip if file no longer exists
            if !current_path.exists() {
                if debug_mode {
                    println!("DEBUG: File no longer exists, invalidating cache: {file_path}");
                }
                invalidated_files.insert(file_path.clone());
                continue;
            }

            // Calculate current MD5
            match calculate_file_md5(current_path) {
                Ok(current_md5) => {
                    if &current_md5 != cached_md5 {
                        if debug_mode {
                            println!("DEBUG: File MD5 changed, invalidating cache: {file_path} (was: {cached_md5}, now: {current_md5})");
                        }
                        invalidated_files.insert(file_path.clone());
                    }
                }
                Err(e) => {
                    if debug_mode {
                        println!(
                            "DEBUG: Error calculating MD5 for {file_path}, invalidating cache: {e}"
                        );
                    }
                    invalidated_files.insert(file_path.clone());
                }
            }
        }

        // Remove block identifiers from invalidated files
        for file_path in &invalidated_files {
            let normalized_path = normalize_path(file_path);

            // Find all block identifiers that belong to this file
            for block_id in &self.block_identifiers {
                if let Some(colon_pos) = block_id.find(':') {
                    let block_file_part = &block_id[..colon_pos];
                    let normalized_block_file = normalize_path(block_file_part);

                    if normalized_block_file == normalized_path {
                        blocks_to_remove.insert(block_id.clone());
                    }
                }
            }
        }

        // Remove invalidated blocks from cache
        for block_id in &blocks_to_remove {
            self.block_identifiers.remove(block_id);
        }

        // Remove invalidated file hashes
        for file_path in &invalidated_files {
            self.file_md5_hashes.remove(file_path);
        }

        if debug_mode && (!invalidated_files.is_empty() || !blocks_to_remove.is_empty()) {
            println!(
                "DEBUG: Invalidated {} files and removed {} cached blocks due to MD5 changes",
                invalidated_files.len(),
                blocks_to_remove.len()
            );
        }

        Ok(())
    }

    /// Update the MD5 hash for a file
    pub fn update_file_md5(&mut self, file_path: &str) -> Result<()> {
        let normalized_path = normalize_path(file_path);
        let path = Path::new(&normalized_path);

        if path.exists() {
            let md5_hash = calculate_file_md5(path)?;
            self.file_md5_hashes.insert(normalized_path, md5_hash);
        }

        Ok(())
    }

    /// Get the path to the cache file
    pub fn get_cache_path(session_id: &str, query_hash: &str) -> PathBuf {
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home_dir
            .join(".cache")
            .join("probe")
            .join("sessions")
            .join(format!("{session_id}_{query_hash}.json"))
    }
}
/// Normalize a file path for consistent cache keys
/// Removes leading "./" and ensures consistent format
fn normalize_path(path: &str) -> String {
    // Remove leading "./"
    let normalized = if let Some(stripped) = path.strip_prefix("./") {
        stripped
    } else {
        path
    };

    normalized.to_string()
}

/// Generate a cache key for a search result
/// Format: "file.rs:23-45" (file path with start-end line numbers)
pub fn generate_cache_key(result: &SearchResult) -> String {
    let normalized_path = normalize_path(&result.file);
    format!("{normalized_path}:{}-{}", result.lines.0, result.lines.1)
}

/// Filter search results using the cache without adding to the cache
pub fn filter_results_with_cache(
    results: &[SearchResult],
    session_id: &str,
    query: &str,
) -> Result<(Vec<SearchResult>, usize)> {
    let query_hash = hash_query(query);
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    // Check if this is a new session by looking for the cache file
    let cache_path = SessionCache::get_cache_path(session_id, &query_hash);
    let is_new_session = !cache_path.exists();

    // For a new session, don't skip any results
    if is_new_session {
        if debug_mode {
            println!("DEBUG: New session, not filtering results");
        }
        // Return all results with no skipped blocks
        return Ok((results.to_vec(), 0));
    }

    // Load the cache
    let cache = SessionCache::load(session_id, &query_hash)?;

    // If the cache is empty, don't skip any results
    if cache.block_identifiers.is_empty() {
        if debug_mode {
            println!("DEBUG: Cache is empty, not filtering results");
        }
        return Ok((results.to_vec(), 0));
    }

    if debug_mode {
        println!(
            "DEBUG: Filtering {} results against {} cached blocks",
            results.len(),
            cache.block_identifiers.len()
        );
    }

    // Count of skipped blocks
    let mut skipped_count = 0;

    // For existing sessions, filter the results
    let filtered_results: Vec<SearchResult> = results
        .iter()
        .filter(|result| {
            let cache_key = generate_cache_key(result);
            let is_cached = cache.is_cached(&cache_key);

            if is_cached {
                if debug_mode && skipped_count < 5 {
                    println!("DEBUG: Skipping cached block: {cache_key}");
                }
                skipped_count += 1;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect();

    if debug_mode {
        println!(
            "DEBUG: Filtered out {} cached blocks, returning {} results",
            skipped_count,
            filtered_results.len()
        );
    }

    Ok((filtered_results, skipped_count))
}

/// Filter matched lines using the cache to skip already cached blocks
/// This is applied early in the search process, right after ripgrep results
pub fn filter_matched_lines_with_cache(
    file_term_map: &mut HashMap<PathBuf, HashMap<usize, HashSet<usize>>>,
    session_id: &str,
    query: &str,
) -> Result<usize> {
    let query_hash = hash_query(query);
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    // Check if this is a new session by looking for the cache file
    let cache_path = SessionCache::get_cache_path(session_id, &query_hash);
    let is_new_session = !cache_path.exists();

    // For a new session, don't skip any lines
    if is_new_session {
        if debug_mode {
            println!("DEBUG: New session, not filtering matched lines");
        }
        return Ok(0);
    }

    // Load the cache
    let cache = SessionCache::load(session_id, &query_hash)?;

    // If the cache is empty, don't skip any lines
    if cache.block_identifiers.is_empty() {
        if debug_mode {
            println!("DEBUG: Cache is empty, not filtering matched lines");
        }
        return Ok(0);
    }

    if debug_mode {
        println!(
            "DEBUG: Early filtering of matched lines against {} cached blocks",
            cache.block_identifiers.len()
        );
    }

    // Count of skipped lines
    let mut skipped_count = 0;
    let mut files_to_remove = Vec::new();

    // For each file in the map
    for (file_path, term_map) in file_term_map.iter_mut() {
        if term_map.is_empty() {
            continue;
        }

        // Get all matched lines for this file
        let mut all_lines = HashSet::new();
        for lineset in term_map.values() {
            all_lines.extend(lineset.iter());
        }

        if debug_mode {
            println!(
                "DEBUG: File {:?} has {} matched lines before filtering",
                file_path,
                all_lines.len()
            );
        }

        // Check each line against the cache
        let mut lines_to_remove = HashSet::new();
        for &line_num in &all_lines {
            // Create a simple cache key for this line
            // Format: "file.rs:line_num"
            let path_str = file_path.to_string_lossy();
            let normalized_path = normalize_path(&path_str);
            let line_cache_key = format!("{normalized_path}:{line_num}");

            // Check if this line is part of a cached block
            let is_cached = cache.block_identifiers.iter().any(|block_id| {
                // Parse the block ID to get file and line range
                if let Some(colon_pos) = block_id.find(':') {
                    if let Some(dash_pos) = block_id[colon_pos + 1..].find('-') {
                        let file_part = &block_id[..colon_pos];
                        let start_line_str = &block_id[colon_pos + 1..colon_pos + 1 + dash_pos];
                        let end_line_str = &block_id[colon_pos + 1 + dash_pos + 1..];

                        if let (Ok(start_line), Ok(end_line)) = (
                            start_line_str.parse::<usize>(),
                            end_line_str.parse::<usize>(),
                        ) {
                            // Check if this line is within a cached block from the same file
                            let path_str = file_path.to_string_lossy();
                            let normalized_path = normalize_path(&path_str);
                            let normalized_file_part = normalize_path(file_part);

                            return normalized_file_part == normalized_path
                                && line_num >= start_line
                                && line_num <= end_line;
                        }
                    }
                }
                false
            });

            if is_cached {
                if debug_mode && skipped_count < 5 {
                    println!("DEBUG: Skipping cached line: {line_cache_key}");
                }
                lines_to_remove.insert(line_num);
                skipped_count += 1;
            }
        }

        // Remove cached lines from each term's line set
        for term_lines in term_map.values_mut() {
            for line in &lines_to_remove {
                term_lines.remove(line);
            }
        }

        // Remove terms with empty line sets
        term_map.retain(|_, lines| !lines.is_empty());

        // Mark file for removal if all terms have been removed
        if term_map.is_empty() {
            files_to_remove.push(file_path.clone());
        }

        if debug_mode {
            let remaining_lines: HashSet<_> =
                term_map.values().flat_map(|lines| lines.iter()).collect();
            println!(
                "DEBUG: File {:?} has {} matched lines after filtering",
                file_path,
                remaining_lines.len()
            );
        }
    }

    // Remove files with no remaining terms
    for file in files_to_remove {
        file_term_map.remove(&file);
    }

    if debug_mode {
        println!(
            "DEBUG: Early filtering removed {} cached lines, {} files remain",
            skipped_count,
            file_term_map.len()
        );
    }

    Ok(skipped_count)
}

/// Add search results to the cache
pub fn add_results_to_cache(results: &[SearchResult], session_id: &str, query: &str) -> Result<()> {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
    let query_hash = hash_query(query);

    // Load or create the cache
    let mut cache = SessionCache::load(session_id, &query_hash)?;

    if debug_mode {
        println!(
            "DEBUG: Adding {} results to cache for session {}",
            results.len(),
            session_id
        );
        println!(
            "DEBUG: Cache had {} entries before update",
            cache.block_identifiers.len()
        );
    }

    // Track unique files to update MD5 hashes
    let mut unique_files = HashSet::new();

    // Add all results to the cache
    let mut new_entries = 0;
    for result in results {
        let cache_key = generate_cache_key(result);
        if !cache.is_cached(&cache_key) {
            new_entries += 1;
            if debug_mode && new_entries <= 5 {
                println!("DEBUG: Adding new cache entry: {cache_key}");
            }
        }
        cache.add_to_cache(cache_key);

        // Track this file for MD5 update
        unique_files.insert(normalize_path(&result.file));
    }

    // Update MD5 hashes for all files in this batch
    for file_path in &unique_files {
        if let Err(e) = cache.update_file_md5(file_path) {
            if debug_mode {
                println!("DEBUG: Failed to update MD5 for {file_path}: {e}");
            }
        }
    }

    if debug_mode {
        println!("DEBUG: Added {new_entries} new entries to cache");
        println!("DEBUG: Updated MD5 hashes for {} files", unique_files.len());
        println!(
            "DEBUG: Cache now has {} entries and {} file hashes",
            cache.block_identifiers.len(),
            cache.file_md5_hashes.len()
        );
    }

    // Save the updated cache
    cache.save()?;

    Ok(())
}

/// Debug function to print cache contents (only used when DEBUG=1)
pub fn debug_print_cache(session_id: &str, query: &str) -> Result<()> {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
    if !debug_mode {
        return Ok(());
    }

    let query_hash = hash_query(query);
    let cache = SessionCache::load(session_id, &query_hash)?;

    println!("DEBUG: Cache for session {session_id} with query hash {query_hash}");
    println!(
        "DEBUG: Contains {} cached blocks",
        cache.block_identifiers.len()
    );

    for (i, block_id) in cache.block_identifiers.iter().enumerate().take(10) {
        println!("DEBUG: Cached block {i}: {block_id}");
    }

    if cache.block_identifiers.len() > 10 {
        let _remaining = cache.block_identifiers.len() - 10;
        println!("DEBUG: ... and {} more", cache.block_identifiers.len() - 10);
    }

    Ok(())
}

/// Generate a unique 4-character alphanumeric session ID
/// Returns a tuple of (session_id, is_new) where is_new indicates if this is a newly generated ID
pub fn generate_session_id() -> Result<(&'static str, bool)> {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    // Generate a single session ID instead of looping
    if (0..10).next().is_some() {
        // Generate a deterministic 4-character session ID based on process info
        use ahash::RandomState;
        use std::hash::{BuildHasher, Hash, Hasher};

        let build_hasher = RandomState::with_seeds(12345, 67890, 11111, 22222);
        let mut hasher = build_hasher.build_hasher();
        std::process::id().hash(&mut hasher);
        "PROBE_SESSION_SALT_2024".hash(&mut hasher);

        // Use day-level timestamp for cache invalidation while maintaining determinism within the day
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let day_timestamp = now.as_secs() / 86400; // Day-level granularity
        day_timestamp.hash(&mut hasher);

        let session_id = format!("{:04x}", hasher.finish() & 0xFFFF);

        // Convert to lowercase for consistency
        let session_id = session_id.to_lowercase();

        if debug_mode {
            println!("DEBUG: Generated session ID: {session_id}");
        }

        // We don't check for existing cache files here since we're just generating a session ID
        // The actual cache file will be created with both session ID and query hash
        if debug_mode {
            println!("DEBUG: Generated new session ID: {session_id}");
        }
        // Convert to a static string (this leaks memory, but it's a small amount and only happens once per session)
        let static_id: &'static str = Box::leak(session_id.into_boxed_str());
        return Ok((static_id, true));
    }

    // If we couldn't generate a unique ID after 10 attempts, return an error
    Err(anyhow::anyhow!(
        "Failed to generate a unique session ID after multiple attempts"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use probe_code::models::SearchResult;

    #[test]
    fn test_path_normalization() {
        // Test that normalize_path removes leading "./"
        assert_eq!(normalize_path("./path/to/file.rs"), "path/to/file.rs");
        assert_eq!(normalize_path("path/to/file.rs"), "path/to/file.rs");
    }

    #[test]
    fn test_query_hashing() {
        // Test that different queries produce different hashes
        let hash1 = hash_query("query1");
        let hash2 = hash_query("query2");
        assert_ne!(hash1, hash2);

        // Test that the same query produces the same hash
        let hash3 = hash_query("query1");
        assert_eq!(hash1, hash3);
    }

    #[test]
    fn test_cache_key_generation_with_different_path_formats() {
        // Create two search results with the same path but different formats
        let result1 = SearchResult {
            file: "./path/to/file.rs".to_string(),
            lines: (10, 20),
            node_type: "function".to_string(),
            code: "".to_string(),
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

        let result2 = SearchResult {
            file: "path/to/file.rs".to_string(),
            lines: (10, 20),
            node_type: "function".to_string(),
            code: "".to_string(),
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

        // Generate cache keys for both results
        let key1 = generate_cache_key(&result1);
        let key2 = generate_cache_key(&result2);

        // The cache keys should be identical
        assert_eq!(key1, key2);
        assert_eq!(key1, "path/to/file.rs:10-20");
    }

    #[test]
    fn test_session_cache_with_query_hash() {
        // Test that different queries for the same session have different cache paths
        let session_id = "test_session";
        let query1 = "query1";
        let query2 = "query2";

        let hash1 = hash_query(query1);
        let hash2 = hash_query(query2);

        let path1 = SessionCache::get_cache_path(session_id, &hash1);
        let path2 = SessionCache::get_cache_path(session_id, &hash2);

        // Paths should be different for different queries
        assert_ne!(path1, path2);

        // Create caches with different queries
        let cache1 = SessionCache::new(session_id.to_string(), hash1);
        let cache2 = SessionCache::new(session_id.to_string(), hash2);

        // Caches should have different query hashes
        assert_ne!(cache1.query_hash, cache2.query_hash);
    }
}
