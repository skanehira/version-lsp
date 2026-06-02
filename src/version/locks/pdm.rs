//! Resolves locked versions from `pdm.lock`.

use std::fs;

use tower_lsp::lsp_types::Url;
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::error::LockError;
use crate::version::lock::{LockResolver, extract_package_version_from_toml_lock, find_lock_file};

const LOCK_FILE_NAME: &str = "pdm.lock";

pub struct PdmLockResolver;

impl LockResolver for PdmLockResolver {
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

    const PDM_LOCK_SAMPLE: &str = r#"[metadata]
groups = ["default"]
strategy = ["cross_platform"]
lock_version = "4.4.1"

[[package]]
name = "click"
version = "8.1.7"
requires_python = ">=3.7"
summary = "Composable command line interface toolkit"

[[package]]
name = "requests"
version = "2.31.0"
requires_python = ">=3.7"
"#;

    #[test]
    fn resolves_version_from_pdm_lock() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("pyproject.toml");
        fs::write(&manifest, "").unwrap();
        fs::write(tmp.path().join(LOCK_FILE_NAME), PDM_LOCK_SAMPLE).unwrap();

        let uri = Url::from_file_path(&manifest).unwrap();
        let result = PdmLockResolver
            .resolve_locked_version(&uri, "click")
            .unwrap();

        assert_eq!(result, Some("8.1.7".to_string()));
    }
}
