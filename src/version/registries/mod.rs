//! Registry implementations for fetching package versions

pub mod crates_io;
pub mod github;
pub mod go_proxy;
pub mod jsr;
pub mod npm;
pub mod pypi;

pub use crates_io::CratesIoRegistry;
pub use github::GitHubRegistry;
pub use go_proxy::GoProxyRegistry;
pub use jsr::JsrRegistry;
pub use npm::NpmRegistry;
pub use pypi::PypiRegistry;
