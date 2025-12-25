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
}
