//! npm version matcher
//!
//! Supports npm semver range specifications:
//! - `1.2.3` - exact match
//! - `^1.2.3` - compatible with version (>=1.2.3 <2.0.0)
//! - `~1.2.3` - approximately equivalent (>=1.2.3 <1.3.0)
//! - `>=1.2.3`, `>1.2.3`, `<=1.2.3`, `<1.2.3` - comparison operators
//! - `1.2.x`, `1.x`, `*` - wildcards

use semver::Version;

use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::semver::{CompareResult, parse_version};

pub struct NpmVersionMatcher;

/// Top-level version specification parser
/// Handles compound ranges (AND, OR) as well as simple ranges
#[derive(Debug)]
enum VersionSpec {
    /// Single range (^1.0.0, >=1.0.0, etc.)
    Single(VersionRange),
    /// AND of ranges (>=1.0.0 <2.0.0) - space-separated, all must satisfy
    And(Vec<VersionSpec>),
    /// OR of specs (^1.0.0 || ^2.0.0) - any must satisfy
    Or(Vec<VersionSpec>),
}

impl VersionSpec {
    /// Parse a version specification string
    fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        // First, check for OR (||) - lowest precedence
        if spec.contains("||") {
            let or_parts: Vec<&str> = spec.split("||").map(|s| s.trim()).collect();
            if or_parts.len() > 1 {
                let specs: Option<Vec<VersionSpec>> = or_parts
                    .into_iter()
                    .map(Self::parse_and_or_single)
                    .collect();
                return specs.map(VersionSpec::Or);
            }
        }

        // No OR, parse as AND or single
        Self::parse_and_or_single(spec)
    }

    /// Parse a spec that may be AND (space-separated) or a single range
    fn parse_and_or_single(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        // Check for hyphen range first (contains " - " but is not AND)
        if VersionRange::parse_hyphen(spec).is_some() {
            return VersionRange::parse(spec).map(VersionSpec::Single);
        }

        // Try to split by spaces for AND ranges
        // Be careful not to split hyphen ranges incorrectly
        let parts = Self::split_and_parts(spec);

        if parts.len() > 1 {
            // Multiple parts = AND range
            let ranges: Option<Vec<VersionSpec>> = parts
                .into_iter()
                .map(|p| VersionRange::parse(p).map(VersionSpec::Single))
                .collect();
            ranges.map(VersionSpec::And)
        } else {
            // Single range
            VersionRange::parse(spec).map(VersionSpec::Single)
        }
    }

    /// Split spec into AND parts (space-separated ranges)
    fn split_and_parts(spec: &str) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut current_start = 0;
        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == ' ' {
                // Check if this space is part of a range operator or separator
                let before = &spec[current_start..i].trim();
                if !before.is_empty() {
                    // Check if this might be a hyphen range separator " - "
                    // Look ahead for " - " pattern
                    if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] == ' ' {
                        // This is a hyphen range, skip to after " - "
                        i += 3;
                        continue;
                    }

                    // This is a space separator for AND
                    parts.push(*before);
                    current_start = i + 1;
                }
            }
            i += 1;
        }

        // Add the last part
        let last = spec[current_start..].trim();
        if !last.is_empty() {
            parts.push(last);
        }

        parts
    }

    /// Check if a version satisfies this spec
    fn satisfies(&self, version: &Version) -> bool {
        match self {
            VersionSpec::Single(range) => range.satisfies(version),
            VersionSpec::And(specs) => specs.iter().all(|s| s.satisfies(version)),
            VersionSpec::Or(specs) => specs.iter().any(|s| s.satisfies(version)),
        }
    }

    /// Get the base version from this spec (for comparison purposes)
    fn base_version(&self) -> Option<Version> {
        match self {
            VersionSpec::Single(range) => range.base_version(),
            VersionSpec::And(specs) => {
                // For AND ranges, use the first range's base version
                specs.first().and_then(|s| s.base_version())
            }
            VersionSpec::Or(specs) => {
                // For OR ranges, use the first spec's base version
                specs.first().and_then(|s| s.base_version())
            }
        }
    }
}

/// Represents a parsed npm version range
#[derive(Debug)]
enum VersionRange {
    /// Exact version match
    Exact(Version),
    /// Caret range: ^1.2.3 means >=1.2.3 <2.0.0 (or special cases for 0.x)
    Caret(Version),
    /// Tilde range: ~1.2.3 means >=1.2.3 <1.3.0
    Tilde(Version),
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
    /// Wildcard major: 1.x means >=1.0.0 <2.0.0
    WildcardMajor(u64),
    /// Wildcard minor: 1.2.x means >=1.2.0 <1.3.0
    WildcardMinor(u64, u64),
    /// Hyphen range: 1.0.0 - 2.0.0 means >=1.0.0 <=2.0.0
    Hyphen { from: Version, to: Version },
}

