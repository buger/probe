//! Language-specific processing pipelines for indexing
//!
//! This module provides configurable processing pipelines for different programming languages.
//! Each pipeline can extract symbols, analyze structure, and prepare data for semantic search.
//! Feature flags allow selective enabling/disabling of indexing capabilities.

use crate::indexing::config::IndexingFeatures;
use crate::indexing::language_strategies::{
    IndexingPriority, LanguageIndexingStrategy, LanguageStrategyFactory,
};
use crate::language_detector::Language;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{debug, error, info};

/// Configuration for a language-specific pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Language this pipeline handles
    pub language: Language,

    /// Features to enable for this language
    pub features: IndexingFeatures,

    /// Maximum file size to process (bytes)
    pub max_file_size: u64,

    /// Timeout for processing a single file (milliseconds)
    pub timeout_ms: u64,

    /// File extensions to process for this language
    pub file_extensions: Vec<String>,

    /// Patterns to exclude from processing
    pub exclude_patterns: Vec<String>,

    /// Parser-specific configuration
    pub parser_config: HashMap<String, serde_json::Value>,
}

impl PipelineConfig {
    /// Create default configuration for a language
    pub fn for_language(language: Language) -> Self {
        let (extensions, features) = match language {
            Language::Rust => {
                let mut features = IndexingFeatures::default();
                features.set_language_feature("extract_macros".to_string(), true);
                features.set_language_feature("extract_traits".to_string(), true);
                (vec!["rs".to_string()], features)
            }
            Language::TypeScript => {
                let mut features = IndexingFeatures::default();
                features.set_language_feature("extract_interfaces".to_string(), true);
                features.set_language_feature("extract_decorators".to_string(), true);
                (vec!["ts".to_string(), "tsx".to_string()], features)
            }
            Language::JavaScript => {
                let mut features = IndexingFeatures::default();
                features.set_language_feature("extract_prototypes".to_string(), true);
                (
                    vec!["js".to_string(), "jsx".to_string(), "mjs".to_string()],
                    features,
                )
            }
            Language::Python => {
                let mut features = IndexingFeatures::default();
                features.set_language_feature("extract_decorators".to_string(), true);
                features.set_language_feature("extract_docstrings".to_string(), true);
                (vec!["py".to_string(), "pyi".to_string()], features)
            }
            Language::Go => {
                let mut features = IndexingFeatures::default();
                features.set_language_feature("extract_interfaces".to_string(), true);
                features.set_language_feature("extract_receivers".to_string(), true);
                (vec!["go".to_string()], features)
            }
            Language::Java => {
                let mut features = IndexingFeatures::default();
                features.set_language_feature("extract_annotations".to_string(), true);
                (vec!["java".to_string()], features)
            }
            Language::C => {
                let mut features = IndexingFeatures::minimal();
                features.set_language_feature("extract_preprocessor".to_string(), true);
                (vec!["c".to_string(), "h".to_string()], features)
            }
            Language::Cpp => {
                let mut features = IndexingFeatures::default();
                features.set_language_feature("extract_templates".to_string(), true);
                features.set_language_feature("extract_namespaces".to_string(), true);
                (
                    vec![
                        "cpp".to_string(),
                        "cc".to_string(),
                        "cxx".to_string(),
                        "hpp".to_string(),
                    ],
                    features,
                )
            }
            _ => (vec![], IndexingFeatures::minimal()),
        };

        Self {
            language,
            features,
            max_file_size: 10 * 1024 * 1024, // 10MB
            timeout_ms: 30000,               // 30 seconds
            file_extensions: extensions,
            // Don't exclude test files - they're valid source code that should be indexed
            exclude_patterns: vec![],
            parser_config: HashMap::new(),
        }
    }

    /// Check if this pipeline should process the given file
    pub fn should_process_file(&self, file_path: &Path) -> bool {
        // Check file extension
        if !self.file_extensions.is_empty() {
            if let Some(extension) = file_path.extension().and_then(|ext| ext.to_str()) {
                if !self.file_extensions.iter().any(|ext| ext == extension) {
                    return false;
                }
            } else {
                return false; // No extension and extensions are specified
            }
        }

        // Check exclusion patterns
        let path_str = file_path.to_string_lossy();
        for pattern in &self.exclude_patterns {
            if Self::matches_pattern(&path_str, pattern) {
                return false;
            }
        }

        true
    }

