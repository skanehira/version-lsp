use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tracing::{debug, error, info, warn};

use crate::config::{LspConfig, data_dir, db_path};
use crate::lsp::code_action::{
    PackageIndex, generate_bump_code_actions, generate_bump_code_actions_with_sha,
};
use crate::lsp::diagnostics::generate_diagnostics;
use crate::lsp::refresh::{fetch_missing_packages, refresh_packages};
use crate::lsp::resolver::{PackageResolver, create_default_resolvers};
use crate::parser::types::{PackageInfo, RegistryType, detect_parser_type};
use crate::version::cache::Cache;
use crate::version::checker::VersionStorer;
use crate::version::registries::github::GitHubRegistry;
use crate::version::registry::Registry;

/// Cached parsed packages for a document
struct DocumentCache {
    packages: Vec<PackageInfo>,
}

pub struct Backend<S: VersionStorer> {
    client: Client,
    storer: Option<Arc<S>>,
    config: Arc<RwLock<LspConfig>>,
    resolvers: HashMap<RegistryType, PackageResolver>,
    documents: Arc<RwLock<HashMap<Url, DocumentCache>>>,
}

impl Backend<Cache> {
    pub fn new(client: Client) -> Self {
        let config = LspConfig::default();
        let storer = Self::initialize_storer(&config);
        let resolvers = create_default_resolvers();
        Self {
            client,
            storer,
            config: Arc::new(RwLock::new(config)),
            resolvers,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn initialize_storer(config: &LspConfig) -> Option<Arc<Cache>> {
        let data_dir = data_dir();
        let db_path = db_path();

        // Create data directory if it doesn't exist
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            error!("Failed to create data directory {:?}: {}", data_dir, e);
            return None;
        }

        match Cache::new(&db_path, config.cache.refresh_interval) {
            Ok(cache) => {
                info!("Cache initialized at {:?}", db_path);
                Some(Arc::new(cache))
            }
            Err(e) => {
                error!("Failed to initialize cache: {}", e);
                None
            }
        }
    }
}

impl<S: VersionStorer> Backend<S> {
    /// Build a Backend with custom storer and resolvers
    pub fn build(
        client: Client,
        storer: Arc<S>,
        resolvers: HashMap<RegistryType, PackageResolver>,
    ) -> Self {
        Self {
            client,
            storer: Some(storer),
            config: Arc::new(RwLock::new(LspConfig::default())),
            resolvers,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Parse document and cache packages
    fn cache_document(&self, uri: &Url, content: &str) {
        let uri_str = uri.as_str();
        let packages = detect_parser_type(uri_str)
            .and_then(|registry_type| self.resolvers.get(&registry_type))
            .map(|resolver| {
                resolver
                    .parser()
                    .parse(content)
                    .inspect_err(|e| warn!("Failed to parse {}: {}", uri_str, e))
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        let mut docs = self.documents.write().expect("documents lock poisoned");
        docs.insert(uri.clone(), DocumentCache { packages });
    }

    /// Check if a registry is enabled in the configuration
    fn is_registry_enabled(&self, registry_type: RegistryType) -> bool {
        let config = self.config.read().expect("config lock poisoned");
        match registry_type {
            RegistryType::Npm => config.registries.npm.enabled,
            RegistryType::CratesIo => config.registries.crates.enabled,
            RegistryType::GoProxy => config.registries.go_proxy.enabled,
            RegistryType::GitHubActions => config.registries.github.enabled,
            RegistryType::PnpmCatalog => config.registries.pnpm_catalog.enabled,
            RegistryType::Jsr => config.registries.jsr.enabled,
        }
    }

    /// Spawn background task to fetch configuration from client
    fn spawn_fetch_configuration(&self) {
        let client = self.client.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            let items = vec![ConfigurationItem {
                scope_uri: None,
                section: Some("version-lsp".to_string()),
            }];

            match client.configuration(items).await {
                Ok(configs) => {
                    if let Some(config_value) = configs.into_iter().next() {
                        // Handle null/empty configuration by using defaults
                        let new_config = if config_value.is_null() {
                            LspConfig::default()
                        } else {
                            match serde_json::from_value::<LspConfig>(config_value) {
                                Ok(c) => c,
                                Err(e) => {
                                    let msg = format!("Failed to parse configuration: {}", e);
                                    warn!("{}", msg);
                                    client.show_message(MessageType::ERROR, msg).await;
                                    return;
                                }
                            }
                        };
                        info!("Configuration updated: {:?}", new_config);
                        let mut cfg = config.write().expect("config lock poisoned");
                        *cfg = new_config;
                    }
                }
                Err(e) => {
                    // Client may not support workspace/configuration, which is fine
                    debug!("workspace/configuration request failed: {}", e);
                }
            }
        });
    }

    pub fn server_capabilities() -> ServerCapabilities {
        ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::FULL),
                    ..Default::default()
                },
            )),
            code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
            ..Default::default()
        }
    }

    fn spawn_background_refresh(&self) {
        let Some(storer) = self.storer.clone() else {
            warn!("Storer not available, skipping background refresh");
            return;
        };

        // Extract registries from resolvers for background task
        let registries: HashMap<RegistryType, Arc<dyn Registry>> = self
            .resolvers
            .iter()
            .map(|(k, v)| (*k, v.registry().clone()))
            .collect();

        tokio::spawn(async move {
            let Some(packages) = storer
                .get_packages_needing_refresh()
                .inspect_err(|e| error!("Failed to get packages needing refresh: {}", e))
                .ok()
            else {
                return;
            };

            if packages.is_empty() {
                info!("No packages need refresh");
                return;
            }

            info!("{} packages need refresh", packages.len());

            // Group packages by registry type
            let mut packages_by_registry: HashMap<RegistryType, Vec<_>> = HashMap::new();
            for package in packages {
                packages_by_registry
                    .entry(package.registry_type)
                    .or_default()
                    .push(package);
            }

            // Refresh packages for each registry type
            for (registry_type, packages) in packages_by_registry {
                if let Some(registry) = registries.get(&registry_type) {
                    refresh_packages(&*storer, &**registry, packages).await;
                }
            }
        });
    }

    async fn check_and_publish_diagnostics(&self, uri: Url, content: String) {
        let uri_str = uri.as_str();
        debug!("Checking diagnostics for URI: {}", uri_str);

        let Some(registry_type) = detect_parser_type(uri_str) else {
            debug!("No parser type detected for URI: {}", uri_str);
            return;
        };
        debug!("Detected registry type: {:?}", registry_type);

        // Skip if registry is disabled
        if !self.is_registry_enabled(registry_type) {
            debug!(
                "Registry {:?} is disabled, skipping diagnostics",
                registry_type
            );
            return;
        }

        let Some(resolver) = self.resolvers.get(&registry_type) else {
            debug!("No resolver found for registry type: {:?}", registry_type);
            return;
        };

        let Some(storer) = &self.storer else {
            self.client
                .show_message(
                    MessageType::WARNING,
                    "Cache not available, version checking disabled",
                )
                .await;
            return;
        };

        // Parse document to get packages (needed for on-demand fetch)
        let packages = resolver
            .parser()
            .parse(&content)
            .inspect_err(|e| warn!("Failed to parse {}: {}", uri_str, e))
            .unwrap_or_default();
        debug!("Parsed {} packages: {:?}", packages.len(), packages);

        let diagnostics = generate_diagnostics(
            &**resolver.parser(),
            &**resolver.matcher(),
            &**storer,
            &content,
        );

        self.client
            .log_message(
                MessageType::LOG,
                format!(
                    "Publishing {} diagnostics for {}",
                    diagnostics.len(),
                    uri_str
                ),
            )
            .await;

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;

        // Spawn background task to fetch missing packages
        if !packages.is_empty() {
            debug!(
                "Spawning background task to fetch {} packages",
                packages.len()
            );
            let registry = resolver.registry().clone();
            let storer = storer.clone();
            let client = self.client.clone();
            let parser = resolver.parser().clone();
            let matcher = resolver.matcher().clone();

            tokio::spawn(async move {
                debug!("Background task started for fetching packages");
                let fetched = fetch_missing_packages(&*storer, &*registry, &packages).await;
                debug!("fetch_missing_packages returned {} packages", fetched.len());

                if !fetched.is_empty() {
                    client
                        .log_message(
                            MessageType::LOG,
                            format!(
                                "Fetched {} missing packages, republishing diagnostics",
                                fetched.len()
                            ),
                        )
                        .await;

                    let diagnostics = generate_diagnostics(&*parser, &*matcher, &*storer, &content);

                    client.publish_diagnostics(uri, diagnostics, None).await;
                }
            });
        }
    }
}

