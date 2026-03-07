//! Docker container image tag version matcher
//!
//! Handles version comparison for Docker image tags with suffix support.
//! Tags like "1.25-alpine" are split into version part "1.25" and suffix "-alpine".
//! The matcher prefers comparing within the same suffix group.

use semver::Version;
use tracing::warn;

use crate::parser::types::RegistryType;
use crate::version::matcher::{BumpTargets, VersionMatcher};
use crate::version::semver::CompareResult;

pub struct DockerVersionMatcher;

impl VersionMatcher for DockerVersionMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Docker
    }

    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool {
        available_versions.iter().any(|v| v == version_spec)
    }

    fn compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult {
        let Some(current_parsed) = parse_docker_tag(current_version) else {
            warn!("Invalid Docker tag format: '{}'", current_version);
            return CompareResult::Invalid;
        };

        let Some(latest_parsed) = parse_docker_tag(latest_version) else {
            warn!("Invalid Docker tag format: '{}'", latest_version);
            return CompareResult::Invalid;
        };

        let parts = count_version_parts(&current_parsed.version_part);

        match parts {
            1 => match current_parsed.semver.major.cmp(&latest_parsed.semver.major) {
                std::cmp::Ordering::Equal => CompareResult::Latest,
                std::cmp::Ordering::Less => CompareResult::Outdated,
                std::cmp::Ordering::Greater => CompareResult::Newer,
            },
            2 => {
                match (current_parsed.semver.major, current_parsed.semver.minor)
                    .cmp(&(latest_parsed.semver.major, latest_parsed.semver.minor))
                {
                    std::cmp::Ordering::Equal => CompareResult::Latest,
                    std::cmp::Ordering::Less => CompareResult::Outdated,
                    std::cmp::Ordering::Greater => CompareResult::Newer,
                }
            }
            _ => match current_parsed.semver.cmp(&latest_parsed.semver) {
                std::cmp::Ordering::Equal => CompareResult::Latest,
                std::cmp::Ordering::Less => CompareResult::Outdated,
                std::cmp::Ordering::Greater => CompareResult::Newer,
            },
        }
    }

    fn calculate_bump_targets(
        &self,
        current_version: &str,
        available_versions: &[String],
    ) -> BumpTargets {
        let Some(current_parsed) = parse_docker_tag(current_version) else {
            return BumpTargets::default();
        };

        let current_suffix = &current_parsed.suffix;

        // Collect versions with the same suffix, parsed into semver
        let same_suffix_versions: Vec<(&str, Version)> = available_versions
            .iter()
            .filter_map(|v| {
                let parsed = parse_docker_tag(v)?;
                if parsed.suffix == *current_suffix {
                    Some((v.as_str(), parsed.semver))
                } else {
                    None
                }
            })
            .collect();

        // patch: same major.minor, higher patch
        let patch = same_suffix_versions
            .iter()
            .filter(|(_, sv)| {
                sv.major == current_parsed.semver.major
                    && sv.minor == current_parsed.semver.minor
                    && *sv > current_parsed.semver
            })
            .max_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(tag, _)| tag.to_string());

        // minor: same major, higher minor
        let minor = same_suffix_versions
            .iter()
            .filter(|(_, sv)| {
                sv.major == current_parsed.semver.major && sv.minor > current_parsed.semver.minor
            })
            .max_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(tag, _)| tag.to_string());

        // major: higher major
        let major = same_suffix_versions
            .iter()
            .filter(|(_, sv)| sv.major > current_parsed.semver.major)
            .max_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(tag, _)| tag.to_string());

        BumpTargets {
            patch,
            minor,
            major,
        }
    }

    fn resolve_latest(
        &self,
        current_version: &str,
        latest_version: &str,
        all_versions: &[String],
    ) -> String {
        let Some(current_parsed) = parse_docker_tag(current_version) else {
            return latest_version.to_string();
        };

        let suffix = &current_parsed.suffix;

        // If no suffix, just return the latest_version as-is
        if suffix.is_empty() {
            return latest_version.to_string();
        }

        // Find the best version with the same suffix
        let best_same_suffix = all_versions
            .iter()
            .filter_map(|v| {
                let parsed = parse_docker_tag(v)?;
                if parsed.suffix == *suffix {
                    Some((v.clone(), parsed))
                } else {
                    None
                }
            })
            .max_by(|(_, a), (_, b)| a.semver.cmp(&b.semver));

        match best_same_suffix {
            Some((tag, parsed)) if parsed.semver > current_parsed.semver => tag,
            _ => latest_version.to_string(),
        }
    }
}

