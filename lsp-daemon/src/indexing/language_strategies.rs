//! Language-specific indexing strategies
//!
//! This module defines strategies for optimizing indexing based on language-specific patterns,
//! conventions, and ecosystem characteristics. Each language has unique constructs and idioms
//! that require specialized handling for effective semantic indexing.

use crate::language_detector::Language;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info};

/// Priority levels for indexing operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum IndexingPriority {
    /// Critical symbols that are essential for understanding the codebase
    Critical = 4,
    /// High priority symbols that are frequently referenced
    High = 3,
    /// Medium priority symbols with moderate importance
    #[default]
    Medium = 2,
    /// Low priority symbols that are less frequently needed
    Low = 1,
    /// Minimal priority for rarely accessed symbols
    Minimal = 0,
}

/// Strategy for determining file importance in a workspace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileImportanceStrategy {
    /// Base priority for all files of this type
    pub base_priority: IndexingPriority,

    /// File patterns that should be prioritized higher
    pub high_priority_patterns: Vec<String>,

    /// File patterns that should be deprioritized
    pub low_priority_patterns: Vec<String>,

    /// Whether test files should be included in indexing
    pub include_tests: bool,

    /// Maximum file size to consider for indexing (bytes)
    pub max_file_size: u64,

    /// File extensions that should be processed
    pub target_extensions: Vec<String>,
}

impl Default for FileImportanceStrategy {
    fn default() -> Self {
        Self {
            base_priority: IndexingPriority::Medium,
            high_priority_patterns: vec![],
            low_priority_patterns: vec!["*test*".to_string(), "*spec*".to_string()],
            include_tests: false,
            max_file_size: 10 * 1024 * 1024, // 10MB
            target_extensions: vec![],
        }
    }
}

/// Strategy for symbol priority calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolPriorityStrategy {
    /// Base priorities for different symbol types
    pub symbol_type_priorities: HashMap<String, IndexingPriority>,

    /// Visibility modifiers and their priority impact
    pub visibility_priorities: HashMap<String, IndexingPriority>,

    /// Whether to prioritize symbols with documentation
    pub prioritize_documented: bool,

    /// Whether to prioritize exported/public symbols
    pub prioritize_exports: bool,

    /// Patterns for identifying important symbols
    pub important_symbol_patterns: Vec<String>,
}

impl Default for SymbolPriorityStrategy {
    fn default() -> Self {
        let mut symbol_type_priorities = HashMap::new();
        symbol_type_priorities.insert("function".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("class".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("interface".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("type".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("variable".to_string(), IndexingPriority::Low);

        let mut visibility_priorities = HashMap::new();
        visibility_priorities.insert("public".to_string(), IndexingPriority::High);
        visibility_priorities.insert("export".to_string(), IndexingPriority::High);
        visibility_priorities.insert("private".to_string(), IndexingPriority::Low);

        Self {
            symbol_type_priorities,
            visibility_priorities,
            prioritize_documented: true,
            prioritize_exports: true,
            important_symbol_patterns: vec!["main".to_string(), "init".to_string()],
        }
    }
}

/// LSP operations to perform for different symbol types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspOperationStrategy {
    /// Symbol types that should have call hierarchy extracted
    pub call_hierarchy_types: Vec<String>,

    /// Symbol types that should have references indexed
    pub reference_types: Vec<String>,

    /// Symbol types that should have definitions cached
    pub definition_types: Vec<String>,

    /// Symbol types that should have hover information cached
    pub hover_types: Vec<String>,

    /// Whether to build dependency graphs for this language
    pub build_dependency_graph: bool,

    /// Maximum depth for call graph traversal
    pub max_call_depth: u32,
}

impl Default for LspOperationStrategy {
    fn default() -> Self {
        Self {
            call_hierarchy_types: vec![
                "function".to_string(),
                "method".to_string(),
                "constructor".to_string(),
            ],
            reference_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
            ],
            definition_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
                "variable".to_string(),
            ],
            hover_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
            ],
            build_dependency_graph: true,
            max_call_depth: 5,
        }
    }
}

/// Comprehensive language-specific indexing strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageIndexingStrategy {
    /// Language this strategy applies to
    pub language: Language,

    /// Strategy for determining file importance
    pub file_strategy: FileImportanceStrategy,

    /// Strategy for symbol priority calculation
    pub symbol_strategy: SymbolPriorityStrategy,

    /// Strategy for LSP operations
    pub lsp_strategy: LspOperationStrategy,

    /// Language-specific metadata
    pub metadata: HashMap<String, serde_json::Value>,
}