#[tower_lsp::async_trait]
impl<S: VersionStorer> LanguageServer for Backend<S> {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        self.client
            .log_message(MessageType::INFO, "LSP server initializing")
            .await;
        Ok(InitializeResult {
            capabilities: Self::server_capabilities(),
            server_info: Some(ServerInfo {
                name: "version-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "LSP server initialized")
            .await;

        // Request configuration from client via workspace/configuration (non-blocking)
        self.spawn_fetch_configuration();

        self.spawn_background_refresh();
    }

    async fn shutdown(&self) -> Result<()> {
        self.client
            .log_message(MessageType::INFO, "LSP server shutting down")
            .await;
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.client
            .log_message(
                MessageType::LOG,
                format!("Document opened: {}", params.text_document.uri),
            )
            .await;

        // Parse and cache packages
        self.cache_document(&params.text_document.uri, &params.text_document.text);

        self.check_and_publish_diagnostics(params.text_document.uri, params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // With FULL sync mode, the last content change contains the full document text
        let Some(content) = params.content_changes.into_iter().last().map(|c| c.text) else {
            return;
        };

        self.client
            .log_message(
                MessageType::LOG,
                format!("Document changed: {}", params.text_document.uri),
            )
            .await;

        // Re-parse and cache packages
        self.cache_document(&params.text_document.uri, &content);

        self.check_and_publish_diagnostics(params.text_document.uri, content)
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.client
            .log_message(
                MessageType::LOG,
                format!("Document closed: {}", params.text_document.uri),
            )
            .await;

        // Remove document from cache
        {
            let mut docs = self.documents.write().expect("documents lock poisoned");
            docs.remove(&params.text_document.uri);
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let uri_str = uri.as_str();
        debug!("Code action requested for URI: {}", uri_str);

        let Some(registry_type) = detect_parser_type(uri_str) else {
            debug!("No parser type detected for URI: {}", uri_str);
            return Ok(None);
        };

        if !self.is_registry_enabled(registry_type) {
            debug!(
                "Registry {:?} is disabled, skipping code actions",
                registry_type
            );
            return Ok(None);
        }

        let Some(storer) = &self.storer else {
            debug!("Storer not available");
            return Ok(None);
        };

        // Get cached packages
        let packages = {
            let docs = self.documents.read().expect("documents lock poisoned");
            let Some(cache) = docs.get(uri) else {
                debug!("Document not found in cache: {}", uri_str);
                return Ok(None);
            };
            cache.packages.clone()
        };

        if packages.is_empty() {
            return Ok(None);
        }

        let index = PackageIndex::new(&packages);
        let position = params.range.start;

        let Some(package) = index.find_at_position(position) else {
            debug!("No package found at position {:?}", position);
            return Ok(None);
        };

        debug!(
            "Found package at cursor: {} {}",
            package.name, package.version
        );

        // For GitHub Actions with commit hash, use async function to fetch SHA
        let actions = if package.registry_type == RegistryType::GitHubActions
            && package.commit_hash.is_some()
        {
            let sha_fetcher = GitHubRegistry::default();
            generate_bump_code_actions_with_sha(&**storer, package, uri, &sha_fetcher).await
        } else {
            generate_bump_code_actions(&**storer, package, uri)
        };

        if actions.is_empty() {
            return Ok(None);
        }

        Ok(Some(
            actions
                .into_iter()
                .map(CodeActionOrCommand::CodeAction)
                .collect(),
        ))
    }
}
