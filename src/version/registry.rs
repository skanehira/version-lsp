//! Registry trait for fetching package versions from various sources

#[cfg(test)]
use mockall::automock;

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::types::PackageVersions;

/// Trait for fetching package versions from a registry
#[cfg_attr(test, automock)]
#[async_trait::async_trait]
pub trait Registry: Send + Sync {
    /// Returns the type of registry this implementation handles
    fn registry_type(&self) -> RegistryType;

    /// Fetches all versions for a package from the registry
    ///
    /// # Arguments
    /// * `package_name` - The name of the package (e.g., "actions/checkout" for GitHub Actions)
    ///
    /// # Returns
    /// * `Ok(PackageVersions)` - List of versions, ordered from newest to oldest
    /// * `Err(RegistryError)` - If the fetch fails
    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError>;
}
