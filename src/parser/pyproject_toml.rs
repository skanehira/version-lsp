//! pyproject.toml parser for Python dependencies (PEP 508/PEP 440)
//!
//! Supports the following sections:
//! - `[project].dependencies` - Main project dependencies
//! - `[build-system].requires` - Build system requirements
//! - `[project.optional-dependencies]` - Optional dependencies
//!
//! URL dependencies (e.g., `pkg @ git+https://...`) are skipped
//! as they don't exist on PyPI.

use std::str::FromStr;

use pep508_rs::{Requirement, VerbatimUrl, VersionOrUrl};
use tracing::warn;

use crate::parser::traits::{ParseError, Parser};
use crate::parser::types::{PackageInfo, RegistryType};

/// Parser for pyproject.toml files
pub struct PyprojectTomlParser;

impl PyprojectTomlParser {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PyprojectTomlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for PyprojectTomlParser {
    fn parse(&self, content: &str) -> Result<Vec<PackageInfo>, ParseError> {
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter_toml_ng::LANGUAGE;
        parser.set_language(&language.into()).map_err(|e| {
            warn!("Failed to set TOML language for tree-sitter: {}", e);
            ParseError::TreeSitter(e.to_string())
        })?;

        let tree = parser.parse(content, None).ok_or_else(|| {
            warn!("Failed to parse TOML content");
            ParseError::ParseFailed("Failed to parse TOML".to_string())
        })?;

        let root = tree.root_node();
        let mut results = Vec::new();

        self.extract_dependencies(root, content, &mut results);

        Ok(results)
    }
}

impl PyprojectTomlParser {
    /// Extract dependencies from all dependency sections
    fn extract_dependencies(
        &self,
        root: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "table" {
                self.process_table(child, content, results);
            }
        }
    }

    /// Process a TOML table node
    fn process_table(
        &self,
        table_node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        // Get the table header
        let Some(header) = table_node.child(0) else {
            return;
        };

        if header.kind() != "[" {
            return;
        }

        // Find the table name
        let mut cursor = table_node.walk();
        let mut table_name: Option<String> = None;

        for child in table_node.children(&mut cursor) {
            if child.kind() == "bare_key" || child.kind() == "dotted_key" {
                table_name = Some(content[child.byte_range()].to_string());
                break;
            }
        }

        let Some(name) = table_name else {
            return;
        };

        // Handle each table type differently
        match name.as_str() {
            "project" => {
                // [project] - look for "dependencies" key
                self.extract_key_array(table_node, content, "dependencies", results);
            }
            "build-system" => {
                // [build-system] - look for "requires" key
                self.extract_key_array(table_node, content, "requires", results);
            }
            "project.optional-dependencies" => {
                // [project.optional-dependencies] - all keys have arrays
                self.extract_all_arrays(table_node, content, results);
            }
            _ => {}
        }
    }

