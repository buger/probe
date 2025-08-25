use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tracing::{debug, info, warn};

use crate::cache_types::LspOperation;
use crate::language_detector::Language;

/// Comprehensive configuration for the indexing subsystem
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    /// Master switch to enable/disable indexing entirely
    pub enabled: bool,

    /// Auto-index workspaces when they are initialized
    pub auto_index: bool,

    /// Enable file watching for incremental indexing
    pub watch_files: bool,

    /// Default indexing depth for nested projects
    pub default_depth: u32,

    /// Number of worker threads for indexing
    pub max_workers: usize,

    /// Memory budget in megabytes (0 = unlimited)
    pub memory_budget_mb: u64,

    /// Memory pressure threshold (0.0-1.0) to trigger backpressure
    pub memory_pressure_threshold: f64,

    /// Maximum queue size for pending files (0 = unlimited)
    pub max_queue_size: usize,

    /// Global file patterns to exclude from indexing
    pub global_exclude_patterns: Vec<String>,

    /// Global file patterns to include (empty = include all)
    pub global_include_patterns: Vec<String>,

    /// Maximum file size to index (bytes)
    pub max_file_size_bytes: u64,

    /// Whether to use incremental indexing based on file modification time
    pub incremental_mode: bool,

    /// Batch size for file discovery operations
    pub discovery_batch_size: usize,

    /// Interval between status updates (seconds)
    pub status_update_interval_secs: u64,

    /// Timeout for processing a single file (milliseconds)
    pub file_processing_timeout_ms: u64,

    /// Enable parallel processing within a single file
    pub parallel_file_processing: bool,

    /// Cache parsed results to disk
    pub persist_cache: bool,

    /// Directory for persistent cache storage
    pub cache_directory: Option<PathBuf>,

    /// Global indexing features configuration
    pub features: IndexingFeatures,

    /// Per-language configuration overrides
    pub language_configs: HashMap<Language, LanguageIndexConfig>,

    /// Priority languages to index first
    pub priority_languages: Vec<Language>,

    /// Languages to completely skip during indexing
    pub disabled_languages: Vec<Language>,

    /// LSP operation caching configuration
    #[serde(default)]
    pub lsp_caching: LspCachingConfig,
}

/// Configuration for LSP operation caching during indexing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCachingConfig {
    /// Enable caching of call hierarchy operations during indexing
    pub cache_call_hierarchy: bool,

    /// Enable caching of definition lookups during indexing
    pub cache_definitions: bool,

    /// Enable caching of reference lookups during indexing
    pub cache_references: bool,

    /// Enable caching of hover information during indexing
    pub cache_hover: bool,

    /// Enable caching of document symbols during indexing
    pub cache_document_symbols: bool,

    /// Whether to perform LSP operations during indexing (vs only on-demand)
    pub cache_during_indexing: bool,

    /// Whether to preload cache with common operations after indexing
    pub preload_common_symbols: bool,

    /// Maximum number of LSP operations to cache per operation type during indexing
    pub max_cache_entries_per_operation: usize,

    /// Timeout for LSP operations during indexing (milliseconds)
    pub lsp_operation_timeout_ms: u64,

    /// Operations to prioritize during indexing (performed first)
    pub priority_operations: Vec<LspOperation>,

    /// Operations to completely skip during indexing
    pub disabled_operations: Vec<LspOperation>,
}

impl Default for LspCachingConfig {
    fn default() -> Self {
        Self {
            // CORRECTED defaults - cache operations actually used by search/extract
            cache_call_hierarchy: true, // ✅ MOST IMPORTANT - primary operation for search/extract
            cache_definitions: false,   // ❌ NOT used by search/extract commands
            cache_references: true,     // ✅ Used by extract for reference counts
            cache_hover: true,          // ✅ Used by extract for documentation/type info
            cache_document_symbols: false, // ❌ NOT used by search/extract commands

            // Indexing behavior
            cache_during_indexing: false, // Off by default to avoid slowing indexing
            preload_common_symbols: false, // Off by default to avoid overhead

            // Limits and timeouts
            max_cache_entries_per_operation: 1000, // Reasonable limit
            lsp_operation_timeout_ms: 5000,        // 5 second timeout during indexing

            // Priority and filtering - CORRECTED to prioritize operations used by search/extract
            priority_operations: vec![
                LspOperation::CallHierarchy,
                LspOperation::References,
                LspOperation::Hover,
            ],
            disabled_operations: vec![], // None disabled by default
        }
    }
}

impl LspCachingConfig {
    /// Load LSP caching configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();

        // Individual operation caching flags
        if let Ok(value) = std::env::var("PROBE_LSP_CACHE_CALL_HIERARCHY") {
            config.cache_call_hierarchy = parse_bool_env(&value, "PROBE_LSP_CACHE_CALL_HIERARCHY")?;
        }

        if let Ok(value) = std::env::var("PROBE_LSP_CACHE_DEFINITIONS") {
            config.cache_definitions = parse_bool_env(&value, "PROBE_LSP_CACHE_DEFINITIONS")?;
        }

        if let Ok(value) = std::env::var("PROBE_LSP_CACHE_REFERENCES") {
            config.cache_references = parse_bool_env(&value, "PROBE_LSP_CACHE_REFERENCES")?;
        }

        if let Ok(value) = std::env::var("PROBE_LSP_CACHE_HOVER") {
            config.cache_hover = parse_bool_env(&value, "PROBE_LSP_CACHE_HOVER")?;
        }

        if let Ok(value) = std::env::var("PROBE_LSP_CACHE_DOCUMENT_SYMBOLS") {
            config.cache_document_symbols =
                parse_bool_env(&value, "PROBE_LSP_CACHE_DOCUMENT_SYMBOLS")?;
        }

        // Indexing behavior flags
        if let Ok(value) = std::env::var("PROBE_LSP_CACHE_DURING_INDEXING") {
            config.cache_during_indexing =
                parse_bool_env(&value, "PROBE_LSP_CACHE_DURING_INDEXING")?;
        }

        if let Ok(value) = std::env::var("PROBE_LSP_PRELOAD_COMMON_SYMBOLS") {
            config.preload_common_symbols =
                parse_bool_env(&value, "PROBE_LSP_PRELOAD_COMMON_SYMBOLS")?;
        }

        // Numeric configurations
        if let Ok(value) = std::env::var("PROBE_LSP_MAX_CACHE_ENTRIES_PER_OPERATION") {
            config.max_cache_entries_per_operation = value
                .parse()
                .context("Invalid value for PROBE_LSP_MAX_CACHE_ENTRIES_PER_OPERATION")?;
        }

        if let Ok(value) = std::env::var("PROBE_LSP_OPERATION_TIMEOUT_MS") {
            config.lsp_operation_timeout_ms = value
                .parse()
                .context("Invalid value for PROBE_LSP_OPERATION_TIMEOUT_MS")?;
        }

        // Priority operations (comma-separated list)
        if let Ok(value) = std::env::var("PROBE_LSP_PRIORITY_OPERATIONS") {
            config.priority_operations =
                parse_lsp_operations_list(&value, "PROBE_LSP_PRIORITY_OPERATIONS")?;
        }

        // Disabled operations (comma-separated list)
        if let Ok(value) = std::env::var("PROBE_LSP_DISABLED_OPERATIONS") {
            config.disabled_operations =
                parse_lsp_operations_list(&value, "PROBE_LSP_DISABLED_OPERATIONS")?;
        }

