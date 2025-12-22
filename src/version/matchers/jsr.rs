//! JSR version matcher
//!
//! JSR uses the same semver specification as npm, so we delegate
//! to the npm version matching logic.

use crate::parser::types::RegistryType;
use crate::version::matcher::VersionMatcher;
use crate::version::matchers::npm::{npm_compare_to_latest, npm_version_exists};
use crate::version::semver::CompareResult;

pub struct JsrVersionMatcher;

impl VersionMatcher for JsrVersionMatcher {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Jsr
    }

    fn version_exists(&self, version_spec: &str, available_versions: &[String]) -> bool {
        npm_version_exists(version_spec, available_versions)
    }

    fn compare_to_latest(&self, current_version: &str, latest_version: &str) -> CompareResult {
        npm_compare_to_latest(current_version, latest_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("^1.0.0", &["1.0.0", "1.5.0", "2.0.0"], true)]
    #[case("^1.0.0", &["0.9.0", "2.0.0"], false)]
    #[case("~1.2.0", &["1.2.0", "1.2.5"], true)]
    #[case("1.0.0", &["1.0.0"], true)]
    fn version_exists_returns_expected(
        #[case] version_spec: &str,
        #[case] available: &[&str],
        #[case] expected: bool,
    ) {
        let matcher = JsrVersionMatcher;
        let available: Vec<String> = available.iter().map(|s| s.to_string()).collect();
        assert_eq!(matcher.version_exists(version_spec, &available), expected);
    }

    #[rstest]
    #[case("1.0.0", "1.0.0", CompareResult::Latest)]
    #[case("1.0.0", "2.0.0", CompareResult::Outdated)]
    #[case("2.0.0", "1.0.0", CompareResult::Newer)]
    #[case("invalid", "1.0.0", CompareResult::Invalid)]
    fn compare_to_latest_returns_expected(
        #[case] current: &str,
        #[case] latest: &str,
        #[case] expected: CompareResult,
    ) {
        let matcher = JsrVersionMatcher;
        assert_eq!(matcher.compare_to_latest(current, latest), expected);
    }
}