    /// Extract dependencies from a specific key's array value
    fn extract_key_array(
        &self,
        table_node: tree_sitter::Node,
        content: &str,
        key_name: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = table_node.walk();

        for child in table_node.children(&mut cursor) {
            if child.kind() == "pair" {
                let mut pair_cursor = child.walk();
                let mut is_target_key = false;

                for pair_child in child.children(&mut pair_cursor) {
                    match pair_child.kind() {
                        "bare_key" => {
                            let key = &content[pair_child.byte_range()];
                            is_target_key = key == key_name;
                        }
                        "array" if is_target_key => {
                            self.extract_from_array(pair_child, content, results);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Extract dependencies from all array values in the table
    fn extract_all_arrays(
        &self,
        table_node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = table_node.walk();

        for child in table_node.children(&mut cursor) {
            if child.kind() == "pair" {
                self.extract_from_pair(child, content, results);
            }
        }
    }

    /// Extract dependencies from an array of dependency strings
    fn extract_from_array(
        &self,
        array_node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = array_node.walk();

        for child in array_node.children(&mut cursor) {
            if child.kind() == "string" {
                let text = &content[child.byte_range()];
                // Remove only the outer quotes from TOML string
                // TOML strings are either "..." or '...' (literal string)
                let trimmed = text.trim();
                let dep_str = if (trimmed.starts_with('"') && trimmed.ends_with('"'))
                    || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
                {
                    &trimmed[1..trimmed.len() - 1]
                } else {
                    trimmed
                };

                if let Some(info) = self.parse_dependency_string(dep_str, child, content) {
                    results.push(info);
                }
            }
        }
    }

    /// Extract from key = array pair
    fn extract_from_pair(
        &self,
        pair_node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = pair_node.walk();

        for child in pair_node.children(&mut cursor) {
            if child.kind() == "array" {
                self.extract_from_array(child, content, results);
            }
        }
    }

    /// Parse a PEP 508 dependency string and extract PackageInfo
    fn parse_dependency_string(
        &self,
        dep_str: &str,
        string_node: tree_sitter::Node,
        content: &str,
    ) -> Option<PackageInfo> {
        // Parse with pep508_rs
        let req = Requirement::<VerbatimUrl>::from_str(dep_str)
            .inspect_err(|e| warn!("Failed to parse dependency '{}': {}", dep_str, e))
            .ok()?;

        // Skip URL dependencies (e.g., pkg @ git+https://...)
        let version_spec = match &req.version_or_url {
            Some(VersionOrUrl::Url(_)) => return None,
            Some(VersionOrUrl::VersionSpecifier(specs)) => specs.to_string(),
            None => String::new(),
        };

        // Calculate the position within the string for the version specifier
        let string_start = string_node.start_byte();
        let string_text = &content[string_node.byte_range()];

        // Find where the version spec starts in the original string
        // The format is: "package_name[extras]version_spec; markers"
        let package_name = req.name.to_string();

        // Find the version specifier position in the string
        let (start_offset, end_offset) = if version_spec.is_empty() {
            // No version spec - use the whole string range
            (string_start + 1, string_node.end_byte() - 1)
        } else {
            // Find version spec in the string
            let inner = string_text
                .trim_start_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('"')
                .trim_end_matches('\'');

            // Find the start of version specifier (first operator character after package name)
            let version_ops = [">=", "<=", "!=", "~=", "==", ">", "<"];
            let mut version_start_in_inner = inner.len();

            for op in version_ops {
                if let Some(pos) = inner.find(op)
                    && pos < version_start_in_inner
                {
                    version_start_in_inner = pos;
                }
            }

            if version_start_in_inner >= inner.len() {
                // No version spec found, use package name range
                (string_start + 1, string_start + 1 + package_name.len())
            } else {
                // Calculate positions - account for the opening quote
                let quote_offset = 1; // Opening quote
                let start = string_start + quote_offset + version_start_in_inner;

                // Find the end of version spec (before ; or end of string)
                let version_end_in_inner = inner.find(';').unwrap_or(inner.len());
                let end = string_start + quote_offset + version_end_in_inner;

                (start, end)
            }
        };

        let start_point = string_node.start_position();
        // Calculate column offset for version spec
        let version_column_offset = start_offset - string_start;

        Some(PackageInfo {
            name: package_name,
            version: version_spec,
            commit_hash: None,
            registry_type: RegistryType::PyPI,
            start_offset,
            end_offset,
            line: start_point.row,
            column: start_point.column + version_column_offset,
            extra_info: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_project_dependencies() {
        let parser = PyprojectTomlParser::new();
        // Standard pyproject.toml format: dependencies array in [project] section
        let content = r#"[project]
name = "my-app"
version = "0.1.0"
dependencies = [
    "requests>=2.28.0",
    "flask>=2.0.0",
]
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "requests");
        assert_eq!(result[0].version, ">=2.28.0");
        assert_eq!(result[0].registry_type, RegistryType::PyPI);
        assert_eq!(result[1].name, "flask");
        assert_eq!(result[1].version, ">=2.0.0");
    }

    #[test]
    fn parse_extracts_build_system_requires() {
        let parser = PyprojectTomlParser::new();
        let content = r#"[build-system]
requires = [
    "setuptools>=61.0",
    "wheel",
]
build-backend = "setuptools.build_meta"
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "setuptools");
        assert_eq!(result[0].version, ">=61.0");
        assert_eq!(result[1].name, "wheel");
        assert_eq!(result[1].version, "");
    }

    #[test]
    fn parse_extracts_optional_dependencies() {
        let parser = PyprojectTomlParser::new();
        let content = r#"[project.optional-dependencies]
dev = [
    "pytest>=7.0",
    "black>=23.0",
]
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "pytest");
        assert_eq!(result[0].version, ">=7.0");
        assert_eq!(result[1].name, "black");
        assert_eq!(result[1].version, ">=23.0");
    }

    #[test]
    fn parse_handles_complex_version_specifiers() {
        let parser = PyprojectTomlParser::new();
        let content = r#"[project]
dependencies = [
    "django>=3.2,<4.0",
    "numpy~=1.21.0",
    "pandas==2.0.0",
]
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "django");
        assert_eq!(result[0].version, ">=3.2, <4.0");
        assert_eq!(result[1].name, "numpy");
        assert_eq!(result[1].version, "~=1.21.0");
        assert_eq!(result[2].name, "pandas");
        assert_eq!(result[2].version, "==2.0.0");
    }

    #[test]
    fn parse_handles_extras() {
        let parser = PyprojectTomlParser::new();
        let content = r#"[project]
dependencies = [
    "flask[async]>=2.0",
    "requests[security]>=2.28.0",
]
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "flask");
        assert_eq!(result[0].version, ">=2.0");
        assert_eq!(result[1].name, "requests");
        assert_eq!(result[1].version, ">=2.28.0");
    }

    #[test]
    fn parse_handles_environment_markers() {
        let parser = PyprojectTomlParser::new();
        // PEP 508 environment markers with single quotes (valid in TOML double-quoted strings)
        let content = "[project]\ndependencies = [\n    \"pywin32>=300; sys_platform == 'win32'\",\n    \"pandas>=2.0; python_version >= '3.9'\",\n]\n";
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "pywin32");
        assert_eq!(result[0].version, ">=300");
        assert_eq!(result[1].name, "pandas");
        assert_eq!(result[1].version, ">=2.0");
    }

    #[test]
    fn parse_skips_url_dependencies() {
        let parser = PyprojectTomlParser::new();
        let content = r#"[project]
dependencies = [
    "requests>=2.28.0",
    "my-package @ git+https://github.com/user/repo.git",
    "flask>=2.0",
]
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "requests");
        assert_eq!(result[1].name, "flask");
    }

    #[test]
    fn parse_returns_empty_for_no_dependencies() {
        let parser = PyprojectTomlParser::new();
        let content = r#"[project]
name = "my-app"
version = "0.1.0"
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_handles_package_without_version() {
        let parser = PyprojectTomlParser::new();
        let content = r#"[project]
dependencies = [
    "requests",
    "flask",
]
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "requests");
        assert_eq!(result[0].version, "");
        assert_eq!(result[1].name, "flask");
        assert_eq!(result[1].version, "");
    }

    #[test]
    fn parse_extracts_named_optional_dependencies_subsection() {
        let parser = PyprojectTomlParser::new();
        let content = r#"[project.optional-dependencies]
test = [
    "pytest>=7.0",
]
docs = [
    "sphinx>=5.0",
]
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "pytest");
        assert_eq!(result[1].name, "sphinx");
    }
}