impl VersionRange {
    /// Parse a version specification string into a VersionRange
    fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();

        // Check for hyphen range first (e.g., "1.0.0 - 2.0.0")
        if let Some(range) = Self::parse_hyphen(spec) {
            return Some(range);
        }

        if let Some(rest) = spec.strip_prefix(">=") {
            parse_version(rest.trim()).map(VersionRange::Gte)
        } else if let Some(rest) = spec.strip_prefix('>') {
            parse_version(rest.trim()).map(VersionRange::Gt)
        } else if let Some(rest) = spec.strip_prefix("<=") {
            parse_version(rest.trim()).map(VersionRange::Lte)
        } else if let Some(rest) = spec.strip_prefix('<') {
            parse_version(rest.trim()).map(VersionRange::Lt)
        } else if let Some(rest) = spec.strip_prefix('^') {
            parse_version(rest.trim()).map(VersionRange::Caret)
        } else if let Some(rest) = spec.strip_prefix('~') {
            parse_version(rest.trim()).map(VersionRange::Tilde)
        } else if spec == "*" {
            Some(VersionRange::Any)
        } else if let Some(range) = Self::parse_wildcard(spec) {
            Some(range)
        } else {
            parse_version(spec).map(VersionRange::Exact)
        }
    }

    /// Parse hyphen range like "1.0.0 - 2.0.0"
    fn parse_hyphen(spec: &str) -> Option<Self> {
        // Split by " - " (with spaces)
        let parts: Vec<&str> = spec.split(" - ").collect();
        if parts.len() != 2 {
            return None;
        }

        let from = parse_version(parts[0].trim())?;
        let to = parse_version(parts[1].trim())?;

        Some(VersionRange::Hyphen { from, to })
    }

    /// Parse wildcard patterns like "1.x" or "1.2.x"
    fn parse_wildcard(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.split('.').collect();

        match parts.as_slice() {
            // 1.x or 1.X
            [major, x] if x.eq_ignore_ascii_case("x") => {
                major.parse::<u64>().ok().map(VersionRange::WildcardMajor)
            }
            // 1.2.x or 1.2.X
            [major, minor, x] if x.eq_ignore_ascii_case("x") => {
                let major = major.parse::<u64>().ok()?;
                let minor = minor.parse::<u64>().ok()?;
                Some(VersionRange::WildcardMinor(major, minor))
            }
            _ => None,
        }
    }

    /// Check if a version satisfies this range
    fn satisfies(&self, version: &Version) -> bool {
        match self {
            VersionRange::Exact(v) => version == v,
            VersionRange::Caret(v) => {
                if version < v {
                    return false;
                }
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
            VersionRange::Tilde(v) => {
                // ~1.2.3 -> >=1.2.3 <1.3.0
                version >= v && version.major == v.major && version.minor == v.minor
            }
            VersionRange::Gte(v) => version >= v,
            VersionRange::Gt(v) => version > v,
            VersionRange::Lte(v) => version <= v,
            VersionRange::Lt(v) => version < v,
            VersionRange::Any => true,
            VersionRange::WildcardMajor(major) => version.major == *major,
            VersionRange::WildcardMinor(major, minor) => {
                version.major == *major && version.minor == *minor
            }
            VersionRange::Hyphen { from, to } => version >= from && version <= to,
        }
    }

    /// Get the base version from this range (for comparison purposes)
    /// Returns None for Any (*) since any version is acceptable
    fn base_version(&self) -> Option<Version> {
        match self {
            VersionRange::Exact(v)
            | VersionRange::Caret(v)
            | VersionRange::Tilde(v)
            | VersionRange::Gte(v)
            | VersionRange::Gt(v)
            | VersionRange::Lte(v)
            | VersionRange::Lt(v) => Some(v.clone()),
            VersionRange::Any => None,
            VersionRange::WildcardMajor(major) => Some(Version::new(*major, 0, 0)),
            VersionRange::WildcardMinor(major, minor) => Some(Version::new(*major, *minor, 0)),
            VersionRange::Hyphen { from, .. } => Some(from.clone()),
        }
    }
}

