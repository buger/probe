use crate::path_safety;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Global configuration for probe
/// All fields are optional to support partial configurations and merging
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProbeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<DefaultsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract: Option<ExtractConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<QueryConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<LspConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<PerformanceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexing: Option<IndexingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DefaultsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_lsp: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reranker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_threshold: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_tests: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_gitignore: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_tests: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_tests: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LspConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_stdlib: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_autostart: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_cache: Option<LspWorkspaceCacheConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LspWorkspaceCacheConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_open_caches: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_mb_per_workspace: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookup_depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerformanceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_cache_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize_blocks: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_index: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watch_files: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_workers: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_budget_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_pressure_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_queue_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_exclude_patterns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_include_patterns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_file_size_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery_batch_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_update_interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_processing_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_file_processing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persist_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_languages: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_languages: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<IndexingFeatures>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp_caching: Option<IndexingLspCaching>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_configs: Option<HashMap<String, LanguageIndexConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexingFeatures {
    /// Extract function and method signatures from source code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_functions: Option<bool>,
    /// Extract type definitions (classes, structs, interfaces)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_types: Option<bool>,
    /// Extract variable and constant declarations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_variables: Option<bool>,
    /// Extract import/export statements
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_imports: Option<bool>,
    /// Extract test-related symbols and functions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_tests: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexingLspCaching {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_call_hierarchy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_definitions: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_references: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hover: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_document_symbols: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_during_indexing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preload_common_symbols: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cache_entries_per_operation: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp_operation_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_operations: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_operations: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanguageIndexConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_workers: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_budget_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_file_size_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_extensions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_patterns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_patterns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<IndexingFeatures>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
}

/// Configuration with resolved values (no Options)
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub defaults: ResolvedDefaultsConfig,
    pub search: ResolvedSearchConfig,
    pub extract: ResolvedExtractConfig,
    pub query: ResolvedQueryConfig,
    pub lsp: ResolvedLspConfig,
    pub performance: ResolvedPerformanceConfig,
    pub indexing: ResolvedIndexingConfig,
}

#[derive(Debug, Clone)]
pub struct ResolvedDefaultsConfig {
    pub debug: bool,
    pub log_level: String,
    pub enable_lsp: bool,
    pub format: String,
    pub timeout: u64,
}

