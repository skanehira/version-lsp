//! Code action generation for version bumping

use crate::parser::types::PackageInfo;
use crate::version::checker::VersionStorer;
use crate::version::semver::{
    calculate_latest_major, calculate_latest_minor, calculate_latest_patch,
};
use std::collections::HashMap;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Position, Range, TextEdit, Url, WorkspaceEdit,
};

/// Index of packages grouped by line number for efficient lookup
pub struct PackageIndex<'a> {
    by_line: HashMap<u32, Vec<&'a PackageInfo>>,
}

impl<'a> PackageIndex<'a> {
    /// Build an index from a slice of packages
    pub fn new(packages: &'a [PackageInfo]) -> Self {
        let mut by_line: HashMap<u32, Vec<&'a PackageInfo>> = HashMap::new();
        for pkg in packages {
            by_line.entry(pkg.line as u32).or_default().push(pkg);
        }
        Self { by_line }
    }

    /// Find the package at the given cursor position
    ///
    /// Returns the package if the cursor is within the version range of a package.
    pub fn find_at_position(&self, position: Position) -> Option<&'a PackageInfo> {
        let packages_on_line = self.by_line.get(&position.line)?;

        packages_on_line.iter().find_map(|&pkg| {
            let start_col = pkg.column as u32;
            let end_col = start_col + pkg.version.len() as u32;

            if position.character >= start_col && position.character < end_col {
                Some(pkg)
            } else {
                None
            }
        })
    }
}

/// Find the package at the given cursor position
///
/// Returns the package if the cursor is within the version range of a package.
/// For single lookups. Use PackageIndex for multiple lookups.
pub fn find_package_at_position(
    packages: &[PackageInfo],
    position: Position,
) -> Option<&PackageInfo> {
    PackageIndex::new(packages).find_at_position(position)
}

/// Extract version prefix (^, ~, >=, <=, >, <, =) from a version string
fn extract_version_prefix(version: &str) -> &str {
    if version.starts_with(">=") {
        ">="
    } else if version.starts_with("<=") {
        "<="
    } else if version.starts_with('>') {
        ">"
    } else if version.starts_with('<') {
        "<"
    } else if version.starts_with('=') {
        "="
    } else if version.starts_with('^') {
        "^"
    } else if version.starts_with('~') {
        "~"
    } else {
        ""
    }
}

/// Generate Code Actions for version bumping
///
/// Creates up to 3 code actions (patch, minor, major) based on available versions.
/// Returns an empty Vec if no newer versions are available or if versions are not in cache.
pub fn generate_bump_code_actions<S: VersionStorer>(
    storer: &S,
    package: &PackageInfo,
    uri: &Url,
) -> Vec<CodeAction> {
    let Ok(versions) = storer.get_versions(package.registry_type, &package.name) else {
        return vec![];
    };

    if versions.is_empty() {
        return vec![];
    }

    let current = &package.version;
    let prefix = extract_version_prefix(current);

    // Calculate bump targets
    let patch = calculate_latest_patch(current, &versions);
    let minor = calculate_latest_minor(current, &versions);
    let major = calculate_latest_major(current, &versions);

    // Collect unique bump targets with their labels
    let mut seen = std::collections::HashSet::new();
    let bump_targets = [(patch, "patch"), (minor, "minor"), (major, "major")];

    bump_targets
        .into_iter()
        .filter_map(|(version, label)| {
            let v = version?;
            if seen.insert(v.clone()) {
                let new_version = format!("{prefix}{v}");
                Some(create_bump_action(
                    &format!("Bump to latest {label}: {new_version}"),
                    &new_version,
                    package,
                    uri,
                ))
            } else {
                None
            }
        })
        .collect()
}