impl VersionMatcher for NpmVersionMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Npm
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

    // version_exists tests - exact match
    #[rstest]
    #[case("1.0.0", vec!["1.0.0", "2.0.0"], true)]
    #[case("1.0.0", vec!["1.0.1", "2.0.0"], false)]
    fn version_exists_exact_match(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - caret (^) range
    #[rstest]
    // ^1.2.3 matches >=1.2.3 <2.0.0
    #[case("^1.2.3", vec!["1.2.3", "1.3.0", "2.0.0"], true)]
    #[case("^1.2.3", vec!["1.9.9", "2.0.0"], true)]
    #[case("^1.2.3", vec!["1.2.2", "2.0.0"], false)]
    #[case("^1.2.3", vec!["2.0.0", "3.0.0"], false)]
    // ^0.2.3 matches >=0.2.3 <0.3.0 (special case for 0.x)
    #[case("^0.2.3", vec!["0.2.3", "0.2.9"], true)]
    #[case("^0.2.3", vec!["0.3.0", "1.0.0"], false)]
    // ^0.0.3 matches >=0.0.3 <0.0.4 (special case for 0.0.x)
    #[case("^0.0.3", vec!["0.0.3"], true)]
    #[case("^0.0.3", vec!["0.0.4", "0.1.0"], false)]
    fn version_exists_caret_range(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - tilde (~) range
    #[rstest]
    // ~1.2.3 matches >=1.2.3 <1.3.0
    #[case("~1.2.3", vec!["1.2.3", "1.2.9"], true)]
    #[case("~1.2.3", vec!["1.3.0", "2.0.0"], false)]
    #[case("~1.2.3", vec!["1.2.2"], false)]
    fn version_exists_tilde_range(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - comparison operators
    #[rstest]
    #[case(">=1.0.0", vec!["1.0.0", "2.0.0"], true)]
    #[case(">=1.0.0", vec!["0.9.9"], false)]
    #[case(">1.0.0", vec!["1.0.1", "2.0.0"], true)]
    #[case(">1.0.0", vec!["1.0.0", "0.9.9"], false)]
    #[case("<=1.0.0", vec!["1.0.0", "0.9.0"], true)]
    #[case("<=1.0.0", vec!["1.0.1", "2.0.0"], false)]
    #[case("<1.0.0", vec!["0.9.9", "0.1.0"], true)]
    #[case("<1.0.0", vec!["1.0.0", "2.0.0"], false)]
    fn version_exists_comparison_operators(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - wildcards
    #[rstest]
    // * matches any version
    #[case("*", vec!["1.0.0", "2.0.0"], true)]
    #[case("*", vec!["0.0.1"], true)]
    // 1.x matches >=1.0.0 <2.0.0
    #[case("1.x", vec!["1.0.0", "1.9.9"], true)]
    #[case("1.x", vec!["0.9.9", "2.0.0"], false)]
    #[case("1.X", vec!["1.5.0"], true)]
    // 1.2.x matches >=1.2.0 <1.3.0
    #[case("1.2.x", vec!["1.2.0", "1.2.9"], true)]
    #[case("1.2.x", vec!["1.1.9", "1.3.0"], false)]
    #[case("1.2.X", vec!["1.2.5"], true)]
    fn version_exists_wildcards(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - OR range (|| separated)
    #[rstest]
    // ^1.0.0 || ^2.0.0 means satisfy EITHER range
    #[case("^1.0.0 || ^2.0.0", vec!["1.5.0"], true)]
    #[case("^1.0.0 || ^2.0.0", vec!["2.5.0"], true)]
    #[case("^1.0.0 || ^2.0.0", vec!["3.0.0"], false)]
    #[case(">=1.0.0 <1.5.0 || >=2.0.0", vec!["1.2.0"], true)]
    #[case(">=1.0.0 <1.5.0 || >=2.0.0", vec!["1.6.0"], false)]
    #[case(">=1.0.0 <1.5.0 || >=2.0.0", vec!["2.5.0"], true)]
    #[case("1.0.0 || 2.0.0 || 3.0.0", vec!["2.0.0"], true)]
    #[case("1.0.0 || 2.0.0 || 3.0.0", vec!["4.0.0"], false)]
    fn version_exists_or_range(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - AND range (space-separated)
    #[rstest]
    // >=1.0.0 <2.0.0 means version must satisfy BOTH conditions
    #[case(">=1.0.0 <2.0.0", vec!["1.0.0"], true)]
    #[case(">=1.0.0 <2.0.0", vec!["1.5.0"], true)]
    #[case(">=1.0.0 <2.0.0", vec!["1.9.9"], true)]
    #[case(">=1.0.0 <2.0.0", vec!["0.9.9"], false)]
    #[case(">=1.0.0 <2.0.0", vec!["2.0.0"], false)]
    #[case(">1.0.0 <=2.0.0", vec!["1.0.1"], true)]
    #[case(">1.0.0 <=2.0.0", vec!["2.0.0"], true)]
    #[case(">1.0.0 <=2.0.0", vec!["1.0.0"], false)]
    #[case(">=1.2.0 <1.3.0", vec!["1.2.5"], true)]
    #[case(">=1.2.0 <1.3.0", vec!["1.3.0"], false)]
    fn version_exists_and_range(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - hyphen range
    #[rstest]
    // 1.0.0 - 2.0.0 matches >=1.0.0 <=2.0.0
    #[case("1.0.0 - 2.0.0", vec!["1.0.0"], true)]
    #[case("1.0.0 - 2.0.0", vec!["1.5.0"], true)]
    #[case("1.0.0 - 2.0.0", vec!["2.0.0"], true)]
    #[case("1.0.0 - 2.0.0", vec!["0.9.9"], false)]
    #[case("1.0.0 - 2.0.0", vec!["2.0.1"], false)]
    #[case("1.2.3 - 2.3.4", vec!["1.2.3", "2.3.4"], true)]
    #[case("1.2.3 - 2.3.4", vec!["1.2.2"], false)]
    #[case("1.2.3 - 2.3.4", vec!["2.3.5"], false)]
    fn version_exists_hyphen_range(
        #[case] version_spec: &str,
        #[case] available: Vec<&str>,
        #[case] expected: bool,
    ) {
        let available: Vec<String> = available.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // version_exists tests - partial versions (normalized)
    #[rstest]
    // 0.14 should be treated as exact 0.14.0 in npm
    #[case("0.14", vec!["0.14.0"], true)]
    #[case("0.14", vec!["0.14.1"], false)]
    // 1 should be treated as exact 1.0.0
    #[case("1", vec!["1.0.0"], true)]
    #[case("1", vec!["1.0.1"], false)]
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
            NpmVersionMatcher.version_exists(version_spec, &available),
            expected
        );
    }

    // compare_to_latest tests
    #[rstest]
    // Partial version comparison
    #[case("0.14", "0.14.0", CompareResult::Latest)]
    #[case("0.14", "0.15.0", CompareResult::Outdated)]
    #[case("1", "1.0.0", CompareResult::Latest)]
    #[case("1", "2.0.0", CompareResult::Outdated)]
    // Exact version comparison
    #[case("1.0.0", "1.0.0", CompareResult::Latest)]
    #[case("1.0.0", "2.0.0", CompareResult::Outdated)]
    #[case("2.0.0", "1.0.0", CompareResult::Newer)]
    // Range spec - compare base version to latest
    #[case("^1.0.0", "1.9.9", CompareResult::Latest)]
    #[case("^1.0.0", "2.0.0", CompareResult::Outdated)]
    #[case("~1.2.0", "1.2.9", CompareResult::Latest)]
    #[case("~1.2.0", "1.3.0", CompareResult::Outdated)]
    // Wildcards
    #[case("*", "999.0.0", CompareResult::Latest)]
    #[case("1.x", "1.9.9", CompareResult::Latest)]
    #[case("1.x", "2.0.0", CompareResult::Outdated)]
    #[case("1.2.x", "1.2.9", CompareResult::Latest)]
    #[case("1.2.x", "1.3.0", CompareResult::Outdated)]
    // Hyphen ranges
    #[case("1.0.0 - 2.0.0", "1.5.0", CompareResult::Latest)]
    #[case("1.0.0 - 2.0.0", "2.0.0", CompareResult::Latest)]
    #[case("1.0.0 - 2.0.0", "2.5.0", CompareResult::Outdated)]
    // AND ranges
    #[case(">=1.0.0 <2.0.0", "1.5.0", CompareResult::Latest)]
    #[case(">=1.0.0 <2.0.0", "2.0.0", CompareResult::Outdated)]
    #[case(">=1.0.0 <2.0.0", "0.9.0", CompareResult::Newer)]
    // OR ranges
    #[case("^1.0.0 || ^2.0.0", "1.5.0", CompareResult::Latest)]
    #[case("^1.0.0 || ^2.0.0", "2.5.0", CompareResult::Latest)]
    #[case("^1.0.0 || ^2.0.0", "3.0.0", CompareResult::Outdated)]
    // Invalid versions
    #[case("invalid", "1.0.0", CompareResult::Invalid)]
    #[case("1.0.0", "invalid", CompareResult::Invalid)]
    fn compare_to_latest_returns_expected(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] expected: CompareResult,
    ) {
        assert_eq!(
            NpmVersionMatcher.compare_to_latest(current, latest),
            expected
        );
    }
}
