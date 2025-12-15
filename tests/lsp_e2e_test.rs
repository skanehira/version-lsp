//! LSP E2E tests
//!
//! These tests verify the LSP protocol communication through tower-lsp's Service layer.
//! Uses real Cache (with tempfile) and mock Registry.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tempfile::TempDir;
use tokio::time::timeout;
use tower::Service;
use tower_lsp::ClientSocket;
use tower_lsp::LspService;
use tower_lsp::jsonrpc::Request;
use tower_lsp::lsp_types::*;

use version_lsp::lsp::backend::Backend;
use version_lsp::parser::types::RegistryType;
use version_lsp::version::cache::Cache;
use version_lsp::version::checker::VersionStorer;
use version_lsp::version::error::RegistryError;
use version_lsp::version::registry::Registry;
use version_lsp::version::types::PackageVersions;

/// Mock registry for testing
struct MockRegistry {
    registry_type: RegistryType,
    versions: HashMap<String, Vec<String>>,
}

impl MockRegistry {
    fn new(registry_type: RegistryType) -> Self {
        Self {
            registry_type,
            versions: HashMap::new(),
        }
    }

    fn with_versions(mut self, package: &str, versions: Vec<&str>) -> Self {
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

fn create_test_cache(
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

fn create_initialize_request(id: i64) -> Request {
    Request::build("initialize")
        .id(id)
        .params(serde_json::to_value(InitializeParams::default()).unwrap())
        .finish()
}

fn create_initialized_notification() -> Request {
    Request::build("initialized")
        .params(serde_json::to_value(InitializedParams {}).unwrap())
        .finish()
}

fn create_did_open_notification(uri: &str, content: &str) -> Request {
    Request::build("textDocument/didOpen")
        .params(
            serde_json::to_value(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.parse().unwrap(),
                    language_id: "yaml".to_string(),
                    version: 1,
                    text: content.to_string(),
                },
            })
            .unwrap(),
        )
        .finish()
}

fn create_did_change_notification(uri: &str, content: &str, version: i32) -> Request {
    Request::build("textDocument/didChange")
        .params(
            serde_json::to_value(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.parse().unwrap(),
                    version,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: content.to_string(),
                }],
            })
            .unwrap(),
        )
        .finish()
}

use tokio::sync::mpsc;

/// Collect notifications in background and return a receiver
fn spawn_notification_collector(mut socket: ClientSocket) -> mpsc::Receiver<Request> {
    let (tx, rx) = mpsc::channel(100);

    tokio::spawn(async move {
        while let Some(notification) = socket.next().await {
            if tx.send(notification).await.is_err() {
                break;
            }
        }
    });

    rx
}

