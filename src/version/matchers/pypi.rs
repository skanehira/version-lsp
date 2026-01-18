//! PyPI version matcher using PEP 440 version specifiers

use std::str::FromStr;

use pep508_rs::pep440_rs::{Version, VersionSpecifiers};
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::semver::CompareResult;

/// Version matcher for PyPI packages using PEP 440 specifiers
pub struct PypiVersionMatcher;

impl VersionMatcher for PypiVersionMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::PyPI
    }

    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool {
        // Empty spec matches any version
        if version_spec.is_empty() {
            return !available_versions.is_empty();
        }

        // Parse the version specifiers
        let Ok(specifiers) = VersionSpecifiers::from_str(version_spec).inspect_err(|e| {
            warn!(
                "Failed to parse version specifiers '{}': {}",
                version_spec, e
            );
        }) else {
            return false;
        };

        // Check if any available version satisfies the specification
        available_versions.iter().any(|v| {
            Version::from_str(v)
                .map(|ver| specifiers.contains(&ver))
                .unwrap_or(false)
        })
    }

    fn compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult {
        // Empty spec is always satisfied by latest
        if current_version.is_empty() {
            return CompareResult::Latest;
        }

        // Parse the latest version
        let Ok(latest) = Version::from_str(latest_version).inspect_err(|e| {
            warn!("Failed to parse latest version '{}': {}", latest_version, e);
        }) else {
            return CompareResult::Invalid;
        };

        // Parse the version specifiers
        let Ok(specifiers) = VersionSpecifiers::from_str(current_version).inspect_err(|e| {
            warn!(
                "Failed to parse version specifiers '{}': {}",
                current_version, e
            );
        }) else {
            return CompareResult::Invalid;
        };

        // Check if the latest version satisfies the specification
        if specifiers.contains(&latest) {
            return CompareResult::Latest;
        }

        // If latest doesn't satisfy the spec, try to determine if we're outdated or newer
        // Extract the base version from the first specifier for comparison
        let spec_str = current_version.trim();

        // Try to extract a version number for comparison
        let base_version_str = extract_base_version(spec_str);

        let Some(base) = base_version_str.and_then(|s| Version::from_str(s).ok()) else {
            // Can't determine base version, assume outdated since latest doesn't satisfy
            return CompareResult::Outdated;
        };

        if base <= latest {
            CompareResult::Outdated
        } else {
            CompareResult::Newer
        }
    }
}

