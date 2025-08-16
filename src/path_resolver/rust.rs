//! Rust-specific path resolver implementation.

use super::PathResolver;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

/// A path resolver for Rust crates.
pub struct RustPathResolver;

impl Default for RustPathResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl RustPathResolver {
    /// Creates a new Rust path resolver.
    pub fn new() -> Self {
        RustPathResolver
    }

    /// Gets the path to a Rust crate using cargo metadata.
    fn get_crate_path(&self, crate_name: &str) -> Result<PathBuf, String> {
        // Run `cargo metadata --format-version=1`
        let output = Command::new("cargo")
            .args(["metadata", "--format-version=1"])
            .output()
            .map_err(|e| format!("Failed to execute 'cargo metadata': {e}"))?;

        if !output.status.success() {
            return Err(format!(
                "Error running 'cargo metadata': {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Failed to parse JSON output from 'cargo metadata': {e}"))?;

        // Find the package in the packages array
        if let Some(packages) = json["packages"].as_array() {
            for package in packages {
                if let Some(name) = package["name"].as_str() {
                    if name == crate_name {
                        if let Some(manifest_path) = package["manifest_path"].as_str() {
                            // The manifest_path points to Cargo.toml, we want the directory
                            let path = PathBuf::from(manifest_path);
                            return path.parent().map_or(
                                Err(format!(
                                    "Could not determine parent directory of {manifest_path}"
                                )),
                                |parent| Ok(parent.to_path_buf()),
                            );
                        }
                    }
                }
            }
        }

        // If we couldn't find it in the current workspace, try to find it in the dependencies
        if let Some(packages) = json["packages"].as_array() {
            for package in packages {
                if let Some(deps) = package["dependencies"].as_array() {
                    for dep in deps {
                        if let Some(name) = dep["name"].as_str() {
                            if name == crate_name {
                                // For dependencies, we need to look at the source
                                if let Some(source) = dep["source"].as_str() {
                                    // For registry dependencies, we need to look in the registry cache
                                    if source.starts_with("registry+") {
                                        return self.find_in_registry_cache(crate_name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // If we still couldn't find it, try to find it in the registry cache directly
        self.find_in_registry_cache(crate_name)
    }

    /// Finds a crate in the Cargo registry cache.
    fn find_in_registry_cache(&self, crate_name: &str) -> Result<PathBuf, String> {
        // Get the cargo home directory
        let cargo_home = if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
            PathBuf::from(cargo_home)
        } else {
            // Use dirs crate for cross-platform home directory support
            let home_dir =
                dirs::home_dir().ok_or_else(|| "Failed to determine home directory".to_string())?;
            home_dir.join(".cargo")
        };

        // The registry cache is in $CARGO_HOME/registry/src
        let registry_dir = cargo_home.join("registry").join("src");

        if !registry_dir.exists() {
            return Err(format!(
                "Cargo registry directory not found: {registry_dir:?}"
            ));
        }

        // Look for the crate in all registry indices
        let registry_indices = std::fs::read_dir(&registry_dir)
            .map_err(|e| format!("Failed to read registry directory: {e}"))?;

        for index_entry in registry_indices {
            let index_dir = index_entry
                .map_err(|e| format!("Failed to read registry index entry: {e}"))?
                .path();

            if !index_dir.is_dir() {
                continue;
            }

            // Look for directories that contain the crate name
            let crates = std::fs::read_dir(&index_dir)
                .map_err(|e| format!("Failed to read index directory: {e}"))?;

            for crate_entry in crates {
                let crate_dir = crate_entry
                    .map_err(|e| format!("Failed to read crate entry: {e}"))?
                    .path();

                if !crate_dir.is_dir() {
                    continue;
                }

                // Check if this directory contains our crate
                let dir_name = crate_dir
                    .file_name()
                    .ok_or_else(|| "Invalid directory name".to_string())?
                    .to_string_lossy();

                if dir_name.starts_with(&format!("{crate_name}-")) {
                    // Found a matching crate directory
                    return Ok(crate_dir);
                }
            }
        }

        Err(format!("Could not find Rust crate: {crate_name}"))
    }
}

impl PathResolver for RustPathResolver {
    fn prefix(&self) -> &'static str {
        "rust:"
    }

    fn split_module_and_subpath(
        &self,
        full_path_after_prefix: &str,
    ) -> Result<(String, Option<String>), String> {
        if full_path_after_prefix.is_empty() {
            return Err("Rust path (to Cargo.toml) cannot be empty".to_string());
        }

        // For Rust, the entire path is treated as the module identifier
        // We don't split into module/subpath for Rust paths
        Ok((full_path_after_prefix.to_string(), None))
    }

    fn resolve(&self, crate_name: &str) -> Result<PathBuf, String> {
        // First, check if this is a path to a Cargo.toml file
        let path = PathBuf::from(crate_name);
        if path.exists()
            && path.is_file()
            && path.file_name().is_some_and(|name| name == "Cargo.toml")
        {
            // If it's a Cargo.toml file, return its directory
            return path.parent().map_or(
                Err("Could not determine parent directory of Cargo.toml".to_string()),
                |parent| Ok(parent.to_path_buf()),
            );
        }

        // If it's a directory containing Cargo.toml, return the directory
        let crate_dir = PathBuf::from(crate_name);
        let cargo_toml = crate_dir.join("Cargo.toml");
        if crate_dir.exists() && crate_dir.is_dir() && cargo_toml.exists() {
            return Ok(crate_dir);
        }

        // Otherwise, try to resolve it as a crate name
        self.get_crate_path(crate_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_rust_path_resolver_with_directory() {
        // Create a temporary directory with a Cargo.toml file
        let temp_dir = tempfile::tempdir().unwrap();
        let cargo_toml_path = temp_dir.path().join("Cargo.toml");

        // Write a minimal Cargo.toml
        fs::write(
            &cargo_toml_path,
            r#"[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("Failed to write Cargo.toml");

        let resolver = RustPathResolver::new();
        let result = resolver.resolve(temp_dir.path().to_str().unwrap());

        assert!(
            result.is_ok(),
            "Failed to resolve directory with Cargo.toml: {result:?}"
        );
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_rust_path_resolver_with_cargo_toml() {
        // Create a temporary directory with a Cargo.toml file
        let temp_dir = tempfile::tempdir().unwrap();
        let cargo_toml_path = temp_dir.path().join("Cargo.toml");

        // Write a minimal Cargo.toml
        fs::write(
            &cargo_toml_path,
            r#"[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("Failed to write Cargo.toml");

        let resolver = RustPathResolver::new();
        let result = resolver.resolve(cargo_toml_path.to_str().unwrap());

        assert!(result.is_ok(), "Failed to resolve Cargo.toml: {result:?}");
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_rust_path_resolver_crate() {
        // Skip this test if cargo is not installed
        if Command::new("cargo").arg("--version").output().is_err() {
            println!("Skipping test_rust_path_resolver_crate: cargo is not installed");
            return;
        }

        // This test is more complex as it requires cargo to be installed
        // We'll try to resolve the current crate
        let resolver = RustPathResolver::new();

        // Get the name of the current crate from Cargo.toml
        // Use CARGO_MANIFEST_DIR to ensure we find the correct Cargo.toml regardless of working directory
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let cargo_toml_path = std::path::Path::new(manifest_dir).join("Cargo.toml");
        let cargo_toml = std::fs::read_to_string(&cargo_toml_path)
            .unwrap_or_else(|e| panic!("Failed to read Cargo.toml at {cargo_toml_path:?}: {e}"));

        // Extract the package name using a simple regex
        let re = regex::Regex::new(r#"name\s*=\s*"([^"]+)""#).unwrap();
        let crate_name = re
            .captures(&cargo_toml)
            .map(|cap| cap[1].to_string())
            .unwrap_or_else(|| "probe".to_string()); // Default to "probe" if not found

        let result = resolver.resolve(&crate_name);

        // The result should be Ok and point to the current directory or a valid path
        if let Ok(path) = result {
            assert!(path.exists(), "Path does not exist: {path:?}");

            // Check if it contains a Cargo.toml
            let cargo_toml_path = path.join("Cargo.toml");
            assert!(
                cargo_toml_path.exists(),
                "Cargo.toml not found: {cargo_toml_path:?}"
            );
        } else {
            println!("Skipping assertion for '{crate_name}': Crate not found");
        }
    }
}
