//! Package resolution coordinator for a specific registry type
//!
//! Groups parser, matcher, and registry components that work together
//! to resolve and validate package versions.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::{LspConfig, RegistryConfig};
use crate::parser::cargo_toml::CargoTomlParser;
use crate::parser::compose::ComposeParser;
use crate::parser::deno_json::DenoJsonParser;
use crate::parser::github_actions::GitHubActionsParser;
use crate::parser::go_mod::GoModParser;
use crate::parser::package_json::PackageJsonParser;
use crate::parser::pnpm_workspace::PnpmWorkspaceParser;
use crate::parser::pyproject_toml::PyprojectTomlParser;
use crate::parser::traits::Parser;
use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::matchers::{
    CratesVersionMatcher, DockerVersionMatcher, GitHubActionsMatcher, GoVersionMatcher,
    JsrVersionMatcher, NpmVersionMatcher, PnpmCatalogMatcher, PypiVersionMatcher,
};
use crate::version::registries::crates_io::CratesIoRegistry;
use crate::version::registries::docker::DockerRegistry;
use crate::version::registries::github::{GitHubRegistry, TagShaFetcher};
use crate::version::registries::go_proxy::GoProxyRegistry;
use crate::version::registries::jsr::JsrRegistry;
use crate::version::registries::npm::NpmRegistry;
use crate::version::registries::pypi::PypiRegistry;
use crate::version::registry::Registry;

/// Groups all components needed to resolve and validate package versions for a specific registry.
///
/// Each registry type (Npm, CratesIo, GoProxy, GitHubActions) has one PackageResolver instance
/// that coordinates:
/// - Parsing files to extract package information
/// - Matching version specifications against available versions
/// - Fetching package versions from the remote registry
pub struct PackageResolver {
    parser: Arc<dyn Parser>,
    matcher: Arc<dyn VersionMatcher>,
    registry: Arc<dyn Registry>,
    sha_fetcher: Option<Arc<dyn TagShaFetcher>>,
}

impl PackageResolver {
    /// Create a new PackageResolver with the given components
    pub fn new(
        parser: Arc<dyn Parser>,
        matcher: Arc<dyn VersionMatcher>,
        registry: Arc<dyn Registry>,
    ) -> Self {
        Self {
            parser,
            matcher,
            registry,
            sha_fetcher: None,
        }
    }

    /// Attach a tag-SHA fetcher (used by GitHub Actions to resolve commit
    /// hashes). Keeping it on the resolver ensures the configured registry URL
    /// override is honored wherever SHA fetching happens.
    pub fn with_sha_fetcher(mut self, sha_fetcher: Arc<dyn TagShaFetcher>) -> Self {
        self.sha_fetcher = Some(sha_fetcher);
        self
    }

    /// Get the parser for this registry type
    pub fn parser(&self) -> &Arc<dyn Parser> {
        &self.parser
    }

    /// Get the version matcher for this registry type
    pub fn matcher(&self) -> &Arc<dyn VersionMatcher> {
        &self.matcher
    }

    /// Get the registry for fetching versions
    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }

    /// Get the tag-SHA fetcher, if this resolver provides one
    pub fn sha_fetcher(&self) -> Option<&Arc<dyn TagShaFetcher>> {
        self.sha_fetcher.as_ref()
    }
}

