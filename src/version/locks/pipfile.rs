//! Resolves locked versions from `Pipfile.lock`.

use std::fs;

use serde_json::Value;
use tower_lsp::lsp_types::Url;
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::error::LockError;
use crate::version::lock::{LockResolver, find_lock_file};

const LOCK_FILE_NAME: &str = "Pipfile.lock";
const PACKAGE_SECTIONS: [&str; 2] = ["default", "develop"];

pub struct PipfileLockResolver;

impl LockResolver for PipfileLockResolver {
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

        let json: Value =
            serde_json::from_str(&content).map_err(|e| LockError::InvalidFormat(e.to_string()))?;

        Ok(extract_locked_version(&json, package_name))
    }
}

/// Pipfile.lock stores `"version": "==X.Y.Z"`. We strip the leading `==` so the
/// caller gets a bare version (the pin formatter will re-add the operator
/// according to the registry's contract).
fn extract_locked_version(json: &Value, package_name: &str) -> Option<String> {
    for section in PACKAGE_SECTIONS {
        if let Some(version) = json
            .get(section)
            .and_then(|s| s.get(package_name))
            .and_then(|entry| entry.get("version"))
            .and_then(Value::as_str)
        {
            return Some(version.trim_start_matches("==").to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(content: &str) -> Value {
        serde_json::from_str(content).unwrap()
    }

    #[test]
    fn extracts_version_from_default_section() {
        let json = parse(
            r#"{
                "_meta": {},
                "default": {
                    "click": { "version": "==8.1.7", "hashes": [] },
                    "requests": { "version": "==2.31.0" }
                },
                "develop": {}
            }"#,
        );
        assert_eq!(
            extract_locked_version(&json, "requests"),
            Some("2.31.0".to_string())
        );
    }

    #[test]
    fn extracts_version_from_develop_section() {
        let json = parse(
            r#"{
                "default": {},
                "develop": {
                    "pytest": { "version": "==7.4.0" }
                }
            }"#,
        );
        assert_eq!(
            extract_locked_version(&json, "pytest"),
            Some("7.4.0".to_string())
        );
    }

    #[test]
    fn returns_none_for_unknown_package() {
        let json = parse(
            r#"{
                "default": {
                    "click": { "version": "==8.1.7" }
                },
                "develop": {}
            }"#,
        );
        assert_eq!(extract_locked_version(&json, "django"), None);
    }
}