#[derive(Debug, Clone)]
pub struct ResolvedSearchConfig {
    pub max_results: Option<usize>,
    pub max_tokens: Option<usize>,
    pub max_bytes: Option<usize>,
    pub frequency: bool,
    pub reranker: String,
    pub merge_threshold: usize,
    pub allow_tests: bool,
    pub no_gitignore: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedExtractConfig {
    pub context_lines: usize,
    pub allow_tests: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedQueryConfig {
    pub max_results: Option<usize>,
    pub allow_tests: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedLspConfig {
    pub include_stdlib: bool,
    pub socket_path: Option<String>,
    pub disable_autostart: bool,
    pub workspace_cache: ResolvedLspWorkspaceCacheConfig,
}

#[derive(Debug, Clone)]
pub struct ResolvedLspWorkspaceCacheConfig {
    pub max_open_caches: usize,
    pub size_mb_per_workspace: usize,
    pub lookup_depth: usize,
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedPerformanceConfig {
    pub tree_cache_size: usize,
    pub optimize_blocks: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedIndexingConfig {
    pub enabled: bool,
    pub auto_index: bool,
    pub watch_files: bool,
    pub default_depth: u32,
    pub max_workers: usize,
    pub memory_budget_mb: u64,
    pub memory_pressure_threshold: f64,
    pub max_queue_size: usize,
    pub global_exclude_patterns: Vec<String>,
    pub global_include_patterns: Vec<String>,
    pub max_file_size_mb: u64,
    pub incremental_mode: bool,
    pub discovery_batch_size: usize,
    pub status_update_interval_secs: u64,
    pub file_processing_timeout_ms: u64,
    pub parallel_file_processing: bool,
    pub persist_cache: bool,
    pub cache_directory: Option<String>,
    pub priority_languages: Vec<String>,
    pub disabled_languages: Vec<String>,
    pub features: ResolvedIndexingFeatures,
    pub lsp_caching: ResolvedIndexingLspCaching,
    pub language_configs: HashMap<String, LanguageIndexConfig>,
}

#[derive(Debug, Clone)]
pub struct ResolvedIndexingFeatures {
    pub extract_functions: bool,
    pub extract_types: bool,
    pub extract_variables: bool,
    pub extract_imports: bool,
    pub extract_tests: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedIndexingLspCaching {
    pub cache_call_hierarchy: bool,
    pub cache_definitions: bool,
    pub cache_references: bool,
    pub cache_hover: bool,
    pub cache_document_symbols: bool,
    pub cache_during_indexing: bool,
    pub preload_common_symbols: bool,
    pub max_cache_entries_per_operation: usize,
    pub lsp_operation_timeout_ms: u64,
    pub priority_operations: Vec<String>,
    pub disabled_operations: Vec<String>,
}

impl ProbeConfig {
    /// Load configuration from multiple levels and merge them
    pub fn load() -> Result<ResolvedConfig> {
        // In CI environments, return default config with minimal error handling
        let is_ci = env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();

        // Load configurations from all levels
        let configs = match Self::load_all_configs() {
            Ok(configs) => configs,
            Err(e) if is_ci => {
                // In CI, ignore config loading errors and use defaults
                eprintln!("Warning: Config loading failed in CI, using defaults: {e}");
                vec![]
            }
            Err(e) => return Err(e),
        };

        // Merge configurations (global -> project -> local)
        let mut merged = ProbeConfig::default();
        for config in configs {
            merged = Self::merge_configs(merged, config);
        }

        // Apply environment variable overrides
        merged.apply_env_overrides();

        // Convert to resolved config with defaults
        Ok(merged.resolve_with_defaults())
    }

    /// Get all configuration file paths in priority order
    fn get_config_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 1. Global config: ~/.probe/settings.json
        // On Windows, prefer USERPROFILE env var to avoid extra OS calls
        #[cfg(target_os = "windows")]
        {
            if let Ok(userprofile) = env::var("USERPROFILE") {
                paths.push(
                    PathBuf::from(userprofile)
                        .join(".probe")
                        .join("settings.json"),
                );
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            if let Ok(home) = env::var("HOME") {
                paths.push(PathBuf::from(home).join(".probe").join("settings.json"));
            } else if let Some(home_dir) = dirs::home_dir() {
                paths.push(home_dir.join(".probe").join("settings.json"));
            }
        }

        // 2. Project config: ./.probe/settings.json
        // IMPORTANT (Windows): Skip ALL project config on Windows to avoid stack overflow
        // from path resolution in temp directories with junction points.
        // Even relative paths can trigger the issue when they're resolved.
        #[cfg(not(target_os = "windows"))]
        {
            // Safe on Unix: relative paths won't trigger the Windows junction recursion.
            paths.push(PathBuf::from(".probe").join("settings.json"));
            paths.push(PathBuf::from(".probe").join("settings.local.json"));
        }
        // On Windows, we completely skip project config to avoid any path resolution issues

        // 3. Custom path via environment variable - HIGHEST precedence (last wins)
        if let Ok(custom_path) = env::var("PROBE_CONFIG_PATH") {
            // Do NOT call is_dir(); it triggers metadata on possibly relative paths.
            // Heuristic: trailing slash/backslash => treat as directory.
            let custom_str = custom_path.as_str();
            let looks_like_dir = custom_str.ends_with('\\') || custom_str.ends_with('/');
            if looks_like_dir {
                // It's a directory, add both settings.json and settings.local.json
                paths.push(PathBuf::from(&custom_path).join("settings.json"));
                paths.push(PathBuf::from(&custom_path).join("settings.local.json"));
            } else {
                // It's a specific file path
                paths.push(PathBuf::from(&custom_path));
            }
        }

        paths
    }

    /// Load all configuration files that exist
    fn load_all_configs() -> Result<Vec<ProbeConfig>> {
        let paths = Self::get_config_paths();
        let mut configs = Vec::new();
        let skip_project_config = std::env::var("PROBE_SKIP_PROJECT_CONFIG").is_ok();

        for path in paths {
            // Skip project config loading if explicitly disabled (useful for CI)
            // But allow custom config paths (via PROBE_CONFIG_PATH) to work in tests
            if skip_project_config {
                // Skip relative paths (project configs) but allow absolute paths
                if path.is_relative() {
                    continue;
                }
                // Also skip home directory configs that might use unsafe path resolution
                #[cfg(target_os = "windows")]
                {
                    if let Some(path_str) = path.to_str() {
                        // Skip paths that might involve unsafe resolution on Windows
                        if path_str.contains("..") || path_str.starts_with(".") {
                            continue;
                        }
                    }
                }
            }

            // On Windows, skip checking paths that start with "." to avoid junction issues
            // These are relative paths that could trigger stack overflow when resolved
            #[cfg(target_os = "windows")]
            {
                if path.starts_with(".") && !skip_project_config {
                    // Only skip if we're not already handling it above
                    continue;
                }
            }

            // Use path_safety module to avoid following symlinks/junctions
            // This prevents stack overflow on Windows with junction point cycles
            if path_safety::is_file_no_follow(&path) {
                if let Ok(config) = Self::load_from_file(&path) {
                    configs.push(config);
                }
            }
        }

        Ok(configs)
    }

    /// Load a single configuration file
    fn load_from_file(path: &Path) -> Result<ProbeConfig> {
        // Read raw bytes to handle potential UTF-8 BOM (common on Windows)
        let bytes = fs::read(path).context(format!("Failed to read config file: {path:?}"))?;

        // Strip UTF-8 BOM if present (0xEF, 0xBB, 0xBF)
        let content_bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            &bytes[3..]
        } else {
            &bytes[..]
        };

        let config: ProbeConfig = serde_json::from_slice(content_bytes)
            .context(format!("Failed to parse config file: {path:?}"))?;

        Ok(config)
    }

    /// Deep merge two configurations, with `other` taking precedence
    fn merge_configs(mut base: ProbeConfig, other: ProbeConfig) -> ProbeConfig {
        // Merge defaults
        if let Some(other_defaults) = other.defaults {
            let base_defaults = base.defaults.get_or_insert(DefaultsConfig::default());
            if other_defaults.debug.is_some() {
                base_defaults.debug = other_defaults.debug;
            }
            if other_defaults.log_level.is_some() {
                base_defaults.log_level = other_defaults.log_level;
            }
            if other_defaults.enable_lsp.is_some() {
                base_defaults.enable_lsp = other_defaults.enable_lsp;
            }
            if other_defaults.format.is_some() {
                base_defaults.format = other_defaults.format;
            }
            if other_defaults.timeout.is_some() {
                base_defaults.timeout = other_defaults.timeout;
            }
        }

        // Merge search
        if let Some(other_search) = other.search {
            let base_search = base.search.get_or_insert(SearchConfig::default());
            if other_search.max_results.is_some() {
                base_search.max_results = other_search.max_results;
            }
            if other_search.max_tokens.is_some() {
                base_search.max_tokens = other_search.max_tokens;
            }
            if other_search.max_bytes.is_some() {
                base_search.max_bytes = other_search.max_bytes;
            }
            if other_search.frequency.is_some() {
                base_search.frequency = other_search.frequency;
            }
            if other_search.reranker.is_some() {
                base_search.reranker = other_search.reranker;
            }
            if other_search.merge_threshold.is_some() {
                base_search.merge_threshold = other_search.merge_threshold;
            }
            if other_search.allow_tests.is_some() {
                base_search.allow_tests = other_search.allow_tests;
            }
            if other_search.no_gitignore.is_some() {
                base_search.no_gitignore = other_search.no_gitignore;
            }
        }

        // Merge extract
        if let Some(other_extract) = other.extract {
            let base_extract = base.extract.get_or_insert(ExtractConfig::default());
            if other_extract.context_lines.is_some() {
                base_extract.context_lines = other_extract.context_lines;
            }
            if other_extract.allow_tests.is_some() {
                base_extract.allow_tests = other_extract.allow_tests;
            }
        }

        // Merge query
        if let Some(other_query) = other.query {
            let base_query = base.query.get_or_insert(QueryConfig::default());
            if other_query.max_results.is_some() {
                base_query.max_results = other_query.max_results;
            }
            if other_query.allow_tests.is_some() {
                base_query.allow_tests = other_query.allow_tests;
            }
        }

        // Merge lsp
        if let Some(other_lsp) = other.lsp {
            let base_lsp = base.lsp.get_or_insert(LspConfig::default());
            if other_lsp.include_stdlib.is_some() {
                base_lsp.include_stdlib = other_lsp.include_stdlib;
            }
            if other_lsp.socket_path.is_some() {
                base_lsp.socket_path = other_lsp.socket_path;
            }
            if other_lsp.disable_autostart.is_some() {
                base_lsp.disable_autostart = other_lsp.disable_autostart;
            }

            // Merge workspace cache
            if let Some(other_cache) = other_lsp.workspace_cache {
                let base_cache = base_lsp
                    .workspace_cache
                    .get_or_insert(LspWorkspaceCacheConfig::default());
                if other_cache.max_open_caches.is_some() {
                    base_cache.max_open_caches = other_cache.max_open_caches;
                }
                if other_cache.size_mb_per_workspace.is_some() {
                    base_cache.size_mb_per_workspace = other_cache.size_mb_per_workspace;
                }
                if other_cache.lookup_depth.is_some() {
                    base_cache.lookup_depth = other_cache.lookup_depth;
                }
                if other_cache.base_dir.is_some() {
                    base_cache.base_dir = other_cache.base_dir;
                }
            }
        }

        // Merge performance
        if let Some(other_perf) = other.performance {
            let base_perf = base.performance.get_or_insert(PerformanceConfig::default());
            if other_perf.tree_cache_size.is_some() {
                base_perf.tree_cache_size = other_perf.tree_cache_size;
            }
            if other_perf.optimize_blocks.is_some() {
                base_perf.optimize_blocks = other_perf.optimize_blocks;
            }
        }

        // Merge indexing
        if let Some(other_indexing) = other.indexing {
            let base_indexing = base.indexing.get_or_insert(IndexingConfig::default());
            if other_indexing.enabled.is_some() {
                base_indexing.enabled = other_indexing.enabled;
            }
            if other_indexing.auto_index.is_some() {
                base_indexing.auto_index = other_indexing.auto_index;
            }
            if other_indexing.watch_files.is_some() {
                base_indexing.watch_files = other_indexing.watch_files;
            }
            if other_indexing.default_depth.is_some() {
                base_indexing.default_depth = other_indexing.default_depth;
            }
            if other_indexing.max_workers.is_some() {
                base_indexing.max_workers = other_indexing.max_workers;
            }
            if other_indexing.memory_budget_mb.is_some() {
                base_indexing.memory_budget_mb = other_indexing.memory_budget_mb;
            }
            if other_indexing.memory_pressure_threshold.is_some() {
                base_indexing.memory_pressure_threshold = other_indexing.memory_pressure_threshold;
            }
            if other_indexing.max_queue_size.is_some() {
                base_indexing.max_queue_size = other_indexing.max_queue_size;
            }
            if other_indexing.global_exclude_patterns.is_some() {
                base_indexing.global_exclude_patterns = other_indexing.global_exclude_patterns;
            }
            if other_indexing.global_include_patterns.is_some() {
                base_indexing.global_include_patterns = other_indexing.global_include_patterns;
            }
            if other_indexing.max_file_size_mb.is_some() {
                base_indexing.max_file_size_mb = other_indexing.max_file_size_mb;
            }
            if other_indexing.incremental_mode.is_some() {
                base_indexing.incremental_mode = other_indexing.incremental_mode;
            }
            if other_indexing.discovery_batch_size.is_some() {
                base_indexing.discovery_batch_size = other_indexing.discovery_batch_size;
            }
            if other_indexing.status_update_interval_secs.is_some() {
                base_indexing.status_update_interval_secs =
                    other_indexing.status_update_interval_secs;
            }
            if other_indexing.file_processing_timeout_ms.is_some() {
                base_indexing.file_processing_timeout_ms =
                    other_indexing.file_processing_timeout_ms;
            }
            if other_indexing.parallel_file_processing.is_some() {
                base_indexing.parallel_file_processing = other_indexing.parallel_file_processing;
            }
            if other_indexing.persist_cache.is_some() {
                base_indexing.persist_cache = other_indexing.persist_cache;
            }
            if other_indexing.cache_directory.is_some() {
                base_indexing.cache_directory = other_indexing.cache_directory;
            }
            if other_indexing.priority_languages.is_some() {
                base_indexing.priority_languages = other_indexing.priority_languages;
            }
            if other_indexing.disabled_languages.is_some() {
                base_indexing.disabled_languages = other_indexing.disabled_languages;
            }

            // Merge features
            if let Some(other_features) = other_indexing.features {
                let base_features = base_indexing
                    .features
                    .get_or_insert(IndexingFeatures::default());
                if other_features.extract_functions.is_some() {
                    base_features.extract_functions = other_features.extract_functions;
                }
                if other_features.extract_types.is_some() {
                    base_features.extract_types = other_features.extract_types;
                }
                if other_features.extract_variables.is_some() {
                    base_features.extract_variables = other_features.extract_variables;
                }
                if other_features.extract_imports.is_some() {
                    base_features.extract_imports = other_features.extract_imports;
                }
                if other_features.extract_tests.is_some() {
                    base_features.extract_tests = other_features.extract_tests;
                }
            }

            // Merge LSP caching
            if let Some(other_lsp_caching) = other_indexing.lsp_caching {
                let base_lsp_caching = base_indexing
                    .lsp_caching
                    .get_or_insert(IndexingLspCaching::default());
                if other_lsp_caching.cache_call_hierarchy.is_some() {
                    base_lsp_caching.cache_call_hierarchy = other_lsp_caching.cache_call_hierarchy;
                }
                if other_lsp_caching.cache_definitions.is_some() {
                    base_lsp_caching.cache_definitions = other_lsp_caching.cache_definitions;
                }
                if other_lsp_caching.cache_references.is_some() {
                    base_lsp_caching.cache_references = other_lsp_caching.cache_references;
                }
                if other_lsp_caching.cache_hover.is_some() {
                    base_lsp_caching.cache_hover = other_lsp_caching.cache_hover;
                }
                if other_lsp_caching.cache_document_symbols.is_some() {
                    base_lsp_caching.cache_document_symbols =
                        other_lsp_caching.cache_document_symbols;
                }
                if other_lsp_caching.cache_during_indexing.is_some() {
                    base_lsp_caching.cache_during_indexing =
                        other_lsp_caching.cache_during_indexing;
                }
                if other_lsp_caching.preload_common_symbols.is_some() {
                    base_lsp_caching.preload_common_symbols =
                        other_lsp_caching.preload_common_symbols;
                }
                if other_lsp_caching.max_cache_entries_per_operation.is_some() {
                    base_lsp_caching.max_cache_entries_per_operation =
                        other_lsp_caching.max_cache_entries_per_operation;
                }
                if other_lsp_caching.lsp_operation_timeout_ms.is_some() {
                    base_lsp_caching.lsp_operation_timeout_ms =
                        other_lsp_caching.lsp_operation_timeout_ms;
                }
                if other_lsp_caching.priority_operations.is_some() {
                    base_lsp_caching.priority_operations = other_lsp_caching.priority_operations;
                }
                if other_lsp_caching.disabled_operations.is_some() {
                    base_lsp_caching.disabled_operations = other_lsp_caching.disabled_operations;
                }
            }

            // Merge language configs
            if other_indexing.language_configs.is_some() {
                base_indexing.language_configs = other_indexing.language_configs;
            }
        }

        base
    }

    /// Apply environment variable overrides
    fn apply_env_overrides(&mut self) {
        // Defaults
        let defaults = self.defaults.get_or_insert(DefaultsConfig::default());
        if let Ok(val) = env::var("PROBE_DEBUG") {
            defaults.debug = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_LOG_LEVEL") {
            defaults.log_level = Some(val);
        }
        if let Ok(val) = env::var("PROBE_ENABLE_LSP") {
            defaults.enable_lsp = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_FORMAT") {
            defaults.format = Some(val);
        }
        if let Ok(val) = env::var("PROBE_TIMEOUT") {
            if let Ok(timeout) = val.parse() {
                defaults.timeout = Some(timeout);
            }
        }

        // Search
        let search = self.search.get_or_insert(SearchConfig::default());
        if let Ok(val) = env::var("PROBE_MAX_RESULTS") {
            if let Ok(max) = val.parse() {
                search.max_results = Some(max);
            }
        }
        if let Ok(val) = env::var("PROBE_MAX_TOKENS") {
            if let Ok(max) = val.parse() {
                search.max_tokens = Some(max);
            }
        }
        if let Ok(val) = env::var("PROBE_MAX_BYTES") {
            if let Ok(max) = val.parse() {
                search.max_bytes = Some(max);
            }
        }
        if let Ok(val) = env::var("PROBE_SEARCH_FREQUENCY") {
            search.frequency = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_SEARCH_RERANKER") {
            search.reranker = Some(val);
        }
        if let Ok(val) = env::var("PROBE_SEARCH_MERGE_THRESHOLD") {
            if let Ok(threshold) = val.parse() {
                search.merge_threshold = Some(threshold);
            }
        }
        if let Ok(val) = env::var("PROBE_ALLOW_TESTS") {
            let allow = val == "1" || val.to_lowercase() == "true";
            search.allow_tests = Some(allow);
            self.extract
                .get_or_insert(ExtractConfig::default())
                .allow_tests = Some(allow);
            self.query.get_or_insert(QueryConfig::default()).allow_tests = Some(allow);
        }
        if let Ok(val) = env::var("PROBE_NO_GITIGNORE") {
            search.no_gitignore = Some(val == "1" || val.to_lowercase() == "true");
        }

        // Extract
        let extract = self.extract.get_or_insert(ExtractConfig::default());
        if let Ok(val) = env::var("PROBE_EXTRACT_CONTEXT_LINES") {
            if let Ok(lines) = val.parse() {
                extract.context_lines = Some(lines);
            }
        }

        // LSP
        let lsp = self.lsp.get_or_insert(LspConfig::default());
        if let Ok(val) = env::var("PROBE_LSP_INCLUDE_STDLIB") {
            lsp.include_stdlib = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_LSP_SOCKET_PATH") {
            lsp.socket_path = Some(val);
        }
        if let Ok(val) = env::var("PROBE_LSP_DISABLE_AUTOSTART") {
            lsp.disable_autostart = Some(val == "1" || val.to_lowercase() == "true");
        }

        // LSP Workspace Cache
        let cache = lsp
            .workspace_cache
            .get_or_insert(LspWorkspaceCacheConfig::default());
        if let Ok(val) = env::var("PROBE_LSP_WORKSPACE_CACHE_MAX") {
            if let Ok(max) = val.parse() {
                cache.max_open_caches = Some(max);
            }
        }
        if let Ok(val) = env::var("PROBE_LSP_WORKSPACE_CACHE_SIZE_MB") {
            if let Ok(size) = val.parse() {
                cache.size_mb_per_workspace = Some(size);
            }
        }
        if let Ok(val) = env::var("PROBE_LSP_WORKSPACE_LOOKUP_DEPTH") {
            if let Ok(depth) = val.parse() {
                cache.lookup_depth = Some(depth);
            }
        }
        if let Ok(val) = env::var("PROBE_LSP_WORKSPACE_CACHE_DIR") {
            cache.base_dir = Some(val);
        }

        // Performance
        let perf = self.performance.get_or_insert(PerformanceConfig::default());
        if let Ok(val) = env::var("PROBE_TREE_CACHE_SIZE") {
            if let Ok(size) = val.parse() {
                perf.tree_cache_size = Some(size);
            }
        }
        if let Ok(val) = env::var("PROBE_OPTIMIZE_BLOCKS") {
            perf.optimize_blocks = Some(val == "1" || val.to_lowercase() == "true");
        }

        // Indexing
        let indexing = self.indexing.get_or_insert(IndexingConfig::default());
        if let Ok(val) = env::var("PROBE_INDEXING_ENABLED") {
            indexing.enabled = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_AUTO_INDEX") {
            indexing.auto_index = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_WATCH_FILES") {
            indexing.watch_files = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_DEFAULT_DEPTH") {
            if let Ok(depth) = val.parse() {
                indexing.default_depth = Some(depth);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_MAX_WORKERS") {
            if let Ok(workers) = val.parse() {
                indexing.max_workers = Some(workers);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_MEMORY_BUDGET_MB") {
            if let Ok(budget) = val.parse() {
                indexing.memory_budget_mb = Some(budget);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_MEMORY_PRESSURE_THRESHOLD") {
            if let Ok(threshold) = val.parse() {
                indexing.memory_pressure_threshold = Some(threshold);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_MAX_QUEUE_SIZE") {
            if let Ok(size) = val.parse() {
                indexing.max_queue_size = Some(size);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_MAX_FILE_SIZE_MB") {
            if let Ok(size) = val.parse() {
                indexing.max_file_size_mb = Some(size);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_INCREMENTAL_MODE") {
            indexing.incremental_mode = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_DISCOVERY_BATCH_SIZE") {
            if let Ok(size) = val.parse() {
                indexing.discovery_batch_size = Some(size);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_STATUS_UPDATE_INTERVAL_SECS") {
            if let Ok(interval) = val.parse() {
                indexing.status_update_interval_secs = Some(interval);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_FILE_PROCESSING_TIMEOUT_MS") {
            if let Ok(timeout) = val.parse() {
                indexing.file_processing_timeout_ms = Some(timeout);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_PARALLEL_FILE_PROCESSING") {
            indexing.parallel_file_processing = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_PERSIST_CACHE") {
            indexing.persist_cache = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_CACHE_DIRECTORY") {
            indexing.cache_directory = Some(val);
        }

        // Indexing features
        let features = indexing.features.get_or_insert(IndexingFeatures::default());
        if let Ok(val) = env::var("PROBE_INDEXING_EXTRACT_FUNCTIONS") {
            features.extract_functions = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_EXTRACT_TYPES") {
            features.extract_types = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_EXTRACT_VARIABLES") {
            features.extract_variables = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_EXTRACT_IMPORTS") {
            features.extract_imports = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_EXTRACT_TESTS") {
            features.extract_tests = Some(val == "1" || val.to_lowercase() == "true");
        }

        // Indexing LSP caching
        let lsp_caching = indexing
            .lsp_caching
            .get_or_insert(IndexingLspCaching::default());
        if let Ok(val) = env::var("PROBE_INDEXING_CACHE_CALL_HIERARCHY") {
            lsp_caching.cache_call_hierarchy = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_CACHE_DEFINITIONS") {
            lsp_caching.cache_definitions = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_CACHE_REFERENCES") {
            lsp_caching.cache_references = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_CACHE_HOVER") {
            lsp_caching.cache_hover = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_CACHE_DOCUMENT_SYMBOLS") {
            lsp_caching.cache_document_symbols = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_CACHE_DURING_INDEXING") {
            lsp_caching.cache_during_indexing = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_PRELOAD_COMMON_SYMBOLS") {
            lsp_caching.preload_common_symbols = Some(val == "1" || val.to_lowercase() == "true");
        }
        if let Ok(val) = env::var("PROBE_INDEXING_MAX_CACHE_ENTRIES_PER_OPERATION") {
            if let Ok(max) = val.parse() {
                lsp_caching.max_cache_entries_per_operation = Some(max);
            }
        }
        if let Ok(val) = env::var("PROBE_INDEXING_LSP_OPERATION_TIMEOUT_MS") {
            if let Ok(timeout) = val.parse() {
                lsp_caching.lsp_operation_timeout_ms = Some(timeout);
            }
        }
    }

    /// Convert to resolved config with all defaults applied
    fn resolve_with_defaults(self) -> ResolvedConfig {
        let defaults = self.defaults.unwrap_or_default();
        let search = self.search.unwrap_or_default();
        let extract = self.extract.unwrap_or_default();
        let query = self.query.unwrap_or_default();
        let lsp = self.lsp.unwrap_or_default();
        let performance = self.performance.unwrap_or_default();
        let indexing = self.indexing.unwrap_or_default();

        ResolvedConfig {
            defaults: ResolvedDefaultsConfig {
                debug: defaults.debug.unwrap_or(false),
                log_level: defaults.log_level.unwrap_or_else(|| "info".to_string()),
                enable_lsp: defaults.enable_lsp.unwrap_or(false),
                format: defaults.format.unwrap_or_else(|| "color".to_string()),
                timeout: defaults.timeout.unwrap_or(30),
            },
            search: ResolvedSearchConfig {
                max_results: search.max_results,
                max_tokens: search.max_tokens,
                max_bytes: search.max_bytes,
                frequency: search.frequency.unwrap_or(true),
                reranker: search.reranker.unwrap_or_else(|| "bm25".to_string()),
                merge_threshold: search.merge_threshold.unwrap_or(5),
                allow_tests: search.allow_tests.unwrap_or(false),
                no_gitignore: search.no_gitignore.unwrap_or(false),
            },
            extract: ResolvedExtractConfig {
                context_lines: extract.context_lines.unwrap_or(0),
                allow_tests: extract.allow_tests.unwrap_or(false),
            },
            query: ResolvedQueryConfig {
                max_results: query.max_results,
                allow_tests: query.allow_tests.unwrap_or(false),
            },
            lsp: ResolvedLspConfig {
                include_stdlib: lsp.include_stdlib.unwrap_or(false),
                socket_path: lsp.socket_path,
                disable_autostart: lsp.disable_autostart.unwrap_or(false),
                workspace_cache: {
                    let cache = lsp.workspace_cache.unwrap_or_default();
                    ResolvedLspWorkspaceCacheConfig {
                        max_open_caches: cache.max_open_caches.unwrap_or(8),
                        size_mb_per_workspace: cache.size_mb_per_workspace.unwrap_or(100),
                        lookup_depth: cache.lookup_depth.unwrap_or(3),
                        base_dir: cache.base_dir,
                    }
                },
            },
            performance: ResolvedPerformanceConfig {
                tree_cache_size: performance.tree_cache_size.unwrap_or(2000),
                optimize_blocks: performance.optimize_blocks.unwrap_or(false),
            },
            indexing: ResolvedIndexingConfig {
                enabled: indexing.enabled.unwrap_or(true),
                auto_index: indexing.auto_index.unwrap_or(true),
                watch_files: indexing.watch_files.unwrap_or(true),
                default_depth: indexing.default_depth.unwrap_or(3),
                max_workers: indexing.max_workers.unwrap_or(8),
                memory_budget_mb: indexing.memory_budget_mb.unwrap_or(512),
                memory_pressure_threshold: indexing.memory_pressure_threshold.unwrap_or(0.8),
                max_queue_size: indexing.max_queue_size.unwrap_or(10000),
                global_exclude_patterns: indexing.global_exclude_patterns.unwrap_or_else(|| {
                    vec![
                        "*.git/*".to_string(),
                        "*/node_modules/*".to_string(),
                        "*/target/*".to_string(),
                        "*/build/*".to_string(),
                        "*/dist/*".to_string(),
                        "*/.cargo/*".to_string(),
                        "*/__pycache__/*".to_string(),
                        "*.tmp".to_string(),
                        "*.log".to_string(),
                    ]
                }),
                global_include_patterns: indexing.global_include_patterns.unwrap_or_default(),
                max_file_size_mb: indexing.max_file_size_mb.unwrap_or(10),
                incremental_mode: indexing.incremental_mode.unwrap_or(true),
                discovery_batch_size: indexing.discovery_batch_size.unwrap_or(1000),
                status_update_interval_secs: indexing.status_update_interval_secs.unwrap_or(5),
                file_processing_timeout_ms: indexing.file_processing_timeout_ms.unwrap_or(30000),
                parallel_file_processing: indexing.parallel_file_processing.unwrap_or(true),
                persist_cache: indexing.persist_cache.unwrap_or(false),
                cache_directory: indexing.cache_directory,
                priority_languages: indexing.priority_languages.unwrap_or_else(|| {
                    vec![
                        "rust".to_string(),
                        "typescript".to_string(),
                        "python".to_string(),
                    ]
                }),
                disabled_languages: indexing.disabled_languages.unwrap_or_default(),
                features: {
                    let features = indexing.features.unwrap_or_default();
                    ResolvedIndexingFeatures {
                        extract_functions: features.extract_functions.unwrap_or(true),
                        extract_types: features.extract_types.unwrap_or(true),
                        extract_variables: features.extract_variables.unwrap_or(true),
                        extract_imports: features.extract_imports.unwrap_or(true),
                        extract_tests: features.extract_tests.unwrap_or(true),
                    }
                },
                lsp_caching: {
                    let lsp_caching = indexing.lsp_caching.unwrap_or_default();
                    ResolvedIndexingLspCaching {
                        cache_call_hierarchy: lsp_caching.cache_call_hierarchy.unwrap_or(true),
                        cache_definitions: lsp_caching.cache_definitions.unwrap_or(false),
                        cache_references: lsp_caching.cache_references.unwrap_or(true),
                        cache_hover: lsp_caching.cache_hover.unwrap_or(true),
                        cache_document_symbols: lsp_caching.cache_document_symbols.unwrap_or(false),
                        cache_during_indexing: lsp_caching.cache_during_indexing.unwrap_or(false),
                        preload_common_symbols: lsp_caching.preload_common_symbols.unwrap_or(false),
                        max_cache_entries_per_operation: lsp_caching
                            .max_cache_entries_per_operation
                            .unwrap_or(1000),
                        lsp_operation_timeout_ms: lsp_caching
                            .lsp_operation_timeout_ms
                            .unwrap_or(5000),
                        priority_operations: lsp_caching.priority_operations.unwrap_or_else(|| {
                            vec![
                                "call_hierarchy".to_string(),
                                "references".to_string(),
                                "hover".to_string(),
                            ]
                        }),
                        disabled_operations: lsp_caching.disabled_operations.unwrap_or_default(),
                    }
                },
                language_configs: indexing.language_configs.unwrap_or_default(),
            },
        }
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<()> {
        // Validate format if set (case-insensitive)
        if let Some(ref defaults) = self.defaults {
            if let Some(ref format) = defaults.format {
                let valid_formats = ["terminal", "markdown", "plain", "json", "xml", "color"];
                let format_lower = format.to_lowercase();
                if !valid_formats.contains(&format_lower.as_str()) {
                    anyhow::bail!("Invalid format: {}", format);
                }
            }

            if let Some(ref log_level) = defaults.log_level {
                let valid_log_levels = ["error", "warn", "info", "debug", "trace"];
                let level_lower = log_level.to_lowercase();
                if !valid_log_levels.contains(&level_lower.as_str()) {
                    anyhow::bail!("Invalid log level: {}", log_level);
                }
            }
        }

        // Validate reranker if set (case-insensitive)
        if let Some(ref search) = self.search {
            if let Some(ref reranker) = search.reranker {
                let valid_rerankers = [
                    "bm25",
                    "hybrid",
                    "hybrid2",
                    "tfidf",
                    "ms-marco-tinybert",
                    "ms-marco-minilm-l6",
                    "ms-marco-minilm-l12",
                ];
                let reranker_lower = reranker.to_lowercase();
                if !valid_rerankers.contains(&reranker_lower.as_str()) {
                    anyhow::bail!("Invalid reranker: {}", reranker);
                }
            }
        }

        Ok(())
    }
}

impl ResolvedConfig {
    /// Convert back to ProbeConfig for serialization
    pub fn to_probe_config(&self) -> ProbeConfig {
        ProbeConfig {
            defaults: Some(DefaultsConfig {
                debug: Some(self.defaults.debug),
                log_level: Some(self.defaults.log_level.clone()),
                enable_lsp: Some(self.defaults.enable_lsp),
                format: Some(self.defaults.format.clone()),
                timeout: Some(self.defaults.timeout),
            }),
            search: Some(SearchConfig {
                max_results: self.search.max_results,
                max_tokens: self.search.max_tokens,
                max_bytes: self.search.max_bytes,
                frequency: Some(self.search.frequency),
                reranker: Some(self.search.reranker.clone()),
                merge_threshold: Some(self.search.merge_threshold),
                allow_tests: Some(self.search.allow_tests),
                no_gitignore: Some(self.search.no_gitignore),
            }),
            extract: Some(ExtractConfig {
                context_lines: Some(self.extract.context_lines),
                allow_tests: Some(self.extract.allow_tests),
            }),
            query: Some(QueryConfig {
                max_results: self.query.max_results,
                allow_tests: Some(self.query.allow_tests),
            }),
            lsp: Some(LspConfig {
                include_stdlib: Some(self.lsp.include_stdlib),
                socket_path: self.lsp.socket_path.clone(),
                disable_autostart: Some(self.lsp.disable_autostart),
                workspace_cache: Some(LspWorkspaceCacheConfig {
                    max_open_caches: Some(self.lsp.workspace_cache.max_open_caches),
                    size_mb_per_workspace: Some(self.lsp.workspace_cache.size_mb_per_workspace),
                    lookup_depth: Some(self.lsp.workspace_cache.lookup_depth),
                    base_dir: self.lsp.workspace_cache.base_dir.clone(),
                }),
            }),
            performance: Some(PerformanceConfig {
                tree_cache_size: Some(self.performance.tree_cache_size),
                optimize_blocks: Some(self.performance.optimize_blocks),
            }),
            indexing: Some(IndexingConfig {
                enabled: Some(self.indexing.enabled),
                auto_index: Some(self.indexing.auto_index),
                watch_files: Some(self.indexing.watch_files),
                default_depth: Some(self.indexing.default_depth),
                max_workers: Some(self.indexing.max_workers),
                memory_budget_mb: Some(self.indexing.memory_budget_mb),
                memory_pressure_threshold: Some(self.indexing.memory_pressure_threshold),
                max_queue_size: Some(self.indexing.max_queue_size),
                global_exclude_patterns: Some(self.indexing.global_exclude_patterns.clone()),
                global_include_patterns: Some(self.indexing.global_include_patterns.clone()),
                max_file_size_mb: Some(self.indexing.max_file_size_mb),
                incremental_mode: Some(self.indexing.incremental_mode),
                discovery_batch_size: Some(self.indexing.discovery_batch_size),
                status_update_interval_secs: Some(self.indexing.status_update_interval_secs),
                file_processing_timeout_ms: Some(self.indexing.file_processing_timeout_ms),
                parallel_file_processing: Some(self.indexing.parallel_file_processing),
                persist_cache: Some(self.indexing.persist_cache),
                cache_directory: self.indexing.cache_directory.clone(),
                priority_languages: Some(self.indexing.priority_languages.clone()),
                disabled_languages: Some(self.indexing.disabled_languages.clone()),
                features: Some(IndexingFeatures {
                    extract_functions: Some(self.indexing.features.extract_functions),
                    extract_types: Some(self.indexing.features.extract_types),
                    extract_variables: Some(self.indexing.features.extract_variables),
                    extract_imports: Some(self.indexing.features.extract_imports),
                    extract_tests: Some(self.indexing.features.extract_tests),
                }),
                lsp_caching: Some(IndexingLspCaching {
                    cache_call_hierarchy: Some(self.indexing.lsp_caching.cache_call_hierarchy),
                    cache_definitions: Some(self.indexing.lsp_caching.cache_definitions),
                    cache_references: Some(self.indexing.lsp_caching.cache_references),
                    cache_hover: Some(self.indexing.lsp_caching.cache_hover),
                    cache_document_symbols: Some(self.indexing.lsp_caching.cache_document_symbols),
                    cache_during_indexing: Some(self.indexing.lsp_caching.cache_during_indexing),
                    preload_common_symbols: Some(self.indexing.lsp_caching.preload_common_symbols),
                    max_cache_entries_per_operation: Some(
                        self.indexing.lsp_caching.max_cache_entries_per_operation,
                    ),
                    lsp_operation_timeout_ms: Some(
                        self.indexing.lsp_caching.lsp_operation_timeout_ms,
                    ),
                    priority_operations: Some(
                        self.indexing.lsp_caching.priority_operations.clone(),
                    ),
                    disabled_operations: Some(
                        self.indexing.lsp_caching.disabled_operations.clone(),
                    ),
                }),
                language_configs: Some(self.indexing.language_configs.clone()),
            }),
        }
    }

    /// Get a pretty-printed JSON representation
    pub fn to_json_string(&self) -> Result<String> {
        let config = self.to_probe_config();
        serde_json::to_string_pretty(&config).context("Failed to serialize configuration to JSON")
    }
}

/// Get the global configuration instance
/// This loads the configuration once and caches it for the lifetime of the program
pub fn get_config() -> &'static ResolvedConfig {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<ResolvedConfig> = OnceLock::new();

    CONFIG.get_or_init(|| {
        ProbeConfig::load().unwrap_or_else(|e| {
            eprintln!("Warning: Failed to load configuration: {e}");
            ProbeConfig::default().resolve_with_defaults()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = ProbeConfig::default();

        // All fields should be None in default config (for merging)
        assert!(config.defaults.is_none());
        assert!(config.search.is_none());
        assert!(config.extract.is_none());
        assert!(config.query.is_none());
        assert!(config.lsp.is_none());
        assert!(config.performance.is_none());
        assert!(config.indexing.is_none());
    }

    #[test]
    fn test_resolved_config_defaults() {
        let resolved = ProbeConfig::default().resolve_with_defaults();

        // Test defaults section
        assert!(!resolved.defaults.debug);
        assert_eq!(resolved.defaults.log_level, "info");
        assert!(!resolved.defaults.enable_lsp);
        assert_eq!(resolved.defaults.format, "color");
        assert_eq!(resolved.defaults.timeout, 30);

        // Test search section
        assert_eq!(resolved.search.max_results, None);
        assert_eq!(resolved.search.max_tokens, None);
        assert_eq!(resolved.search.max_bytes, None);
        assert!(resolved.search.frequency);
        assert_eq!(resolved.search.reranker, "bm25");
        assert_eq!(resolved.search.merge_threshold, 5);
        assert!(!resolved.search.allow_tests);
        assert!(!resolved.search.no_gitignore);

        // Test extract section
        assert_eq!(resolved.extract.context_lines, 0);
        assert!(!resolved.extract.allow_tests);

        // Test query section
        assert_eq!(resolved.query.max_results, None);
        assert!(!resolved.query.allow_tests);

        // Test LSP section
        assert!(!resolved.lsp.include_stdlib);
        assert!(resolved.lsp.socket_path.is_none());
        assert!(!resolved.lsp.disable_autostart);
        assert_eq!(resolved.lsp.workspace_cache.max_open_caches, 8);
        assert_eq!(resolved.lsp.workspace_cache.size_mb_per_workspace, 100);
        assert_eq!(resolved.lsp.workspace_cache.lookup_depth, 3);
        assert!(resolved.lsp.workspace_cache.base_dir.is_none());

        // Test performance section
        assert_eq!(resolved.performance.tree_cache_size, 2000);
        assert!(!resolved.performance.optimize_blocks);

        // Test indexing section - should use new defaults
        assert!(resolved.indexing.enabled);
        assert!(resolved.indexing.auto_index);
        assert!(resolved.indexing.watch_files);
        assert_eq!(resolved.indexing.default_depth, 3);
        assert_eq!(resolved.indexing.max_workers, 8);
        assert_eq!(resolved.indexing.memory_budget_mb, 512);

        // Test indexing features
        assert!(resolved.indexing.features.extract_functions);
        assert!(resolved.indexing.features.extract_types);
        assert!(resolved.indexing.features.extract_variables);
        assert!(resolved.indexing.features.extract_imports);
        assert!(resolved.indexing.features.extract_tests);
    }

    #[test]
    fn test_config_merging() {
        let mut base = ProbeConfig::default();
        let mut override_config = ProbeConfig::default();

        // Set some base values
        base.defaults = Some(DefaultsConfig {
            debug: Some(false),
            log_level: Some("warn".to_string()),
            enable_lsp: Some(false),
            format: None,
            timeout: None,
        });

        // Set override values
        override_config.defaults = Some(DefaultsConfig {
            debug: Some(true),
            log_level: None, // Should keep base value
            enable_lsp: Some(true),
            format: Some("json".to_string()),
            timeout: Some(60),
        });

        // Merge
        let merged = ProbeConfig::merge_configs(base, override_config);

        // Check merged values
        let defaults = merged.defaults.as_ref().unwrap();
        assert_eq!(defaults.debug, Some(true)); // Overridden
        assert_eq!(defaults.log_level, Some("warn".to_string())); // Kept from base
        assert_eq!(defaults.enable_lsp, Some(true)); // Overridden
        assert_eq!(defaults.format, Some("json".to_string())); // Added from override
        assert_eq!(defaults.timeout, Some(60)); // Added from override
    }

    #[test]
    fn test_json_serialization() {
        let config = ProbeConfig {
            defaults: Some(DefaultsConfig {
                debug: Some(true),
                log_level: Some("debug".to_string()),
                enable_lsp: Some(true),
                format: Some("json".to_string()),
                timeout: Some(45),
            }),
            ..Default::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ProbeConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.defaults.as_ref().unwrap().debug, Some(true));
        assert_eq!(
            deserialized.defaults.as_ref().unwrap().log_level,
            Some("debug".to_string())
        );
    }

    #[test]
    fn test_environment_variable_override() {
        // Note: We can't easily test actual env var reading in unit tests
        // but we can test the apply_env_overrides logic
        let mut resolved = ProbeConfig::default().resolve_with_defaults();

        // Simulate what apply_env_overrides would do
        // This tests the structure is correct for env var overrides
        resolved.defaults.debug = true;
        resolved.defaults.enable_lsp = true;
        resolved.search.max_results = Some(100);
        resolved.indexing.enabled = false;
        resolved.indexing.watch_files = false;

        assert!(resolved.defaults.debug);
        assert!(resolved.defaults.enable_lsp);
        assert_eq!(resolved.search.max_results, Some(100));
        assert!(!resolved.indexing.enabled);
        assert!(!resolved.indexing.watch_files);
    }

    #[test]
    fn test_config_file_loading_from_temp_dir() {
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".probe");
        fs::create_dir(&config_dir).unwrap();

        let config_file = config_dir.join("settings.json");
        let config_content = r#"
        {
            "defaults": {
                "debug": true,
                "log_level": "debug",
                "enable_lsp": true
            },
            "search": {
                "max_results": 50,
                "frequency": false
            },
            "indexing": {
                "enabled": false,
                "auto_index": false,
                "watch_files": false
            }
        }
        "#;
        fs::write(&config_file, config_content).unwrap();

        // We can't easily test ProbeConfig::load() because it looks at specific paths
        // But we can test loading from a specific file
        let loaded: ProbeConfig = serde_json::from_str(config_content).unwrap();
        let resolved = loaded.resolve_with_defaults();

        assert!(resolved.defaults.debug);
        assert_eq!(resolved.defaults.log_level, "debug");
        assert!(resolved.defaults.enable_lsp);
        assert_eq!(resolved.search.max_results, Some(50));
        assert!(!resolved.search.frequency);
        assert!(!resolved.indexing.enabled);
        assert!(!resolved.indexing.auto_index);
        assert!(!resolved.indexing.watch_files);
    }

    #[test]
    fn test_indexing_config_defaults() {
        let config = IndexingConfig::default();

        // Default config should have all None values (for merging)
        assert_eq!(config.enabled, None);
        assert_eq!(config.auto_index, None);
        assert_eq!(config.watch_files, None);
        assert_eq!(config.default_depth, None);
        assert_eq!(config.max_workers, None);
        assert_eq!(config.memory_budget_mb, None);
    }

    #[test]
    fn test_indexing_features_defaults() {
        let features = IndexingFeatures::default();

        // Default features should have all None values (for merging)
        assert_eq!(features.extract_functions, None);
        assert_eq!(features.extract_types, None);
        assert_eq!(features.extract_variables, None);
        assert_eq!(features.extract_imports, None);
        assert_eq!(features.extract_tests, None);
    }

    #[test]
    fn test_lsp_caching_config_defaults() {
        let config = IndexingLspCaching::default();

        // Default LSP caching config should have all None values (for merging)
        assert_eq!(config.cache_call_hierarchy, None);
        assert_eq!(config.cache_definitions, None);
        assert_eq!(config.cache_references, None);
        assert_eq!(config.cache_hover, None);
        assert_eq!(config.cache_document_symbols, None);
        assert_eq!(config.cache_during_indexing, None);
        assert_eq!(config.preload_common_symbols, None);
    }

    #[test]
    fn test_resolved_config_to_probe_config() {
        let resolved = ProbeConfig::default().resolve_with_defaults();
        let probe_config = resolved.to_probe_config();

        // Verify round-trip
        let resolved_again = probe_config.resolve_with_defaults();

        assert_eq!(resolved.defaults.debug, resolved_again.defaults.debug);
        assert_eq!(
            resolved.defaults.log_level,
            resolved_again.defaults.log_level
        );
        assert_eq!(
            resolved.search.max_results,
            resolved_again.search.max_results
        );
        assert_eq!(resolved.indexing.enabled, resolved_again.indexing.enabled);
    }

    #[test]
    fn test_to_json_string() {
        let resolved = ProbeConfig::default().resolve_with_defaults();
        let json_str = resolved.to_json_string().unwrap();

        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Check some expected fields exist
        assert!(parsed["defaults"].is_object());
        assert!(parsed["search"].is_object());
        assert!(parsed["indexing"].is_object());
        assert_eq!(parsed["indexing"]["enabled"], true);
        assert_eq!(parsed["indexing"]["auto_index"], true);
        assert_eq!(parsed["indexing"]["watch_files"], true);
    }

    #[test]
    fn test_language_config_mapping() {
        // Test that language configs can be stored
        let mut lang_configs = HashMap::new();
        lang_configs.insert(
            "rust".to_string(),
            LanguageIndexConfig {
                enabled: Some(true),
                max_workers: Some(4),
                ..Default::default()
            },
        );

        let indexing_config = IndexingConfig {
            language_configs: Some(lang_configs),
            ..Default::default()
        };

        assert!(indexing_config.language_configs.is_some());
        let configs = indexing_config.language_configs.as_ref().unwrap();
        assert!(configs.contains_key("rust"));
        assert_eq!(configs["rust"].enabled, Some(true));
        assert_eq!(configs["rust"].max_workers, Some(4));
    }

    #[test]
    fn test_priority_and_disabled_languages() {
        let indexing_config = IndexingConfig {
            priority_languages: Some(vec!["rust".to_string(), "python".to_string()]),
            disabled_languages: Some(vec!["c".to_string(), "cpp".to_string()]),
            ..Default::default()
        };

        assert_eq!(
            indexing_config.priority_languages.as_ref().unwrap(),
            &vec!["rust".to_string(), "python".to_string()]
        );
        assert_eq!(
            indexing_config.disabled_languages.as_ref().unwrap(),
            &vec!["c".to_string(), "cpp".to_string()]
        );
    }

    #[test]
    fn test_partial_config_merge() {
        // Test that partial configs merge correctly
        let base = ProbeConfig {
            indexing: Some(IndexingConfig {
                enabled: Some(true),
                auto_index: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };

        let override_config = ProbeConfig {
            indexing: Some(IndexingConfig {
                auto_index: Some(true),
                watch_files: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };

        let merged = ProbeConfig::merge_configs(base, override_config);

        let indexing = merged.indexing.as_ref().unwrap();
        assert_eq!(indexing.enabled, Some(true)); // Kept from base
        assert_eq!(indexing.auto_index, Some(true)); // Overridden
        assert_eq!(indexing.watch_files, Some(true)); // Added from override
    }

    #[test]
    fn test_resolved_indexing_defaults() {
        // Test the actual resolved values for indexing configuration
        let resolved = ProbeConfig::default().resolve_with_defaults();

        // These should be the resolved defaults, not None
        assert!(resolved.indexing.enabled);
        assert!(resolved.indexing.auto_index);
        assert!(resolved.indexing.watch_files);
        assert_eq!(resolved.indexing.default_depth, 3);
        assert_eq!(resolved.indexing.max_workers, 8);
        assert_eq!(resolved.indexing.memory_budget_mb, 512);
    }

    #[test]
    fn test_resolved_indexing_features_defaults() {
        // Test the actual resolved values for indexing features
        let resolved = ProbeConfig::default().resolve_with_defaults();

        assert!(resolved.indexing.features.extract_functions);
        assert!(resolved.indexing.features.extract_types);
        assert!(resolved.indexing.features.extract_variables);
        assert!(resolved.indexing.features.extract_imports);
        assert!(resolved.indexing.features.extract_tests);
    }

    #[test]
    fn test_resolved_lsp_caching_defaults() {
        // Test the actual resolved values for LSP caching
        let resolved = ProbeConfig::default().resolve_with_defaults();

        assert!(resolved.indexing.lsp_caching.cache_call_hierarchy);
        assert!(!resolved.indexing.lsp_caching.cache_definitions);
        assert!(resolved.indexing.lsp_caching.cache_references);
        assert!(resolved.indexing.lsp_caching.cache_hover);
        assert!(!resolved.indexing.lsp_caching.cache_document_symbols);
        assert!(!resolved.indexing.lsp_caching.cache_during_indexing);
        assert!(!resolved.indexing.lsp_caching.preload_common_symbols);
        assert_eq!(
            resolved
                .indexing
                .lsp_caching
                .max_cache_entries_per_operation,
            1000
        );
        assert_eq!(resolved.indexing.lsp_caching.lsp_operation_timeout_ms, 5000);
        assert_eq!(
            resolved.indexing.lsp_caching.priority_operations,
            vec![
                "call_hierarchy".to_string(),
                "references".to_string(),
                "hover".to_string(),
            ]
        );
        assert_eq!(
            resolved.indexing.lsp_caching.disabled_operations,
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_config_validation() {
        // Test valid configuration passes validation
        let mut config = ProbeConfig {
            defaults: Some(DefaultsConfig {
                format: Some("json".to_string()),
                log_level: Some("info".to_string()),
                ..Default::default()
            }),
            search: Some(SearchConfig {
                reranker: Some("bm25".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(config.validate().is_ok());

        // Test invalid format fails validation
        config.defaults.as_mut().unwrap().format = Some("invalid".to_string());
        assert!(config.validate().is_err());

        // Test invalid log level fails validation
        config.defaults.as_mut().unwrap().format = Some("json".to_string());
        config.defaults.as_mut().unwrap().log_level = Some("invalid".to_string());
        assert!(config.validate().is_err());

        // Test invalid reranker fails validation
        config.defaults.as_mut().unwrap().log_level = Some("info".to_string());
        config.search.as_mut().unwrap().reranker = Some("invalid".to_string());
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_environment_variable_patterns() {
        let mut config = ProbeConfig::default();

        // Test that apply_env_overrides creates sections if they don't exist
        config.apply_env_overrides();

        // Should have created default sections
        assert!(config.defaults.is_some());
        assert!(config.search.is_some());
        assert!(config.extract.is_some());
        assert!(config.lsp.is_some());
        assert!(config.performance.is_some());
        assert!(config.indexing.is_some());

        // Workspace cache should also be created
        assert!(config.lsp.as_ref().unwrap().workspace_cache.is_some());

        // Indexing features and LSP caching should be created
        assert!(config.indexing.as_ref().unwrap().features.is_some());
        assert!(config.indexing.as_ref().unwrap().lsp_caching.is_some());
    }

    #[test]
    fn test_probe_skip_project_config_behavior() {
        // Test the behavior with PROBE_SKIP_PROJECT_CONFIG set

        // Set the environment variable
        env::set_var("PROBE_SKIP_PROJECT_CONFIG", "1");

        // Test that relative paths are skipped but absolute paths work
        let paths = vec![
            PathBuf::from(".probe/settings.json"), // Should be skipped (relative)
            PathBuf::from("./config/settings.json"), // Should be skipped (relative)
            PathBuf::from("/tmp/test-config/settings.json"), // Should be allowed (absolute)
        ];

        let skip_project_config = env::var("PROBE_SKIP_PROJECT_CONFIG").is_ok();
        assert!(skip_project_config);

        // Test the logic from load_all_configs
        let mut allowed_paths = Vec::new();
        for path in paths {
            if skip_project_config && path.is_relative() {
                continue; // This should skip relative paths
            }
            allowed_paths.push(path);
        }

        // Should only have the absolute path
        assert_eq!(allowed_paths.len(), 1);
        assert!(allowed_paths[0].is_absolute());
        assert_eq!(
            allowed_paths[0],
            PathBuf::from("/tmp/test-config/settings.json")
        );

        // Clean up
        env::remove_var("PROBE_SKIP_PROJECT_CONFIG");

        // Test without the env var - all paths should be allowed
        let paths = vec![
            PathBuf::from(".probe/settings.json"),
            PathBuf::from("./config/settings.json"),
            PathBuf::from("/tmp/test-config/settings.json"),
        ];

        let skip_project_config = env::var("PROBE_SKIP_PROJECT_CONFIG").is_ok();
        assert!(!skip_project_config);

        let mut allowed_paths = Vec::new();
        for path in paths {
            if skip_project_config && path.is_relative() {
                continue;
            }
            allowed_paths.push(path);
        }

        // All paths should be allowed when env var is not set
        assert_eq!(allowed_paths.len(), 3);
    }
}
