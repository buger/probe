use std::path::Path;

/// Function to determine if a file is a test file based on common naming conventions and directory patterns
pub fn is_test_file(path: &Path) -> bool {
    let _debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // Check file name patterns
    if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
        // Rust: *_test.rs, *_tests.rs, test_*.rs, tests.rs
        if file_name.ends_with("_test.rs")
            || file_name.ends_with("_tests.rs")
            || file_name.starts_with("test_")
            || file_name == "tests.rs"
        {
            if _debug_mode {
                println!("DEBUG: Test file detected (Rust pattern): {file_name}");
            }
            return true;
        }

        // JavaScript/TypeScript: *.test.js, *.spec.js, *.test.ts, *.spec.ts
        if file_name.ends_with(".test.js")
            || file_name.ends_with(".spec.js")
            || file_name.ends_with(".test.jsx")
            || file_name.ends_with(".spec.jsx")
            || file_name.ends_with(".test.ts")
            || file_name.ends_with(".spec.ts")
            || file_name.ends_with(".test.tsx")
            || file_name.ends_with(".spec.tsx")
        {
            if _debug_mode {
                println!("DEBUG: Test file detected (JS/TS pattern): {file_name}");
            }
            return true;
        }

        // Python: test_*.py, *_test.py
        if file_name.starts_with("test_") && file_name.ends_with(".py")
            || file_name.ends_with("_test.py")
        {
            if _debug_mode {
                println!("DEBUG: Test file detected (Python pattern): {file_name}");
            }
            return true;
        }

        // Go: *_test.go
        if file_name.ends_with("_test.go") {
            if _debug_mode {
                println!("DEBUG: Test file detected (Go pattern): {file_name}");
            }
            return true;
        }

        // C/C++: test_*.c, *_test.c, *_tests.c, test_*.cpp, *_test.cpp, *_tests.cpp
        if (file_name.starts_with("test_")
            || file_name.ends_with("_test.c")
            || file_name.ends_with("_tests.c"))
            && (file_name.ends_with(".c") || file_name.ends_with(".h"))
            || (file_name.starts_with("test_")
                || file_name.ends_with("_test.cpp")
                || file_name.ends_with("_tests.cpp"))
                && (file_name.ends_with(".cpp")
                    || file_name.ends_with(".hpp")
                    || file_name.ends_with(".cc")
                    || file_name.ends_with(".hxx")
                    || file_name.ends_with(".cxx"))
        {
            if _debug_mode {
                println!("DEBUG: Test file detected (C/C++ pattern): {file_name}");
            }
            return true;
        }

        // Java: *Test.java, Test*.java
        if file_name.ends_with("Test.java")
            || file_name.starts_with("Test") && file_name.ends_with(".java")
        {
            if _debug_mode {
                println!("DEBUG: Test file detected (Java pattern): {file_name}");
            }
            return true;
        }

        // Ruby: test_*.rb, *_test.rb, *_spec.rb
        if file_name.starts_with("test_") && file_name.ends_with(".rb")
            || file_name.ends_with("_test.rb")
            || file_name.ends_with("_spec.rb")
        {
            if _debug_mode {
                println!("DEBUG: Test file detected (Ruby pattern): {file_name}");
            }
            return true;
        }

        // PHP: *Test.php, Test*.php
        if file_name.ends_with("Test.php")
            || file_name.starts_with("Test") && file_name.ends_with(".php")
        {
            if _debug_mode {
                println!("DEBUG: Test file detected (PHP pattern): {file_name}");
            }
            return true;
        }
    }

    // Check directory patterns
    let path_str = path.to_string_lossy();

    // Common test directory patterns across languages
    if path_str.contains("/test/")
        || path_str.contains("/tests/")
        || path_str.contains("/spec/")
        || path_str.contains("/specs/")
        || path_str.contains("/__tests__/")
        || path_str.contains("/__test__/")
    {
        if _debug_mode {
            println!("DEBUG: Test file detected (in test directory): {path_str}");
        }
        return true;
    }

    false
}
