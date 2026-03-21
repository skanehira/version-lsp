//! Docker Compose file parser
//!
//! Parses compose.yaml / docker-compose.yaml to extract container image tags.
//! Supports Docker Hub (official and user images) and ghcr.io images.

use crate::parser::traits::{ParseError, Parser};
use crate::parser::types::{PackageInfo, RegistryType};
use tracing::warn;

/// Parser for compose.yaml / docker-compose.yaml files
#[derive(Default)]
pub struct ComposeParser;

impl ComposeParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for ComposeParser {
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

        find_services_images(root, content, &mut results);

        Ok(results)
    }
}

/// Find services section and extract image fields
fn find_services_images(node: tree_sitter::Node, content: &str, results: &mut Vec<PackageInfo>) {
    if node.kind() == "block_mapping_pair"
        && let Some(key_node) = node.child_by_field_name("key")
    {
        let key = get_node_text(key_node, content);
        if key == "services" {
            if let Some(value_node) = node.child_by_field_name("value") {
                extract_service_images(value_node, content, results);
            }
            return;
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_services_images(child, content, results);
    }
}

/// Extract image fields from each service definition
fn extract_service_images(node: tree_sitter::Node, content: &str, results: &mut Vec<PackageInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "block_mapping" {
            // Each child of block_mapping is a service definition
            let mut inner_cursor = child.walk();
            for service_pair in child.children(&mut inner_cursor) {
                if service_pair.kind() == "block_mapping_pair"
                    && let Some(value_node) = service_pair.child_by_field_name("value")
                {
                    extract_image_from_service(value_node, content, results);
                }
            }
        }
    }
}

/// Extract the image field from a service's mapping
fn extract_image_from_service(
    node: tree_sitter::Node,
    content: &str,
    results: &mut Vec<PackageInfo>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "block_mapping" {
            let mut inner_cursor = child.walk();
            for pair in child.children(&mut inner_cursor) {
                if pair.kind() == "block_mapping_pair"
                    && let Some(key_node) = pair.child_by_field_name("key")
                {
                    let key = get_node_text(key_node, content);
                    if key == "image"
                        && let Some(value_node) = pair.child_by_field_name("value")
                        && let Some(info) = parse_image_value(value_node, content)
                    {
                        results.push(info);
                    }
                }
            }
        }
    }
}

/// Parse an image value like "nginx:1.25" or "ghcr.io/owner/repo:v1.0.0"
fn parse_image_value(node: tree_sitter::Node, content: &str) -> Option<PackageInfo> {
    let raw_text = &content[node.byte_range()];
    let trimmed = raw_text.trim();

    // Remove quotes if present
    let image_ref = if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    // Skip variable expansions
    if image_ref.contains("${") || image_ref.contains("$") {
        return None;
    }

    // Skip digest references
    if image_ref.contains('@') {
        return None;
    }

    // Split image:tag
    let (image_name, tag) = image_ref.rsplit_once(':')?;

    // Skip "latest" tag
    if tag == "latest" {
        return None;
    }

    // Skip empty tag
    if tag.is_empty() {
        return None;
    }

    // Determine registry and normalize name
    let name = resolve_image_name(image_name)?;

    // Calculate offset for the tag part (after the colon)
    let colon_pos = image_ref.rfind(':')?;
    let value_start = node.start_byte();

    // Account for quotes
    let quote_offset = if raw_text.trim().starts_with('"') || raw_text.trim().starts_with('\'') {
        1
    } else {
        0
    };

    // Find where the actual text starts within the node (skip leading whitespace)
    let leading_whitespace = raw_text.len() - raw_text.trim_start().len();

    let tag_start_offset = value_start + leading_whitespace + quote_offset + colon_pos + 1;
    let tag_end_offset = tag_start_offset + tag.len();

    // Calculate line/column from tag start offset
    let (line, column) = offset_to_line_col(content, tag_start_offset);

    Some(PackageInfo {
        name,
        version: tag.to_string(),
        commit_hash: None,
        registry_type: RegistryType::Docker,
        start_offset: tag_start_offset,
        end_offset: tag_end_offset,
        line,
        column,
        extra_info: None,
    })
}