fn create_bump_action(
    title: &str,
    new_version: &str,
    package: &PackageInfo,
    uri: &Url,
) -> CodeAction {
    let start = Position {
        line: package.line as u32,
        character: package.column as u32,
    };
    let end = Position {
        line: package.line as u32,
        character: package.column as u32 + package.version.len() as u32,
    };

    let text_edit = TextEdit {
        range: Range { start, end },
        new_text: new_version.to_string(),
    };

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![text_edit]);

    CodeAction {
        title: title.to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::types::RegistryType;
    use crate::version::cache::PackageId;
    use crate::version::error::CacheError;
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
        }
    }

    #[rstest]
    #[case(
        Position { line: 3, character: 17 }, // cursor within version range
        vec![make_package("lodash", "4.17.21", 3, 15, 7)],
        Some("lodash")
    )]
    #[case(
        Position { line: 3, character: 10 }, // cursor before version range
        vec![make_package("lodash", "4.17.21", 3, 15, 7)],
        None
    )]
    #[case(
        Position { line: 3, character: 25 }, // cursor after version range
        vec![make_package("lodash", "4.17.21", 3, 15, 7)],
        None
    )]
    #[case(
        Position { line: 2, character: 17 }, // wrong line
        vec![make_package("lodash", "4.17.21", 3, 15, 7)],
        None
    )]
    #[case(
        Position { line: 5, character: 12 }, // multiple packages, cursor on second
        vec![
            make_package("lodash", "4.17.21", 3, 15, 7),
            make_package("react", "18.2.0", 5, 10, 6),
        ],
        Some("react")
    )]
    fn test_find_package_at_position(
        #[case] position: Position,
        #[case] packages: Vec<PackageInfo>,
        #[case] expected_name: Option<&str>,
    ) {
        let result = find_package_at_position(&packages, position);
        assert_eq!(result.map(|p| p.name.as_str()), expected_name);
    }

    /// Mock storer for testing code action generation
    struct MockStorer {
        versions: Vec<String>,
    }

    impl MockStorer {
        fn new(versions: Vec<&str>) -> Self {
            Self {
                versions: versions.into_iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl VersionStorer for MockStorer {
        fn get_latest_version(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<Option<String>, CacheError> {
            Ok(self.versions.iter().max().cloned())
        }

        fn get_versions(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<Vec<String>, CacheError> {
            Ok(self.versions.clone())
        }

        fn version_exists(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
            version: &str,
        ) -> Result<bool, CacheError> {
            Ok(self.versions.contains(&version.to_string()))
        }

        fn replace_versions(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
            _versions: Vec<String>,
        ) -> Result<(), CacheError> {
            Ok(())
        }

        fn get_packages_needing_refresh(&self) -> Result<Vec<PackageId>, CacheError> {
            Ok(vec![])
        }

        fn try_start_fetch(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<bool, CacheError> {
            Ok(true)
        }

        fn finish_fetch(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<(), CacheError> {
            Ok(())
        }

        fn get_dist_tag(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
            _tag_name: &str,
        ) -> Result<Option<String>, CacheError> {
            Ok(None)
        }

        fn save_dist_tags(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
            _dist_tags: &std::collections::HashMap<String, String>,
        ) -> Result<(), CacheError> {
            Ok(())
        }

        fn filter_packages_not_in_cache(
            &self,
            _registry_type: RegistryType,
            _package_names: &[String],
        ) -> Result<Vec<String>, CacheError> {
            Ok(vec![])
        }
    }

    #[test]
    fn generate_bump_code_actions_returns_three_actions_when_all_bumps_available() {
        let storer = MockStorer::new(vec!["4.17.19", "4.17.21", "4.18.0", "5.0.0"]);
        let package = make_package("lodash", "4.17.19", 3, 15, 7);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_bump_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].title, "Bump to latest patch: 4.17.21");
        assert_eq!(actions[1].title, "Bump to latest minor: 4.18.0");
        assert_eq!(actions[2].title, "Bump to latest major: 5.0.0");
    }

    #[test]
    fn generate_bump_code_actions_returns_empty_when_no_versions_in_cache() {
        let storer = MockStorer::new(vec![]);
        let package = make_package("lodash", "4.17.19", 3, 15, 7);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_bump_code_actions(&storer, &package, &uri);

        assert!(actions.is_empty());
    }

    #[test]
    fn generate_bump_code_actions_returns_empty_when_already_latest() {
        let storer = MockStorer::new(vec!["5.0.0"]);
        let package = make_package("lodash", "5.0.0", 3, 15, 5);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_bump_code_actions(&storer, &package, &uri);

        assert!(actions.is_empty());
    }

    #[test]
    fn generate_bump_code_actions_creates_correct_text_edit() {
        let storer = MockStorer::new(vec!["4.17.19", "4.17.21"]);
        let package = make_package("lodash", "4.17.19", 3, 15, 7);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_bump_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 1);
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "4.17.21");
        assert_eq!(
            edits[0].range,
            Range {
                start: Position {
                    line: 3,
                    character: 15
                },
                end: Position {
                    line: 3,
                    character: 22
                },
            }
        );
    }

    #[test]
    fn generate_bump_code_actions_preserves_caret_prefix() {
        let storer = MockStorer::new(vec!["4.17.19", "4.17.21", "4.18.0", "5.0.0"]);
        let package = make_package("lodash", "^4.17.19", 3, 15, 8);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_bump_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].title, "Bump to latest patch: ^4.17.21");
        assert_eq!(actions[1].title, "Bump to latest minor: ^4.18.0");
        assert_eq!(actions[2].title, "Bump to latest major: ^5.0.0");

        // Verify TextEdit preserves prefix
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits[0].new_text, "^4.17.21");
    }

    #[test]
    fn generate_bump_code_actions_preserves_tilde_prefix() {
        let storer = MockStorer::new(vec!["4.17.19", "4.17.21"]);
        let package = make_package("lodash", "~4.17.19", 3, 15, 8);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_bump_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Bump to latest patch: ~4.17.21");
    }

    #[test]
    fn generate_bump_code_actions_preserves_gte_prefix() {
        let storer = MockStorer::new(vec!["4.17.19", "5.0.0"]);
        let package = make_package("lodash", ">=4.17.19", 3, 15, 9);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_bump_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Bump to latest major: >=5.0.0");
    }
}
