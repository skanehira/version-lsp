//! Docker (compose.yaml) E2E tests

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
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::Docker,
        &[(
            "library/nginx",
            vec!["1.25-alpine", "1.25", "1.27-alpine", "1.27"],
        )],
    );

    let registry = MockRegistry::new(RegistryType::Docker).with_versions(
        "library/nginx",
        vec!["1.25-alpine", "1.25", "1.27-alpine", "1.27"],
    );

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::Docker,
        create_test_resolver(RegistryType::Docker, registry),
    )]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let compose_yaml = r#"services:
  web:
    image: nginx:1.25
"#;

    service
        .call(create_did_open_notification(
            "file:///test/compose.yaml",
            compose_yaml,
        ))
        .await
        .unwrap();

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
        "Update available: 1.25 -> 1.27"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_diagnostics_for_latest_version() {
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::Docker,
        &[("library/nginx", vec!["1.25", "1.27"])],
    );

    let registry = MockRegistry::new(RegistryType::Docker)
        .with_versions("library/nginx", vec!["1.25", "1.27"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::Docker,
        create_test_resolver(RegistryType::Docker, registry),
    )]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let compose_yaml = r#"services:
  web:
    image: nginx:1.27
"#;

    service
        .call(create_did_open_notification(
            "file:///test/compose.yaml",
            compose_yaml,
        ))
        .await
        .unwrap();

    let notification =
        wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
            .await
            .expect("Expected publishDiagnostics notification");
    let params: PublishDiagnosticsParams =
        serde_json::from_value(notification.params().unwrap().clone()).unwrap();
    assert!(params.diagnostics.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn publishes_error_for_nonexistent_tag() {
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::Docker,
        &[("library/nginx", vec!["1.25", "1.27"])],
    );

    let registry = MockRegistry::new(RegistryType::Docker)
        .with_versions("library/nginx", vec!["1.25", "1.27"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::Docker,
        create_test_resolver(RegistryType::Docker, registry),
    )]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let compose_yaml = r#"services:
  web:
    image: nginx:999.0
"#;

    service
        .call(create_did_open_notification(
            "file:///test/compose.yaml",
            compose_yaml,
        ))
        .await
        .unwrap();

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
        "Version 999.0 not found in registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn handles_suffixed_tag_comparison() {
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::Docker,
        &[(
            "library/nginx",
            vec!["1.25-alpine", "1.25", "1.27-alpine", "1.27"],
        )],
    );

    let registry = MockRegistry::new(RegistryType::Docker).with_versions(
        "library/nginx",
        vec!["1.25-alpine", "1.25", "1.27-alpine", "1.27"],
    );

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::Docker,
        create_test_resolver(RegistryType::Docker, registry),
    )]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let compose_yaml = r#"services:
  web:
    image: nginx:1.25-alpine
"#;

    service
        .call(create_did_open_notification(
            "file:///test/compose.yaml",
            compose_yaml,
        ))
        .await
        .unwrap();

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
    // Should suggest 1.27-alpine (same suffix available)
    assert_eq!(
        params.diagnostics[0].message,
        "Update available: 1.25-alpine -> 1.27-alpine"
    );
}
