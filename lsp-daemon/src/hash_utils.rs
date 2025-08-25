use anyhow::Result;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Compute MD5 hash of a file's contents
pub fn md5_hex_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    let digest = md5::compute(&buffer);
    Ok(format!("{digest:x}"))
}

/// Compute MD5 hash of string content
pub fn md5_hex(content: &str) -> String {
    let digest = md5::compute(content.as_bytes());
    format!("{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_md5_hex() {
        let content = "Hello, World!";
        let hash = md5_hex(content);
        assert_eq!(hash, "65a8e27d8879283831b664bd8b7f0ad4");
    }

    #[test]
    fn test_md5_hex_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "Test content")?;

        let hash = md5_hex_file(&file_path)?;
        assert_eq!(hash, "9e107d9d372bb6826bd81d3542a419d6");

        Ok(())
    }

    #[test]
    fn test_md5_consistency() -> Result<()> {
        let content = "Consistent content";
        let hash1 = md5_hex(content);
        let hash2 = md5_hex(content);
        assert_eq!(hash1, hash2, "Same content should produce same hash");

        Ok(())
    }
}
