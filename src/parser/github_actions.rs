//! GitHub Actions workflow file parser

use crate::parser::traits::{ParseError, Parser};
use crate::parser::types::{PackageInfo, RegistryType};
use tracing::warn;

/// Parser for GitHub Actions workflow files (.github/workflows/*.yml)
pub struct GitHubActionsParser;

impl GitHubActionsParser {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GitHubActionsParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for GitHubActionsParser {
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

        // Find all 'uses' keys in the YAML
        self.find_uses_nodes(root, content, &mut results);

        Ok(results)
    }
}

impl GitHubActionsParser {
    /// Find all 'steps' blocks and extract 'uses' values from them
    ///
    /// YAML tree structure for GitHub Actions workflow:
    /// ```text
    /// stream
    ///   document
    ///     block_node
    ///       block_mapping
    ///         block_mapping_pair          <- "steps: ..."
    ///           flow_node                 <- key: "steps"
    ///           block_node
    ///             block_sequence          <- list of steps
    ///               block_sequence_item   <- "- uses: ..."
    ///                 block_node
    ///                   block_mapping
    ///                     block_mapping_pair    <- TARGET: "uses: actions/checkout@v4"
    ///                       flow_node           <- key: "uses"
    ///                       flow_node           <- value: "actions/checkout@v4"
    /// ```
    fn find_uses_nodes(
        &self,
        node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        // Look for "steps" key and only extract uses from within steps
        if node.kind() == "block_mapping_pair"
            && let Some(key_node) = node.child_by_field_name("key")
            && self.get_node_text(key_node, content) == "steps"
            && let Some(value_node) = node.child_by_field_name("value")
        {
            // Found a "steps" block, extract uses from it
            self.find_uses_in_steps(value_node, content, results);
            return;
        }

        // Recurse into children to find "steps" blocks
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.find_uses_nodes(child, content, results);
        }
    }

    /// Extract 'uses' values from within a steps block
    fn find_uses_in_steps(
        &self,
        node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        // Check if this is a block_mapping_pair with key "uses"
        if node.kind() == "block_mapping_pair"
            && let Some(key_node) = node.child_by_field_name("key")
            && self.get_node_text(key_node, content) == "uses"
            && let Some(value_node) = node.child_by_field_name("value")
        {
            let value_text = self.get_node_text(value_node, content);
            if let Some(info) = self.parse_uses_value(&value_text, value_node, content) {
                results.push(info);
            }
        }

        // Recurse into children within steps
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.find_uses_in_steps(child, content, results);
        }
    }

    /// Get text content of a node, removing quotes if present
    fn get_node_text(&self, node: tree_sitter::Node, content: &str) -> String {
        let text = &content[node.byte_range()];
        // Remove surrounding quotes if present
        text.trim()
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim_start_matches('\'')
            .trim_end_matches('\'')
            .to_string()
    }

    /// Parse a 'uses' value into PackageInfo
    ///
    /// # Arguments
    /// * `value` - The uses value text with quotes removed
    ///   - `"actions/checkout@v4"`
    ///   - `"actions/checkout@8e5e7e5ab8b370d6c329ec480221332ada57f0ab"`
    ///   - `"actions/aws/ec2@v1"`
    /// * `node` - The tree-sitter node for the value (flow_node containing the uses value)
    /// * `content` - The original YAML content for position calculation
    ///
    /// # Returns
    /// * `Some(PackageInfo)` - Parsed package info with name="owner/repo" and version
    /// * `None` - If the value doesn't match expected format
    fn parse_uses_value(
        &self,
        value: &str,
        node: tree_sitter::Node,
        content: &str,
    ) -> Option<PackageInfo> {
        // Parse: owner/repo@version or owner/repo/subdir@version
        let at_pos = value.find('@')?;
        let (repo_part, version) = value.split_at(at_pos);
        let version = &version[1..]; // Skip '@'

        // Parse owner/repo (ignore subdirectories like actions/aws/ec2)
        let parts: Vec<&str> = repo_part.split('/').collect();
        if parts.len() < 2 {
            return None;
        }

        let owner = parts[0];
        let repo = parts[1];
        let name = format!("{}/{}", owner, repo);

        // Calculate position info
        let start_offset = node.start_byte();
        let end_offset = node.end_byte();
        let start_point = node.start_position();

        // Find the version position within the value
        let value_text = self.get_node_text(node, content);
        let version_start_in_value = value_text.find('@').map(|p| p + 1).unwrap_or(0);
        let version_column = start_point.column + version_start_in_value;

        // Check if the ref is a commit hash (40 hex characters)
        let is_hash = version.len() == 40 && version.chars().all(|c| c.is_ascii_hexdigit());

        let (final_version, commit_hash) = if is_hash {
            // Try to extract version from comment in the line
            let line_start = content[..start_offset].rfind('\n').map_or(0, |p| p + 1);
            let line_end = content[start_offset..]
                .find('\n')
                .map_or(content.len(), |p| start_offset + p);
            let line_text = &content[line_start..line_end];

            // Look for # comment with version
            let comment_version = line_text
                .find('#')
                .map(|p| line_text[p + 1..].trim().to_string())
                .filter(|v| !v.is_empty());

            (
                comment_version.unwrap_or_else(|| version.to_string()),
                Some(version.to_string()),
            )
        } else {
            (version.to_string(), None)
        };

        Some(PackageInfo {
            name,
            version: final_version,
            commit_hash,
            registry_type: RegistryType::GitHubActions,
            start_offset: start_offset + version_start_in_value,
            end_offset,
            line: start_point.row,
            column: version_column,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_action_with_version_tag() {
        let parser = GitHubActionsParser::new();
        let content = r#"name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "actions/checkout".to_string(),
                version: "v4".to_string(),
                commit_hash: None,
                registry_type: RegistryType::GitHubActions,
                start_offset: 102,
                end_offset: 104,
                line: 6,
                column: 31,
            }
        );
    }

    #[test]
    fn parse_extracts_multiple_actions() {
        let parser = GitHubActionsParser::new();
        let content = r#"name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
      - uses: actions/cache@v3
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "actions/checkout".to_string(),
                    version: "v4".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 102,
                    end_offset: 104,
                    line: 6,
                    column: 31,
                },
                PackageInfo {
                    name: "actions/setup-node".to_string(),
                    version: "v4".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 138,
                    end_offset: 140,
                    line: 7,
                    column: 33,
                },
                PackageInfo {
                    name: "actions/cache".to_string(),
                    version: "v3".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 169,
                    end_offset: 171,
                    line: 8,
                    column: 28,
                },
            ]
        );
    }

    #[test]
    fn parse_extracts_action_with_commit_hash() {
        let parser = GitHubActionsParser::new();
        let content = r#"name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@8e5e7e5ab8b370d6c329ec480221332ada57f0ab
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "actions/checkout".to_string(),
                version: "8e5e7e5ab8b370d6c329ec480221332ada57f0ab".to_string(),
                commit_hash: Some("8e5e7e5ab8b370d6c329ec480221332ada57f0ab".to_string()),
                registry_type: RegistryType::GitHubActions,
                start_offset: 102,
                end_offset: 142,
                line: 6,
                column: 31,
            }
        );
    }

    #[test]
    fn parse_extracts_action_with_subdirectory() {
        let parser = GitHubActionsParser::new();
        let content = r#"name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/aws/ec2@v1
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "actions/aws".to_string(),
                version: "v1".to_string(),
                commit_hash: None,
                registry_type: RegistryType::GitHubActions,
                start_offset: 101,
                end_offset: 103,
                line: 6,
                column: 30,
            }
        );
    }

    #[test]
    fn parse_handles_quoted_uses() {
        let parser = GitHubActionsParser::new();
        let content = r#"name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: "actions/checkout@v4"
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "actions/checkout".to_string(),
                version: "v4".to_string(),
                commit_hash: None,
                registry_type: RegistryType::GitHubActions,
                start_offset: 102,
                end_offset: 106,
                line: 6,
                column: 31,
            }
        );
    }

    #[test]
    fn parse_returns_empty_for_no_steps() {
        let parser = GitHubActionsParser::new();
        let content = r#"name: CI
on: push
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_extracts_semantic_version_formats() {
        let parser = GitHubActionsParser::new();
        let content = r#"jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3.5.0
      - uses: actions/setup-go@v4.1.0
      - uses: actions/cache@main
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "actions/checkout".to_string(),
                    version: "v3.5.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 83,
                    end_offset: 89,
                    line: 4,
                    column: 31,
                },
                PackageInfo {
                    name: "actions/setup-go".to_string(),
                    version: "v4.1.0".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 121,
                    end_offset: 127,
                    line: 5,
                    column: 31,
                },
                PackageInfo {
                    name: "actions/cache".to_string(),
                    version: "main".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 156,
                    end_offset: 160,
                    line: 6,
                    column: 28,
                },
            ]
        );
    }

    #[test]
    fn parse_handles_trailing_comments() {
        let parser = GitHubActionsParser::new();
        let content = r#"jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3  # latest stable
      - uses: actions/setup-node@v4  # Node.js setup
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "actions/checkout".to_string(),
                    version: "v3".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 83,
                    end_offset: 85,
                    line: 4,
                    column: 31,
                },
                PackageInfo {
                    name: "actions/setup-node".to_string(),
                    version: "v4".to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 136,
                    end_offset: 138,
                    line: 5,
                    column: 33,
                },
            ]
        );
    }

    #[test]
    fn parse_ignores_uses_outside_of_steps() {
        let parser = GitHubActionsParser::new();
        let content = r#"name: Test Workflow
on:
  workflow_call:
jobs:
  reusable:
    uses: org/repo/.github/workflows/reusable.yml@main
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/setup-node@v4
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "actions/setup-node".to_string(),
                version: "v4".to_string(),
                commit_hash: None,
                registry_type: RegistryType::GitHubActions,
                start_offset: 193,
                end_offset: 195,
                line: 9,
                column: 33,
            }
        );
    }

    #[test]
    fn parse_returns_empty_for_buffer_without_actions() {
        let parser = GitHubActionsParser::new();
        let content = r#"name: Test Workflow
on:
  push:
    branches: [main]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - name: Echo
        run: echo "hello"
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_extracts_hash_with_version_comment() {
        let parser = GitHubActionsParser::new();
        let content = r#"jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: taiki-e/install-action@e30c5b8cfc4910a9f163907c8149ac1e54f1ab11 # v2.62.25
      - uses: actions/checkout@a5ac7e51b41094c92402da3b24376905380afc29 # v4.1.6
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(
            result,
            vec![
                PackageInfo {
                    name: "taiki-e/install-action".to_string(),
                    version: "v2.62.25".to_string(),
                    commit_hash: Some("e30c5b8cfc4910a9f163907c8149ac1e54f1ab11".to_string()),
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 89,
                    end_offset: 129,
                    line: 4,
                    column: 37,
                },
                PackageInfo {
                    name: "actions/checkout".to_string(),
                    version: "v4.1.6".to_string(),
                    commit_hash: Some("a5ac7e51b41094c92402da3b24376905380afc29".to_string()),
                    registry_type: RegistryType::GitHubActions,
                    start_offset: 172,
                    end_offset: 212,
                    line: 5,
                    column: 31,
                },
            ]
        );
    }

    #[test]
    fn parse_extracts_hash_only_without_version_comment() {
        let parser = GitHubActionsParser::new();
        let content = r#"jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@8e5e7e5ab8b370d6c329ec480221332ada57f0ab
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "actions/checkout".to_string(),
                version: "8e5e7e5ab8b370d6c329ec480221332ada57f0ab".to_string(),
                commit_hash: Some("8e5e7e5ab8b370d6c329ec480221332ada57f0ab".to_string()),
                registry_type: RegistryType::GitHubActions,
                start_offset: 83,
                end_offset: 123,
                line: 4,
                column: 31,
            }
        );
    }

    #[test]
    fn parse_version_tag_has_no_commit_hash() {
        let parser = GitHubActionsParser::new();
        let content = r#"jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "actions/checkout".to_string(),
                version: "v4".to_string(),
                commit_hash: None,
                registry_type: RegistryType::GitHubActions,
                start_offset: 83,
                end_offset: 85,
                line: 4,
                column: 31,
            }
        );
    }
}
