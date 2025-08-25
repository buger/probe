use crate::path_safety;
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    Php,
    Swift,
    Kotlin,
    Scala,
    Haskell,
    Elixir,
    Clojure,
    Lua,
    Zig,
    Unknown,
}

impl Language {
    pub fn as_str(&self) -> &str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::Java => "java",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Ruby => "ruby",
            Language::Php => "php",
            Language::Swift => "swift",
            Language::Kotlin => "kotlin",
            Language::Scala => "scala",
            Language::Haskell => "haskell",
            Language::Elixir => "elixir",
            Language::Clojure => "clojure",
            Language::Lua => "lua",
            Language::Zig => "zig",
            Language::Unknown => "unknown",
        }
    }

    /// Parse a language from string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "rust" => Some(Language::Rust),
            "typescript" => Some(Language::TypeScript),
            "javascript" => Some(Language::JavaScript),
            "python" => Some(Language::Python),
            "go" => Some(Language::Go),
            "java" => Some(Language::Java),
            "c" => Some(Language::C),
            "cpp" | "c++" => Some(Language::Cpp),
            "csharp" | "c#" => Some(Language::CSharp),
            "ruby" => Some(Language::Ruby),
            "php" => Some(Language::Php),
            "swift" => Some(Language::Swift),
            "kotlin" => Some(Language::Kotlin),
            "scala" => Some(Language::Scala),
            "haskell" => Some(Language::Haskell),
            "elixir" => Some(Language::Elixir),
            "clojure" => Some(Language::Clojure),
            "lua" => Some(Language::Lua),
            "zig" => Some(Language::Zig),
            "unknown" => Some(Language::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct LanguageDetector {
    extension_map: HashMap<String, Language>,
    shebang_patterns: Vec<(Regex, Language)>,
}

impl Default for LanguageDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageDetector {
    pub fn new() -> Self {
        let mut extension_map = HashMap::new();

        // Rust
        extension_map.insert("rs".to_string(), Language::Rust);

        // TypeScript/JavaScript
        extension_map.insert("ts".to_string(), Language::TypeScript);
        extension_map.insert("tsx".to_string(), Language::TypeScript);
        extension_map.insert("js".to_string(), Language::JavaScript);
        extension_map.insert("jsx".to_string(), Language::JavaScript);
        extension_map.insert("mjs".to_string(), Language::JavaScript);
        extension_map.insert("cjs".to_string(), Language::JavaScript);

        // Python
        extension_map.insert("py".to_string(), Language::Python);
        extension_map.insert("pyw".to_string(), Language::Python);
        extension_map.insert("pyi".to_string(), Language::Python);

        // Go
        extension_map.insert("go".to_string(), Language::Go);

        // Java
        extension_map.insert("java".to_string(), Language::Java);

        // C/C++
        extension_map.insert("c".to_string(), Language::C);
        extension_map.insert("h".to_string(), Language::C);
        extension_map.insert("cpp".to_string(), Language::Cpp);
        extension_map.insert("cxx".to_string(), Language::Cpp);
        extension_map.insert("cc".to_string(), Language::Cpp);
        extension_map.insert("hpp".to_string(), Language::Cpp);
        extension_map.insert("hxx".to_string(), Language::Cpp);

        // C#
        extension_map.insert("cs".to_string(), Language::CSharp);

        // Ruby
        extension_map.insert("rb".to_string(), Language::Ruby);
        extension_map.insert("rake".to_string(), Language::Ruby);

        // PHP
        extension_map.insert("php".to_string(), Language::Php);
        extension_map.insert("phtml".to_string(), Language::Php);

        // Swift
        extension_map.insert("swift".to_string(), Language::Swift);

        // Kotlin
        extension_map.insert("kt".to_string(), Language::Kotlin);
        extension_map.insert("kts".to_string(), Language::Kotlin);

        // Scala
        extension_map.insert("scala".to_string(), Language::Scala);
        extension_map.insert("sc".to_string(), Language::Scala);

        // Haskell
        extension_map.insert("hs".to_string(), Language::Haskell);
        extension_map.insert("lhs".to_string(), Language::Haskell);

        // Elixir
        extension_map.insert("ex".to_string(), Language::Elixir);
        extension_map.insert("exs".to_string(), Language::Elixir);

        // Clojure
        extension_map.insert("clj".to_string(), Language::Clojure);
        extension_map.insert("cljs".to_string(), Language::Clojure);
        extension_map.insert("cljc".to_string(), Language::Clojure);

        // Lua
        extension_map.insert("lua".to_string(), Language::Lua);

        // Zig
        extension_map.insert("zig".to_string(), Language::Zig);

        let shebang_patterns = vec![
            (Regex::new(r"^#!/.*\bpython").unwrap(), Language::Python),
            (Regex::new(r"^#!/.*\bruby").unwrap(), Language::Ruby),
            (Regex::new(r"^#!/.*\bnode").unwrap(), Language::JavaScript),
            (Regex::new(r"^#!/.*\bphp").unwrap(), Language::Php),
            (Regex::new(r"^#!/.*\blua").unwrap(), Language::Lua),
            (Regex::new(r"^#!/.*\belixir").unwrap(), Language::Elixir),
        ];

        Self {
            extension_map,
            shebang_patterns,
        }
    }

    pub fn detect(&self, file_path: &Path) -> Result<Language> {
        // First try extension-based detection
        if let Some(ext) = file_path.extension() {
            if let Some(ext_str) = ext.to_str() {
                if let Some(&lang) = self.extension_map.get(ext_str) {
                    return Ok(lang);
                }
            }
        }

        // Try to read the file for shebang detection
        if let Ok(content) = fs::read_to_string(file_path) {
            if let Some(first_line) = content.lines().next() {
                for (pattern, lang) in &self.shebang_patterns {
                    if pattern.is_match(first_line) {
                        return Ok(*lang);
                    }
                }
            }
        }

        // Default to unknown
        Ok(Language::Unknown)
    }

    pub fn detect_from_extension(&self, extension: &str) -> Option<Language> {
        self.extension_map.get(extension).copied()
    }

    /// Discover workspaces in a directory
    pub fn discover_workspaces(
        &self,
        root: &Path,
        recursive: bool,
    ) -> Result<HashMap<PathBuf, HashSet<Language>>> {
        let mut workspaces: HashMap<PathBuf, HashSet<Language>> = HashMap::new();

        // Check for workspace marker in root directory
        if let Some(languages) = self.detect_workspace_languages(root)? {
            if !languages.is_empty() {
                workspaces.insert(root.to_path_buf(), languages);
            }
        }

        // If recursive, search for nested workspaces
        if recursive {
            self.discover_nested_workspaces(root, &mut workspaces)?;
        }

        // If no workspace markers found, detect languages from files in root
        if workspaces.is_empty() {
            if let Some(languages) = self.detect_languages_from_files(root)? {
                if !languages.is_empty() {
                    workspaces.insert(root.to_path_buf(), languages);
                }
            }
        }

        Ok(workspaces)
    }

    /// Recursively discover nested workspaces with depth tracking
    fn discover_nested_workspaces(
        &self,
        dir: &Path,
        workspaces: &mut HashMap<PathBuf, HashSet<Language>>,
    ) -> Result<()> {
        self.discover_nested_workspaces_with_depth(dir, workspaces, 0, 10)
    }

    fn discover_nested_workspaces_with_depth(
        &self,
        dir: &Path,
        workspaces: &mut HashMap<PathBuf, HashSet<Language>>,
        current_depth: usize,
        max_depth: usize,
    ) -> Result<()> {
        // Prevent excessive recursion
        if current_depth >= max_depth {
            return Ok(());
        }

        // Read directory entries safely
        if let Ok(entries) = path_safety::safe_read_dir(dir) {
            for entry in entries {
                let path = entry.path();

                // Skip hidden directories and common build/dependency directories
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.')
                        || name == "node_modules"
                        || name == "target"
                        || name == "dist"
                        || name == "build"
                        || name == "vendor"
                        || name == "__pycache__"
                        || name == ".git"
                        || name == "out"
                        || name == "bin"
                        || name == "obj"
                    {
                        continue;
                    }
                }

                if path.is_dir() {
                    // Check if this directory is a workspace
                    if let Some(languages) = self.detect_workspace_languages(&path)? {
                        if !languages.is_empty() {
                            workspaces.insert(path.clone(), languages);
                            // IMPORTANT: Continue recursing to find nested workspaces
                            // This allows discovery of monorepo sub-projects
                        }
                    }

                    // Always recurse into subdirectory to find all nested workspaces
                    self.discover_nested_workspaces_with_depth(
                        &path,
                        workspaces,
                        current_depth + 1,
                        max_depth,
                    )?;
                }
            }
        }

        Ok(())
    }

    /// Detect workspace languages based on marker files
    fn detect_workspace_languages(&self, dir: &Path) -> Result<Option<HashSet<Language>>> {
        let mut languages = HashSet::new();

        // Check for language-specific workspace markers
        let markers = [
            ("Cargo.toml", Language::Rust),
            ("package.json", Language::TypeScript), // Can be JS or TS
            ("tsconfig.json", Language::TypeScript),
            ("go.mod", Language::Go),
            ("pom.xml", Language::Java),
            ("build.gradle", Language::Java),
            ("build.gradle.kts", Language::Kotlin),
            ("requirements.txt", Language::Python),
            ("pyproject.toml", Language::Python),
            ("setup.py", Language::Python),
            ("Pipfile", Language::Python),
            ("composer.json", Language::Php),
            ("Gemfile", Language::Ruby),
            ("Package.swift", Language::Swift),
            ("build.sbt", Language::Scala),
            ("stack.yaml", Language::Haskell),
            ("mix.exs", Language::Elixir),
            ("project.clj", Language::Clojure),
            ("deps.edn", Language::Clojure),
            ("CMakeLists.txt", Language::Cpp),
            (".csproj", Language::CSharp),
            (".sln", Language::CSharp),
        ];

        for (marker, language) in markers {
            if path_safety::exists_no_follow(&dir.join(marker)) {
                languages.insert(language);
            }
        }

        // Special case: Check for .csproj or .sln files
        if let Ok(entries) = path_safety::safe_read_dir(dir) {
            for entry in entries {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".csproj") || name.ends_with(".sln") {
                        languages.insert(Language::CSharp);
                    }
                }
            }
        }

        // If package.json exists, check if it's TypeScript or JavaScript
        if dir.join("package.json").exists() {
            if dir.join("tsconfig.json").exists() {
                languages.insert(Language::TypeScript);
            } else {
                // Check for TypeScript files
                let has_ts = self.has_files_with_extension(dir, &["ts", "tsx"])?;
                if has_ts {
                    languages.insert(Language::TypeScript);
                } else {
                    languages.insert(Language::JavaScript);
                }
            }
        }

        if languages.is_empty() {
            Ok(None)
        } else {
            Ok(Some(languages))
        }
    }

    /// Detect languages from files in a directory (fallback when no workspace markers)
    fn detect_languages_from_files(&self, dir: &Path) -> Result<Option<HashSet<Language>>> {
        let mut languages = HashSet::new();
        let mut checked_extensions = HashSet::new();

        // Scan files in the directory (non-recursive)
        if let Ok(entries) = path_safety::safe_read_dir(dir) {
            for entry in entries {
                let path = entry.path();
                if path_safety::is_file_no_follow(&path) {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        // Only check each extension once
                        if !checked_extensions.contains(ext) {
                            checked_extensions.insert(ext.to_string());
                            if let Some(lang) = self.detect_from_extension(ext) {
                                if lang != Language::Unknown {
                                    languages.insert(lang);
                                }
                            }
                        }
                    }
                }
            }
        }

        if languages.is_empty() {
            Ok(None)
        } else {
            Ok(Some(languages))
        }
    }

    /// Check if directory contains files with given extensions
    fn has_files_with_extension(&self, dir: &Path, extensions: &[&str]) -> Result<bool> {
        if let Ok(entries) = path_safety::safe_read_dir(dir) {
            for entry in entries {
                let path = entry.path();
                if path_safety::is_file_no_follow(&path) {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if extensions.contains(&ext) {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_extension_detection() {
        let detector = LanguageDetector::new();

        assert_eq!(detector.detect_from_extension("rs"), Some(Language::Rust));
        assert_eq!(detector.detect_from_extension("py"), Some(Language::Python));
        assert_eq!(
            detector.detect_from_extension("ts"),
            Some(Language::TypeScript)
        );
        assert_eq!(detector.detect_from_extension("go"), Some(Language::Go));
        assert_eq!(detector.detect_from_extension("unknown"), None);
    }

    #[test]
    fn test_file_detection() -> Result<()> {
        let detector = LanguageDetector::new();
        let dir = tempdir()?;

        // Test Rust file
        let rust_file = dir.path().join("test.rs");
        File::create(&rust_file)?;
        assert_eq!(detector.detect(&rust_file)?, Language::Rust);

        // Test Python file with shebang
        let py_file = dir.path().join("script");
        let mut file = File::create(&py_file)?;
        writeln!(file, "#!/usr/bin/env python3")?;
        writeln!(file, "print('Hello')")?;
        assert_eq!(detector.detect(&py_file)?, Language::Python);

        Ok(())
    }
}
