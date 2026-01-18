//! Background refresh logic for package version cache

use std::time::Duration;

use futures::future::join_all;
use tokio::time::sleep;
use tracing::{debug, error, info};

use crate::config::FETCH_STAGGER_DELAY_MS;
use crate::parser::types::{PackageInfo, RegistryType};
use crate::version::cache::PackageId;
use crate::version::checker::VersionStorer;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;

/// Fetch and cache a single package's versions
///
/// Handles:
/// - Acquiring fetch lock (prevents duplicate fetches)
/// - Fetching versions from registry
/// - Saving versions and dist tags to cache
/// - Releasing fetch lock
///
/// Returns true if the package was successfully fetched and cached.
async fn fetch_and_cache_package<S: VersionStorer>(
    storer: &S,
    registry: &dyn Registry,
    registry_type: RegistryType,
    package_name: &str,
) -> bool {
    let registry_type_str = registry_type.as_str();

    // Try to acquire fetch lock (returns false if another process is fetching)
    let can_fetch = storer
        .try_start_fetch(registry_type, package_name)
        .inspect_err(|e| {
            error!(
                "Failed to start fetch for {}/{}: {}",
                registry_type_str, package_name, e
            )
        })
        .unwrap_or(false);

    if !can_fetch {
        info!(
            "Skipping {}/{}: already being fetched by another process",
            registry_type_str, package_name
        );
        return false;
    }

    let success = match registry.fetch_all_versions(package_name).await {
        Ok(pkg_versions) => {
            let version_count = pkg_versions.versions.len();
            let save_result =
                storer.replace_versions(registry_type, package_name, pkg_versions.versions);

            if save_result
                .inspect_err(|e| {
                    error!(
                        "Failed to save versions for {}/{}: {}",
                        registry_type_str, package_name, e
                    );
                })
                .is_ok()
            {
                info!(
                    "Saved {} versions for {}/{}",
                    version_count, registry_type_str, package_name
                );

                // Save dist tags if available
                if !pkg_versions.dist_tags.is_empty() {
                    let _ = storer
                        .save_dist_tags(registry_type, package_name, &pkg_versions.dist_tags)
                        .inspect_err(|e| {
                            error!(
                                "Failed to save dist tags for {}/{}: {}",
                                registry_type_str, package_name, e
                            );
                        });
                }

                true
            } else {
                false
            }
        }
        Err(RegistryError::NotFound(_)) => {
            info!(
                "Package not found: {}/{}. Marking as not found to skip future fetches.",
                registry_type_str, package_name
            );
            let _ = storer
                .mark_not_found(registry_type, package_name)
                .inspect_err(|e| {
                    error!(
                        "Failed to mark {}/{} as not found: {}",
                        registry_type_str, package_name, e
                    )
                });
            false
        }
        Err(e) => {
            error!(
                "Failed to fetch versions for {}/{}: {}",
                registry_type_str, package_name, e
            );
            false
        }
    };

    // Release fetch lock (always call regardless of success/failure)
    let _ = storer
        .finish_fetch(registry_type, package_name)
        .inspect_err(|e| {
            error!(
                "Failed to finish fetch for {}/{}: {}",
                registry_type_str, package_name, e
            )
        });

    success
}

/// Refresh versions for packages that need updating
///
/// Fetches latest versions from the registry and updates the cache.
/// Uses try_start_fetch/finish_fetch to prevent duplicate fetches across processes.
/// Errors are logged but do not stop processing of other packages.
/// Fetches are executed in parallel with staggered start times to avoid rate limiting.
pub async fn refresh_packages<S: VersionStorer>(
    storer: &S,
    registry: &dyn Registry,
    packages: Vec<PackageId>,
) {
    let futures = packages.into_iter().enumerate().map(|(i, package)| {
        let delay = Duration::from_millis(FETCH_STAGGER_DELAY_MS * i as u64);
        async move {
            sleep(delay).await;
            fetch_and_cache_package(
                storer,
                registry,
                package.registry_type,
                &package.package_name,
            )
            .await;
        }
    });

    join_all(futures).await;
}

