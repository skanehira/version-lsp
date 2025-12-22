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
