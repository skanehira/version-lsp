//! Package.swift parser
//!
//! Parses Swift Package Manager manifests, extracting dependencies declared via
//! `.package(url: ..., from: ...)` and related forms.
//!
//! Supported version constraint forms:
//! - `.package(url: "https://github.com/x/y.git", from: "1.2.3")`
//! - `.package(url: "https://github.com/x/y.git", exact: "1.2.3")`
//! - `.package(url: "https://github.com/x/y.git", .upToNextMajor(from: "1.2.3"))`
//! - `.package(url: "https://github.com/x/y.git", .upToNextMinor(from: "1.2.3"))`
//! - `.package(url: "https://github.com/x/y.git", "1.0.0" ..< "2.0.0")`
//! - `.package(url: "https://github.com/x/y.git", "1.0.0" ... "2.0.0")`
//!
//! Branch and revision pins (`branch:`, `revision:`) and non-version-pinned
//! dependencies are skipped, since they have no version to compare. By default
//! only `github.com` URLs are emitted; additional hosts can be supplied via
//! [`PackageSwiftParser::with_allowed_hosts`] (typically derived from the
//! configured `swiftPm.url` so private GitHub Enterprise mirrors work).

use tracing::{debug, warn};

use crate::parser::traits::{ParseError, Parser};
use crate::parser::types::{PackageInfo, RegistryType};

/// Default Git host accepted when no explicit allow-list is configured.
pub const DEFAULT_ALLOWED_HOST: &str = "github.com";

pub struct PackageSwiftParser {
    /// Lowercased Git hosts whose `owner/repo` URLs should be extracted.
    /// Always contains at least `github.com`.
    allowed_hosts: Vec<String>,
}

impl PackageSwiftParser {
    /// Create a parser that only accepts the default GitHub host.
    pub fn new() -> Self {
        Self {
            allowed_hosts: vec![DEFAULT_ALLOWED_HOST.to_string()],
        }
    }

    /// Create a parser that accepts the default GitHub host plus any additional
    /// hosts supplied (typically the host of a private GitHub Enterprise
    /// mirror configured via `registries.swiftPm.url`).
    ///
    /// Hosts are compared case-insensitively. Empty strings are ignored.
    /// `github.com` is always accepted regardless of input.
    pub fn with_allowed_hosts<I, S>(extra_hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut allowed = vec![DEFAULT_ALLOWED_HOST.to_string()];
        for host in extra_hosts {
            let host = host.as_ref().trim().to_ascii_lowercase();
            if host.is_empty() || allowed.iter().any(|h| h == &host) {
                continue;
            }
            allowed.push(host);
        }
        Self {
            allowed_hosts: allowed,
        }
    }
}

impl Default for PackageSwiftParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for PackageSwiftParser {
    fn parse(&self, content: &str) -> Result<Vec<PackageInfo>, ParseError> {
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter_swift::LANGUAGE;
        parser.set_language(&language.into()).map_err(|e| {
            warn!("Failed to set Swift language for tree-sitter: {}", e);
            ParseError::TreeSitter(e.to_string())
        })?;

        let tree = parser.parse(content, None).ok_or_else(|| {
            warn!("Failed to parse Swift content");
            ParseError::ParseFailed("Failed to parse Swift".to_string())
        })?;

        let mut results = Vec::new();
        self.walk_for_package_calls(tree.root_node(), content, &mut results);
        Ok(results)
    }
}

/// Recursively walk the tree looking for `.package(...)` call expressions.
impl PackageSwiftParser {
    fn walk_for_package_calls(
        &self,
        node: tree_sitter::Node,
        content: &str,
        results: &mut Vec<PackageInfo>,
    ) {
        if node.kind() == "call_expression" && is_package_call(node, content) {
            if let Some(info) = self.extract_package_info(node, content) {
                results.push(info);
            }
            // Don't recurse into the matched .package(...) call: nested `.upToNextMajor(...)`
            // is not itself a top-level dependency declaration.
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_for_package_calls(child, content, results);
        }
    }

