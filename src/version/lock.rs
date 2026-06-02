//! Lock-file resolution for pinning to versions actually installed in the workspace.

use std::path::PathBuf;

#[cfg(test)]
use mockall::automock;
use tower_lsp::lsp_types::Url;

use crate::parser::types::RegistryType;
use crate::version::error::LockError;

/// Resolves package versions from a manifest's accompanying lock file.
#[cfg_attr(test, automock)]
pub trait LockResolver: Send + Sync {
    fn registry_type(&self) -> RegistryType;

    /// Returns the version pinned in the lock file for `package_name`, if any.
    ///
    /// Returns `Ok(None)` when no lock file is found or the package is not listed.
    fn resolve_locked_version(
        &self,
        manifest_uri: &Url,
        package_name: &str,
    ) -> Result<Option<String>, LockError>;
}

/// Scan a TOML lock file body (as used by Cargo and uv) for the first
/// `[[package]]` block whose `name` matches `package_name` and return its
/// `version` value, if any.
pub fn extract_package_version_from_toml_lock(content: &str, package_name: &str) -> Option<String> {
    let mut in_package = false;
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;

    let finalize =
        |name: &Option<String>, version: &Option<String>, target: &str| -> Option<String> {
            if name.as_deref() == Some(target) {
                version.clone()
            } else {
                None
            }
        };

    for raw_line in content.lines() {
        let line = raw_line.trim();

        if line == "[[package]]" {
            if let Some(v) = finalize(&current_name, &current_version, package_name) {
                return Some(v);
            }
            in_package = true;
            current_name = None;
            current_version = None;
            continue;
        }

        if line.starts_with('[') {
            if let Some(v) = finalize(&current_name, &current_version, package_name) {
                return Some(v);
            }
            in_package = false;
            current_name = None;
            current_version = None;
            continue;
        }

        if !in_package {
            continue;
        }

        if let Some(value) = parse_quoted_value(line, "name") {
            current_name = Some(value);
        } else if let Some(value) = parse_quoted_value(line, "version") {
            current_version = Some(value);
        }
    }

    finalize(&current_name, &current_version, package_name)
}

fn parse_quoted_value(line: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} = \"");
    let rest = line.strip_prefix(&prefix)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Walks up from the manifest's directory looking for `lock_filename`.
pub fn find_lock_file(
    manifest_uri: &Url,
    lock_filename: &str,
) -> Result<Option<PathBuf>, LockError> {
    let manifest_path = manifest_uri
        .to_file_path()
        .map_err(|_| LockError::InvalidManifestUri(manifest_uri.to_string()))?;

    let Some(start) = manifest_path.parent() else {
        return Ok(None);
    };

    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(lock_filename);
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const TOML_LOCK_SAMPLE: &str = r#"# Auto-generated
version = 4

[[package]]
name = "alpha"
version = "1.0.0"
source = "registry+x"

[[package]]
name = "beta"
version = "2.3.4"
"#;

    #[test]
    fn extract_package_version_from_toml_lock_finds_first_package() {
        assert_eq!(
            extract_package_version_from_toml_lock(TOML_LOCK_SAMPLE, "alpha"),
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn extract_package_version_from_toml_lock_finds_last_package() {
        assert_eq!(
            extract_package_version_from_toml_lock(TOML_LOCK_SAMPLE, "beta"),
            Some("2.3.4".to_string())
        );
    }

    #[test]
    fn extract_package_version_from_toml_lock_returns_none_for_unknown() {
        assert_eq!(
            extract_package_version_from_toml_lock(TOML_LOCK_SAMPLE, "gamma"),
            None
        );
    }

    #[test]
    fn extract_package_version_from_toml_lock_returns_first_match_for_duplicates() {
        let content = r#"
[[package]]
name = "foo"
version = "1.0.0"

[[package]]
name = "foo"
version = "2.0.0"
"#;
        assert_eq!(
            extract_package_version_from_toml_lock(content, "foo"),
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn find_lock_file_returns_path_when_in_same_directory() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("package.json");
        let lock = tmp.path().join("package-lock.json");
        fs::write(&manifest, "{}").unwrap();
        fs::write(&lock, "{}").unwrap();

        let uri = Url::from_file_path(&manifest).unwrap();
        let found = find_lock_file(&uri, "package-lock.json").unwrap();

        assert_eq!(found, Some(lock));
    }

    #[test]
    fn find_lock_file_walks_up_to_workspace_root() {
        let tmp = TempDir::new().unwrap();
        let crate_dir = tmp.path().join("crates").join("foo");
        fs::create_dir_all(&crate_dir).unwrap();
        let manifest = crate_dir.join("Cargo.toml");
        let lock = tmp.path().join("Cargo.lock");
        fs::write(&manifest, "").unwrap();
        fs::write(&lock, "").unwrap();

        let uri = Url::from_file_path(&manifest).unwrap();
        let found = find_lock_file(&uri, "Cargo.lock").unwrap();

        assert_eq!(found, Some(lock));
    }

    #[test]
    fn find_lock_file_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("Cargo.toml");
        fs::write(&manifest, "").unwrap();

        let uri = Url::from_file_path(&manifest).unwrap();
        let found = find_lock_file(&uri, "Cargo.lock").unwrap();

        assert_eq!(found, None);
    }
}
