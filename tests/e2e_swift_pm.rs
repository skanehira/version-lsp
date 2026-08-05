//! Swift Package Manager (Package.swift) E2E tests

mod helper;

use std::collections::HashMap;

use tower::Service;
use tower_lsp::LspService;
use tower_lsp::lsp_types::*;

use helper::{
    MockRegistry, create_did_open_notification, create_initialize_request,
    create_initialized_notification, create_swift_pm_resolver_with_hosts, create_test_cache,
    create_test_resolver, spawn_notification_collector, wait_for_notification,
};
use version_lsp::lsp::backend::Backend;
use version_lsp::lsp::resolver::PackageResolver;
use version_lsp::parser::types::RegistryType;

/// GitHub release tags conventionally have a `v` prefix; SwiftPmVersionMatcher
/// strips it before comparing against the bare versions in Package.swift.
fn make_resolvers(
    package_name: &str,
    versions: Vec<&str>,
) -> HashMap<RegistryType, PackageResolver> {
    let registry = MockRegistry::new(RegistryType::SwiftPm).with_versions(package_name, versions);
    HashMap::from([(
        RegistryType::SwiftPm,
        create_test_resolver(RegistryType::SwiftPm, registry),
    )])
}

#[tokio::test(flavor = "multi_thread")]
async fn publishes_outdated_version_warning() {
    // `from: "4.90.0"` is caret-like (>=4.90.0, <5.0.0). When the latest
    // available version is in a new major (5.0.0), the dependency is outdated.
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::SwiftPm,
        &[("vapor/vapor", vec!["v4.90.0", "v4.95.0", "v5.0.0"])],
    );

    let resolvers = make_resolvers("vapor/vapor", vec!["v4.90.0", "v4.95.0", "v5.0.0"]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let package_swift = r#"// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "MyApp",
    dependencies: [
        .package(url: "https://github.com/vapor/vapor.git", from: "4.90.0"),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
        "Update available: 4.90.0 -> 5.0.0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_diagnostics_for_latest_version() {
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::SwiftPm,
        &[("apple/swift-nio", vec!["v2.50.0", "v2.51.0"])],
    );

    let resolvers = make_resolvers("apple/swift-nio", vec!["v2.50.0", "v2.51.0"]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    // `from: "2.50.0"` is caret-like, so latest 2.51.0 satisfies it.
    let package_swift = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/apple/swift-nio.git", from: "2.50.0"),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
async fn publishes_error_for_nonexistent_version() {
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::SwiftPm,
        &[("vapor/vapor", vec!["v4.90.0", "v4.92.0"])],
    );

    let resolvers = make_resolvers("vapor/vapor", vec!["v4.90.0", "v4.92.0"]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let package_swift = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/vapor/vapor.git", exact: "9.9.9"),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
        "Version 9.9.9 not found in registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn up_to_next_major_is_latest_when_satisfied() {
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::SwiftPm,
        &[("apple/swift-log", vec!["v1.5.0", "v1.6.0", "v1.7.0"])],
    );

    let resolvers = make_resolvers("apple/swift-log", vec!["v1.5.0", "v1.6.0", "v1.7.0"]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let package_swift = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/apple/swift-log.git", .upToNextMajor(from: "1.5.0")),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
async fn skips_branch_pinned_dependency() {
    let (_temp_dir, cache) = create_test_cache(RegistryType::SwiftPm, &[]);

    let resolvers = make_resolvers("some/repo", vec!["v1.0.0"]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let package_swift = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/some/repo.git", branch: "main"),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
async fn half_open_range_is_latest_when_satisfied() {
    // `"1.0.0" ..< "5.0.0"` becomes `>=1.0.0, <5.0.0`. When the registry's
    // latest is `4.9.0`, the range satisfies it — no diagnostic.
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::SwiftPm,
        &[("apple/swift-crypto", vec!["v1.0.0", "v3.0.0", "v4.9.0"])],
    );

    let resolvers = make_resolvers("apple/swift-crypto", vec!["v1.0.0", "v3.0.0", "v4.9.0"]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let package_swift = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", "1.0.0" ..< "5.0.0"),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
async fn half_open_range_warns_when_latest_exceeds_upper_bound() {
    // `"1.0.0" ..< "5.0.0"` becomes `>=1.0.0, <5.0.0`. Latest is `5.1.0`,
    // outside the range — warning expected.
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::SwiftPm,
        &[("apple/swift-crypto", vec!["v1.0.0", "v4.9.0", "v5.1.0"])],
    );

    let resolvers = make_resolvers("apple/swift-crypto", vec!["v1.0.0", "v4.9.0", "v5.1.0"]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let package_swift = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", "1.0.0" ..< "5.0.0"),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
        "Update available: >=1.0.0, <5.0.0 -> 5.1.0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn closed_range_includes_upper_bound() {
    // `"1.0.0" ... "5.0.0"` becomes `>=1.0.0, <=5.0.0`. Latest `5.0.0` is
    // within the closed range — no diagnostic.
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::SwiftPm,
        &[("apple/swift-crypto", vec!["v1.0.0", "v4.0.0", "v5.0.0"])],
    );

    let resolvers = make_resolvers("apple/swift-crypto", vec!["v1.0.0", "v4.0.0", "v5.0.0"]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let package_swift = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", "1.0.0" ... "5.0.0"),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
async fn private_github_enterprise_host_is_checked_when_configured() {
    // Simulates `registries.swiftPm.url = "https://github.example.com/api/v3"`.
    // The parser is built with `github.example.com` in its allow-list, so the
    // dependency on that host is extracted and version-checked.
    let (_temp_dir, cache) = create_test_cache(
        RegistryType::SwiftPm,
        &[("team/internal-lib", vec!["v1.0.0", "v1.5.0", "v2.0.0"])],
    );

    let registry = MockRegistry::new(RegistryType::SwiftPm)
        .with_versions("team/internal-lib", vec!["v1.0.0", "v1.5.0", "v2.0.0"]);

    let resolvers: HashMap<RegistryType, PackageResolver> = HashMap::from([(
        RegistryType::SwiftPm,
        create_swift_pm_resolver_with_hosts(registry, ["github.example.com"]),
    )]);

    let (mut service, socket) =
        LspService::build(|client| Backend::build(client, cache.clone(), resolvers)).finish();

    let mut notification_rx = spawn_notification_collector(socket);

    service.call(create_initialize_request(1)).await.unwrap();
    service
        .call(create_initialized_notification())
        .await
        .unwrap();

    let package_swift = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.example.com/team/internal-lib.git", from: "1.0.0"),
    ]
)
"#;

    service
        .call(create_did_open_notification(
            "file:///test/Package.swift",
            package_swift,
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
        "Update available: 1.0.0 -> 2.0.0"
    );
}
