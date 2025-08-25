use crate::language_detector::Language;
use crate::path_safety;
use crate::socket_path::normalize_executable;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub language: Language,
    pub command: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initialization_options: Option<Value>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default = "default_initialization_timeout")]
    pub initialization_timeout_secs: u64,
}

fn default_initialization_timeout() -> u64 {
    30
}

impl Default for LspServerConfig {
    fn default() -> Self {
        Self {
            language: Language::Unknown,
            command: String::new(),
            args: Vec::new(),
            initialization_options: None,
            root_markers: Vec::new(),
            initialization_timeout_secs: 30,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LspRegistry {
    servers: HashMap<Language, LspServerConfig>,
}

impl LspRegistry {
    pub fn new() -> Result<Self> {
        let mut registry = Self {
            servers: HashMap::new(),
        };

        // Register built-in language servers
        registry.register_builtin_servers()?;

        // Load user configurations if they exist
        if let Ok(config) = Self::load_user_config() {
            registry.merge_user_config(config);
        }

        Ok(registry)
    }

    fn register_builtin_servers(&mut self) -> Result<()> {
        // Rust
        self.register(LspServerConfig {
            language: Language::Rust,
            command: "rust-analyzer".to_string(),
            args: vec![],
            initialization_options: Some(serde_json::json!({
                "cargo": {
                    "buildScripts": { "enable": true }
                },
                "procMacro": { "enable": true },
                // Optimizations to prevent indexing from getting stuck
                "checkOnSave": {
                    "enable": false  // Disable cargo check on save to reduce load
                },
                "completion": {
                    "limit": 25  // Limit completion results
                },
                "workspace": {
                    "symbol": {
                        "search": {
                            "limit": 128,  // Limit symbol search results
                            "kind": "only_types"  // Focus on types for better performance
                        }
                    }
                }
            })),
            root_markers: vec!["Cargo.toml".to_string()],
            initialization_timeout_secs: 10, // Reduced from 300s to 10s
        });

        // TypeScript/JavaScript
        self.register(LspServerConfig {
            language: Language::TypeScript,
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            initialization_options: None,
            root_markers: vec!["package.json".to_string(), "tsconfig.json".to_string()],
            initialization_timeout_secs: 30,
        });

        self.register(LspServerConfig {
            language: Language::JavaScript,
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            initialization_options: None,
            root_markers: vec!["package.json".to_string(), "jsconfig.json".to_string()],
            initialization_timeout_secs: 30,
        });

        // Python
        self.register(LspServerConfig {
            language: Language::Python,
            command: "pylsp".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec![
                "setup.py".to_string(),
                "pyproject.toml".to_string(),
                "requirements.txt".to_string(),
            ],
            initialization_timeout_secs: 30,
        });

        // Go
        self.register(LspServerConfig {
            language: Language::Go,
            command: "gopls".to_string(),
            args: vec!["serve".to_string(), "-mode=stdio".to_string()],
            initialization_options: Some(serde_json::json!({
                // NOTE: Do not set directoryFilters here.
                // Misconfiguring filters can exclude the module root and cause
                // "no package metadata for file" in LSP.
                // MUST be true for gopls to find package metadata!
                // When false, causes "no package metadata" errors
                "expandWorkspaceToModule": true,
                // Only search workspace packages, not all dependencies
                "symbolScope": "workspace",
                // Disable deep completion which can be slow
                "deepCompletion": false,
                // Reduce analysis scope
                "staticcheck": false,
                "analyses": {
                    "fieldalignment": false,
                    "unusedparams": false
                }
            })),
            root_markers: vec!["go.mod".to_string(), "go.work".to_string()],
            initialization_timeout_secs: 60,
        });

        // Java
        self.register(LspServerConfig {
            language: Language::Java,
            command: "jdtls".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec![
                "pom.xml".to_string(),
                "build.gradle".to_string(),
                "build.gradle.kts".to_string(),
            ],
            initialization_timeout_secs: 45,
        });

        // C/C++
        self.register(LspServerConfig {
            language: Language::C,
            command: "clangd".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec![
                "compile_commands.json".to_string(),
                ".clangd".to_string(),
                "Makefile".to_string(),
            ],
            initialization_timeout_secs: 30,
        });

        self.register(LspServerConfig {
            language: Language::Cpp,
            command: "clangd".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec![
                "compile_commands.json".to_string(),
                ".clangd".to_string(),
                "CMakeLists.txt".to_string(),
                "Makefile".to_string(),
            ],
            initialization_timeout_secs: 30,
        });

        // C#
        self.register(LspServerConfig {
            language: Language::CSharp,
            command: "omnisharp".to_string(),
            args: vec![
                "--languageserver".to_string(),
                "--hostPID".to_string(),
                "0".to_string(),
            ],
            initialization_options: None,
            root_markers: vec!["*.sln".to_string(), "*.csproj".to_string()],
            initialization_timeout_secs: 45,
        });

        // Ruby
        self.register(LspServerConfig {
            language: Language::Ruby,
            command: "solargraph".to_string(),
            args: vec!["stdio".to_string()],
            initialization_options: None,
            root_markers: vec!["Gemfile".to_string(), ".solargraph.yml".to_string()],
            initialization_timeout_secs: 30,
        });

        // PHP
        self.register(LspServerConfig {
            language: Language::Php,
            command: "intelephense".to_string(),
            args: vec!["--stdio".to_string()],
            initialization_options: None,
            root_markers: vec!["composer.json".to_string(), ".git".to_string()],
            initialization_timeout_secs: 30,
        });

        // Swift
        self.register(LspServerConfig {
            language: Language::Swift,
            command: "sourcekit-lsp".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec!["Package.swift".to_string(), "*.xcodeproj".to_string()],
            initialization_timeout_secs: 30,
        });

        // Kotlin
        self.register(LspServerConfig {
            language: Language::Kotlin,
            command: "kotlin-language-server".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec![
                "build.gradle.kts".to_string(),
                "build.gradle".to_string(),
                "settings.gradle.kts".to_string(),
            ],
            initialization_timeout_secs: 45,
        });

        // Scala
        self.register(LspServerConfig {
            language: Language::Scala,
            command: "metals".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec!["build.sbt".to_string(), "build.sc".to_string()],
            initialization_timeout_secs: 60,
        });

        // Haskell
        self.register(LspServerConfig {
            language: Language::Haskell,
            command: "haskell-language-server-wrapper".to_string(),
            args: vec!["--lsp".to_string()],
            initialization_options: None,
            root_markers: vec![
                "stack.yaml".to_string(),
                "*.cabal".to_string(),
                "cabal.project".to_string(),
            ],
            initialization_timeout_secs: 45,
        });

        // Elixir
        self.register(LspServerConfig {
            language: Language::Elixir,
            command: "elixir-ls".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec!["mix.exs".to_string()],
            initialization_timeout_secs: 30,
        });

        // Clojure
        self.register(LspServerConfig {
            language: Language::Clojure,
            command: "clojure-lsp".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec!["project.clj".to_string(), "deps.edn".to_string()],
            initialization_timeout_secs: 45,
        });

        // Lua
        self.register(LspServerConfig {
            language: Language::Lua,
            command: "lua-language-server".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec![".luarc.json".to_string(), ".git".to_string()],
            initialization_timeout_secs: 30,
        });

        // Zig
        self.register(LspServerConfig {
            language: Language::Zig,
            command: "zls".to_string(),
            args: vec![],
            initialization_options: None,
            root_markers: vec!["build.zig".to_string()],
            initialization_timeout_secs: 30,
        });

        Ok(())
    }

    pub fn register(&mut self, config: LspServerConfig) {
        self.servers.insert(config.language, config);
    }

    pub fn get(&self, language: Language) -> Option<&LspServerConfig> {
        self.servers.get(&language)
    }

    pub fn get_mut(&mut self, language: Language) -> Option<&mut LspServerConfig> {
        self.servers.get_mut(&language)
    }

    pub fn find_project_root(&self, file_path: &Path, language: Language) -> Option<PathBuf> {
        let config = self.get(language)?;

        let mut current = file_path.parent()?;

        // Walk up the directory tree looking for root markers
        while current != current.parent().unwrap_or(current) {
            for marker in &config.root_markers {
                // Handle glob patterns (e.g., "*.sln")
                if marker.contains('*') {
                    if let Ok(entries) = path_safety::safe_read_dir(current) {
                        for entry in entries {
                            if let Some(name) = entry.file_name().to_str() {
                                if Self::matches_glob(name, marker) {
                                    return Some(current.to_path_buf());
                                }
                            }
                        }
                    }
                } else {
                    // Direct file/directory check using safe path operations
                    if path_safety::exists_no_follow(&current.join(marker)) {
                        return Some(current.to_path_buf());
                    }
                }
            }

            current = current.parent()?;
        }

        // If no root marker found, use the file's directory
        file_path.parent().map(|p| p.to_path_buf())
    }

    fn matches_glob(name: &str, pattern: &str) -> bool {
        // Simple glob matching for * wildcard
        if pattern == "*" {
            return true;
        }

        if let Some(prefix) = pattern.strip_suffix('*') {
            return name.starts_with(prefix);
        }

        if let Some(suffix) = pattern.strip_prefix('*') {
            return name.ends_with(suffix);
        }

        name == pattern
    }

    fn load_user_config() -> Result<HashMap<Language, LspServerConfig>> {
        let config_dir =
            dirs::config_dir().ok_or_else(|| anyhow!("Could not find config directory"))?;
        let config_path = config_dir.join("lsp-daemon").join("config.toml");

        if !path_safety::exists_no_follow(&config_path) {
            return Ok(HashMap::new());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: toml::Value = toml::from_str(&content)?;

        let mut servers = HashMap::new();

        if let Some(languages) = config.get("languages").and_then(|v| v.as_table()) {
            for (lang_str, value) in languages {
                if let Ok(config) =
                    serde_json::from_value::<LspServerConfig>(serde_json::to_value(value)?)
                {
                    // Parse language from string
                    let language = match lang_str.as_str() {
                        "rust" => Language::Rust,
                        "typescript" => Language::TypeScript,
                        "javascript" => Language::JavaScript,
                        "python" => Language::Python,
                        "go" => Language::Go,
                        "java" => Language::Java,
                        "c" => Language::C,
                        "cpp" => Language::Cpp,
                        "csharp" => Language::CSharp,
                        "ruby" => Language::Ruby,
                        "php" => Language::Php,
                        "swift" => Language::Swift,
                        "kotlin" => Language::Kotlin,
                        "scala" => Language::Scala,
                        "haskell" => Language::Haskell,
                        "elixir" => Language::Elixir,
                        "clojure" => Language::Clojure,
                        "lua" => Language::Lua,
                        "zig" => Language::Zig,
                        _ => continue,
                    };

                    servers.insert(language, config);
                }
            }
        }

        Ok(servers)
    }

    fn merge_user_config(&mut self, user_configs: HashMap<Language, LspServerConfig>) {
        for (language, config) in user_configs {
            self.servers.insert(language, config);
        }
    }

    pub fn is_lsp_available(&self, language: Language) -> bool {
        if let Some(config) = self.get(language) {
            // Check if the command exists in PATH (with platform-specific executable extension)
            let command = normalize_executable(&config.command);
            match which::which(&command) {
                Ok(path) => {
                    tracing::trace!("LSP server for {:?} found at: {:?}", language, path);
                    true
                }
                Err(e) => {
                    tracing::trace!("LSP server for {:?} not available: {}", language, e);
                    false
                }
            }
        } else {
            tracing::trace!("No LSP configuration found for {:?}", language);
            false
        }
    }

    pub fn list_available_servers(&self) -> Vec<(Language, bool)> {
        let mut servers = Vec::new();
        for (language, config) in &self.servers {
            let command = normalize_executable(&config.command);
            let available = match which::which(&command) {
                Ok(_) => {
                    tracing::trace!("LSP server for {:?} is available", language);
                    true
                }
                Err(e) => {
                    tracing::trace!("LSP server for {:?} is not available: {}", language, e);
                    false
                }
            };
            servers.push((*language, available));
        }
        servers.sort_by_key(|(lang, _)| lang.as_str().to_string());
        servers
    }
}