/// Extract the base version from a PEP 440 version specifier
fn extract_base_version(spec: &str) -> Option<&str> {
    let spec = spec.trim();

    // Handle operators: >=, <=, ==, !=, ~=, >, <
    let operators = [">=", "<=", "==", "!=", "~=", ">", "<"];

    for op in operators {
        if let Some(rest) = spec.strip_prefix(op) {
            // Handle comma-separated specs (e.g., ">=1.0, <2.0")
            let version_part = rest.split(',').next()?.trim();
            return Some(version_part);
        }
    }

    // If no operator, assume it's a bare version
    Some(spec.split(',').next()?.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // version_exists tests - basic operators
    #[rstest]
    #[case(">=2.28.0", vec!["2.28.0", "2.32.0"], true)]
    #[case(">=2.28.0", vec!["2.27.0"], false)]
    #[case("<=2.0.0", vec!["1.9.0", "2.0.0"], true)]
    #[case("<=2.0.0", vec!["2.0.1"], false)]
    #[case(">1.0.0", vec!["1.0.1", "2.0.0"], true)]
    #[case(">1.0.0", vec!["1.0.0", "0.9.0"], false)]
    #[case("<2.0.0", vec!["1.9.0", "1.0.0"], true)]
    #[case("<2.0.0", vec!["2.0.0", "3.0.0"], false)]
    #[case("==2.0.0", vec!["2.0.0"], true)]
    #[case("==2.0.0", vec!["2.0.1", "1.9.0"], false)]
    #[case("!=2.0.0", vec!["2.0.1", "1.9.0"], true)]
    #[case("!=2.0.0", vec!["2.0.0"], false)]
    fn version_exists_basic_operators(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            PypiVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - compatible release (~=)
    #[rstest]
    #[case("~=1.4.2", vec!["1.4.2", "1.4.5"], true)]
    #[case("~=1.4.2", vec!["1.5.0", "2.0.0"], false)]
    #[case("~=1.4", vec!["1.4.0", "1.5.0", "1.9.0"], true)]
    #[case("~=1.4", vec!["2.0.0"], false)]
    fn version_exists_compatible_release(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            PypiVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - compound specifiers
    #[rstest]
    #[case(">=2.0, <3.0", vec!["2.5.0"], true)]
    #[case(">=2.0, <3.0", vec!["3.0.0"], false)]
    #[case(">=2.0, <3.0", vec!["1.9.0"], false)]
    #[case(">=1.0, !=1.5.0", vec!["1.0.0", "1.4.0"], true)]
    #[case(">=1.0, !=1.5.0", vec!["1.5.0"], false)]
    fn version_exists_compound_specifiers(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            PypiVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - empty and edge cases
    #[test]
    fn version_exists_with_empty_spec_returns_true_if_versions_available() {
        let available = vec!["1.0.0".to_string()];
        assert!(PypiVersionMatcher.version_exists("", &available));
    }

    #[test]
    fn version_exists_with_empty_spec_returns_false_if_no_versions() {
        let available: Vec<String> = vec![];
        assert!(!PypiVersionMatcher.version_exists("", &available));
    }

    #[test]
    fn version_exists_with_invalid_spec_returns_false() {
        let available = vec!["1.0.0".to_string()];
        assert!(!PypiVersionMatcher.version_exists("invalid>>=spec", &available));
    }

    // compare_to_latest tests
    #[rstest]
    #[case(">=2.28.0", "2.32.0", CompareResult::Latest)]
    #[case(">=2.28.0", "2.28.0", CompareResult::Latest)]
    #[case(">=2.28.0", "2.27.0", CompareResult::Newer)]
    #[case(">=2.0, <3.0", "2.5.0", CompareResult::Latest)]
    #[case(">=2.0, <3.0", "3.0.0", CompareResult::Outdated)]
    #[case("~=1.4.2", "1.4.5", CompareResult::Latest)]
    #[case("~=1.4.2", "1.5.0", CompareResult::Outdated)]
    #[case("==2.0.0", "2.0.0", CompareResult::Latest)]
    #[case("==2.0.0", "2.0.1", CompareResult::Outdated)]
    #[case("!=2.0.0", "2.0.1", CompareResult::Latest)]
    #[case(">1.0", "1.1.0", CompareResult::Latest)]
    #[case(">1.0", "0.9.0", CompareResult::Newer)]
    #[case("<2.0", "1.9.0", CompareResult::Latest)]
    #[case("<2.0", "2.0.0", CompareResult::Outdated)]
    #[case("<=2.0", "2.0.0", CompareResult::Latest)]
    #[case("<=2.0", "2.0.1", CompareResult::Outdated)]
    fn compare_to_latest_returns_expected(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] expected: CompareResult,
    ) {
        assert_eq!(
            PypiVersionMatcher.compare_to_latest(current, latest),
            expected
        );
    }

    #[test]
    fn compare_to_latest_with_empty_spec_returns_latest() {
        assert_eq!(
            PypiVersionMatcher.compare_to_latest("", "1.0.0"),
            CompareResult::Latest
        );
    }

    #[test]
    fn compare_to_latest_with_invalid_spec_returns_invalid() {
        assert_eq!(
            PypiVersionMatcher.compare_to_latest("invalid>>=spec", "1.0.0"),
            CompareResult::Invalid
        );
    }

    #[test]
    fn compare_to_latest_with_invalid_latest_version_returns_invalid() {
        assert_eq!(
            PypiVersionMatcher.compare_to_latest(">=1.0.0", "not-a-version"),
            CompareResult::Invalid
        );
    }

    #[test]
    fn registry_type_returns_pypi() {
        assert_eq!(PypiVersionMatcher.registry_type(), RegistryType::PyPI);
    }
}