impl LanguageIndexingStrategy {
    /// Calculate priority for a file based on its path and characteristics
    pub fn calculate_file_priority(&self, file_path: &Path) -> IndexingPriority {
        let path_str = file_path.to_string_lossy().to_lowercase();

        // Check high priority patterns first
        for pattern in &self.file_strategy.high_priority_patterns {
            if Self::matches_glob_pattern(&path_str, pattern) {
                debug!(
                    "File {:?} matches high priority pattern: {}",
                    file_path, pattern
                );
                return IndexingPriority::High;
            }
        }

        // Check low priority patterns
        for pattern in &self.file_strategy.low_priority_patterns {
            if Self::matches_glob_pattern(&path_str, pattern) {
                debug!(
                    "File {:?} matches low priority pattern: {}",
                    file_path, pattern
                );
                return IndexingPriority::Low;
            }
        }

        // Check if it's a test file and tests are excluded
        if !self.file_strategy.include_tests && self.is_test_file(file_path) {
            return IndexingPriority::Minimal;
        }

        self.file_strategy.base_priority
    }

    /// Calculate priority for a symbol based on its type and characteristics
    pub fn calculate_symbol_priority(
        &self,
        symbol_type: &str,
        visibility: Option<&str>,
        has_documentation: bool,
        is_exported: bool,
    ) -> IndexingPriority {
        // Start with base priority for symbol type
        let mut priority = self
            .symbol_strategy
            .symbol_type_priorities
            .get(symbol_type)
            .copied()
            .unwrap_or(IndexingPriority::Medium);

        // Adjust for visibility
        if let Some(vis) = visibility {
            if let Some(&vis_priority) = self.symbol_strategy.visibility_priorities.get(vis) {
                priority = priority.max(vis_priority);
            }
        }

        // Boost priority for documented symbols
        if has_documentation && self.symbol_strategy.prioritize_documented {
            priority = match priority {
                IndexingPriority::Low => IndexingPriority::Medium,
                IndexingPriority::Medium => IndexingPriority::High,
                other => other,
            };
        }

        // Boost priority for exported symbols
        if is_exported && self.symbol_strategy.prioritize_exports {
            priority = priority.max(IndexingPriority::High);
        }

        priority
    }

    /// Check if file should be processed based on extension
    pub fn should_process_file(&self, file_path: &Path) -> bool {
        if let Some(ext) = file_path.extension().and_then(|e| e.to_str()) {
            self.file_strategy.target_extensions.is_empty()
                || self
                    .file_strategy
                    .target_extensions
                    .contains(&format!(".{ext}"))
        } else {
            false
        }
    }

    /// Check if a symbol type should have call hierarchy extracted
    pub fn should_extract_call_hierarchy(&self, symbol_type: &str) -> bool {
        self.lsp_strategy
            .call_hierarchy_types
            .contains(&symbol_type.to_string())
    }

    /// Check if a symbol type should have references indexed
    pub fn should_index_references(&self, symbol_type: &str) -> bool {
        self.lsp_strategy
            .reference_types
            .contains(&symbol_type.to_string())
    }

    /// Check if a symbol type should have definitions cached
    pub fn should_cache_definitions(&self, symbol_type: &str) -> bool {
        self.lsp_strategy
            .definition_types
            .contains(&symbol_type.to_string())
    }

    /// Check if a symbol type should have hover information cached
    pub fn should_cache_hover(&self, symbol_type: &str) -> bool {
        self.lsp_strategy
            .hover_types
            .contains(&symbol_type.to_string())
    }

    /// Determine if a file is a test file based on language-specific patterns
    pub fn is_test_file(&self, file_path: &Path) -> bool {
        let path_str = file_path.to_string_lossy().to_lowercase();
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();

        match self.language {
            Language::Rust => {
                path_str.contains("/tests/")
                    || file_name.starts_with("test_")
                    || file_name.ends_with("_test.rs")
                    || file_name == "lib.rs" && path_str.contains("/tests/")
            }
            Language::Go => file_name.ends_with("_test.go"),
            Language::Python => {
                path_str.contains("/test")
                    || file_name.starts_with("test_")
                    || file_name.ends_with("_test.py")
                    || path_str.contains("/__test")
            }
            Language::JavaScript | Language::TypeScript => {
                path_str.contains("/test")
                    || path_str.contains("/__test")
                    || path_str.contains("/spec")
                    || file_name.ends_with(".test.js")
                    || file_name.ends_with(".test.ts")
                    || file_name.ends_with(".spec.js")
                    || file_name.ends_with(".spec.ts")
            }
            Language::Java => {
                path_str.contains("/test/")
                    || file_name.ends_with("test.java")
                    || file_name.starts_with("test")
            }
            _ => {
                // Generic test detection
                path_str.contains("/test") || file_name.contains("test")
            }
        }
    }

