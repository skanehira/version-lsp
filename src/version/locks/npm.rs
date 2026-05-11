//! Resolves locked versions from `package-lock.json`.

use std::fs;

use serde_json::Value;
use tower_lsp::lsp_types::Url;
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::error::LockError;
use crate::version::lock::{LockResolver, find_lock_file};

const LOCK_FILE_NAME: &str = "package-lock.json";

pub struct NpmLockResolver;

impl LockResolver for NpmLockResolver {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Npm
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

/// Look up `package_name` in either `packages` (lockfileVersion 2/3) or
/// `dependencies` (v1) sections.
///
/// For v2/v3, prefers the top-level `node_modules/<name>` entry.
fn extract_locked_version(json: &Value, package_name: &str) -> Option<String> {
    if let Some(packages) = json.get("packages").and_then(Value::as_object) {
        let key = format!("node_modules/{package_name}");
        if let Some(version) = packages
            .get(&key)
            .and_then(|entry| entry.get("version"))
            .and_then(Value::as_str)
        {
            return Some(version.to_string());
        }
    }

    if let Some(deps) = json.get("dependencies").and_then(Value::as_object)
        && let Some(version) = deps
            .get(package_name)
            .and_then(|entry| entry.get("version"))
            .and_then(Value::as_str)
    {
        return Some(version.to_string());
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
    fn extracts_version_from_lockfile_v3_packages() {
        let json = parse(
            r#"{
                "name": "myapp",
                "lockfileVersion": 3,
                "packages": {
                    "": { "name": "myapp", "version": "1.0.0" },
                    "node_modules/lodash": { "version": "4.17.21" },
                    "node_modules/react": { "version": "18.2.0" }
                }
            }"#,
        );
        assert_eq!(
            extract_locked_version(&json, "lodash"),
            Some("4.17.21".to_string())
        );
    }

    #[test]
    fn extracts_version_from_lockfile_v1_dependencies() {
        let json = parse(
            r#"{
                "name": "myapp",
                "lockfileVersion": 1,
                "dependencies": {
                    "lodash": { "version": "4.17.20", "resolved": "..." }
                }
            }"#,
        );
        assert_eq!(
            extract_locked_version(&json, "lodash"),
            Some("4.17.20".to_string())
        );
    }

    #[test]
    fn prefers_top_level_node_modules_entry_over_nested() {
        let json = parse(
            r#"{
                "lockfileVersion": 3,
                "packages": {
                    "node_modules/lodash": { "version": "4.17.21" },
                    "node_modules/foo/node_modules/lodash": { "version": "3.10.1" }
                }
            }"#,
        );
        assert_eq!(
            extract_locked_version(&json, "lodash"),
            Some("4.17.21".to_string())
        );
    }

    #[test]
    fn returns_none_for_unknown_package() {
        let json = parse(
            r#"{
                "lockfileVersion": 3,
                "packages": {
                    "node_modules/lodash": { "version": "4.17.21" }
                }
            }"#,
        );
        assert_eq!(extract_locked_version(&json, "react"), None);
    }

    #[test]
    fn handles_scoped_package_name() {
        let json = parse(
            r#"{
                "lockfileVersion": 3,
                "packages": {
                    "node_modules/@types/node": { "version": "20.10.0" }
                }
            }"#,
        );
        assert_eq!(
            extract_locked_version(&json, "@types/node"),
            Some("20.10.0".to_string())
        );
    }
}