/// Fetch packages that are not in the cache (on-demand fetch)
///
/// Identifies packages not in cache, fetches from registry, and updates cache.
/// Uses try_start_fetch/finish_fetch to prevent duplicate fetches across processes.
/// Returns the list of packages that were successfully fetched and cached.
/// Fetches are executed in parallel with staggered start times to avoid rate limiting.
pub async fn fetch_missing_packages<S: VersionStorer>(
    storer: &S,
    registry: &dyn Registry,
    packages: &[PackageInfo],
) -> Vec<String> {
    if packages.is_empty() {
        return Vec::new();
    }

    // Get registry type from the first package (all packages should have the same registry type)
    let registry_type = packages[0].registry_type;

    // Get all package names for batch query
    let package_names: Vec<_> = packages.iter().map(|p| p.name.clone()).collect();
    debug!("Checking cache for packages: {:?}", package_names);

    // Filter to packages not in cache using batch WHERE IN query
    let not_in_cache = storer
        .filter_packages_not_in_cache(registry_type, &package_names)
        .inspect_err(|e| error!("Failed to filter packages not in cache: {}", e))
        .unwrap_or_default();
    debug!("Packages not in cache: {:?}", not_in_cache);

    // Create a HashSet for efficient lookup
    let not_in_cache_set: std::collections::HashSet<_> = not_in_cache.into_iter().collect();

    // Filter original packages to those not in cache
    let packages_to_fetch: Vec<_> = packages
        .iter()
        .filter(|p| not_in_cache_set.contains(&p.name))
        .collect();

    if packages_to_fetch.is_empty() {
        debug!("All packages are already in cache");
        return Vec::new();
    }

    let futures = packages_to_fetch
        .into_iter()
        .enumerate()
        .map(|(i, package)| {
            let delay = Duration::from_millis(FETCH_STAGGER_DELAY_MS * i as u64);
            let package_name = package.name.clone();
            async move {
                sleep(delay).await;
                info!(
                    "Fetching missing package {}/{} from registry",
                    package.registry_type.as_str(),
                    package.name
                );
                let success =
                    fetch_and_cache_package(storer, registry, package.registry_type, &package.name)
                        .await;
                if success { Some(package_name) } else { None }
            }
        });

    join_all(futures).await.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::types::RegistryType;
    use crate::version::cache::Cache;
    use crate::version::registry::MockRegistry;
    use crate::version::types::PackageVersions;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_cache() -> (TempDir, Arc<Cache>) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400000, false).unwrap();
        (temp_dir, Arc::new(cache))
    }

    fn make_package_info(name: &str, version: &str) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            commit_hash: None,
            registry_type: RegistryType::GitHubActions,
            start_offset: 0,
            end_offset: version.len(),
            line: 0,
            column: 0,
            extra_info: None,
        }
    }

    #[tokio::test]
    async fn refresh_packages_fetches_versions_from_registry_and_saves_to_cache() {
        let (_temp_dir, cache) = create_test_cache();

        let mut registry = MockRegistry::new();
        registry
            .expect_registry_type()
            .returning(|| RegistryType::GitHubActions);
        registry
            .expect_fetch_all_versions()
            .withf(|name| name == "actions/checkout")
            .times(1)
            .returning(|_| {
                Ok(PackageVersions::new(vec![
                    "v4.0.0".to_string(),
                    "v3.0.0".to_string(),
                ]))
            });

        let packages = vec![PackageId {
            registry_type: RegistryType::GitHubActions,
            package_name: "actions/checkout".to_string(),
        }];

        refresh_packages(&*cache, &registry, packages).await;

        // Verify versions were saved to cache
        let mut versions = cache
            .get_versions(RegistryType::GitHubActions, "actions/checkout")
            .unwrap();
        versions.sort();
        assert_eq!(versions, vec!["v3.0.0", "v4.0.0"]);
    }

    #[tokio::test]
    async fn refresh_packages_continues_on_registry_error() {
        let (_temp_dir, cache) = create_test_cache();

        let mut registry = MockRegistry::new();
        registry
            .expect_registry_type()
            .returning(|| RegistryType::GitHubActions);

        // First package fails
        registry
            .expect_fetch_all_versions()
            .withf(|name| name == "failing/repo")
            .times(1)
            .returning(|_| {
                Err(crate::version::error::RegistryError::NotFound(
                    "failing/repo".to_string(),
                ))
            });

        // Second package succeeds
        registry
            .expect_fetch_all_versions()
            .withf(|name| name == "actions/checkout")
            .times(1)
            .returning(|_| Ok(PackageVersions::new(vec!["v4.0.0".to_string()])));

        let packages = vec![
            PackageId {
                registry_type: RegistryType::GitHubActions,
                package_name: "failing/repo".to_string(),
            },
            PackageId {
                registry_type: RegistryType::GitHubActions,
                package_name: "actions/checkout".to_string(),
            },
        ];

        refresh_packages(&*cache, &registry, packages).await;

        // First package should not be in cache
        let failing_versions = cache
            .get_versions(RegistryType::GitHubActions, "failing/repo")
            .unwrap();
        assert!(failing_versions.is_empty());

        // Second package should be saved
        let checkout_versions = cache
            .get_versions(RegistryType::GitHubActions, "actions/checkout")
            .unwrap();
        assert_eq!(checkout_versions, vec!["v4.0.0"]);
    }

    #[tokio::test]
    async fn refresh_packages_handles_empty_package_list() {
        let (_temp_dir, cache) = create_test_cache();

        let mut registry = MockRegistry::new();
        registry
            .expect_registry_type()
            .returning(|| RegistryType::GitHubActions);
        // fetch_all_versions should never be called
        registry.expect_fetch_all_versions().times(0);

        let packages = vec![];

        refresh_packages(&*cache, &registry, packages).await;
        // No panic, no error
    }

    #[tokio::test]
    async fn fetch_missing_packages_fetches_packages_not_in_cache() {
        let (_temp_dir, cache) = create_test_cache();

        let mut registry = MockRegistry::new();
        registry
            .expect_registry_type()
            .returning(|| RegistryType::GitHubActions);
        registry
            .expect_fetch_all_versions()
            .withf(|name| name == "actions/checkout")
            .times(1)
            .returning(|_| {
                Ok(PackageVersions::new(vec![
                    "v4.0.0".to_string(),
                    "v3.0.0".to_string(),
                ]))
            });

        let packages = vec![make_package_info("actions/checkout", "v3.0.0")];

        let fetched = fetch_missing_packages(&*cache, &registry, &packages).await;

        assert_eq!(fetched, vec!["actions/checkout"]);

        // Verify versions were saved to cache
        let versions = cache
            .get_versions(RegistryType::GitHubActions, "actions/checkout")
            .unwrap();
        assert!(!versions.is_empty());
    }

    #[tokio::test]
    async fn fetch_missing_packages_skips_packages_already_in_cache() {
        let (_temp_dir, cache) = create_test_cache();

        // Pre-populate cache
        cache
            .replace_versions(
                RegistryType::GitHubActions,
                "actions/checkout",
                vec!["v4.0.0".to_string()],
            )
            .unwrap();

        let mut registry = MockRegistry::new();
        registry
            .expect_registry_type()
            .returning(|| RegistryType::GitHubActions);
        // fetch_all_versions should NOT be called for cached package
        registry.expect_fetch_all_versions().times(0);

        let packages = vec![make_package_info("actions/checkout", "v3.0.0")];

        let fetched = fetch_missing_packages(&*cache, &registry, &packages).await;

        // No packages should be fetched
        assert!(fetched.is_empty());
    }

    #[tokio::test]
    async fn fetch_missing_packages_fetches_only_uncached_packages() {
        let (_temp_dir, cache) = create_test_cache();

        // Pre-populate cache with one package
        cache
            .replace_versions(
                RegistryType::GitHubActions,
                "actions/checkout",
                vec!["v4.0.0".to_string()],
            )
            .unwrap();

        let mut registry = MockRegistry::new();
        registry
            .expect_registry_type()
            .returning(|| RegistryType::GitHubActions);
        // Only the uncached package should be fetched
        registry
            .expect_fetch_all_versions()
            .withf(|name| name == "actions/setup-node")
            .times(1)
            .returning(|_| Ok(PackageVersions::new(vec!["v4.0.0".to_string()])));

        let packages = vec![
            make_package_info("actions/checkout", "v3.0.0"),
            make_package_info("actions/setup-node", "v3.0.0"),
        ];

        let fetched = fetch_missing_packages(&*cache, &registry, &packages).await;

        assert_eq!(fetched, vec!["actions/setup-node"]);

        // Verify only setup-node was fetched
        let setup_node_versions = cache
            .get_versions(RegistryType::GitHubActions, "actions/setup-node")
            .unwrap();
        assert!(!setup_node_versions.is_empty());
    }
}
