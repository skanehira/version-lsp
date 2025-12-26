//! package.json parser

use crate::parser::traits::{ParseError, Parser};
use crate::parser::types::{PackageInfo, RegistryType};
use tracing::warn;

/// Parser for package.json files
pub struct PackageJsonParser;

impl PackageJsonParser {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PackageJsonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for PackageJsonParser {
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
            self.extract_dependencies(document, content, &mut results);
        }

        Ok(results)
    }
}

impl PackageJsonParser {
    /// Dependency field names to extract
    const DEPENDENCY_FIELDS: [&'static str; 3] =
        ["dependencies", "devDependencies", "peerDependencies"];

    /// Parse npm alias format: npm:package@version or npm:@scope/package@version
    /// Returns (actual_package_name, version)
    fn parse_npm_alias(value: &str) -> Option<(String, String)> {
        let rest = value.strip_prefix("npm:")?;

        // Handle scoped packages: @scope/package@version
        if rest.starts_with('@') {
            // Find the second @ which separates package name from version
            // @scope/package@version -> find @ after the first /
            let slash_pos = rest.find('/')?;
            let after_slash = &rest[slash_pos + 1..];

            if let Some(at_pos) = after_slash.find('@') {
                // Has version: @scope/package@version
                let package_name = &rest[..slash_pos + 1 + at_pos];
                let version = &after_slash[at_pos + 1..];
                Some((package_name.to_string(), version.to_string()))
            } else {
                // No version: @scope/package -> use "latest"
                Some((rest.to_string(), "latest".to_string()))
            }
        } else {
            // Non-scoped package: package@version
            if let Some(at_pos) = rest.find('@') {
                let package_name = &rest[..at_pos];
                let version = &rest[at_pos + 1..];
                Some((package_name.to_string(), version.to_string()))
            } else {
                // No version: package -> use "latest"
                Some((rest.to_string(), "latest".to_string()))
            }
        }
    }

    /// Extract dependencies from the root object
    fn extract_dependencies(
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

            if !Self::DEPENDENCY_FIELDS.contains(&key_text.as_str()) {
                continue;
            }

            let Some(value_node) = child.child_by_field_name("value") else {
                continue;
            };

            if value_node.kind() == "object" {
                self.extract_packages_from_object(value_node, content, results);
            }
        }
    }

