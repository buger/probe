use std::env;
use std::sync::{Mutex, OnceLock};

/// Normalize a URI-like or path-like string to a lowercased, forward-slash-separated path.
/// Strips common URI schemes such as file://, jar:, and jrt: while keeping the inner path.
fn normalize_path_like(mut s: &str) -> String {
    // Strip common URI schemes
    if let Some(rest) = s.strip_prefix("file://") {
        s = rest;
    } else if let Some(rest) = s.strip_prefix("jar:file://") {
        // e.g., jar:file:///path/to/jar.jar!/com/...
        s = rest;
    } else if let Some(rest) = s.strip_prefix("jar:") {
        s = rest;
    } else if let Some(rest) = s.strip_prefix("jrt:/") {
        // Java 9+ module system (virtual fs)
        s = rest;
    }
    // Drop the !/ portion inside jar URIs, keep the inner path
    let s = if let Some(idx) = s.find("!/") {
        &s[idx + 2..]
    } else {
        s
    };
    // Lowercase and normalize separators
    s.replace('\\', "/").to_lowercase()
}

static CACHE: OnceLock<Mutex<std::collections::HashMap<String, bool>>> = OnceLock::new();

fn cache() -> &'static Mutex<std::collections::HashMap<String, bool>> {
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Returns true if the path points to a language's *standard library* (not third-party).
/// Heuristics are deliberately conservative to avoid hiding user or third‑party code.
pub fn is_stdlib_path(path: &str) -> bool {
    let p = normalize_path_like(path);

    // Rust (rustup sysroot source)
    if p.contains("/rustlib/src/rust/library/") {
        return true;
    }

    // Python (stdlib only; exclude site/dist-packages)
    if p.contains("/lib/python") && !p.contains("site-packages") && !p.contains("dist-packages") {
        // Typical locations
        if p.contains("/usr/lib/python")
            || p.contains("/usr/local/lib/python")
            || p.contains("/.pyenv/versions/")
            || p.contains("/frameworks/python.framework/versions/")
            || p.contains("/opt/homebrew/cellar/python")
        {
            return true;
        }
    }

    // Go (GOROOT/src only; never GOPATH)
    // Respect GOROOT if available
    if let Ok(goroot) = env::var("GOROOT") {
        let g = normalize_path_like(&goroot);
        if p.starts_with(&(g.clone() + "/src/")) {
            return true;
        }
    }
    // Common GOROOTs
    if p.starts_with("/usr/local/go/src/")
        || p.starts_with("/usr/lib/go/src/")
        || p.contains("/cellar/go/") && p.contains("/libexec/src/")
        || p.starts_with("/c:/go/src/")
        || p.starts_with("/c:/program files/go/src/")
    {
        return true;
    }

    // Java (JDK stdlib via jars or jrt modules)
    if p.starts_with("java.") || p.starts_with("jdk.") {
        // came from jrt:/java.* after normalization above
        return true;
    }
    if p.contains("/jre/lib/rt.jar!/") || p.contains("/lib/rt.jar!/") {
        return true;
    }
    // For normalized jar URIs that become just the inner path, check if it's a java stdlib package
    if p.starts_with("java/")
        || p.starts_with("javax/")
        || p.starts_with("sun/")
        || p.starts_with("com/sun/")
    {
        return true;
    }
    if p.contains("/lib/modules") && p.contains("/java/") {
        // heuristic for exploded jmods content
        return true;
    }

    // TypeScript standard lib .d.ts
    if p.contains("/node_modules/typescript/lib/lib.") && p.ends_with(".d.ts") {
        return true;
    }

    // Ruby (core lib; exclude gems)
    if p.contains("/lib/ruby/") && !p.contains("/gems/") {
        return true;
    }

    // .NET
    if p.contains("/dotnet/shared/microsoft.netcore.app/")
        || p.contains("/windows/assembly/gac")
        || p.contains("/program files/dotnet/shared/microsoft.netcore.app/")
    {
        return true;
    }

    // C/C++ (very conservative)
    // Avoid blanket filtering of /usr/include; only filter known stdlib locations
    if p.contains("/libcxx/")
        || p.contains("/c++/v1/")
        || p.contains("/libstdc++/")
        || p.contains("/include/c++/")
        || p.contains("/lib/clang/") && p.contains("/include/")
    {
        return true;
    }

    false
}

/// Cached variant to avoid re-running heuristics for repeat paths.
pub fn is_stdlib_path_cached(path: &str) -> bool {
    let key = normalize_path_like(path);
    // Fast path: read lock via try
    if let Some(v) = cache().lock().unwrap().get(&key).cloned() {
        return v;
    }
    let v = is_stdlib_path(&key);
    cache().lock().unwrap().insert(key, v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_core_matches() {
        assert!(is_stdlib_path(
            "/.rustup/toolchains/stable/lib/rustlib/src/rust/library/core/src/option.rs"
        ));
    }

    #[test]
    fn test_python_stdlib() {
        assert!(is_stdlib_path("/usr/lib/python3.9/os.py"));
        assert!(is_stdlib_path("/System/Library/Frameworks/Python.framework/Versions/3.9/lib/python3.9/json/__init__.py"));
        // Third-party should NOT match
        assert!(!is_stdlib_path(
            "/Users/user/.local/lib/python3.9/site-packages/requests/__init__.py"
        ));
    }

    #[test]
    fn test_go_stdlib() {
        assert!(is_stdlib_path("/usr/local/go/src/fmt/print.go"));
        // GOPATH should NOT match
        assert!(!is_stdlib_path(
            "/Users/user/go/src/github.com/gin-gonic/gin/gin.go"
        ));
    }

    #[test]
    fn test_java_uris() {
        assert!(is_stdlib_path("jrt:/java.base/java/lang/String.class"));
        assert!(is_stdlib_path(
            "jar:file:///usr/lib/jvm/java-11/lib/rt.jar!/java/lang/Object.class"
        ));
    }

    #[test]
    fn test_typescript_stdlib() {
        assert!(is_stdlib_path(
            "/Users/user/project/node_modules/typescript/lib/lib.es2015.d.ts"
        ));
        // Regular node_modules should NOT match
        assert!(!is_stdlib_path(
            "/Users/user/project/node_modules/express/index.js"
        ));
    }

    #[test]
    fn test_path_normalization() {
        assert_eq!(normalize_path_like("file:///usr/lib/test"), "/usr/lib/test");
        assert_eq!(
            normalize_path_like("C:\\Windows\\System32"),
            "c:/windows/system32"
        );
    }
}
