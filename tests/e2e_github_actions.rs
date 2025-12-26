//! GitHub Actions E2E tests

mod helper;

use std::collections::HashMap;

use mockito::Server;
use tower::Service;
use tower_lsp::LspService;
use tower_lsp::lsp_types::*;

use helper::{
    MockRegistry, create_code_action_request, create_did_open_notification,
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

#[tokio::test(flavor = "multi_thread")]
async fn code_action_returns_bump_actions_for_version_tag() {
    // Pattern 3: Version tag only → Returns version bump code actions
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["v3.0.0", "v3.1.0", "v4.0.0"])],
    );

    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["v3.0.0", "v3.1.0", "v4.0.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::GitHubActions,
        create_test_resolver(RegistryType::GitHubActions, registry),
    )]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // Open document with outdated version tag
    let uri = "file:///test/.github/workflows/ci.yml";
    let workflow_content = r#"name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3.0.0
"#;

    service
        .call(create_did_open_notification(uri, workflow_content))
        .await
        .unwrap();

    // Wait for diagnostics to be published
    wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
        .await
        .expect("Expected publishDiagnostics notification");

    // Send code action request at the version position (line 6, column 31 where "v3.0.0" starts)
    let response = service
        .call(create_code_action_request(2, uri, 6, 31))
        .await
        .unwrap();

    // Parse response
    let response = response.expect("Expected code action response");
    let result: Option<Vec<CodeActionOrCommand>> =
        serde_json::from_value(response.result().unwrap().clone()).unwrap();

    let actions = result.expect("Expected code actions");
    assert!(!actions.is_empty(), "Expected at least one code action");

    // Check that we got bump actions
    let titles: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => Some(ca.title.clone()),
            _ => None,
        })
        .collect();

    // Should have minor and major bump options
    assert!(
        titles.iter().any(|t| t.contains("minor")),
        "Expected minor bump action, got: {:?}",
        titles
    );
    assert!(
        titles.iter().any(|t| t.contains("major")),
        "Expected major bump action, got: {:?}",
        titles
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn code_action_returns_bump_actions_for_hash_with_comment() {
    // Pattern 2: Hash + comment → Returns version bump code actions with SHA replacement
    let mut server = Server::new_async().await;

    // Mock GitHub Tags API (called twice: once for patch, once for minor)
    let mock = server
        .mock("GET", "/repos/actions/checkout/tags")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"[
                {"name": "v4.2.0", "commit": {"sha": "newsha4200000000000000000000000000000000"}},
                {"name": "v4.1.7", "commit": {"sha": "newsha4170000000000000000000000000000000"}},
                {"name": "v4.1.6", "commit": {"sha": "8e5e7e5ab8b370d6c329ec480221332ada57f0ab"}}
            ]"#,
        )
        .expect(2)
        .create_async()
        .await;

    // Set environment variable to use mock server
    // SAFETY: This test runs in isolation and the env var is cleaned up at the end
    unsafe { std::env::set_var("GITHUB_API_BASE_URL", server.url()) };

    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["v4.1.6", "v4.1.7", "v4.2.0"])],
    );

    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["v4.1.6", "v4.1.7", "v4.2.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::GitHubActions,
        create_test_resolver(RegistryType::GitHubActions, registry),
    )]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // Open document with hash + comment pattern
    let uri = "file:///test/.github/workflows/ci.yml";
    let workflow_content = r#"name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@8e5e7e5ab8b370d6c329ec480221332ada57f0ab # v4.1.6
"#;

    service
        .call(create_did_open_notification(uri, workflow_content))
        .await
        .unwrap();

    // Wait for diagnostics
    wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
        .await
        .expect("Expected publishDiagnostics notification");

    // Send code action request at the hash position (line 6, column 31 where hash starts)
    let response = service
        .call(create_code_action_request(2, uri, 6, 31))
        .await
        .unwrap();

    // Parse response
    let response = response.expect("Expected code action response");
    let result: Option<Vec<CodeActionOrCommand>> =
        serde_json::from_value(response.result().unwrap().clone()).unwrap();

    let actions = result.expect("Expected code actions");
    assert!(!actions.is_empty(), "Expected at least one code action");

    // Check that we got bump actions with version in title
    let code_actions: Vec<&CodeAction> = actions
        .iter()
        .filter_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => Some(ca),
            _ => None,
        })
        .collect();

    // Should have patch and minor bump options
    assert!(
        code_actions.iter().any(|ca| ca.title.contains("v4.1.7")),
        "Expected patch bump to v4.1.7, got: {:?}",
        code_actions.iter().map(|ca| &ca.title).collect::<Vec<_>>()
    );

    // Check that the edit replaces both hash and comment
    let patch_action = code_actions
        .iter()
        .find(|ca| ca.title.contains("v4.1.7"))
        .expect("Expected patch action");

    let edit = patch_action.edit.as_ref().expect("Expected edit");
    let changes = edit.changes.as_ref().expect("Expected changes");
    let text_edits = changes
        .get(&uri.parse().unwrap())
        .expect("Expected edits for URI");

    assert_eq!(text_edits.len(), 1);
    // The new text should contain the new SHA and the new version comment
    assert!(
        text_edits[0].new_text.contains("newsha417"),
        "Expected new SHA in edit, got: {}",
        text_edits[0].new_text
    );
    assert!(
        text_edits[0].new_text.contains("# v4.1.7"),
        "Expected version comment in edit, got: {}",
        text_edits[0].new_text
    );

    mock.assert_async().await;

    // Clean up environment variable
    // SAFETY: Restoring environment to original state
    unsafe { std::env::remove_var("GITHUB_API_BASE_URL") };
}

