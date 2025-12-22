//! JSR (deno.json) E2E tests

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
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::Jsr,
        &[("@luca/flag", vec!["1.0.0", "1.0.1", "1.0.2"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::Jsr)
        .with_versions("@luca/flag", vec!["1.0.0", "1.0.1", "1.0.2"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::Jsr,
        create_test_resolver(RegistryType::Jsr, registry),
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

    // 5. didOpen with outdated version
    let deno_json = r#"{
  "imports": {
    "@luca/flag": "jsr:@luca/flag@1.0.1"
  }
}"#;

    service
        .call(create_did_open_notification(
            "file:///test/deno.json",
            deno_json,
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
        "Update available: 1.0.1 -> 1.0.2"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_diagnostics_for_latest_version() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) =
        create_test_cache(RegistryType::Jsr, &[("@std/path", vec!["1.0.0", "1.0.1"])]);

    // 2. Setup mock Registry and resolver
    let registry =
        MockRegistry::new(RegistryType::Jsr).with_versions("@std/path", vec!["1.0.0", "1.0.1"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::Jsr,
        create_test_resolver(RegistryType::Jsr, registry),
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

    // 5. didOpen with latest version
    let deno_json = r#"{
  "imports": {
    "@std/path": "jsr:@std/path@1.0.1"
  }
}"#;

    service
        .call(create_did_open_notification(
            "file:///test/deno.json",
            deno_json,
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
    // 1. Setup real Cache with test data (oldest first, newest last)
    let (_temp_dir, cache) =
        create_test_cache(RegistryType::Jsr, &[("@luca/flag", vec!["1.0.0", "1.0.1"])]);

    // 2. Setup mock Registry and resolver
    let registry =
        MockRegistry::new(RegistryType::Jsr).with_versions("@luca/flag", vec!["1.0.0", "1.0.1"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::Jsr,
        create_test_resolver(RegistryType::Jsr, registry),
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
    let deno_json = r#"{
  "imports": {
    "@luca/flag": "jsr:@luca/flag@999.0.0"
  }
}"#;

    service
        .call(create_did_open_notification(
            "file:///test/deno.json",
            deno_json,
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
async fn caret_range_is_latest_when_satisfied() {
    // 1. Setup real Cache with test data (oldest first, newest last)
    // caret range ^1.0.0 satisfies latest 1.0.2
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::Jsr,
        &[("@luca/flag", vec!["1.0.0", "1.0.1", "1.0.2"])],
    );

    // 2. Setup mock Registry and resolver
    let registry = MockRegistry::new(RegistryType::Jsr)
        .with_versions("@luca/flag", vec!["1.0.0", "1.0.1", "1.0.2"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::Jsr,
        create_test_resolver(RegistryType::Jsr, registry),
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

    // 5. didOpen with caret range that includes latest
    let deno_json = r#"{
  "imports": {
    "@luca/flag": "jsr:@luca/flag@^1.0.0"
  }
}"#;

    service
        .call(create_did_open_notification(
            "file:///test/deno.json",
            deno_json,
        ))
        .await
        .unwrap();

    // 6. Receive publishDiagnostics notification - should be empty (latest 1.0.2 satisfies ^1.0.0)
    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert!(params.diagnostics.is_empty());
}
