use std::path::PathBuf;

/// Get the default socket/pipe path for the current platform
pub fn get_default_socket_path() -> String {
    // Check for environment variable override first
    if let Ok(path) = std::env::var("PROBE_LSP_SOCKET_PATH") {
        return path;
    }

    #[cfg(unix)]
    {
        std::env::temp_dir()
            .join("lsp-daemon.sock")
            .to_string_lossy()
            .to_string()
    }

    #[cfg(windows)]
    {
        r"\\.\pipe\lsp-daemon".to_string()
    }
}

/// Check if a socket/pipe path exists
pub fn socket_exists(path: &str) -> bool {
    #[cfg(unix)]
    {
        std::path::Path::new(path).exists()
    }

    #[cfg(windows)]
    {
        // On Windows, check if we can connect to the named pipe
        use tokio::net::windows::named_pipe::ClientOptions;

        // Try to connect with a short timeout to check if pipe exists
        let _client =
            ClientOptions::new().pipe_mode(tokio::net::windows::named_pipe::PipeMode::Message);

        // Use blocking I/O for the existence check (quick operation)
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
        {
            Ok(_) => {
                tracing::trace!("Named pipe exists and is accessible: {}", path);
                true
            }
            Err(e) => {
                tracing::trace!(
                    "Named pipe does not exist or is not accessible: {} (error: {})",
                    path,
                    e
                );
                false
            }
        }
    }
}

/// Remove a socket file (Unix only, no-op on Windows)
pub fn remove_socket_file(path: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        if std::path::Path::new(path).exists() {
            std::fs::remove_file(path)?;
        }
    }

    #[cfg(windows)]
    {
        // Named pipes don't leave files on Windows, so this is a no-op
        tracing::trace!("Socket removal is no-op on Windows for path: {}", path);
    }

    Ok(())
}

/// Get the parent directory for socket file (Unix only)
pub fn get_socket_parent_dir(path: &str) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::path::Path::new(path).parent().map(|p| p.to_path_buf())
    }

    #[cfg(windows)]
    {
        // Named pipes don't need parent directory creation on Windows
        tracing::trace!(
            "Parent directory creation is not needed on Windows for path: {}",
            path
        );
        None
    }
}

/// Normalize executable command for the platform
pub fn normalize_executable(command: &str) -> String {
    #[cfg(windows)]
    {
        // Add .exe extension if not present
        if !command.ends_with(".exe")
            && !command.ends_with(".bat")
            && !command.ends_with(".cmd")
            && !command.contains('.')
        {
            format!("{}.exe", command)
        } else {
            command.to_string()
        }
    }

    #[cfg(unix)]
    {
        command.to_string()
    }
}

/// Get platform-specific path separator
pub fn path_separator() -> &'static str {
    #[cfg(windows)]
    {
        "\\"
    }

    #[cfg(unix)]
    {
        "/"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_socket_path() {
        let path = get_default_socket_path();

        #[cfg(unix)]
        assert!(path.ends_with("lsp-daemon.sock"));

        #[cfg(windows)]
        assert_eq!(path, r"\\.\pipe\lsp-daemon");
    }

    #[test]
    fn test_normalize_executable() {
        #[cfg(windows)]
        {
            assert_eq!(normalize_executable("rust-analyzer"), "rust-analyzer.exe");
            assert_eq!(normalize_executable("script.bat"), "script.bat");
            assert_eq!(normalize_executable("tool.exe"), "tool.exe");
        }

        #[cfg(unix)]
        {
            assert_eq!(normalize_executable("rust-analyzer"), "rust-analyzer");
            assert_eq!(normalize_executable("script.sh"), "script.sh");
        }
    }
}
