use anyhow::Result;
use ignore::WalkBuilder;
use lazy_static::lazy_static;
use probe_code::search::tokenization;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// A struct to hold the cached file list for a specific directory
#[derive(Debug, Clone)]
pub struct FileList {
    /// The list of files in the directory (respecting ignore patterns)
    pub files: Vec<PathBuf>,
    /// When this cache was created
    #[allow(dead_code)]
    pub created_at: Instant,
}

// Global in-memory cache for file lists
lazy_static! {
    static ref FILE_LIST_CACHE: RwLock<HashMap<String, Arc<FileList>>> =
        RwLock::new(HashMap::new());
}

/// Helper function to format duration in a human-readable way
fn format_duration(duration: std::time::Duration) -> String {
    if duration.as_millis() < 1000 {
        let duration_millis = duration.as_millis();
        format!("{duration_millis}ms")
    } else {
        let duration_secs = duration.as_secs_f64();
        format!("{duration_secs:.2}s")
    }
}

/// Generate a cache key for a specific directory and options
fn generate_cache_key(
    path: &Path,
    allow_tests: bool,
    custom_ignores: &[String],
    no_gitignore: bool,
) -> String {
    // Create a unique identifier for this cache based on the path and options
    let path_str = path.to_string_lossy();
    let allow_tests_str = if allow_tests {
        "with_tests"
    } else {
        "no_tests"
    };

    let gitignore_str = if no_gitignore {
        "no_gitignore"
    } else {
        "with_gitignore"
    };

    // Create a hash of the custom ignores to include in the cache key
    let ignores_hash = if custom_ignores.is_empty() {
        "no_ignores".to_string()
    } else {
        // Simple hash function for the custom ignores
        let mut hash = 0u64;
        for ignore in custom_ignores {
            for byte in ignore.bytes() {
                hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
            }
        }
        format!("ignores_{hash:x}")
    };

    format!("{path_str}_{allow_tests_str}_{ignores_hash}_{gitignore_str}")
}

/// Get a list of files in a directory, respecting ignore patterns and test file exclusions.
/// This function will use a cached list if available, or build and cache a new list if not.
pub fn get_file_list(
    path: &Path,
    allow_tests: bool,
    custom_ignores: &[String],
    no_gitignore: bool,
) -> Result<Arc<FileList>> {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
    let start_time = Instant::now();

    if debug_mode {
        println!("DEBUG: Getting file list for path: {path:?}");
        println!("DEBUG: allow_tests: {allow_tests}");
        println!("DEBUG: custom_ignores: {custom_ignores:?}");
        println!("DEBUG: no_gitignore: {no_gitignore}");
    }

    // Create a cache key for this request
    let cache_key = generate_cache_key(path, allow_tests, custom_ignores, no_gitignore);

    // Check if we have this file list in the cache
    {
        let cache = FILE_LIST_CACHE.read().unwrap();
        if let Some(file_list) = cache.get(&cache_key) {
            let elapsed = start_time.elapsed();
            if debug_mode {
                println!(
                    "DEBUG: Found file list in cache with {} files (retrieved in {})",
                    file_list.files.len(),
                    format_duration(elapsed)
                );
            }
            return Ok(Arc::clone(file_list));
        }
    }

    // If not in cache, build the file list
    if debug_mode {
        println!("DEBUG: File list not found in cache, building new list");
    }

    let file_list = build_file_list(path, allow_tests, custom_ignores, no_gitignore)?;
    let file_count = file_list.files.len();

    // Cache the file list
    let file_list = Arc::new(file_list);
    {
        let mut cache = FILE_LIST_CACHE.write().unwrap();
        cache.insert(cache_key, Arc::clone(&file_list));
    }

    let elapsed = start_time.elapsed();
    if debug_mode {
        println!(
            "DEBUG: Built and cached new file list with {} files in {}",
            file_count,
            format_duration(elapsed)
        );
    }

    Ok(file_list)
}

