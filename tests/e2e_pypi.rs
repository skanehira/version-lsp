//! PyPI (pyproject.toml) E2E tests

mod helper;

use std::collections::HashMap;

use tower::Service;
use tower_lsp::LspService;
use tower_lsp::lsp_types::*;

use helper::{
    MockRegistry, create_did_open_notification, create_initialize_request,
    create_initialized_notification, create_test_cache, create_test_resolver,
    spawn_notification_collector, wait_for_notification,
};
use version_lsp::lsp::backend::Backend;
use version_lsp::lsp::resolver::PackageResolver;
use version_lsp::parser::types::RegistryType;

#[tokio::test(flavor = "multi_thread")]
async fn publishes_outdated_version_warning() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    // Using >=2.28.0 which requires >= 2.28.0
    // Latest is 2.32.0 which satisfies the requirement, so it's latest
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::PyPI,
        &[("requests", vec!["2.27.0", "2.28.0", "2.32.0"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::PyPI)
        .with_versions("requests", vec!["2.27.0", "2.28.0", "2.32.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::PyPI,
        create_test_resolver(RegistryType::PyPI, registry),
    )]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with requirement that doesn't satisfy latest
    // ~=2.28.0 means >=2.28.0 <2.29.0, so 2.32.0 is outside the range -> outdated
    let pyproject_toml = r#"[project]
name = "my-app"
version = "0.1.0"
dependencies = [
    "requests~=2.28.0",
]
"#;

    service
        .call(create_did_open_notification(
            "file:///test/pyproject.toml",
            pyproject_toml,
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
        "Update available: ~=2.28.0 -> 2.32.0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_diagnostics_for_latest_version() {
    // 1. Setup real Cache with test data
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::PyPI,
        &[("flask", vec!["2.0.0", "2.5.0", "3.0.0"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::PyPI)
        .with_versions("flask", vec!["2.0.0", "2.5.0", "3.0.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::PyPI,
        create_test_resolver(RegistryType::PyPI, registry),
    )]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with requirement that includes latest
    // >=2.0.0 includes 3.0.0, so it's latest
    let pyproject_toml = r#"[project]
name = "my-app"
version = "0.1.0"
dependencies = [
    "flask>=2.0.0",
]
"#;

    service
        .call(create_did_open_notification(
            "file:///test/pyproject.toml",
            pyproject_toml,
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
async fn publishes_error_for_nonexistent_version() {
    // 1. Setup real Cache with test data
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::PyPI,
        &[("django", vec!["4.0.0", "4.1.0", "4.2.0"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::PyPI)
        .with_versions("django", vec!["4.0.0", "4.1.0", "4.2.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::PyPI,
        create_test_resolver(RegistryType::PyPI, registry),
    )]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with nonexistent version
    let pyproject_toml = r#"[project]
name = "my-app"
version = "0.1.0"
dependencies = [
    "django==999.0.0",
]
"#;

    service
        .call(create_did_open_notification(
            "file:///test/pyproject.toml",
            pyproject_toml,
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
        "Version ==999.0.0 not found in registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn build_system_requires_outdated_warning() {
    // 1. Setup real Cache with test data
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::PyPI,
        &[("setuptools", vec!["60.0.0", "61.0.0", "70.0.0"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::PyPI)
        .with_versions("setuptools", vec!["60.0.0", "61.0.0", "70.0.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::PyPI,
        create_test_resolver(RegistryType::PyPI, registry),
    )]);

    // 3. Create LspService
    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // 4. Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // 5. didOpen with [build-system] requires that doesn't satisfy latest
    // >=61.0, <62.0 doesn't include 70.0.0
    let pyproject_toml = r#"[build-system]
requires = [
    "setuptools>=61.0, <62.0",
]
build-backend = "setuptools.build_meta"
"#;

    service
        .call(create_did_open_notification(
            "file:///test/pyproject.toml",
            pyproject_toml,
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
        "Update available: >=61.0, <62.0 -> 70.0.0"
    );
}