    /// Extract `PackageInfo` from a `.package(...)` call expression. Returns `None`
    /// if the call cannot be turned into a version-pinned dependency we can check
    /// (e.g. branch/revision pins, non-allowed-host URLs, or malformed input).
    fn extract_package_info(&self, node: tree_sitter::Node, content: &str) -> Option<PackageInfo> {
        let value_arguments = find_value_arguments(node)?;

        let mut url: Option<String> = None;
        let mut version_field: Option<VersionField> = None;

        let mut cursor = value_arguments.walk();
        for arg in value_arguments.children(&mut cursor) {
            if arg.kind() != "value_argument" {
                continue;
            }

            let label = label_text(arg, content);

            match label.as_deref() {
                Some("url") => {
                    url = string_literal_value(arg, content);
                }
                Some("from") | Some("exact") => {
                    version_field = extract_string_field(arg, content);
                }
                Some("branch") | Some("revision") | Some("name") | Some("path") => {
                    // Skip — no version to check.
                }
                _ => {
                    // Positional argument: may be a `.upToNextMajor(from: "...")`,
                    // `.upToNextMinor(from: "...")`, or a range like `"1.0.0" ..< "2.0.0"`.
                    if let Some(field) = extract_constructor_version(arg, content) {
                        version_field = Some(field);
                    } else if let Some(field) = extract_range_version(arg, content) {
                        version_field = Some(field);
                    }
                }
            }
        }

        let url = url?;
        let version_field = version_field?;
        let name = self.owner_repo_for_url(&url)?;

        Some(PackageInfo {
            name,
            version: version_field.version,
            commit_hash: None,
            registry_type: RegistryType::SwiftPm,
            start_offset: version_field.start_offset,
            end_offset: version_field.end_offset,
            line: version_field.line,
            column: version_field.column,
            extra_info: None,
        })
    }

    /// Convert a Git URL into the `owner/repo` slug used by the GitHub
    /// Releases API. Returns `None` if the URL's host is not in the
    /// allow-list or the URL is malformed.
    ///
    /// Accepts both HTTPS (`https://host/owner/repo[.git][/]`) and SSH
    /// (`git@host:owner/repo[.git]`) forms.
    fn owner_repo_for_url(&self, url: &str) -> Option<String> {
        let (host, rest) = split_host_and_path(url)?;
        if !self.allowed_hosts.iter().any(|h| h == &host) {
            debug!(
                "Skipping SPM dependency: host '{}' not in allowed_hosts {:?}",
                host, self.allowed_hosts
            );
            return None;
        }
        parse_owner_repo(rest)
    }
}

/// Check whether a `call_expression` is a `.package(...)` member call.
fn is_package_call(node: tree_sitter::Node, content: &str) -> bool {
    let Some(prefix) = first_child_of_kind(node, "prefix_expression") else {
        return false;
    };
    // prefix_expression children: ".", simple_identifier
    let Some(ident) = first_child_of_kind(prefix, "simple_identifier") else {
        return false;
    };
    content[ident.byte_range()].trim() == "package"
}

/// A located version literal (start/end offsets bracket the version text in the
/// source — for single-version constraints they exclude the surrounding quotes;
/// for range expressions they span both bounds plus the operator so the
/// diagnostic underline covers the whole range).
struct VersionField {
    version: String,
    start_offset: usize,
    end_offset: usize,
    line: usize,
    column: usize,
}

/// Locate the `value_arguments` node inside a `.package(...)` call_expression.
fn find_value_arguments(call: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut cursor = call.walk();
    for child in call.children(&mut cursor) {
        if child.kind() == "call_suffix" {
            let mut suffix_cursor = child.walk();
            for grandchild in child.children(&mut suffix_cursor) {
                if grandchild.kind() == "value_arguments" {
                    return Some(grandchild);
                }
            }
        }
    }
    None
}

