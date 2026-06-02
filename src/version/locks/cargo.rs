//! Resolves locked versions from `Cargo.lock`.

use std::fs;

use tower_lsp::lsp_types::Url;
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::error::LockError;
use crate::version::lock::{LockResolver, extract_package_version_from_toml_lock, find_lock_file};

const LOCK_FILE_NAME: &str = "Cargo.lock";

pub struct CargoLockResolver;

impl LockResolver for CargoLockResolver {
    fn registry_type(&self) -> RegistryType {
        RegistryType::CratesIo
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
