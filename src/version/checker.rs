//! Version comparison for packages

#[cfg(test)]
use mockall::automock;

use crate::parser::types::RegistryType;
use crate::version::error::CacheError;
use crate::version::matcher::VersionMatcher;
use crate::version::semver::CompareResult;

use crate::version::cache::PackageId;

/// Trait for storing and retrieving version information
#[cfg_attr(test, automock)]
pub trait VersionStorer: Send + Sync + 'static {
    /// Get the latest version for a package
    fn get_latest_version(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<Option<String>, CacheError>;

    /// Get all versions for a package
    fn get_versions(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<Vec<String>, CacheError>;

    /// Check if a specific version exists for a package
    fn version_exists(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        version: &str,
    ) -> Result<bool, CacheError>;

    /// Replace all versions for a package
    fn replace_versions(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        versions: Vec<String>,
    ) -> Result<(), CacheError>;

    /// Get packages that need to be refreshed
    fn get_packages_needing_refresh(&self) -> Result<Vec<PackageId>, CacheError>;

    /// Try to start fetching a package. Returns true if fetch can proceed.
    /// Returns false if another process is already fetching this package.
    fn try_start_fetch(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<bool, CacheError>;

    /// Mark fetch as complete (success or failure)
    fn finish_fetch(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<(), CacheError>;

    /// Get a specific dist tag for a package (e.g., "latest" -> "4.17.21")
    fn get_dist_tag(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        tag_name: &str,
    ) -> Result<Option<String>, CacheError>;

    /// Save dist tags for a package
    fn save_dist_tags(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        dist_tags: &std::collections::HashMap<String, String>,
    ) -> Result<(), CacheError>;

    /// Filter packages that are not in the cache
    /// Returns package names that have no entries in the cache
    fn filter_packages_not_in_cache(
        &self,
        registry_type: RegistryType,
        package_names: &[String],
    ) -> Result<Vec<String>, CacheError>;

    /// Mark a package as not found (does not exist in registry)
    /// This prevents repeated fetch attempts for non-existent packages
    fn mark_not_found(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<(), CacheError>;
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

/// Common npm dist-tag names that we should not treat as invalid versions
/// These are well-known dist-tags used in the npm ecosystem
const KNOWN_DIST_TAGS: &[&str] = &[
    "latest",
    "next",
    "beta",
    "alpha",
    "canary",
    "rc",
    "stable",
    "dev",
    "experimental",
    "nightly",
    "preview",
    "insiders",
    "edge",
];

/// Check if a version string is a known dist-tag pattern
/// Returns true only for well-known dist-tag names
fn is_potential_dist_tag(version: &str) -> bool {
    KNOWN_DIST_TAGS.contains(&version.to_lowercase().as_str())
}

/// Compare the version status for a package
pub fn compare_version<S: VersionStorer>(
    storer: &S,
    matcher: &dyn VersionMatcher,
    package_name: &str,
    current_version: &str,
) -> Result<VersionCompareResult, CacheError> {
    let registry_type = matcher.registry_type();

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

    // Try to resolve dist-tag to actual version (e.g., "latest" -> "4.17.21")
    let dist_tag_resolution = storer.get_dist_tag(registry_type, package_name, current_version)?;

    // If version looks like a dist-tag but we couldn't resolve it, return NotInCache
    // This avoids showing "Invalid version format" for unresolved dist-tags like "latest"
    let resolved_version = match dist_tag_resolution {
        Some(version) => version,
        None if is_potential_dist_tag(current_version) => {
            return Ok(VersionCompareResult {
                current_version: current_version.to_string(),
                latest_version: Some(latest),
                status: VersionStatus::NotInCache,
            });
        }
        None => current_version.to_string(),
    };

    // Check if current version exists in registry
    let all_versions = storer.get_versions(registry_type, package_name)?;
    let version_exists = matcher.version_exists(&resolved_version, &all_versions);

    // Compare versions
    let status = match matcher.compare_to_latest(&resolved_version, &latest) {
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
    use crate::version::matchers::GitHubActionsMatcher;
    use rstest::rstest;

    /// Mock storer for testing
    struct MockStorer {
        latest_version: Option<String>,
        existing_versions: Vec<String>,
        dist_tags: std::collections::HashMap<String, String>,
    }

    impl MockStorer {
        fn new(latest: Option<&str>, versions: Vec<&str>) -> Self {
            Self {
                latest_version: latest.map(|s| s.to_string()),
                existing_versions: versions.into_iter().map(|s| s.to_string()).collect(),
                dist_tags: std::collections::HashMap::new(),
            }
        }

        fn with_dist_tags(
            latest: Option<&str>,
            versions: Vec<&str>,
            dist_tags: std::collections::HashMap<String, String>,
        ) -> Self {
            Self {
                latest_version: latest.map(|s| s.to_string()),
                existing_versions: versions.into_iter().map(|s| s.to_string()).collect(),
                dist_tags,
            }
        }
    }

    impl VersionStorer for MockStorer {
        fn get_latest_version(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<Option<String>, CacheError> {
            Ok(self.latest_version.clone())
        }

        fn get_versions(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<Vec<String>, CacheError> {
            Ok(self.existing_versions.clone())
        }

        fn version_exists(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
            version: &str,
        ) -> Result<bool, CacheError> {
            Ok(self.existing_versions.contains(&version.to_string()))
        }

        fn replace_versions(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
            _versions: Vec<String>,
        ) -> Result<(), CacheError> {
            Ok(())
        }

        fn get_packages_needing_refresh(&self) -> Result<Vec<PackageId>, CacheError> {
            Ok(vec![])
        }

        fn try_start_fetch(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<bool, CacheError> {
            Ok(true)
        }

        fn finish_fetch(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<(), CacheError> {
            Ok(())
        }

        fn get_dist_tag(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
            tag_name: &str,
        ) -> Result<Option<String>, CacheError> {
            Ok(self.dist_tags.get(tag_name).cloned())
        }

        fn save_dist_tags(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
            _dist_tags: &std::collections::HashMap<String, String>,
        ) -> Result<(), CacheError> {
            Ok(())
        }

        fn filter_packages_not_in_cache(
            &self,
            _registry_type: RegistryType,
            package_names: &[String],
        ) -> Result<Vec<String>, CacheError> {
            // Return all packages as not in cache (mock behavior)
            Ok(package_names.to_vec())
        }

        fn mark_not_found(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<(), CacheError> {
            Ok(())
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
        let matcher = GitHubActionsMatcher;

        let result = compare_version(&storer, &matcher, "actions/checkout", current).unwrap();

        assert_eq!(result.current_version, current);
        assert_eq!(result.latest_version, Some(latest.to_string()));
        assert_eq!(result.status, expected);
    }

    #[test]
    fn compare_version_returns_not_in_cache_when_package_not_cached() {
        let storer = MockStorer::new(None, vec![]);
        let matcher = GitHubActionsMatcher;

        let result = compare_version(&storer, &matcher, "nonexistent/repo", "1.0.0").unwrap();

        assert_eq!(
            result,
            VersionCompareResult {
                current_version: "1.0.0".to_string(),
                latest_version: None,
                status: VersionStatus::NotInCache,
            }
        );
    }

    #[rstest]
    // Major only: v6 matches v6.0.0, v6.1.0, etc.
    #[case("v6", "v6.0.0", vec!["v6.0.0", "v5.0.0"], VersionStatus::Latest)]
    #[case("6", "v6.0.0", vec!["v6.0.0", "v5.0.0"], VersionStatus::Latest)]
    #[case("v5", "v6.0.0", vec!["v6.0.0", "v5.0.0"], VersionStatus::Outdated)]
    // Major.minor: v6.1 matches v6.1.0, v6.1.5, etc.
    #[case("v6.1", "v6.2.0", vec!["v6.2.0", "v6.1.0"], VersionStatus::Outdated)]
    #[case("v6.2", "v6.2.0", vec!["v6.2.0", "v6.1.0"], VersionStatus::Latest)]
    // Full version: exact match required
    #[case("v6.0.0", "v6.0.0", vec!["v6.0.0", "v5.0.0"], VersionStatus::Latest)]
    #[case("v5.0.0", "v6.0.0", vec!["v6.0.0", "v5.0.0"], VersionStatus::Outdated)]
    fn compare_version_handles_partial_version_matching(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] existing: Vec<&str>,
        #[case] expected: VersionStatus,
    ) {
        let storer = MockStorer::new(Some(latest), existing);
        let matcher = GitHubActionsMatcher;

        let result = compare_version(&storer, &matcher, "actions/checkout", current).unwrap();

        assert_eq!(result.status, expected);
    }

    mod dist_tags {
        use super::*;
        use crate::version::matchers::NpmVersionMatcher;

        #[test]
        fn compare_version_resolves_latest_dist_tag_to_actual_version() {
            let mut dist_tags = std::collections::HashMap::new();
            dist_tags.insert("latest".to_string(), "4.17.21".to_string());

            let storer =
                MockStorer::with_dist_tags(Some("4.17.21"), vec!["4.17.20", "4.17.21"], dist_tags);
            let matcher = NpmVersionMatcher;

            // "latest" should resolve to "4.17.21" which is the latest
            let result = compare_version(&storer, &matcher, "lodash", "latest").unwrap();

            assert_eq!(result.status, VersionStatus::Latest);
            assert_eq!(result.current_version, "latest");
        }

        #[test]
        fn compare_version_resolves_beta_dist_tag() {
            let mut dist_tags = std::collections::HashMap::new();
            dist_tags.insert("beta".to_string(), "5.0.0-beta.1".to_string());

            let storer = MockStorer::with_dist_tags(
                Some("4.17.21"), // Latest stable
                vec!["4.17.20", "4.17.21", "5.0.0-beta.1"],
                dist_tags,
            );
            let matcher = NpmVersionMatcher;

            // "beta" should resolve to "5.0.0-beta.1" which is newer than latest stable
            let result = compare_version(&storer, &matcher, "lodash", "beta").unwrap();

            assert_eq!(result.status, VersionStatus::Newer);
            assert_eq!(result.current_version, "beta");
        }

        #[test]
        fn compare_version_returns_not_in_cache_for_unresolved_dist_tag() {
            let storer = MockStorer::with_dist_tags(
                Some("4.17.21"),
                vec!["4.17.20", "4.17.21"],
                std::collections::HashMap::new(), // No dist tags
            );
            let matcher = NpmVersionMatcher;

            // "latest" is a potential dist-tag, but we don't have dist-tag info
            // Return NotInCache to avoid confusing "Invalid version format" error
            let result = compare_version(&storer, &matcher, "lodash", "latest").unwrap();

            assert_eq!(result.status, VersionStatus::NotInCache);
        }

        #[test]
        fn compare_version_returns_not_in_cache_for_unresolved_beta_tag() {
            let storer = MockStorer::with_dist_tags(
                Some("4.17.21"),
                vec!["4.17.20", "4.17.21"],
                std::collections::HashMap::new(), // No dist tags
            );
            let matcher = NpmVersionMatcher;

            // "beta" is a potential dist-tag that we can't resolve
            let result = compare_version(&storer, &matcher, "lodash", "beta").unwrap();

            assert_eq!(result.status, VersionStatus::NotInCache);
        }

        #[test]
        fn compare_version_returns_invalid_for_truly_invalid_version() {
            let storer = MockStorer::with_dist_tags(
                Some("4.17.21"),
                vec!["4.17.20", "4.17.21"],
                std::collections::HashMap::new(),
            );
            let matcher = NpmVersionMatcher;

            // "invalid@#$" is not a valid semver and not a potential dist-tag
            let result = compare_version(&storer, &matcher, "lodash", "invalid@#$").unwrap();

            assert_eq!(result.status, VersionStatus::Invalid);
        }
    }
}
