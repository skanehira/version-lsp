//! Crates.io version matcher
//!
//! Supports Cargo version requirement specifications:
//! - `1.2.3` - default (caret-like): >=1.2.3 <2.0.0 (or special cases for 0.x)
//! - `^1.2.3` - explicit caret (same as default)
//! - `~1.2.3` - tilde: >=1.2.3 <1.3.0
//! - `>=1.2.3`, `>1.2.3`, `<=1.2.3`, `<1.2.3`, `=1.2.3` - comparison operators
//! - `1.2.*`, `1.*`, `*` - wildcards

use semver::Version;

use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::semver::{CompareResult, parse_version};

pub struct CratesVersionMatcher;

/// Represents a parsed Cargo version requirement
#[derive(Debug)]
enum VersionRequirement {
    /// Default/Caret: ^1.2.3 or 1.2.3 means >=1.2.3 <2.0.0 (special handling for 0.x)
    Caret(Version),
    /// Tilde: ~1.2.3 means >=1.2.3 <1.3.0
    Tilde(Version),
    /// Exact: =1.2.3 means exactly 1.2.3
    Exact(Version),
    /// Greater than or equal
    Gte(Version),
    /// Greater than
    Gt(Version),
    /// Less than or equal
    Lte(Version),
    /// Less than
    Lt(Version),
    /// Any version: * matches all versions
    Any,
    /// Wildcard major: 1.* means >=1.0.0 <2.0.0
    WildcardMajor(u64),
    /// Wildcard minor: 1.2.* means >=1.2.0 <1.3.0
    WildcardMinor(u64, u64),
}

impl VersionRequirement {
    /// Parse a single version requirement (not comma-separated)
    fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();

        if let Some(rest) = spec.strip_prefix(">=") {
            parse_version(rest.trim()).map(VersionRequirement::Gte)
        } else if let Some(rest) = spec.strip_prefix('>') {
            parse_version(rest.trim()).map(VersionRequirement::Gt)
        } else if let Some(rest) = spec.strip_prefix("<=") {
            parse_version(rest.trim()).map(VersionRequirement::Lte)
        } else if let Some(rest) = spec.strip_prefix('<') {
            parse_version(rest.trim()).map(VersionRequirement::Lt)
        } else if let Some(rest) = spec.strip_prefix('=') {
            parse_version(rest.trim()).map(VersionRequirement::Exact)
        } else if let Some(rest) = spec.strip_prefix('^') {
            parse_version(rest.trim()).map(VersionRequirement::Caret)
        } else if let Some(rest) = spec.strip_prefix('~') {
            parse_version(rest.trim()).map(VersionRequirement::Tilde)
        } else if spec == "*" {
            Some(VersionRequirement::Any)
        } else if let Some(req) = Self::parse_wildcard(spec) {
            Some(req)
        } else {
            // Default (no prefix) behaves like caret in Cargo
            parse_version(spec).map(VersionRequirement::Caret)
        }
    }

    /// Parse wildcard patterns like "1.*" or "1.2.*"
    fn parse_wildcard(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.split('.').collect();

        match parts.as_slice() {
            // 1.*
            [major, "*"] => major
                .parse::<u64>()
                .ok()
                .map(VersionRequirement::WildcardMajor),
            // 1.2.*
            [major, minor, "*"] => {
                let major = major.parse::<u64>().ok()?;
                let minor = minor.parse::<u64>().ok()?;
                Some(VersionRequirement::WildcardMinor(major, minor))
            }
            _ => None,
        }
    }

    /// Check if a version satisfies this requirement
    fn satisfies(&self, version: &Version) -> bool {
        match self {
            VersionRequirement::Caret(v) => {
                if version < v {
                    return false;
                }
                // Cargo caret behavior:
                // ^1.2.3 -> >=1.2.3 <2.0.0
                // ^0.2.3 -> >=0.2.3 <0.3.0
                // ^0.0.3 -> >=0.0.3 <0.0.4
                if v.major == 0 {
                    if v.minor == 0 {
                        // ^0.0.x: only patch must match
                        version.major == 0 && version.minor == 0 && version.patch == v.patch
                    } else {
                        // ^0.x.y: major and minor must match
                        version.major == 0 && version.minor == v.minor
                    }
                } else {
                    // ^x.y.z: major must match
                    version.major == v.major
                }
            }
            VersionRequirement::Tilde(v) => {
                // ~1.2.3 -> >=1.2.3 <1.3.0
                version >= v && version.major == v.major && version.minor == v.minor
            }
            VersionRequirement::Exact(v) => version == v,
            VersionRequirement::Gte(v) => version >= v,
            VersionRequirement::Gt(v) => version > v,
            VersionRequirement::Lte(v) => version <= v,
            VersionRequirement::Lt(v) => version < v,
            VersionRequirement::Any => true,
            VersionRequirement::WildcardMajor(major) => version.major == *major,
            VersionRequirement::WildcardMinor(major, minor) => {
                version.major == *major && version.minor == *minor
            }
        }
    }

    /// Get the base version from this requirement (for comparison purposes)
    fn base_version(&self) -> Option<Version> {
        match self {
            VersionRequirement::Caret(v)
            | VersionRequirement::Tilde(v)
            | VersionRequirement::Exact(v)
            | VersionRequirement::Gte(v)
            | VersionRequirement::Gt(v)
            | VersionRequirement::Lte(v)
            | VersionRequirement::Lt(v) => Some(v.clone()),
            VersionRequirement::Any => None,
            VersionRequirement::WildcardMajor(major) => Some(Version::new(*major, 0, 0)),
            VersionRequirement::WildcardMinor(major, minor) => {
                Some(Version::new(*major, *minor, 0))
            }
        }
    }
}

