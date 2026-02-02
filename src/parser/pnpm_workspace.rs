//! pnpm-workspace.yaml catalog parser

use crate::parser::traits::{ParseError, Parser};
use crate::parser::types::{PackageInfo, RegistryType};
use tracing::warn;

/// Parser for pnpm-workspace.yaml catalog files
pub struct PnpmWorkspaceParser;

impl Parser for PnpmWorkspaceParser {
    fn parse(&self, content: &str) -> Result<Vec<PackageInfo>, ParseError> {
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter_yaml::LANGUAGE;
        parser.set_language(&language.into()).map_err(|e| {
            warn!("Failed to set YAML language for tree-sitter: {}", e);
            ParseError::TreeSitter(e.to_string())
        })?;

        let tree = parser.parse(content, None).ok_or_else(|| {
            warn!("Failed to parse YAML content");
            ParseError::ParseFailed("Failed to parse YAML".to_string())
        })?;

        let root = tree.root_node();
        let mut results = Vec::new();

        // Find catalog or catalogs sections
        self.find_catalog_entries(root, content, &mut results);

        Ok(results)
    }
}

impl PnpmWorkspaceParser {
    /// Find catalog entries in the YAML structure
    ///
    /// Supports two formats:
    /// 1. Single catalog: `catalog:` with direct package entries
    /// 2. Named catalogs: `catalogs:` with nested catalog groups
    fn find_catalog_entries(
        &self,
        node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        if node.kind() == "block_mapping_pair"
            && let Some(key_node) = node.child_by_field_name("key")
        {
            let key = self.get_node_text(key_node, content);

            if key == "catalog" {
                // Single catalog format
                if let Some(value_node) = node.child_by_field_name("value") {
                    self.extract_packages_from_mapping(value_node, content, results);
                }
                return;
            } else if key == "catalogs" {
                // Named catalogs format
                if let Some(value_node) = node.child_by_field_name("value") {
                    self.extract_named_catalogs(value_node, content, results);
                }
                return;
            }
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.find_catalog_entries(child, content, results);
        }
    }

