//! MojModelBridge native engine.
//!
//! The Python Beta 12 engine in ../oldVer remains a read-only behavioral
//! reference. This crate does not invoke Python, import Python modules, or
//! require a Python installation.

pub mod archive;
pub mod animation;
pub mod blockbench;
pub mod bytecode;
pub mod discovery;
pub mod metadata;
pub mod mod_graph;
pub mod model;
pub mod pipeline;
pub mod scan;
pub mod source;
pub mod texture;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LEGACY_REFERENCE_VERSION: &str = "3.4.0-beta12";

pub mod workspace_validation;
