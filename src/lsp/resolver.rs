//! Package resolution coordinator for a specific registry type
//!
//! Groups parser, matcher, and registry components that work together
//! to resolve and validate package versions.

use std::collections::HashMap;
use std::sync::Arc;

use crate::parser::cargo_toml::CargoTomlParser;
use crate::parser::deno_json::DenoJsonParser;
use crate::parser::github_actions::GitHubActionsParser;
use crate::parser::go_mod::GoModParser;
use crate::parser::package_json::PackageJsonParser;
use crate::parser::pnpm_workspace::PnpmWorkspaceParser;
use crate::parser::traits::Parser;
use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::matchers::{
    CratesVersionMatcher, GitHubActionsMatcher, GoVersionMatcher, JsrVersionMatcher,
    NpmVersionMatcher, PnpmCatalogMatcher,
};
use crate::version::registries::crates_io::CratesIoRegistry;
use crate::version::registries::github::GitHubRegistry;
use crate::version::registries::go_proxy::GoProxyRegistry;
use crate::version::registries::jsr::JsrRegistry;
use crate::version::registries::npm::NpmRegistry;
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
        }
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
}

/// Create the default set of package resolvers for all supported registry types
pub fn create_default_resolvers() -> HashMap<RegistryType, PackageResolver> {
    let mut resolvers = HashMap::new();
    let npm_restistry = NpmRegistry::default();

    resolvers.insert(
        RegistryType::GitHubActions,
        PackageResolver::new(
            Arc::new(GitHubActionsParser::new()),
            Arc::new(GitHubActionsMatcher),
            Arc::new(GitHubRegistry::default()),
        ),
    );

    resolvers.insert(
        RegistryType::Npm,
        PackageResolver::new(
            Arc::new(PackageJsonParser::new()),
            Arc::new(NpmVersionMatcher),
            Arc::new(npm_restistry.clone()),
        ),
    );

    resolvers.insert(
        RegistryType::CratesIo,
        PackageResolver::new(
            Arc::new(CargoTomlParser::new()),
            Arc::new(CratesVersionMatcher),
            Arc::new(CratesIoRegistry::default()),
        ),
    );

    resolvers.insert(
        RegistryType::GoProxy,
        PackageResolver::new(
            Arc::new(GoModParser::new()),
            Arc::new(GoVersionMatcher),
            Arc::new(GoProxyRegistry::default()),
        ),
    );

    resolvers.insert(
        RegistryType::PnpmCatalog,
        PackageResolver::new(
            Arc::new(PnpmWorkspaceParser),
            Arc::new(PnpmCatalogMatcher),
            Arc::new(npm_restistry),
        ),
    );

    resolvers.insert(
        RegistryType::Jsr,
        PackageResolver::new(
            Arc::new(DenoJsonParser::new()),
            Arc::new(JsrVersionMatcher),
            Arc::new(JsrRegistry::default()),
        ),
    );

    resolvers
}