/// Resolve image name to registry-appropriate format.
///
/// - `nginx` → `library/nginx` (Docker Hub official)
/// - `myuser/myapp` → `myuser/myapp` (Docker Hub user)
/// - `ghcr.io/owner/repo` → `ghcr.io/owner/repo` (GitHub Container Registry)
/// - `mcr.microsoft.com/...` → None (unsupported)
fn resolve_image_name(image_name: &str) -> Option<String> {
    // Check if it has a domain (contains '.')
    if let Some((domain, _rest)) = image_name.split_once('/')
        && domain.contains('.')
    {
        if domain == "ghcr.io" {
            return Some(image_name.to_string());
        }
        // Unsupported third-party registries
        return None;
    }

    // Docker Hub: no domain part
    if image_name.contains('/') {
        // User image: myuser/myapp
        Some(image_name.to_string())
    } else {
        // Official image: nginx → library/nginx
        Some(format!("library/{}", image_name))
    }
}

/// Calculate line (0-indexed) and column (0-indexed) from byte offset
fn offset_to_line_col(content: &str, offset: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in content.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Get text content of a node, removing quotes if present
fn get_node_text(node: tree_sitter::Node, content: &str) -> String {
    let text = &content[node.byte_range()];
    text.trim()
        .trim_start_matches('"')
        .trim_end_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('\'')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn parse_extracts_docker_hub_official_image() {
        let parser = ComposeParser::new();
        let content = "services:\n  web:\n    image: nginx:1.25\n";
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PackageInfo {
                name: "library/nginx".to_string(),
                version: "1.25".to_string(),
                commit_hash: None,
                registry_type: RegistryType::Docker,
                start_offset: 34,
                end_offset: 38,
                line: 2,
                column: 17,
                extra_info: None,
            }
        );
    }

    #[test]
    fn parse_extracts_docker_hub_user_image() {
        let parser = ComposeParser::new();
        let content = "services:\n  app:\n    image: myuser/myapp:v1.0.0\n";
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "myuser/myapp");
        assert_eq!(result[0].version, "v1.0.0");
    }

    #[test]
    fn parse_extracts_ghcr_image() {
        let parser = ComposeParser::new();
        let content = "services:\n  app:\n    image: ghcr.io/owner/repo:v1.0.0\n";
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ghcr.io/owner/repo");
        assert_eq!(result[0].version, "v1.0.0");
    }

    #[test]
    fn parse_extracts_multiple_services() {
        let parser = ComposeParser::new();
        let content = r#"services:
  web:
    image: nginx:1.25
  db:
    image: postgres:15
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "library/nginx");
        assert_eq!(result[0].version, "1.25");
        assert_eq!(result[1].name, "library/postgres");
        assert_eq!(result[1].version, "15");
    }

    #[test]
    fn parse_skips_image_without_tag() {
        let parser = ComposeParser::new();
        let content = "services:\n  web:\n    image: nginx\n";
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_skips_latest_tag() {
        let parser = ComposeParser::new();
        let content = "services:\n  web:\n    image: nginx:latest\n";
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_skips_digest_reference() {
        let parser = ComposeParser::new();
        let content = "services:\n  web:\n    image: nginx@sha256:abc123def456\n";
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_skips_variable_expansion() {
        let parser = ComposeParser::new();
        let content = "services:\n  web:\n    image: ${IMAGE_NAME}:${TAG}\n";
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_skips_unsupported_registry() {
        let parser = ComposeParser::new();
        let content = "services:\n  web:\n    image: mcr.microsoft.com/dotnet/sdk:8.0\n";
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_skips_service_with_build_only() {
        let parser = ComposeParser::new();
        let content = "services:\n  app:\n    build: .\n";
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_handles_suffixed_tag() {
        let parser = ComposeParser::new();
        let content = "services:\n  web:\n    image: nginx:1.25-alpine\n";
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "library/nginx");
        assert_eq!(result[0].version, "1.25-alpine");
    }

    #[test]
    fn parse_handles_quoted_image() {
        let parser = ComposeParser::new();
        let content = "services:\n  web:\n    image: \"nginx:1.25\"\n";
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "library/nginx");
        assert_eq!(result[0].version, "1.25");
    }

    #[rstest]
    #[case("nginx", Some("library/nginx"))]
    #[case("myuser/myapp", Some("myuser/myapp"))]
    #[case("ghcr.io/owner/repo", Some("ghcr.io/owner/repo"))]
    #[case("mcr.microsoft.com/dotnet/sdk", None)]
    #[case("quay.io/prometheus/node-exporter", None)]
    fn resolve_image_name_returns_expected(#[case] input: &str, #[case] expected: Option<&str>) {
        assert_eq!(resolve_image_name(input), expected.map(|s| s.to_string()));
    }

    #[test]
    fn parse_returns_empty_for_non_compose_yaml() {
        let parser = ComposeParser::new();
        let content = "name: test\nversion: 1.0\n";
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }
}
