use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use tree_sitter::Parser;

use crate::language::factory;
use crate::search::file_list_cache;

// PHASE 4 OPTIMIZATION: Dynamic pool sizing based on CPU cores
const DEFAULT_MAX_PARSERS_PER_LANGUAGE: usize = 4;

fn get_max_parsers_per_language() -> usize {
    std::env::var("PROBE_PARSER_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::cmp::max(
                rayon::current_num_threads(),
                DEFAULT_MAX_PARSERS_PER_LANGUAGE,
            )
        })
}

lazy_static::lazy_static! {
    /// A thread-safe pool of tree-sitter parsers organized by language extension
    ///
    /// This pool maintains pre-configured parsers for each supported language to avoid
    /// the expensive overhead of creating new parsers and setting their language for every file.
    ///
    /// The pool is keyed by file extension (e.g., "rs", "js", "py") and contains a vector
    /// of ready-to-use parsers that have already been configured with the appropriate
    /// tree-sitter language grammar.
    static ref PARSER_POOL: Mutex<HashMap<String, Vec<Parser>>> = Mutex::new(HashMap::new());

    // PHASE 4 OPTIMIZATION: Pre-warm parsers for supported languages
    static ref PARSER_WARMER: () = {
        if std::env::var("PROBE_NO_PARSER_WARMUP").is_err() {
            // Tier 1: Critical languages - used most frequently, warm immediately
            let critical_languages = ["rs", "js", "ts", "py", "go", "java"];

            // Tier 2: Common languages - warm with lower priority
            let common_languages = ["cpp", "c", "jsx", "tsx", "rb", "php", "cs"];

            // Tier 3: Specialized languages - warm last
            let specialized_languages = ["swift", "h", "cc", "cxx", "hpp", "hxx"];

            // Create a single parser per language to initialize the pool
            // This reduces startup latency for the first file of each type
            let all_tiers = [critical_languages.as_slice(), common_languages.as_slice(), specialized_languages.as_slice()];

            for tier in &all_tiers {
                for lang in *tier {
                    if let Ok(parser) = get_pooled_parser(lang) {
                        return_pooled_parser(lang, parser);
                    }
                }
            }
        }
    };
}

/// Initialize parser pool warming
pub fn warm_parser_pool() {
    let _ = &*PARSER_WARMER;
}

/// Detect languages present in a directory by scanning file extensions
/// This is much faster than content-based detection and works for 99% of cases
fn detect_languages_in_directory(path: &Path) -> HashSet<String> {
    let mut detected_extensions = HashSet::new();

    // Use the existing file discovery system
    if let Ok(file_list) = file_list_cache::get_file_list(
        path,
        true,  // Include tests for complete detection
        &[],   // No custom ignores
        false, // Respect gitignore
    ) {
        for file in &file_list.files {
            if let Some(extension) = file.extension() {
                if let Some(ext_str) = extension.to_str() {
                    // Only collect extensions that we have language implementations for
                    if factory::get_language_impl(ext_str).is_some() {
                        detected_extensions.insert(ext_str.to_string());
                    }
                }
            }
        }
    }

    detected_extensions
}

