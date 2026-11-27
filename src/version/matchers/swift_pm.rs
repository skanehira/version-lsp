//! Swift Package Manager version matcher
//!
//! SPM dependencies use semver constraints. The semantic of a bare `from: "1.2.3"`
//! is "compatible with 1.2.3" — identical to Cargo's bare `1.2.3` (caret-like).
//! `exact:` declares an exact version, and `.upToNextMajor`/`.upToNextMinor`
//! are caret/tilde respectively. Since the parser stores only the version
//! literal (no operator prefix), we delegate to the Cargo matcher whose
//! default-caret behavior matches SPM's `from:` semantics.

use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::matchers::CratesVersionMatcher;
use crate::version::semver::CompareResult;

pub struct SwiftPmVersionMatcher;

impl VersionMatcher for SwiftPmVersionMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::SwiftPm
    }

    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool {
        // GitHub Releases tag names typically use a `v` prefix (e.g. `v4.92.0`).
        // SPM constraint versions are bare (e.g. `4.92.0`). Strip the `v` prefix
        // from available versions so the Cargo matcher's semver comparison works.
        let normalized = strip_v_prefix(available_versions);
        CratesVersionMatcher.version_exists(version_spec, &normalized)
    }

    fn compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult {
        let latest_stripped = latest_version
            .strip_prefix('v')
            .unwrap_or(latest_version)
            .to_string();
        CratesVersionMatcher.compare_to_latest(current_version, &latest_stripped)
    }

    /// Strip the `v` prefix from the latest version so diagnostics show
    /// `Update available: 4.90.0 -> 5.0.0` instead of `... -> v5.0.0`,
    /// matching the form users write in Package.swift.
    fn resolve_latest(
        &self,
        _current_version: &str,
        latest_version: &str,
        _all_versions: &[String],
    ) -> String {
        latest_version
            .strip_prefix('v')
            .unwrap_or(latest_version)
            .to_string()
    }
}

fn strip_v_prefix(versions: &[String]) -> Vec<String> {
    versions
        .iter()
        .map(|v| v.strip_prefix('v').unwrap_or(v).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_type_returns_swift_pm() {
        assert_eq!(SwiftPmVersionMatcher.registry_type(), RegistryType::SwiftPm);
    }

    #[test]
    fn version_exists_matches_bare_against_v_prefixed_tags() {
        let tags = vec![
            "v1.0.0".to_string(),
            "v1.2.3".to_string(),
            "v2.0.0".to_string(),
        ];
        assert!(SwiftPmVersionMatcher.version_exists("1.2.3", &tags));
        assert!(SwiftPmVersionMatcher.version_exists("1.0.0", &tags));
        assert!(!SwiftPmVersionMatcher.version_exists("9.9.9", &tags));
    }

    #[test]
    fn version_exists_matches_bare_against_bare_tags() {
        let tags = vec!["1.0.0".to_string(), "1.2.3".to_string()];
        assert!(SwiftPmVersionMatcher.version_exists("1.2.3", &tags));
    }

    #[test]
    fn compare_to_latest_treats_bare_as_caret_like_cargo() {
        // Bare `1.0.0` should be considered compatible with `1.5.0`
        // (caret semantics, same as Cargo's default behavior).
        assert_eq!(
            SwiftPmVersionMatcher.compare_to_latest("1.0.0", "1.5.0"),
            CompareResult::Latest
        );
    }

    #[test]
    fn compare_to_latest_flags_outdated_when_major_lags() {
        assert_eq!(
            SwiftPmVersionMatcher.compare_to_latest("1.0.0", "2.0.0"),
            CompareResult::Outdated
        );
    }

    #[test]
    fn compare_to_latest_strips_v_prefix_from_latest() {
        assert_eq!(
            SwiftPmVersionMatcher.compare_to_latest("4.92.0", "v4.92.0"),
            CompareResult::Latest
        );
    }

    #[test]
    fn resolve_latest_strips_v_prefix() {
        let m = SwiftPmVersionMatcher;
        assert_eq!(m.resolve_latest("1.0.0", "v2.5.0", &[]), "2.5.0");
        assert_eq!(m.resolve_latest("1.0.0", "2.5.0", &[]), "2.5.0");
    }
}
