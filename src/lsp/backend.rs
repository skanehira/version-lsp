use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tracing::{error, info, warn};

use crate::config::{DEFAULT_REFRESH_INTERVAL_MS, data_dir, db_path};
use crate::lsp::diagnostics::generate_diagnostics;
use crate::lsp::refresh::{fetch_missing_packages, refresh_packages};
use crate::parser::github_actions::GitHubActionsParser;
use crate::parser::traits::Parser;
use crate::parser::types::{RegistryType, detect_parser_type};
use crate::version::cache::Cache;
use crate::version::checker::VersionStorer;
use crate::version::registries::github::GitHubRegistry;
use crate::version::registry::Registry;

pub struct Backend<S: VersionStorer> {
    client: Client,
    storer: Option<Arc<S>>,
    parsers: HashMap<RegistryType, Arc<dyn Parser>>,
    registries: HashMap<RegistryType, Arc<dyn Registry>>,
}

impl Backend<Cache> {
    pub fn new(client: Client) -> Self {
        let storer = Self::initialize_storer();
        let parsers = Self::initialize_parsers();
        let registries = Self::initialize_registries();
        Self {
            client,
            storer,
            parsers,
            registries,
        }
    }

    fn initialize_storer() -> Option<Arc<Cache>> {
        let data_dir = data_dir();
        let db_path = db_path();

        // Create data directory if it doesn't exist
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            error!("Failed to create data directory {:?}: {}", data_dir, e);
            return None;
        }

        match Cache::new(&db_path, DEFAULT_REFRESH_INTERVAL_MS) {
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
    /// Build a Backend with custom storer and registries
    pub fn build(
        client: Client,
        storer: Arc<S>,
        registries: HashMap<RegistryType, Arc<dyn Registry>>,
    ) -> Self {
        Self {
            client,
            storer: Some(storer),
            parsers: Self::initialize_parsers(),
            registries,
        }
    }

    fn initialize_parsers() -> HashMap<RegistryType, Arc<dyn Parser>> {
        let mut parsers: HashMap<RegistryType, Arc<dyn Parser>> = HashMap::new();
        parsers.insert(
            RegistryType::GitHubActions,
            Arc::new(GitHubActionsParser::new()),
        );
        parsers
    }

    fn initialize_registries() -> HashMap<RegistryType, Arc<dyn Registry>> {
        let mut registries: HashMap<RegistryType, Arc<dyn Registry>> = HashMap::new();
        registries.insert(
            RegistryType::GitHubActions,
            Arc::new(GitHubRegistry::default()),
        );
        registries
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
            ..Default::default()
        }
    }

    fn spawn_background_refresh(&self) {
        let Some(storer) = self.storer.clone() else {
            warn!("Storer not available, skipping background refresh");
            return;
        };

        let Some(registry) = self.registries.get(&RegistryType::GitHubActions).cloned() else {
            warn!("GitHub Actions registry not available");
            return;
        };

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

            // Filter to only GitHub Actions packages for now
            let github_packages: Vec<_> = packages
                .into_iter()
                .filter(|p| p.registry_type == RegistryType::GitHubActions.as_str())
                .collect();

            if !github_packages.is_empty() {
                refresh_packages(&*storer, &*registry, github_packages).await;
            }
        });
    }

    async fn check_and_publish_diagnostics(&self, uri: Url, content: String) {
        let uri_str = uri.as_str();

        let Some(parser_type) = detect_parser_type(uri_str) else {
            return;
        };

        let Some(parser) = self.parsers.get(&parser_type) else {
            return;
        };

        let Some(storer) = &self.storer else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    "Storer not available, skipping diagnostics",
                )
                .await;
            return;
        };

        // Parse document to get packages (needed for on-demand fetch)
        let packages = parser.parse(&content).unwrap_or_default();

        let diagnostics = generate_diagnostics(&**parser, &**storer, &content);

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
            let Some(registry) = self.registries.get(&parser_type).cloned() else {
                return;
            };

            let storer = storer.clone();
            let client = self.client.clone();
            let parser = parser.clone();

            tokio::spawn(async move {
                let fetched = fetch_missing_packages(&*storer, &*registry, &packages).await;

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

                    let diagnostics = generate_diagnostics(&*parser, &*storer, &content);

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

        self.check_and_publish_diagnostics(params.text_document.uri, content)
            .await;
    }
}
