use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

#[inline]
pub fn md5_hex_str(s: &str) -> String {
    format!("{:x}", md5::compute(s.as_bytes()))
}

#[inline]
pub fn md5_hex_bytes(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}

/// Compute the lowercase-hex MD5 for a file on disk.
pub fn md5_hex_file(path: &Path) -> Result<String> {
    let f = File::open(path)
        .with_context(|| format!("Failed to open file for MD5: {}", path.display()))?;
    let mut reader = BufReader::new(f);
    let mut hasher = md5::Context::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.consume(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.compute()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_md5_hex_str() {
        let hash = md5_hex_str("hello world");
        assert_eq!(hash, "5eb63bbbe01eeed093cb22bb8f5acdc3");
    }

    #[test]
    fn test_md5_hex_bytes() {
        let hash = md5_hex_bytes(b"hello world");
        assert_eq!(hash, "5eb63bbbe01eeed093cb22bb8f5acdc3");
    }

    #[test]
    fn test_md5_hex_file() -> Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world")?;

        let hash = md5_hex_file(&file_path)?;
        assert_eq!(hash, "5eb63bbbe01eeed093cb22bb8f5acdc3");

        Ok(())
    }
}