        Ok(config)
    }

    /// Merge with another LspCachingConfig, giving priority to the other
    pub fn merge_with(&mut self, other: Self) {
        // Use macro to reduce boilerplate
        macro_rules! merge_bool_field {
            ($field:ident) => {
                if other.$field != Self::default().$field {
                    self.$field = other.$field;
                }
            };
        }

        merge_bool_field!(cache_call_hierarchy);
        merge_bool_field!(cache_definitions);
        merge_bool_field!(cache_references);
        merge_bool_field!(cache_hover);
        merge_bool_field!(cache_document_symbols);
        merge_bool_field!(cache_during_indexing);
        merge_bool_field!(preload_common_symbols);

        if other.max_cache_entries_per_operation != Self::default().max_cache_entries_per_operation
        {
            self.max_cache_entries_per_operation = other.max_cache_entries_per_operation;
        }

        if other.lsp_operation_timeout_ms != Self::default().lsp_operation_timeout_ms {
            self.lsp_operation_timeout_ms = other.lsp_operation_timeout_ms;
        }

        if !other.priority_operations.is_empty() {
            self.priority_operations = other.priority_operations;
        }

        if !other.disabled_operations.is_empty() {
            self.disabled_operations = other.disabled_operations;
        }
    }

    /// Validate LSP caching configuration
    pub fn validate(&self) -> Result<()> {
        if self.lsp_operation_timeout_ms < 1000 {
            return Err(anyhow!("lsp_operation_timeout_ms must be at least 1000ms"));
        }

        if self.max_cache_entries_per_operation == 0 {
            return Err(anyhow!(
                "max_cache_entries_per_operation must be greater than 0"
            ));
        }

        if self.max_cache_entries_per_operation > 100000 {
            warn!(
                "max_cache_entries_per_operation is very high ({}), may consume excessive memory",
                self.max_cache_entries_per_operation
            );
        }

        Ok(())
    }

    /// Check if a specific LSP operation should be cached
    pub fn should_cache_operation(&self, operation: &LspOperation) -> bool {
        // First check if the operation is disabled
        if self.disabled_operations.contains(operation) {
            return false;
        }

        // Check operation-specific flags
        match operation {
            LspOperation::CallHierarchy => self.cache_call_hierarchy,
            LspOperation::Definition => self.cache_definitions,
            LspOperation::References => self.cache_references,
            LspOperation::Hover => self.cache_hover,
            LspOperation::DocumentSymbols => self.cache_document_symbols,
        }
    }

    /// Get priority for an LSP operation (higher = processed first)
    pub fn get_operation_priority(&self, operation: &LspOperation) -> u8 {
        if self.priority_operations.contains(operation) {
            100
        } else {
            50
        }
    }
}

/// Parse a comma-separated list of LSP operations
fn parse_lsp_operations_list(value: &str, var_name: &str) -> Result<Vec<LspOperation>> {
    let operations = value
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| match s.to_lowercase().as_str() {
            "call_hierarchy" | "callhierarchy" => Ok(LspOperation::CallHierarchy),
            "definition" | "definitions" => Ok(LspOperation::Definition),
            "references" => Ok(LspOperation::References),
            "hover" => Ok(LspOperation::Hover),
            "document_symbols" | "documentsymbols" => Ok(LspOperation::DocumentSymbols),
            _ => Err(anyhow!("Invalid LSP operation: {}", s)),
        })
        .collect::<Result<Vec<_>>>()
        .context(format!("Invalid LSP operations list for {var_name}"))?;

    Ok(operations)
}

/// Enhanced indexing features with fine-grained control
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingFeatures {
    /// Extract function and method signatures
    pub extract_functions: bool,

    /// Extract type definitions (classes, structs, interfaces)
    pub extract_types: bool,

    /// Extract variable and constant declarations
    pub extract_variables: bool,

    /// Extract import/export statements
    pub extract_imports: bool,

    /// Extract test-related symbols and functions
    pub extract_tests: bool,

    /// Extract error handling patterns
    pub extract_error_handling: bool,

    /// Extract configuration and setup code
    pub extract_config: bool,

    /// Extract database/ORM related symbols
    pub extract_database: bool,

    /// Extract API/HTTP endpoint definitions
    pub extract_api_endpoints: bool,

    /// Extract security-related annotations and patterns
    pub extract_security: bool,

    /// Extract performance-critical sections
    pub extract_performance: bool,

    /// Language-specific feature flags
    pub language_features: HashMap<String, bool>,

    /// Custom feature flags for extensibility
    pub custom_features: HashMap<String, bool>,
}

/// Per-language indexing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageIndexConfig {
    /// Override global enabled flag for this language
    pub enabled: Option<bool>,

    /// Language-specific worker count override
    pub max_workers: Option<usize>,

    /// Language-specific memory budget override (MB)
    pub memory_budget_mb: Option<u64>,

    /// Language-specific file size limit override
    pub max_file_size_bytes: Option<u64>,

    /// Language-specific timeout override (ms)
    pub timeout_ms: Option<u64>,

    /// File extensions to process for this language
    pub file_extensions: Vec<String>,

    /// Language-specific exclude patterns
    pub exclude_patterns: Vec<String>,

    /// Language-specific include patterns
    pub include_patterns: Vec<String>,

    /// Features specific to this language
    pub features: Option<IndexingFeatures>,

    /// Custom parser configuration for this language
    pub parser_config: HashMap<String, serde_json::Value>,

    /// Priority level for this language (higher = processed first)
    pub priority: u32,

    /// Enable parallel processing for this language
    pub parallel_processing: Option<bool>,

    /// Cache strategy for this language
    pub cache_strategy: CacheStrategy,
}

/// Cache strategy for language-specific indexing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CacheStrategy {
    /// No caching
    None,
    /// Memory-only caching
    Memory,
    /// Disk-based caching
    Disk,
    /// Hybrid memory + disk caching
    Hybrid,
}

impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            enabled: true,     // Enabled by default - matches test expectations
            auto_index: true,  // Auto-index enabled by default
            watch_files: true, // File watching enabled by default
            default_depth: 3,
            max_workers: num_cpus::get().min(8), // Reasonable default
            memory_budget_mb: 512,
            memory_pressure_threshold: 0.8,
            max_queue_size: 10000,
            global_exclude_patterns: vec![
                "*.git/*".to_string(),
                "*/node_modules/*".to_string(),
                "*/target/*".to_string(),
                "*/build/*".to_string(),
                "*/dist/*".to_string(),
                "*/.cargo/*".to_string(),
                "*/__pycache__/*".to_string(),
                "*.tmp".to_string(),
                "*.log".to_string(),
            ],
            global_include_patterns: vec![],
            max_file_size_bytes: 10 * 1024 * 1024, // 10MB - matches main config max_file_size_mb default
            incremental_mode: true,
            discovery_batch_size: 1000,
            status_update_interval_secs: 5,
            file_processing_timeout_ms: 30000, // 30 seconds
            parallel_file_processing: true,
            persist_cache: false,
            cache_directory: None,
            features: IndexingFeatures::default(),
            language_configs: HashMap::new(),
            priority_languages: vec![Language::Rust, Language::TypeScript, Language::Python],
            disabled_languages: vec![],
            lsp_caching: LspCachingConfig::default(),
        }
    }
}

impl Default for IndexingFeatures {
    fn default() -> Self {
        Self {
            extract_functions: true,
            extract_types: true,
            extract_variables: true,
            extract_imports: true,
            extract_tests: true,
            extract_error_handling: false,
            extract_config: false,
            extract_database: false,
            extract_api_endpoints: false,
            extract_security: false,
            extract_performance: false,
            language_features: HashMap::new(),
            custom_features: HashMap::new(),
        }
    }
}

impl Default for LanguageIndexConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            max_workers: None,
            memory_budget_mb: None,
            max_file_size_bytes: None,
            timeout_ms: None,
            file_extensions: vec![],
            exclude_patterns: vec![],
            include_patterns: vec![],
            features: None,
            parser_config: HashMap::new(),
            priority: 50, // Medium priority by default
            parallel_processing: None,
            cache_strategy: CacheStrategy::Memory,
        }
    }
}

impl IndexingFeatures {
    /// Create a minimal feature set for basic indexing
    pub fn minimal() -> Self {
        Self {
            extract_functions: true,
            extract_types: true,
            extract_variables: false,
            extract_imports: false,
            extract_tests: false,
            extract_error_handling: false,
            extract_config: false,
            extract_database: false,
            extract_api_endpoints: false,
            extract_security: false,
            extract_performance: false,
            language_features: HashMap::new(),
            custom_features: HashMap::new(),
        }
    }

