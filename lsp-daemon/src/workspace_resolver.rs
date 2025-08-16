use crate::language_detector::Language;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

pub struct WorkspaceResolver {
    allowed_roots: Option<Vec<PathBuf>>,
    workspace_cache: HashMap<PathBuf, PathBuf>, // file_dir -> workspace_root
    max_cache_size: usize,
}

impl WorkspaceResolver {
    pub fn new(allowed_roots: Option<Vec<PathBuf>>) -> Self {
        Self {
            allowed_roots,
            workspace_cache: HashMap::new(),
            max_cache_size: 1000,
        }
    }

    /// Resolve workspace root for a given file path
    pub fn resolve_workspace(
        &mut self,
        file_path: &Path,
        hint: Option<PathBuf>,
    ) -> Result<PathBuf> {
        info!(
            "Resolving workspace for file: {:?}, hint: {:?}",
            file_path, hint
        );

        // 1. Use client hint if provided and valid
        if let Some(hint_root) = hint {
            // Canonicalize the hint path to ensure it's absolute
            let canonical_hint = hint_root.canonicalize().unwrap_or_else(|e| {
                warn!("Failed to canonicalize hint {:?}: {}", hint_root, e);
                hint_root.clone()
            });
            if self.is_valid_workspace(&canonical_hint, file_path)? {
                info!("Using client workspace hint: {:?}", canonical_hint);
                return Ok(canonical_hint);
            }
            warn!(
                "Client workspace hint {:?} is invalid for file {:?}",
                canonical_hint, file_path
            );
        }

        // 2. Check cache
        let file_dir = file_path.parent().unwrap_or(file_path).to_path_buf();
        if let Some(cached_root) = self.workspace_cache.get(&file_dir) {
            debug!("Using cached workspace: {:?}", cached_root);
            return Ok(cached_root.clone());
        }

        // 3. Auto-detect workspace
        let detected_root = self.detect_workspace(file_path)?;
        info!("Auto-detected workspace: {:?}", detected_root);

        // Canonicalize the detected root to ensure it's an absolute path
        let canonical_root = detected_root.canonicalize().unwrap_or_else(|e| {
            warn!(
                "Failed to canonicalize detected root {:?}: {}",
                detected_root, e
            );
            detected_root.clone()
        });

        // 4. Validate against allowed_roots if configured
        if let Some(ref allowed) = self.allowed_roots {
            if !allowed.iter().any(|root| canonical_root.starts_with(root)) {
                return Err(anyhow!(
                    "Workspace {:?} not in allowed roots: {:?}",
                    canonical_root,
                    allowed
                ));
            }
        }

        // 5. Cache and return the canonical path
        self.cache_workspace(file_dir, canonical_root.clone());
        info!("Resolved workspace root: {:?}", canonical_root);
        Ok(canonical_root)
    }

    /// Detect the most appropriate workspace root for a file
    fn detect_workspace(&self, file_path: &Path) -> Result<PathBuf> {
        let file_dir = file_path.parent().unwrap_or(file_path);

        // Look for common project markers
        let mut current = file_dir;
        let mut best_match: Option<(PathBuf, usize)> = None; // (path, priority)

        while let Some(parent) = current.parent() {
            // Check for various project markers with priorities
            let markers_with_priority = [
                // High priority - language-specific project files
                ("go.mod", 100),
                ("go.work", 95),
                ("Cargo.toml", 100),
                ("package.json", 90),
                ("pyproject.toml", 100),
                ("setup.py", 80),
                ("pom.xml", 100),
                ("build.gradle", 90),
                ("build.gradle.kts", 90),
                ("CMakeLists.txt", 85),
                // Medium priority - build/config files
                ("Makefile", 60),
                ("makefile", 60),
                ("configure.ac", 70),
                ("meson.build", 70),
                // Lower priority - VCS roots
                (".git", 50),
                (".svn", 40),
                (".hg", 40),
                // Very low priority - generic markers
                ("LICENSE", 20),
                ("README.md", 20),
            ];

            for (marker, priority) in &markers_with_priority {
                if current.join(marker).exists() {
                    match &best_match {
                        None => best_match = Some((current.to_path_buf(), *priority)),
                        Some((_, current_priority)) => {
                            if *priority > *current_priority {
                                best_match = Some((current.to_path_buf(), *priority));
                            }
                        }
                    }
                }
            }

            // Don't go too far up the tree
            if current.ancestors().count() > 10 {
                break;
            }

            current = parent;
        }

        // Return best match or file's directory
        Ok(best_match
            .map(|(path, _)| path)
            .unwrap_or_else(|| file_dir.to_path_buf()))
    }

