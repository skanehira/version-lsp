//! Code action generation for version bumping

mod upgrade;

pub use upgrade::{generate_bump_code_actions, generate_bump_code_actions_with_sha};

use crate::parser::types::PackageInfo;
use std::collections::HashMap;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Position, Range, TextEdit, Url, WorkspaceEdit,
};

/// Index of packages grouped by line number for efficient lookup
pub struct PackageIndex<'a> {
    by_line: HashMap<u32, Vec<&'a PackageInfo>>,
}

impl<'a> PackageIndex<'a> {
    /// Build an index from a slice of packages
    pub fn new(packages: &'a [PackageInfo]) -> Self {
        let mut by_line: HashMap<u32, Vec<&'a PackageInfo>> = HashMap::new();
        for pkg in packages {
            by_line.entry(pkg.line as u32).or_default().push(pkg);
        }
        Self { by_line }
    }

    /// Find the package at the given cursor position
    ///
    /// Returns the package if the cursor is within the version range of a package.
    pub fn find_at_position(&self, position: Position) -> Option<&'a PackageInfo> {
        let packages_on_line = self.by_line.get(&position.line)?;

        packages_on_line.iter().find_map(|&pkg| {
            let start_col = pkg.column as u32;
            let end_col = start_col + pkg.version.len() as u32;

            if position.character >= start_col && position.character < end_col {
                Some(pkg)
            } else {
                None
            }
        })
    }
}

/// Extract version prefix (^, ~, >=, <=, >, <, =, v) from a version string
fn extract_version_prefix(version: &str) -> &str {
    if version.starts_with(">=") {
        ">="
    } else if version.starts_with("<=") {
        "<="
    } else if version.starts_with('>') {
        ">"
    } else if version.starts_with('<') {
        "<"
    } else if version.starts_with('=') {
        "="
    } else if version.starts_with('^') {
        "^"
    } else if version.starts_with('~') {
        "~"
    } else if version.starts_with('v') {
        "v"
    } else {
        ""
    }
}

fn create_bump_action(
    title: &str,
    new_version: &str,
    package: &PackageInfo,
    uri: &Url,
) -> CodeAction {
    let start = Position {
        line: package.line as u32,
        character: package.column as u32,
    };
    let end = Position {
        line: package.line as u32,
        character: package.column as u32 + package.version.len() as u32,
    };

    let text_edit = TextEdit {
        range: Range { start, end },
        new_text: new_version.to_string(),
    };

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![text_edit]);

    CodeAction {
        title: title.to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::types::RegistryType;
    use rstest::rstest;

    fn make_package(name: &str, version: &str, line: u32, column: u32, len: usize) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            commit_hash: None,
            registry_type: RegistryType::Npm,
            start_offset: 0,
            end_offset: len,
            line: line as usize,
            column: column as usize,
            extra_info: None,
        }
    }

    #[rstest]
    #[case(
        Position { line: 3, character: 17 }, // cursor within version range
        vec![make_package("lodash", "4.17.21", 3, 15, 7)],
        Some("lodash")
    )]
    #[case(
        Position { line: 3, character: 10 }, // cursor before version range
        vec![make_package("lodash", "4.17.21", 3, 15, 7)],
        None
    )]
    #[case(
        Position { line: 3, character: 25 }, // cursor after version range
        vec![make_package("lodash", "4.17.21", 3, 15, 7)],
        None
    )]
    #[case(
        Position { line: 2, character: 17 }, // wrong line
        vec![make_package("lodash", "4.17.21", 3, 15, 7)],
        None
    )]
    #[case(
        Position { line: 5, character: 12 }, // multiple packages, cursor on second
        vec![
            make_package("lodash", "4.17.21", 3, 15, 7),
            make_package("react", "18.2.0", 5, 10, 6),
        ],
        Some("react")
    )]
    fn test_package_index_find_at_position(
        #[case] position: Position,
        #[case] packages: Vec<PackageInfo>,
        #[case] expected_name: Option<&str>,
    ) {
        let index = PackageIndex::new(&packages);
        let result = index.find_at_position(position);
        assert_eq!(result.map(|p| p.name.as_str()), expected_name);
    }
}
