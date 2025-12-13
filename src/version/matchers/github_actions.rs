//! GitHub Actions version matcher
//!
//! Supports partial version matching:
//! - v6 matches v6.0.0, v6.1.0, etc.
//! - v6.1 matches v6.1.0, v6.1.5, etc.
//! - v6.1.0 requires exact match

use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::semver::{compare_versions, version_matches_any, CompareResult};

pub struct GitHubActionsMatcher;

impl VersionMatcher for GitHubActionsMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::GitHubActions
    }

    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool {
        version_matches_any(version_spec, available_versions)
    }

    fn compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult {
        compare_versions(current_version, latest_version)
    }
}
