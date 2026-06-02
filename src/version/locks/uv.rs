//! Resolves locked versions from `uv.lock` (uv's TOML lock file).

use std::fs;

use tower_lsp::lsp_types::Url;
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::error::LockError;
use crate::version::lock::{LockResolver, extract_package_version_from_toml_lock, find_lock_file};

const LOCK_FILE_NAME: &str = "uv.lock";

pub struct UvLockResolver;

impl LockResolver for UvLockResolver {
    fn registry_type(&self) -> RegistryType {
        RegistryType::PyPI
    }

    fn resolve_locked_version(
        &self,
        manifest_uri: &Url,
        package_name: &str,
    ) -> Result<Option<String>, LockError> {
        let Some(lock_path) = find_lock_file(manifest_uri, LOCK_FILE_NAME)? else {
            return Ok(None);
        };

        let content = fs::read_to_string(&lock_path)
            .inspect_err(|e| warn!("Failed to read {:?}: {}", lock_path, e))?;

        Ok(extract_package_version_from_toml_lock(
            &content,
            package_name,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const UV_LOCK_SAMPLE: &str = r#"version = 1
requires-python = ">=3.10"

[[package]]
name = "click"
version = "8.1.7"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "requests"
version = "2.31.0"
source = { registry = "https://pypi.org/simple" }
dependencies = [
    { name = "urllib3" },
]
"#;

    #[test]
    fn resolves_version_from_uv_lock() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("pyproject.toml");
        fs::write(&manifest, "").unwrap();
        fs::write(tmp.path().join(LOCK_FILE_NAME), UV_LOCK_SAMPLE).unwrap();

        let uri = Url::from_file_path(&manifest).unwrap();
        let result = UvLockResolver
            .resolve_locked_version(&uri, "requests")
            .unwrap();

        assert_eq!(result, Some("2.31.0".to_string()));
    }

    #[test]
    fn returns_none_for_unknown_package() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("pyproject.toml");
        fs::write(&manifest, "").unwrap();
        fs::write(tmp.path().join(LOCK_FILE_NAME), UV_LOCK_SAMPLE).unwrap();

        let uri = Url::from_file_path(&manifest).unwrap();
        let result = UvLockResolver
            .resolve_locked_version(&uri, "django")
            .unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn returns_none_when_lock_file_missing() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("pyproject.toml");
        fs::write(&manifest, "").unwrap();

        let uri = Url::from_file_path(&manifest).unwrap();
        let result = UvLockResolver
            .resolve_locked_version(&uri, "click")
            .unwrap();

        assert_eq!(result, None);
    }
}
