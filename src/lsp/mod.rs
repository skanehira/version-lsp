//! LSP (Language Server Protocol) implementation layer
//!
//! This module handles communication with editors via LSP and provides
//! diagnostics for package version checking.
//!
//! # Modules
//!
//! - [`backend`]: Main LSP backend implementing `LanguageServer` trait
//! - [`diagnostics`]: Generates version-related diagnostics (warnings, errors)
//! - [`refresh`]: Background refresh logic for package version cache
//! - [`resolver`]: Groups parser, matcher, and registry per registry type
//! - [`server`]: LSP server initialization and lifecycle

pub mod backend;
pub mod code_action;
pub mod diagnostics;
pub mod refresh;
pub mod resolver;
pub mod server;
