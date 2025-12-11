#![allow(dead_code)]

use semver::Version;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionStatus {
    Latest,
    Outdated,
    Newer,
    Invalid,
}

pub fn compare_versions(current: &str, latest: &str) -> VersionStatus {
    let Some(current_ver) = Version::parse(current)
        .inspect_err(|e| warn!("Invalid current version '{}': {}", current, e))
        .ok()
    else {
        return VersionStatus::Invalid;
    };

    let Some(latest_ver) = Version::parse(latest)
        .inspect_err(|e| warn!("Invalid latest version '{}': {}", latest, e))
        .ok()
    else {
        return VersionStatus::Invalid;
    };

    match current_ver.cmp(&latest_ver) {
        std::cmp::Ordering::Equal => VersionStatus::Latest,
        std::cmp::Ordering::Less => VersionStatus::Outdated,
        std::cmp::Ordering::Greater => VersionStatus::Newer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("1.0.0", "1.0.0", VersionStatus::Latest)]
    #[case("1.0.0", "2.0.0", VersionStatus::Outdated)]
    #[case("2.0.0", "1.0.0", VersionStatus::Newer)]
    #[case("1.0.0", "1.0.1", VersionStatus::Outdated)]
    #[case("1.0.1", "1.0.0", VersionStatus::Newer)]
    #[case("1.1.0", "1.0.0", VersionStatus::Newer)]
    #[case("1.0.0", "1.1.0", VersionStatus::Outdated)]
    #[case("1.0.0-alpha", "1.0.0", VersionStatus::Outdated)]
    #[case("1.0.0", "1.0.0-alpha", VersionStatus::Newer)]
    #[case("1.0.0-alpha", "1.0.0-beta", VersionStatus::Outdated)]
    fn compare_versions_returns_expected(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] expected: VersionStatus,
    ) {
        assert_eq!(compare_versions(current, latest), expected);
    }

    #[rstest]
    #[case("invalid", "1.0.0")]
    #[case("1.0.0", "invalid")]
    #[case("not-a-version", "also-not")]
    fn compare_versions_returns_invalid_for_bad_input(#[case] current: &str, #[case] latest: &str) {
        assert_eq!(compare_versions(current, latest), VersionStatus::Invalid);
    }
}