/// Read the label of a `value_argument` node (e.g. "url", "from", "exact").
fn label_text(arg: tree_sitter::Node, content: &str) -> Option<String> {
    let mut cursor = arg.walk();
    for child in arg.children(&mut cursor) {
        if child.kind() == "value_argument_label" {
            let mut label_cursor = child.walk();
            for inner in child.children(&mut label_cursor) {
                if inner.kind() == "simple_identifier" {
                    return Some(content[inner.byte_range()].to_string());
                }
            }
        }
    }
    None
}

/// Extract the inner text of a `line_string_literal` child of a `value_argument`.
fn string_literal_value(arg: tree_sitter::Node, content: &str) -> Option<String> {
    line_str_text_node(arg).map(|node| content[node.byte_range()].to_string())
}

/// Extract a `VersionField` from a `value_argument` whose value is a `line_string_literal`.
/// Used for `from: "..."` and `exact: "..."`.
fn extract_string_field(arg: tree_sitter::Node, content: &str) -> Option<VersionField> {
    let str_text = line_str_text_node(arg)?;
    let start_point = str_text.start_position();
    Some(VersionField {
        version: content[str_text.byte_range()].to_string(),
        start_offset: str_text.start_byte(),
        end_offset: str_text.end_byte(),
        line: start_point.row,
        column: start_point.column,
    })
}

/// Extract a `VersionField` from a positional argument that is itself a
/// `call_expression` such as `.upToNextMajor(from: "1.2.3")` or
/// `.upToNextMinor(from: "1.2.3")`.
fn extract_constructor_version(arg: tree_sitter::Node, content: &str) -> Option<VersionField> {
    let call = first_child_of_kind(arg, "call_expression")?;

    // Verify the prefix identifier is one we recognize.
    let prefix = first_child_of_kind(call, "prefix_expression")?;
    let ident_node = first_child_of_kind(prefix, "simple_identifier")?;
    let ident = &content[ident_node.byte_range()];
    if ident != "upToNextMajor" && ident != "upToNextMinor" {
        return None;
    }

    // Find the inner `from: "..."` argument.
    let suffix = first_child_of_kind(call, "call_suffix")?;
    let inner_args = first_child_of_kind(suffix, "value_arguments")?;
    let mut cursor = inner_args.walk();
    for inner in inner_args.children(&mut cursor) {
        if inner.kind() != "value_argument" {
            continue;
        }
        if label_text(inner, content).as_deref() == Some("from") {
            return extract_string_field(inner, content);
        }
    }
    None
}

/// Extract a `VersionField` from a positional argument that is a Swift range
/// expression: `"A" ..< "B"` (half-open) or `"A" ... "B"` (closed).
///
/// Half-open ranges become `>=A, <B` (Cargo-compound, matching SPM semantics).
/// Closed ranges become `>=A, <=B`. The matcher delegates to the Cargo matcher
/// which already understands comma-separated AND requirements.
///
/// Offsets span from the start of the lower bound (excluding its leading `"`)
/// through the end of the upper bound (excluding its trailing `"`), so the
/// diagnostic underline covers the entire range expression.
fn extract_range_version(arg: tree_sitter::Node, content: &str) -> Option<VersionField> {
    let range = first_child_of_kind(arg, "range_expression")?;

    // Collect the two string-literal bounds and the operator kind.
    let mut bounds: Vec<tree_sitter::Node> = Vec::with_capacity(2);
    let mut operator: Option<&str> = None;
    let mut cursor = range.walk();
    for child in range.children(&mut cursor) {
        match child.kind() {
            "line_string_literal" => bounds.push(child),
            "..<" => operator = Some("..<"),
            "..." => operator = Some("..."),
            _ => {}
        }
    }

    if bounds.len() != 2 {
        return None;
    }
    let op = operator?;

    let lower_text = first_child_of_kind(bounds[0], "line_str_text")?;
    let upper_text = first_child_of_kind(bounds[1], "line_str_text")?;

    let lower = &content[lower_text.byte_range()];
    let upper = &content[upper_text.byte_range()];

    let upper_op = match op {
        "..<" => "<",
        "..." => "<=",
        _ => return None,
    };
    let version = format!(">={}, {}{}", lower, upper_op, upper);

    let start_point = lower_text.start_position();
    Some(VersionField {
        version,
        // Span the full range expression (bound to bound, operator included)
        // so diagnostics underline the user-visible source verbatim.
        start_offset: lower_text.start_byte(),
        end_offset: upper_text.end_byte(),
        line: start_point.row,
        column: start_point.column,
    })
}

