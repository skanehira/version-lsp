//! go.mod parser
//!
//! Parses go.mod files to extract module dependencies.
//! Supports both single-line require directives and require blocks.
//!
//! Format examples:
//! - Single: `require golang.org/x/text v0.14.0`
//! - Block:
//!   ```text
//!   require (
//!       golang.org/x/text v0.14.0
//!       golang.org/x/net v0.20.0 // indirect
//!   )
//!   ```

use regex::Regex;

use crate::parser::traits::{ParseError, Parser};
use crate::parser::types::{PackageInfo, RegistryType};

/// Parser for go.mod files
pub struct GoModParser {
    /// Regex for single-line require: `require module/path v1.2.3`
    single_require_re: Regex,
    /// Regex for require block start: `require (`
    block_start_re: Regex,
    /// Regex for require spec inside block: `module/path v1.2.3`
    require_spec_re: Regex,
}

impl GoModParser {
    pub fn new() -> Self {
        Self {
            // Match: require module/path v1.2.3 [// comment]
            single_require_re: Regex::new(r"^require\s+(\S+)\s+(v[^\s]+)(?:\s*//.*)?$").unwrap(),
            // Match: require (
            block_start_re: Regex::new(r"^require\s*\(\s*$").unwrap(),
            // Match: module/path v1.2.3 [// comment]
            require_spec_re: Regex::new(r"^\s*(\S+)\s+(v[^\s]+)(?:\s*//.*)?$").unwrap(),
        }
    }
}

impl Default for GoModParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for GoModParser {
    fn parse(&self, content: &str) -> Result<Vec<PackageInfo>, ParseError> {
        let mut results = Vec::new();
        let mut in_require_block = false;

        for (line_num, line) in content.lines().enumerate() {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }

            // Check for block end
            if in_require_block && trimmed == ")" {
                in_require_block = false;
                continue;
            }

            // Check for block start
            if self.block_start_re.is_match(trimmed) {
                in_require_block = true;
                continue;
            }

            // Parse require spec
            if in_require_block {
                if let Some(caps) = self.require_spec_re.captures(line) {
                    let module_path = caps.get(1).unwrap().as_str();
                    let version_match = caps.get(2).unwrap();
                    let version = version_match.as_str();

                    // Calculate byte offset for version
                    let line_start = content
                        .lines()
                        .take(line_num)
                        .map(|l| l.len() + 1)
                        .sum::<usize>();
                    let version_start = line_start + version_match.start();
                    let version_end = line_start + version_match.end();

                    // Calculate column (byte offset within line)
                    let column = version_match.start();

                    results.push(PackageInfo {
                        name: module_path.to_string(),
                        version: version.to_string(),
                        commit_hash: None,
                        registry_type: RegistryType::GoProxy,
                        start_offset: version_start,
                        end_offset: version_end,
                        line: line_num,
                        column,
                        extra_info: None,
                    });
                }
            } else if let Some(caps) = self.single_require_re.captures(trimmed) {
                let module_path = caps.get(1).unwrap().as_str();
                let version_match = caps.get(2).unwrap();
                let version = version_match.as_str();

                // Calculate byte offset for version
                let line_start = content
                    .lines()
                    .take(line_num)
                    .map(|l| l.len() + 1)
                    .sum::<usize>();
                // Find actual position in the original line (not trimmed)
                let require_pos = line.find("require").unwrap_or(0);
                let version_pos_in_line = line[require_pos..]
                    .find(version)
                    .map(|p| require_pos + p)
                    .unwrap_or(0);
                let version_start = line_start + version_pos_in_line;
                let version_end = version_start + version.len();

                results.push(PackageInfo {
                    name: module_path.to_string(),
                    version: version.to_string(),
                    commit_hash: None,
                    registry_type: RegistryType::GoProxy,
                    start_offset: version_start,
                    end_offset: version_end,
                    line: line_num,
                    column: version_pos_in_line,
                    extra_info: None,
                });
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_single_require() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

go 1.21

require golang.org/x/text v0.14.0
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "golang.org/x/text");
        assert_eq!(result[0].version, "v0.14.0");
        assert_eq!(result[0].registry_type, RegistryType::GoProxy);
    }

    #[test]
    fn parse_extracts_require_block() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

go 1.21

require (
	golang.org/x/text v0.14.0
	golang.org/x/net v0.20.0
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "golang.org/x/text");
        assert_eq!(result[0].version, "v0.14.0");
        assert_eq!(result[1].name, "golang.org/x/net");
        assert_eq!(result[1].version, "v0.20.0");
    }

    #[test]
    fn parse_handles_indirect_dependencies() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

require (
	golang.org/x/text v0.14.0 // indirect
	golang.org/x/net v0.20.0
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "golang.org/x/text");
        assert_eq!(result[0].version, "v0.14.0");
    }

    #[test]
    fn parse_handles_prerelease_versions() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

require golang.org/x/text v0.14.0-beta.1
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "v0.14.0-beta.1");
    }

    #[test]
    fn parse_handles_incompatible_suffix() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

require github.com/some/repo v2.0.0+incompatible
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "v2.0.0+incompatible");
    }

    #[test]
    fn parse_handles_pseudo_versions() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

require github.com/some/repo v0.0.0-20210101000000-abcdef123456
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "v0.0.0-20210101000000-abcdef123456");
    }

    #[test]
    fn parse_returns_empty_for_no_requires() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

go 1.21
"#;
        let result = parser.parse(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_handles_mixed_single_and_block() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

require golang.org/x/text v0.14.0

require (
	golang.org/x/net v0.20.0
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "golang.org/x/text");
        assert_eq!(result[1].name, "golang.org/x/net");
    }

    #[test]
    fn parse_skips_replace_directive() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

go 1.21

require golang.org/x/text v0.14.0

replace golang.org/x/text v0.14.0 => ./local/text

replace (
	golang.org/x/net => ../fork/net
	example.com/old => example.com/new v1.0.0
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "golang.org/x/text");
    }

    #[test]
    fn parse_skips_exclude_directive() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

require golang.org/x/text v0.14.0

exclude golang.org/x/net v1.2.3

exclude (
	golang.org/x/crypto v1.4.5
	golang.org/x/text v1.6.7
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "golang.org/x/text");
    }

    #[test]
    fn parse_skips_retract_directive() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

require golang.org/x/text v0.14.0

retract v1.0.0

retract (
	v1.0.1
	[v1.0.0, v1.9.9]
)
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "golang.org/x/text");
    }

    #[test]
    fn parse_handles_all_directives_mixed() {
        let parser = GoModParser::new();
        let content = r#"module example.com/myapp

go 1.21

require (
	golang.org/x/text v0.14.0
	golang.org/x/net v0.20.0
)

replace golang.org/x/text => ./local/text

exclude golang.org/x/crypto v1.0.0

retract v0.0.1
"#;
        let result = parser.parse(content).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "golang.org/x/text");
        assert_eq!(result[1].name, "golang.org/x/net");
    }
}
