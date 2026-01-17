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
    /// pnpm catalog (pnpm-workspace.yaml)
    PnpmCatalog,
    /// JSR (deno.json, deno.jsonc)
    Jsr,
    /// PyPI (pyproject.toml)
    PyPI,
}

impl RegistryType {
    /// Returns the string representation of the registry type
    pub fn as_str(&self) -> &'static str {
        match self {
            RegistryType::GitHubActions => "github_actions",
            RegistryType::Npm => "npm",
            RegistryType::CratesIo => "crates_io",
            RegistryType::GoProxy => "go_proxy",
            RegistryType::PnpmCatalog => "pnpm_catalog",
            RegistryType::Jsr => "jsr",
            RegistryType::PyPI => "pypi",
        }
    }
}

impl std::str::FromStr for RegistryType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github_actions" => Ok(RegistryType::GitHubActions),
            "npm" => Ok(RegistryType::Npm),
            "crates_io" => Ok(RegistryType::CratesIo),
            "go_proxy" => Ok(RegistryType::GoProxy),
            "pnpm_catalog" => Ok(RegistryType::PnpmCatalog),
            "jsr" => Ok(RegistryType::Jsr),
            "pypi" => Ok(RegistryType::PyPI),
            _ => Err(()),
        }
    }
}

/// Detect the appropriate parser type based on URI
pub fn detect_parser_type(uri: &str) -> Option<RegistryType> {
    if is_github_actions_workflow(uri) {
        Some(RegistryType::GitHubActions)
    } else if uri.ends_with("/package.json") {
        Some(RegistryType::Npm)
    } else if uri.ends_with("/Cargo.toml") {
        Some(RegistryType::CratesIo)
    } else if uri.ends_with("/go.mod") {
        Some(RegistryType::GoProxy)
    } else if uri.ends_with("/pnpm-workspace.yaml") {
        Some(RegistryType::PnpmCatalog)
    } else if uri.ends_with("/deno.json") || uri.ends_with("/deno.jsonc") {
        Some(RegistryType::Jsr)
    } else if uri.ends_with("/pyproject.toml") {
        Some(RegistryType::PyPI)
    } else {
        None
    }
}

fn is_github_actions_workflow(uri: &str) -> bool {
    let is_github_dir = uri.contains(".github/workflows/")
        || uri.contains(".github\\workflows\\")
        || uri.contains(".github/actions/")
        || uri.contains(".github\\actions\\");
    let is_yaml = uri.ends_with(".yml") || uri.ends_with(".yaml");
    is_github_dir && is_yaml
}

/// Registry-specific additional information
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraInfo {
    /// GitHub Actions specific: comment information
    GitHubActions {
        /// Comment text (e.g., "v4.0.0")
        comment_text: String,
        /// Start offset of the comment (position of #)
        comment_start_offset: usize,
        /// End offset of the comment
        comment_end_offset: usize,
    },
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
    /// Registry-specific additional information
    pub extra_info: Option<ExtraInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn extra_info_github_actions_holds_comment_data() {
        let extra = ExtraInfo::GitHubActions {
            comment_text: "v4.1.6".to_string(),
            comment_start_offset: 100,
            comment_end_offset: 108,
        };

        match extra {
            ExtraInfo::GitHubActions {
                comment_text,
                comment_start_offset,
                comment_end_offset,
            } => {
                assert_eq!(comment_text, "v4.1.6");
                assert_eq!(comment_start_offset, 100);
                assert_eq!(comment_end_offset, 108);
            }
        }
    }

    #[test]
    fn package_info_with_extra_info() {
        let info = PackageInfo {
            name: "actions/checkout".to_string(),
            version: "v4.1.6".to_string(),
            commit_hash: Some("8e5e7e5ab8b370d6c329ec480221332ada57f0ab".to_string()),
            registry_type: RegistryType::GitHubActions,
            start_offset: 50,
            end_offset: 90,
            line: 5,
            column: 10,
            extra_info: Some(ExtraInfo::GitHubActions {
                comment_text: "v4.1.6".to_string(),
                comment_start_offset: 92,
                comment_end_offset: 100,
            }),
        };

        assert!(info.extra_info.is_some());
    }

    #[rstest]
    #[case(".github/workflows/ci.yml", Some(RegistryType::GitHubActions))]
    #[case(".github/workflows/release.yaml", Some(RegistryType::GitHubActions))]
    #[case(
        "/home/user/project/.github/workflows/test.yml",
        Some(RegistryType::GitHubActions)
    )]
    #[case(
        "file:///home/user/.github/workflows/build.yml",
        Some(RegistryType::GitHubActions)
    )]
    #[case(".github\\workflows\\ci.yml", Some(RegistryType::GitHubActions))]
    #[case(
        ".github/actions/my-action/action.yml",
        Some(RegistryType::GitHubActions)
    )]
    #[case(
        ".github\\actions\\my-action\\action.yml",
        Some(RegistryType::GitHubActions)
    )]
    #[case("/path/to/package.json", Some(RegistryType::Npm))]
    #[case("/path/to/Cargo.toml", Some(RegistryType::CratesIo))]
    #[case("/path/to/go.mod", Some(RegistryType::GoProxy))]
    #[case("/path/to/pnpm-workspace.yaml", Some(RegistryType::PnpmCatalog))]
    #[case("/project/pnpm-workspace.yaml", Some(RegistryType::PnpmCatalog))]
    #[case(
        "file:///home/user/pnpm-workspace.yaml",
        Some(RegistryType::PnpmCatalog)
    )]
    #[case("/path/to/deno.json", Some(RegistryType::Jsr))]
    #[case("/project/deno.json", Some(RegistryType::Jsr))]
    #[case("file:///home/user/deno.json", Some(RegistryType::Jsr))]
    #[case("/path/to/deno.jsonc", Some(RegistryType::Jsr))]
    #[case("/project/deno.jsonc", Some(RegistryType::Jsr))]
    #[case("file:///home/user/deno.jsonc", Some(RegistryType::Jsr))]
    #[case("/path/to/pyproject.toml", Some(RegistryType::PyPI))]
    #[case("/project/pyproject.toml", Some(RegistryType::PyPI))]
    #[case("file:///home/user/pyproject.toml", Some(RegistryType::PyPI))]
    #[case("workflow.yml", None)]
    #[case("random.txt", None)]
    fn detect_parser_type_returns_expected(
        #[case] uri: &str,
        #[case] expected: Option<RegistryType>,
    ) {
        assert_eq!(detect_parser_type(uri), expected);
    }
}
