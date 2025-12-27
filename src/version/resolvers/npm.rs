//! npm latest version resolver

use std::collections::HashMap;

use crate::version::resolver::LatestVersionResolver;
use crate::version::resolvers::find_semantic_max;

/// npm latest version resolver
///
/// Prioritizes dist-tag "latest" over semantic max version.
pub struct NpmLatestResolver;

impl LatestVersionResolver for NpmLatestResolver {
    fn resolve_latest(
        &self,
        versions: &[String],
        dist_tags: Option<&HashMap<String, String>>,
    ) -> Option<String> {
        // Try dist-tag "latest" first
        if let Some(tags) = dist_tags
            && let Some(latest) = tags.get("latest")
        {
            return Some(latest.clone());
        }

        // Fallback to semantic max
        find_semantic_max(versions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prioritizes_dist_tag_latest() {
        let resolver = NpmLatestResolver;
        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "3.0.0".to_string(),
        ];
        let mut dist_tags = HashMap::new();
        dist_tags.insert("latest".to_string(), "2.0.0".to_string());

        assert_eq!(
            resolver.resolve_latest(&versions, Some(&dist_tags)),
            Some("2.0.0".to_string())
        );
    }

    #[test]
    fn falls_back_to_semantic_max_without_dist_tag() {
        let resolver = NpmLatestResolver;
        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "1.5.0".to_string(),
        ];

        assert_eq!(
            resolver.resolve_latest(&versions, None),
            Some("2.0.0".to_string())
        );
    }

    #[test]
    fn falls_back_to_semantic_max_with_empty_dist_tags() {
        let resolver = NpmLatestResolver;
        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "1.5.0".to_string(),
        ];
        let dist_tags = HashMap::new();

        assert_eq!(
            resolver.resolve_latest(&versions, Some(&dist_tags)),
            Some("2.0.0".to_string())
        );
    }

    #[test]
    fn returns_none_for_empty_versions_and_no_dist_tag() {
        let resolver = NpmLatestResolver;
        let versions: Vec<String> = vec![];

        assert_eq!(resolver.resolve_latest(&versions, None), None);
    }
}
