use semver::Version;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareResult {
    Latest,
    Outdated,
    Newer,
    Invalid,
}

/// Parse a version string into a semver::Version, normalizing partial versions.
///
/// Handles partial versions like "1" or "1.2" by padding with zeros.
/// Strips version range prefixes (^, ~, >=, <=, >, <, =) and 'v' prefix.
///
/// Examples:
/// - "1" -> Version(1, 0, 0)
/// - "1.2" -> Version(1, 2, 0)
/// - "1.2.3" -> Version(1, 2, 3)
/// - "^1.2.3" -> Version(1, 2, 3)
/// - "~1.2.3" -> Version(1, 2, 3)
/// - ">=1.2.3" -> Version(1, 2, 3)
/// - "v1.2.3" -> Version(1, 2, 3)
pub fn parse_version(version: &str) -> Option<Version> {
    // Strip version range prefixes and 'v' prefix
    let stripped = version
        .trim_start_matches(">=")
        .trim_start_matches("<=")
        .trim_start_matches('>')
        .trim_start_matches('<')
        .trim_start_matches('=')
        .trim_start_matches('^')
        .trim_start_matches('~')
        .trim_start_matches('v');

    let parts: Vec<&str> = stripped.split('.').collect();
    let normalized = match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => stripped.to_string(),
    };
    Version::parse(&normalized).ok()
}

/// Calculate the latest patch version within the same major.minor
///
/// Returns the latest patch version if a newer patch exists,
/// or None if the current version is already the latest patch.
pub fn calculate_latest_patch(
    current_version: &str,
    available_versions: &[String],
) -> Option<String> {
    let current = parse_version(current_version)?;

    let latest_patch = available_versions
        .iter()
        .filter_map(|v| parse_version(v))
        .filter(|v| v.major == current.major && v.minor == current.minor)
        .max()?;

    if latest_patch > current {
        Some(latest_patch.to_string())
    } else {
        None
    }
}

/// Calculate the latest minor version within the same major
///
/// Returns the latest minor.patch version if a newer minor exists,
/// or None if the current version is already the latest minor.
pub fn calculate_latest_minor(
    current_version: &str,
    available_versions: &[String],
) -> Option<String> {
    let current = parse_version(current_version)?;

    let latest_minor = available_versions
        .iter()
        .filter_map(|v| parse_version(v))
        .filter(|v| v.major == current.major)
        .max()?;

    if latest_minor > current {
        Some(latest_minor.to_string())
    } else {
        None
    }
}

/// Calculate the latest major version
///
/// Returns the latest version if a newer major version exists,
/// or None if the current version is already the latest.
pub fn calculate_latest_major(
    current_version: &str,
    available_versions: &[String],
) -> Option<String> {
    let current = parse_version(current_version)?;

    let latest = available_versions
        .iter()
        .filter_map(|v| parse_version(v))
        .max()?;

    if latest > current {
        Some(latest.to_string())
    } else {
        None
    }
}