/// Build the set of package resolvers for all supported registry types using
/// URL overrides from the supplied configuration. Any registry whose
/// [`RegistryConfig::url`] is `None` uses its hardcoded default URL.
pub fn create_resolvers(config: &LspConfig) -> HashMap<RegistryType, PackageResolver> {
    let registries = &config.registries;
    let mut resolvers = HashMap::new();

    // Single shared NpmRegistry for both Npm and PnpmCatalog. They map to
    // separate config keys so a user could override them independently, but
    // sharing the instance when both URLs match avoids duplicate HTTP clients.
    // We accept the rare case where they differ by building two clients.
    let npm_registry = npm_registry_from(&registries.npm);

    // One GitHubRegistry instance serves both the version fetch (Registry) and
    // the commit-hash → SHA fetch (TagShaFetcher) so the configured URL
    // override is honored on both paths.
    let github_registry = Arc::new(github_registry_from(&registries.github));

    resolvers.insert(
        RegistryType::GitHubActions,
        PackageResolver::new(
            Arc::new(GitHubActionsParser::new()),
            Arc::new(GitHubActionsMatcher),
            github_registry.clone(),
        )
        .with_sha_fetcher(github_registry),
    );

    resolvers.insert(
        RegistryType::Npm,
        PackageResolver::new(
            Arc::new(PackageJsonParser::new()),
            Arc::new(NpmVersionMatcher),
            Arc::new(npm_registry.clone()),
        ),
    );

    resolvers.insert(
        RegistryType::CratesIo,
        PackageResolver::new(
            Arc::new(CargoTomlParser::new()),
            Arc::new(CratesVersionMatcher),
            Arc::new(crates_registry_from(&registries.crates)),
        ),
    );

    resolvers.insert(
        RegistryType::GoProxy,
        PackageResolver::new(
            Arc::new(GoModParser::new()),
            Arc::new(GoVersionMatcher),
            Arc::new(go_proxy_registry_from(&registries.go_proxy)),
        ),
    );

    // pnpm catalog reuses the npm registry. If the user overrides the
    // pnpmCatalog URL independently of npm, build a second NpmRegistry.
    let pnpm_registry = if registries.pnpm_catalog.url == registries.npm.url {
        npm_registry
    } else {
        npm_registry_from(&registries.pnpm_catalog)
    };

    resolvers.insert(
        RegistryType::PnpmCatalog,
        PackageResolver::new(
            Arc::new(PnpmWorkspaceParser),
            Arc::new(PnpmCatalogMatcher),
            Arc::new(pnpm_registry),
        ),
    );

    resolvers.insert(
        RegistryType::Jsr,
        PackageResolver::new(
            Arc::new(DenoJsonParser::new()),
            Arc::new(JsrVersionMatcher),
            Arc::new(jsr_registry_from(&registries.jsr)),
        ),
    );

    resolvers.insert(
        RegistryType::PyPI,
        PackageResolver::new(
            Arc::new(PyprojectTomlParser::new()),
            Arc::new(PypiVersionMatcher),
            Arc::new(pypi_registry_from(&registries.pypi)),
        ),
    );

    resolvers.insert(
        RegistryType::Docker,
        PackageResolver::new(
            Arc::new(ComposeParser::new()),
            Arc::new(DockerVersionMatcher),
            Arc::new(DockerRegistry::with_overrides(
                registries.docker.docker_hub_registry_url.as_deref(),
                registries.docker.docker_hub_auth_url.as_deref(),
                registries.docker.ghcr_registry_url.as_deref(),
                registries.docker.ghcr_auth_url.as_deref(),
            )),
        ),
    );

    resolvers
}

/// Build the default set of resolvers (no URL overrides). Equivalent to
/// `create_resolvers(&LspConfig::default())`.
pub fn create_default_resolvers() -> HashMap<RegistryType, PackageResolver> {
    create_resolvers(&LspConfig::default())
}

fn pypi_registry_from(cfg: &RegistryConfig) -> PypiRegistry {
    cfg.url
        .as_deref()
        .map(|u| PypiRegistry::new(u.to_string()))
        .unwrap_or_default()
}

fn npm_registry_from(cfg: &RegistryConfig) -> NpmRegistry {
    cfg.url.as_deref().map(NpmRegistry::new).unwrap_or_default()
}

fn crates_registry_from(cfg: &RegistryConfig) -> CratesIoRegistry {
    cfg.url
        .as_deref()
        .map(CratesIoRegistry::new)
        .unwrap_or_default()
}

fn go_proxy_registry_from(cfg: &RegistryConfig) -> GoProxyRegistry {
    cfg.url
        .as_deref()
        .map(GoProxyRegistry::new)
        .unwrap_or_default()
}

fn jsr_registry_from(cfg: &RegistryConfig) -> JsrRegistry {
    cfg.url.as_deref().map(JsrRegistry::new).unwrap_or_default()
}