/// Build a list of files in a directory, respecting ignore patterns and test file exclusions.
fn build_file_list(
    path: &Path,
    allow_tests: bool,
    custom_ignores: &[String],
    no_gitignore: bool,
) -> Result<FileList> {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
    let start_time = Instant::now();

    if debug_mode {
        println!("DEBUG: Building file list for path: {path:?}");
    }

    // Create a WalkBuilder that respects .gitignore files and common ignore patterns
    let builder_start = Instant::now();
    let mut builder = WalkBuilder::new(path);

    // Configure the builder to conditionally respect gitignore files
    if !no_gitignore {
        builder.git_ignore(true);
        builder.git_global(true);
        builder.git_exclude(true);
        // IMPORTANT: Allow .gitignore files to work even outside git repositories
        // This makes the ignore crate work consistently regardless of whether
        // the directory is a git repository or not
        builder.require_git(false);
    } else {
        builder.git_ignore(false);
        builder.git_global(false);
        builder.git_exclude(false);
        if debug_mode {
            println!("DEBUG: Gitignore disabled - will not respect .gitignore files");
        }
    }

    // Enable parallel walking for large directories
    builder.threads(rayon::current_num_threads());

    // Add common directories to ignore
    let mut common_ignores: Vec<String> = vec![
        "node_modules",
        "vendor",
        "target",
        "dist",
        "build",
        ".git",
        ".svn",
        ".hg",
        ".idea",
        ".vscode",
        "__pycache__",
        "*.pyc",
        "*.pyo",
        "*.class",
        "*.o",
        "*.obj",
        "*.a",
        "*.lib",
        "*.so",
        "*.dylib",
        "*.dll",
        "*.exe",
        "*.out",
        "*.app",
        "*.jar",
        "*.war",
        "*.ear",
        "*.zip",
        "*.tar.gz",
        "*.rar",
        "*.log",
        "*.tmp",
        "*.temp",
        "*.swp",
        "*.swo",
        "*.bak",
        "*.orig",
        "*.DS_Store",
        "Thumbs.db",
        "*.yml",
        "*.yaml",
        "*.json",
        "*.tconf",
        "*.conf",
        "go.sum",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    // Add test file patterns if allow_tests is false
    if !allow_tests {
        let test_patterns: Vec<String> = vec![
            "*_test.rs",
            "*_tests.rs",
            "test_*.rs",
            "tests.rs",
            "*.spec.js",
            "*.test.js",
            "*.spec.ts",
            "*.test.ts",
            "*.spec.jsx",
            "*.test.jsx",
            "*.spec.tsx",
            "*.test.tsx",
            "test_*.py",
            "*_test.go",
            "test_*.c",
            "*_test.c",
            "*_test.cpp",
            "*_test.cc",
            "*_test.cxx",
            "*Test.java",
            "*_test.rb",
            "test_*.rb",
            "*_spec.rb",
            "*Test.php",
            "test_*.php",
            "**/tests/**",
            "**/test/**",
            "**/__tests__/**",
            "**/__test__/**",
            "**/spec/**",
            "**/specs/**",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        common_ignores.extend(test_patterns);
    }

    // Add custom ignore patterns to the common ignores
    for pattern in custom_ignores {
        common_ignores.push(pattern.clone());
    }

    // Create a single override builder for all ignore patterns
    let mut override_builder = ignore::overrides::OverrideBuilder::new(path);

    // Add all ignore patterns to the override builder
    for pattern in &common_ignores {
        if let Err(err) = override_builder.add(&format!("!{pattern}")) {
            eprintln!("Error adding ignore pattern {pattern:?}: {err}");
        }
    }

    // Build and apply the overrides
    match override_builder.build() {
        Ok(overrides) => {
            builder.overrides(overrides);
        }
        Err(err) => {
            eprintln!("Error building ignore overrides: {err}");
        }
    }

    let builder_duration = builder_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Builder configuration completed in {}",
            format_duration(builder_duration)
        );
    }

    // Collect files
    let walk_start = Instant::now();
    let mut files = Vec::new();
    let mut total_files = 0;

    for result in builder.build() {
        total_files += 1;
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("Error walking directory: {err}");
                continue;
            }
        };

        // Skip directories
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        files.push(entry.path().to_path_buf());
    }

    // Sort files for deterministic ordering to fix non-deterministic behavior
    // This ensures that file discovery is consistent across runs
    files.sort();

    let walk_duration = walk_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Directory walk completed in {} - Found {} files out of {} entries (sorted for determinism)",
            format_duration(walk_duration),
            files.len(),
            total_files
        );
    }

    let total_duration = start_time.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Total file list building completed in {}",
            format_duration(total_duration)
        );
    }

    Ok(FileList {
        files,
        created_at: Instant::now(),
    })
}

