//! Constraint code actions — switching version constraint operators

use crate::parser::types::PackageInfo;
use tower_lsp::lsp_types::{CodeAction, Url};

use super::{create_bump_action, extract_version_prefix, strip_version_prefix};

/// Check if a PyPI version spec is simple (no compound range with comma)
fn is_simple_pypi_version(version: &str) -> bool {
    !version.contains(',')
}

/// Extract PyPI operator and bare version from a version spec
fn parse_pypi_version(version: &str) -> Option<(&str, &str)> {
    let prefix = extract_version_prefix(version);
    if matches!(prefix, "==" | ">=" | "~=") {
        Some((prefix, &version[prefix.len()..]))
    } else {
        None
    }
}

/// Generate constraint code actions for semver registries (npm, crates, jsr, pnpm catalogs)
///
/// Changes only the version prefix, not the version itself.
/// - `^X.Y.Z` → offers exact pin and `~`
/// - `~X.Y.Z` → offers exact pin and `^`
/// - `X.Y.Z` (bare) → offers `^` and `~`
pub fn generate_constraint_code_actions(package: &PackageInfo, uri: &Url) -> Vec<CodeAction> {
    let current = &package.version;
    let prefix = extract_version_prefix(current);
    let bare = strip_version_prefix(current);

    // Constraint switching only makes sense for full semver (X.Y.Z).
    // Partial versions like "22" or "22.0" have range semantics themselves,
    // so switching prefixes on them would be misleading.
    if bare.matches('.').count() < 2 {
        return vec![];
    }

    match prefix {
        "^" => vec![
            create_bump_action(&format!("Pin to exact version: {bare}"), bare, package, uri),
            create_bump_action(
                &format!("Patch updates only (~): ~{bare}"),
                &format!("~{bare}"),
                package,
                uri,
            ),
        ],
        "~" => vec![
            create_bump_action(&format!("Pin to exact version: {bare}"), bare, package, uri),
            create_bump_action(
                &format!("Compatible updates (^): ^{bare}"),
                &format!("^{bare}"),
                package,
                uri,
            ),
        ],
        "" => vec![
            create_bump_action(
                &format!("Compatible updates (^): ^{bare}"),
                &format!("^{bare}"),
                package,
                uri,
            ),
            create_bump_action(
                &format!("Patch updates only (~): ~{bare}"),
                &format!("~{bare}"),
                package,
                uri,
            ),
        ],
        _ => vec![],
    }
}