/// Represents a compound version specification (multiple requirements)
#[derive(Debug)]
struct VersionSpec {
    /// All requirements must be satisfied (AND)
    requirements: Vec<VersionRequirement>,
}

impl VersionSpec {
    /// Parse a version specification (may be comma-separated)
    fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        // Split by comma for multiple requirements
        let parts: Vec<&str> = spec.split(',').map(|s| s.trim()).collect();

        let requirements: Option<Vec<VersionRequirement>> =
            parts.into_iter().map(VersionRequirement::parse).collect();

        requirements.map(|reqs| VersionSpec { requirements: reqs })
    }

    /// Check if a version satisfies all requirements
    fn satisfies(&self, version: &Version) -> bool {
        self.requirements.iter().all(|req| req.satisfies(version))
    }

    /// Get the base version from the first requirement
    fn base_version(&self) -> Option<Version> {
        self.requirements.first().and_then(|r| r.base_version())
    }
}

impl VersionMatcher for CratesVersionMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::CratesIo
    }

    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool {
        let Some(spec) = VersionSpec::parse(version_spec) else {
            return false;
        };

        available_versions.iter().any(|v| {
            Version::parse(v)
                .map(|ver| spec.satisfies(&ver))
                .unwrap_or(false)
        })
    }

    fn compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult {
        let Some(spec) = VersionSpec::parse(current_version) else {
            return CompareResult::Invalid;
        };

        let Ok(latest) = Version::parse(latest_version) else {
            return CompareResult::Invalid;
        };

        // Check if latest is within the spec
        if spec.satisfies(&latest) {
            return CompareResult::Latest;
        }

        // For Any (*), if not satisfied (which can't happen), treat as Latest
        let Some(base) = spec.base_version() else {
            return CompareResult::Latest;
        };

        if base < latest {
            CompareResult::Outdated
        } else {
            CompareResult::Newer
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // version_exists tests - default (caret-like) requirements
    #[rstest]
    // 1.2.3 means >=1.2.3, <2.0.0
    #[case("1.2.3", vec!["1.2.3", "1.3.0", "2.0.0"], true)]
    #[case("1.2.3", vec!["1.9.9", "2.0.0"], true)]
    #[case("1.2.3", vec!["1.2.2", "2.0.0"], false)]
    #[case("1.2.3", vec!["2.0.0", "3.0.0"], false)]
    // 0.2.3 means >=0.2.3, <0.3.0 (minor is breaking for 0.x)
    #[case("0.2.3", vec!["0.2.3", "0.2.9"], true)]
    #[case("0.2.3", vec!["0.3.0", "1.0.0"], false)]
    // 0.0.3 means >=0.0.3, <0.0.4 (patch is breaking for 0.0.x)
    #[case("0.0.3", vec!["0.0.3"], true)]
    #[case("0.0.3", vec!["0.0.4", "0.1.0"], false)]
    fn version_exists_default_requirement(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            CratesVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - explicit caret (^)
    #[rstest]
    #[case("^1.2.3", vec!["1.2.3", "1.9.9"], true)]
    #[case("^1.2.3", vec!["2.0.0"], false)]
    #[case("^0.2.3", vec!["0.2.5"], true)]
    #[case("^0.2.3", vec!["0.3.0"], false)]
    fn version_exists_caret_requirement(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            CratesVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - tilde (~)
    #[rstest]
    // ~1.2.3 means >=1.2.3, <1.3.0
    #[case("~1.2.3", vec!["1.2.3", "1.2.9"], true)]
    #[case("~1.2.3", vec!["1.3.0"], false)]
    fn version_exists_tilde_requirement(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            CratesVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - comparison operators
    #[rstest]
    #[case(">=1.0.0", vec!["1.0.0", "2.0.0"], true)]
    #[case(">=1.0.0", vec!["0.9.9"], false)]
    #[case(">1.0.0", vec!["1.0.1"], true)]
    #[case(">1.0.0", vec!["1.0.0"], false)]
    #[case("<=1.0.0", vec!["1.0.0", "0.9.0"], true)]
    #[case("<=1.0.0", vec!["1.0.1"], false)]
    #[case("<1.0.0", vec!["0.9.9"], true)]
    #[case("<1.0.0", vec!["1.0.0"], false)]
    #[case("=1.0.0", vec!["1.0.0"], true)]
    #[case("=1.0.0", vec!["1.0.1"], false)]
    fn version_exists_comparison_operators(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            CratesVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - wildcards
    #[rstest]
    #[case("*", vec!["1.0.0"], true)]
    #[case("1.*", vec!["1.5.0"], true)]
    #[case("1.*", vec!["2.0.0"], false)]
    #[case("1.2.*", vec!["1.2.5"], true)]
    #[case("1.2.*", vec!["1.3.0"], false)]
    fn version_exists_wildcards(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            CratesVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - multiple requirements (comma-separated)
    #[rstest]
    // >=1.0, <2.0 means both conditions must be satisfied
    #[case(">=1.0.0, <2.0.0", vec!["1.5.0"], true)]
    #[case(">=1.0.0, <2.0.0", vec!["2.0.0"], false)]
    #[case(">=1.0.0, <2.0.0", vec!["0.9.0"], false)]
    fn version_exists_multiple_requirements(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            CratesVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - partial versions (normalized)
    #[rstest]
    // 0.14 should be treated as 0.14.0 (caret: >=0.14.0 <0.15.0)
    #[case("0.14", vec!["0.14.0", "0.14.5"], true)]
    #[case("0.14", vec!["0.15.0"], false)]
    // 1 should be treated as 1.0.0 (caret: >=1.0.0 <2.0.0)
    #[case("1", vec!["1.0.0", "1.5.0"], true)]
    #[case("1", vec!["2.0.0"], false)]
    // ^0.14 should be treated as ^0.14.0
    #[case("^0.14", vec!["0.14.0", "0.14.5"], true)]
    #[case("^0.14", vec!["0.15.0"], false)]
    // ~1.2 should be treated as ~1.2.0
    #[case("~1.2", vec!["1.2.0", "1.2.9"], true)]
    #[case("~1.2", vec!["1.3.0"], false)]
    // >=1.2 should be treated as >=1.2.0
    #[case(">=1.2", vec!["1.2.0", "2.0.0"], true)]
    #[case(">=1.2", vec!["1.1.9"], false)]
    fn version_exists_partial_versions(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            CratesVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // compare_to_latest tests
    #[rstest]
    // Partial version comparison
    #[case("0.14", "0.14.5", CompareResult::Latest)]
    #[case("0.14", "0.15.0", CompareResult::Outdated)]
    #[case("1", "1.9.9", CompareResult::Latest)]
    #[case("1", "2.0.0", CompareResult::Outdated)]
    // Default requirements
    #[case("1.0.0", "1.9.9", CompareResult::Latest)]
    #[case("1.0.0", "2.0.0", CompareResult::Outdated)]
    #[case("2.0.0", "1.0.0", CompareResult::Newer)]
    // Explicit caret
    #[case("^1.0.0", "1.5.0", CompareResult::Latest)]
    #[case("^1.0.0", "2.0.0", CompareResult::Outdated)]
    // Tilde
    #[case("~1.2.0", "1.2.9", CompareResult::Latest)]
    #[case("~1.2.0", "1.3.0", CompareResult::Outdated)]
    // Wildcards
    #[case("*", "999.0.0", CompareResult::Latest)]
    #[case("1.*", "1.9.9", CompareResult::Latest)]
    #[case("1.*", "2.0.0", CompareResult::Outdated)]
    // Invalid
    #[case("invalid", "1.0.0", CompareResult::Invalid)]
    fn compare_to_latest_returns_expected(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] expected: CompareResult,
    ) {
        assert_eq!(
            CratesVersionMatcher.compare_to_latest(current, latest),
            expected
        );
    }
}
