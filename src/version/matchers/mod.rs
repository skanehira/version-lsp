//! Registry-specific version matchers

pub mod crates;
pub mod github_actions;
pub mod go;
pub mod npm;

pub use crates::CratesVersionMatcher;
pub use github_actions::GitHubActionsMatcher;
pub use go::GoVersionMatcher;
pub use npm::NpmVersionMatcher;
