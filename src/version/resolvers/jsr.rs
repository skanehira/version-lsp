//! JSR latest version resolver

use crate::version::resolver::LatestVersionResolver;

/// JSR latest version resolver
///
/// Uses default implementation (semantic max version).
/// Default behavior is tested in `src/version/resolver.rs`.
pub struct JsrLatestResolver;

impl LatestVersionResolver for JsrLatestResolver {
    // Uses default implementation (semantic max)
}
