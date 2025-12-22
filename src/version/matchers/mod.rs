//! Registry-specific version matchers

pub mod crates;
pub mod github_actions;
pub mod go;
pub mod jsr;
pub mod npm;
pub mod pnpm;

pub use crates::CratesVersionMatcher;
pub use github_actions::GitHubActionsMatcher;
pub use go::GoVersionMatcher;
pub use jsr::JsrVersionMatcher;
pub use npm::NpmVersionMatcher;
pub use pnpm::PnpmCatalogMatcher;