    /// Simple glob pattern matching
    fn matches_glob_pattern(text: &str, pattern: &str) -> bool {
        // Handle patterns with wildcards
        if pattern.contains('*') {
            // Special case for patterns like "*text*" - just check if text contains the middle part
            if pattern.starts_with('*') && pattern.ends_with('*') {
                let middle = &pattern[1..pattern.len() - 1];
                if middle.is_empty() {
                    return true; // "*" matches everything
                }
                return text.contains(middle);
            }

            // Split on * and check each part matches in order
            let parts: Vec<&str> = pattern.split('*').filter(|p| !p.is_empty()).collect();

            if parts.is_empty() {
                return true; // "*" matches everything
            }

            let mut search_pos = 0;

            for (i, part) in parts.iter().enumerate() {
                if i == 0 && !pattern.starts_with('*') {
                    // First part and pattern doesn't start with *, so must match at beginning
                    if !text[search_pos..].starts_with(part) {
                        return false;
                    }
                    search_pos += part.len();
                } else if i == parts.len() - 1 && !pattern.ends_with('*') {
                    // Last part and pattern doesn't end with *, so must match at the end
                    return text[search_pos..].ends_with(part);
                } else {
                    // Find the part in the remaining text
                    if let Some(pos) = text[search_pos..].find(part) {
                        search_pos += pos + part.len();
                    } else {
                        return false;
                    }
                }
            }

            true
        } else {
            text.contains(pattern)
        }
    }
}

/// Factory for creating language-specific indexing strategies
pub struct LanguageStrategyFactory;

impl LanguageStrategyFactory {
    /// Create a strategy for the specified language
    pub fn create_strategy(language: Language) -> LanguageIndexingStrategy {
        match language {
            Language::Rust => Self::create_rust_strategy(),
            Language::Python => Self::create_python_strategy(),
            Language::Go => Self::create_go_strategy(),
            Language::TypeScript => Self::create_typescript_strategy(),
            Language::JavaScript => Self::create_javascript_strategy(),
            Language::Java => Self::create_java_strategy(),
            Language::C => Self::create_c_strategy(),
            Language::Cpp => Self::create_cpp_strategy(),
            _ => Self::create_default_strategy(language),
        }
    }

    /// Create Rust-specific indexing strategy
    fn create_rust_strategy() -> LanguageIndexingStrategy {
        let file_strategy = FileImportanceStrategy {
            high_priority_patterns: vec![
                "*lib.rs".to_string(),
                "*main.rs".to_string(),
                "*mod.rs".to_string(),
                "*/src/*".to_string(),
                "*cargo.toml".to_string(),
            ],
            low_priority_patterns: vec![
                "*/tests/*".to_string(),
                "*_test.rs".to_string(),
                "*/target/*".to_string(),
                "*/examples/*".to_string(),
            ],
            target_extensions: vec![".rs".to_string()],
            include_tests: false,
            ..Default::default()
        };

        let mut symbol_strategy = SymbolPriorityStrategy::default();
        symbol_strategy
            .symbol_type_priorities
            .insert("trait".to_string(), IndexingPriority::Critical);
        symbol_strategy
            .symbol_type_priorities
            .insert("impl".to_string(), IndexingPriority::High);
        symbol_strategy
            .symbol_type_priorities
            .insert("macro".to_string(), IndexingPriority::High);
        symbol_strategy
            .symbol_type_priorities
            .insert("struct".to_string(), IndexingPriority::High);
        symbol_strategy
            .symbol_type_priorities
            .insert("enum".to_string(), IndexingPriority::High);
        symbol_strategy.important_symbol_patterns = vec![
            "main".to_string(),
            "new".to_string(),
            "default".to_string(),
            "from".to_string(),
            "into".to_string(),
        ];

        let mut lsp_strategy = LspOperationStrategy::default();
        lsp_strategy.call_hierarchy_types.extend([
            "trait".to_string(),
            "impl".to_string(),
            "macro".to_string(),
        ]);
        lsp_strategy.reference_types.extend([
            "trait".to_string(),
            "struct".to_string(),
            "enum".to_string(),
            "macro".to_string(),
        ]);

        let mut metadata = HashMap::new();
        metadata.insert("ecosystem".to_string(), serde_json::json!("cargo"));
        metadata.insert("build_system".to_string(), serde_json::json!("cargo"));
        metadata.insert("package_manager".to_string(), serde_json::json!("cargo"));

        LanguageIndexingStrategy {
            language: Language::Rust,
            file_strategy,
            symbol_strategy,
            lsp_strategy,
            metadata,
        }
    }

