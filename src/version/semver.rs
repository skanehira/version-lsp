use semver::Version;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareResult {
    Latest,
    Outdated,
    Newer,
    Invalid,
}

/// Normalize a version string to strict SemVer format (X.Y.Z).
///
/// Handles:
/// - `v` prefix: `v5.0.0` → `5.0.0`
/// - Major only: `5` → `5.0.0`
/// - Major.minor: `5.0` → `5.0.0`
/// - Pre-release: `1.2.3-alpha` → `1.2.3-alpha`
///
/// Returns `None` if the input cannot be parsed as a version.
pub fn normalize_version(version: &str) -> Option<String> {
    // Remove 'v' or 'V' prefix
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = version.strip_prefix('V').unwrap_or(version);

    if version.is_empty() {
        return None;
    }

    // Split by '-' to handle pre-release versions
    let (base, prerelease) = match version.split_once('-') {
        Some((base, pre)) => (base, Some(pre)),
        None => (version, None),
    };

    // Split the base version by '.'
    let parts: Vec<&str> = base.split('.').collect();

    // Validate and pad version parts
    let (major, minor, patch) = match parts.as_slice() {
        [major] => {
            let major: u64 = major.parse().ok()?;
            (major, 0u64, 0u64)
        }
        [major, minor] => {
            let major: u64 = major.parse().ok()?;
            let minor: u64 = minor.parse().ok()?;
            (major, minor, 0u64)
        }
        [major, minor, patch] => {
            let major: u64 = major.parse().ok()?;
            let minor: u64 = minor.parse().ok()?;
            let patch: u64 = patch.parse().ok()?;
            (major, minor, patch)
        }
        _ => return None,
    };

    // Reconstruct the version string
    match prerelease {
        Some(pre) => Some(format!("{}.{}.{}-{}", major, minor, patch, pre)),
        None => Some(format!("{}.{}.{}", major, minor, patch)),
    }
}

/// Count how many version parts were specified in the original input.
/// Returns 1 for major only, 2 for major.minor, 3 for full version.
fn count_version_parts(version: &str) -> usize {
    // Remove 'v' or 'V' prefix
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = version.strip_prefix('V').unwrap_or(version);

    // Remove pre-release suffix
    let base = version.split('-').next().unwrap_or(version);

    base.split('.').count()
}

/// Check if a version matches any version in the available list.
///
/// Uses partial matching based on the number of version parts specified:
/// - `v6` matches any version with major version 6 (e.g., `v6.0.0`, `v6.1.0`)
/// - `v6.1` matches any version with major.minor 6.1 (e.g., `v6.1.0`, `v6.1.5`)
/// - `v6.1.0` requires exact match
pub fn version_matches_any(current: &str, available_versions: &[String]) -> bool {
    let Some(current_normalized) = normalize_version(current) else {
        return false;
    };

    let Ok(current_ver) = Version::parse(&current_normalized) else {
        return false;
    };

    let parts = count_version_parts(current);

    for available in available_versions {
        let Some(available_normalized) = normalize_version(available) else {
            continue;
        };

        let Ok(available_ver) = Version::parse(&available_normalized) else {
            continue;
        };

        let matches = match parts {
            1 => current_ver.major == available_ver.major,
            2 => {
                current_ver.major == available_ver.major && current_ver.minor == available_ver.minor
            }
            _ => current_ver == available_ver,
        };

        if matches {
            return true;
        }
    }

    false
}

