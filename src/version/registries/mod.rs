//! Registry implementations for fetching package versions

pub mod crates_io;
pub mod github;
pub mod go_proxy;
pub mod npm;

pub use crates_io::CratesIoRegistry;
pub use github::GitHubRegistry;
pub use go_proxy::GoProxyRegistry;
pub use npm::NpmRegistry;