/// Wait for a notification with the specified method name from the receiver
async fn wait_for_notification(rx: &mut mpsc::Receiver<Request>, method: &str) -> Option<Request> {
    let timeout_duration = Duration::from_secs(5);

    loop {
        match timeout(timeout_duration, rx.recv()).await {
            Ok(Some(notification)) => {
                if notification.method() == method {
                    return Some(notification);
                }
                // Skip other notifications (like log_message)
            }
            _ => return None,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_did_open_publishes_outdated_version_warning() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["2.0.0", "3.0.0", "4.0.0"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["2.0.0", "3.0.0", "4.0.0"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::GitHubActions, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    // Start notification collector immediately
    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    let init_response = service.call(create_initialize_request(1)).await.unwrap();
    assert!(init_response.is_some());

    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with outdated version
    let workflow_content = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3.0.0
"#;

    service
        .call(create_did_open_notification(
            "file:///test/.github/workflows/ci.yml",
            workflow_content,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");

    // Verify diagnostic content
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(
        params.diagnostics[0].severity,
        Some(DiagnosticSeverity::WARNING)
    );
    assert_eq!(
        params.diagnostics[0].message,
        "Update available: 3.0.0 -> 4.0.0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_did_open_no_diagnostics_for_latest_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["3.0.0", "4.0.0"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["3.0.0", "4.0.0"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::GitHubActions, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    // Start notification collector immediately
    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with latest version
    let workflow_content = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@4.0.0
"#;

    service
        .call(create_did_open_notification(
            "file:///test/.github/workflows/ci.yml",
            workflow_content,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should have empty diagnostics
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert!(params.diagnostics.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_did_open_publishes_error_for_nonexistent_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["3.0.0", "4.0.0"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["3.0.0", "4.0.0"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::GitHubActions, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    // Start notification collector immediately
    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with nonexistent version (version not in cache)
    let workflow_content = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@999.0.0
"#;

    service
        .call(create_did_open_notification(
            "file:///test/.github/workflows/ci.yml",
            workflow_content,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should have ERROR diagnostic
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(
        params.diagnostics[0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
    assert_eq!(
        params.diagnostics[0].message,
        "Version 999.0.0 not found in registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_did_change_publishes_diagnostics_on_version_update() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["2.0.0", "3.0.0", "4.0.0"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["2.0.0", "3.0.0", "4.0.0"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::GitHubActions, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    // Start notification collector immediately
    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let uri = "file:///test/.github/workflows/ci.yml";

    // 5. didOpen with latest version (no warning)
    let initial_content = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@4.0.0
"#;

    service
        .call(create_did_open_notification(uri, initial_content))
        .await
        .unwrap();

    // Wait for initial publishDiagnostics (should be empty)
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert!(params.diagnostics.is_empty());

    // 6. didChange to outdated version
    let updated_content = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3.0.0
"#;

    service
        .call(create_did_change_notification(uri, updated_content, 2))
        .await
        .unwrap();

    // 7. Receive publishDiagnostics notification with warning
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification after didChange");

    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(
        params.diagnostics[0].severity,
        Some(DiagnosticSeverity::WARNING)
    );
    assert_eq!(
        params.diagnostics[0].message,
        "Update available: 3.0.0 -> 4.0.0"
    );
}

// ============================================================================
// package.json E2E tests
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn e2e_package_json_publishes_outdated_version_warning() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::Npm,
        &[("lodash", vec!["4.17.19", "4.17.20", "4.17.21"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::Npm)
        .with_versions("lodash", vec!["4.17.19", "4.17.20", "4.17.21"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::Npm, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with outdated version
    let package_json = r#"{
  "name": "test-project",
  "dependencies": {
    "lodash": "4.17.20"
  }
}"#;

    service
        .call(create_did_open_notification(
            "file:///test/package.json",
            package_json,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");

    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(
        params.diagnostics[0].severity,
        Some(DiagnosticSeverity::WARNING)
    );
    assert_eq!(
        params.diagnostics[0].message,
        "Update available: 4.17.20 -> 4.17.21"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_package_json_no_diagnostics_for_latest_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) =
        create_test_cache(RegistryType::Npm, &[("lodash", vec!["4.17.20", "4.17.21"])]);

    // 2. Setup mock Registry
    let registry =
        MockRegistry::new(RegistryType::Npm).with_versions("lodash", vec!["4.17.20", "4.17.21"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::Npm, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with latest version
    let package_json = r#"{
  "name": "test-project",
  "dependencies": {
    "lodash": "4.17.21"
  }
}"#;

    service
        .call(create_did_open_notification(
            "file:///test/package.json",
            package_json,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should be empty
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert!(params.diagnostics.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_package_json_publishes_error_for_nonexistent_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) =
        create_test_cache(RegistryType::Npm, &[("lodash", vec!["4.17.20", "4.17.21"])]);

    // 2. Setup mock Registry
    let registry =
        MockRegistry::new(RegistryType::Npm).with_versions("lodash", vec!["4.17.20", "4.17.21"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::Npm, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with nonexistent version
    let package_json = r#"{
  "name": "test-project",
  "dependencies": {
    "lodash": "999.0.0"
  }
}"#;

    service
        .call(create_did_open_notification(
            "file:///test/package.json",
            package_json,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should have ERROR diagnostic
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(
        params.diagnostics[0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
    assert_eq!(
        params.diagnostics[0].message,
        "Version 999.0.0 not found in registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_package_json_caret_range_is_latest_when_satisfied() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    // caret range ^4.17.0 satisfies latest 4.17.21
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::Npm,
        &[("lodash", vec!["4.17.0", "4.17.20", "4.17.21"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::Npm)
        .with_versions("lodash", vec!["4.17.0", "4.17.20", "4.17.21"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::Npm, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with caret range that includes latest
    let package_json = r#"{
  "name": "test-project",
  "dependencies": {
    "lodash": "^4.17.0"
  }
}"#;

    service
        .call(create_did_open_notification(
            "file:///test/package.json",
            package_json,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should be empty (latest 4.17.21 satisfies ^4.17.0)
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert!(params.diagnostics.is_empty());
}

// ============================================================================
// Cargo.toml E2E tests
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn e2e_cargo_toml_publishes_outdated_version_warning() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    // Using tilde requirement ~1.0.100 which means >=1.0.100 <1.1.0
    // Latest is 1.1.0 which is outside the range, so it's outdated
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::CratesIo,
        &[("serde", vec!["1.0.0", "1.0.100", "1.1.0"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::CratesIo)
        .with_versions("serde", vec!["1.0.0", "1.0.100", "1.1.0"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::CratesIo, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with tilde requirement (outdated because latest 1.1.0 is outside ~1.0.100)
    let cargo_toml = r#"[package]
name = "test-project"
version = "0.1.0"

[dependencies]
serde = "~1.0.100"
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Cargo.toml",
            cargo_toml,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");

    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(
        params.diagnostics[0].severity,
        Some(DiagnosticSeverity::WARNING)
    );
    assert_eq!(
        params.diagnostics[0].message,
        "Update available: ~1.0.100 -> 1.1.0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_cargo_toml_no_diagnostics_for_latest_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::CratesIo,
        &[("serde", vec!["1.0.100", "1.0.200"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::CratesIo)
        .with_versions("serde", vec!["1.0.100", "1.0.200"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::CratesIo, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with latest version (caret requirement that includes latest)
    let cargo_toml = r#"[package]
name = "test-project"
version = "0.1.0"

[dependencies]
serde = "1.0.200"
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Cargo.toml",
            cargo_toml,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should be empty
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert!(params.diagnostics.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_cargo_toml_publishes_error_for_nonexistent_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::CratesIo,
        &[("serde", vec!["1.0.100", "1.0.200"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::CratesIo)
        .with_versions("serde", vec!["1.0.100", "1.0.200"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::CratesIo, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with nonexistent version
    let cargo_toml = r#"[package]
name = "test-project"
version = "0.1.0"

[dependencies]
serde = "=999.0.0"
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Cargo.toml",
            cargo_toml,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should have ERROR diagnostic
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(
        params.diagnostics[0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
    assert_eq!(
        params.diagnostics[0].message,
        "Version =999.0.0 not found in registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_cargo_toml_caret_range_is_latest_when_satisfied() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    // Cargo's default requirement (no prefix) is caret-like: 1.0.0 means >=1.0.0 <2.0.0
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::CratesIo,
        &[("serde", vec!["1.0.0", "1.0.100", "1.0.200"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::CratesIo)
        .with_versions("serde", vec!["1.0.0", "1.0.100", "1.0.200"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::CratesIo, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with caret range that includes latest
    // "1.0.0" in Cargo means ^1.0.0, which satisfies 1.0.200
    let cargo_toml = r#"[package]
name = "test-project"
version = "0.1.0"

[dependencies]
serde = "1.0.0"
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Cargo.toml",
            cargo_toml,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should be empty (latest 1.0.200 satisfies 1.0.0)
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert!(params.diagnostics.is_empty());
}

/// Test [workspace.dependencies] format
#[tokio::test(flavor = "multi_thread")]
async fn e2e_cargo_toml_workspace_dependencies_outdated_warning() {
    // 1. Setup real Cache with test data
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::CratesIo,
        &[("prost", vec!["0.12.0", "0.13.0", "0.14.0", "0.14.1"])],
    );

    // 2. Setup mock Registry
    let registry = MockRegistry::new(RegistryType::CratesIo)
        .with_versions("prost", vec!["0.12.0", "0.13.0", "0.14.0", "0.14.1"]);

    let registries: HashMap<RegistryType, Arc<dyn Registry>> =
        HashMap::from([(RegistryType::CratesIo, Arc::new(registry) as _)]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), registries)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with workspace.dependencies format
    // "0.13" means ^0.13.0, which does NOT satisfy 0.14.1 (0.x caret semantics)
    let cargo_toml = r#"[workspace]
members = ["crates/*"]

[workspace.dependencies]
prost = "0.13"
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Cargo.toml",
            cargo_toml,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should have WARNING
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(
        params.diagnostics[0].severity,
        Some(DiagnosticSeverity::WARNING)
    );
    assert_eq!(
        params.diagnostics[0].message,
        "Update available: 0.13 -> 0.14.1"
    );
}