    /// Create a comprehensive feature set for full indexing
    pub fn comprehensive() -> Self {
        Self {
            extract_functions: true,
            extract_types: true,
            extract_variables: true,
            extract_imports: true,
            extract_tests: true,
            extract_error_handling: true,
            extract_config: true,
            extract_database: true,
            extract_api_endpoints: true,
            extract_security: true,
            extract_performance: true,
            language_features: HashMap::new(),
            custom_features: HashMap::new(),
        }
    }

    /// Create a performance-focused feature set
    pub fn performance_focused() -> Self {
        Self {
            extract_functions: true,
            extract_types: true,
            extract_variables: false,
            extract_imports: true,
            extract_tests: false,
            extract_error_handling: true,
            extract_config: false,
            extract_database: true,
            extract_api_endpoints: true,
            extract_security: false,
            extract_performance: true,
            language_features: HashMap::new(),
            custom_features: HashMap::new(),
        }
    }

    /// Create a security-focused feature set
    pub fn security_focused() -> Self {
        Self {
            extract_functions: true,
            extract_types: true,
            extract_variables: true,
            extract_imports: true,
            extract_tests: false,
            extract_error_handling: true,
            extract_config: true, // Important for security misconfigurations
            extract_database: true,
            extract_api_endpoints: true,
            extract_security: true,
            extract_performance: false,
            language_features: HashMap::new(),
            custom_features: HashMap::new(),
        }
    }

    /// Enable/disable a language-specific feature
    pub fn set_language_feature(&mut self, feature_name: String, enabled: bool) {
        self.language_features.insert(feature_name, enabled);
    }

    /// Check if a language-specific feature is enabled
    pub fn is_language_feature_enabled(&self, feature_name: &str) -> bool {
        self.language_features
            .get(feature_name)
            .copied()
            .unwrap_or(false)
    }

    /// Enable/disable a custom feature
    pub fn set_custom_feature(&mut self, feature_name: String, enabled: bool) {
        self.custom_features.insert(feature_name, enabled);
    }

    /// Check if a custom feature is enabled
    pub fn is_custom_feature_enabled(&self, feature_name: &str) -> bool {
        self.custom_features
            .get(feature_name)
            .copied()
            .unwrap_or(false)
    }
}

impl IndexingConfig {
    /// Create IndexingConfig from the main application's configuration
    /// This bridges the gap between src/config.rs and lsp-daemon/src/indexing/config.rs
    pub fn from_main_config(main_indexing: &crate::protocol::IndexingConfig) -> Result<Self> {
        let mut config = Self::default();

        // Map fields from main config to LSP daemon config
        if let Some(workers) = main_indexing.max_workers {
            config.max_workers = workers;
        }

        if let Some(memory_mb) = main_indexing.memory_budget_mb {
            config.memory_budget_mb = memory_mb;
        }

        if !main_indexing.exclude_patterns.is_empty() {
            config.global_exclude_patterns = main_indexing.exclude_patterns.clone();
        }

        if !main_indexing.include_patterns.is_empty() {
            config.global_include_patterns = main_indexing.include_patterns.clone();
        }

        if let Some(file_size_mb) = main_indexing.max_file_size_mb {
            config.max_file_size_bytes = file_size_mb * 1024 * 1024;
        }

        if let Some(incremental) = main_indexing.incremental {
            config.incremental_mode = incremental;
        }

        if !main_indexing.languages.is_empty() {
            config.priority_languages = main_indexing
                .languages
                .iter()
                .filter_map(|s| s.parse().ok())
                .collect();
        }

        // Map LSP caching configuration
        config.lsp_caching.cache_call_hierarchy =
            main_indexing.cache_call_hierarchy.unwrap_or(true);
        config.lsp_caching.cache_definitions = main_indexing.cache_definitions.unwrap_or(false);
        config.lsp_caching.cache_references = main_indexing.cache_references.unwrap_or(true);
        config.lsp_caching.cache_hover = main_indexing.cache_hover.unwrap_or(true);
        config.lsp_caching.cache_document_symbols =
            main_indexing.cache_document_symbols.unwrap_or(false);
        config.lsp_caching.cache_during_indexing =
            main_indexing.cache_during_indexing.unwrap_or(false);
        config.lsp_caching.preload_common_symbols =
            main_indexing.preload_common_symbols.unwrap_or(false);

        if let Some(max_entries) = main_indexing.max_cache_entries_per_operation {
            config.lsp_caching.max_cache_entries_per_operation = max_entries;
        }

        if let Some(timeout_ms) = main_indexing.lsp_operation_timeout_ms {
            config.lsp_caching.lsp_operation_timeout_ms = timeout_ms;
        }

        // Map priority operations
        if !main_indexing.lsp_priority_operations.is_empty() {
            config.lsp_caching.priority_operations = main_indexing
                .lsp_priority_operations
                .iter()
                .filter_map(|s| match s.to_lowercase().as_str() {
                    "call_hierarchy" | "callhierarchy" => Some(LspOperation::CallHierarchy),
                    "definition" | "definitions" => Some(LspOperation::Definition),
                    "references" => Some(LspOperation::References),
                    "hover" => Some(LspOperation::Hover),
                    "document_symbols" | "documentsymbols" => Some(LspOperation::DocumentSymbols),
                    _ => None,
                })
                .collect();
        }

        // Map disabled operations
        if !main_indexing.lsp_disabled_operations.is_empty() {
            config.lsp_caching.disabled_operations = main_indexing
                .lsp_disabled_operations
                .iter()
                .filter_map(|s| match s.to_lowercase().as_str() {
                    "call_hierarchy" | "callhierarchy" => Some(LspOperation::CallHierarchy),
                    "definition" | "definitions" => Some(LspOperation::Definition),
                    "references" => Some(LspOperation::References),
                    "hover" => Some(LspOperation::Hover),
                    "document_symbols" | "documentsymbols" => Some(LspOperation::DocumentSymbols),
                    _ => None,
                })
                .collect();
        }

        config.validate()?;
        Ok(config)
    }

    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();

        // Master switches
        if let Ok(value) = std::env::var("PROBE_INDEX_ENABLED") {
            config.enabled = parse_bool_env(&value, "PROBE_INDEX_ENABLED")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_AUTO") {
            config.auto_index = parse_bool_env(&value, "PROBE_INDEX_AUTO")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_WATCH") {
            config.watch_files = parse_bool_env(&value, "PROBE_INDEX_WATCH")?;
        }

        // Numeric configurations
        if let Ok(value) = std::env::var("PROBE_INDEX_DEPTH") {
            config.default_depth = value
                .parse()
                .context("Invalid value for PROBE_INDEX_DEPTH")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_WORKERS") {
            let workers: usize = value
                .parse()
                .context("Invalid value for PROBE_INDEX_WORKERS")?;
            if workers > 0 && workers <= 64 {
                config.max_workers = workers;
            } else {
                return Err(anyhow!(
                    "PROBE_INDEX_WORKERS must be between 1 and 64, got {}",
                    workers
                ));
            }
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_MEMORY_MB") {
            config.memory_budget_mb = value
                .parse()
                .context("Invalid value for PROBE_INDEX_MEMORY_MB")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_MEMORY_THRESHOLD") {
            let threshold: f64 = value
                .parse()
                .context("Invalid value for PROBE_INDEX_MEMORY_THRESHOLD")?;
            if (0.0..=1.0).contains(&threshold) {
                config.memory_pressure_threshold = threshold;
            } else {
                return Err(anyhow!(
                    "PROBE_INDEX_MEMORY_THRESHOLD must be between 0.0 and 1.0, got {}",
                    threshold
                ));
            }
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_QUEUE_SIZE") {
            config.max_queue_size = value
                .parse()
                .context("Invalid value for PROBE_INDEX_QUEUE_SIZE")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_FILE_SIZE_MB") {
            let size_mb: u64 = value
                .parse()
                .context("Invalid value for PROBE_INDEX_FILE_SIZE_MB")?;
            config.max_file_size_bytes = size_mb * 1024 * 1024;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_TIMEOUT_MS") {
            config.file_processing_timeout_ms = value
                .parse()
                .context("Invalid value for PROBE_INDEX_TIMEOUT_MS")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_BATCH_SIZE") {
            config.discovery_batch_size = value
                .parse()
                .context("Invalid value for PROBE_INDEX_BATCH_SIZE")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_STATUS_INTERVAL") {
            config.status_update_interval_secs = value
                .parse()
                .context("Invalid value for PROBE_INDEX_STATUS_INTERVAL")?;
        }