/// Generate PyPI constraint code actions
///
/// Changes only the operator, not the version. For simple PyPI version specs
/// (no compound ranges), offers to switch between == (pin), >= (minimum),
/// and ~= (compatible release) operators.
pub fn generate_pypi_constraint_code_actions(package: &PackageInfo, uri: &Url) -> Vec<CodeAction> {
    let current = &package.version;

    if !is_simple_pypi_version(current) {
        return vec![];
    }

    let Some((op, bare_version)) = parse_pypi_version(current) else {
        return vec![];
    };

    let alternatives: &[(&str, &str)] = match op {
        ">=" => &[("==", "Pin to exact version"), ("~=", "Compatible release")],
        "==" => &[(">=", "Minimum version"), ("~=", "Compatible release")],
        "~=" => &[("==", "Pin to exact version"), (">=", "Minimum version")],
        _ => return vec![],
    };

    alternatives
        .iter()
        .map(|&(alt_op, label)| {
            let new_version = format!("{alt_op}{bare_version}");
            create_bump_action(
                &format!("{label} ({alt_op}): {new_version}"),
                &new_version,
                package,
                uri,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::types::RegistryType;
    use rstest::rstest;

    fn make_package(name: &str, version: &str, line: u32, column: u32, len: usize) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            commit_hash: None,
            registry_type: RegistryType::Npm,
            start_offset: 0,
            end_offset: len,
            line: line as usize,
            column: column as usize,
            extra_info: None,
        }
    }

    fn make_pypi_package(name: &str, version: &str, line: u32, column: u32) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            commit_hash: None,
            registry_type: RegistryType::PyPI,
            start_offset: 0,
            end_offset: version.len(),
            line: line as usize,
            column: column as usize,
            extra_info: None,
        }
    }

    // ── Semver constraint action tests ──

    #[test]
    fn constraint_from_caret_offers_pin_and_tilde() {
        let package = make_package("lodash", "^4.17.19", 3, 15, 8);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_constraint_code_actions(&package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Pin to exact version: 4.17.19");
        assert_eq!(actions[1].title, "Patch updates only (~): ~4.17.19");

        // Verify text edit removes prefix for pin
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits[0].new_text, "4.17.19");
    }

    #[test]
    fn constraint_from_tilde_offers_pin_and_caret() {
        let package = make_package("lodash", "~4.17.19", 3, 15, 8);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_constraint_code_actions(&package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Pin to exact version: 4.17.19");
        assert_eq!(actions[1].title, "Compatible updates (^): ^4.17.19");
    }

    #[test]
    fn constraint_from_bare_offers_caret_and_tilde() {
        let package = make_package("lodash", "4.17.19", 3, 15, 7);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_constraint_code_actions(&package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Compatible updates (^): ^4.17.19");
        assert_eq!(actions[1].title, "Patch updates only (~): ~4.17.19");

        // Verify text edit adds prefix
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits[0].new_text, "^4.17.19");
    }

    #[test]
    fn constraint_returns_empty_for_non_semver_prefix() {
        let package = make_package("golang.org/x/text", "v0.14.0", 3, 15, 7);
        let uri = Url::parse("file:///test/go.mod").unwrap();

        let actions = generate_constraint_code_actions(&package, &uri);

        assert!(actions.is_empty());
    }

    #[test]
    fn constraint_returns_empty_for_gte_prefix() {
        let package = make_package("lodash", ">=4.17.19", 3, 15, 9);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_constraint_code_actions(&package, &uri);

        assert!(actions.is_empty());
    }

    #[test]
    fn constraint_returns_empty_for_major_only_version() {
        let package = make_package("node", "^22", 3, 15, 3);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_constraint_code_actions(&package, &uri);

        assert!(actions.is_empty());
    }

    #[test]
    fn constraint_returns_empty_for_major_minor_version() {
        let package = make_package("node", "^22.0", 3, 15, 5);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_constraint_code_actions(&package, &uri);

        assert!(actions.is_empty());
    }

    // ── PyPI constraint action tests ──

    #[test]
    fn pypi_constraint_from_gte_offers_pin_and_compatible() {
        let package = make_pypi_package("requests", ">=2.28.0", 3, 15);
        let uri = Url::parse("file:///test/pyproject.toml").unwrap();

        let actions = generate_pypi_constraint_code_actions(&package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Pin to exact version (==): ==2.28.0");
        assert_eq!(actions[1].title, "Compatible release (~=): ~=2.28.0");
    }

    #[test]
    fn pypi_constraint_from_eq_offers_minimum_and_compatible() {
        let package = make_pypi_package("django", "==2.0.0", 3, 15);
        let uri = Url::parse("file:///test/pyproject.toml").unwrap();

        let actions = generate_pypi_constraint_code_actions(&package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Minimum version (>=): >=2.0.0");
        assert_eq!(actions[1].title, "Compatible release (~=): ~=2.0.0");
    }

    #[test]
    fn pypi_constraint_from_compatible_offers_pin_and_minimum() {
        let package = make_pypi_package("numpy", "~=1.21.0", 3, 15);
        let uri = Url::parse("file:///test/pyproject.toml").unwrap();

        let actions = generate_pypi_constraint_code_actions(&package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Pin to exact version (==): ==1.21.0");
        assert_eq!(actions[1].title, "Minimum version (>=): >=1.21.0");
    }

    #[test]
    fn pypi_constraint_returns_empty_for_compound_range() {
        let package = make_pypi_package("django", ">=3.2, <4.0", 3, 15);
        let uri = Url::parse("file:///test/pyproject.toml").unwrap();

        let actions = generate_pypi_constraint_code_actions(&package, &uri);

        assert!(actions.is_empty());
    }

    #[test]
    fn pypi_constraint_returns_empty_for_bare_version() {
        let package = make_pypi_package("requests", "2.28.0", 3, 15);
        let uri = Url::parse("file:///test/pyproject.toml").unwrap();

        let actions = generate_pypi_constraint_code_actions(&package, &uri);

        assert!(actions.is_empty());
    }

    // ── Helper function tests ──

    #[rstest]
    #[case("^4.17.19", "4.17.19")]
    #[case("~4.17.19", "4.17.19")]
    #[case("4.17.19", "4.17.19")]
    #[case(">=2.0.0", "2.0.0")]
    #[case("==2.0.0", "2.0.0")]
    #[case("~=1.4.2", "1.4.2")]
    #[case("v1.0.0", "1.0.0")]
    fn test_strip_version_prefix(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(strip_version_prefix(input), expected);
    }

    #[rstest]
    #[case(">=2.0.0", true)]
    #[case("==2.0.0", true)]
    #[case("~=1.4.2", true)]
    #[case(">=2.0, <3.0", false)]
    #[case("!=2.0.0", true)]
    fn test_is_simple_pypi_version(#[case] input: &str, #[case] expected: bool) {
        assert_eq!(is_simple_pypi_version(input), expected);
    }

    #[rstest]
    #[case(">=2.0.0", Some((">=", "2.0.0")))]
    #[case("==2.0.0", Some(("==", "2.0.0")))]
    #[case("~=1.4.2", Some(("~=", "1.4.2")))]
    #[case("!=2.0.0", None)]
    #[case("2.0.0", None)]
    #[case("^4.0.0", None)]
    fn test_parse_pypi_version(#[case] input: &str, #[case] expected: Option<(&str, &str)>) {
        assert_eq!(parse_pypi_version(input), expected);
    }
}
