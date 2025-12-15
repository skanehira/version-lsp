//! Parser layer
//! - traits.rs: Parser trait definition
//! - types.rs: Common types (PackageInfo, RegistryType)
//! - github_actions.rs: GitHub Actions workflow parser
//! - package_json.rs: package.json parser
//! - cargo_toml.rs: Cargo.toml parser
//! - go_mod.rs: go.mod parser

pub mod cargo_toml;
pub mod github_actions;
pub mod go_mod;
pub mod package_json;
pub mod traits;
pub mod types;

pub use cargo_toml::CargoTomlParser;
pub use github_actions::GitHubActionsParser;
pub use go_mod::GoModParser;
pub use package_json::PackageJsonParser;
pub use traits::{ParseError, Parser};
pub use types::{PackageInfo, RegistryType};
