//! Go module version matcher
//!
//! Go modules use exact version matching with optional suffixes:
//! - Standard semver: v1.2.3
//! - +incompatible suffix: v2.0.0+incompatible (pre-go.mod v2+ modules)
//! - Pseudo-versions: v0.0.0-20210101000000-abcdef123456

use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::semver::CompareResult;
use semver::Version;
use tracing::warn;

pub struct GoVersionMatcher;

impl VersionMatcher for GoVersionMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::GoProxy
    }

    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool {
        // Pseudo-versions are commit-specific and not listed in /@v/list
        // Skip validation for them
        if is_pseudo_version(version_spec) {
            return true;
        }

        // Go modules use exact version matching
        // Normalize both versions for comparison
        let normalized = normalize_go_version(version_spec);

        available_versions
            .iter()
            .any(|v| normalize_go_version(v) == normalized)
    }

    fn compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult {
        compare_go_versions(current_version, latest_version)
    }
}

/// Normalize a Go module version for comparison.
///
/// Handles:
/// - v prefix: v1.2.3 -> 1.2.3
/// - +incompatible suffix: v2.0.0+incompatible -> 2.0.0
fn normalize_go_version(version: &str) -> String {
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = version.strip_suffix("+incompatible").unwrap_or(version);
    version.to_string()
}

/// Check if a version is a pseudo-version.
///
/// Pseudo-version formats:
/// - v0.0.0-YYYYMMDDHHMMSS-commit (no base version)
/// - vX.Y.Z-0.YYYYMMDDHHMMSS-commit (with base version)
fn is_pseudo_version(version: &str) -> bool {
    let normalized = normalize_go_version(version);

    let Some((_, rest)) = normalized.split_once('-') else {
        return false;
    };

    let parts: Vec<&str> = rest.split('-').collect();
    if parts.len() < 2 {
        return false;
    }

    // Check for timestamp: either direct (14 digits) or prefixed with "0." (16 chars)
    let timestamp = parts[0];
    if timestamp.len() == 14 && timestamp.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if timestamp.starts_with("0.") && timestamp.len() == 16 {
        let after_prefix = &timestamp[2..];
        if after_prefix.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }

    false
}

/// Parse a Go version into semver::Version
///
/// Handles pseudo-versions by extracting the timestamp for comparison.
fn parse_go_version(version: &str) -> Option<(Version, Option<String>)> {
    let normalized = normalize_go_version(version);

    // Check for pseudo-version: v0.0.0-YYYYMMDDHHMMSS-commit
    if let Some((base, rest)) = normalized.split_once('-') {
        // Check if it looks like a pseudo-version (timestamp + commit)
        let parts: Vec<&str> = rest.split('-').collect();
        if parts.len() >= 2 && parts[0].len() == 14 && parts[0].chars().all(|c| c.is_ascii_digit())
        {
            // It's a pseudo-version, use the timestamp for sorting
            let parsed = Version::parse(base).ok()?;
            return Some((parsed, Some(parts[0].to_string())));
        }

        // Regular pre-release version
        let full = format!("{}-{}", base, rest);
        let parsed = Version::parse(&full).ok()?;
        return Some((parsed, None));
    }

    let parsed = Version::parse(&normalized).ok()?;
    Some((parsed, None))
}