/// Find the inner `line_str_text` node of a value_argument's `line_string_literal`.
fn line_str_text_node(arg: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let lit = first_child_of_kind(arg, "line_string_literal")?;
    first_child_of_kind(lit, "line_str_text")
}

fn first_child_of_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

/// Split a Git URL into `(host, path_after_host)`. Returns `None` for
/// unsupported schemes or malformed input. The returned host is lowercased
/// for case-insensitive comparison against the parser's allow-list.
///
/// Accepts:
/// - `https://host/owner/repo[.git][/]`
/// - `http://host/owner/repo[.git][/]`
/// - `git@host:owner/repo[.git]` (SSH)
fn split_host_and_path(url: &str) -> Option<(String, &str)> {
    let url = url.trim();

    // SSH form: git@host:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        if host.is_empty() {
            return None;
        }
        return Some((host.to_ascii_lowercase(), path));
    }

    // HTTPS / HTTP form
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = after_scheme.split_once('/')?;
    if host.is_empty() {
        return None;
    }
    Some((host.to_ascii_lowercase(), path))
}

fn parse_owner_repo(rest: &str) -> Option<String> {
    let trimmed = rest.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let mut parts = trimmed.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    // Reject anything past owner/repo (e.g. trailing tree paths) — those are
    // not valid SPM package URLs.
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{}/{}", owner, repo))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn parse_extracts_from_dependency() {
        let parser = PackageSwiftParser::new();
        let content = r#"// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "MyApp",
    dependencies: [
        .package(url: "https://github.com/vapor/vapor.git", from: "4.92.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        let pkg = &result[0];
        assert_eq!(pkg.name, "vapor/vapor");
        assert_eq!(pkg.version, "4.92.0");
        assert_eq!(pkg.registry_type, RegistryType::SwiftPm);
        assert!(pkg.commit_hash.is_none());
        // Offsets should land on the version literal (excluding quotes).
        assert_eq!(&content[pkg.start_offset..pkg.end_offset], "4.92.0");
    }

    #[test]
    fn parse_extracts_exact_dependency() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/apple/swift-nio.git", exact: "2.50.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "apple/swift-nio");
        assert_eq!(result[0].version, "2.50.0");
    }

    #[test]
    fn parse_extracts_up_to_next_major() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/apple/swift-log.git", .upToNextMajor(from: "1.5.0")),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "apple/swift-log");
        assert_eq!(result[0].version, "1.5.0");
    }

    #[test]
    fn parse_extracts_up_to_next_minor() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    name: "App",
    dependencies: [
        .package(url: "https://github.com/apple/swift-metrics.git", .upToNextMinor(from: "2.4.0")),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "apple/swift-metrics");
        assert_eq!(result[0].version, "2.4.0");
    }

    #[test]
    fn parse_skips_branch_pin() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://github.com/some/repo.git", branch: "main"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_skips_revision_pin() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://github.com/some/repo.git", revision: "abc123"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_extracts_half_open_range_expression() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", "1.0.0" ..< "5.0.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "apple/swift-crypto");
        assert_eq!(result[0].version, ">=1.0.0, <5.0.0");
        // Underline should span the full range expression in the source.
        assert_eq!(
            &content[result[0].start_offset..result[0].end_offset],
            "1.0.0\" ..< \"5.0.0"
        );
    }

    #[test]
    fn parse_extracts_closed_range_expression() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", "1.0.0" ... "5.0.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "apple/swift-crypto");
        assert_eq!(result[0].version, ">=1.0.0, <=5.0.0");
    }

    #[test]
    fn parse_handles_range_expression_without_whitespace() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", "1.0.0"..<"5.0.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, ">=1.0.0, <5.0.0");
    }

    #[test]
    fn parse_skips_non_allowed_host_by_default() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://gitlab.com/some/repo.git", from: "1.0.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_accepts_private_host_when_in_allowed_list() {
        let parser = PackageSwiftParser::with_allowed_hosts(["github.example.com"]);
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://github.example.com/team/internal-lib.git", from: "1.0.0"),
        .package(url: "https://github.com/vapor/vapor.git", from: "4.92.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "team/internal-lib");
        assert_eq!(result[1].name, "vapor/vapor");
    }

    #[test]
    fn parse_rejects_unlisted_host_even_with_other_allowed_hosts() {
        let parser = PackageSwiftParser::with_allowed_hosts(["github.example.com"]);
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://gitlab.com/x/y.git", from: "1.0.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn with_allowed_hosts_is_case_insensitive_and_dedupes() {
        let parser =
            PackageSwiftParser::with_allowed_hosts(["GITHUB.COM", "GitHub.Example.Com", "  "]);
        // github.com plus the (lowercased) private host; duplicates and empty
        // entries are skipped.
        assert_eq!(
            parser.allowed_hosts,
            vec!["github.com".to_string(), "github.example.com".to_string()]
        );
    }

    #[rstest]
    #[case("https://github.com/vapor/vapor.git", Some("vapor/vapor"))]
    #[case("https://github.com/vapor/vapor", Some("vapor/vapor"))]
    #[case("https://github.com/apple/swift-nio.git/", Some("apple/swift-nio"))]
    #[case("git@github.com:vapor/vapor.git", Some("vapor/vapor"))]
    #[case("https://gitlab.com/x/y.git", None)]
    #[case("https://github.com/owner", None)]
    #[case("https://github.com/owner/repo/extra", None)]
    #[case("not-a-url", None)]
    fn owner_repo_for_url_with_default_hosts(#[case] url: &str, #[case] expected: Option<&str>) {
        let parser = PackageSwiftParser::new();
        assert_eq!(parser.owner_repo_for_url(url).as_deref(), expected);
    }

    #[rstest]
    #[case("https://github.example.com/team/lib.git", Some("team/lib"))]
    #[case("git@github.example.com:team/lib.git", Some("team/lib"))]
    #[case("https://github.com/vapor/vapor.git", Some("vapor/vapor"))]
    #[case("https://gitlab.com/x/y.git", None)]
    fn owner_repo_for_url_with_private_host(#[case] url: &str, #[case] expected: Option<&str>) {
        let parser = PackageSwiftParser::with_allowed_hosts(["github.example.com"]);
        assert_eq!(parser.owner_repo_for_url(url).as_deref(), expected);
    }

    #[test]
    fn parse_extracts_multiple_dependencies() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://github.com/vapor/vapor.git", from: "4.92.0"),
        .package(url: "https://github.com/apple/swift-nio.git", exact: "2.50.0"),
        .package(url: "https://github.com/apple/swift-log.git", .upToNextMajor(from: "1.5.0")),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "vapor/vapor");
        assert_eq!(result[0].version, "4.92.0");
        assert_eq!(result[1].name, "apple/swift-nio");
        assert_eq!(result[1].version, "2.50.0");
        assert_eq!(result[2].name, "apple/swift-log");
        assert_eq!(result[2].version, "1.5.0");
    }

    #[test]
    fn parse_returns_empty_for_no_dependencies() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(name: "MyApp")
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_handles_dotgit_suffix_and_trailing_slash() {
        let parser = PackageSwiftParser::new();
        let content = r#"import PackageDescription
let package = Package(
    dependencies: [
        .package(url: "https://github.com/owner/repo/", from: "1.0.0"),
    ]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "owner/repo");
    }
}
