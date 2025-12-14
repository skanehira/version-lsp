//! Background refresh logic for package version cache

use tracing::{error, info};

use crate::parser::types::PackageInfo;
use crate::version::cache::PackageId;
use crate::version::checker::VersionStorer;
use crate::version::registry::Registry;

/// Refresh versions for packages that need updating
///
/// Fetches latest versions from the registry and updates the cache.
/// Uses try_start_fetch/finish_fetch to prevent duplicate fetches across processes.
/// Errors are logged but do not stop processing of other packages.
pub async fn refresh_packages<S: VersionStorer>(
    storer: &S,
    registry: &dyn Registry,
    packages: Vec<PackageId>,
) {
    for package in packages {
        // Try to acquire fetch lock (returns false if another process is fetching)
        let can_fetch = storer
            .try_start_fetch(&package.registry_type, &package.package_name)
            .inspect_err(|e| {
                error!(
                    "Failed to start fetch for {}/{}: {}",
                    package.registry_type, package.package_name, e
                )
            })
            .unwrap_or(false);

        if !can_fetch {
            info!(
                "Skipping {}/{}: already being fetched by another process",
                package.registry_type, package.package_name
            );
            continue;
        }

        let result = registry.fetch_all_versions(&package.package_name).await;

        match result {
            Ok(versions) => {
                let save_result = storer.replace_versions(
                    &package.registry_type,
                    &package.package_name,
                    versions.versions,
                );
                if let Err(e) = save_result {
                    error!(
                        "Failed to save versions for {}/{}: {}",
                        package.registry_type, package.package_name, e
                    );
                }
            }
            Err(e) => {
                error!(
                    "Failed to fetch versions for {}/{}: {}",
                    package.registry_type, package.package_name, e
                );
            }
        }

        // Release fetch lock (always call regardless of success/failure)
        let _ = storer
            .finish_fetch(&package.registry_type, &package.package_name)
            .inspect_err(|e| {
                error!(
                    "Failed to finish fetch for {}/{}: {}",
                    package.registry_type, package.package_name, e
                )
            });
    }
}

/// Fetch packages that are not in the cache (on-demand fetch)
///
/// Identifies packages not in cache, fetches from registry, and updates cache.
/// Uses try_start_fetch/finish_fetch to prevent duplicate fetches across processes.
/// Returns the list of packages that were successfully fetched and cached.
pub async fn fetch_missing_packages<S: VersionStorer>(
    storer: &S,
    registry: &dyn Registry,
    packages: &[PackageInfo],
) -> Vec<String> {
    let mut fetched = Vec::new();

    for package in packages {
        let registry_type = package.registry_type.as_str();

        // Check if package is in cache
        let in_cache = storer
            .get_latest_version(registry_type, &package.name)
            .ok()
            .flatten()
            .is_some();

        if in_cache {
            continue;
        }

        // Try to acquire fetch lock (returns false if another process is fetching)
        let can_fetch = storer
            .try_start_fetch(registry_type, &package.name)
            .inspect_err(|e| {
                error!(
                    "Failed to start fetch for {}/{}: {}",
                    registry_type, package.name, e
                )
            })
            .unwrap_or(false);

        if !can_fetch {
            info!(
                "Skipping {}/{}: already being fetched by another process",
                registry_type, package.name
            );
            continue;
        }

        info!(
            "Fetching missing package {}/{} from registry",
            registry_type, package.name
        );

        let result = registry.fetch_all_versions(&package.name).await;

        match result {
            Ok(versions) => {
                let save_result =
                    storer.replace_versions(registry_type, &package.name, versions.versions);
                if save_result.is_ok() {
                    fetched.push(package.name.clone());
                } else if let Err(e) = save_result {
                    error!(
                        "Failed to save versions for {}/{}: {}",
                        registry_type, package.name, e
                    );
                }
            }
            Err(e) => {
                error!(
                    "Failed to fetch versions for {}/{}: {}",
                    registry_type, package.name, e
                );
            }
        }

        // Release fetch lock (always call regardless of success/failure)
        let _ = storer
            .finish_fetch(registry_type, &package.name)
            .inspect_err(|e| {
                error!(
                    "Failed to finish fetch for {}/{}: {}",
                    registry_type, package.name, e
                )
            });
    }

    fetched
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
        let cache = Cache::new(&db_path, 86400000).unwrap();
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
            registry_type: "github_actions".to_string(),
            package_name: "actions/checkout".to_string(),
        }];

        refresh_packages(&*cache, &registry, packages).await;

        // Verify versions were saved to cache
        let mut versions = cache
            .get_versions("github_actions", "actions/checkout")
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
                registry_type: "github_actions".to_string(),
                package_name: "failing/repo".to_string(),
            },
            PackageId {
                registry_type: "github_actions".to_string(),
                package_name: "actions/checkout".to_string(),
            },
        ];

        refresh_packages(&*cache, &registry, packages).await;

        // First package should not be in cache
        let failing_versions = cache
            .get_versions("github_actions", "failing/repo")
            .unwrap();
        assert!(failing_versions.is_empty());

        // Second package should be saved
        let checkout_versions = cache
            .get_versions("github_actions", "actions/checkout")
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
            .get_versions("github_actions", "actions/checkout")
            .unwrap();
        assert!(!versions.is_empty());
    }

    #[tokio::test]
    async fn fetch_missing_packages_skips_packages_already_in_cache() {
        let (_temp_dir, cache) = create_test_cache();

        // Pre-populate cache
        cache
            .replace_versions(
                "github_actions",
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
                "github_actions",
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
            .get_versions("github_actions", "actions/setup-node")
            .unwrap();
        assert!(!setup_node_versions.is_empty());
    }
}
