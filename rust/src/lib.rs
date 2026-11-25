#![recursion_limit = "512"]

//! MojModelBridge native engine.
//!
//! The Python Beta 12 engine in ../oldVer remains a read-only behavioral
//! reference. This crate does not invoke Python, import Python modules, or
//! require a Python installation.

pub mod archive;
pub mod animation;
pub mod blockbench;
pub mod block_model_bridge;
pub mod bytecode;
pub mod compiled_registry;
pub mod discovery;
pub mod metadata;
pub mod neoforge_docs;
pub mod neoforge_project;
pub mod neoforge_port;
pub mod item_method_dictionary;
pub mod legacy_api_dictionary;
pub mod native_recipe_bridge;
pub mod native_block_entity;
pub mod native_lore;
pub mod localization;
pub mod mcreator_model;
pub mod mcreator_entity;
pub mod mod_graph;
pub mod model;
pub mod pipeline;
pub mod recipe_bridge;
pub mod resource_bridge;
pub mod source_bridge;
pub mod scan;
pub mod source;
pub mod texture;
pub mod viewer;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LEGACY_REFERENCE_VERSION: &str = "3.4.0-beta12";

pub mod workspace_validation;
pub mod worldgen_bridge;
pub mod mcreator_workspace;

pub mod mcreator_equipment;

pub mod mcreator_armor;

pub mod mcreator_block;

pub mod mcreator_plant;

pub mod mcreator_worldgen;

pub mod mcreator_projectile;
pub mod mcreator_pipeline;
pub mod gradle_bridge;