        // Boolean flags
        if let Ok(value) = std::env::var("PROBE_INDEX_INCREMENTAL") {
            config.incremental_mode = parse_bool_env(&value, "PROBE_INDEX_INCREMENTAL")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_PARALLEL") {
            config.parallel_file_processing = parse_bool_env(&value, "PROBE_INDEX_PARALLEL")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_PERSIST_CACHE") {
            config.persist_cache = parse_bool_env(&value, "PROBE_INDEX_PERSIST_CACHE")?;
        }

        // Patterns
        if let Ok(value) = std::env::var("PROBE_INDEX_EXCLUDE") {
            config.global_exclude_patterns = value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_INCLUDE") {
            config.global_include_patterns = value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        // Cache directory
        if let Ok(value) = std::env::var("PROBE_INDEX_CACHE_DIR") {
            config.cache_directory = Some(PathBuf::from(value));
        }

        // Priority languages
        if let Ok(value) = std::env::var("PROBE_INDEX_PRIORITY_LANGS") {
            let languages: Result<Vec<Language>, _> =
                value.split(',').map(|s| s.trim().parse()).collect();
            config.priority_languages =
                languages.context("Invalid language in PROBE_INDEX_PRIORITY_LANGS")?;
        }

        // Disabled languages
        if let Ok(value) = std::env::var("PROBE_INDEX_DISABLED_LANGS") {
            let languages: Result<Vec<Language>, _> =
                value.split(',').map(|s| s.trim().parse()).collect();
            config.disabled_languages =
                languages.context("Invalid language in PROBE_INDEX_DISABLED_LANGS")?;
        }

        // Load feature configuration from environment
        config.features = IndexingFeatures::from_env()?;

        // Load LSP caching configuration from environment
        config.lsp_caching = LspCachingConfig::from_env()?;

        // Load per-language configurations
        config.language_configs = load_language_configs_from_env()?;

        Ok(config)
    }

    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .context(format!("Failed to read config file: {:?}", path.as_ref()))?;

        let config: Self =
            toml::from_str(&content).context("Failed to parse TOML configuration")?;