/// Parsed Docker tag components
struct ParsedDockerTag {
    /// The numeric version part (e.g., "1.25.0")
    version_part: String,
    /// The suffix after the version (e.g., "-alpine"), empty if none
    suffix: String,
    /// Parsed semver
    semver: Version,
}

/// Parse a Docker tag into version part and suffix.
///
/// Examples:
/// - "1.25.0-alpine" → version_part="1.25.0", suffix="-alpine"
/// - "16-alpine" → version_part="16", suffix="-alpine"
/// - "v1.0.0" → version_part="1.0.0", suffix=""
/// - "1.25" → version_part="1.25", suffix=""
fn parse_docker_tag(tag: &str) -> Option<ParsedDockerTag> {
    // Strip v/V prefix
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let tag = tag.strip_prefix('V').unwrap_or(tag);

    if tag.is_empty() {
        return None;
    }

    // First char must be a digit
    if !tag.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }

    // Find where the numeric version part ends
    // Version part: digits and dots
    let version_end = tag
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(tag.len());

    let version_part = &tag[..version_end];
    let suffix = &tag[version_end..];

    if version_part.is_empty() {
        return None;
    }

    // Normalize to semver
    let normalized = normalize_to_semver(version_part)?;
    let semver = Version::parse(&normalized).ok()?;

    Some(ParsedDockerTag {
        version_part: version_part.to_string(),
        suffix: suffix.to_string(),
        semver,
    })
}

/// Normalize a version string to X.Y.Z format
fn normalize_to_semver(version: &str) -> Option<String> {
    let parts: Vec<&str> = version.split('.').collect();
    match parts.as_slice() {
        [major] => {
            let _: u64 = major.parse().ok()?;
            Some(format!("{}.0.0", major))
        }
        [major, minor] => {
            let _: u64 = major.parse().ok()?;
            let _: u64 = minor.parse().ok()?;
            Some(format!("{}.{}.0", major, minor))
        }
        [major, minor, patch] => {
            let _: u64 = major.parse().ok()?;
            let _: u64 = minor.parse().ok()?;
            let _: u64 = patch.parse().ok()?;
            Some(format!("{}.{}.{}", major, minor, patch))
        }
        _ => None,
    }
}

