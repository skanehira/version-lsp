//! Upgrade code actions — version bumping across all registries

use crate::parser::types::{ExtraInfo, PackageInfo};
use crate::version::checker::VersionStorer;
use crate::version::registries::github::TagShaFetcher;
use crate::version::semver::{
    calculate_latest_major, calculate_latest_minor, calculate_latest_patch, calculate_next_major,
    calculate_next_minor,
};
use std::collections::HashMap;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Position, Range, TextEdit, Url, WorkspaceEdit,
};

use super::{create_bump_action, extract_version_prefix};

/// Compute deduplicated bump targets from smallest to largest jump.
///
/// Returns `(bare_version, label)` pairs with duplicates removed.
fn compute_bump_targets<'a>(current: &str, versions: &[String]) -> Vec<(String, &'a str)> {
    let patch = calculate_latest_patch(current, versions);
    let next_minor = calculate_next_minor(current, versions);
    let minor = calculate_latest_minor(current, versions);
    let next_major = calculate_next_major(current, versions);
    let major = calculate_latest_major(current, versions);

    // Only include "next" targets when they differ from "latest" — otherwise
    // the "next" label would win the dedup race and hide the "latest" label.
    let effective_next_minor = next_minor.filter(|nm| minor.as_ref() != Some(nm));
    let effective_next_major = next_major.filter(|nm| major.as_ref() != Some(nm));

    let mut seen = std::collections::HashSet::new();
    let candidates = [
        (patch, "latest patch"),
        (effective_next_minor, "next minor"),
        (minor, "latest minor"),
        (effective_next_major, "next major"),
        (major, "latest major"),
    ];

    candidates
        .into_iter()
        .filter_map(|(version, label)| {
            let v = version?;
            if seen.insert(v.clone()) {
                Some((v, label))
            } else {
                None
            }
        })
        .collect()
}

/// Generate upgrade code actions
///
/// Creates up to 5 code actions (patch, next minor, minor, next major, major)
/// based on available versions. Preserves the current version prefix.
/// Returns an empty Vec if no newer versions are available or if versions are not in cache.
pub fn generate_upgrade_code_actions<S: VersionStorer>(
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

    compute_bump_targets(current, &versions)
        .into_iter()
        .map(|(v, label)| {
            let new_version = format!("{prefix}{v}");
            create_bump_action(
                &format!("Upgrade to {label}: {new_version}"),
                &new_version,
                package,
                uri,
            )
        })
        .collect()
}