        config.validate()?;
        Ok(config)
    }

    /// Load configuration with priority: main config -> file -> env -> defaults
    pub fn load() -> Result<Self> {
        // First, try to load from the main application config system
        // This creates proper integration between the CLI config and LSP daemon config
        if let Ok(main_config) = load_main_config() {
            info!("Loading indexing configuration from main application config");
            let mut config = Self::from_main_config(&main_config)?;

            // Still allow environment variable overrides
            let env_config = Self::from_env()?;
            config.merge_with(env_config);

            config.validate()?;
            return Ok(config);
        } else {
            warn!("Could not load main application config, falling back to file/env configuration");
        }

        // Fallback: Start with defaults
        let mut config = Self::default();

        // Try to load from standard config locations
        let config_paths = [
            std::env::var("PROBE_INDEX_CONFIG").ok().map(PathBuf::from),
            dirs::config_dir().map(|d| d.join("probe").join("indexing.toml")),
            dirs::home_dir().map(|d| d.join(".probe").join("indexing.toml")),
            Some(PathBuf::from("indexing.toml")),
        ];

        for config_path in config_paths.into_iter().flatten() {
            if config_path.exists() {
                info!("Loading indexing configuration from {:?}", config_path);
                config = Self::from_file(&config_path)
                    .with_context(|| format!("Failed to load config from {config_path:?}"))?;
                break;
            }
        }

        // Override with environment variables
        let env_config = Self::from_env()?;
        config.merge_with(env_config);

        // Final validation
        config.validate()?;

        Ok(config)
    }

    /// Merge configuration with another, giving priority to the other
    pub fn merge_with(&mut self, other: Self) {
        // Use macro to reduce boilerplate for optional fields
        macro_rules! merge_field {
            ($field:ident) => {
                if other.$field != Self::default().$field {
                    self.$field = other.$field;
                }
            };
        }

        merge_field!(enabled);
        merge_field!(auto_index);
        merge_field!(watch_files);
        merge_field!(default_depth);
        merge_field!(max_workers);
        merge_field!(memory_budget_mb);
        merge_field!(memory_pressure_threshold);
        merge_field!(max_queue_size);
        merge_field!(max_file_size_bytes);
        merge_field!(incremental_mode);
        merge_field!(discovery_batch_size);
        merge_field!(status_update_interval_secs);
        merge_field!(file_processing_timeout_ms);
        merge_field!(parallel_file_processing);
        merge_field!(persist_cache);

        if !other.global_exclude_patterns.is_empty() {
            self.global_exclude_patterns = other.global_exclude_patterns;
        }

        if !other.global_include_patterns.is_empty() {
            self.global_include_patterns = other.global_include_patterns;
        }

        if other.cache_directory.is_some() {
            self.cache_directory = other.cache_directory;
        }

        if !other.priority_languages.is_empty() {
            self.priority_languages = other.priority_languages;
        }

        if !other.disabled_languages.is_empty() {
            self.disabled_languages = other.disabled_languages;
        }

        // Merge features, LSP caching config, and language configs
        self.features.merge_with(other.features);
        self.lsp_caching.merge_with(other.lsp_caching);
        for (lang, config) in other.language_configs {
            self.language_configs.insert(lang, config);
        }
    }

    /// Validate configuration for consistency and correctness
    pub fn validate(&self) -> Result<()> {
        // Check numeric constraints
        if self.max_workers == 0 {
            return Err(anyhow!("max_workers must be greater than 0"));
        }

        if self.max_workers > 64 {
            return Err(anyhow!(
                "max_workers should not exceed 64 for performance reasons"
            ));
        }

        if self.memory_pressure_threshold < 0.0 || self.memory_pressure_threshold > 1.0 {
            return Err(anyhow!(
                "memory_pressure_threshold must be between 0.0 and 1.0"
            ));
        }

        if self.default_depth == 0 {
            return Err(anyhow!("default_depth must be greater than 0"));
        }

        if self.file_processing_timeout_ms < 1000 {
            warn!(
                "file_processing_timeout_ms is very low ({}ms), may cause timeouts",
                self.file_processing_timeout_ms
            );
        }

        // Check cache directory if specified
        if let Some(ref cache_dir) = self.cache_directory {
            if self.persist_cache && !cache_dir.exists() {
                std::fs::create_dir_all(cache_dir)
                    .context(format!("Failed to create cache directory: {cache_dir:?}"))?;
            }
        }

        // Validate LSP caching configuration
        self.lsp_caching.validate()?;

        // Validate language configs
        for (language, config) in &self.language_configs {
            config.validate(language)?;
        }

        debug!("Configuration validation passed");
        Ok(())
    }

    /// Get effective configuration for a specific language
    pub fn for_language(&self, language: Language) -> EffectiveConfig {
        let language_config = self.language_configs.get(&language);

        EffectiveConfig {
            enabled: language_config
                .and_then(|c| c.enabled)
                .unwrap_or(self.enabled && !self.disabled_languages.contains(&language)),
            max_workers: language_config
                .and_then(|c| c.max_workers)
                .unwrap_or(self.max_workers),
            memory_budget_mb: language_config
                .and_then(|c| c.memory_budget_mb)
                .unwrap_or(self.memory_budget_mb),
            max_file_size_bytes: language_config
                .and_then(|c| c.max_file_size_bytes)
                .unwrap_or(self.max_file_size_bytes),
            timeout_ms: language_config
                .and_then(|c| c.timeout_ms)
                .unwrap_or(self.file_processing_timeout_ms),
            file_extensions: language_config
                .map(|c| c.file_extensions.clone())
                .unwrap_or_else(|| default_extensions_for_language(language)),
            exclude_patterns: {
                let mut patterns = self.global_exclude_patterns.clone();
                if let Some(lang_config) = language_config {
                    patterns.extend(lang_config.exclude_patterns.clone());
                }
                patterns
            },
            include_patterns: {
                let mut patterns = self.global_include_patterns.clone();
                if let Some(lang_config) = language_config {
                    patterns.extend(lang_config.include_patterns.clone());
                }
                patterns
            },
            features: language_config
                .and_then(|c| c.features.clone())
                .unwrap_or_else(|| self.features.clone()),
            parser_config: language_config
                .map(|c| c.parser_config.clone())
                .unwrap_or_default(),
            priority: language_config.map(|c| c.priority).unwrap_or_else(|| {
                if self.priority_languages.contains(&language) {
                    100
                } else {
                    50
                }
            }),
            parallel_processing: language_config
                .and_then(|c| c.parallel_processing)
                .unwrap_or(self.parallel_file_processing),
            cache_strategy: language_config.map(|c| c.cache_strategy.clone()).unwrap_or(
                if self.persist_cache {
                    CacheStrategy::Hybrid
                } else {
                    CacheStrategy::Memory
                },
            ),
        }
    }

    /// Convert to protocol IndexingConfig for API compatibility
    pub fn to_protocol_config(&self) -> crate::protocol::IndexingConfig {
        // Helper function to convert LspOperation to string
        let op_to_string = |op: &crate::cache_types::LspOperation| -> String {
            match op {
                crate::cache_types::LspOperation::CallHierarchy => "call_hierarchy".to_string(),
                crate::cache_types::LspOperation::Definition => "definition".to_string(),
                crate::cache_types::LspOperation::References => "references".to_string(),
                crate::cache_types::LspOperation::Hover => "hover".to_string(),
                crate::cache_types::LspOperation::DocumentSymbols => "document_symbols".to_string(),
            }
        };

        crate::protocol::IndexingConfig {
            max_workers: Some(self.max_workers),
            memory_budget_mb: Some(self.memory_budget_mb),
            exclude_patterns: self.global_exclude_patterns.clone(),
            include_patterns: self.global_include_patterns.clone(),
            max_file_size_mb: Some(self.max_file_size_bytes / 1024 / 1024),
            incremental: Some(self.incremental_mode),
            languages: self
                .priority_languages
                .iter()
                .map(|l| format!("{l:?}"))
                .collect(),
            recursive: true, // Always true in new config system

            // LSP Caching Configuration
            cache_call_hierarchy: Some(self.lsp_caching.cache_call_hierarchy),
            cache_definitions: Some(self.lsp_caching.cache_definitions),
            cache_references: Some(self.lsp_caching.cache_references),
            cache_hover: Some(self.lsp_caching.cache_hover),
            cache_document_symbols: Some(self.lsp_caching.cache_document_symbols),
            cache_during_indexing: Some(self.lsp_caching.cache_during_indexing),
            preload_common_symbols: Some(self.lsp_caching.preload_common_symbols),
            max_cache_entries_per_operation: Some(self.lsp_caching.max_cache_entries_per_operation),
            lsp_operation_timeout_ms: Some(self.lsp_caching.lsp_operation_timeout_ms),
            lsp_priority_operations: self
                .lsp_caching
                .priority_operations
                .iter()
                .map(op_to_string)
                .collect(),
            lsp_disabled_operations: self
                .lsp_caching
                .disabled_operations
                .iter()
                .map(op_to_string)
                .collect(),
        }
    }

    /// Create from protocol IndexingConfig for API compatibility
    pub fn from_protocol_config(protocol: &crate::protocol::IndexingConfig) -> Self {
        // Helper function to parse LSP operation from string
        let string_to_op = |s: &str| -> Option<crate::cache_types::LspOperation> {
            match s.to_lowercase().as_str() {
                "call_hierarchy" | "callhierarchy" => {
                    Some(crate::cache_types::LspOperation::CallHierarchy)
                }
                "definition" | "definitions" => Some(crate::cache_types::LspOperation::Definition),
                "references" => Some(crate::cache_types::LspOperation::References),
                "hover" => Some(crate::cache_types::LspOperation::Hover),
                "document_symbols" | "documentsymbols" => {
                    Some(crate::cache_types::LspOperation::DocumentSymbols)
                }
                _ => None,
            }
        };

        let mut config = Self::default();

        // Basic configuration
        if let Some(workers) = protocol.max_workers {
            config.max_workers = workers;
        }

        if let Some(memory) = protocol.memory_budget_mb {
            config.memory_budget_mb = memory;
        }

        if !protocol.exclude_patterns.is_empty() {
            config.global_exclude_patterns = protocol.exclude_patterns.clone();
        }

        if !protocol.include_patterns.is_empty() {
            config.global_include_patterns = protocol.include_patterns.clone();
        }

        if let Some(file_size) = protocol.max_file_size_mb {
            config.max_file_size_bytes = file_size * 1024 * 1024;
        }

        if let Some(incremental) = protocol.incremental {
            config.incremental_mode = incremental;
        }

        if !protocol.languages.is_empty() {
            config.priority_languages = protocol
                .languages
                .iter()
                .filter_map(|s| s.parse().ok())
                .collect();
        }

        // LSP Caching Configuration
        if let Some(cache_call_hierarchy) = protocol.cache_call_hierarchy {
            config.lsp_caching.cache_call_hierarchy = cache_call_hierarchy;
        }

        if let Some(cache_definitions) = protocol.cache_definitions {
            config.lsp_caching.cache_definitions = cache_definitions;
        }

        if let Some(cache_references) = protocol.cache_references {
            config.lsp_caching.cache_references = cache_references;
        }

        if let Some(cache_hover) = protocol.cache_hover {
            config.lsp_caching.cache_hover = cache_hover;
        }

        if let Some(cache_document_symbols) = protocol.cache_document_symbols {
            config.lsp_caching.cache_document_symbols = cache_document_symbols;
        }

        if let Some(cache_during_indexing) = protocol.cache_during_indexing {
            config.lsp_caching.cache_during_indexing = cache_during_indexing;
        }

        if let Some(preload_common_symbols) = protocol.preload_common_symbols {
            config.lsp_caching.preload_common_symbols = preload_common_symbols;
        }

        if let Some(max_cache_entries_per_operation) = protocol.max_cache_entries_per_operation {
            config.lsp_caching.max_cache_entries_per_operation = max_cache_entries_per_operation;
        }

        if let Some(lsp_operation_timeout_ms) = protocol.lsp_operation_timeout_ms {
            config.lsp_caching.lsp_operation_timeout_ms = lsp_operation_timeout_ms;
        }

        if !protocol.lsp_priority_operations.is_empty() {
            config.lsp_caching.priority_operations = protocol
                .lsp_priority_operations
                .iter()
                .filter_map(|s| string_to_op(s))
                .collect();
        }

        if !protocol.lsp_disabled_operations.is_empty() {
            config.lsp_caching.disabled_operations = protocol
                .lsp_disabled_operations
                .iter()
                .filter_map(|s| string_to_op(s))
                .collect();
        }

        config
    }
}

/// Effective configuration for a specific language after merging global and language-specific settings
#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub enabled: bool,
    pub max_workers: usize,
    pub memory_budget_mb: u64,
    pub max_file_size_bytes: u64,
    pub timeout_ms: u64,
    pub file_extensions: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub include_patterns: Vec<String>,
    pub features: IndexingFeatures,
    pub parser_config: HashMap<String, serde_json::Value>,
    pub priority: u32,
    pub parallel_processing: bool,
    pub cache_strategy: CacheStrategy,
}

impl IndexingFeatures {
    /// Load feature configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let mut features = Self::default();