/// Smart pre-warming: only warm parsers for languages detected in the target directory
/// This dramatically reduces memory usage while maintaining performance benefits
pub fn smart_warm_parser_pool_for_directory(path: &Path) {
    if std::env::var("PROBE_NO_PARSER_WARMUP").is_ok() {
        return;
    }

    let detected_languages = detect_languages_in_directory(path);

    if detected_languages.is_empty() {
        // Fallback to minimal warming if no supported languages detected
        let fallback_languages = ["rs", "js", "py"]; // Most common
        for lang in &fallback_languages {
            if let Ok(parser) = get_pooled_parser(lang) {
                return_pooled_parser(lang, parser);
            }
        }
        return;
    }

    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
    if debug_mode {
        println!("[DEBUG] Smart pre-warming detected languages: {detected_languages:?}");
    }

    // Prioritize languages by usage frequency (most common first)
    let priority_order = [
        "rs", "js", "ts", "py", "go", "java", // Tier 1: Critical
        "cpp", "c", "jsx", "tsx", "rb", "php", "cs", // Tier 2: Common
        "swift", "h", "cc", "cxx", "hpp", "hxx", // Tier 3: Specialized
    ];

    // Warm detected languages in priority order
    for lang in &priority_order {
        if detected_languages.contains(*lang) {
            if let Ok(parser) = get_pooled_parser(lang) {
                return_pooled_parser(lang, parser);
                if debug_mode {
                    println!("[DEBUG] Pre-warmed parser for language: {lang}");
                }
            }
        }
    }

    // Warm any remaining detected languages not in priority list
    for lang in &detected_languages {
        if !priority_order.contains(&lang.as_str()) {
            if let Ok(parser) = get_pooled_parser(lang) {
                return_pooled_parser(lang, parser);
                if debug_mode {
                    println!("[DEBUG] Pre-warmed parser for additional language: {lang}");
                }
            }
        }
    }
}

/// Gets a parser from the pool for the specified language extension.
///
/// If a pre-configured parser is available in the pool, it will be returned immediately.
/// Otherwise, a new parser will be created and configured with the appropriate language.
/// This function handles the expensive parser initialization and language configuration
/// so that callers receive a ready-to-use parser.
///
/// # Arguments
///
/// * `extension` - The file extension (e.g., "rs", "js", "py") that determines the language
///
/// # Returns
///
/// * `Ok(Parser)` - A configured parser ready for use
/// * `Err(anyhow::Error)` - If the language is not supported or parser setup fails
///
/// # Example
///
/// ```rust
/// let parser = get_pooled_parser("rs")?;
/// let tree = parser.parse(rust_code, None)?;
/// return_pooled_parser("rs", parser);
/// ```
pub fn get_pooled_parser(extension: &str) -> Result<Parser> {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    // First, try to get a parser from the pool
    {
        let mut pool = PARSER_POOL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(parsers) = pool.get_mut(extension) {
            if let Some(parser) = parsers.pop() {
                if debug_mode {
                    println!("[DEBUG] Parser pool: Retrieved pooled parser for extension '{extension}' (pool size: {})", parsers.len());
                }
                return Ok(parser);
            }
        }
    }

    // No parser available in pool, create a new one
    if debug_mode {
        println!("[DEBUG] Parser pool: Creating new parser for extension '{extension}'");
    }

    let language_impl = factory::get_language_impl(extension)
        .ok_or_else(|| anyhow::anyhow!("Unsupported language extension: {}", extension))?;

    let mut parser = Parser::new();
    parser
        .set_language(&language_impl.get_tree_sitter_language())
        .map_err(|e| anyhow::anyhow!("Failed to set parser language: {}", e))?;

    Ok(parser)
}

/// Returns a parser to the pool for reuse.
///
/// This function adds the parser back to the pool for the specified language extension,
/// making it available for future `get_pooled_parser` calls. This avoids the need to
/// recreate and reconfigure parsers, providing significant performance benefits.
///
/// # Arguments
///
/// * `extension` - The file extension the parser was configured for
/// * `parser` - The parser to return to the pool
///
/// # Note
///
/// The parser should be in a clean state (no active parsing context) when returned.
/// The pool will maintain a reasonable number of parsers per language to balance
/// performance with memory usage.
pub fn return_pooled_parser(extension: &str, parser: Parser) {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    let mut pool = PARSER_POOL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let parsers = pool.entry(extension.to_string()).or_default();

    // PHASE 4 OPTIMIZATION: Use dynamic pool sizing
    let max_parsers = get_max_parsers_per_language();

    if parsers.len() < max_parsers {
        parsers.push(parser);
        if debug_mode {
            println!(
                "[DEBUG] Parser pool: Returned parser for extension '{extension}' (pool size: {})",
                parsers.len()
            );
        }
    } else if debug_mode {
        println!("[DEBUG] Parser pool: Discarded parser for extension '{extension}' (pool at capacity: {})", parsers.len());
    }
}

