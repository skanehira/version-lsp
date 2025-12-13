//! Version matching abstraction for different registries

use crate::parser::types::RegistryType;
use crate::version::semver::CompareResult;

/// Trait for registry-specific version matching logic
///
/// Each registry has different version matching rules:
/// - GitHub Actions: partial version matching (v6 matches v6.x.x)
/// - npm: range specifications (^1.0.0, ~1.0.0, etc.)
pub trait VersionMatcher: Send + Sync {
    /// Returns the registry type this matcher handles
    fn registry_type(&self) -> RegistryType;

    /// Check if a version specification matches any available version
    ///
    /// For GitHub Actions: v6 matches v6.0.0, v6.1.0, etc.
    /// For npm: ^1.0.0 matches 1.0.0, 1.1.0, 1.9.9, but not 2.0.0
    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool;

    /// Compare the current version specification to the latest version
    ///
    /// Returns whether the current version is latest, outdated, newer, or invalid
    fn compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult;
}