    /// Create Python-specific indexing strategy  
    fn create_python_strategy() -> LanguageIndexingStrategy {
        let file_strategy = FileImportanceStrategy {
            high_priority_patterns: vec![
                "*__init__.py".to_string(),
                "*setup.py".to_string(),
                "*pyproject.toml".to_string(),
                "*main.py".to_string(),
                "*app.py".to_string(),
                "*manage.py".to_string(),
            ],
            low_priority_patterns: vec![
                "*/test_*".to_string(),
                "*_test.py".to_string(),
                "*/tests/*".to_string(),
                "*/__pycache__/*".to_string(),
                "*/venv/*".to_string(),
                "*/env/*".to_string(),
            ],
            target_extensions: vec![".py".to_string(), ".pyi".to_string()],
            ..Default::default()
        };

        let mut symbol_type_priorities = HashMap::new();
        symbol_type_priorities.insert("function".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("class".to_string(), IndexingPriority::Critical);
        symbol_type_priorities.insert("interface".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("type".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("variable".to_string(), IndexingPriority::Low);
        symbol_type_priorities.insert("decorator".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("property".to_string(), IndexingPriority::Medium);

        let symbol_strategy = SymbolPriorityStrategy {
            symbol_type_priorities,
            important_symbol_patterns: vec![
                "__init__".to_string(),
                "__new__".to_string(),
                "__call__".to_string(),
                "main".to_string(),
            ],
            ..Default::default()
        };

        let lsp_strategy = LspOperationStrategy {
            call_hierarchy_types: vec![
                "function".to_string(),
                "method".to_string(),
                "constructor".to_string(),
                "class".to_string(),
                "decorator".to_string(),
            ],
            reference_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
                "import".to_string(),
                "decorator".to_string(),
            ],
            ..Default::default()
        };

        let mut metadata = HashMap::new();
        metadata.insert("ecosystem".to_string(), serde_json::json!("pip"));
        metadata.insert(
            "package_managers".to_string(),
            serde_json::json!(["pip", "conda", "poetry"]),
        );
        metadata.insert(
            "virtual_envs".to_string(),
            serde_json::json!(["venv", "virtualenv", "conda"]),
        );

        LanguageIndexingStrategy {
            language: Language::Python,
            file_strategy,
            symbol_strategy,
            lsp_strategy,
            metadata,
        }
    }

    /// Create Go-specific indexing strategy
    fn create_go_strategy() -> LanguageIndexingStrategy {
        let file_strategy = FileImportanceStrategy {
            high_priority_patterns: vec![
                "*main.go".to_string(),
                "*go.mod".to_string(),
                "*go.sum".to_string(),
                "*/cmd/*".to_string(),
                "*/internal/*".to_string(),
                "*/pkg/*".to_string(),
            ],
            low_priority_patterns: vec![
                "*_test.go".to_string(),
                "*/vendor/*".to_string(),
                "*/testdata/*".to_string(),
            ],
            target_extensions: vec![".go".to_string()],
            ..Default::default()
        };

        let mut symbol_type_priorities = HashMap::new();
        symbol_type_priorities.insert("function".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("class".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("interface".to_string(), IndexingPriority::Critical);
        symbol_type_priorities.insert("type".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("variable".to_string(), IndexingPriority::Low);
        symbol_type_priorities.insert("package".to_string(), IndexingPriority::Critical);
        symbol_type_priorities.insert("struct".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("receiver".to_string(), IndexingPriority::High);

        let symbol_strategy = SymbolPriorityStrategy {
            symbol_type_priorities,
            important_symbol_patterns: vec![
                "main".to_string(),
                "New".to_string(),
                "init".to_string(),
                "String".to_string(),
                "Error".to_string(),
            ],
            ..Default::default()
        };

        let lsp_strategy = LspOperationStrategy {
            call_hierarchy_types: vec![
                "function".to_string(),
                "method".to_string(),
                "constructor".to_string(),
                "interface".to_string(),
                "struct".to_string(),
                "receiver".to_string(),
            ],
            reference_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
                "package".to_string(),
                "import".to_string(),
            ],
            ..Default::default()
        };

        let mut metadata = HashMap::new();
        metadata.insert("ecosystem".to_string(), serde_json::json!("go"));
        metadata.insert("build_system".to_string(), serde_json::json!("go"));
        metadata.insert("package_manager".to_string(), serde_json::json!("go"));

        LanguageIndexingStrategy {
            language: Language::Go,
            file_strategy,
            symbol_strategy,
            lsp_strategy,
            metadata,
        }
    }

    /// Create TypeScript-specific indexing strategy
    fn create_typescript_strategy() -> LanguageIndexingStrategy {
        let file_strategy = FileImportanceStrategy {
            high_priority_patterns: vec![
                "*index.ts".to_string(),
                "*index.tsx".to_string(),
                "*main.ts".to_string(),
                "*app.ts".to_string(),
                "*app.tsx".to_string(),
                "*package.json".to_string(),
                "*tsconfig.json".to_string(),
                "*/src/*".to_string(),
                "*/types/*".to_string(),
            ],
            low_priority_patterns: vec![
                "*.test.ts".to_string(),
                "*.test.tsx".to_string(),
                "*.spec.ts".to_string(),
                "*.spec.tsx".to_string(),
                "*/tests/*".to_string(),
                "*/node_modules/*".to_string(),
                "*/dist/*".to_string(),
                "*/build/*".to_string(),
            ],
            target_extensions: vec![".ts".to_string(), ".tsx".to_string()],
            ..Default::default()
        };

        let mut symbol_type_priorities = HashMap::new();
        symbol_type_priorities.insert("function".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("class".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("interface".to_string(), IndexingPriority::Critical);
        symbol_type_priorities.insert("type".to_string(), IndexingPriority::Critical);
        symbol_type_priorities.insert("variable".to_string(), IndexingPriority::Low);
        symbol_type_priorities.insert("export".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("decorator".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("component".to_string(), IndexingPriority::High);

        let symbol_strategy = SymbolPriorityStrategy {
            symbol_type_priorities,
            important_symbol_patterns: vec![
                "default".to_string(),
                "main".to_string(),
                "App".to_string(),
                "Component".to_string(),
            ],
            ..Default::default()
        };

        let lsp_strategy = LspOperationStrategy {
            call_hierarchy_types: vec![
                "function".to_string(),
                "method".to_string(),
                "constructor".to_string(),
                "interface".to_string(),
                "type".to_string(),
                "component".to_string(),
                "decorator".to_string(),
            ],
            reference_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
                "export".to_string(),
                "import".to_string(),
            ],
            ..Default::default()
        };

        let mut metadata = HashMap::new();
        metadata.insert("ecosystem".to_string(), serde_json::json!("npm"));
        metadata.insert(
            "build_systems".to_string(),
            serde_json::json!(["tsc", "webpack", "vite", "rollup"]),
        );
        metadata.insert("package_manager".to_string(), serde_json::json!("npm"));

        LanguageIndexingStrategy {
            language: Language::TypeScript,
            file_strategy,
            symbol_strategy,
            lsp_strategy,
            metadata,
        }
    }

    /// Create JavaScript-specific indexing strategy
    fn create_javascript_strategy() -> LanguageIndexingStrategy {
        let file_strategy = FileImportanceStrategy {
            high_priority_patterns: vec![
                "*index.js".to_string(),
                "*index.jsx".to_string(),
                "*main.js".to_string(),
                "*app.js".to_string(),
                "*app.jsx".to_string(),
                "*package.json".to_string(),
                "*/src/*".to_string(),
            ],
            low_priority_patterns: vec![
                "*.test.js".to_string(),
                "*.test.jsx".to_string(),
                "*.spec.js".to_string(),
                "*.spec.jsx".to_string(),
                "*/tests/*".to_string(),
                "*/node_modules/*".to_string(),
                "*/dist/*".to_string(),
                "*/build/*".to_string(),
            ],
            target_extensions: vec![".js".to_string(), ".jsx".to_string(), ".mjs".to_string()],
            ..Default::default()
        };

        let mut symbol_type_priorities = HashMap::new();
        symbol_type_priorities.insert("function".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("class".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("interface".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("type".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("variable".to_string(), IndexingPriority::Low);
        symbol_type_priorities.insert("export".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("prototype".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("component".to_string(), IndexingPriority::High);

        let symbol_strategy = SymbolPriorityStrategy {
            symbol_type_priorities,
            important_symbol_patterns: vec![
                "default".to_string(),
                "main".to_string(),
                "App".to_string(),
                "Component".to_string(),
                "module".to_string(),
            ],
            ..Default::default()
        };

        let lsp_strategy = LspOperationStrategy {
            call_hierarchy_types: vec![
                "function".to_string(),
                "method".to_string(),
                "constructor".to_string(),
                "prototype".to_string(),
                "component".to_string(),
            ],
            reference_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
                "export".to_string(),
                "import".to_string(),
                "require".to_string(),
            ],
            ..Default::default()
        };

        let mut metadata = HashMap::new();
        metadata.insert("ecosystem".to_string(), serde_json::json!("npm"));
        metadata.insert(
            "build_systems".to_string(),
            serde_json::json!(["webpack", "vite", "rollup", "parcel"]),
        );
        metadata.insert("package_manager".to_string(), serde_json::json!("npm"));

        LanguageIndexingStrategy {
            language: Language::JavaScript,
            file_strategy,
            symbol_strategy,
            lsp_strategy,
            metadata,
        }
    }

    /// Create Java-specific indexing strategy
    fn create_java_strategy() -> LanguageIndexingStrategy {
        let file_strategy = FileImportanceStrategy {
            high_priority_patterns: vec![
                "*Application.java".to_string(),
                "*Main.java".to_string(),
                "*src/main*".to_string(), // Fixed pattern
                "*pom.xml".to_string(),
                "*build.gradle".to_string(),
            ],
            low_priority_patterns: vec![
                "*src/test*".to_string(), // Fixed pattern
                "*Test.java".to_string(),
                "*Tests.java".to_string(),
                "*target*".to_string(), // Fixed pattern
                "*build*".to_string(),  // Fixed pattern
            ],
            target_extensions: vec![".java".to_string()],
            ..Default::default()
        };

        let mut symbol_type_priorities = HashMap::new();
        symbol_type_priorities.insert("function".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("class".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("interface".to_string(), IndexingPriority::Critical);
        symbol_type_priorities.insert("type".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("variable".to_string(), IndexingPriority::Low);
        symbol_type_priorities.insert("annotation".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("abstract".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("enum".to_string(), IndexingPriority::Medium);

        let symbol_strategy = SymbolPriorityStrategy {
            symbol_type_priorities,
            important_symbol_patterns: vec![
                "main".to_string(),
                "Application".to_string(),
                "Service".to_string(),
                "Controller".to_string(),
                "Repository".to_string(),
            ],
            ..Default::default()
        };

        let lsp_strategy = LspOperationStrategy {
            call_hierarchy_types: vec![
                "function".to_string(),
                "method".to_string(),
                "constructor".to_string(),
                "interface".to_string(),
                "annotation".to_string(),
                "abstract".to_string(),
            ],
            reference_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
                "annotation".to_string(),
                "import".to_string(),
                "extends".to_string(),
                "implements".to_string(),
            ],
            ..Default::default()
        };

        let mut metadata = HashMap::new();
        metadata.insert("ecosystem".to_string(), serde_json::json!("maven"));
        metadata.insert(
            "build_systems".to_string(),
            serde_json::json!(["maven", "gradle", "ant"]),
        );
        metadata.insert(
            "package_managers".to_string(),
            serde_json::json!(["maven", "gradle"]),
        );

        LanguageIndexingStrategy {
            language: Language::Java,
            file_strategy,
            symbol_strategy,
            lsp_strategy,
            metadata,
        }
    }

    /// Create C-specific indexing strategy
    fn create_c_strategy() -> LanguageIndexingStrategy {
        let file_strategy = FileImportanceStrategy {
            high_priority_patterns: vec![
                "*main.c".to_string(),
                "*.h".to_string(),
                "*Makefile".to_string(),
                "*CMakeLists.txt".to_string(),
                "*/include/*".to_string(),
            ],
            low_priority_patterns: vec![
                "*/test/*".to_string(),
                "*test.c".to_string(),
                "*/build/*".to_string(),
            ],
            target_extensions: vec![".c".to_string(), ".h".to_string()],
            ..Default::default()
        };

        let mut symbol_type_priorities = HashMap::new();
        symbol_type_priorities.insert("function".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("class".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("interface".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("type".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("variable".to_string(), IndexingPriority::Low);
        symbol_type_priorities.insert("preprocessor".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("struct".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("union".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("typedef".to_string(), IndexingPriority::High);

        let symbol_strategy = SymbolPriorityStrategy {
            symbol_type_priorities,
            important_symbol_patterns: vec![
                "main".to_string(),
                "init".to_string(),
                "cleanup".to_string(),
            ],
            ..Default::default()
        };

        let lsp_strategy = LspOperationStrategy {
            call_hierarchy_types: vec![
                "function".to_string(),
                "method".to_string(),
                "constructor".to_string(),
                "struct".to_string(),
                "typedef".to_string(),
            ],
            ..Default::default()
        };

        let mut metadata = HashMap::new();
        metadata.insert("ecosystem".to_string(), serde_json::json!("system"));
        metadata.insert(
            "build_systems".to_string(),
            serde_json::json!(["make", "cmake", "autotools"]),
        );

        LanguageIndexingStrategy {
            language: Language::C,
            file_strategy,
            symbol_strategy,
            lsp_strategy,
            metadata,
        }
    }

    /// Create C++-specific indexing strategy
    fn create_cpp_strategy() -> LanguageIndexingStrategy {
        let file_strategy = FileImportanceStrategy {
            high_priority_patterns: vec![
                "*main.cpp".to_string(),
                "*.hpp".to_string(),
                "*.h".to_string(),
                "*CMakeLists.txt".to_string(),
                "*/include/*".to_string(),
            ],
            low_priority_patterns: vec![
                "*/test/*".to_string(),
                "*test.cpp".to_string(),
                "*/build/*".to_string(),
            ],
            target_extensions: vec![
                ".cpp".to_string(),
                ".cc".to_string(),
                ".cxx".to_string(),
                ".hpp".to_string(),
                ".hxx".to_string(),
                ".h".to_string(),
            ],
            ..Default::default()
        };

        let mut symbol_type_priorities = HashMap::new();
        symbol_type_priorities.insert("function".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("class".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("interface".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("type".to_string(), IndexingPriority::Medium);
        symbol_type_priorities.insert("variable".to_string(), IndexingPriority::Low);
        symbol_type_priorities.insert("template".to_string(), IndexingPriority::Critical);
        symbol_type_priorities.insert("namespace".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("struct".to_string(), IndexingPriority::High);
        symbol_type_priorities.insert("union".to_string(), IndexingPriority::Medium);

        let symbol_strategy = SymbolPriorityStrategy {
            symbol_type_priorities,
            important_symbol_patterns: vec![
                "main".to_string(),
                "std".to_string(),
                "template".to_string(),
            ],
            ..Default::default()
        };

        let lsp_strategy = LspOperationStrategy {
            call_hierarchy_types: vec![
                "function".to_string(),
                "method".to_string(),
                "constructor".to_string(),
                "template".to_string(),
                "namespace".to_string(),
                "struct".to_string(),
            ],
            reference_types: vec![
                "function".to_string(),
                "method".to_string(),
                "class".to_string(),
                "interface".to_string(),
                "type".to_string(),
                "template".to_string(),
                "namespace".to_string(),
                "using".to_string(),
            ],
            ..Default::default()
        };

        let mut metadata = HashMap::new();
        metadata.insert("ecosystem".to_string(), serde_json::json!("system"));
        metadata.insert(
            "build_systems".to_string(),
            serde_json::json!(["cmake", "make", "autotools", "bazel"]),
        );

        LanguageIndexingStrategy {
            language: Language::Cpp,
            file_strategy,
            symbol_strategy,
            lsp_strategy,
            metadata,
        }
    }

    /// Create default strategy for unknown languages
    fn create_default_strategy(language: Language) -> LanguageIndexingStrategy {
        info!(
            "Creating default indexing strategy for language: {:?}",
            language
        );

        // For unknown languages, use low priority since we don't know how to process them well
        let file_strategy = FileImportanceStrategy {
            base_priority: IndexingPriority::Low,
            ..Default::default()
        };

        LanguageIndexingStrategy {
            language,
            file_strategy,
            symbol_strategy: SymbolPriorityStrategy::default(),
            lsp_strategy: LspOperationStrategy::default(),
            metadata: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_rust_strategy() {
        let strategy = LanguageStrategyFactory::create_strategy(Language::Rust);
        assert_eq!(strategy.language, Language::Rust);

        // Test file priority calculation
        let lib_path = PathBuf::from("src/lib.rs");
        assert_eq!(
            strategy.calculate_file_priority(&lib_path),
            IndexingPriority::High
        );

        let test_path = PathBuf::from("tests/test_module.rs");
        assert_eq!(
            strategy.calculate_file_priority(&test_path),
            IndexingPriority::Minimal
        );

        // Test symbol priority calculation
        let trait_priority =
            strategy.calculate_symbol_priority("trait", Some("public"), true, true);
        assert_eq!(trait_priority, IndexingPriority::Critical);

        // Test LSP operations
        assert!(strategy.should_extract_call_hierarchy("function"));
        assert!(strategy.should_extract_call_hierarchy("trait"));
        assert!(!strategy.should_extract_call_hierarchy("variable"));
    }

    #[test]
    fn test_python_strategy() {
        let strategy = LanguageStrategyFactory::create_strategy(Language::Python);
        assert_eq!(strategy.language, Language::Python);

        // Test file priority calculation
        let init_path = PathBuf::from("package/__init__.py");
        assert_eq!(
            strategy.calculate_file_priority(&init_path),
            IndexingPriority::High
        );

        let test_path = PathBuf::from("test_module.py");
        assert_eq!(
            strategy.calculate_file_priority(&test_path),
            IndexingPriority::Minimal
        );

        // Test symbol priority calculation
        let class_priority =
            strategy.calculate_symbol_priority("class", Some("public"), true, true);
        assert_eq!(class_priority, IndexingPriority::Critical);
    }

    #[test]
    fn test_go_strategy() {
        let strategy = LanguageStrategyFactory::create_strategy(Language::Go);
        assert_eq!(strategy.language, Language::Go);

        // Test file priority calculation
        let main_path = PathBuf::from("cmd/main.go");
        assert_eq!(
            strategy.calculate_file_priority(&main_path),
            IndexingPriority::High
        );

        let test_path = PathBuf::from("main_test.go");
        assert_eq!(
            strategy.calculate_file_priority(&test_path),
            IndexingPriority::Low
        );

        // Test symbol priority calculation
        let interface_priority =
            strategy.calculate_symbol_priority("interface", Some("public"), true, true);
        assert_eq!(interface_priority, IndexingPriority::Critical);
    }

    #[test]
    fn test_typescript_strategy() {
        let strategy = LanguageStrategyFactory::create_strategy(Language::TypeScript);
        assert_eq!(strategy.language, Language::TypeScript);

        // Test file priority calculation
        let index_path = PathBuf::from("src/index.ts");
        assert_eq!(
            strategy.calculate_file_priority(&index_path),
            IndexingPriority::High
        );

        let test_path = PathBuf::from("component.test.ts");
        assert_eq!(
            strategy.calculate_file_priority(&test_path),
            IndexingPriority::Low
        );

        // Test symbol priority calculation
        let interface_priority =
            strategy.calculate_symbol_priority("interface", Some("export"), true, true);
        assert_eq!(interface_priority, IndexingPriority::Critical);
    }

    #[test]
    fn test_java_strategy() {
        let strategy = LanguageStrategyFactory::create_strategy(Language::Java);
        assert_eq!(strategy.language, Language::Java);

        // Test file priority calculation
        let app_path = PathBuf::from("src/main/java/Application.java");
        assert_eq!(
            strategy.calculate_file_priority(&app_path),
            IndexingPriority::High
        );

        let test_path = PathBuf::from("src/test/java/ApplicationTest.java");
        assert_eq!(
            strategy.calculate_file_priority(&test_path),
            IndexingPriority::Low
        );

        // Test symbol priority calculation
        let interface_priority =
            strategy.calculate_symbol_priority("interface", Some("public"), true, true);
        assert_eq!(interface_priority, IndexingPriority::Critical);
    }

    #[test]
    fn test_glob_pattern_matching() {
        // Test various glob patterns
        assert!(LanguageIndexingStrategy::matches_glob_pattern(
            "test_module.rs",
            "*test*"
        ));
        assert!(LanguageIndexingStrategy::matches_glob_pattern(
            "module_test.rs",
            "*test*"
        ));
        assert!(!LanguageIndexingStrategy::matches_glob_pattern(
            "module.rs",
            "*test*"
        ));

        assert!(LanguageIndexingStrategy::matches_glob_pattern(
            "test_module.rs",
            "test_*"
        ));
        assert!(!LanguageIndexingStrategy::matches_glob_pattern(
            "module_test.rs",
            "test_*"
        ));

        assert!(LanguageIndexingStrategy::matches_glob_pattern(
            "module.rs",
            "*.rs"
        ));
        assert!(!LanguageIndexingStrategy::matches_glob_pattern(
            "module.py",
            "*.rs"
        ));
    }

    #[test]
    fn test_test_file_detection() {
        let rust_strategy = LanguageStrategyFactory::create_strategy(Language::Rust);
        assert!(rust_strategy.is_test_file(&PathBuf::from("tests/test_module.rs")));
        assert!(rust_strategy.is_test_file(&PathBuf::from("src/module_test.rs")));
        assert!(!rust_strategy.is_test_file(&PathBuf::from("src/module.rs")));

        let go_strategy = LanguageStrategyFactory::create_strategy(Language::Go);
        assert!(go_strategy.is_test_file(&PathBuf::from("main_test.go")));
        assert!(!go_strategy.is_test_file(&PathBuf::from("main.go")));

        let python_strategy = LanguageStrategyFactory::create_strategy(Language::Python);
        assert!(python_strategy.is_test_file(&PathBuf::from("test_module.py")));
        assert!(python_strategy.is_test_file(&PathBuf::from("tests/test_app.py")));
        assert!(!python_strategy.is_test_file(&PathBuf::from("app.py")));

        let ts_strategy = LanguageStrategyFactory::create_strategy(Language::TypeScript);
        assert!(ts_strategy.is_test_file(&PathBuf::from("component.test.ts")));
        assert!(ts_strategy.is_test_file(&PathBuf::from("component.spec.ts")));
        assert!(!ts_strategy.is_test_file(&PathBuf::from("component.ts")));

        let java_strategy = LanguageStrategyFactory::create_strategy(Language::Java);
        assert!(java_strategy.is_test_file(&PathBuf::from("src/test/java/AppTest.java")));
        assert!(java_strategy.is_test_file(&PathBuf::from("ApplicationTest.java")));
        assert!(!java_strategy.is_test_file(&PathBuf::from("Application.java")));
    }

    #[test]
    fn test_symbol_priority_calculation() {
        let strategy = LanguageStrategyFactory::create_strategy(Language::Rust);

        // Test base priorities
        assert_eq!(
            strategy.calculate_symbol_priority("function", None, false, false),
            IndexingPriority::High
        );

        // Test visibility boost
        assert_eq!(
            strategy.calculate_symbol_priority("function", Some("public"), false, false),
            IndexingPriority::High
        );

        // Test documentation boost
        assert_eq!(
            strategy.calculate_symbol_priority("variable", None, true, false),
            IndexingPriority::Medium
        );

        // Test export boost
        assert_eq!(
            strategy.calculate_symbol_priority("function", None, false, true),
            IndexingPriority::High
        );

        // Test combined boosts
        assert_eq!(
            strategy.calculate_symbol_priority("trait", Some("public"), true, true),
            IndexingPriority::Critical
        );
    }
}
