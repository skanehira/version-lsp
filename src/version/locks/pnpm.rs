//! Resolves locked versions from `pnpm-lock.yaml`.

use std::fs;

use serde_yaml_ng::Value;
use tower_lsp::lsp_types::Url;
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::error::LockError;
use crate::version::lock::{LockResolver, find_lock_file};

const LOCK_FILE_NAME: &str = "pnpm-lock.yaml";
const DEPENDENCY_SECTIONS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
];

pub struct PnpmLockResolver;

impl LockResolver for PnpmLockResolver {
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

        let yaml: Value = serde_yaml_ng::from_str(&content)
            .map_err(|e| LockError::InvalidFormat(e.to_string()))?;

        Ok(extract_locked_version(&yaml, package_name))
    }
}

/// Look up `package_name` in any importer's dependency sections, then fall back
/// to the global `packages` table. The returned version has any peer-dep suffix
/// (e.g. `(react@18.0.0)`) stripped.
fn extract_locked_version(yaml: &Value, package_name: &str) -> Option<String> {
    if let Some(importers) = yaml.get("importers").and_then(Value::as_mapping) {
        for (_, importer) in importers {
            for section in DEPENDENCY_SECTIONS {
                if let Some(version) = importer
                    .get(section)
                    .and_then(|deps| deps.get(package_name))
                    .and_then(|entry| entry.get("version"))
                    .and_then(Value::as_str)
                {
                    return Some(strip_peer_suffix(version).to_string());
                }
            }
        }
    }

    if let Some(packages) = yaml.get("packages").and_then(Value::as_mapping) {
        for (key, _) in packages {
            let Some(key_str) = key.as_str() else {
                continue;
            };
            if let Some(version) = match_package_key(key_str, package_name) {
                return Some(version.to_string());
            }
        }
    }

    None
}

/// `lodash@4.17.21` -> `4.17.21` when name matches; handles scoped names like
/// `@types/node@20.10.0` by splitting on the last `@`.
fn match_package_key<'a>(key: &'a str, package_name: &str) -> Option<&'a str> {
    let split_at = key.rfind('@').filter(|&i| i > 0)?;
    let (name, version_part) = key.split_at(split_at);
    if name != package_name {
        return None;
    }
    Some(strip_peer_suffix(&version_part[1..]))
}

fn strip_peer_suffix(version: &str) -> &str {
    match version.find('(') {
        Some(idx) => &version[..idx],
        None => version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(content: &str) -> Value {
        serde_yaml_ng::from_str(content).unwrap()
    }

    #[test]
    fn extracts_version_from_root_importer_dependencies() {
        let yaml = parse(
            r#"
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      lodash:
        specifier: ^4.17.0
        version: 4.17.21
"#,
        );
        assert_eq!(
            extract_locked_version(&yaml, "lodash"),
            Some("4.17.21".to_string())
        );
    }

    #[test]
    fn extracts_version_from_dev_dependencies() {
        let yaml = parse(
            r#"
importers:
  .:
    devDependencies:
      typescript:
        specifier: ^5.0.0
        version: 5.4.2
"#,
        );
        assert_eq!(
            extract_locked_version(&yaml, "typescript"),
            Some("5.4.2".to_string())
        );
    }

    #[test]
    fn strips_peer_dep_suffix() {
        let yaml = parse(
            r#"
importers:
  .:
    dependencies:
      react-dom:
        specifier: ^18.0.0
        version: 18.2.0(react@18.2.0)
"#,
        );
        assert_eq!(
            extract_locked_version(&yaml, "react-dom"),
            Some("18.2.0".to_string())
        );
    }

    #[test]
    fn extracts_version_from_workspace_importer() {
        let yaml = parse(
            r#"
importers:
  packages/app:
    dependencies:
      lodash:
        specifier: ^4.17.0
        version: 4.17.21
"#,
        );
        assert_eq!(
            extract_locked_version(&yaml, "lodash"),
            Some("4.17.21".to_string())
        );
    }

    #[test]
    fn falls_back_to_packages_table() {
        let yaml = parse(
            r#"
packages:
  lodash@4.17.21:
    resolution: {integrity: sha512-fake}
  '@types/node@20.10.0':
    resolution: {integrity: sha512-fake}
"#,
        );
        assert_eq!(
            extract_locked_version(&yaml, "lodash"),
            Some("4.17.21".to_string())
        );
        assert_eq!(
            extract_locked_version(&yaml, "@types/node"),
            Some("20.10.0".to_string())
        );
    }

    #[test]
    fn returns_none_for_unknown_package() {
        let yaml = parse(
            r#"
importers:
  .:
    dependencies:
      lodash:
        specifier: ^4.17.0
        version: 4.17.21
"#,
        );
        assert_eq!(extract_locked_version(&yaml, "react"), None);
    }
}
