//! Concrete `LockResolver` implementations.
//!
//! Resolvers are added per registry family; see `src/lsp/resolver.rs`
//! for registration and priority ordering.

pub mod cargo;
pub mod npm;
pub mod pnpm;

pub use cargo::CargoLockResolver;
pub use npm::NpmLockResolver;
pub use pnpm::PnpmLockResolver;
