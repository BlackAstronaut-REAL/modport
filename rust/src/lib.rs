//! MojModelBridge native engine.
//!
//! This crate is the Rust implementation. The Python Beta 12 engine in
//! ../oldVer remains a read-only behavioral reference and is not invoked by
//! this crate or required at runtime.

pub mod archive;
pub mod bytecode;
pub mod discovery;
pub mod metadata;
pub mod model;
pub mod scan;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LEGACY_REFERENCE_VERSION: &str = "3.4.0-beta12";