/// Gets statistics about the current parser pool state.
///
/// This function is primarily useful for debugging and monitoring pool effectiveness.
/// It returns information about how many parsers are currently pooled for each language.
///
/// # Returns
///
/// A HashMap mapping language extensions to the number of pooled parsers
pub fn get_pool_stats() -> HashMap<String, usize> {
    let pool = PARSER_POOL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    pool.iter()
        .map(|(ext, parsers)| (ext.clone(), parsers.len()))
        .collect()
}

/// Clears the entire parser pool.
///
/// This function removes all pooled parsers, forcing future parser requests
/// to create new instances. This can be useful for testing or memory management.
#[allow(dead_code)]
pub fn clear_parser_pool() {
    let mut pool = PARSER_POOL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        let total_parsers: usize = pool.values().map(|v| v.len()).sum();
        println!(
            "[DEBUG] Parser pool: Clearing pool with {} parsers across {} languages",
            total_parsers,
            pool.len()
        );
    }

    pool.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Test isolation: Only one parser pool test can run at a time
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    /// Create a controlled test environment that bypasses global state
    fn with_isolated_pool<F, R>(test_fn: F) -> R
    where
        F: FnOnce() -> R,
    {
        // Handle poisoned mutex by recovering the lock
        let _lock = match TEST_MUTEX.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                // The mutex was poisoned by a panicking thread.
                // We can still use it, but we need to clear the poison.
                poisoned.into_inner()
            }
        };

        // Force trigger the warmer to ensure consistent state
        let _ = &*PARSER_WARMER;

        // Clear pool and run test
        clear_parser_pool();
        test_fn()
    }

    #[test]
    fn test_parser_pool_basic_functionality() {
        with_isolated_pool(|| {
            // Use PHP (not in warmup list) to avoid race conditions
            let test_lang = "php";
            let parser1 = get_pooled_parser(test_lang).expect("Should create PHP parser");

            // Return it to pool
            return_pooled_parser(test_lang, parser1);

            // Get another parser - should be the same one from pool
            let _parser2 = get_pooled_parser(test_lang).expect("Should get pooled PHP parser");

            // Check pool stats
            let stats = get_pool_stats();

            // Parser should be checked out (not available in pool)
            // Both behaviors (Some(&0) and None) indicate the parser is checked out
            let checked_out = stats.get(test_lang).is_none_or(|&count| count == 0);
            assert!(
                checked_out,
                "Parser should be checked out (expected pool size 0 or entry removed, got {:?})",
                stats.get(test_lang)
            );
        })
    }

    #[test]
    fn test_parser_pool_multiple_languages() {
        with_isolated_pool(|| {
            // Use languages not in warmup list to avoid race conditions
            let extensions = ["php", "rb", "cs", "h"];
            let mut parsers = Vec::new();

            // Get parsers for different languages
            for ext in &extensions {
                let parser =
                    get_pooled_parser(ext).unwrap_or_else(|_| panic!("Should create {ext} parser"));
                parsers.push((ext, parser));
            }

            // Return all parsers
            for (ext, parser) in parsers {
                return_pooled_parser(ext, parser);
            }

            // Check that we have some parsers in the pool
            // Note: Due to capacity limits, not all languages might be retained
            let stats = get_pool_stats();

            // At least some parsers should be in the pool
            let total_parsers: usize = stats.values().sum();
            assert!(
                total_parsers > 0,
                "Should have at least some parsers in the pool after returning them"
            );

            // The pool should track multiple languages (but maybe not all due to capacity)
            // Just verify it's working for multiple languages
            assert!(
                !stats.is_empty(),
                "Pool should have at least one language after returning parsers"
            );
        })
    }

    #[test]
    fn test_parser_pool_capacity_limit() {
        with_isolated_pool(|| {
            // Use a language not in warmup list to avoid race conditions
            let test_lang = "php";

            // Create more parsers than the pool capacity
            let mut parsers = Vec::new();
            for _ in 0..10 {
                let parser = get_pooled_parser(test_lang).expect("Should create PHP parser");
                parsers.push(parser);
            }

            // Return all parsers
            for parser in parsers {
                return_pooled_parser(test_lang, parser);
            }

            // Pool should be limited to the dynamic max parsers per language
            let max_parsers = get_max_parsers_per_language();
            let stats = get_pool_stats();
            // The pool may have entries for this language or not, depending on implementation
            if let Some(count) = stats.get(test_lang) {
                assert!(
                    *count <= max_parsers,
                    "Pool size {count} exceeds max {max_parsers}"
                );
            }
            // If there's no entry, that's also valid (pool may be empty)
        })
    }

    #[test]
    fn test_unsupported_extension() {
        let result = get_pooled_parser("unsupported");
        assert!(result.is_err());
        assert!(format!("{:?}", result.err().unwrap()).contains("Unsupported language extension"));
    }

    #[test]
    fn benchmark_parser_creation_performance() {
        use std::time::Instant;

        // Test languages across all tiers (using only supported ones)
        let test_languages = [
            // Tier 1 (should be fastest - pre-warmed)
            "rs", "js", "py", // Tier 2 (should be fast - pre-warmed)
            "cpp", "rb", "php", // Tier 3 (should be fast - pre-warmed)
            "cs", "h",
        ];

        println!("\n=== Parser Creation Performance Benchmark ===");

        // First test: Cold start (no pool)
        println!("\n--- Cold Start Performance (no pre-warming) ---");
        std::env::set_var("PROBE_NO_PARSER_WARMUP", "1");
        clear_parser_pool();

        let mut cold_times = Vec::new();
        for lang in &test_languages {
            let start = Instant::now();
            if let Ok(parser) = get_pooled_parser(lang) {
                let duration = start.elapsed();
                cold_times.push((*lang, duration));
                println!("{lang}: {duration:?}");
                return_pooled_parser(lang, parser);
            }
        }

        // Second test: Warm start (with pool)
        println!("\n--- Warm Start Performance (with pre-warming) ---");
        std::env::remove_var("PROBE_NO_PARSER_WARMUP");
        clear_parser_pool();

        // Force warming by accessing the lazy static
        let _ = &*PARSER_WARMER;

        let mut warm_times = Vec::new();
        for lang in &test_languages {
            let start = Instant::now();
            if let Ok(parser) = get_pooled_parser(lang) {
                let duration = start.elapsed();
                warm_times.push((*lang, duration));
                println!("{lang}: {duration:?}");
                return_pooled_parser(lang, parser);
            }
        }

        // Analysis
        println!("\n--- Performance Analysis ---");
        let cold_avg =
            cold_times.iter().map(|(_, d)| d.as_nanos()).sum::<u128>() / cold_times.len() as u128;
        let warm_avg =
            warm_times.iter().map(|(_, d)| d.as_nanos()).sum::<u128>() / warm_times.len() as u128;

        println!(
            "Cold start average: {:?}",
            std::time::Duration::from_nanos(cold_avg as u64)
        );
        println!(
            "Warm start average: {:?}",
            std::time::Duration::from_nanos(warm_avg as u64)
        );
        println!("Speedup: {:.2}x", cold_avg as f64 / warm_avg as f64);

        // Per-language comparison
        println!("\n--- Per-Language Comparison ---");
        for ((lang_cold, cold_time), (lang_warm, warm_time)) in
            cold_times.iter().zip(warm_times.iter())
        {
            assert_eq!(lang_cold, lang_warm);
            let speedup = cold_time.as_nanos() as f64 / warm_time.as_nanos() as f64;
            println!("{lang_cold}: cold={cold_time:?}, warm={warm_time:?}, speedup={speedup:.2}x");
        }

        // Memory usage estimation
        println!("\n--- Memory Usage Estimation ---");
        let stats = get_pool_stats();
        let total_parsers: usize = stats.values().sum();
        println!("Total parsers in pool: {total_parsers}");
        println!("Languages in pool: {}", stats.len());
        println!("Estimated memory usage: ~{}MB", total_parsers * 10); // ~10MB per parser estimate

        // Reset state
        std::env::remove_var("PROBE_NO_PARSER_WARMUP");
    }

    #[test]
    fn benchmark_smart_vs_full_prewarming() {
        use std::time::Instant;

        println!("\n=== Smart vs Full Pre-warming Benchmark ===");

        // Test with current directory (should detect Rust files)
        let current_dir = std::env::current_dir().unwrap();

        // Test 1: Full pre-warming (current approach)
        println!("\n--- Full Pre-warming Performance ---");
        let start = Instant::now();
        std::env::remove_var("PROBE_NO_PARSER_WARMUP");
        clear_parser_pool();

        // Force full warming
        let _ = &*PARSER_WARMER;
        let full_warming_time = start.elapsed();
        let full_stats = get_pool_stats();
        let full_memory = full_stats.values().sum::<usize>() * 10; // ~10MB per parser

        println!("Full warming time: {full_warming_time:?}");
        println!("Languages warmed: {}", full_stats.len());
        println!("Total parsers: {}", full_stats.values().sum::<usize>());
        println!("Estimated memory: ~{full_memory}MB");

        // Test 2: Smart pre-warming (new approach)
        println!("\n--- Smart Pre-warming Performance ---");
        let start = Instant::now();
        clear_parser_pool();

        smart_warm_parser_pool_for_directory(&current_dir);
        let smart_warming_time = start.elapsed();
        let smart_stats = get_pool_stats();
        let smart_memory = smart_stats.values().sum::<usize>() * 10; // ~10MB per parser

        println!("Smart warming time: {smart_warming_time:?}");
        println!("Languages warmed: {}", smart_stats.len());
        println!("Total parsers: {}", smart_stats.values().sum::<usize>());
        println!("Estimated memory: ~{smart_memory}MB");
        println!(
            "Detected languages: {:?}",
            smart_stats.keys().collect::<Vec<_>>()
        );

        // Test 3: Language detection performance
        println!("\n--- Language Detection Performance ---");
        let start = Instant::now();
        let detected = detect_languages_in_directory(&current_dir);
        let detection_time = start.elapsed();

        println!("Detection time: {detection_time:?}");
        println!("Detected extensions: {detected:?}");

        // Analysis
        println!("\n--- Performance Analysis ---");
        println!(
            "Memory savings: {}MB ({:.1}% reduction)",
            full_memory.saturating_sub(smart_memory),
            (full_memory.saturating_sub(smart_memory) as f64 / full_memory as f64) * 100.0
        );

        println!("Time comparison:");
        println!("  Full warming: {full_warming_time:?}");
        println!("  Smart warming: {smart_warming_time:?}");
        println!("  Detection overhead: {detection_time:?}");

        // Performance per language test
        println!("\n--- Performance Test: Parser Creation Speed ---");
        let test_languages = detected.iter().take(3).collect::<Vec<_>>(); // Test first 3 detected

        for lang in &test_languages {
            let start = Instant::now();
            if let Ok(parser) = get_pooled_parser(lang) {
                let duration = start.elapsed();
                println!("{lang}: {duration:?} (should be fast - pre-warmed)");
                return_pooled_parser(lang, parser);
            }
        }

        // Reset state
        std::env::remove_var("PROBE_NO_PARSER_WARMUP");
    }
}
