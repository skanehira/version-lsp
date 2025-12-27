//! GitHub Actions latest version resolver

use crate::version::resolver::LatestVersionResolver;

/// GitHub Actions latest version resolver
///
/// Uses published order (last item) to match GitHub release ordering.
pub struct GitHubActionsLatestResolver;

impl LatestVersionResolver for GitHubActionsLatestResolver {
    fn resolve_latest(
        &self,
        versions: &[String],
        _dist_tags: Option<&std::collections::HashMap<String, String>>,
    ) -> Option<String> {
        versions.last().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn returns_last_version_in_list() {
        let resolver = GitHubActionsLatestResolver;
        let versions = vec![
            "v3.0.0".to_string(),
            "v4.0.0".to_string(),
            "v4.1.0".to_string(),
        ];

        assert_eq!(
            resolver.resolve_latest(&versions, None),
            Some("v4.1.0".to_string())
        );
    }

    #[test]
    fn prefers_last_over_semantic_max() {
        let resolver = GitHubActionsLatestResolver;
        let versions = vec!["v2.0.0".to_string(), "v1.9.9".to_string()];

        assert_eq!(
            resolver.resolve_latest(&versions, None),
            Some("v1.9.9".to_string())
        );
    }

    #[test]
    fn supports_non_semver_tags() {
        let resolver = GitHubActionsLatestResolver;
        let versions = vec!["v4".to_string(), "v4-beta".to_string()];

        assert_eq!(
            resolver.resolve_latest(&versions, None),
            Some("v4-beta".to_string())
        );
    }

    #[test]
    fn returns_none_for_empty_versions() {
        let resolver = GitHubActionsLatestResolver;
        let versions: Vec<String> = vec![];

        assert_eq!(resolver.resolve_latest(&versions, None), None);
    }
}
