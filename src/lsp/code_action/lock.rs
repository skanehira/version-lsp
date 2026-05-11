//! Code action that pins a dependency to the version resolved in the lock file.

use tower_lsp::lsp_types::{CodeAction, Url};
use tracing::warn;

use crate::parser::types::{PackageInfo, RegistryType};
use crate::version::lock::LockResolver;

use super::create_bump_action;

/// Generate a single "Pin to locked version" code action when the lock file
/// pins the package and the resulting pinned spec differs from what's written.
pub fn generate_lock_pin_code_action(
    lock_resolver: &dyn LockResolver,
    package: &PackageInfo,
    uri: &Url,
) -> Option<CodeAction> {
    let locked = lock_resolver
        .resolve_locked_version(uri, &package.name)
        .inspect_err(|e| warn!("Lock resolver failed for {}: {}", package.name, e))
        .ok()??;

    let pinned_spec = format_pinned_spec(package.registry_type, &locked);
    if pinned_spec == package.version {
        return None;
    }

    Some(create_bump_action(
        &format!("Pin to locked version: {locked}"),
        &pinned_spec,
        package,
        uri,
    ))
}

/// Format the locked version into a spec that can replace the captured
/// `PackageInfo.version` text without breaking the manifest's syntax.
///
/// Each registry has its own contract:
/// - **PyPI**: PEP 508 strings embed the operator (`requests>=2.20`), so
///   pinning must emit `==<version>` — a bare version would concat with
///   the package name (`requests7.1.0`).
/// - **GoProxy**: `go.mod` requires the `v` prefix (`v1.2.3`).
/// - **Npm / PnpmCatalog / CratesIo / Jsr / GitHubActions**: the operator
///   (if any) lives outside the captured version string, so a bare semver
///   replacement is always valid.
///
/// The match is exhaustive on purpose: adding a new `RegistryType` variant
/// forces an explicit decision here.
fn format_pinned_spec(registry_type: RegistryType, locked_version: &str) -> String {
    match registry_type {
        RegistryType::PyPI => format!("=={locked_version}"),
        RegistryType::GoProxy => format!("v{}", locked_version.trim_start_matches('v')),
        RegistryType::Npm
        | RegistryType::PnpmCatalog
        | RegistryType::CratesIo
        | RegistryType::Jsr
        | RegistryType::GitHubActions => locked_version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::error::LockError;
    use crate::version::lock::MockLockResolver;
    use mockall::predicate::*;
    use rstest::rstest;

    fn make_package(name: &str, version: &str, registry_type: RegistryType) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            commit_hash: None,
            registry_type,
            start_offset: 0,
            end_offset: version.len(),
            line: 3,
            column: 15,
            extra_info: None,
        }
    }

    #[test]
    fn generates_pin_action_when_locked_version_differs() {
        let mut resolver = MockLockResolver::new();
        resolver
            .expect_resolve_locked_version()
            .returning(|_, _| Ok(Some("4.17.21".to_string())));

        let package = make_package("lodash", "^4.17.0", RegistryType::Npm);
        let uri = Url::parse("file:///proj/package.json").unwrap();

        let action = generate_lock_pin_code_action(&resolver, &package, &uri).unwrap();

        assert_eq!(action.title, "Pin to locked version: 4.17.21");
        let edit = action.edit.unwrap();
        let edits = edit.changes.unwrap().remove(&uri).unwrap();
        assert_eq!(edits[0].new_text, "4.17.21");
    }

    #[test]
    fn returns_none_when_locked_matches_current_text() {
        let mut resolver = MockLockResolver::new();
        resolver
            .expect_resolve_locked_version()
            .returning(|_, _| Ok(Some("4.17.21".to_string())));

        let package = make_package("lodash", "4.17.21", RegistryType::Npm);
        let uri = Url::parse("file:///proj/package.json").unwrap();

        assert!(generate_lock_pin_code_action(&resolver, &package, &uri).is_none());
    }

    #[test]
    fn returns_none_when_no_lock_file_found() {
        let mut resolver = MockLockResolver::new();
        resolver
            .expect_resolve_locked_version()
            .returning(|_, _| Ok(None));

        let package = make_package("lodash", "^4.17.0", RegistryType::Npm);
        let uri = Url::parse("file:///proj/package.json").unwrap();

        assert!(generate_lock_pin_code_action(&resolver, &package, &uri).is_none());
    }

    #[test]
    fn returns_none_when_resolver_errors() {
        let mut resolver = MockLockResolver::new();
        resolver
            .expect_resolve_locked_version()
            .returning(|_, _| Err(LockError::InvalidFormat("bad json".into())));

        let package = make_package("lodash", "^4.17.0", RegistryType::Npm);
        let uri = Url::parse("file:///proj/package.json").unwrap();

        assert!(generate_lock_pin_code_action(&resolver, &package, &uri).is_none());
    }

    #[test]
    fn pypi_pin_action_emits_equals_operator() {
        let mut resolver = MockLockResolver::new();
        resolver
            .expect_resolve_locked_version()
            .returning(|_, _| Ok(Some("7.1.0".to_string())));

        let package = make_package("python-gitlab", ">=4.0.0", RegistryType::PyPI);
        let uri = Url::parse("file:///proj/pyproject.toml").unwrap();

        let action = generate_lock_pin_code_action(&resolver, &package, &uri).unwrap();

        assert_eq!(action.title, "Pin to locked version: 7.1.0");
        let edit = action.edit.unwrap();
        let edits = edit.changes.unwrap().remove(&uri).unwrap();
        assert_eq!(edits[0].new_text, "==7.1.0");
    }

    /// Per-registry contract for `format_pinned_spec`: the produced text,
    /// inserted verbatim in place of `PackageInfo.version`, must yield a
    /// syntactically valid manifest entry. This case-table is the central
    /// place where each registry's pinning behavior is documented; adding
    /// a new RegistryType variant breaks `format_pinned_spec`'s exhaustive
    /// match and forces a deliberate addition here.
    #[rstest]
    #[case::npm(RegistryType::Npm, "4.17.21", "4.17.21")]
    #[case::pnpm_catalog(RegistryType::PnpmCatalog, "4.17.21", "4.17.21")]
    #[case::crates(RegistryType::CratesIo, "1.0.219", "1.0.219")]
    #[case::jsr(RegistryType::Jsr, "1.2.3", "1.2.3")]
    #[case::github_actions(RegistryType::GitHubActions, "v4.1.6", "v4.1.6")]
    #[case::pypi_emits_equals(RegistryType::PyPI, "7.1.0", "==7.1.0")]
    #[case::go_emits_v_prefix(RegistryType::GoProxy, "1.2.3", "v1.2.3")]
    #[case::go_does_not_double_v_prefix(RegistryType::GoProxy, "v1.2.3", "v1.2.3")]
    fn format_pinned_spec_per_registry(
        #[case] registry_type: RegistryType,
        #[case] locked: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(format_pinned_spec(registry_type, locked), expected);
    }

    #[test]
    fn pypi_pin_action_skipped_when_already_pinned_to_locked() {
        let mut resolver = MockLockResolver::new();
        resolver
            .expect_resolve_locked_version()
            .returning(|_, _| Ok(Some("7.1.0".to_string())));

        let package = make_package("python-gitlab", "==7.1.0", RegistryType::PyPI);
        let uri = Url::parse("file:///proj/pyproject.toml").unwrap();

        assert!(generate_lock_pin_code_action(&resolver, &package, &uri).is_none());
    }
}
