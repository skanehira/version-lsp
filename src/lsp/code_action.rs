//! Code action generation for version bumping

use crate::parser::types::{ExtraInfo, PackageInfo};
use crate::version::checker::VersionStorer;
use crate::version::registries::github::TagShaFetcher;
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

/// Extract version prefix (^, ~, >=, <=, >, <, =, v) from a version string
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
    } else if version.starts_with('v') {
        "v"
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

/// Generate Code Actions for version bumping with SHA fetching for GitHub Actions
///
/// When the package has a commit hash (GitHub Actions), this function will fetch
/// the commit SHA for each bump target and generate code actions that replace
/// the hash (and optionally the comment) with the new SHA and version.
pub async fn generate_bump_code_actions_with_sha<S: VersionStorer, F: TagShaFetcher>(
    storer: &S,
    package: &PackageInfo,
    uri: &Url,
    sha_fetcher: &F,
) -> Vec<CodeAction> {
    let Ok(versions) = storer.get_versions(package.registry_type, &package.name) else {
        return vec![];
    };

    if versions.is_empty() {
        return vec![];
    }

    // For hash-only packages (no comment version), we need special handling
    // because the version field contains the hash itself, not a semver version
    let is_hash_only = package.commit_hash.is_some() && package.extra_info.is_none();

    if is_hash_only {
        // Pattern 1: Hash only - just offer the latest version
        return generate_hash_only_actions(storer, package, uri, sha_fetcher, &versions).await;
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

    let mut actions = Vec::new();

    for (version, label) in bump_targets {
        let Some(v) = version else { continue };
        if !seen.insert(v.clone()) {
            continue;
        }

        let new_version = format!("{prefix}{v}");

        // If package has a commit hash, we need to fetch the SHA for the new version
        if package.commit_hash.is_some() {
            // Fetch the SHA for this version
            let sha_result = sha_fetcher.fetch_tag_sha(&package.name, &new_version).await;

            let Ok(new_sha) = sha_result else {
                // SHA fetch failed, skip this code action
                continue;
            };

            let action = create_hash_bump_action(
                &format!("Bump to latest {label}: {new_version}"),
                &new_sha,
                &new_version,
                package,
                uri,
            );
            actions.push(action);
        } else {
            // No commit hash, use the existing logic
            actions.push(create_bump_action(
                &format!("Bump to latest {label}: {new_version}"),
                &new_version,
                package,
                uri,
            ));
        }
    }

    actions
}

/// Generate code actions for hash-only packages (Pattern 1)
///
/// For hash-only packages, we don't know the current semantic version,
/// so we just offer to update to the latest available version.
async fn generate_hash_only_actions<S: VersionStorer, F: TagShaFetcher>(
    storer: &S,
    package: &PackageInfo,
    uri: &Url,
    sha_fetcher: &F,
    _versions: &[String],
) -> Vec<CodeAction> {
    // For hash-only, we don't know the current version, so just offer the latest
    let Ok(Some(latest)) = storer.get_latest_version(package.registry_type, &package.name) else {
        return vec![];
    };

    // Fetch the SHA for the latest version
    let sha_result = sha_fetcher.fetch_tag_sha(&package.name, &latest).await;

    let Ok(new_sha) = sha_result else {
        return vec![];
    };

    vec![create_hash_bump_action(
        &format!("Bump to latest: {latest}"),
        &new_sha,
        &latest,
        package,
        uri,
    )]
}

/// Create a code action for hash-based version bumping (GitHub Actions)
fn create_hash_bump_action(
    title: &str,
    new_sha: &str,
    new_version: &str,
    package: &PackageInfo,
    uri: &Url,
) -> CodeAction {
    let start = Position {
        line: package.line as u32,
        character: package.column as u32,
    };

    // Determine the end position and new text based on whether there's a comment
    let (end_character, new_text) = match &package.extra_info {
        Some(ExtraInfo::GitHubActions {
            comment_end_offset, ..
        }) => {
            // Pattern 2: Hash + comment
            // Calculate end column from the comment end offset
            // The comment_end_offset is absolute, but we need to convert it to a column
            // We need to find the line start offset to calculate the column
            let hash_start_offset = package.start_offset;
            let end_col = package.column + (comment_end_offset - hash_start_offset);
            (end_col as u32, format!("{new_sha} # {new_version}"))
        }
        None => {
            // Pattern 1: Hash only
            // Replace just the hash (40 characters)
            let hash_len = package.commit_hash.as_ref().map(|h| h.len()).unwrap_or(40);
            (package.column as u32 + hash_len as u32, new_sha.to_string())
        }
    };

    let end = Position {
        line: package.line as u32,
        character: end_character,
    };

    let text_edit = TextEdit {
        range: Range { start, end },
        new_text,
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
            extra_info: None,
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
    fn test_package_index_find_at_position(
        #[case] position: Position,
        #[case] packages: Vec<PackageInfo>,
        #[case] expected_name: Option<&str>,
    ) {
        let index = PackageIndex::new(&packages);
        let result = index.find_at_position(position);
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

        fn mark_not_found(
            &self,
            _registry_type: RegistryType,
            _package_name: &str,
        ) -> Result<(), CacheError> {
            Ok(())
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

    #[test]
    fn generate_bump_code_actions_preserves_v_prefix_for_go() {
        let storer = MockStorer::new(vec!["0.14.0", "0.15.0", "1.0.0"]);
        let package = make_package("golang.org/x/text", "v0.14.0", 3, 15, 7);
        let uri = Url::parse("file:///test/go.mod").unwrap();

        let actions = generate_bump_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Bump to latest minor: v0.15.0");
        assert_eq!(actions[1].title, "Bump to latest major: v1.0.0");
    }

    // Tests for generate_bump_code_actions_with_sha

    use crate::version::error::RegistryError;

    /// Mock TagShaFetcher for testing
    struct MockTagShaFetcher {
        sha_map: std::collections::HashMap<String, String>,
        should_fail: bool,
    }

    impl MockTagShaFetcher {
        fn new(sha_map: Vec<(&str, &str)>) -> Self {
            Self {
                sha_map: sha_map
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                should_fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                sha_map: std::collections::HashMap::new(),
                should_fail: true,
            }
        }
    }

    #[async_trait::async_trait]
    impl TagShaFetcher for MockTagShaFetcher {
        async fn fetch_tag_sha(
            &self,
            _package_name: &str,
            tag_name: &str,
        ) -> Result<String, RegistryError> {
            if self.should_fail {
                return Err(RegistryError::NotFound("SHA fetch failed".to_string()));
            }
            self.sha_map
                .get(tag_name)
                .cloned()
                .ok_or_else(|| RegistryError::NotFound(format!("Tag {} not found", tag_name)))
        }
    }

    fn make_github_actions_package_hash_only(
        name: &str,
        version: &str,
        commit_hash: &str,
        line: u32,
        column: u32,
    ) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            commit_hash: Some(commit_hash.to_string()),
            registry_type: RegistryType::GitHubActions,
            start_offset: column as usize,
            end_offset: column as usize + commit_hash.len(),
            line: line as usize,
            column: column as usize,
            extra_info: None,
        }
    }

    fn make_github_actions_package_with_comment(
        name: &str,
        version: &str,
        commit_hash: &str,
        line: u32,
        column: u32,
        comment_start: usize,
        comment_end: usize,
    ) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            commit_hash: Some(commit_hash.to_string()),
            registry_type: RegistryType::GitHubActions,
            start_offset: column as usize,
            end_offset: column as usize + commit_hash.len(),
            line: line as usize,
            column: column as usize,
            extra_info: Some(ExtraInfo::GitHubActions {
                comment_text: version.to_string(),
                comment_start_offset: comment_start,
                comment_end_offset: comment_end,
            }),
        }
    }

    #[tokio::test]
    async fn generate_bump_code_actions_with_sha_pattern1_hash_only() {
        // Pattern 1: Hash only → New hash
        // Note: GitHub Actions releases typically have "v" prefix in tag names
        let storer = MockStorer::new(vec!["v4.1.5", "v4.1.6"]);
        let sha_fetcher =
            MockTagShaFetcher::new(vec![("v4.1.6", "newsha1234567890newsha1234567890newsha12")]);
        // Column 31, hash is 40 chars
        let package = make_github_actions_package_hash_only(
            "actions/checkout",
            "8e5e7e5ab8b370d6c329ec480221332ada57f0ab", // version is the hash when no comment
            "8e5e7e5ab8b370d6c329ec480221332ada57f0ab",
            4,
            31,
        );
        let uri = Url::parse("file:///test/.github/workflows/ci.yml").unwrap();

        let actions =
            generate_bump_code_actions_with_sha(&storer, &package, &uri, &sha_fetcher).await;

        assert_eq!(actions.len(), 1);
        // For hash-only packages, we don't know the current version, so just offer "Bump to latest"
        assert_eq!(actions[0].title, "Bump to latest: v4.1.6");

        // Check the text edit replaces the hash with the new SHA
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "newsha1234567890newsha1234567890newsha12"
        );
        assert_eq!(edits[0].range.start.line, 4);
        assert_eq!(edits[0].range.start.character, 31);
        assert_eq!(edits[0].range.end.line, 4);
        assert_eq!(edits[0].range.end.character, 71); // 31 + 40 (hash length)
    }

    #[tokio::test]
    async fn generate_bump_code_actions_with_sha_pattern2_hash_with_comment() {
        // Pattern 2: Hash + comment → New hash + new comment
        // Note: GitHub Actions releases typically have "v" prefix in tag names
        let storer = MockStorer::new(vec!["v4.1.5", "v4.1.6"]);
        let sha_fetcher =
            MockTagShaFetcher::new(vec![("v4.1.6", "newsha1234567890newsha1234567890newsha12")]);
        // Column 31, hash is 40 chars, " # v4.1.5" comment follows
        // comment_start_offset = 31 + 40 = 71, comment_end_offset = 71 + 9 = 80 (includes " # v4.1.5")
        let package = make_github_actions_package_with_comment(
            "actions/checkout",
            "v4.1.5",
            "8e5e7e5ab8b370d6c329ec480221332ada57f0ab",
            4,
            31,
            71, // comment start (# position)
            80, // comment end
        );
        let uri = Url::parse("file:///test/.github/workflows/ci.yml").unwrap();

        let actions =
            generate_bump_code_actions_with_sha(&storer, &package, &uri, &sha_fetcher).await;

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Bump to latest patch: v4.1.6");

        // Check the text edit replaces both hash and comment
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "newsha1234567890newsha1234567890newsha12 # v4.1.6"
        );
        assert_eq!(edits[0].range.start.line, 4);
        assert_eq!(edits[0].range.start.character, 31);
        assert_eq!(edits[0].range.end.line, 4);
        assert_eq!(edits[0].range.end.character, 80); // column + (comment_end - start_offset)
    }

    #[tokio::test]
    async fn generate_bump_code_actions_with_sha_returns_empty_when_sha_fetch_fails() {
        // SHA fetch failure → No code action generated
        let storer = MockStorer::new(vec!["v4.1.5", "v4.1.6"]);
        let sha_fetcher = MockTagShaFetcher::failing();
        let package = make_github_actions_package_hash_only(
            "actions/checkout",
            "8e5e7e5ab8b370d6c329ec480221332ada57f0ab",
            "8e5e7e5ab8b370d6c329ec480221332ada57f0ab",
            4,
            31,
        );
        let uri = Url::parse("file:///test/.github/workflows/ci.yml").unwrap();

        let actions =
            generate_bump_code_actions_with_sha(&storer, &package, &uri, &sha_fetcher).await;

        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn generate_bump_code_actions_with_sha_pattern3_version_tag_only() {
        // Pattern 3: Version tag only → Existing behavior (no commit hash)
        let storer = MockStorer::new(vec!["3.0.0", "4.0.0"]);
        let sha_fetcher = MockTagShaFetcher::new(vec![]);
        let package = make_package("actions/checkout", "v3.0.0", 4, 31, 6);
        let uri = Url::parse("file:///test/.github/workflows/ci.yml").unwrap();

        let actions =
            generate_bump_code_actions_with_sha(&storer, &package, &uri, &sha_fetcher).await;

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Bump to latest major: v4.0.0");

        // Check the text edit replaces just the version string
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "v4.0.0");
        assert_eq!(edits[0].range.start.character, 31);
        assert_eq!(edits[0].range.end.character, 37); // 31 + 6 (version length)
    }
}
