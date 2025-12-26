//! Diagnostics generation for version checking results

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use tracing::warn;

use crate::parser::traits::Parser;
use crate::parser::types::PackageInfo;
use crate::version::checker::{
    VersionCompareResult, VersionStatus, VersionStorer, compare_version,
};
use crate::version::matcher::VersionMatcher;

const PACKAGE_NAME: &str = env!("CARGO_PKG_NAME");

/// Generate diagnostics for a document by parsing and checking versions
pub fn generate_diagnostics<S: VersionStorer>(
    parser: &dyn Parser,
    matcher: &dyn VersionMatcher,
    storer: &S,
    content: &str,
) -> Vec<Diagnostic> {
    let packages = parser
        .parse(content)
        .inspect_err(|e| warn!("Failed to parse document: {}", e))
        .unwrap_or_default();

    packages
        .iter()
        .filter_map(|package| {
            let result = compare_version(storer, matcher, &package.name, &package.version).ok()?;
            create_diagnostic(package, &result)
        })
        .collect()
}

/// Create a diagnostic from package info and version check result
/// Returns None if no diagnostic should be shown (e.g., NotInCache)
fn create_diagnostic(package: &PackageInfo, result: &VersionCompareResult) -> Option<Diagnostic> {
    let (severity, message) = match result.status {
        // No diagnostic for: not cached, latest version, or newer than latest
        // Newer: version exists but is newer than dist-tags.latest (valid scenario)
        VersionStatus::NotInCache | VersionStatus::Latest | VersionStatus::Newer => return None,
        VersionStatus::Outdated => (
            DiagnosticSeverity::WARNING,
            format!(
                "Update available: {} -> {}",
                result.current_version,
                result.latest_version.as_deref().unwrap_or("unknown")
            ),
        ),
        VersionStatus::NotFound => (
            DiagnosticSeverity::ERROR,
            format!("Version {} not found in registry", result.current_version),
        ),
        VersionStatus::Invalid => (
            DiagnosticSeverity::ERROR,
            format!("Invalid version format: {}", result.current_version),
        ),
    };

    let range = Range {
        start: Position {
            line: package.line as u32,
            character: package.column as u32,
        },
        end: Position {
            line: package.line as u32,
            character: (package.column + package.end_offset - package.start_offset) as u32,
        },
    };

    Some(Diagnostic {
        range,
        severity: Some(severity),
        message,
        source: Some(PACKAGE_NAME.to_string()),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::traits::MockParser;
    use crate::parser::types::RegistryType;
    use crate::version::checker::MockVersionStorer;
    use crate::version::matchers::GitHubActionsMatcher;
    use rstest::rstest;

    fn make_package_info(name: &str, version: &str, line: usize, column: usize) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            commit_hash: None,
            registry_type: RegistryType::GitHubActions,
            start_offset: column,
            end_offset: column + version.len(),
            line,
            column,
            extra_info: None,
        }
    }

    #[rstest]
    #[case(
        "3.0.0",
        true,
        DiagnosticSeverity::WARNING,
        "Update available: 3.0.0 -> 4.0.0"
    )]
    #[case(
        "9.9.9",
        false,
        DiagnosticSeverity::ERROR,
        "Version 9.9.9 not found in registry"
    )]
    #[case(
        "invalid",
        true,
        DiagnosticSeverity::ERROR,
        "Invalid version format: invalid"
    )]
    fn generate_diagnostics_returns_expected_diagnostic(
        #[case] current_version: &str,
        #[case] version_exists: bool,
        #[case] expected_severity: DiagnosticSeverity,
        #[case] expected_message: &str,
    ) {
        let version = current_version.to_string();
        let mut parser = MockParser::new();
        parser.expect_parse().returning(move |_| {
            Ok(vec![make_package_info(
                "actions/checkout",
                &version.clone(),
                5,
                14,
            )])
        });

        let exists = version_exists;
        let version_for_closure = current_version.to_string();
        let mut storer = MockVersionStorer::new();
        storer
            .expect_get_latest_version()
            .returning(|_, _| Ok(Some("4.0.0".to_string())));
        storer.expect_get_dist_tag().returning(|_, _, _| Ok(None)); // GitHub Actions don't have dist-tags
        storer.expect_get_versions().returning(move |_, _| {
            if exists {
                // Return versions that include the current version for existence check
                Ok(vec![version_for_closure.clone(), "4.0.0".to_string()])
            } else {
                // Return versions without the current version
                Ok(vec!["4.0.0".to_string()])
            }
        });
        let matcher = GitHubActionsMatcher;

        let diagnostics = generate_diagnostics(&parser, &matcher, &storer, "content");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Some(expected_severity));
        assert_eq!(diagnostics[0].message, expected_message);
    }

    #[test]
    fn generate_diagnostics_returns_empty_for_latest_package() {
        let mut parser = MockParser::new();
        parser
            .expect_parse()
            .returning(|_| Ok(vec![make_package_info("actions/checkout", "4.0.0", 5, 14)]));

        let mut storer = MockVersionStorer::new();
        storer
            .expect_get_latest_version()
            .returning(|_, _| Ok(Some("4.0.0".to_string())));
        storer.expect_get_dist_tag().returning(|_, _, _| Ok(None));
        storer
            .expect_get_versions()
            .returning(|_, _| Ok(vec!["4.0.0".to_string()]));
        let matcher = GitHubActionsMatcher;

        let diagnostics = generate_diagnostics(&parser, &matcher, &storer, "content");

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn generate_diagnostics_skips_packages_not_in_cache() {
        let mut parser = MockParser::new();
        parser
            .expect_parse()
            .returning(|_| Ok(vec![make_package_info("actions/checkout", "4.0.0", 5, 14)]));

        let mut storer = MockVersionStorer::new();
        storer
            .expect_get_latest_version()
            .returning(|_, _| Ok(None));
        let matcher = GitHubActionsMatcher;

        let diagnostics = generate_diagnostics(&parser, &matcher, &storer, "content");

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn generate_diagnostics_skips_version_newer_than_latest() {
        // When a version exists but is newer than the "latest" dist-tag
        // (e.g., ag-grid 33.0.3 exists but dist-tags.latest is 32.3.9)
        // we should NOT show any diagnostic
        let mut parser = MockParser::new();
        parser
            .expect_parse()
            .returning(|_| Ok(vec![make_package_info("actions/checkout", "5.0.0", 5, 14)]));

        let mut storer = MockVersionStorer::new();
        storer
            .expect_get_latest_version()
            .returning(|_, _| Ok(Some("4.0.0".to_string())));
        storer.expect_get_dist_tag().returning(|_, _, _| Ok(None));
        storer
            .expect_get_versions()
            .returning(|_, _| Ok(vec!["5.0.0".to_string(), "4.0.0".to_string()]));
        let matcher = GitHubActionsMatcher;

        let diagnostics = generate_diagnostics(&parser, &matcher, &storer, "content");

        // Version 5.0.0 exists and is newer than latest (4.0.0) - no diagnostic
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn generate_diagnostics_calculates_correct_range() {
        let mut parser = MockParser::new();
        parser.expect_parse().returning(|_| {
            Ok(vec![PackageInfo {
                name: "actions/checkout".to_string(),
                version: "3.0.0".to_string(),
                commit_hash: None,
                registry_type: RegistryType::GitHubActions,
                start_offset: 10,
                end_offset: 20,
                line: 5,
                column: 10,
                extra_info: None,
            }])
        });

        let mut storer = MockVersionStorer::new();
        storer
            .expect_get_latest_version()
            .returning(|_, _| Ok(Some("4.0.0".to_string())));
        storer.expect_get_dist_tag().returning(|_, _, _| Ok(None));
        storer
            .expect_get_versions()
            .returning(|_, _| Ok(vec!["3.0.0".to_string(), "4.0.0".to_string()]));
        let matcher = GitHubActionsMatcher;

        let diagnostics = generate_diagnostics(&parser, &matcher, &storer, "content");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].range,
            Range {
                start: Position {
                    line: 5,
                    character: 10
                },
                end: Position {
                    line: 5,
                    character: 20
                },
            }
        );
    }
}
