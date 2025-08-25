use probe_code::path_resolver::{
    resolve_path, GoPathResolver, JavaScriptPathResolver, PathResolver, RustPathResolver,
};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

// Helper function to check if node is installed and a module exists
fn check_js_module(module: &str) -> bool {
    if Command::new("node").arg("--version").output().is_err() {
        return false;
    }
    let script = format!(
        "try {{ require.resolve('{module}/package.json'); process.exit(0); }} catch (e) {{ process.exit(1); }}"
    );
    Command::new("node")
        .arg("-e")
        .arg(&script)
        .status()
        .is_ok_and(|s| s.success())
}

// Helper function to check if go is installed and a module exists
fn check_go_module(module: &str) -> bool {
    if Command::new("go").arg("version").output().is_err() {
        return false;
    }
    Command::new("go")
        .args(["list", module])
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn test_regular_path_no_prefix() {
    // Use a relative path that should exist
    let path = "Cargo.toml"; // Assumes test runs from project root
    let result = resolve_path(path);
    assert!(result.is_ok());
    // Expect it to be returned as is
    assert_eq!(result.unwrap(), PathBuf::from(path));

    // Test with an absolute path (use temp dir for reliability)
    let temp_dir = tempfile::tempdir().unwrap();
    let abs_path_str = temp_dir
        .path()
        .join("somefile")
        .to_str()
        .unwrap()
        .to_string();
    let result_abs = resolve_path(&abs_path_str);
    assert!(result_abs.is_ok());
    assert_eq!(result_abs.unwrap(), PathBuf::from(&abs_path_str));
}

// --- Go Tests ---

#[test]
fn test_go_stdlib_resolution_no_subpath() {
    if !check_go_module("fmt") {
        println!("Skipping test_go_stdlib_resolution_no_subpath: Go or 'fmt' module not available");
        return;
    }
    let result = resolve_path("go:fmt");
    assert!(result.is_ok(), "Failed to resolve 'go:fmt': {result:?}");
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    assert!(path.is_dir(), "Path is not a directory: {path:?}");
    assert!(
        path.join("print.go").exists(),
        "fmt/print.go not found in {path:?}"
    ); // Check a known file
}

#[test]
fn test_go_stdlib_resolution_with_subpath_file() {
    if !check_go_module("net/http") {
        // Need a stdlib package with known files/subdirs
        println!(
            "Skipping test_go_stdlib_resolution_with_subpath_file: Go or 'net/http' not available"
        );
        return;
    }
    let result = resolve_path("go:net/http/server.go"); // Path is module=net/http, sub=server.go
    assert!(
        result.is_ok(),
        "Failed to resolve 'go:net/http/server.go': {result:?}"
    );
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    assert!(path.is_file(), "Path is not a file: {path:?}");
    assert_eq!(path.file_name().unwrap(), "server.go");
}

#[test]
fn test_go_stdlib_resolution_with_subpath_dir() {
    if !check_go_module("net/http/pprof") {
        // Check if pprof sub-package is resolvable
        println!(
            "Skipping test_go_stdlib_resolution_with_subpath_dir: Go or 'net/http/pprof' not available"
        );
        return;
    }
    // Our current heuristic treats "net/http/pprof" as the module. Let's test that directly first.
    let result_mod = resolve_path("go:net/http/pprof");
    assert!(
        result_mod.is_ok(),
        "Failed to resolve 'go:net/http/pprof' as module: {result_mod:?}"
    );
    let path_mod = result_mod.unwrap();
    assert!(path_mod.exists(), "Path does not exist: {path_mod:?}");
    assert!(path_mod.is_dir(), "Path is not a directory: {path_mod:?}");
    assert!(
        path_mod.join("pprof.go").exists(),
        "pprof.go not found in {path_mod:?}"
    );
}

#[test]
fn test_go_external_resolution_no_subpath() {
    let module = "github.com/stretchr/testify";
    if !check_go_module(module) {
        println!("Skipping test_go_external_resolution_no_subpath: Go or '{module}' not available");
        return;
    }
    let result = resolve_path(&format!("go:{module}"));
    assert!(
        result.is_ok(),
        "Failed to resolve 'go:{module}': {result:?}"
    );
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    assert!(path.is_dir(), "Path is not a directory: {path:?}");
    assert!(
        path.join("assert").exists(),
        "'assert' subdir not found in {path:?}"
    ); // Check subdir
}

#[test]
fn test_go_external_resolution_with_subpath() {
    let module = "github.com/stretchr/testify";
    let subpath = "assert";
    let full_module_path = format!("{module}/{subpath}"); // e.g. github.com/stretchr/testify/assert
    if !check_go_module(&full_module_path) {
        // Check if the sub-package itself is resolvable by go list
        if !check_go_module(module) {
            // If not, check if base module exists before skipping entirely
            println!(
                "Skipping test_go_external_resolution_with_subpath: Go or base module '{module}' not available"
            );
            return;
        }
        // Base module exists, but sub-package doesn't resolve directly. Our split logic should handle this.
        println!(
            "Note: '{full_module_path}' not directly resolvable by 'go list', testing split logic."
        );
    } else if !check_go_module(module) {
        println!(
            "Skipping test_go_external_resolution_with_subpath: Go or base module '{module}' not available"
        );
        return;
    }

    let input_path = format!("go:{module}/{subpath}"); // "go:github.com/stretchr/testify/assert"
    let result = resolve_path(&input_path);
    assert!(
        result.is_ok(),
        "Failed to resolve '{input_path}': {result:?}"
    );
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    assert!(path.is_dir(), "Path is not a directory: {path:?}"); // assert is a directory
    assert!(
        path.file_name().unwrap() == subpath,
        "Path should end with '{subpath}': {path:?}"
    );
    assert!(
        path.join("assertions.go").exists(),
        "assertions.go not found in {path:?}"
    ); // Check known file
}

#[test]
fn test_go_nonexistent_package() {
    if Command::new("go").arg("version").output().is_err() {
        println!("Skipping test_go_nonexistent_package: Go not installed");
        return;
    }
    let result = resolve_path("go:nonexistent_gopkg_xyz_123_abc");
    assert!(
        result.is_err(),
        "Expected error for non-existent package, got Ok: {result:?}"
    );
}

// --- JavaScript Tests ---

#[test]
fn test_js_resolution_no_subpath() {
    // Use a commonly installed package like 'npm' itself or a dev dependency
    let module = "npm"; // Or change to a dev dep of *this* project if available
    if !check_js_module(module) {
        println!("Skipping test_js_resolution_no_subpath: Node or '{module}' module not available");
        return;
    }
    let result = resolve_path(&format!("js:{module}"));
    assert!(
        result.is_ok(),
        "Failed to resolve 'js:{module}': {result:?}"
    );
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    assert!(path.is_dir(), "Path is not a directory: {path:?}");
    assert!(
        path.join("package.json").exists(),
        "package.json not found in {path:?}"
    );
}

#[test]
fn test_js_resolution_with_subpath_file() {
    let module = "npm"; // Assuming npm has a known file/structure
    let subpath = "index.js"; // Common entry point, might exist
    if !check_js_module(module) {
        println!(
            "Skipping test_js_resolution_with_subpath_file: Node or '{module}' module not available"
        );
        return;
    }

    let input_path = format!("js:{module}/{subpath}");
    let result = resolve_path(&input_path);

    // Check if the subpath *actually* exists before asserting Ok
    let expected_path = match resolve_path(&format!("js:{module}")) {
        Ok(p) => p.join(subpath),
        Err(_) => {
            // Can't resolve base module
            println!(
                "Skipping test_js_resolution_with_subpath_file: Cannot resolve base module '{module}'"
            );
            return;
        }
    };

    if !expected_path.exists() {
        println!(
            "Skipping test_js_resolution_with_subpath_file: Expected subpath '{}' does not exist in module '{}'",
            expected_path.display(),
            module
        );
        // Optional: Assert error instead? Or just skip? Let's skip.
        return;
    }

    assert!(
        result.is_ok(),
        "Failed to resolve '{input_path}': {result:?}"
    );
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    assert!(path.is_file(), "Path is not a file: {path:?}"); // Check if it's a file
    assert!(
        path.file_name().unwrap() == subpath,
        "Path should end with '{subpath}': {path:?}"
    );
}

#[test]
fn test_js_resolution_with_subpath_dir() {
    let module = "npm";
    let subpath = "lib"; // npm usually has a lib directory
    if !check_js_module(module) {
        println!(
            "Skipping test_js_resolution_with_subpath_dir: Node or '{module}' module not available"
        );
        return;
    }

    let input_path = format!("js:{module}/{subpath}");
    let result = resolve_path(&input_path);

    // Check if the subpath *actually* exists
    let expected_path = match resolve_path(&format!("js:{module}")) {
        Ok(p) => p.join(subpath),
        Err(_) => {
            println!(
                "Skipping test_js_resolution_with_subpath_dir: Cannot resolve base module '{module}'"
            );
            return;
        }
    };
    if !expected_path.exists() || !expected_path.is_dir() {
        println!(
            "Skipping test_js_resolution_with_subpath_dir: Expected subpath dir '{}' does not exist in module '{}'",
            expected_path.display(),
            module
        );
        return;
    }

    assert!(
        result.is_ok(),
        "Failed to resolve '{input_path}': {result:?}"
    );
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    assert!(path.is_dir(), "Path is not a directory: {path:?}");
    assert!(
        path.file_name().unwrap() == subpath,
        "Path should end with '{subpath}': {path:?}"
    );
}

#[test]
fn test_js_scoped_resolution_no_subpath() {
    // Requires a globally or locally installed scoped package
    let module = "@npmcli/config"; // Example, check if installed
    if !check_js_module(module) {
        println!(
            "Skipping test_js_scoped_resolution_no_subpath: Node or '{module}' module not available"
        );
        return;
    }
    let result = resolve_path(&format!("js:{module}"));
    assert!(
        result.is_ok(),
        "Failed to resolve 'js:{module}': {result:?}"
    );
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    assert!(path.is_dir(), "Path is not a directory: {path:?}");
    assert!(
        path.join("package.json").exists(),
        "package.json not found in {path:?}"
    );
}

#[test]
fn test_js_scoped_resolution_with_subpath() {
    let module = "@npmcli/config";
    let subpath = "lib/index.js"; // Check actual structure if test fails
    if !check_js_module(module) {
        println!(
            "Skipping test_js_scoped_resolution_with_subpath: Node or '{module}' module not available"
        );
        return;
    }

    let input_path = format!("js:{module}/{subpath}");
    let result = resolve_path(&input_path);

    // Check if the subpath *actually* exists
    let expected_path = match resolve_path(&format!("js:{module}")) {
        Ok(p) => p.join(subpath),
        Err(_) => {
            println!(
                "Skipping test_js_scoped_resolution_with_subpath: Cannot resolve base module '{module}'"
            );
            return;
        }
    };
    if !expected_path.exists() {
        println!(
            "Skipping test_js_scoped_resolution_with_subpath: Expected subpath '{}' does not exist in module '{}'",
            expected_path.display(),
            module
        );
        return;
    }

    assert!(
        result.is_ok(),
        "Failed to resolve '{input_path}': {result:?}"
    );
    let path = result.unwrap();
    assert!(path.exists(), "Path does not exist: {path:?}");
    // assert!(path.is_file(), "Path is not a file: {:?}", path); // Adjust if subpath is dir
    assert!(
        path.ends_with(subpath.replace('/', std::path::MAIN_SEPARATOR_STR)),
        "Path should end with '{subpath}': {path:?}"
    );
}

#[test]
fn test_js_nonexistent_package() {
    if Command::new("node").arg("--version").output().is_err() {
        println!("Skipping test_js_nonexistent_package: Node not installed");
        return;
    }
    let result = resolve_path("js:nonexistent_jspkg_xyz_123_abc");
    assert!(
        result.is_err(),
        "Expected error for non-existent package, got Ok: {result:?}"
    );

    // The error message might vary depending on the environment, so just check that it's an error
    let err_msg = result.unwrap_err();
    println!("Error message for nonexistent JS package: {err_msg}");
    // Could contain "not found", "Cannot find module", "Failed to resolve", etc.
}

// --- Rust Tests ---

#[test]
fn test_rust_resolution() {
    // Create a temporary directory with a Cargo.toml file
    let temp_dir = tempfile::tempdir().unwrap();
    let crate_root = temp_dir.path();
    let cargo_toml_path = crate_root.join("Cargo.toml");

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

    // Test with the path to the Cargo.toml file
    // Use canonicalize to handle potential relative path issues in tests
    let canonical_toml_path = fs::canonicalize(&cargo_toml_path).unwrap();
    let path_str = format!("rust:{}", canonical_toml_path.to_str().unwrap());
    let result = resolve_path(&path_str);

    assert!(result.is_ok(), "Failed to resolve rust: path: {result:?}");
    // Expect the parent directory of the Cargo.toml
    let expected_path = fs::canonicalize(crate_root).unwrap();
    assert_eq!(result.unwrap(), expected_path);
}

#[test]
fn test_rust_nonexistent_toml() {
    let path_str = "rust:/non/existent/path/to/Cargo.toml";
    let result = resolve_path(path_str);
    assert!(
        result.is_err(),
        "Expected error for non-existent Cargo.toml, got Ok: {result:?}"
    );
}

#[test]
fn test_rust_path_is_not_toml() {
    // Create a temporary directory
    let temp_dir = tempfile::tempdir().unwrap();
    let dir_path = temp_dir.path();
    let canonical_dir_path = fs::canonicalize(dir_path).unwrap();
    let path_str = format!("rust:{}", canonical_dir_path.to_str().unwrap()); // Path to dir, not file

    let result = resolve_path(&path_str);
    // Resolve should fail because the path is not a file
    assert!(
        result.is_err(),
        "Expected error for path not pointing to a file, got Ok: {result:?}"
    );
}

// --- Dep Prefix Tests ---

#[test]
fn test_dep_go_resolution() {
    if !check_go_module("fmt") {
        println!("Skipping test_dep_go_resolution: Go or 'fmt' module not available");
        return;
    }

    // Test with standard library package using /dep/go prefix
    let result = resolve_path("/dep/go/fmt");

    // Compare with traditional go: prefix
    let traditional_result = resolve_path("go:fmt");

    assert!(
        result.is_ok(),
        "Failed to resolve '/dep/go/fmt': {result:?}"
    );
    assert!(
        traditional_result.is_ok(),
        "Failed to resolve 'go:fmt': {traditional_result:?}"
    );

    // Both paths should resolve to the same location
    assert_eq!(result.unwrap(), traditional_result.unwrap());
}

#[test]
fn test_dep_js_resolution() {
    let module = "npm"; // Or change to a dev dep of *this* project if available
    if !check_js_module(module) {
        println!("Skipping test_dep_js_resolution: Node or '{module}' module not available");
        return;
    }

    // Test with npm package using /dep/js prefix
    let result = resolve_path(&format!("/dep/js/{module}"));

    // Compare with traditional js: prefix
    let traditional_result = resolve_path(&format!("js:{module}"));

    assert!(
        result.is_ok(),
        "Failed to resolve '/dep/js/{module}': {result:?}"
    );
    assert!(
        traditional_result.is_ok(),
        "Failed to resolve 'js:{module}': {traditional_result:?}"
    );

    // Both paths should resolve to the same location
    assert_eq!(result.unwrap(), traditional_result.unwrap());
}

#[test]
fn test_dep_rust_resolution() {
    // Create a temporary directory with a Cargo.toml file
    let temp_dir = tempfile::tempdir().unwrap();
    let crate_root = temp_dir.path();
    let cargo_toml_path = crate_root.join("Cargo.toml");

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

    // Use canonicalize to handle potential relative path issues in tests
    let canonical_toml_path = fs::canonicalize(&cargo_toml_path).unwrap();

    // Test with /dep/rust prefix
    let dep_path_str = format!("/dep/rust/{}", canonical_toml_path.to_str().unwrap());
    let result = resolve_path(&dep_path_str);

    // Compare with traditional rust: prefix
    let traditional_path_str = format!("rust:{}", canonical_toml_path.to_str().unwrap());
    let traditional_result = resolve_path(&traditional_path_str);

    assert!(
        result.is_ok(),
        "Failed to resolve /dep/rust path: {result:?}"
    );
    assert!(
        traditional_result.is_ok(),
        "Failed to resolve rust: path: {traditional_result:?}"
    );

    // Both paths should resolve to the same location
    assert_eq!(result.unwrap(), traditional_result.unwrap());
}

#[test]
fn test_invalid_dep_paths() {
    // Test with invalid /dep/ path (missing language identifier)
    let result = resolve_path("/dep/");
    assert!(result.is_err());

    // Test with unknown language identifier
    let result = resolve_path("/dep/unknown/package");
    assert!(result.is_err());

    // Test with empty path after language identifier
    let result = resolve_path("/dep/go/");
    assert!(result.is_err());
}

// --- General/Error Tests ---
#[test]
fn test_invalid_prefix() {
    let result = resolve_path("invalidprefix:some/path");
    // Should be treated as a regular path
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PathBuf::from("invalidprefix:some/path"));
}

