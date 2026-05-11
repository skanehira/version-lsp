//! Concrete `LockResolver` implementations.
//!
//! Resolvers are added per registry family; see `src/lsp/resolver.rs`
//! for registration and priority ordering.

pub mod cargo;
pub mod npm;
pub mod pnpm;
pub mod poetry;
pub mod uv;
pub mod yarn;

pub use cargo::CargoLockResolver;
pub use npm::NpmLockResolver;
pub use pnpm::PnpmLockResolver;
pub use poetry::PoetryLockResolver;
pub use uv::UvLockResolver;
pub use yarn::YarnLockResolver;
