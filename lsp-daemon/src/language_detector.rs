use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

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
}

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
