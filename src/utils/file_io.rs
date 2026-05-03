use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub const MAX_CONFIG_FILE_SIZE: u64 = 10 * 1024 * 1024;

pub fn read_config_file(path: &Path) -> Result<String> {
    read_config_file_with_limit(path, MAX_CONFIG_FILE_SIZE)
}

pub fn check_file_size(file: &File, path: &Path) -> Result<()> {
    check_file_size_with_limit(file, path, MAX_CONFIG_FILE_SIZE)
}

pub fn check_file_size_with_limit(file: &File, path: &Path, max_size: u64) -> Result<()> {
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to get metadata for {}", path.display()))?;

    if metadata.len() > max_size {
        bail!(
            "config file {} is too large ({} bytes, max {} bytes)",
            path.display(),
            metadata.len(),
            max_size
        );
    }

    Ok(())
}

pub fn read_config_file_with_limit(path: &Path, max_size: u64) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;

    let metadata = file
        .metadata()
        .with_context(|| format!("failed to get metadata for {}", path.display()))?;

    if metadata.len() > max_size {
        bail!(
            "config file {} is too large ({} bytes, max {} bytes)",
            path.display(),
            metadata.len(),
            max_size
        );
    }

    let mut contents = String::with_capacity(metadata.len() as usize);
    file.read_to_string(&mut contents)
        .with_context(|| format!("failed to read {}", path.display()))?;

    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_config_file_success() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "[package]\nname = \"test\"").unwrap();

        let content = read_config_file(file.path()).unwrap();
        assert!(content.contains("test"));
    }

    #[test]
    fn test_read_config_file_too_large() {
        let mut file = NamedTempFile::new().unwrap();
        let large_content = "x".repeat(1024);
        file.write_all(large_content.as_bytes()).unwrap();

        let result = read_config_file_with_limit(file.path(), 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_read_config_file_not_found() {
        let result = read_config_file(Path::new("/nonexistent/path/file.toml"));
        assert!(result.is_err());
    }
}
