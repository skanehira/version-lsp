//! Registry-specific latest version resolvers

mod crates;
mod github_actions;
mod go;
mod jsr;
mod npm;
mod pnpm;

pub use crates::CratesLatestResolver;
pub use github_actions::GitHubActionsLatestResolver;
pub use go::GoLatestResolver;
pub use jsr::JsrLatestResolver;
pub use npm::NpmLatestResolver;
pub use pnpm::PnpmCatalogLatestResolver;

use semver::Version;

/// Find the semantically maximum version from a list
///
/// Handles both `v`-prefixed (e.g., "v1.0.0") and non-prefixed versions.
/// Invalid versions are skipped.
pub fn find_semantic_max(versions: &[String]) -> Option<String> {
    versions
        .iter()
        .filter_map(|v| {
            let v_stripped = v.strip_prefix('v').unwrap_or(v);
            Version::parse(v_stripped).ok().map(|parsed| (v, parsed))
        })
        .max_by(|(_, a), (_, b)| a.cmp(b))
        .map(|(original, _)| original.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(vec![], None)]
    #[case(vec!["v1.0.0", "v2.0.0", "v1.5.0"], Some("v2.0.0"))]
    #[case(vec!["1.0.0", "2.0.0", "1.5.0"], Some("2.0.0"))]
    #[case(vec!["v1.0.0", "2.0.0", "v1.5.0"], Some("2.0.0"))]
    #[case(vec!["invalid", "v1.0.0", "not-semver"], Some("v1.0.0"))]
    #[case(vec!["invalid", "not-semver"], None)]
    fn find_semantic_max_returns_expected(
        #[case] versions: Vec<&str>,
        #[case] expected: Option<&str>,
    ) {
        let versions: Vec<String> = versions.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(
            find_semantic_max(&versions),
            expected.map(|s| s.to_string())
        );
    }
}