/// Generate upgrade code actions with SHA fetching for GitHub Actions
///
/// When the package has a commit hash (GitHub Actions), this function will fetch
/// the commit SHA for each bump target and generate code actions that replace
/// the hash (and optionally the comment) with the new SHA and version.
pub async fn generate_upgrade_code_actions_with_sha<S: VersionStorer, F: TagShaFetcher>(
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
        return generate_hash_only_actions(storer, package, uri, sha_fetcher).await;
    }

    let current = &package.version;
    let prefix = extract_version_prefix(current);

    let mut actions = Vec::new();

    for (v, label) in compute_bump_targets(current, &versions) {
        let new_version = format!("{prefix}{v}");

        // If package has a commit hash, we need to fetch the SHA for the new version
        if package.commit_hash.is_some() {
            let sha_result = sha_fetcher.fetch_tag_sha(&package.name, &new_version).await;

            let Ok(new_sha) = sha_result else {
                continue;
            };

            actions.push(create_hash_bump_action(
                &format!("Upgrade to {label}: {new_version}"),
                &new_sha,
                &new_version,
                package,
                uri,
            ));
        } else {
            actions.push(create_bump_action(
                &format!("Upgrade to {label}: {new_version}"),
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
        &format!("Upgrade to latest: {latest}"),
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
    use crate::version::error::{CacheError, RegistryError};
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
    fn upgrade_returns_three_actions_when_all_levels_available() {
        let storer = MockStorer::new(vec!["4.17.19", "4.17.21", "4.18.0", "5.0.0"]);
        let package = make_package("lodash", "4.17.19", 3, 15, 7);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].title, "Upgrade to latest patch: 4.17.21");
        assert_eq!(actions[1].title, "Upgrade to latest minor: 4.18.0");
        assert_eq!(actions[2].title, "Upgrade to latest major: 5.0.0");
    }

    #[test]
    fn upgrade_returns_empty_when_no_versions_in_cache() {
        let storer = MockStorer::new(vec![]);
        let package = make_package("lodash", "4.17.19", 3, 15, 7);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert!(actions.is_empty());
    }

    #[test]
    fn upgrade_returns_empty_when_already_latest() {
        let storer = MockStorer::new(vec!["5.0.0"]);
        let package = make_package("lodash", "5.0.0", 3, 15, 5);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert!(actions.is_empty());
    }

    #[test]
    fn upgrade_creates_correct_text_edit() {
        let storer = MockStorer::new(vec!["4.17.19", "4.17.21"]);
        let package = make_package("lodash", "4.17.19", 3, 15, 7);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

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
    fn upgrade_preserves_caret_prefix() {
        let storer = MockStorer::new(vec!["4.17.19", "4.17.21", "4.18.0", "5.0.0"]);
        let package = make_package("lodash", "^4.17.19", 3, 15, 8);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].title, "Upgrade to latest patch: ^4.17.21");
        assert_eq!(actions[1].title, "Upgrade to latest minor: ^4.18.0");
        assert_eq!(actions[2].title, "Upgrade to latest major: ^5.0.0");

        // Verify TextEdit preserves prefix
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits[0].new_text, "^4.17.21");
    }

    #[test]
    fn upgrade_preserves_tilde_prefix() {
        let storer = MockStorer::new(vec!["4.17.19", "4.17.21"]);
        let package = make_package("lodash", "~4.17.19", 3, 15, 8);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Upgrade to latest patch: ~4.17.21");
    }

    #[test]
    fn upgrade_preserves_gte_prefix() {
        let storer = MockStorer::new(vec!["4.17.19", "5.0.0"]);
        let package = make_package("lodash", ">=4.17.19", 3, 15, 9);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Upgrade to latest major: >=5.0.0");
    }

    #[test]
    fn upgrade_preserves_v_prefix_for_go() {
        let storer = MockStorer::new(vec!["0.14.0", "0.15.0", "1.0.0"]);
        let package = make_package("golang.org/x/text", "v0.14.0", 3, 15, 7);
        let uri = Url::parse("file:///test/go.mod").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Upgrade to latest minor: v0.15.0");
        assert_eq!(actions[1].title, "Upgrade to latest major: v1.0.0");
    }

    #[test]
    fn upgrade_shows_next_and_latest_major_when_multiple_behind() {
        let storer = MockStorer::new(vec!["2.0.0", "3.0.0", "3.5.0", "4.0.0", "4.2.0", "5.0.0"]);
        let package = make_package("lodash", "^2.0.0", 3, 15, 6);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Upgrade to next major: ^3.5.0");
        assert_eq!(actions[1].title, "Upgrade to latest major: ^5.0.0");
    }

    #[test]
    fn upgrade_shows_next_and_latest_minor_when_multiple_behind() {
        let storer = MockStorer::new(vec!["4.17.0", "4.18.0", "4.18.5", "4.19.0", "4.20.0"]);
        let package = make_package("lodash", "^4.17.0", 3, 15, 7);
        let uri = Url::parse("file:///test/package.json").unwrap();

        let actions = generate_upgrade_code_actions(&storer, &package, &uri);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Upgrade to next minor: ^4.18.5");
        assert_eq!(actions[1].title, "Upgrade to latest minor: ^4.20.0");
    }

    // ── Upgrade with SHA tests ──

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
    async fn upgrade_with_sha_pattern1_hash_only() {
        let storer = MockStorer::new(vec!["v4.1.5", "v4.1.6"]);
        let sha_fetcher =
            MockTagShaFetcher::new(vec![("v4.1.6", "newsha1234567890newsha1234567890newsha12")]);
        let package = make_github_actions_package_hash_only(
            "actions/checkout",
            "8e5e7e5ab8b370d6c329ec480221332ada57f0ab",
            "8e5e7e5ab8b370d6c329ec480221332ada57f0ab",
            4,
            31,
        );
        let uri = Url::parse("file:///test/.github/workflows/ci.yml").unwrap();

        let actions =
            generate_upgrade_code_actions_with_sha(&storer, &package, &uri, &sha_fetcher).await;

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Upgrade to latest: v4.1.6");

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
        assert_eq!(edits[0].range.end.character, 71);
    }

    #[tokio::test]
    async fn upgrade_with_sha_pattern2_hash_with_comment() {
        let storer = MockStorer::new(vec!["v4.1.5", "v4.1.6"]);
        let sha_fetcher =
            MockTagShaFetcher::new(vec![("v4.1.6", "newsha1234567890newsha1234567890newsha12")]);
        let package = make_github_actions_package_with_comment(
            "actions/checkout",
            "v4.1.5",
            "8e5e7e5ab8b370d6c329ec480221332ada57f0ab",
            4,
            31,
            71,
            80,
        );
        let uri = Url::parse("file:///test/.github/workflows/ci.yml").unwrap();

        let actions =
            generate_upgrade_code_actions_with_sha(&storer, &package, &uri, &sha_fetcher).await;

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Upgrade to latest patch: v4.1.6");

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
        assert_eq!(edits[0].range.end.character, 80);
    }

    #[tokio::test]
    async fn upgrade_with_sha_returns_empty_when_sha_fetch_fails() {
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
            generate_upgrade_code_actions_with_sha(&storer, &package, &uri, &sha_fetcher).await;

        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn upgrade_with_sha_pattern3_version_tag_only() {
        let storer = MockStorer::new(vec!["3.0.0", "4.0.0"]);
        let sha_fetcher = MockTagShaFetcher::new(vec![]);
        let package = make_package("actions/checkout", "v3.0.0", 4, 31, 6);
        let uri = Url::parse("file:///test/.github/workflows/ci.yml").unwrap();

        let actions =
            generate_upgrade_code_actions_with_sha(&storer, &package, &uri, &sha_fetcher).await;

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Upgrade to latest major: v4.0.0");

        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        let edits = changes.get(&uri).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "v4.0.0");
        assert_eq!(edits[0].range.start.character, 31);
        assert_eq!(edits[0].range.end.character, 37);
    }

    #[rstest]
    #[case("^4.17.19", "^")]
    #[case("~4.17.19", "~")]
    #[case("4.17.19", "")]
    #[case(">=2.0.0", ">=")]
    #[case("==2.0.0", "==")]
    #[case("~=1.4.2", "~=")]
    #[case("!=2.0.0", "!=")]
    #[case("v1.0.0", "v")]
    fn test_extract_version_prefix(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(extract_version_prefix(input), expected);
    }
}