    /// Simple pattern matching (supports * wildcards)
    fn matches_pattern(text: &str, pattern: &str) -> bool {
        // Simple glob-like pattern matching
        if pattern.contains('*') {
            let parts: Vec<&str> = pattern.split('*').collect();
            if parts.len() == 2 {
                let (prefix, suffix) = (parts[0], parts[1]);
                return text.starts_with(prefix) && text.ends_with(suffix);
            } else if parts.len() > 2 {
                // Multiple wildcards - check if text contains all the parts in order
                let mut search_start = 0;
                for (i, part) in parts.iter().enumerate() {
                    if part.is_empty() {
                        continue; // Skip empty parts from consecutive '*'
                    }

                    if i == 0 {
                        // First part should be at the beginning
                        if !text.starts_with(part) {
                            return false;
                        }
                        search_start = part.len();
                    } else if i == parts.len() - 1 {
                        // Last part should be at the end
                        return text.ends_with(part);
                    } else {
                        // Middle parts should be found in order
                        if let Some(pos) = text[search_start..].find(part) {
                            search_start += pos + part.len();
                        } else {
                            return false;
                        }
                    }
                }
                return true;
            }
        }

        text.contains(pattern)
    }

    /// Create pipeline configuration from comprehensive IndexingConfig
    pub fn from_indexing_config(
        indexing_config: &crate::indexing::IndexingConfig,
        language: Language,
    ) -> Self {
        let effective_config = indexing_config.for_language(language);

        Self {
            language,
            features: effective_config.features,
            max_file_size: effective_config.max_file_size_bytes,
            timeout_ms: effective_config.timeout_ms,
            file_extensions: effective_config.file_extensions,
            exclude_patterns: effective_config.exclude_patterns,
            parser_config: effective_config.parser_config,
        }
    }
}

/// Result of processing a file through a pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    /// File that was processed
    pub file_path: PathBuf,

    /// Language detected/used
    pub language: Language,

    /// Number of bytes processed
    pub bytes_processed: u64,

    /// Number of symbols found
    pub symbols_found: u64,

    /// Processing time in milliseconds
    pub processing_time_ms: u64,

    /// Extracted symbols by category
    pub symbols: HashMap<String, Vec<SymbolInfo>>,

    /// Errors encountered during processing
    pub errors: Vec<String>,

    /// Warnings generated
    pub warnings: Vec<String>,

    /// Additional metadata
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Information about an extracted symbol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    /// Symbol name
    pub name: String,

    /// Symbol kind (function, class, variable, etc.)
    pub kind: String,

    /// Line number where symbol is defined
    pub line: u32,

    /// Column number where symbol starts
    pub column: u32,

    /// End line (for multi-line symbols)
    pub end_line: Option<u32>,

    /// End column
    pub end_column: Option<u32>,

    /// Documentation string if available
    pub documentation: Option<String>,

    /// Symbol signature or type information
    pub signature: Option<String>,

    /// Visibility (public, private, etc.)
    pub visibility: Option<String>,

    /// Indexing priority calculated by language strategy
    pub priority: Option<IndexingPriority>,

    /// Whether this symbol is exported/public
    pub is_exported: bool,

    /// Additional attributes
    pub attributes: HashMap<String, String>,
}

/// Language-specific processing pipeline
#[derive(Debug, Clone)]
pub struct LanguagePipeline {
    /// Configuration for this pipeline
    config: PipelineConfig,

    /// Language-specific indexing strategy
    strategy: LanguageIndexingStrategy,

    /// Performance metrics
    files_processed: u64,
    total_processing_time: u64,
    last_error: Option<String>,
}

impl LanguagePipeline {
    /// Create a new language pipeline
    pub fn new(language: Language) -> Self {
        let config = PipelineConfig::for_language(language);
        let strategy = LanguageStrategyFactory::create_strategy(language);

        info!("Created language pipeline for {:?} with strategy", language);

        Self {
            config,
            strategy,
            files_processed: 0,
            total_processing_time: 0,
            last_error: None,
        }
    }

    /// Create a pipeline with custom configuration
    pub fn with_config(config: PipelineConfig) -> Self {
        let strategy = LanguageStrategyFactory::create_strategy(config.language);

        Self {
            config,
            strategy,
            files_processed: 0,
            total_processing_time: 0,
            last_error: None,
        }
    }

