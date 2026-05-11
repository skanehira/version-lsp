//! Concrete `LockResolver` implementations.
//!
//! Resolvers are added per registry family; see `src/lsp/resolver.rs`
//! for registration and priority ordering.

pub mod cargo;

pub use cargo::CargoLockResolver;