/// Find files whose names match query words
/// Returns a map of file paths to the term indices that matched the filename
#[allow(clippy::too_many_arguments)]
pub fn find_matching_filenames(
    path: &Path,
    queries: &[String],
    already_found_files: &HashSet<PathBuf>,
    custom_ignores: &[String],
    allow_tests: bool,
    term_indices: &HashMap<String, usize>,
    language: Option<&str>,
    no_gitignore: bool,
) -> Result<HashMap<PathBuf, HashSet<usize>>> {
    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
    let start_time = Instant::now();

    if debug_mode {
        println!("DEBUG: Finding files with matching filenames");
        println!("DEBUG: Queries: {queries:?}");
        println!(
            "DEBUG: Already found files count: {}",
            already_found_files.len()
        );
        println!("DEBUG: Term indices: {term_indices:?}");
    }

    // Get the cached file list, with language filtering if specified
    let file_list =
        get_file_list_by_language(path, allow_tests, custom_ignores, language, no_gitignore)?;

    if debug_mode {
        println!(
            "DEBUG: Searching through {} files from cache",
            file_list.files.len()
        );
    }

    // Tokenize query terms for matching using the standard tokenizer
    let query_tokens: Vec<String> = queries
        .iter()
        .flat_map(|q| tokenization::tokenize(q))
        .collect();

    if debug_mode {
        println!("DEBUG: Query tokens for filename matching: {query_tokens:?}");
    }

    // Search each file for matching filenames
    let mut matching_files = HashMap::new();

    for file_path in &file_list.files {
        // Skip if this file is already in the results
        if already_found_files.contains(file_path) {
            continue;
        }

        // Get the full relative path including directory structure
        let relative_path = file_path.to_string_lossy().to_string();

        // Tokenize the full relative path using the standard tokenizer
        let filename_tokens = tokenization::tokenize(&relative_path);

        if debug_mode && !filename_tokens.is_empty() {
            println!("DEBUG: Path '{relative_path}' tokenized as: {filename_tokens:?}");
        }
        // Find which terms match the filename
        let mut matched_terms = HashSet::new();

        for (term, &idx) in term_indices {
            let term_tokens = tokenization::tokenize(term);

            // Check if any term token matches any filename token
            let matched = term_tokens.iter().any(|term_token| {
                filename_tokens.iter().any(|filename_token| {
                    filename_token.contains(term_token) || term_token.contains(filename_token)
                })
            });

            if matched {
                matched_terms.insert(idx);
                if debug_mode {
                    println!(
                        "DEBUG: Term '{term}' matched path '{relative_path}', adding index {idx}"
                    );
                }
            }
        }

        // Only add the file if we found at least one matching term
        if !matched_terms.is_empty() {
            matching_files.insert(file_path.clone(), matched_terms);
        }
    }

    let elapsed = start_time.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Found {} files with matching filenames in {}",
            matching_files.len(),
            format_duration(elapsed)
        );
    }

    Ok(matching_files)
}

