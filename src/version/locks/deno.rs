//! Resolves locked versions from `deno.lock`.

use std::fs;

use serde_json::Value;
use tower_lsp::lsp_types::Url;
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::error::LockError;
use crate::version::lock::{LockResolver, find_lock_file};

const LOCK_FILE_NAME: &str = "deno.lock";

pub struct DenoLockResolver;

impl LockResolver for DenoLockResolver {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Jsr
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

/// `deno.lock` (v3/v4) maps each spec like `jsr:@std/fs@^1` to its resolved
/// version. We scan `specifiers` for keys starting with `jsr:<package_name>@`
/// and return the resolved value with any peer-dep suffix (`_react@18.2.0`)
/// stripped.
fn extract_locked_version(json: &Value, package_name: &str) -> Option<String> {
    let specifiers = json.get("specifiers").and_then(Value::as_object)?;
    let prefix = format!("jsr:{package_name}@");

    for (key, value) in specifiers {
        if !key.starts_with(&prefix) {
            continue;
        }
        let resolved = value.as_str()?;
        return Some(strip_peer_suffix(resolved).to_string());
    }
    None
}

fn strip_peer_suffix(version: &str) -> &str {
    match version.find('_') {
        Some(idx) => &version[..idx],
        None => version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(content: &str) -> Value {
        serde_json::from_str(content).unwrap()
    }

    #[test]
    fn extracts_jsr_version_from_specifiers() {
        let json = parse(
            r#"{
                "version": "4",
                "specifiers": {
                    "jsr:@std/fs@^1": "1.0.5",
                    "jsr:@std/path@^1": "1.0.6",
                    "npm:lodash@^4": "4.17.21"
                }
            }"#,
        );
        assert_eq!(
            extract_locked_version(&json, "@std/fs"),
            Some("1.0.5".to_string())
        );
    }

    #[test]
    fn strips_peer_dep_suffix() {
        let json = parse(
            r#"{
                "version": "4",
                "specifiers": {
                    "jsr:@scope/pkg@^1": "1.2.3_other@4.5.6"
                }
            }"#,
        );
        assert_eq!(
            extract_locked_version(&json, "@scope/pkg"),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn ignores_npm_specifiers_for_jsr_lookup() {
        let json = parse(
            r#"{
                "specifiers": {
                    "npm:@std/fs@^1": "9.9.9"
                }
            }"#,
        );
        assert_eq!(extract_locked_version(&json, "@std/fs"), None);
    }

    #[test]
    fn returns_none_when_specifiers_missing() {
        let json = parse(r#"{ "version": "4" }"#);
        assert_eq!(extract_locked_version(&json, "@std/fs"), None);
    }
}
