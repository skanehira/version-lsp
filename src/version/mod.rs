//! Version management layer for package version checking
//!
//! This module provides the core functionality for fetching, caching, and comparing
//! package versions across multiple registries (npm, crates.io, Go proxy, GitHub Actions).
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
//! │   Registry  │────▶│    Cache    │◀────│   Checker   │
//! │  (fetch)    │     │  (storage)  │     │  (compare)  │
//! └─────────────┘     └─────────────┘     └─────────────┘
//!        │                                       │
//!        ▼                                       ▼
//! ┌─────────────┐                         ┌─────────────┐
//! │  Registries │                         │   Matcher   │
//! │ (npm,crates)│                         │(version cmp)│
//! └─────────────┘                         └─────────────┘
//! ```
//!
//! # Modules
//!
//! - [`cache`]: SQLite-based version cache with refresh logic
//! - [`checker`]: Version comparison and status determination
//! - [`matcher`]: Version matching trait and registry-specific implementations
//! - [`registry`]: Registry trait for fetching versions from remote sources
//! - [`registries`]: Concrete registry implementations (npm, crates.io, etc.)
//! - [`error`]: Error types for cache and registry operations
//! - [`semver`]: Shared semver utilities
//! - [`types`]: Common types like `PackageVersions`

pub mod cache;
pub mod checker;
pub mod error;
pub mod matcher;
pub mod matchers;
pub mod registries;
pub mod registry;
pub mod resolver;
pub mod resolvers;
pub mod semver;
pub mod types;