        // Core features
        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_FUNCTIONS") {
            features.extract_functions = parse_bool_env(&value, "PROBE_INDEX_EXTRACT_FUNCTIONS")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_TYPES") {
            features.extract_types = parse_bool_env(&value, "PROBE_INDEX_EXTRACT_TYPES")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_VARIABLES") {
            features.extract_variables = parse_bool_env(&value, "PROBE_INDEX_EXTRACT_VARIABLES")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_IMPORTS") {
            features.extract_imports = parse_bool_env(&value, "PROBE_INDEX_EXTRACT_IMPORTS")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_TESTS") {
            features.extract_tests = parse_bool_env(&value, "PROBE_INDEX_EXTRACT_TESTS")?;
        }

        // Extended features
        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_ERROR_HANDLING") {
            features.extract_error_handling =
                parse_bool_env(&value, "PROBE_INDEX_EXTRACT_ERROR_HANDLING")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_CONFIG") {
            features.extract_config = parse_bool_env(&value, "PROBE_INDEX_EXTRACT_CONFIG")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_DATABASE") {
            features.extract_database = parse_bool_env(&value, "PROBE_INDEX_EXTRACT_DATABASE")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_API_ENDPOINTS") {
            features.extract_api_endpoints =
                parse_bool_env(&value, "PROBE_INDEX_EXTRACT_API_ENDPOINTS")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_SECURITY") {
            features.extract_security = parse_bool_env(&value, "PROBE_INDEX_EXTRACT_SECURITY")?;
        }

        if let Ok(value) = std::env::var("PROBE_INDEX_EXTRACT_PERFORMANCE") {
            features.extract_performance =
                parse_bool_env(&value, "PROBE_INDEX_EXTRACT_PERFORMANCE")?;
        }

        // Load language-specific features using pattern matching
        for (key, value) in std::env::vars() {
            if let Some(feature_name) = key.strip_prefix("PROBE_INDEX_LANG_") {
                if let Some(suffix) = feature_name.strip_suffix("_PIPELINE") {
                    let enabled = parse_bool_env(&value, &key)?;
                    features.set_language_feature(suffix.to_lowercase(), enabled);
                }
            }

            if let Some(feature_name) = key.strip_prefix("PROBE_INDEX_CUSTOM_") {
                let enabled = parse_bool_env(&value, &key)?;
                features.set_custom_feature(feature_name.to_lowercase(), enabled);
            }
        }

        Ok(features)
    }

    /// Merge with another IndexingFeatures, giving priority to the other
    pub fn merge_with(&mut self, other: Self) {
        // Use macro to reduce boilerplate
        macro_rules! merge_bool_field {
            ($field:ident) => {
                if other.$field != Self::default().$field {
                    self.$field = other.$field;
                }
            };
        }

        merge_bool_field!(extract_functions);
        merge_bool_field!(extract_types);
        merge_bool_field!(extract_variables);
        merge_bool_field!(extract_imports);
        merge_bool_field!(extract_tests);
        merge_bool_field!(extract_error_handling);
        merge_bool_field!(extract_config);
        merge_bool_field!(extract_database);
        merge_bool_field!(extract_api_endpoints);
        merge_bool_field!(extract_security);
        merge_bool_field!(extract_performance);

        // Merge maps
        for (key, value) in other.language_features {
            self.language_features.insert(key, value);
        }

        for (key, value) in other.custom_features {
            self.custom_features.insert(key, value);
        }
    }
}

impl LanguageIndexConfig {
    /// Validate language-specific configuration
    pub fn validate(&self, language: &Language) -> Result<()> {
        if let Some(workers) = self.max_workers {
            if workers == 0 || workers > 32 {
                return Err(anyhow!(
                    "max_workers for {:?} must be between 1 and 32",
                    language
                ));
            }
        }

        if let Some(timeout) = self.timeout_ms {
            if timeout < 1000 {
                warn!("timeout_ms for {:?} is very low ({}ms)", language, timeout);
            }
        }

        if self.priority > 255 {
            return Err(anyhow!("priority for {:?} must not exceed 255", language));
        }

        Ok(())
    }
}

/// Load per-language configurations from environment variables
fn load_language_configs_from_env() -> Result<HashMap<Language, LanguageIndexConfig>> {
    let mut configs = HashMap::new();

    // Load configurations for each supported language
    for language in [
        Language::Rust,
        Language::Python,
        Language::TypeScript,
        Language::JavaScript,
        Language::Go,
        Language::Java,
        Language::C,
        Language::Cpp,
    ] {
        let lang_str = format!("{language:?}").to_uppercase();
        let mut config = LanguageIndexConfig::default();
        let mut has_config = false;

        // Check for language-specific environment variables
        if let Ok(value) = std::env::var(format!("PROBE_INDEX_{lang_str}_ENABLED")) {
            config.enabled = Some(parse_bool_env(
                &value,
                &format!("PROBE_INDEX_{lang_str}_ENABLED"),
            )?);
            has_config = true;
        }

        if let Ok(value) = std::env::var(format!("PROBE_INDEX_{lang_str}_WORKERS")) {
            config.max_workers = Some(
                value
                    .parse()
                    .context(format!("Invalid value for PROBE_INDEX_{lang_str}_WORKERS"))?,
            );
            has_config = true;
        }

        if let Ok(value) = std::env::var(format!("PROBE_INDEX_{lang_str}_MEMORY_MB")) {
            config.memory_budget_mb = Some(value.parse().context(format!(
                "Invalid value for PROBE_INDEX_{lang_str}_MEMORY_MB"
            ))?);
            has_config = true;
        }

        if let Ok(value) = std::env::var(format!("PROBE_INDEX_{lang_str}_TIMEOUT_MS")) {
            config.timeout_ms = Some(value.parse().context(format!(
                "Invalid value for PROBE_INDEX_{lang_str}_TIMEOUT_MS"
            ))?);
            has_config = true;
        }

        if let Ok(value) = std::env::var(format!("PROBE_INDEX_{lang_str}_PRIORITY")) {
            config.priority = value
                .parse()
                .context(format!("Invalid value for PROBE_INDEX_{lang_str}_PRIORITY"))?;
            has_config = true;
        }

        if let Ok(value) = std::env::var(format!("PROBE_INDEX_{lang_str}_EXTENSIONS")) {
            config.file_extensions = value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            has_config = true;
        }

        if let Ok(value) = std::env::var(format!("PROBE_INDEX_{lang_str}_EXCLUDE")) {
            config.exclude_patterns = value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            has_config = true;
        }

        if let Ok(value) = std::env::var(format!("PROBE_INDEX_{lang_str}_PIPELINE")) {
            // Enable language-specific pipeline features
            let pipeline_enabled =
                parse_bool_env(&value, &format!("PROBE_INDEX_{lang_str}_PIPELINE"))?;
            if pipeline_enabled {
                let mut features = IndexingFeatures::default();

                // Enable language-specific features based on the language
                match language {
                    Language::Rust => {
                        features.set_language_feature("extract_macros".to_string(), true);
                        features.set_language_feature("extract_traits".to_string(), true);
                        features.set_language_feature("extract_lifetimes".to_string(), true);
                    }
                    Language::TypeScript | Language::JavaScript => {
                        features.set_language_feature("extract_interfaces".to_string(), true);
                        features.set_language_feature("extract_decorators".to_string(), true);
                        features.set_language_feature("extract_types".to_string(), true);
                    }
                    Language::Python => {
                        features.set_language_feature("extract_decorators".to_string(), true);
                        features.set_language_feature("extract_docstrings".to_string(), true);
                        features.set_language_feature("extract_async".to_string(), true);
                    }
                    Language::Go => {
                        features.set_language_feature("extract_interfaces".to_string(), true);
                        features.set_language_feature("extract_receivers".to_string(), true);
                        features.set_language_feature("extract_channels".to_string(), true);
                    }
                    Language::Java => {
                        features.set_language_feature("extract_annotations".to_string(), true);
                        features.set_language_feature("extract_generics".to_string(), true);
                    }
                    Language::C => {
                        features.set_language_feature("extract_preprocessor".to_string(), true);
                        features.set_language_feature("extract_headers".to_string(), true);
                    }
                    Language::Cpp => {
                        features.set_language_feature("extract_templates".to_string(), true);
                        features.set_language_feature("extract_namespaces".to_string(), true);
                        features.set_language_feature("extract_classes".to_string(), true);
                    }
                    _ => {}
                }

                config.features = Some(features);
                has_config = true;
            }
        }

        if has_config {
            configs.insert(language, config);
        }
    }

    Ok(configs)
}

