//! deno.json parser

use crate::parser::traits::{ParseError, Parser};
use crate::parser::types::{PackageInfo, RegistryType};
use tracing::warn;

/// Parser for deno.json files
pub struct DenoJsonParser;

impl DenoJsonParser {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DenoJsonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for DenoJsonParser {
    fn parse(&self, content: &str) -> Result<Vec<PackageInfo>, ParseError> {
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter_json::LANGUAGE;
        parser.set_language(&language.into()).map_err(|e| {
            warn!("Failed to set JSON language for tree-sitter: {}", e);
            ParseError::TreeSitter(e.to_string())
        })?;

        let tree = parser.parse(content, None).ok_or_else(|| {
            warn!("Failed to parse JSON content");
            ParseError::ParseFailed("Failed to parse JSON".to_string())
        })?;

        let root = tree.root_node();
        let mut results = Vec::new();

        // Find the root object
        if let Some(document) = root.child(0)
            && document.kind() == "object"
        {
            self.extract_imports(document, content, &mut results);
        }

        Ok(results)
    }
}

impl DenoJsonParser {
    /// Parse JSR specifier format: jsr:@scope/package@version
    /// Returns (package_name, version)
    fn parse_jsr_specifier(value: &str) -> Option<(String, String)> {
        let rest = value.strip_prefix("jsr:")?;

        // @scope/package@version format
        // Find the @ that separates package from version (after the scope's @)
        let slash_pos = rest.find('/')?;
        let after_slash = &rest[slash_pos + 1..];

        if let Some(at_pos) = after_slash.find('@') {
            let package_name = &rest[..slash_pos + 1 + at_pos];
            let version = &after_slash[at_pos + 1..];
            Some((package_name.to_string(), version.to_string()))
        } else {
            // No version specified
            Some((rest.to_string(), "latest".to_string()))
        }
    }

    /// Extract imports from the root object
    fn extract_imports(
        &self,
        object_node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = object_node.walk();

        for child in object_node.children(&mut cursor) {
            if child.kind() != "pair" {
                continue;
            }

            let Some(key_node) = child.child_by_field_name("key") else {
                continue;
            };

            let key_text = self.get_string_value(key_node, content);

            if key_text != "imports" {
                continue;
            }

            let Some(value_node) = child.child_by_field_name("value") else {
                continue;
            };

            if value_node.kind() == "object" {
                self.extract_packages_from_imports(value_node, content, results);
            }
        }
    }

    /// Extract packages from the imports object
    fn extract_packages_from_imports(
        &self,
        object_node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        let mut cursor = object_node.walk();

        for child in object_node.children(&mut cursor) {
            if child.kind() != "pair" {
                continue;
            }

            let Some(value_node) = child.child_by_field_name("value") else {
                continue;
            };

            if value_node.kind() != "string" {
                continue;
            }

            let raw_value = self.get_string_value(value_node, content);

            // Only process jsr: prefixed entries
            let Some((package_name, version)) = Self::parse_jsr_specifier(&raw_value) else {
                continue;
            };

            let start_point = value_node.start_position();
            let start_offset = value_node.start_byte();
            let end_offset = value_node.end_byte();

            // Version position: start after "jsr:@scope/package@" to get just the version
            // For now, we'll use the full value node position
            let version_start_offset = start_offset + 1; // Skip opening quote
            let version_end_offset = end_offset - 1; // Skip closing quote
            let version_column = start_point.column + 1; // Skip opening quote

            results.push(PackageInfo {
                name: package_name,
                version,
                commit_hash: None,
                registry_type: RegistryType::Jsr,
                start_offset: version_start_offset,
                end_offset: version_end_offset,
                line: start_point.row,
                column: version_column,
                extra_info: None,
            });
        }
    }

    /// Get the string value from a string node (removes quotes)
    fn get_string_value(&self, node: tree_sitter::Node, content: &str) -> String {
        let text = &content[node.byte_range()];
        text.trim()
            .trim_start_matches('"')
            .trim_end_matches('"')
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_jsr_package() {
        let parser = DenoJsonParser::new();
        let content = r#"{
  "imports": {
    "@luca/flag": "jsr:@luca/flag@^1.0.1"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "@luca/flag".to_string(),
                version: "^1.0.1".to_string(),
                commit_hash: None,
                registry_type: RegistryType::Jsr,
                start_offset: 36,
                end_offset: 57,
                line: 2,
                column: 19,
                extra_info: None,
            }
        );
    }

    #[test]
    fn parse_extracts_multiple_jsr_packages() {
        let parser = DenoJsonParser::new();
        let content = r#"{
  "imports": {
    "@luca/flag": "jsr:@luca/flag@^1.0.1",
    "@std/path": "jsr:@std/path@1.0.0"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "@luca/flag".to_string(),
                    version: "^1.0.1".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Jsr,
                    start_offset: 36,
                    end_offset: 57,
                    line: 2,
                    column: 19,
                    extra_info: None,
                },
                PackageInfo {
                    name: "@std/path".to_string(),
                    version: "1.0.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Jsr,
                    start_offset: 78,
                    end_offset: 97,
                    line: 3,
                    column: 18,
                    extra_info: None,
                },
            ]
        );
    }

    #[test]
    fn parse_skips_non_jsr_entries() {
        let parser = DenoJsonParser::new();
        let content = r#"{
  "imports": {
    "@luca/flag": "jsr:@luca/flag@^1.0.1",
    "lodash": "https://esm.sh/lodash@4.17.21"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![PackageInfo {
                name: "@luca/flag".to_string(),
                version: "^1.0.1".to_string(),
                commit_hash: None,
                registry_type: RegistryType::Jsr,
                start_offset: 36,
                end_offset: 57,
                line: 2,
                column: 19,
                extra_info: None,
            }]
        );
    }

    #[test]
    fn parse_handles_jsr_without_version() {
        let parser = DenoJsonParser::new();
        let content = r#"{
  "imports": {
    "@std/path": "jsr:@std/path"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![PackageInfo {
                name: "@std/path".to_string(),
                version: "latest".to_string(),
                commit_hash: None,
                registry_type: RegistryType::Jsr,
                start_offset: 35,
                end_offset: 48,
                line: 2,
                column: 18,
                extra_info: None,
            }]
        );
    }

    #[test]
    fn parse_returns_empty_for_no_imports() {
        let parser = DenoJsonParser::new();
        let content = r#"{
  "name": "my-app"
}"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_returns_empty_for_empty_imports() {
        let parser = DenoJsonParser::new();
        let content = r#"{
  "imports": {}
}"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }
}