/// Check if a version string is a prerelease version.
/// Returns true if the version has a prerelease suffix (e.g., -alpha, -beta, -rc).
pub fn is_prerelease(version: &str) -> bool {
    parse_version(version)
        .map(|v| !v.pre.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("1.2.3", Some(Version::new(1, 2, 3)))]
    #[case("^1.2.3", Some(Version::new(1, 2, 3)))] // caret prefix
    #[case("~1.2.3", Some(Version::new(1, 2, 3)))] // tilde prefix
    #[case(">=1.2.3", Some(Version::new(1, 2, 3)))] // gte prefix
    #[case("<=1.2.3", Some(Version::new(1, 2, 3)))] // lte prefix
    #[case(">1.2.3", Some(Version::new(1, 2, 3)))] // gt prefix
    #[case("<1.2.3", Some(Version::new(1, 2, 3)))] // lt prefix
    #[case("=1.2.3", Some(Version::new(1, 2, 3)))] // eq prefix
    #[case("v1.2.3", Some(Version::new(1, 2, 3)))] // v prefix
    #[case("1.2", Some(Version::new(1, 2, 0)))] // partial version
    #[case("1", Some(Version::new(1, 0, 0)))] // single number
    #[case("invalid", None)] // invalid version
    fn test_parse_version(#[case] input: &str, #[case] expected: Option<Version>) {
        assert_eq!(parse_version(input), expected);
    }

    #[rstest]
    #[case("1.2.3", &["1.2.3", "1.2.5", "1.3.0", "2.0.0"], Some("1.2.5".to_string()))]
    #[case("^1.2.3", &["1.2.3", "1.2.5", "1.3.0", "2.0.0"], Some("1.2.5".to_string()))] // caret prefix
    #[case("~1.2.3", &["1.2.3", "1.2.5", "1.3.0", "2.0.0"], Some("1.2.5".to_string()))] // tilde prefix
    #[case("1.2.5", &["1.2.3", "1.2.5", "1.3.0", "2.0.0"], None)] // already latest patch
    #[case("invalid", &["1.2.3", "1.2.5"], None)] // unparseable current version
    #[case("1.2.3", &["invalid", "not-a-version"], None)] // no valid available versions
    #[case("1.2.3", &[], None)] // empty available versions
    fn test_calculate_latest_patch(
        #[case] current: &str,
        #[case] available: &[&str],
        #[case] expected: Option<String>,
    ) {
        let available_strings: Vec<String> = available.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            calculate_latest_patch(current, &available_strings),
            expected
        );
    }

    #[rstest]
    #[case("1.2.3", &["1.2.3", "1.3.0", "1.5.0", "2.0.0"], Some("1.5.0".to_string()))]
    #[case("1.5.0", &["1.2.3", "1.3.0", "1.5.0", "2.0.0"], None)] // already latest minor
    #[case("invalid", &["1.2.3", "1.5.0"], None)] // unparseable current version
    #[case("1.2.3", &["invalid", "not-a-version"], None)] // no valid available versions
    #[case("1.2.3", &[], None)] // empty available versions
    fn test_calculate_latest_minor(
        #[case] current: &str,
        #[case] available: &[&str],
        #[case] expected: Option<String>,
    ) {
        let available_strings: Vec<String> = available.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            calculate_latest_minor(current, &available_strings),
            expected
        );
    }

    #[rstest]
    #[case("1.2.3", &["1.2.3", "2.0.0", "3.0.0"], Some("3.0.0".to_string()))]
    #[case("3.0.0", &["1.2.3", "2.0.0", "3.0.0"], None)] // already latest major
    #[case("invalid", &["1.2.3", "2.0.0"], None)] // unparseable current version
    #[case("1.2.3", &["invalid", "not-a-version"], None)] // no valid available versions
    #[case("1.2.3", &[], None)] // empty available versions
    fn test_calculate_latest_major(
        #[case] current: &str,
        #[case] available: &[&str],
        #[case] expected: Option<String>,
    ) {
        let available_strings: Vec<String> = available.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            calculate_latest_major(current, &available_strings),
            expected
        );
    }

    // npm / crates.io / JSR / pnpm format
    #[rstest]
    #[case("1.0.0", false)] // stable version
    #[case("1.0.0-alpha", true)] // alpha
    #[case("1.0.0-alpha.1", true)] // alpha with number
    #[case("1.0.0-beta", true)] // beta
    #[case("1.0.0-beta.1", true)] // beta with number
    #[case("1.0.0-rc.1", true)] // release candidate
    #[case("1.0.0-canary.123", true)] // canary
    #[case("0.0.0-insiders.abc123", true)] // insiders
    #[case("1.0.0+build", false)] // build metadata only (not prerelease)
    #[case("1.0.0-alpha+build", true)] // prerelease + build metadata
    fn test_is_prerelease_npm_format(#[case] version: &str, #[case] expected: bool) {
        assert_eq!(is_prerelease(version), expected);
    }

    // GitHub Actions format (v prefix)
    #[rstest]
    #[case("v1.0.0", false)] // stable with v
    #[case("v1.0.0-alpha", true)] // alpha with v
    #[case("v1.0.0-beta.1", true)] // beta with v
    #[case("v1.0.0-rc.1", true)] // rc with v
    fn test_is_prerelease_github_actions_format(#[case] version: &str, #[case] expected: bool) {
        assert_eq!(is_prerelease(version), expected);
    }

    // Go format (including pseudo-versions)
    #[rstest]
    #[case("v1.0.0", false)] // stable
    #[case("v1.0.0-alpha", true)] // alpha
    #[case("v1.0.0-beta.1", true)] // beta
    #[case("v0.0.0-20210101000000-abc123", true)] // pseudo-version
    #[case("v1.1.3-0.20240916144458-20a13a1f6b7c", true)] // pseudo-version with base
    #[case("v2.0.0+incompatible", false)] // +incompatible is not prerelease
    #[case("v2.0.0-preview.4+incompatible", true)] // prerelease with +incompatible
    fn test_is_prerelease_go_format(#[case] version: &str, #[case] expected: bool) {
        assert_eq!(is_prerelease(version), expected);
    }

    // Edge cases
    #[rstest]
    #[case("invalid", false)] // invalid version
    #[case("", false)] // empty string
    #[case("1", false)] // partial version
    #[case("1.2", false)] // partial version
    fn test_is_prerelease_edge_cases(#[case] version: &str, #[case] expected: bool) {
        assert_eq!(is_prerelease(version), expected);
    }

    #[test]
    fn parse_version_correctly_extracts_prerelease_from_go_incompatible() {
        let version = parse_version("v2.0.0-preview.4+incompatible").unwrap();
        assert_eq!(version.major, 2);
        assert_eq!(version.minor, 0);
        assert_eq!(version.patch, 0);
        assert!(!version.pre.is_empty(), "prerelease should not be empty");
        assert_eq!(version.pre.as_str(), "preview.4");
    }
}