/// Parse boolean environment variable with proper error handling
fn parse_bool_env(value: &str, var_name: &str) -> Result<bool> {
    match value.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" | "enabled" => Ok(true),
        "false" | "0" | "no" | "off" | "disabled" => Ok(false),
        _ => Err(anyhow!("Invalid boolean value for {}: {} (use true/false, 1/0, yes/no, on/off, enabled/disabled)", var_name, value)),
    }
}

/// Get default file extensions for a language
fn default_extensions_for_language(language: Language) -> Vec<String> {
    match language {
        Language::Rust => vec!["rs".to_string()],
        Language::Python => vec!["py".to_string(), "pyi".to_string()],
        Language::TypeScript => vec!["ts".to_string(), "tsx".to_string()],
        Language::JavaScript => vec!["js".to_string(), "jsx".to_string(), "mjs".to_string()],
        Language::Go => vec!["go".to_string()],
        Language::Java => vec!["java".to_string()],
        Language::C => vec!["c".to_string(), "h".to_string()],
        Language::Cpp => vec![
            "cpp".to_string(),
            "cc".to_string(),
            "cxx".to_string(),
            "hpp".to_string(),
            "hxx".to_string(),
        ],
        _ => vec![],
    }
}

impl FromStr for Language {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "rust" => Ok(Language::Rust),
            "python" => Ok(Language::Python),
            "typescript" => Ok(Language::TypeScript),
            "javascript" => Ok(Language::JavaScript),
            "go" => Ok(Language::Go),
            "java" => Ok(Language::Java),
            "c" => Ok(Language::C),
            "cpp" | "c++" => Ok(Language::Cpp),
            _ => Err(anyhow!("Unknown language: {}", s)),
        }
    }
}

