//! Registry test utilities

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tempfile::TempDir;

use version_lsp::lsp::resolver::PackageResolver;
use version_lsp::parser::cargo_toml::CargoTomlParser;
use version_lsp::parser::deno_json::DenoJsonParser;
use version_lsp::parser::github_actions::GitHubActionsParser;
use version_lsp::parser::go_mod::GoModParser;
use version_lsp::parser::package_json::PackageJsonParser;
use version_lsp::parser::pnpm_workspace::PnpmWorkspaceParser;
use version_lsp::parser::types::RegistryType;
use version_lsp::version::cache::Cache;
use version_lsp::version::checker::VersionStorer;
use version_lsp::version::error::RegistryError;
use version_lsp::version::matchers::{
    CratesVersionMatcher, GitHubActionsMatcher, GoVersionMatcher, JsrVersionMatcher,
    NpmVersionMatcher, PnpmCatalogMatcher,
};
use version_lsp::version::registry::Registry;
use version_lsp::version::types::PackageVersions;

/// Mock registry for testing
pub struct MockRegistry {
    registry_type: RegistryType,
    versions: HashMap<String, Vec<String>>,
}

impl MockRegistry {
    pub fn new(registry_type: RegistryType) -> Self {
        Self {
            registry_type,
            versions: HashMap::new(),
        }
    }

    pub fn with_versions(mut self, package: &str, versions: Vec<&str>) -> Self {
        self.versions.insert(
            package.to_string(),
            versions.into_iter().map(|v| v.to_string()).collect(),
        );
        self
    }
}

#[async_trait]
impl Registry for MockRegistry {
    fn registry_type(&self) -> RegistryType {
        self.registry_type
    }

    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError> {
        match self.versions.get(package_name) {
            Some(versions) => Ok(PackageVersions::new(versions.clone())),
            None => Err(RegistryError::NotFound(package_name.to_string())),
        }
    }
}

/// Create a test resolver for the given registry type with a mock registry
pub fn create_test_resolver(
    registry_type: RegistryType,
    mock_registry: MockRegistry,
) -> PackageResolver {
    match registry_type {
        RegistryType::GitHubActions => PackageResolver::new(
            Arc::new(GitHubActionsParser::new()),
            Arc::new(GitHubActionsMatcher),
            Arc::new(mock_registry),
        ),
        RegistryType::Npm => PackageResolver::new(
            Arc::new(PackageJsonParser::new()),
            Arc::new(NpmVersionMatcher),
            Arc::new(mock_registry),
        ),
        RegistryType::CratesIo => PackageResolver::new(
            Arc::new(CargoTomlParser::new()),
            Arc::new(CratesVersionMatcher),
            Arc::new(mock_registry),
        ),
        RegistryType::GoProxy => PackageResolver::new(
            Arc::new(GoModParser::new()),
            Arc::new(GoVersionMatcher),
            Arc::new(mock_registry),
        ),
        RegistryType::PnpmCatalog => PackageResolver::new(
            Arc::new(PnpmWorkspaceParser),
            Arc::new(PnpmCatalogMatcher),
            Arc::new(mock_registry),
        ),
        RegistryType::Jsr => PackageResolver::new(
            Arc::new(DenoJsonParser::new()),
            Arc::new(JsrVersionMatcher),
            Arc::new(mock_registry),
        ),
    }
}

/// Create a test cache with pre-populated versions
pub fn create_test_cache(
    registry_type: RegistryType,
    versions: &[(&str, Vec<&str>)],
) -> (TempDir, Arc<Cache>) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let cache = Cache::new(&db_path, 86400000).unwrap();

    for (package_name, package_versions) in versions {
        cache
            .replace_versions(
                registry_type,
                package_name,
                package_versions.iter().map(|v| v.to_string()).collect(),
            )
            .unwrap();
    }

    (temp_dir, Arc::new(cache))
}
