//! Version matching abstraction for different registries

use crate::parser::types::RegistryType;
use crate::version::semver::{
    CompareResult, calculate_latest_major, calculate_latest_minor, calculate_latest_patch,
};

/// Bump target versions for patch, minor, and major
#[derive(Debug, Default)]
pub struct BumpTargets {
    pub patch: Option<String>,
    pub minor: Option<String>,
    pub major: Option<String>,
}

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

    /// Find the appropriate "latest" version for comparison based on current version context.
    ///
    /// Default: returns latest_version unchanged (existing matchers are unaffected).
    /// Docker: finds the latest version with the same suffix pattern (e.g., `-alpine`).
    fn resolve_latest(
        &self,
        _current_version: &str,
        latest_version: &str,
        _all_versions: &[String],
    ) -> String {
        latest_version.to_string()
    }

    /// Calculate bump targets (patch, minor, major) for code actions.
    ///
    /// Default implementation uses semver-based calculation.
    /// Docker overrides this to handle suffix-aware tag comparison.
    fn calculate_bump_targets(
        &self,
        current_version: &str,
        available_versions: &[String],
    ) -> BumpTargets {
        BumpTargets {
            patch: calculate_latest_patch(current_version, available_versions),
            minor: calculate_latest_minor(current_version, available_versions),
            major: calculate_latest_major(current_version, available_versions),
        }
    }
}