/// Helper function to load main application configuration
/// This bridges the gap between src/config.rs and lsp-daemon configuration
fn load_main_config() -> Result<crate::protocol::IndexingConfig> {
    // For now, we'll load from environment variables and standard config files
    // In the future, this could be enhanced to use IPC or shared configuration

    // Try to load probe configuration from standard locations
    let config_paths = [
        dirs::config_dir().map(|d| d.join("probe").join("settings.json")),
        dirs::home_dir().map(|d| d.join(".probe").join("settings.json")),
        Some(PathBuf::from(".probe/settings.json")),
        Some(PathBuf::from("settings.json")),
    ];

    for config_path in config_paths.into_iter().flatten() {
        if config_path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&config_path) {
                if let Ok(config) = serde_json::from_str::<serde_json::Value>(&contents) {
                    // Try to extract indexing configuration
                    if let Some(indexing) = config.get("indexing") {
                        if let Ok(indexing_config) = serde_json::from_value::<
                            crate::protocol::IndexingConfig,
                        >(indexing.clone())
                        {
                            info!("Loaded main config from {:?}", config_path);
                            return Ok(indexing_config);
                        }
                    }
                }
            }
        }
    }

    // Fallback: Return default protocol config that will be converted properly
    Ok(crate::protocol::IndexingConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = IndexingConfig::default();
        assert!(config.enabled); // Should be enabled by default
        assert!(config.auto_index); // Should be enabled by default
        assert!(config.watch_files); // Should be enabled by default
        assert_eq!(config.default_depth, 3);
        assert!(config.max_workers > 0);
        assert!(config.memory_budget_mb > 0);
    }

    #[test]
    fn test_features_presets() {
        let minimal = IndexingFeatures::minimal();
        assert!(minimal.extract_functions);
        assert!(minimal.extract_types);
        assert!(!minimal.extract_variables);
        assert!(!minimal.extract_imports);

        let comprehensive = IndexingFeatures::comprehensive();
        assert!(comprehensive.extract_functions);
        assert!(comprehensive.extract_types);
        assert!(comprehensive.extract_variables);
        assert!(comprehensive.extract_imports);
        assert!(comprehensive.extract_security);

        let security = IndexingFeatures::security_focused();
        assert!(security.extract_security);
        assert!(security.extract_security); // Important for security
        assert!(security.extract_config);
        assert!(!security.extract_performance);

        let performance = IndexingFeatures::performance_focused();
        assert!(performance.extract_performance);
        assert!(performance.extract_performance);
        assert!(!performance.extract_security);
    }

    #[test]
    fn test_env_var_parsing() {
        // Test boolean parsing
        assert!(parse_bool_env("true", "TEST").unwrap());
        assert!(parse_bool_env("1", "TEST").unwrap());
        assert!(parse_bool_env("yes", "TEST").unwrap());
        assert!(parse_bool_env("on", "TEST").unwrap());
        assert!(parse_bool_env("enabled", "TEST").unwrap());

        assert!(!parse_bool_env("false", "TEST").unwrap());
        assert!(!parse_bool_env("0", "TEST").unwrap());
        assert!(!parse_bool_env("no", "TEST").unwrap());
        assert!(!parse_bool_env("off", "TEST").unwrap());
        assert!(!parse_bool_env("disabled", "TEST").unwrap());

        assert!(parse_bool_env("invalid", "TEST").is_err());
    }

    #[test]
    fn test_language_config_validation() {
        let mut config = LanguageIndexConfig::default();

        // Valid config should pass
        assert!(config.validate(&Language::Rust).is_ok());

        // Invalid worker count
        config.max_workers = Some(0);
        assert!(config.validate(&Language::Rust).is_err());

        config.max_workers = Some(16); // This should be ok (within 1-32 range)
        assert!(config.validate(&Language::Rust).is_ok());

        // Invalid priority
        config.priority = 300;
        assert!(config.validate(&Language::Rust).is_err());
    }

    #[test]
    fn test_effective_config() {
        let mut base_config = IndexingConfig::default();
        base_config.enabled = true;
        base_config.max_workers = 4;

        // Test language without specific config
        let effective = base_config.for_language(Language::Rust);
        assert!(effective.enabled);
        assert_eq!(effective.max_workers, 4);

        // Test language with specific config
        let mut rust_config = LanguageIndexConfig::default();
        rust_config.max_workers = Some(8);
        rust_config.enabled = Some(false);

        base_config
            .language_configs
            .insert(Language::Rust, rust_config);

        let effective = base_config.for_language(Language::Rust);
        assert!(!effective.enabled); // Language-specific override
        assert_eq!(effective.max_workers, 8); // Language-specific override
    }

    #[test]
    fn test_config_merge() {
        let mut base = IndexingConfig::default();
        base.enabled = false; // Override the new default
        base.max_workers = 2;

        let mut override_config = IndexingConfig::default();
        override_config.enabled = true; // Explicitly set to test merge (same as default but explicit)
        override_config.memory_budget_mb = 1024; // Different from default
        override_config.max_workers = 8; // Different from base, should be ignored since it's default

        base.merge_with(override_config);

        // The merge logic only applies fields that differ from default
        // Since override_config.enabled == default (true), it won't merge
        // So base.enabled stays false
        assert!(!base.enabled); // Should remain false since override equals default
        assert_eq!(base.memory_budget_mb, 1024); // Should be overridden (different from default)
        assert_eq!(base.max_workers, 2); // Should remain from base
    }

    #[test]
    fn test_default_extensions() {
        assert_eq!(default_extensions_for_language(Language::Rust), vec!["rs"]);
        assert_eq!(
            default_extensions_for_language(Language::Python),
            vec!["py", "pyi"]
        );
        assert_eq!(
            default_extensions_for_language(Language::TypeScript),
            vec!["ts", "tsx"]
        );
        assert!(default_extensions_for_language(Language::Unknown).is_empty());
    }

    #[test]
    fn test_language_from_str() {
        assert_eq!("rust".parse::<Language>().unwrap(), Language::Rust);
        assert_eq!("python".parse::<Language>().unwrap(), Language::Python);
        assert_eq!(
            "typescript".parse::<Language>().unwrap(),
            Language::TypeScript
        );
        assert_eq!("cpp".parse::<Language>().unwrap(), Language::Cpp);
        assert_eq!("c++".parse::<Language>().unwrap(), Language::Cpp);
        assert!("unknown".parse::<Language>().is_err());
    }

    #[test]
    fn test_comprehensive_config_creation() {
        let config = IndexingConfig::load().unwrap();

        // Test that it creates a valid configuration
        assert!(config.validate().is_ok());

        // Test effective config for different languages
        let rust_config = config.for_language(Language::Rust);
        assert_eq!(rust_config.file_extensions, vec!["rs"]);
        assert!(rust_config.features.extract_functions);

        let python_config = config.for_language(Language::Python);
        assert_eq!(python_config.file_extensions, vec!["py", "pyi"]);
        assert!(python_config.features.extract_functions);
    }

    #[test]
    fn test_feature_flag_inheritance() {
        let mut config = IndexingConfig::default();

        // Set global features
        config.features.extract_security = true;
        config.features.extract_performance = true;

        // Create language-specific config
        let mut rust_config = LanguageIndexConfig::default();
        let mut rust_features = IndexingFeatures::default();
        rust_features.extract_types = false; // Override global
        rust_config.features = Some(rust_features);

        config.language_configs.insert(Language::Rust, rust_config);

        // Test effective configuration
        let effective = config.for_language(Language::Rust);
        assert!(!effective.features.extract_types); // Should be overridden
        assert!(effective.features.extract_functions); // Should come from language default
    }

    #[test]
    fn test_environment_variable_patterns() {
        // Test that environment variable names follow expected patterns
        let config = IndexingConfig::default();

        // Test protocol conversion
        let protocol_config = config.to_protocol_config();
        assert_eq!(protocol_config.max_workers, Some(config.max_workers));
        assert_eq!(
            protocol_config.memory_budget_mb,
            Some(config.memory_budget_mb)
        );

        // Test round-trip conversion
        let restored_config = IndexingConfig::from_protocol_config(&protocol_config);
        assert_eq!(restored_config.max_workers, config.max_workers);
        assert_eq!(restored_config.memory_budget_mb, config.memory_budget_mb);
    }

    #[test]
    fn test_cache_strategy_defaults() {
        let config = LanguageIndexConfig::default();
        match config.cache_strategy {
            CacheStrategy::Memory => {} // Expected default
            _ => panic!("Expected Memory cache strategy as default"),
        }

        // Test that hybrid strategy works with persistence
        let mut indexing_config = IndexingConfig::default();
        indexing_config.persist_cache = true;

        let effective = indexing_config.for_language(Language::Rust);
        match effective.cache_strategy {
            CacheStrategy::Hybrid => {}
            _ => panic!("Expected Hybrid cache strategy when persistence is enabled"),
        }
    }

    #[test]
    fn test_lsp_caching_config() {
        // Test CORRECTED default LSP caching configuration - matches actual search/extract usage
        let config = LspCachingConfig::default();
        assert!(config.cache_call_hierarchy); // ✅ MOST IMPORTANT - primary operation for search/extract
        assert!(!config.cache_definitions); // ❌ NOT used by search/extract commands
        assert!(config.cache_references); // ✅ Used by extract for reference counts
        assert!(config.cache_hover); // ✅ Used by extract for documentation/type info
        assert!(!config.cache_document_symbols); // ❌ NOT used by search/extract commands
        assert!(!config.cache_during_indexing); // Performance default
        assert!(!config.preload_common_symbols); // Performance default
        assert_eq!(config.max_cache_entries_per_operation, 1000);
        assert_eq!(config.lsp_operation_timeout_ms, 5000);

        // Test validation
        assert!(config.validate().is_ok());

        // Test invalid configurations
        let mut invalid_config = config.clone();
        invalid_config.lsp_operation_timeout_ms = 500; // Too low
        assert!(invalid_config.validate().is_err());

        invalid_config.lsp_operation_timeout_ms = 5000;
        invalid_config.max_cache_entries_per_operation = 0; // Invalid
        assert!(invalid_config.validate().is_err());

        // Test operation checking - CORRECTED to match actual usage
        use crate::cache_types::LspOperation;
        assert!(!config.should_cache_operation(&LspOperation::Definition)); // ❌ NOT used by search/extract
        assert!(config.should_cache_operation(&LspOperation::CallHierarchy)); // ✅ MOST IMPORTANT for search/extract

        // Test priority - CORRECTED to prioritize operations used by search/extract
        assert_eq!(
            config.get_operation_priority(&LspOperation::CallHierarchy),
            100
        ); // High priority - primary operation
        assert_eq!(
            config.get_operation_priority(&LspOperation::References),
            100
        ); // High priority - used by extract
        assert_eq!(config.get_operation_priority(&LspOperation::Hover), 100); // High priority - used by extract
        assert_eq!(config.get_operation_priority(&LspOperation::Definition), 50);
        // Normal priority - not used
    }

    #[test]
    fn test_lsp_caching_environment_vars() {
        // This would normally test environment variable parsing, but we can't
        // modify env vars easily in unit tests. The functionality is tested
        // through integration tests.
        let config = LspCachingConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_lsp_operation_parsing() {
        // Test parsing LSP operations from strings
        use crate::cache_types::LspOperation;

        let operations =
            parse_lsp_operations_list("definition,hover,call_hierarchy", "TEST").unwrap();
        assert_eq!(operations.len(), 3);
        assert!(operations.contains(&LspOperation::Definition));
        assert!(operations.contains(&LspOperation::Hover));
        assert!(operations.contains(&LspOperation::CallHierarchy));

        // Test case insensitive parsing
        let operations = parse_lsp_operations_list("DEFINITION,references", "TEST").unwrap();
        assert_eq!(operations.len(), 2);
        assert!(operations.contains(&LspOperation::Definition));
        assert!(operations.contains(&LspOperation::References));

        // Test invalid operation
        let result = parse_lsp_operations_list("definition,invalid_op", "TEST");
        assert!(result.is_err());
    }

    #[test]
    fn test_config_merging_with_lsp_caching() {
        let mut base = IndexingConfig::default();
        base.lsp_caching.cache_definitions = false;

        let mut override_config = IndexingConfig::default();
        override_config.lsp_caching.cache_definitions = true;
        override_config.lsp_caching.cache_call_hierarchy = true;

        base.merge_with(override_config);

        assert!(base.lsp_caching.cache_definitions); // Should be overridden
        assert!(base.lsp_caching.cache_call_hierarchy); // Should be set from override
        assert!(base.lsp_caching.cache_hover); // Should remain from base default
    }

    #[test]
    fn test_protocol_conversion_with_lsp_caching() {
        let mut internal_config = IndexingConfig::default();
        internal_config.lsp_caching.cache_definitions = true;
        internal_config.lsp_caching.cache_call_hierarchy = false;
        internal_config.lsp_caching.max_cache_entries_per_operation = 2000;

        // Test conversion to protocol
        let protocol_config = internal_config.to_protocol_config();
        assert_eq!(protocol_config.cache_definitions, Some(true));
        assert_eq!(protocol_config.cache_call_hierarchy, Some(false));
        assert_eq!(protocol_config.max_cache_entries_per_operation, Some(2000));

        // Test round-trip conversion
        let restored_config = IndexingConfig::from_protocol_config(&protocol_config);
        assert_eq!(restored_config.lsp_caching.cache_definitions, true);
        assert_eq!(restored_config.lsp_caching.cache_call_hierarchy, false);
        assert_eq!(
            restored_config.lsp_caching.max_cache_entries_per_operation,
            2000
        );
    }

    #[test]
    fn test_disabled_languages() {
        let mut config = IndexingConfig::default();
        config.enabled = true;
        config.disabled_languages = vec![Language::C, Language::Cpp];

        let c_effective = config.for_language(Language::C);
        let rust_effective = config.for_language(Language::Rust);

        assert!(!c_effective.enabled); // Should be disabled
        assert!(rust_effective.enabled); // Should be enabled
    }

    #[test]
    fn test_priority_languages() {
        let mut config = IndexingConfig::default();
        config.priority_languages = vec![Language::Rust, Language::Python];

        let rust_effective = config.for_language(Language::Rust);
        let go_effective = config.for_language(Language::Go);

        assert_eq!(rust_effective.priority, 100); // Priority language
        assert_eq!(go_effective.priority, 50); // Default priority
    }
}