    /// Extract packages from a block_mapping (for single catalog)
    fn extract_packages_from_mapping(
        &self,
        node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block_mapping" {
                self.extract_packages_from_mapping(child, content, results);
            } else if child.kind() == "block_mapping_pair"
                && let Some(info) = self.parse_package_entry(child, content)
            {
                results.push(info);
            }
        }
    }

    /// Extract packages from named catalogs (catalogs:)
    fn extract_named_catalogs(
        &self,
        node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block_mapping" {
                // Each child of block_mapping is a named catalog
                let mut inner_cursor = child.walk();
                for catalog_pair in child.children(&mut inner_cursor) {
                    // The value of each catalog pair contains the packages
                    if catalog_pair.kind() == "block_mapping_pair"
                        && let Some(value_node) = catalog_pair.child_by_field_name("value")
                    {
                        self.extract_packages_from_mapping(value_node, content, results);
                    }
                }
            }
        }
    }

    /// Parse a single package entry (package_name: version)
    fn parse_package_entry(&self, node: tree_sitter::Node, content: &str) -> Option<PackageInfo> {
        let key_node = node.child_by_field_name("key")?;
        let value_node = node.child_by_field_name("value")?;

        let name = self.get_node_text(key_node, content);
        let raw_text = &content[value_node.byte_range()];
        let trimmed = raw_text.trim();

        // Detect and remove quotes
        let (version, has_quotes) = if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
            || (trimmed.starts_with('"') && trimmed.ends_with('"'))
        {
            (&trimmed[1..trimmed.len() - 1], true)
        } else {
            (trimmed, false)
        };

        // Skip empty values
        if version.is_empty() {
            return None;
        }

        let start_offset = value_node.start_byte();
        let end_offset = value_node.end_byte();
        let start_point = value_node.start_position();

        // Adjust offsets for quotes (same approach as package_json.rs)
        let (adjusted_start, adjusted_end, adjusted_column) = if has_quotes {
            (start_offset + 1, end_offset - 1, start_point.column + 1)
        } else {
            (start_offset, end_offset, start_point.column)
        };

        Some(PackageInfo {
            name,
            version: version.to_string(),
            commit_hash: None,
            registry_type: RegistryType::PnpmCatalog,
            start_offset: adjusted_start,
            end_offset: adjusted_end,
            line: start_point.row,
            column: adjusted_column,
            extra_info: None,
        })
    }

    /// Get text content of a node, removing quotes if present
    fn get_node_text(&self, node: tree_sitter::Node, content: &str) -> String {
        let text = &content[node.byte_range()];
        text.trim()
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim_start_matches('\'')
            .trim_end_matches('\'')
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_single_catalog_entries() {
        let parser = PnpmWorkspaceParser;
        let content = r#"catalog:
  react: ^18.2.0
  lodash: ^4.17.21
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "react".to_string(),
                    version: "^18.2.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::PnpmCatalog,
                    start_offset: 18,
                    end_offset: 25,
                    line: 1,
                    column: 9,
                    extra_info: None,
                },
                PackageInfo {
                    name: "lodash".to_string(),
                    version: "^4.17.21".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::PnpmCatalog,
                    start_offset: 36,
                    end_offset: 44,
                    line: 2,
                    column: 10,
                    extra_info: None,
                },
            ]
        );
    }

    #[test]
    fn parse_extracts_named_catalogs_entries() {
        let parser = PnpmWorkspaceParser;
        let content = r#"catalogs:
  react17:
    react: ^17.0.2
    react-dom: ^17.0.2
  react18:
    react: ^18.2.0
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "react".to_string(),
                    version: "^17.0.2".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::PnpmCatalog,
                    start_offset: 32,
                    end_offset: 39,
                    line: 2,
                    column: 11,
                    extra_info: None,
                },
                PackageInfo {
                    name: "react-dom".to_string(),
                    version: "^17.0.2".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::PnpmCatalog,
                    start_offset: 55,
                    end_offset: 62,
                    line: 3,
                    column: 15,
                    extra_info: None,
                },
                PackageInfo {
                    name: "react".to_string(),
                    version: "^18.2.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::PnpmCatalog,
                    start_offset: 85,
                    end_offset: 92,
                    line: 5,
                    column: 11,
                    extra_info: None,
                },
            ]
        );
    }

    #[test]
    fn parse_handles_double_quoted_versions() {
        let parser = PnpmWorkspaceParser;
        let content = r#"catalog:
  typescript: "5.0.0"
"#;
        let result = parser.parse(content).unwrap();
        // Offsets should exclude the quotes
        // content[23..30] = "5.0.0" (with quotes), version starts at 24
        assert_eq!(
            result,
            vec![PackageInfo {
                name: "typescript".to_string(),
                version: "5.0.0".to_string(),
                commit_hash: None,
                registry_type: RegistryType::PnpmCatalog,
                start_offset: 24, // After opening quote
                end_offset: 29,   // Before closing quote
                line: 1,
                column: 15, // After opening quote
                extra_info: None,
            }]
        );
    }

    #[test]
    fn parse_handles_single_quoted_versions() {
        let parser = PnpmWorkspaceParser;
        let content = r#"catalog:
  typescript: '5.0.0'
"#;
        let result = parser.parse(content).unwrap();
        // Offsets should exclude the quotes
        assert_eq!(
            result,
            vec![PackageInfo {
                name: "typescript".to_string(),
                version: "5.0.0".to_string(),
                commit_hash: None,
                registry_type: RegistryType::PnpmCatalog,
                start_offset: 24, // After opening quote
                end_offset: 29,   // Before closing quote
                line: 1,
                column: 15, // After opening quote
                extra_info: None,
            }]
        );
    }

    #[test]
    fn parse_returns_empty_for_no_catalog() {
        let parser = PnpmWorkspaceParser;
        let content = r#"packages:
  - packages/*
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_handles_mixed_catalog_with_other_fields() {
        let parser = PnpmWorkspaceParser;
        let content = r#"packages:
  - packages/*
catalog:
  react: ^18.2.0
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "react");
        assert_eq!(result[0].version, "^18.2.0");
    }
}