#[tokio::test(flavor = "multi_thread")]
async fn code_action_returns_bump_actions_for_hash_only() {
    // Pattern 1: Hash only → Returns version bump code action with SHA replacement
    let mut server = Server::new_async().await;

    // Mock GitHub Tags API
    let mock = server
        .mock("GET", "/repos/actions/checkout/tags")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"[
                {"name": "v4.2.0", "commit": {"sha": "newsha4200000000000000000000000000000000"}},
                {"name": "v4.1.7", "commit": {"sha": "newsha4170000000000000000000000000000000"}}
            ]"#,
        )
        .create_async()
        .await;

    // Set environment variable to use mock server
    // SAFETY: This test runs in isolation and the env var is cleaned up at the end
    unsafe { std::env::set_var("GITHUB_API_BASE_URL", server.url()) };

    let (_temp_dir, cache) = create_test_cache(
        RegistryType::GitHubActions,
        &[("actions/checkout", vec!["v4.1.7", "v4.2.0"])],
    );

    let registry = MockRegistry::new(RegistryType::GitHubActions)
        .with_versions("actions/checkout", vec!["v4.1.7", "v4.2.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::GitHubActions,
        create_test_resolver(RegistryType::GitHubActions, registry),
    )]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    // Initialize
    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // Open document with hash only pattern (no version comment)
    let uri = "file:///test/.github/workflows/ci.yml";
    let workflow_content = r#"name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@8e5e7e5ab8b370d6c329ec480221332ada57f0ab
"#;

    service
        .call(create_did_open_notification(uri, workflow_content))
        .await
        .unwrap();

    // Wait for diagnostics
    wait_for_notification(&mut notification_rx, "textDocument/publishDiagnostics")
        .await
        .expect("Expected publishDiagnostics notification");

    // Send code action request at the hash position
    let response = service
        .call(create_code_action_request(2, uri, 6, 31))
        .await
        .unwrap();

    // Parse response
    let response = response.expect("Expected code action response");
    let result: Option<Vec<CodeActionOrCommand>> =
        serde_json::from_value(response.result().unwrap().clone()).unwrap();

    let actions = result.expect("Expected code actions");
    assert!(!actions.is_empty(), "Expected at least one code action");

    // Check that we got a bump action to latest
    let code_actions: Vec<&CodeAction> = actions
        .iter()
        .filter_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => Some(ca),
            _ => None,
        })
        .collect();

    // For hash-only, should offer "Bump to latest"
    assert!(
        code_actions
            .iter()
            .any(|ca| ca.title.contains("Bump to latest")),
        "Expected 'Bump to latest' action, got: {:?}",
        code_actions.iter().map(|ca| &ca.title).collect::<Vec<_>>()
    );

    // Check that the edit contains only the new SHA (no comment)
    let latest_action = code_actions
        .iter()
        .find(|ca| ca.title.contains("Bump to latest"))
        .expect("Expected latest action");

    let edit = latest_action.edit.as_ref().expect("Expected edit");
    let changes = edit.changes.as_ref().expect("Expected changes");
    let text_edits = changes
        .get(&uri.parse().unwrap())
        .expect("Expected edits for URI");

    assert_eq!(text_edits.len(), 1);
    // The new text should be just the SHA (no comment)
    assert!(
        text_edits[0].new_text.starts_with("newsha"),
        "Expected new SHA in edit, got: {}",
        text_edits[0].new_text
    );
    // Should NOT contain a comment for hash-only pattern
    assert!(
        !text_edits[0].new_text.contains("#"),
        "Hash-only should not have comment, got: {}",
        text_edits[0].new_text
    );

    mock.assert_async().await;

    // Clean up environment variable
    // SAFETY: Restoring environment to original state
    unsafe { std::env::remove_var("GITHUB_API_BASE_URL") };
}