/// Get a list of file extensions for a specific programming language
fn get_language_extensions(language: &str) -> Vec<String> {
    match language.to_lowercase().as_str() {
        "rust" => vec![".rs".to_string()],
        "javascript" => vec![".js".to_string(), ".jsx".to_string(), ".mjs".to_string()],
        "typescript" => vec![".ts".to_string(), ".tsx".to_string()],
        "python" => vec![".py".to_string(), ".pyw".to_string(), ".pyi".to_string()],
        "go" => vec![".go".to_string()],
        "c" => vec![".c".to_string(), ".h".to_string()],
        "cpp" => vec![
            ".cpp".to_string(),
            ".cc".to_string(),
            ".cxx".to_string(),
            ".hpp".to_string(),
            ".hxx".to_string(),
            ".h".to_string(),
        ],
        "java" => vec![".java".to_string()],
        "ruby" => vec![".rb".to_string(), ".rake".to_string()],
        "php" => vec![".php".to_string()],
        "swift" => vec![".swift".to_string()],
        "csharp" => vec![".cs".to_string()],
        _ => vec![], // Return empty vector for unknown languages
    }
}

/// Get a list of files in a directory, filtered by language if specified
pub fn get_file_list_by_language(
    path: &Path,
    allow_tests: bool,
    custom_ignores: &[String],
    language: Option<&str>,
    no_gitignore: bool,
) -> Result<Arc<FileList>> {
    // If no language is specified, use the regular get_file_list function
    if language.is_none() {
        return get_file_list(path, allow_tests, custom_ignores, no_gitignore);
    }

    let debug_mode = std::env::var("DEBUG").unwrap_or_default() == "1";
    let start_time = Instant::now();

    if debug_mode {
        println!("DEBUG: Getting file list for path: {path:?} with language filter: {language:?}");
    }

    // Get the full file list first
    let full_file_list = get_file_list(path, allow_tests, custom_ignores, no_gitignore)?;

    // Get the extensions for the specified language
    let extensions = get_language_extensions(language.unwrap());

    if debug_mode {
        println!("DEBUG: Filtering files by extensions: {extensions:?}");
    }

    // Filter the files by extension
    let filtered_files = if extensions.is_empty() {
        // If no extensions are defined for this language, return the full list
        full_file_list.files.clone()
    } else {
        full_file_list
            .files
            .iter()
            .filter(|file| {
                if let Some(ext) = file.extension() {
                    let ext_lossy = ext.to_string_lossy();
                    let ext_str = format!(".{ext_lossy}");
                    extensions.iter().any(|e| e == &ext_str)
                } else {
                    false
                }
            })
            .cloned()
            .collect()
    };

    let elapsed = start_time.elapsed();
    if debug_mode {
        println!(
            "DEBUG: Filtered file list by language in {} - Found {} files out of {}",
            format_duration(elapsed),
            filtered_files.len(),
            full_file_list.files.len()
        );
    }

    // Create a new FileList with the filtered files
    Ok(Arc::new(FileList {
        files: filtered_files,
        created_at: Instant::now(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_underscore_directory_traversal_unix_paths() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure with underscores (Unix-style paths)
        let underscore_dir = temp_dir.path().join("docs_packages").join("hello_kitty");
        fs::create_dir_all(&underscore_dir).unwrap();

        let test_file = underscore_dir.join("test.txt");
        fs::write(&test_file, "test content with search term").unwrap();

        // Also create a file in the parent underscore directory
        let parent_file = temp_dir.path().join("docs_packages").join("parent.txt");
        fs::write(&parent_file, "parent content").unwrap();

        let file_list = get_file_list(temp_dir.path(), true, &[], false).unwrap();

        assert!(
            file_list.files.iter().any(|f| f == &test_file),
            "File in nested underscore directory should be found: {test_file:?}"
        );
        assert!(
            file_list.files.iter().any(|f| f == &parent_file),
            "File in underscore directory should be found: {parent_file:?}"
        );
    }

    #[test]
    fn test_underscore_directory_traversal_windows_style_paths() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure similar to Windows paths with underscores
        let underscore_dir = temp_dir
            .path()
            .join("C_drive")
            .join("_ai")
            .join("docs")
            .join("docs_packages")
            .join("helloKitty");
        fs::create_dir_all(&underscore_dir).unwrap();

        let test_file = underscore_dir.join("dog.txt");
        fs::write(&test_file, "bad kitty > dog.txt").unwrap();

        // Create additional test files in various underscore directories
        let ai_dir_file = temp_dir
            .path()
            .join("C_drive")
            .join("_ai")
            .join("config.txt");
        fs::create_dir_all(ai_dir_file.parent().unwrap()).unwrap();
        fs::write(&ai_dir_file, "ai configuration").unwrap();

        let docs_packages_file = temp_dir
            .path()
            .join("C_drive")
            .join("_ai")
            .join("docs")
            .join("docs_packages")
            .join("readme.md");
        fs::create_dir_all(docs_packages_file.parent().unwrap()).unwrap();
        fs::write(&docs_packages_file, "documentation packages").unwrap();

        let file_list = get_file_list(temp_dir.path(), true, &[], false).unwrap();

        assert!(
            file_list.files.iter().any(|f| f == &test_file),
            "File in deeply nested underscore directory should be found: {test_file:?}"
        );
        assert!(
            file_list.files.iter().any(|f| f == &ai_dir_file),
            "File in _ai directory should be found: {ai_dir_file:?}"
        );
        assert!(
            file_list.files.iter().any(|f| f == &docs_packages_file),
            "File in docs_packages directory should be found: {docs_packages_file:?}"
        );
    }

    #[test]
    fn test_underscore_directory_with_custom_ignores() {
        let temp_dir = TempDir::new().unwrap();

        // Create directory structure with underscores
        let underscore_dir = temp_dir.path().join("test_packages").join("sub_dir");
        fs::create_dir_all(&underscore_dir).unwrap();

        let test_file = underscore_dir.join("test.rs");
        fs::write(&test_file, "fn test() {}").unwrap();

        let ignored_file = underscore_dir.join("ignored.tmp");
        fs::write(&ignored_file, "temporary content").unwrap();

        // Test with custom ignore patterns
        let custom_ignores = vec!["*.tmp".to_string()];
        let file_list = get_file_list(temp_dir.path(), true, &custom_ignores, false).unwrap();

        assert!(
            file_list.files.iter().any(|f| f == &test_file),
            "Rust file in underscore directory should be found: {test_file:?}"
        );
        assert!(
            !file_list.files.iter().any(|f| f == &ignored_file),
            "Ignored file should not be found: {ignored_file:?}"
        );
    }

    #[test]
    fn test_multiple_underscore_patterns() {
        let temp_dir = TempDir::new().unwrap();

        // Create various underscore directory patterns
        let patterns = vec![
            "single_underscore",
            "multiple_under_scores",
            "_leading_underscore",
            "trailing_underscore_",
            "__double__underscore__",
            "mixed-dash_underscore",
        ];

        let mut expected_files = Vec::new();

        for pattern in patterns {
            let dir = temp_dir.path().join(pattern);
            fs::create_dir_all(&dir).unwrap();

            let file = dir.join("content.txt");
            fs::write(&file, format!("content in {pattern}")).unwrap();
            expected_files.push(file);
        }

        let file_list = get_file_list(temp_dir.path(), true, &[], false).unwrap();

        for expected_file in &expected_files {
            assert!(
                file_list.files.iter().any(|f| f == expected_file),
                "File in underscore directory should be found: {expected_file:?}"
            );
        }
    }

    #[test]
    fn test_underscore_directories_respect_gitignore_patterns() {
        let temp_dir = TempDir::new().unwrap();

        // Create underscore directories that should be ignored by common patterns
        let node_modules_dir = temp_dir.path().join("project_dir").join("node_modules");
        fs::create_dir_all(&node_modules_dir).unwrap();
        let node_file = node_modules_dir.join("package.js");
        fs::write(&node_file, "module content").unwrap();

        let target_dir = temp_dir.path().join("rust_project").join("target");
        fs::create_dir_all(&target_dir).unwrap();
        let target_file = target_dir.join("binary");
        fs::write(&target_file, "binary content").unwrap();

        // Create underscore directories that should NOT be ignored
        let valid_dir = temp_dir.path().join("valid_project").join("src_files");
        fs::create_dir_all(&valid_dir).unwrap();
        let valid_file = valid_dir.join("main.rs");
        fs::write(&valid_file, "fn main() {}").unwrap();

        let file_list = get_file_list(temp_dir.path(), true, &[], false).unwrap();

        assert!(
            !file_list.files.iter().any(|f| f == &node_file),
            "Files in node_modules should be ignored: {node_file:?}"
        );
        assert!(
            !file_list.files.iter().any(|f| f == &target_file),
            "Files in target directory should be ignored: {target_file:?}"
        );

        assert!(
            file_list.files.iter().any(|f| f == &valid_file),
            "Files in valid underscore directories should be found: {valid_file:?}"
        );
    }

    #[test]
    fn test_no_gitignore_parameter() {
        let temp_dir = TempDir::new().unwrap();

        // Create a .gitignore file - no git repository needed!
        // The ignore crate will respect .gitignore files even without a git repo
        // when builder.require_git(false) is set
        let gitignore_content = "*.ignored\nignored_dir/\n";
        let gitignore_path = temp_dir.path().join(".gitignore");
        fs::write(&gitignore_path, gitignore_content).unwrap();

        // Create files that would normally be ignored by .gitignore
        let ignored_file = temp_dir.path().join("test.ignored");
        fs::write(&ignored_file, "ignored content").unwrap();

        let ignored_dir = temp_dir.path().join("ignored_dir");
        fs::create_dir_all(&ignored_dir).unwrap();
        let ignored_dir_file = ignored_dir.join("file.txt");
        fs::write(&ignored_dir_file, "file in ignored directory").unwrap();

        // Create a regular file that should always be found
        let regular_file = temp_dir.path().join("regular.txt");
        fs::write(&regular_file, "regular content").unwrap();

        // Test with gitignore enabled (default behavior)
        let file_list_with_gitignore = get_file_list(temp_dir.path(), true, &[], false).unwrap();

        assert!(
            file_list_with_gitignore
                .files
                .iter()
                .any(|f| f == &regular_file),
            "Regular file should be found with gitignore enabled: {regular_file:?}"
        );
        assert!(
            !file_list_with_gitignore
                .files
                .iter()
                .any(|f| f == &ignored_file),
            "Ignored file should not be found with gitignore enabled: {ignored_file:?}"
        );
        assert!(
            !file_list_with_gitignore.files.iter().any(|f| f == &ignored_dir_file),
            "File in ignored directory should not be found with gitignore enabled: {ignored_dir_file:?}"
        );

        // Test with gitignore disabled (no_gitignore = true)
        let file_list_no_gitignore = get_file_list(temp_dir.path(), true, &[], true).unwrap();

        assert!(
            file_list_no_gitignore
                .files
                .iter()
                .any(|f| f == &regular_file),
            "Regular file should be found with gitignore disabled: {regular_file:?}"
        );
        assert!(
            file_list_no_gitignore
                .files
                .iter()
                .any(|f| f == &ignored_file),
            "Ignored file should be found with gitignore disabled: {ignored_file:?}"
        );
        assert!(
            file_list_no_gitignore.files.iter().any(|f| f == &ignored_dir_file),
            "File in ignored directory should be found with gitignore disabled: {ignored_dir_file:?}"
        );
    }

    #[test]
    fn test_cache_key_includes_no_gitignore() {
        let temp_dir = TempDir::new().unwrap();
        let test_path = temp_dir.path();

        // Generate cache keys with different no_gitignore values
        let key_with_gitignore = generate_cache_key(test_path, true, &[], false);
        let key_without_gitignore = generate_cache_key(test_path, true, &[], true);

        // The keys should be different
        assert_ne!(
            key_with_gitignore, key_without_gitignore,
            "Cache keys should differ when no_gitignore parameter differs"
        );

        // Both keys should contain the appropriate gitignore string
        assert!(
            key_with_gitignore.contains("with_gitignore"),
            "Cache key should contain 'with_gitignore' when no_gitignore is false"
        );
        assert!(
            key_without_gitignore.contains("no_gitignore"),
            "Cache key should contain 'no_gitignore' when no_gitignore is true"
        );
    }
}