/// Count how many version parts were specified
fn count_version_parts(version: &str) -> usize {
    version.split('.').count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("1.25.0-alpine", "1.25.0", "-alpine")]
    #[case("16-alpine", "16", "-alpine")]
    #[case("v1.0.0", "1.0.0", "")]
    #[case("1.25", "1.25", "")]
    #[case("15", "15", "")]
    #[case("1.25.0-alpine3.18", "1.25.0", "-alpine3.18")]
    fn parse_docker_tag_extracts_version_and_suffix(
        #[case] tag: &str,
        #[case] expected_version: &str,
        #[case] expected_suffix: &str,
    ) {
        let parsed = parse_docker_tag(tag).unwrap();
        assert_eq!(parsed.version_part, expected_version);
        assert_eq!(parsed.suffix, expected_suffix);
    }

    #[rstest]
    #[case("latest")]
    #[case("alpine")]
    #[case("stable")]
    #[case("")]
    fn parse_docker_tag_returns_none_for_non_numeric(#[case] tag: &str) {
        assert!(parse_docker_tag(tag).is_none());
    }

    #[rstest]
    // Same version → Latest
    #[case("1.25", "1.25", CompareResult::Latest)]
    #[case("1.25.0", "1.25.0", CompareResult::Latest)]
    #[case("15", "15", CompareResult::Latest)]
    // Outdated
    #[case("1.25", "1.27", CompareResult::Outdated)]
    #[case("15", "17", CompareResult::Outdated)]
    #[case("1.25.0", "1.27.0", CompareResult::Outdated)]
    // Newer
    #[case("1.27", "1.25", CompareResult::Newer)]
    #[case("17", "15", CompareResult::Newer)]
    // Suffixed tags - only numeric part compared
    #[case("1.25-alpine", "1.27-alpine", CompareResult::Outdated)]
    #[case("1.27-alpine", "1.27-alpine", CompareResult::Latest)]
    // v prefix
    #[case("v1.0.0", "v2.0.0", CompareResult::Outdated)]
    // Partial version matching
    #[case("15", "17", CompareResult::Outdated)]
    #[case("15", "15", CompareResult::Latest)]
    // Invalid
    #[case("latest", "1.25", CompareResult::Invalid)]
    #[case("1.25", "latest", CompareResult::Invalid)]
    fn compare_to_latest_returns_expected(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] expected: CompareResult,
    ) {
        let matcher = DockerVersionMatcher;
        assert_eq!(matcher.compare_to_latest(current, latest), expected);
    }

    #[test]
    fn resolve_latest_returns_same_suffix_version_when_available() {
        let matcher = DockerVersionMatcher;
        let all_versions = vec![
            "1.25".to_string(),
            "1.25-alpine".to_string(),
            "1.27".to_string(),
            "1.27-alpine".to_string(),
        ];
        assert_eq!(
            matcher.resolve_latest("1.25-alpine", "1.27", &all_versions),
            "1.27-alpine"
        );
    }

    #[test]
    fn resolve_latest_falls_back_to_latest_when_no_same_suffix() {
        let matcher = DockerVersionMatcher;
        let all_versions = vec![
            "1.25".to_string(),
            "1.25-alpine".to_string(),
            "1.27".to_string(),
        ];
        // No "1.27-alpine" exists
        assert_eq!(
            matcher.resolve_latest("1.25-alpine", "1.27", &all_versions),
            "1.27"
        );
    }

    #[test]
    fn resolve_latest_returns_latest_when_no_suffix() {
        let matcher = DockerVersionMatcher;
        let all_versions = vec!["1.25".to_string(), "1.27".to_string()];
        assert_eq!(
            matcher.resolve_latest("1.25", "1.27", &all_versions),
            "1.27"
        );
    }

    #[test]
    fn resolve_latest_falls_back_when_same_suffix_is_not_newer() {
        let matcher = DockerVersionMatcher;
        let all_versions = vec!["1.27".to_string(), "1.27-alpine".to_string()];
        // Current is already at the max for that suffix
        assert_eq!(
            matcher.resolve_latest("1.27-alpine", "1.27", &all_versions),
            "1.27"
        );
    }

    #[test]
    fn version_exists_checks_exact_match() {
        let matcher = DockerVersionMatcher;
        let versions = vec!["1.25".to_string(), "1.25-alpine".to_string()];
        assert!(matcher.version_exists("1.25", &versions));
        assert!(matcher.version_exists("1.25-alpine", &versions));
        assert!(!matcher.version_exists("1.26", &versions));
    }

    #[test]
    fn calculate_bump_targets_returns_same_suffix_versions_when_suffixed() {
        let matcher = DockerVersionMatcher;
        let versions = vec![
            "1.25".to_string(),
            "1.25-alpine".to_string(),
            "1.27".to_string(),
            "1.27-alpine".to_string(),
            "2.0".to_string(),
            "2.0-alpine".to_string(),
        ];
        let targets = matcher.calculate_bump_targets("1.25-alpine", &versions);
        assert_eq!(targets.patch, None);
        assert_eq!(targets.minor, Some("1.27-alpine".to_string()));
        assert_eq!(targets.major, Some("2.0-alpine".to_string()));
    }

    #[test]
    fn calculate_bump_targets_returns_versions_when_no_suffix() {
        let matcher = DockerVersionMatcher;
        let versions = vec!["1.25".to_string(), "1.27".to_string(), "2.0".to_string()];
        let targets = matcher.calculate_bump_targets("1.25", &versions);
        assert_eq!(targets.patch, None);
        assert_eq!(targets.minor, Some("1.27".to_string()));
        assert_eq!(targets.major, Some("2.0".to_string()));
    }

    #[test]
    fn calculate_bump_targets_returns_default_when_invalid_tag() {
        let matcher = DockerVersionMatcher;
        let versions = vec!["1.25".to_string(), "1.27".to_string()];
        let targets = matcher.calculate_bump_targets("latest", &versions);
        assert_eq!(targets.patch, None);
        assert_eq!(targets.minor, None);
        assert_eq!(targets.major, None);
    }
}