pub fn compare_versions(current: &str, latest: &str) -> CompareResult {
    let Some(current_normalized) = normalize_version(current) else {
        warn!("Invalid current version format: '{}'", current);
        return CompareResult::Invalid;
    };

    let Some(latest_normalized) = normalize_version(latest) else {
        warn!("Invalid latest version format: '{}'", latest);
        return CompareResult::Invalid;
    };

    let Some(current_ver) = Version::parse(&current_normalized)
        .inspect_err(|e| warn!("Invalid current version '{}': {}", current_normalized, e))
        .ok()
    else {
        return CompareResult::Invalid;
    };

    let Some(latest_ver) = Version::parse(&latest_normalized)
        .inspect_err(|e| warn!("Invalid latest version '{}': {}", latest_normalized, e))
        .ok()
    else {
        return CompareResult::Invalid;
    };

    // Determine comparison depth based on current version's specificity
    let parts = count_version_parts(current);

    match parts {
        1 => {
            // Major only: compare only major versions
            match current_ver.major.cmp(&latest_ver.major) {
                std::cmp::Ordering::Equal => CompareResult::Latest,
                std::cmp::Ordering::Less => CompareResult::Outdated,
                std::cmp::Ordering::Greater => CompareResult::Newer,
            }
        }
        2 => {
            // Major.minor: compare major and minor versions
            match (current_ver.major, current_ver.minor).cmp(&(latest_ver.major, latest_ver.minor))
            {
                std::cmp::Ordering::Equal => CompareResult::Latest,
                std::cmp::Ordering::Less => CompareResult::Outdated,
                std::cmp::Ordering::Greater => CompareResult::Newer,
            }
        }
        _ => {
            // Full version: compare all parts
            match current_ver.cmp(&latest_ver) {
                std::cmp::Ordering::Equal => CompareResult::Latest,
                std::cmp::Ordering::Less => CompareResult::Outdated,
                std::cmp::Ordering::Greater => CompareResult::Newer,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("5", "5.0.0")]
    #[case("5.0", "5.0.0")]
    #[case("5.0.0", "5.0.0")]
    #[case("v5", "5.0.0")]
    #[case("v5.0", "5.0.0")]
    #[case("v5.0.0", "5.0.0")]
    #[case("1.2.3-alpha", "1.2.3-alpha")]
    #[case("v1.2.3-beta.1", "1.2.3-beta.1")]
    fn normalize_version_returns_semver_format(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(normalize_version(input), Some(expected.to_string()));
    }

    #[rstest]
    #[case("")]
    #[case("invalid")]
    #[case("v")]
    #[case("abc.def.ghi")]
    fn normalize_version_returns_none_for_invalid(#[case] input: &str) {
        assert_eq!(normalize_version(input), None);
    }

    #[rstest]
    #[case("1.0.0", "1.0.0", CompareResult::Latest)]
    #[case("1.0.0", "2.0.0", CompareResult::Outdated)]
    #[case("2.0.0", "1.0.0", CompareResult::Newer)]
    #[case("1.0.0", "1.0.1", CompareResult::Outdated)]
    #[case("1.0.1", "1.0.0", CompareResult::Newer)]
    #[case("1.1.0", "1.0.0", CompareResult::Newer)]
    #[case("1.0.0", "1.1.0", CompareResult::Outdated)]
    #[case("1.0.0-alpha", "1.0.0", CompareResult::Outdated)]
    #[case("1.0.0", "1.0.0-alpha", CompareResult::Newer)]
    #[case("1.0.0-alpha", "1.0.0-beta", CompareResult::Outdated)]
    // With 'v' prefix
    #[case("v1.0.0", "v2.0.0", CompareResult::Outdated)]
    #[case("v2.0.0", "v1.0.0", CompareResult::Newer)]
    #[case("v1.0.0", "1.0.0", CompareResult::Latest)]
    // Partial version matching (major only) - matches any version with same major
    #[case("v4", "v4.0.0", CompareResult::Latest)]
    #[case("v4", "v4.1.0", CompareResult::Latest)]
    #[case("v4", "v4.5.3", CompareResult::Latest)]
    #[case("v4", "v5.0.0", CompareResult::Outdated)]
    #[case("v5", "v4.0.0", CompareResult::Newer)]
    #[case("5", "v5.2.1", CompareResult::Latest)]
    // Partial version matching (major.minor) - matches any version with same major.minor
    #[case("v4.1", "v4.1.0", CompareResult::Latest)]
    #[case("v4.1", "v4.1.5", CompareResult::Latest)]
    #[case("v4.1", "v4.2.0", CompareResult::Outdated)]
    #[case("v4.2", "v4.1.0", CompareResult::Newer)]
    // Full version - exact comparison
    #[case("v4.1.0", "v4.1.0", CompareResult::Latest)]
    #[case("v4.1.0", "v4.1.5", CompareResult::Outdated)]
    #[case("v4.1.5", "v4.1.0", CompareResult::Newer)]
    fn compare_versions_returns_expected(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] expected: CompareResult,
    ) {
        assert_eq!(compare_versions(current, latest), expected);
    }

    #[rstest]
    #[case("invalid", "1.0.0")]
    #[case("1.0.0", "invalid")]
    #[case("not-a-version", "also-not")]
    fn compare_versions_returns_invalid_for_bad_input(#[case] current: &str, #[case] latest: &str) {
        assert_eq!(compare_versions(current, latest), CompareResult::Invalid);
    }
}
