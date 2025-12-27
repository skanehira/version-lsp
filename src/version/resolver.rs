//! Latest version resolver trait
//!
//! Provides registry-specific logic for determining the "latest" version
//! from available versions and optional dist-tags.

use std::collections::HashMap;

use crate::version::resolvers::find_semantic_max;

/// Trait for registry-specific latest version resolution logic
///
/// Each registry has different rules for determining the "latest" version:
/// - Default: Semantic max version (used by Go, Crates.io, JSR)
/// - GitHub Actions: Use published order (last item in the fetched list)
/// - npm/pnpm: Use dist-tag "latest", fallback to semantic max
pub trait LatestVersionResolver: Send + Sync {
    /// Determine the "latest" version from available versions
    ///
    /// # Arguments
    /// * `versions` - All available versions from the registry
    /// * `dist_tags` - Optional dist-tags mapping (e.g., {"latest": "4.17.21"})
    ///
    /// # Returns
    /// The resolved latest version, or None if no valid version found
    ///
    /// Default implementation returns the semantically maximum version.
    /// Override this for registries that need different logic (e.g., npm with dist-tags).
    fn resolve_latest(
        &self,
        versions: &[String],
        _dist_tags: Option<&HashMap<String, String>>,
    ) -> Option<String> {
        find_semantic_max(versions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Test resolver that uses default implementation
    struct TestResolver;

    impl LatestVersionResolver for TestResolver {
        // Uses default implementation
    }

    #[test]
    fn default_implementation_uses_find_semantic_max() {
        let resolver = TestResolver;
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
    fn default_implementation_ignores_dist_tags() {
        let resolver = TestResolver;
        let versions = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        let mut dist_tags = HashMap::new();
        dist_tags.insert("latest".to_string(), "1.0.0".to_string());

        // Default implementation ignores dist_tags and returns semantic max
        assert_eq!(
            resolver.resolve_latest(&versions, Some(&dist_tags)),
            Some("2.0.0".to_string())
        );
    }

    #[test]
    fn default_implementation_returns_none_for_empty_versions() {
        let resolver = TestResolver;
        let versions: Vec<String> = vec![];

        assert_eq!(resolver.resolve_latest(&versions, None), None);
    }
}