/// Compare two Go module versions
fn compare_go_versions(current: &str, latest: &str) -> CompareResult {
    let Some((current_ver, current_timestamp)) = parse_go_version(current) else {
        warn!("Invalid Go version format: '{}'", current);
        return CompareResult::Invalid;
    };

    let Some((latest_ver, latest_timestamp)) = parse_go_version(latest) else {
        warn!("Invalid Go version format: '{}'", latest);
        return CompareResult::Invalid;
    };

    // First compare by semver
    match current_ver.cmp(&latest_ver) {
        std::cmp::Ordering::Less => CompareResult::Outdated,
        std::cmp::Ordering::Greater => CompareResult::Newer,
        std::cmp::Ordering::Equal => {
            // If semver is equal, compare by timestamp (for pseudo-versions)
            match (current_timestamp, latest_timestamp) {
                (Some(ct), Some(lt)) => match ct.cmp(&lt) {
                    std::cmp::Ordering::Equal => CompareResult::Latest,
                    std::cmp::Ordering::Less => CompareResult::Outdated,
                    std::cmp::Ordering::Greater => CompareResult::Newer,
                },
                (None, Some(_)) => CompareResult::Outdated, // Regular version vs pseudo
                (Some(_), None) => CompareResult::Newer,    // Pseudo vs regular
                (None, None) => CompareResult::Latest,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("v1.0.0", "v1.0.0", CompareResult::Latest)]
    #[case("v1.0.0", "v2.0.0", CompareResult::Outdated)]
    #[case("v2.0.0", "v1.0.0", CompareResult::Newer)]
    #[case("v1.0.0", "v1.0.1", CompareResult::Outdated)]
    #[case("v1.0.1", "v1.0.0", CompareResult::Newer)]
    // With +incompatible suffix
    #[case("v2.0.0+incompatible", "v2.0.0+incompatible", CompareResult::Latest)]
    #[case("v2.0.0+incompatible", "v3.0.0+incompatible", CompareResult::Outdated)]
    #[case("v2.0.0+incompatible", "v2.0.0", CompareResult::Latest)]
    #[case("v2.0.0", "v2.0.0+incompatible", CompareResult::Latest)]
    // Pre-release versions
    #[case("v1.0.0-beta.1", "v1.0.0", CompareResult::Outdated)]
    #[case("v1.0.0", "v1.0.0-beta.1", CompareResult::Newer)]
    #[case("v1.0.0-alpha", "v1.0.0-beta", CompareResult::Outdated)]
    // Pseudo-versions
    #[case(
        "v0.0.0-20210101000000-abc123",
        "v0.0.0-20210201000000-def456",
        CompareResult::Outdated
    )]
    #[case(
        "v0.0.0-20210201000000-def456",
        "v0.0.0-20210101000000-abc123",
        CompareResult::Newer
    )]
    #[case(
        "v0.0.0-20210101000000-abc123",
        "v0.0.0-20210101000000-abc123",
        CompareResult::Latest
    )]
    fn compare_to_latest_returns_expected(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] expected: CompareResult,
    ) {
        let matcher = GoVersionMatcher;
        assert_eq!(matcher.compare_to_latest(current, latest), expected);
    }

    #[rstest]
    #[case("v1.0.0", &["v1.0.0", "v1.1.0"], true)]
    #[case("v1.0.0", &["v1.1.0", "v2.0.0"], false)]
    #[case("v2.0.0+incompatible", &["v2.0.0+incompatible", "v3.0.0"], true)]
    #[case("v2.0.0+incompatible", &["v2.0.0"], true)] // +incompatible matches without suffix
    #[case("v2.0.0", &["v2.0.0+incompatible"], true)]
    // without suffix matches +incompatible
    // Pseudo-versions should always return true (skip validation)
    #[case("v0.0.0-20210101000000-abc123", &["v1.0.0", "v2.0.0"], true)]
    #[case("v1.1.3-0.20240916144458-20a13a1f6b7c", &["v1.0.0", "v1.1.3"], true)]
    fn version_exists_returns_expected(
        #[case] version: &str,
        #[case] available: &[&str],
        #[case] expected: bool,
    ) {
        let matcher = GoVersionMatcher;
        let available: Vec<String> = available.iter().map(|s| s.to_string()).collect();
        assert_eq!(matcher.version_exists(version, &available), expected);
    }

    #[test]
    fn normalize_go_version_strips_prefix_and_suffix() {
        assert_eq!(normalize_go_version("v1.0.0"), "1.0.0");
        assert_eq!(normalize_go_version("v2.0.0+incompatible"), "2.0.0");
        assert_eq!(normalize_go_version("1.0.0"), "1.0.0");
    }

    #[rstest]
    // Pseudo-versions without base version
    #[case("v0.0.0-20210101000000-abc123", true)]
    #[case("v0.0.0-20240916144458-20a13a1f6b7c", true)]
    // Pseudo-versions with base version (0.timestamp format)
    #[case("v1.1.3-0.20240916144458-20a13a1f6b7c", true)]
    #[case("v2.0.0-0.20200101120000-abcdef123456", true)]
    // Regular versions (not pseudo)
    #[case("v1.0.0", false)]
    #[case("v1.0.0-beta.1", false)]
    #[case("v1.0.0-alpha", false)]
    #[case("v2.0.0+incompatible", false)]
    fn is_pseudo_version_returns_expected(#[case] version: &str, #[case] expected: bool) {
        assert_eq!(is_pseudo_version(version), expected);
    }
}
