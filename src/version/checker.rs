//! Version comparison for packages

#[cfg(test)]
use mockall::automock;

use crate::version::error::CacheError;
use crate::version::semver::{CompareResult, compare_versions};

use crate::version::cache::PackageId;

/// Trait for storing and retrieving version information
#[cfg_attr(test, automock)]
pub trait VersionStorer: Send + Sync + 'static {
    /// Get the latest version for a package
    fn get_latest_version(
        &self,
        registry_type: &str,
        package_name: &str,
    ) -> Result<Option<String>, CacheError>;

    /// Check if a specific version exists for a package
    fn version_exists(
        &self,
        registry_type: &str,
        package_name: &str,
        version: &str,
    ) -> Result<bool, CacheError>;

    /// Replace all versions for a package
    fn replace_versions(
        &self,
        registry_type: &str,
        package_name: &str,
        versions: Vec<String>,
    ) -> Result<(), CacheError>;

    /// Get packages that need to be refreshed
    fn get_packages_needing_refresh(&self) -> Result<Vec<PackageId>, CacheError>;
}

/// Result of version comparison
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionCompareResult {
    /// Current version from the source file
    pub current_version: String,
    /// Latest version from the registry (if available)
    pub latest_version: Option<String>,
    /// Version status
    pub status: VersionStatus,
}

/// Status of the version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionStatus {
    /// Current version is the latest
    Latest,
    /// Current version is outdated
    Outdated,
    /// Current version is newer than latest (pre-release or development)
    Newer,
    /// Version format is invalid
    Invalid,
    /// Version not found in cache
    NotInCache,
    /// Current version doesn't exist in registry
    NotFound,
}

/// Compare the version status for a package
pub fn compare_version<S: VersionStorer>(
    storer: &S,
    registry_type: &str,
    package_name: &str,
    current_version: &str,
) -> Result<VersionCompareResult, CacheError> {
    // Get latest version from storer
    let latest_version = storer.get_latest_version(registry_type, package_name)?;

    // If no versions in cache, return NotInCache
    let Some(latest) = latest_version else {
        return Ok(VersionCompareResult {
            current_version: current_version.to_string(),
            latest_version: None,
            status: VersionStatus::NotInCache,
        });
    };

    // Check if current version exists in registry
    let version_exists = storer.version_exists(registry_type, package_name, current_version)?;

    // Compare versions
    let status = match compare_versions(current_version, &latest) {
        CompareResult::Invalid => VersionStatus::Invalid,
        _ if !version_exists => VersionStatus::NotFound,
        CompareResult::Latest => VersionStatus::Latest,
        CompareResult::Outdated => VersionStatus::Outdated,
        CompareResult::Newer => VersionStatus::Newer,
    };

    Ok(VersionCompareResult {
        current_version: current_version.to_string(),
        latest_version: Some(latest),
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Mock storer for testing
    struct MockStorer {
        latest_version: Option<String>,
        existing_versions: Vec<String>,
    }

    impl MockStorer {
        fn new(latest: Option<&str>, versions: Vec<&str>) -> Self {
            Self {
                latest_version: latest.map(|s| s.to_string()),
                existing_versions: versions.into_iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl VersionStorer for MockStorer {
        fn get_latest_version(
            &self,
            _registry_type: &str,
            _package_name: &str,
        ) -> Result<Option<String>, CacheError> {
            Ok(self.latest_version.clone())
        }

        fn version_exists(
            &self,
            _registry_type: &str,
            _package_name: &str,
            version: &str,
        ) -> Result<bool, CacheError> {
            Ok(self.existing_versions.contains(&version.to_string()))
        }

        fn replace_versions(
            &self,
            _registry_type: &str,
            _package_name: &str,
            _versions: Vec<String>,
        ) -> Result<(), CacheError> {
            Ok(())
        }

        fn get_packages_needing_refresh(&self) -> Result<Vec<PackageId>, CacheError> {
            Ok(vec![])
        }
    }

    #[rstest]
    #[case("4.0.0", "4.0.0", vec!["4.0.0", "3.0.0"], VersionStatus::Latest)]
    #[case("3.0.0", "4.0.0", vec!["4.0.0", "3.0.0"], VersionStatus::Outdated)]
    #[case("5.0.0", "4.0.0", vec!["5.0.0", "4.0.0"], VersionStatus::Newer)]
    #[case("1.0.0", "4.0.0", vec!["4.0.0", "3.0.0"], VersionStatus::NotFound)]
    #[case("invalid", "4.0.0", vec!["4.0.0", "3.0.0"], VersionStatus::Invalid)]
    fn compare_version_returns_expected_status(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] existing: Vec<&str>,
        #[case] expected: VersionStatus,
    ) {
        let storer = MockStorer::new(Some(latest), existing);

        let result =
            compare_version(&storer, "github_actions", "actions/checkout", current).unwrap();

        assert_eq!(result.current_version, current);
        assert_eq!(result.latest_version, Some(latest.to_string()));
        assert_eq!(result.status, expected);
    }

    #[test]
    fn compare_version_returns_not_in_cache_when_package_not_cached() {
        let storer = MockStorer::new(None, vec![]);

        let result =
            compare_version(&storer, "github_actions", "nonexistent/repo", "1.0.0").unwrap();

        assert_eq!(
            result,
            VersionCompareResult {
                current_version: "1.0.0".to_string(),
                latest_version: None,
                status: VersionStatus::NotInCache,
            }
        );
    }
}
