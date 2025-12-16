use semver::Version;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareResult {
    Latest,
    Outdated,
    Newer,
    Invalid,
}

/// Parse a version string into a semver::Version, normalizing partial versions.
///
/// Handles partial versions like "1" or "1.2" by padding with zeros.
/// Does NOT strip 'v' prefix (use `normalize_version` first if needed).
///
/// Examples:
/// - "1" -> Version(1, 0, 0)
/// - "1.2" -> Version(1, 2, 0)
/// - "1.2.3" -> Version(1, 2, 3)
pub fn parse_version(version: &str) -> Option<Version> {
    let parts: Vec<&str> = version.split('.').collect();
    let normalized = match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => version.to_string(),
    };
    Version::parse(&normalized).ok()
}