/// Build a `GitHubRegistry`. LSP config takes precedence over the
/// `GITHUB_API_BASE_URL` environment variable (which is preserved as a
/// fallback for backward compatibility), which in turn takes precedence over
/// the hardcoded default.
fn github_registry_from(cfg: &RegistryConfig) -> GitHubRegistry {
    if let Some(url) = cfg.url.as_deref() {
        GitHubRegistry::new(url)
    } else {
        GitHubRegistry::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DockerRegistryConfig, RegistriesConfig};

    #[test]
    fn create_resolvers_with_default_config_includes_all_registry_types() {
        let resolvers = create_resolvers(&LspConfig::default());

        for registry_type in [
            RegistryType::Npm,
            RegistryType::CratesIo,
            RegistryType::GoProxy,
            RegistryType::GitHubActions,
            RegistryType::PnpmCatalog,
            RegistryType::Jsr,
            RegistryType::PyPI,
            RegistryType::Docker,
        ] {
            assert!(
                resolvers.contains_key(&registry_type),
                "missing resolver for {:?}",
                registry_type
            );
        }
    }

    #[tokio::test]
    async fn create_resolvers_routes_pypi_fetches_to_overridden_url() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/pypi/requests/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"info":{"version":"1.0.0"},"releases":{"1.0.0":[]}}"#)
            .create_async()
            .await;

        let mut config = LspConfig::default();
        config.registries.pypi.url = Some(server.url());

        let resolvers = create_resolvers(&config);
        let registry = resolvers
            .get(&RegistryType::PyPI)
            .expect("PyPI resolver missing")
            .registry();

        let result = registry.fetch_all_versions("requests").await.unwrap();

        mock.assert_async().await;
        assert_eq!(result.versions, vec!["1.0.0"]);
    }

    #[tokio::test]
    async fn create_resolvers_routes_github_sha_fetches_to_overridden_url() {
        use crate::version::registries::github::TagShaFetcher;

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/repos/actions/checkout/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[{"name":"v4.1.7","commit":{"sha":"newsha4170000000000000000000000000000000"}}]"#)
            .create_async()
            .await;

        let mut config = LspConfig::default();
        config.registries.github.url = Some(server.url());

        let resolvers = create_resolvers(&config);
        let sha_fetcher = resolvers
            .get(&RegistryType::GitHubActions)
            .expect("GitHubActions resolver missing")
            .sha_fetcher()
            .expect("GitHubActions resolver missing SHA fetcher");

        let sha = sha_fetcher
            .fetch_tag_sha("actions/checkout", "v4.1.7")
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(sha, "newsha4170000000000000000000000000000000");
    }

    #[tokio::test]
    async fn create_resolvers_routes_npm_fetches_to_overridden_url() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/lodash")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"versions":{"1.0.0":{}},"dist-tags":{"latest":"1.0.0"},"time":{}}"#)
            .create_async()
            .await;

        let mut config = LspConfig::default();
        config.registries.npm.url = Some(server.url());

        let resolvers = create_resolvers(&config);
        let registry = resolvers
            .get(&RegistryType::Npm)
            .expect("Npm resolver missing")
            .registry();

        let result = registry.fetch_all_versions("lodash").await.unwrap();

        mock.assert_async().await;
        assert_eq!(result.versions, vec!["1.0.0"]);
    }

    #[tokio::test]
    async fn create_resolvers_uses_independent_npm_and_pnpm_urls_when_different() {
        let mut npm_server = mockito::Server::new_async().await;
        let mut pnpm_server = mockito::Server::new_async().await;

        let npm_mock = npm_server
            .mock("GET", "/lodash")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"versions":{"4.0.0":{}},"dist-tags":{"latest":"4.0.0"},"time":{}}"#)
            .create_async()
            .await;
        let pnpm_mock = pnpm_server
            .mock("GET", "/lodash")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"versions":{"5.0.0":{}},"dist-tags":{"latest":"5.0.0"},"time":{}}"#)
            .create_async()
            .await;

        let config = LspConfig {
            registries: RegistriesConfig {
                npm: RegistryConfig {
                    enabled: true,
                    url: Some(npm_server.url()),
                },
                pnpm_catalog: RegistryConfig {
                    enabled: true,
                    url: Some(pnpm_server.url()),
                },
                ..RegistriesConfig::default()
            },
            ..LspConfig::default()
        };

        let resolvers = create_resolvers(&config);

        let npm = resolvers.get(&RegistryType::Npm).unwrap().registry();
        let pnpm = resolvers
            .get(&RegistryType::PnpmCatalog)
            .unwrap()
            .registry();

        let npm_result = npm.fetch_all_versions("lodash").await.unwrap();
        let pnpm_result = pnpm.fetch_all_versions("lodash").await.unwrap();

        npm_mock.assert_async().await;
        pnpm_mock.assert_async().await;
        assert_eq!(npm_result.versions, vec!["4.0.0"]);
        assert_eq!(pnpm_result.versions, vec!["5.0.0"]);
    }

    #[test]
    fn docker_with_overrides_applies_partial_overrides_from_config() {
        // We can't easily HTTP-test Docker here (it makes auth + tag calls in
        // sequence and parsing is complex), so just verify the config path
        // builds a resolver without panicking when partial overrides are set.
        let config = LspConfig {
            registries: RegistriesConfig {
                docker: DockerRegistryConfig {
                    enabled: true,
                    docker_hub_registry_url: Some("https://hub.example.com".to_string()),
                    docker_hub_auth_url: None,
                    ghcr_registry_url: None,
                    ghcr_auth_url: None,
                },
                ..RegistriesConfig::default()
            },
            ..LspConfig::default()
        };

        let resolvers = create_resolvers(&config);
        assert!(resolvers.contains_key(&RegistryType::Docker));
    }
}
