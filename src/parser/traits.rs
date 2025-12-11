//! Parser trait definition

#[cfg(test)]
use mockall::automock;

use crate::parser::types::PackageInfo;

/// Trait for parsing package files
#[cfg_attr(test, automock)]
pub trait Parser: Send + Sync {
    /// Parse the content and extract package information
    fn parse(&self, content: &str) -> Result<Vec<PackageInfo>, ParseError>;
}

/// Error type for parsing operations
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Failed to parse the file structure
    #[error("Failed to parse file: {0}")]
    ParseFailed(String),

    /// Invalid syntax in the file
    #[error("Invalid syntax: {0}")]
    InvalidSyntax(String),

    /// Tree-sitter related error
    #[error("Tree-sitter error: {0}")]
    TreeSitter(String),
}