    /// Extract packages from a dependency object (e.g., "dependencies": { ... })
    fn extract_packages_from_object(
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

            let Some(value_node) = child.child_by_field_name("value") else {
                continue;
            };

            if value_node.kind() != "string" {
                continue;
            }

            let key_name = self.get_string_value(key_node, content);
            let raw_version = self.get_string_value(value_node, content);

            // Check for npm alias format: npm:package@version
            let (package_name, version) =
                if let Some((name, ver)) = Self::parse_npm_alias(&raw_version) {
                    (name, ver)
                } else {
                    (key_name, raw_version)
                };

            let start_point = value_node.start_position();
            let start_offset = value_node.start_byte();
            let end_offset = value_node.end_byte();

            // Adjust for quotes - the actual version starts after the opening quote
            let version_start_offset = start_offset + 1;
            let version_end_offset = end_offset - 1;
            let version_column = start_point.column + 1;

            results.push(PackageInfo {
                name: package_name,
                version,
                commit_hash: None,
                registry_type: RegistryType::Npm,
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
        // Remove surrounding quotes
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
    fn parse_extracts_dependencies() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "name": "my-app",
  "dependencies": {
    "lodash": "4.17.21"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "lodash".to_string(),
                version: "4.17.21".to_string(),
                commit_hash: None,
                registry_type: RegistryType::Npm,
                start_offset: 57,
                end_offset: 64,
                line: 3,
                column: 15,
                extra_info: None,
            }
        );
    }

    #[test]
    fn parse_extracts_dev_dependencies() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "name": "my-app",
  "devDependencies": {
    "typescript": "5.0.0"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "typescript".to_string(),
                version: "5.0.0".to_string(),
                commit_hash: None,
                registry_type: RegistryType::Npm,
                start_offset: 64,
                end_offset: 69,
                line: 3,
                column: 19,
                extra_info: None,
            }
        );
    }

    #[test]
    fn parse_extracts_peer_dependencies() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "name": "my-lib",
  "peerDependencies": {
    "react": ">=16.8.0"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "react".to_string(),
                version: ">=16.8.0".to_string(),
                commit_hash: None,
                registry_type: RegistryType::Npm,
                start_offset: 60,
                end_offset: 68,
                line: 3,
                column: 14,
                extra_info: None,
            }
        );
    }

    #[test]
    fn parse_extracts_all_dependency_types() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "name": "my-app",
  "dependencies": {
    "lodash": "4.17.21"
  },
  "devDependencies": {
    "typescript": "5.0.0"
  },
  "peerDependencies": {
    "react": "18.0.0"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "lodash".to_string(),
                    version: "4.17.21".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Npm,
                    start_offset: 57,
                    end_offset: 64,
                    line: 3,
                    column: 15,
                    extra_info: None,
                },
                PackageInfo {
                    name: "typescript".to_string(),
                    version: "5.0.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Npm,
                    start_offset: 113,
                    end_offset: 118,
                    line: 6,
                    column: 19,
                    extra_info: None,
                },
                PackageInfo {
                    name: "react".to_string(),
                    version: "18.0.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Npm,
                    start_offset: 163,
                    end_offset: 169,
                    line: 9,
                    column: 14,
                    extra_info: None,
                },
            ]
        );
    }

    #[test]
    fn parse_handles_version_ranges() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "name": "my-app",
  "dependencies": {
    "lodash": "^4.17.21",
    "express": "~4.18.0",
    "uuid": ">=9.0.0"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "lodash".to_string(),
                    version: "^4.17.21".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Npm,
                    start_offset: 57,
                    end_offset: 65,
                    line: 3,
                    column: 15,
                    extra_info: None,
                },
                PackageInfo {
                    name: "express".to_string(),
                    version: "~4.18.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Npm,
                    start_offset: 84,
                    end_offset: 91,
                    line: 4,
                    column: 16,
                    extra_info: None,
                },
                PackageInfo {
                    name: "uuid".to_string(),
                    version: ">=9.0.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Npm,
                    start_offset: 107,
                    end_offset: 114,
                    line: 5,
                    column: 13,
                    extra_info: None,
                },
            ]
        );
    }

    #[test]
    fn parse_handles_scoped_packages() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "name": "my-app",
  "dependencies": {
    "@types/node": "20.0.0",
    "@babel/core": "7.22.0"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "@types/node".to_string(),
                    version: "20.0.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Npm,
                    start_offset: 62,
                    end_offset: 68,
                    line: 3,
                    column: 20,
                    extra_info: None,
                },
                PackageInfo {
                    name: "@babel/core".to_string(),
                    version: "7.22.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::Npm,
                    start_offset: 91,
                    end_offset: 97,
                    line: 4,
                    column: 20,
                    extra_info: None,
                },
            ]
        );
    }

    #[test]
    fn parse_returns_empty_for_no_dependencies() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "name": "my-app",
  "version": "1.0.0"
}"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_calculates_correct_position() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "dependencies": {
    "lodash": "4.17.21"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "lodash".to_string(),
                version: "4.17.21".to_string(),
                commit_hash: None,
                registry_type: RegistryType::Npm,
                start_offset: 37,
                end_offset: 44,
                line: 2,
                column: 15,
                extra_info: None,
            }
        );
    }

    #[test]
    fn parse_extracts_npm_alias() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "dependencies": {
    "vite": "npm:rolldown-vite@7.2.2"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        // npm alias should be normalized: name becomes "rolldown-vite", version becomes "7.2.2"
        assert_eq!(result[0].name, "rolldown-vite");
        assert_eq!(result[0].version, "7.2.2");
    }

    #[test]
    fn parse_extracts_npm_alias_with_scope() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "dependencies": {
    "my-fork": "npm:@org/pkg@^1.0.0"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        // npm alias with scoped package: name becomes "@org/pkg", version becomes "^1.0.0"
        assert_eq!(result[0].name, "@org/pkg");
        assert_eq!(result[0].version, "^1.0.0");
    }

    #[test]
    fn parse_extracts_npm_alias_without_version() {
        let parser = PackageJsonParser::new();
        let content = r#"{
  "dependencies": {
    "npa": "npm:npm-package-arg"
  }
}"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        // npm alias without version: name becomes "npm-package-arg", version is empty or "latest"
        assert_eq!(result[0].name, "npm-package-arg");
        assert_eq!(result[0].version, "latest");
    }
}
