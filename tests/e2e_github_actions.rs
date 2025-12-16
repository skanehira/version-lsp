//! GitHub Actions E2E tests

mod helper;

use std::collections::HashMap;

use tower::Service;
use tower_lsp::LspService;
use tower_lsp::lsp_types::*;

use helper::{
    MockRegistry, create_did_open_notification,
    create_initialize_request, create_initialized_notification, create_test_cache,
    create_test_resolver, spawn_notification_collector, wait_for_notification,
};
use version_lsp::lsp::backend::Backend;
use version_lsp::lsp::resolver::PackageResolver;
use version_lsp::parser::types::RegistryType;

use crate::helper::create_did_change_notification;

#[tokio::test(flavor = "multi_thread")]
async fn did_open_publishes_outdated_version_warning() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["2.0.0", "3.0.0", "4.0.0"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["2.0.0", "3.0.0", "4.0.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::GitHubActions,
        create_test_resolver(RegistryType::GitHubActions, registry),
    )]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

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
async fn did_open_no_diagnostics_for_latest_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["3.0.0", "4.0.0"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["3.0.0", "4.0.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::GitHubActions,
        create_test_resolver(RegistryType::GitHubActions, registry),
    )]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

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
async fn did_open_publishes_error_for_nonexistent_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["3.0.0", "4.0.0"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["3.0.0", "4.0.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::GitHubActions,
        create_test_resolver(RegistryType::GitHubActions, registry),
    )]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

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
async fn did_change_publishes_diagnostics_on_version_update() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["2.0.0", "3.0.0", "4.0.0"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["2.0.0", "3.0.0", "4.0.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::GitHubActions,
        create_test_resolver(RegistryType::GitHubActions, registry),
    )]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

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