#[test]
fn test_empty_path_after_prefix_go() {
    if Command::new("go").arg("version").output().is_err() {
        return;
    } // Skip if no go
    let result = resolve_path("go:");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot be empty"));
}

#[test]
fn test_empty_path_after_prefix_js() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    } // Skip if no node
    let result = resolve_path("js:");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot be empty"));
}

#[test]
fn test_empty_path_after_prefix_rust() {
    let result = resolve_path("rust:");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot be empty"));
}

#[test]
fn test_path_with_dotdot_js() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    } // Skip if no node
    let result = resolve_path("js:lodash/../other");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot contain '..'"));
}

#[test]
fn test_path_with_dotdot_go() {
    if Command::new("go").arg("version").output().is_err() {
        return;
    } // Skip if no go
    let result = resolve_path("go:fmt/../other");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot contain '..'"));
}

// --- Split Module and Subpath Tests ---

#[test]
fn test_go_split_module_and_subpath() {
    let resolver = GoPathResolver::new();

    // Test stdlib
    let (module, subpath) = resolver.split_module_and_subpath("fmt").unwrap();
    assert_eq!(module, "fmt");
    assert_eq!(subpath, None);

    // Test stdlib with slashes
    let (module, subpath) = resolver.split_module_and_subpath("net/http").unwrap();
    assert_eq!(module, "net/http");
    assert_eq!(subpath, None);

    // Test GitHub repo
    let (module, subpath) = resolver
        .split_module_and_subpath("github.com/user/repo")
        .unwrap();
    assert_eq!(module, "github.com/user/repo");
    assert_eq!(subpath, None);

    // Test GitHub repo with subpath
    let (module, subpath) = resolver
        .split_module_and_subpath("github.com/user/repo/subpath")
        .unwrap();
    assert_eq!(module, "github.com/user/repo");
    assert_eq!(subpath, Some("subpath".to_string()));

    // Test GitHub repo with multi-segment subpath
    let (module, subpath) = resolver
        .split_module_and_subpath("github.com/user/repo/sub/path")
        .unwrap();
    assert_eq!(module, "github.com/user/repo");
    assert_eq!(subpath, Some("sub/path".to_string()));

    // Test error cases
    assert!(resolver.split_module_and_subpath("").is_err());
    assert!(resolver.split_module_and_subpath("fmt/../other").is_err());
}

