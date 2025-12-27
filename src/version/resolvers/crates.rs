//! Crates.io latest version resolver

use crate::version::resolver::LatestVersionResolver;

/// Crates.io latest version resolver
///
/// Uses default implementation (semantic max version).
/// Default behavior is tested in `src/version/resolver.rs`.
pub struct CratesLatestResolver;

impl LatestVersionResolver for CratesLatestResolver {
    // Uses default implementation (semantic max)
}
