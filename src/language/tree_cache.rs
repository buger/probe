use anyhow::{Context, Result};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use tree_sitter::Tree;

// PHASE 4 OPTIMIZATION: Use environment variable or CPU count for cache size
const DEFAULT_CACHE_SIZE: usize = 2000;

lazy_static::lazy_static! {
    /// A cache for parsed syntax trees to avoid redundant parsing
    ///
    /// This cache stores parsed ASTs keyed by file path and content hash.
    /// When the same file is parsed multiple times, this avoids the overhead
    /// of re-parsing unchanged files.
    /// PHASE 4 OPTIMIZATION: Use LRU cache with bounded size to prevent memory bloat
    static ref TREE_CACHE: Mutex<LruCache<String, (Tree, u64)>> = {
        let cache_size = std::env::var("PROBE_TREE_CACHE_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_CACHE_SIZE);
        Mutex::new(LruCache::new(NonZeroUsize::new(cache_size).unwrap()))
    };

    /// A counter for cache hits, used for testing
    static ref CACHE_HITS: Mutex<usize> = Mutex::new(0);

    /// A mutex for test synchronization to prevent concurrent test execution
    static ref TEST_MUTEX: Mutex<()> = Mutex::new(());
}

/// Compute a deterministic hash of the content for cache validation
///
/// This uses the same FNV-1a algorithm as the line map cache to ensure
/// consistent tree cache behavior across program runs.
fn compute_content_hash(content: &str) -> u64 {
    // FNV-1a hash algorithm - fast and deterministic
    // Constants for 64-bit FNV-1a (same as parser.rs)
    const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in content.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Get a cached tree if available, otherwise parse and cache the result
///
/// This function checks if a valid cached tree exists for the given file path
/// and content. If found and the content hash matches, it returns the cached tree.
/// Otherwise, it parses the content, caches the result, and returns the new tree.
///
/// # Arguments
///
/// * `file_path` - The path of the file being parsed
/// * `content` - The content to parse
/// * `parser` - The tree-sitter parser to use if parsing is needed
///
/// # Returns
///
/// A Result containing the parsed tree, either from cache or freshly parsed
pub fn get_or_parse_tree(
    file_path: &str,
    content: &str,
    parser: &mut tree_sitter::Parser,
) -> Result<Tree> {
    let content_hash = compute_content_hash(content);

    // Check if debug mode is enabled
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // Try to get from cache first
    {
        let mut cache = TREE_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // PHASE 4 OPTIMIZATION: Use peek to check without updating LRU order first
        if let Some((_cached_tree, cached_hash)) = cache.peek(file_path) {
            if cached_hash == &content_hash {
                // Increment cache hit counter
                {
                    let mut hits = CACHE_HITS
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *hits += 1;
                }

                if debug_mode {
                    eprintln!("[DEBUG] Cache hit for file: {file_path}");
                }
                // PHASE 4 OPTIMIZATION: Now update LRU order with get
                let (cached_tree, _) = cache.get(file_path).unwrap();
                return Ok(cached_tree.clone());
            } else {
                // Content changed, explicitly remove the old entry
                // PHASE 4 OPTIMIZATION: LRU cache uses pop instead of remove
                cache.pop(file_path);
                if debug_mode {
                    eprintln!("[DEBUG] Cache invalidated for file: {file_path} (content changed)");
                }
            }
        } else if debug_mode {
            eprintln!("[DEBUG] Cache miss for file: {file_path}");
        }
    }

    // Not in cache or content changed, parse and store
    let tree = parser
        .parse(content, None)
        .context(format!("Failed to parse file: {file_path}"))?;

    // Store in cache
    {
        let mut cache = TREE_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // PHASE 4 OPTIMIZATION: LRU cache automatically evicts old entries
        cache.put(file_path.to_string(), (tree.clone(), content_hash));

        if debug_mode {
            eprintln!("[DEBUG] Cached parsed tree for file: {file_path}");
            eprintln!("[DEBUG] Current cache size: {} entries", cache.len());
        }
    }

    Ok(tree)
}

/// Get a cached tree if available, otherwise parse using a pooled parser and cache the result
///
/// This is an optimized version of `get_or_parse_tree` that uses the parser pool to avoid
/// the expensive overhead of creating and configuring parsers for each file. This function
/// automatically manages parser acquisition and return, providing significant performance
/// improvements for batch processing operations.
///
/// # Arguments
///
/// * `file_path` - The path of the file being parsed (used for cache key)
/// * `content` - The content to parse
/// * `extension` - The file extension to determine the language and parser type
///
/// # Returns
///
/// A Result containing the parsed tree, either from cache or freshly parsed using a pooled parser
pub fn get_or_parse_tree_pooled(file_path: &str, content: &str, extension: &str) -> Result<Tree> {
    use crate::language::parser_pool::{get_pooled_parser, return_pooled_parser};

    let content_hash = compute_content_hash(content);

    // Check if debug mode is enabled
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // Try to get from cache first
    {
        let mut cache = TREE_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // PHASE 4 OPTIMIZATION: Use peek to check without updating LRU order first
        if let Some((_cached_tree, cached_hash)) = cache.peek(file_path) {
            if cached_hash == &content_hash {
                // Increment cache hit counter
                {
                    let mut hits = CACHE_HITS
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *hits += 1;
                }

                if debug_mode {
                    eprintln!("[DEBUG] Tree cache hit for file: {file_path}");
                }
                // PHASE 4 OPTIMIZATION: Now update LRU order with get
                let (cached_tree, _) = cache.get(file_path).unwrap();
                return Ok(cached_tree.clone());
            } else {
                // Content changed, explicitly remove the old entry
                // PHASE 4 OPTIMIZATION: LRU cache uses pop instead of remove
                cache.pop(file_path);
                if debug_mode {
                    println!(
                        "[DEBUG] Tree cache invalidated for file: {file_path} (content changed)"
                    );
                }
            }
        } else if debug_mode {
            eprintln!("[DEBUG] Tree cache miss for file: {file_path}");
        }
    }

    // Not in cache or content changed, get a pooled parser and parse
    let mut parser = get_pooled_parser(extension).context(format!(
        "Failed to get pooled parser for extension: {extension}"
    ))?;

    let tree = parser
        .parse(content, None)
        .context(format!("Failed to parse file: {file_path}"))?;

    // Return parser to pool
    return_pooled_parser(extension, parser);

    // Store in cache
    {
        let mut cache = TREE_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // PHASE 4 OPTIMIZATION: LRU cache automatically evicts old entries
        cache.put(file_path.to_string(), (tree.clone(), content_hash));

        if debug_mode {
            eprintln!("[DEBUG] Cached parsed tree for file: {file_path}");
            eprintln!("[DEBUG] Current tree cache size: {} entries", cache.len());
        }
    }

    Ok(tree)
}

/// Clear the entire tree cache
///
/// This function can be used to free memory or force re-parsing of all files.
#[allow(dead_code)]
pub fn clear_tree_cache() {
    let mut cache = TREE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        eprintln!("[DEBUG] Clearing tree cache ({} entries)", cache.len());
    }

    cache.clear();

    // Also reset the cache hit counter
    let mut hits = CACHE_HITS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *hits = 0;
}

/// Remove a specific file from the tree cache
///
/// # Arguments
///
/// * `file_path` - The path of the file to remove from the cache
#[allow(dead_code)]
pub fn invalidate_cache_entry(file_path: &str) {
    let mut cache = TREE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if cache.pop(file_path).is_some() && debug_mode {
        eprintln!("[DEBUG] Removed file from cache: {file_path}");
    }
}

/// Acquire the test mutex for test synchronization
///
/// This function is used by tests to prevent concurrent access to the cache
/// during test execution, which can lead to flaky tests.
#[allow(dead_code)]
pub fn acquire_test_mutex() -> std::sync::MutexGuard<'static, ()> {
    TEST_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Get the current size of the tree cache
#[allow(dead_code)]
pub fn get_cache_size() -> usize {
    let cache = TREE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.len()
}

/// Check if a specific file exists in the cache
#[allow(dead_code)]
pub fn is_in_cache(file_path: &str) -> bool {
    let cache = TREE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.contains(file_path)
}

/// Reset the cache hit counter (for testing)
#[allow(dead_code)]
pub fn reset_cache_hit_counter() {
    let mut hits = CACHE_HITS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *hits = 0;
}

/// Get the current cache hit count (for testing)
#[allow(dead_code)]
pub fn get_cache_hit_count() -> usize {
    let hits = CACHE_HITS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *hits
}