#[test]
fn test_js_split_module_and_subpath() {
    let resolver = JavaScriptPathResolver::new();

    // Test regular package
    let (module, subpath) = resolver.split_module_and_subpath("lodash").unwrap();
    assert_eq!(module, "lodash");
    assert_eq!(subpath, None);

    // Test regular package with subpath
    let (module, subpath) = resolver.split_module_and_subpath("lodash/get").unwrap();
    assert_eq!(module, "lodash");
    assert_eq!(subpath, Some("get".to_string()));

    // Test scoped package
    let (module, subpath) = resolver.split_module_and_subpath("@types/node").unwrap();
    assert_eq!(module, "@types/node");
    assert_eq!(subpath, None);

    // Test scoped package with subpath
    let (module, subpath) = resolver.split_module_and_subpath("@types/node/fs").unwrap();
    assert_eq!(module, "@types/node");
    assert_eq!(subpath, Some("fs".to_string()));

    // Test error cases
    assert!(resolver.split_module_and_subpath("").is_err());
    assert!(resolver
        .split_module_and_subpath("lodash/../other")
        .is_err());
    assert!(resolver.split_module_and_subpath("@types").is_err());
    assert!(resolver.split_module_and_subpath("@/invalid").is_err());
}

#[test]
fn test_rust_split_module_and_subpath() {
    let resolver = RustPathResolver::new();

    // For Rust, the entire path is treated as the module identifier
    let (module, subpath) = resolver
        .split_module_and_subpath("/path/to/Cargo.toml")
        .unwrap();
    assert_eq!(module, "/path/to/Cargo.toml");
    assert_eq!(subpath, None);

    // Test error cases
    assert!(resolver.split_module_and_subpath("").is_err());
}