    /// Process a file and extract symbols
    pub async fn process_file(&mut self, file_path: &Path) -> Result<PipelineResult> {
        let start_time = Instant::now();

        // Check if we should process this file
        if !self.config.should_process_file(file_path) {
            return Err(anyhow!("File {:?} excluded from processing", file_path));
        }

        // Read file content
        let content =
            fs::read_to_string(file_path).context(format!("Failed to read file: {file_path:?}"))?;

        // Check file size
        if content.len() as u64 > self.config.max_file_size {
            return Err(anyhow!(
                "File {:?} too large ({} bytes, max: {})",
                file_path,
                content.len(),
                self.config.max_file_size
            ));
        }

        // Process with timeout
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(self.config.timeout_ms),
            self.process_content(file_path, &content),
        )
        .await;

        let processing_time = start_time.elapsed().as_millis() as u64;
        self.files_processed += 1;
        self.total_processing_time += processing_time;

        match result {
            Ok(Ok(mut pipeline_result)) => {
                pipeline_result.processing_time_ms = processing_time;
                Ok(pipeline_result)
            }
            Ok(Err(e)) => {
                self.last_error = Some(e.to_string());
                Err(e)
            }
            Err(_) => {
                let error = format!("Processing timeout after {}ms", self.config.timeout_ms);
                self.last_error = Some(error.clone());
                Err(anyhow!(error))
            }
        }
    }

    /// Get the language-specific indexing strategy
    pub fn get_strategy(&self) -> &LanguageIndexingStrategy {
        &self.strategy
    }

    /// Calculate the priority of a file for indexing
    pub fn calculate_file_priority(&self, file_path: &Path) -> IndexingPriority {
        self.strategy.calculate_file_priority(file_path)
    }

    /// Check if the file should be processed based on language strategy
    pub fn should_process_file_with_strategy(&self, file_path: &Path) -> bool {
        self.strategy.should_process_file(file_path) && self.config.should_process_file(file_path)
    }

    /// Calculate symbol priority using language strategy
    pub fn calculate_symbol_priority(
        &self,
        symbol_type: &str,
        visibility: Option<&str>,
        has_documentation: bool,
        is_exported: bool,
    ) -> IndexingPriority {
        self.strategy.calculate_symbol_priority(
            symbol_type,
            visibility,
            has_documentation,
            is_exported,
        )
    }

    /// Process file content and extract symbols
    async fn process_content(&self, file_path: &Path, content: &str) -> Result<PipelineResult> {
        let mut result = PipelineResult {
            file_path: file_path.to_path_buf(),
            language: self.config.language,
            bytes_processed: content.len() as u64,
            symbols_found: 0,
            processing_time_ms: 0, // Will be set by caller
            symbols: HashMap::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            metadata: HashMap::new(),
        };

        // Extract symbols based on enabled features with priority calculation
        if self.config.features.extract_functions {
            let mut functions = self.extract_functions(content).await?;
            self.enhance_symbols_with_priority(&mut functions, "function");
            result.symbols_found += functions.len() as u64;
            result.symbols.insert("functions".to_string(), functions);
        }

        if self.config.features.extract_types {
            let mut types = self.extract_types(content).await?;
            self.enhance_symbols_with_priority(&mut types, "type");
            result.symbols_found += types.len() as u64;
            result.symbols.insert("types".to_string(), types);
        }

        if self.config.features.extract_variables {
            let mut variables = self.extract_variables(content).await?;
            self.enhance_symbols_with_priority(&mut variables, "variable");
            result.symbols_found += variables.len() as u64;
            result.symbols.insert("variables".to_string(), variables);
        }

        if self.config.features.extract_imports {
            let mut imports = self.extract_imports(content).await?;
            self.enhance_symbols_with_priority(&mut imports, "import");
            result.symbols_found += imports.len() as u64;
            result.symbols.insert("imports".to_string(), imports);
        }

        if self.config.features.extract_tests && self.strategy.file_strategy.include_tests {
            let mut tests = self.extract_tests(content).await?;
            self.enhance_symbols_with_priority(&mut tests, "test");
            result.symbols_found += tests.len() as u64;
            result.symbols.insert("tests".to_string(), tests);
        }

        // Language-specific extraction with strategy-based prioritization
        self.extract_language_specific(&mut result, content).await?;

        debug!(
            "Processed {:?}: {} symbols extracted in {} bytes",
            file_path, result.symbols_found, result.bytes_processed
        );

        Ok(result)
    }

    /// Extract function definitions (basic regex-based approach)
    async fn extract_functions(&self, content: &str) -> Result<Vec<SymbolInfo>> {
        let mut functions = Vec::new();

        let pattern = match self.config.language {
            Language::Rust => r"fn\s+(\w+)",
            Language::Python => r"def\s+(\w+)",
            Language::JavaScript | Language::TypeScript => r"function\s+(\w+)|(\w+)\s*=\s*function",
            Language::Go => r"func\s+(\w+)",
            Language::Java | Language::C | Language::Cpp => r"\w+\s+(\w+)\s*\(",
            _ => return Ok(functions), // Unsupported language
        };

        let regex = regex::Regex::new(pattern).context("Invalid function regex")?;

        for (line_num, line) in content.lines().enumerate() {
            for cap in regex.captures_iter(line) {
                if let Some(name_match) = cap.get(1).or_else(|| cap.get(2)) {
                    let function_name = name_match.as_str().to_string();

                    functions.push(SymbolInfo {
                        name: function_name,
                        kind: "function".to_string(),
                        line: (line_num + 1) as u32,
                        column: name_match.start() as u32,
                        end_line: None,
                        end_column: None,
                        documentation: None,
                        signature: Some(line.trim().to_string()),
                        visibility: self.detect_visibility(line),
                        priority: None, // Will be calculated later
                        is_exported: self.detect_export(line),
                        attributes: HashMap::new(),
                    });
                }
            }
        }

        Ok(functions)
    }

    /// Extract type definitions
    async fn extract_types(&self, content: &str) -> Result<Vec<SymbolInfo>> {
        let mut types = Vec::new();

        let pattern = match self.config.language {
            Language::Rust => r"struct\s+(\w+)|enum\s+(\w+)|trait\s+(\w+)|type\s+(\w+)",
            Language::Python => r"class\s+(\w+)",
            Language::TypeScript => r"interface\s+(\w+)|type\s+(\w+)|class\s+(\w+)",
            Language::Go => r"type\s+(\w+)\s+struct|type\s+(\w+)\s+interface",
            Language::Java => r"class\s+(\w+)|interface\s+(\w+)|enum\s+(\w+)",
            Language::C => r"struct\s+(\w+)|union\s+(\w+)|enum\s+(\w+)|typedef.*\s+(\w+)",
            Language::Cpp => r"class\s+(\w+)|struct\s+(\w+)|namespace\s+(\w+)",
            _ => return Ok(types),
        };

        let regex = regex::Regex::new(pattern).context("Invalid type regex")?;

        for (line_num, line) in content.lines().enumerate() {
            for cap in regex.captures_iter(line) {
                // Find the first non-empty capture group
                for i in 1..cap.len() {
                    if let Some(name_match) = cap.get(i) {
                        let type_name = name_match.as_str().to_string();

                        types.push(SymbolInfo {
                            name: type_name,
                            kind: "type".to_string(),
                            line: (line_num + 1) as u32,
                            column: name_match.start() as u32,
                            end_line: None,
                            end_column: None,
                            documentation: None,
                            signature: Some(line.trim().to_string()),
                            visibility: self.detect_visibility(line),
                            priority: None,
                            is_exported: self.detect_export(line),
                            attributes: HashMap::new(),
                        });
                        break;
                    }
                }
            }
        }

        Ok(types)
    }

    /// Extract variable declarations
    async fn extract_variables(&self, content: &str) -> Result<Vec<SymbolInfo>> {
        let mut variables = Vec::new();

        let pattern = match self.config.language {
            Language::Rust => r"let\s+(\w+)|const\s+(\w+)|static\s+(\w+)",
            Language::Python => r"(\w+)\s*=", // Simple assignment
            Language::JavaScript | Language::TypeScript => r"let\s+(\w+)|const\s+(\w+)|var\s+(\w+)",
            Language::Go => r"var\s+(\w+)|(\w+)\s*:=",
            _ => return Ok(variables),
        };

        let regex = regex::Regex::new(pattern).context("Invalid variable regex")?;

        for (line_num, line) in content.lines().enumerate() {
            // Skip function definitions and other non-variable lines
            if line.trim().starts_with("//") || line.trim().starts_with('#') {
                continue;
            }

            for cap in regex.captures_iter(line) {
                for i in 1..cap.len() {
                    if let Some(name_match) = cap.get(i) {
                        let var_name = name_match.as_str().to_string();

                        // Basic filtering to avoid false positives
                        if var_name.len() > 1 && !var_name.chars().all(|c| c.is_uppercase()) {
                            variables.push(SymbolInfo {
                                name: var_name,
                                kind: "variable".to_string(),
                                line: (line_num + 1) as u32,
                                column: name_match.start() as u32,
                                end_line: None,
                                end_column: None,
                                documentation: None,
                                signature: Some(line.trim().to_string()),
                                visibility: self.detect_visibility(line),
                                priority: None,
                                is_exported: self.detect_export(line),
                                attributes: HashMap::new(),
                            });
                        }
                        break;
                    }
                }
            }
        }

        Ok(variables)
    }

    /// Extract import statements
    async fn extract_imports(&self, content: &str) -> Result<Vec<SymbolInfo>> {
        let mut imports = Vec::new();

        let pattern = match self.config.language {
            Language::Rust => r"use\s+([\w:]+)",
            Language::Python => r"import\s+([\w.]+)|from\s+([\w.]+)\s+import",
            Language::JavaScript | Language::TypeScript => {
                r#"import.*from\s+['"]([^'"]+)['"]|import\s+['"]([^'"]+)['"]"#
            }
            Language::Go => r#"import\s+["']([^"']+)["']"#,
            Language::Java => r"import\s+([\w.]+)",
            _ => return Ok(imports),
        };

        let regex = regex::Regex::new(pattern).context("Invalid import regex")?;

        for (line_num, line) in content.lines().enumerate() {
            for cap in regex.captures_iter(line) {
                for i in 1..cap.len() {
                    if let Some(import_match) = cap.get(i) {
                        let import_name = import_match.as_str().to_string();

                        imports.push(SymbolInfo {
                            name: import_name,
                            kind: "import".to_string(),
                            line: (line_num + 1) as u32,
                            column: import_match.start() as u32,
                            end_line: None,
                            end_column: None,
                            documentation: None,
                            signature: Some(line.trim().to_string()),
                            visibility: None, // Imports don't have visibility
                            priority: None,
                            is_exported: false, // Imports are not exported
                            attributes: HashMap::new(),
                        });
                        break;
                    }
                }
            }
        }

        Ok(imports)
    }

    /// Extract test functions/methods
    async fn extract_tests(&self, content: &str) -> Result<Vec<SymbolInfo>> {
        let mut tests = Vec::new();

        let pattern = match self.config.language {
            Language::Rust => r"#\[test\]|#\[tokio::test\]",
            Language::Python => r"def\s+(test_\w+)",
            Language::JavaScript | Language::TypeScript => r"it\s*\(|test\s*\(|describe\s*\(",
            Language::Go => r"func\s+(Test\w+)",
            Language::Java => r"@Test",
            _ => return Ok(tests),
        };

        let regex = regex::Regex::new(pattern).context("Invalid test regex")?;

        for (line_num, line) in content.lines().enumerate() {
            if regex.is_match(line) {
                // For test attributes, look for the function on the next line
                let test_name = if line.trim().starts_with('#') || line.trim().starts_with('@') {
                    format!("test_at_line_{}", line_num + 1)
                } else if let Some(cap) = regex.captures(line) {
                    cap.get(1)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_else(|| format!("test_at_line_{}", line_num + 1))
                } else {
                    format!("test_at_line_{}", line_num + 1)
                };

                tests.push(SymbolInfo {
                    name: test_name,
                    kind: "test".to_string(),
                    line: (line_num + 1) as u32,
                    column: 0,
                    end_line: None,
                    end_column: None,
                    documentation: None,
                    signature: Some(line.trim().to_string()),
                    visibility: None, // Tests don't typically have visibility
                    priority: None,
                    is_exported: false, // Tests are not exported
                    attributes: HashMap::new(),
                });
            }
        }

        Ok(tests)
    }

    /// Extract language-specific symbols
    async fn extract_language_specific(
        &self,
        result: &mut PipelineResult,
        content: &str,
    ) -> Result<()> {
        match self.config.language {
            Language::Rust => {
                if self
                    .config
                    .features
                    .is_language_feature_enabled("extract_macros")
                {
                    let macros = self.extract_rust_macros(content).await?;
                    result.symbols_found += macros.len() as u64;
                    result.symbols.insert("macros".to_string(), macros);
                }
            }
            Language::Python => {
                if self
                    .config
                    .features
                    .is_language_feature_enabled("extract_decorators")
                {
                    let decorators = self.extract_python_decorators(content).await?;
                    result.symbols_found += decorators.len() as u64;
                    result.symbols.insert("decorators".to_string(), decorators);
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Extract Rust macro definitions
    async fn extract_rust_macros(&self, content: &str) -> Result<Vec<SymbolInfo>> {
        let mut macros = Vec::new();
        let regex = regex::Regex::new(r"macro_rules!\s+(\w+)")?;

        for (line_num, line) in content.lines().enumerate() {
            for cap in regex.captures_iter(line) {
                if let Some(name_match) = cap.get(1) {
                    macros.push(SymbolInfo {
                        name: name_match.as_str().to_string(),
                        kind: "macro".to_string(),
                        line: (line_num + 1) as u32,
                        column: name_match.start() as u32,
                        end_line: None,
                        end_column: None,
                        documentation: None,
                        signature: Some(line.trim().to_string()),
                        visibility: self.detect_visibility(line),
                        priority: None,
                        is_exported: self.detect_export(line),
                        attributes: HashMap::new(),
                    });
                }
            }
        }

        Ok(macros)
    }

    /// Extract Python decorators
    async fn extract_python_decorators(&self, content: &str) -> Result<Vec<SymbolInfo>> {
        let mut decorators = Vec::new();
        let regex = regex::Regex::new(r"@(\w+)")?;

        for (line_num, line) in content.lines().enumerate() {
            for cap in regex.captures_iter(line) {
                if let Some(name_match) = cap.get(1) {
                    decorators.push(SymbolInfo {
                        name: name_match.as_str().to_string(),
                        kind: "decorator".to_string(),
                        line: (line_num + 1) as u32,
                        column: name_match.start() as u32,
                        end_line: None,
                        end_column: None,
                        documentation: None,
                        signature: Some(line.trim().to_string()),
                        visibility: None, // Decorators don't have visibility
                        priority: None,
                        is_exported: false, // Decorators are not directly exported
                        attributes: HashMap::new(),
                    });
                }
            }
        }

        Ok(decorators)
    }

    /// Get pipeline statistics
    pub fn get_stats(&self) -> (u64, u64, Option<&String>) {
        (
            self.files_processed,
            self.total_processing_time,
            self.last_error.as_ref(),
        )
    }

    /// Reset pipeline statistics
    pub fn reset_stats(&mut self) {
        self.files_processed = 0;
        self.total_processing_time = 0;
        self.last_error = None;
    }

    /// Enhance symbols with priority information based on language strategy
    fn enhance_symbols_with_priority(&self, symbols: &mut Vec<SymbolInfo>, default_kind: &str) {
        for symbol in symbols {
            let kind = if symbol.kind.is_empty() {
                default_kind
            } else {
                &symbol.kind
            };
            let has_documentation = symbol.documentation.is_some()
                && !symbol.documentation.as_ref().unwrap().is_empty();

            symbol.priority = Some(self.strategy.calculate_symbol_priority(
                kind,
                symbol.visibility.as_deref(),
                has_documentation,
                symbol.is_exported,
            ));
        }
    }

    /// Detect visibility from a line of code
    fn detect_visibility(&self, line: &str) -> Option<String> {
        let trimmed = line.trim();

        match self.config.language {
            Language::Rust => {
                if trimmed.starts_with("pub ") || trimmed.contains(" pub ") {
                    Some("public".to_string())
                } else {
                    Some("private".to_string())
                }
            }
            Language::Python => {
                // Python doesn't have explicit visibility, use naming convention
                if trimmed.contains("def _") || trimmed.contains("class _") {
                    Some("private".to_string())
                } else {
                    Some("public".to_string())
                }
            }
            Language::Go => {
                // Go uses capitalization for visibility
                if let Some(word) = trimmed
                    .split_whitespace()
                    .find(|w| w.chars().next().unwrap_or('a').is_alphabetic())
                {
                    if word.chars().next().unwrap().is_uppercase() {
                        Some("public".to_string())
                    } else {
                        Some("private".to_string())
                    }
                } else {
                    None
                }
            }
            Language::TypeScript | Language::JavaScript => {
                if trimmed.contains("export ") {
                    Some("export".to_string())
                } else if trimmed.contains("private ") {
                    Some("private".to_string())
                } else if trimmed.contains("public ") {
                    Some("public".to_string())
                } else {
                    None
                }
            }
            Language::Java => {
                if trimmed.contains("public ") {
                    Some("public".to_string())
                } else if trimmed.contains("private ") {
                    Some("private".to_string())
                } else if trimmed.contains("protected ") {
                    Some("protected".to_string())
                } else {
                    Some("package".to_string())
                }
            }
            _ => None,
        }
    }

    /// Detect if a symbol is exported/public
    fn detect_export(&self, line: &str) -> bool {
        let trimmed = line.trim();

        match self.config.language {
            Language::Rust => trimmed.starts_with("pub ") || trimmed.contains(" pub "),
            Language::Python => {
                // Python doesn't have explicit exports, assume non-private is exported
                !trimmed.contains("def _") && !trimmed.contains("class _")
            }
            Language::Go => {
                // Go uses capitalization for exports
                if let Some(word) = trimmed
                    .split_whitespace()
                    .find(|w| w.chars().next().unwrap_or('a').is_alphabetic())
                {
                    word.chars().next().unwrap().is_uppercase()
                } else {
                    false
                }
            }
            Language::TypeScript | Language::JavaScript => trimmed.contains("export "),
            Language::Java => trimmed.contains("public "),
            _ => false,
        }
    }
}

/// Main indexing pipeline that manages all language-specific pipelines
#[derive(Debug, Clone)]
pub struct IndexingPipeline {
    /// Language this pipeline handles
    language: Language,

    /// Language-specific processor
    processor: LanguagePipeline,
}

impl IndexingPipeline {
    /// Create a new indexing pipeline for the specified language
    pub fn new(language: Language) -> Result<Self> {
        let processor = LanguagePipeline::new(language);

        Ok(Self {
            language,
            processor,
        })
    }

    /// Create a pipeline with custom configuration
    pub fn with_config(config: PipelineConfig) -> Result<Self> {
        let language = config.language;
        let processor = LanguagePipeline::with_config(config);

        Ok(Self {
            language,
            processor,
        })
    }

    /// Process a file using this pipeline
    pub async fn process_file(&mut self, file_path: &Path) -> Result<PipelineResult> {
        debug!(
            "Processing {:?} with {:?} pipeline",
            file_path, self.language
        );

        match self.processor.process_file(file_path).await {
            Ok(result) => {
                debug!(
                    "Successfully processed {:?}: {} symbols",
                    file_path, result.symbols_found
                );
                Ok(result)
            }
            Err(e) => {
                error!("Failed to process {:?}: {}", file_path, e);
                Err(e)
            }
        }
    }

    /// Get the language this pipeline handles
    pub fn language(&self) -> Language {
        self.language
    }

    /// Get pipeline statistics
    pub fn get_stats(&self) -> (u64, u64, Option<String>) {
        let (files, time, error) = self.processor.get_stats();
        (files, time, error.cloned())
    }

    /// Reset pipeline statistics
    pub fn reset_stats(&mut self) {
        self.processor.reset_stats();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_rust_pipeline() {
        let rust_code = r#"
fn main() {
    println!("Hello, world!");
}

struct Person {
    name: String,
    age: u32,
}

impl Person {
    fn new(name: String, age: u32) -> Self {
        Self { name, age }
    }
}

#[test]
fn test_person_creation() {
    let person = Person::new("Alice".to_string(), 30);
    assert_eq!(person.name, "Alice");
}
        "#;

        let temp_file = NamedTempFile::with_suffix(".rs").unwrap();
        std::fs::write(temp_file.path(), rust_code).unwrap();

        let mut pipeline = IndexingPipeline::new(Language::Rust).unwrap();
        let result = pipeline.process_file(temp_file.path()).await.unwrap();

        assert_eq!(result.language, Language::Rust);
        assert!(result.symbols_found > 0);
        assert!(result.symbols.contains_key("functions"));
        assert!(result.symbols.contains_key("types"));

        // Check that we found the expected symbols
        let functions = result.symbols.get("functions").unwrap();
        assert!(functions.iter().any(|f| f.name == "main"));
        assert!(functions.iter().any(|f| f.name == "new"));

        let types = result.symbols.get("types").unwrap();
        assert!(types.iter().any(|t| t.name == "Person"));
    }

    #[tokio::test]
    async fn test_python_pipeline() {
        let python_code = r#"
import os
from typing import List

class Calculator:
    """A simple calculator class."""
    
    def __init__(self):
        self.history = []
    
    def add(self, a: int, b: int) -> int:
        """Add two numbers."""
        result = a + b
        self.history.append(f"{a} + {b} = {result}")
        return result

def test_calculator():
    calc = Calculator()
    assert calc.add(2, 3) == 5

@property
def version():
    return "1.0.0"
        "#;

        let temp_file = NamedTempFile::with_suffix(".py").unwrap();
        std::fs::write(temp_file.path(), python_code).unwrap();

        let mut pipeline = IndexingPipeline::new(Language::Python).unwrap();
        let result = pipeline.process_file(temp_file.path()).await.unwrap();

        assert_eq!(result.language, Language::Python);
        assert!(result.symbols_found > 0);

        // Check imports
        if let Some(imports) = result.symbols.get("imports") {
            assert!(imports.iter().any(|i| i.name.contains("os")));
        }

        // Check functions
        if let Some(functions) = result.symbols.get("functions") {
            assert!(functions.iter().any(|f| f.name == "add"));
            assert!(functions.iter().any(|f| f.name == "test_calculator"));
        }

        // Check types
        if let Some(types) = result.symbols.get("types") {
            assert!(types.iter().any(|t| t.name == "Calculator"));
        }
    }

    #[test]
    fn test_pipeline_config() {
        let config = PipelineConfig::for_language(Language::TypeScript);
        assert_eq!(config.language, Language::TypeScript);
        assert!(config.features.extract_functions);
        assert!(config.file_extensions.contains(&"ts".to_string()));
        assert!(config
            .features
            .is_language_feature_enabled("extract_interfaces"));
    }

    #[test]
    fn test_indexing_features() {
        let mut features = IndexingFeatures::default();
        assert!(features.extract_functions);
        assert!(features.extract_imports);

        features.set_language_feature("custom_feature".to_string(), true);
        assert!(features.is_language_feature_enabled("custom_feature"));
        assert!(!features.is_language_feature_enabled("nonexistent_feature"));

        let minimal = IndexingFeatures::minimal();
        assert!(minimal.extract_functions);
        assert!(!minimal.extract_variables);

        let comprehensive = IndexingFeatures::comprehensive();
        assert!(comprehensive.extract_imports);
        assert!(comprehensive.extract_security);
    }

    #[test]
    fn test_pattern_matching() {
        // Test the pattern matching function directly
        assert!(PipelineConfig::matches_pattern("test_module.rs", "*test*"));
        assert!(PipelineConfig::matches_pattern("module_test.rs", "*test*"));
        assert!(!PipelineConfig::matches_pattern("module.rs", "*test*"));

        // Test more specific patterns
        assert!(PipelineConfig::matches_pattern("test_module.rs", "test_*"));
        assert!(!PipelineConfig::matches_pattern("module_test.rs", "test_*"));
    }

    #[tokio::test]
    async fn test_file_filtering() {
        let config = PipelineConfig {
            language: Language::Rust,
            features: IndexingFeatures::default(),
            max_file_size: 1000,
            timeout_ms: 5000,
            file_extensions: vec!["rs".to_string()],
            exclude_patterns: vec!["test_*.rs".to_string()], // More specific pattern
            parser_config: HashMap::new(),
        };

        let pipeline = LanguagePipeline::with_config(config);

        // Should process .rs files
        assert!(pipeline.config.should_process_file(Path::new("main.rs")));

        // Should not process .py files
        assert!(!pipeline.config.should_process_file(Path::new("script.py")));

        // Should not process test files that match the pattern
        assert!(!pipeline
            .config
            .should_process_file(Path::new("test_module.rs")));

        // Should process files that don't match the pattern
        assert!(pipeline
            .config
            .should_process_file(Path::new("module_test.rs")));
    }
}
