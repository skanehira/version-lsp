//! Common types for parsers

/// Type of package registry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegistryType {
    /// GitHub Actions (actions/checkout@v3)
    GitHubActions,
    /// npm registry (package.json)
    Npm,
    /// crates.io (Cargo.toml)
    CratesIo,
    /// Go proxy (go.mod)
    GoProxy,
}

impl RegistryType {
    /// Returns the string representation of the registry type
    pub fn as_str(&self) -> &'static str {
        match self {
            RegistryType::GitHubActions => "github_actions",
            RegistryType::Npm => "npm",
            RegistryType::CratesIo => "crates_io",
            RegistryType::GoProxy => "go_proxy",
        }
    }
}

/// Information about a package dependency found in a file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    /// Package name (e.g., "actions/checkout", "serde", "lodash")
    pub name: String,
    /// Current version specified in the file (may be extracted from comment if hash is used)
    pub version: String,
    /// Commit hash if pinned to specific commit (GitHub Actions only)
    /// When present, version may be extracted from trailing comment
    pub commit_hash: Option<String>,
    /// Type of registry this package belongs to
    pub registry_type: RegistryType,
    /// Byte offset of the version string in the source (start)
    pub start_offset: usize,
    /// Byte offset of the version string in the source (end)
    pub end_offset: usize,
    /// Line number (0-indexed)
    pub line: usize,
    /// Column number (0-indexed)
    pub column: usize,
}
