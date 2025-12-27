//! Go latest version resolver

use crate::version::resolver::LatestVersionResolver;

/// Go latest version resolver
///
/// Uses default implementation (semantic max version).
/// Default behavior is tested in `src/version/resolver.rs`.
pub struct GoLatestResolver;

impl LatestVersionResolver for GoLatestResolver {
    // Uses default implementation (semantic max)
}