    /// Check if a workspace hint is valid for the given file
    fn is_valid_workspace(&self, workspace_root: &Path, file_path: &Path) -> Result<bool> {
        // File must be within the workspace
        if !file_path.starts_with(workspace_root) {
            return Ok(false);
        }

        // Workspace must exist
        if !workspace_root.exists() {
            return Ok(false);
        }

        // If allowed_roots is configured, workspace must be within one of them
        if let Some(ref allowed) = self.allowed_roots {
            if !allowed.iter().any(|root| workspace_root.starts_with(root)) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Cache a workspace resolution, with size limit
    fn cache_workspace(&mut self, file_dir: PathBuf, workspace_root: PathBuf) {
        if self.workspace_cache.len() >= self.max_cache_size {
            // Simple cache eviction - remove oldest entries
            // In a more sophisticated implementation, we could use LRU
            let to_remove: Vec<_> = self
                .workspace_cache
                .keys()
                .take(self.max_cache_size / 4)
                .cloned()
                .collect();
            for key in to_remove {
                self.workspace_cache.remove(&key);
            }
        }

        self.workspace_cache.insert(file_dir, workspace_root);
    }

    /// Get language-specific project markers
    #[allow(dead_code)]
    pub fn get_language_markers(&self, language: Language) -> Vec<&'static str> {
        match language {
            Language::Go => vec!["go.mod", "go.work", "vendor"],
            Language::Rust => vec!["Cargo.toml", "Cargo.lock"],
            Language::JavaScript | Language::TypeScript => {
                vec![
                    "package.json",
                    "tsconfig.json",
                    "jsconfig.json",
                    "node_modules",
                ]
            }
            Language::Python => vec![
                "pyproject.toml",
                "setup.py",
                "requirements.txt",
                "setup.cfg",
            ],
            Language::Java => vec!["pom.xml", "build.gradle", "build.gradle.kts"],
            Language::C | Language::Cpp => vec!["CMakeLists.txt", "Makefile", "configure.ac"],
            Language::CSharp => vec!["*.sln", "*.csproj"],
            Language::Ruby => vec!["Gemfile", ".ruby-version"],
            Language::Php => vec!["composer.json", "composer.lock"],
            Language::Swift => vec!["Package.swift", "*.xcodeproj"],
            Language::Kotlin => vec!["build.gradle.kts", "build.gradle"],
            Language::Scala => vec!["build.sbt", "build.sc"],
            Language::Haskell => vec!["stack.yaml", "*.cabal", "cabal.project"],
            Language::Elixir => vec!["mix.exs"],
            Language::Clojure => vec!["project.clj", "deps.edn"],
            Language::Lua => vec![".luarc.json"],
            Language::Zig => vec!["build.zig"],
            Language::Unknown => vec![".git", "README.md"],
        }
    }

    /// Clear the cache
    #[allow(dead_code)]
    pub fn clear_cache(&mut self) {
        self.workspace_cache.clear();
    }

    /// Get cache statistics
    #[allow(dead_code)]
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.workspace_cache.len(), self.max_cache_size)
    }

    /// Check if a path is within allowed roots
    pub fn is_path_allowed(&self, path: &Path) -> bool {
        match &self.allowed_roots {
            None => true, // No restrictions
            Some(allowed) => allowed.iter().any(|root| path.starts_with(root)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detect_go_workspace() {
        let temp_dir = TempDir::new().unwrap();
        let project_root = temp_dir.path().join("project");
        let src_dir = project_root.join("src");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(project_root.join("go.mod"), "module test").unwrap();

        let mut resolver = WorkspaceResolver::new(None);
        let file_path = src_dir.join("main.go");
        let workspace = resolver.resolve_workspace(&file_path, None).unwrap();

        // Canonicalize the expected path for comparison since resolve_workspace now returns canonical paths
        let expected = project_root.canonicalize().unwrap();
        assert_eq!(workspace, expected);
    }

    #[test]
    fn test_workspace_hint() {
        let temp_dir = TempDir::new().unwrap();
        let hint_root = temp_dir.path().to_path_buf();
        let file_path = hint_root.join("test.go");

        let mut resolver = WorkspaceResolver::new(None);
        let workspace = resolver
            .resolve_workspace(&file_path, Some(hint_root.clone()))
            .unwrap();

        // Canonicalize the expected path for comparison
        let expected = hint_root.canonicalize().unwrap();
        assert_eq!(workspace, expected);
    }

    #[test]
    fn test_allowed_roots_restriction() {
        let temp_dir = TempDir::new().unwrap();
        let allowed_root = temp_dir.path().join("allowed");
        let forbidden_root = temp_dir.path().join("forbidden");

        fs::create_dir_all(&allowed_root).unwrap();
        fs::create_dir_all(&forbidden_root).unwrap();

        // Canonicalize the allowed root for the resolver
        let canonical_allowed = allowed_root.canonicalize().unwrap();
        let mut resolver = WorkspaceResolver::new(Some(vec![canonical_allowed]));

        // File in allowed root should work
        let allowed_file = allowed_root.join("test.go");
        let result = resolver.resolve_workspace(&allowed_file, None);
        assert!(result.is_ok());

        // File in forbidden root should fail
        let forbidden_file = forbidden_root.join("test.go");
        let result = resolver.resolve_workspace(&forbidden_file, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_cache_functionality() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.go");

        let mut resolver = WorkspaceResolver::new(None);

        // First resolution should detect and cache
        let workspace1 = resolver.resolve_workspace(&file_path, None).unwrap();

        // Second resolution should use cache
        let workspace2 = resolver.resolve_workspace(&file_path, None).unwrap();

        assert_eq!(workspace1, workspace2);
        assert_eq!(resolver.cache_stats().0, 1); // One entry in cache
    }
}
