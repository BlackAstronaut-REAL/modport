//! Source-driven discovery of modern NeoForge block-entity registrations.
//!
//! This module deliberately only accepts registrations that can be tied back to
//! a generated block. Unmatched block-entity declarations remain warnings rather
//! than generating a type which can never be attached to a block.

use anyhow::Result;
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct BlockEntitySourceRecord {
    pub registry_name: String,
    pub source_class: String,
    pub source_path: String,
    pub associated_blocks: Vec<String>,
    pub warnings: Vec<String>,
}

fn java_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in roots {
        if !root.is_dir() { continue; }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("java")) {
                files.push(entry.into_path());
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn compact(value: &str) -> String {
    value.chars().filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase).collect()
}

fn snake(value: &str) -> String {
    let characters: Vec<char> = value.chars().collect();
    let mut result = String::new();
    for (index, character) in characters.iter().copied().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            let previous = characters[index - 1];
            let next_lower = characters.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower) {
                result.push('_');
            }
        }
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
        } else if !result.is_empty() && !result.ends_with('_') {
            result.push('_');
        }
    }
    result.trim_matches('_').to_owned()
}

fn normalize_registry(value: &str) -> String {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    let value = value.rsplit(':').next().unwrap_or(value);
    let normalized = snake(value);
    if normalized.is_empty() { "converted_block_entity".to_owned() } else { normalized }
}

fn simple_class(value: &str) -> String {
    value.rsplit('.').next().unwrap_or(value).rsplit('$').next().unwrap_or(value).to_owned()
}

fn mapped_block_ids(block_expression: &str, blocks: &[(String, String)]) -> Vec<String> {
    let field_pattern = Regex::new(r"(?:ModBlocks\s*\.\s*)?([A-Za-z_$][A-Za-z0-9_$]*)")
        .expect("static block field regex");
    let mut identifiers = BTreeSet::new();
    for capture in field_pattern.captures_iter(block_expression) {
        if let Some(field) = capture.get(1) {
            identifiers.insert(field.as_str().to_owned());
        }
    }

    let mut matched = BTreeSet::new();
    for field in &identifiers {
        for (registry_name, block_class) in blocks {
            let field_snake = snake(field);
            if field_snake.as_str() == registry_name.as_str()
                || compact(field) == compact(registry_name)
                || compact(field) == compact(block_class)
                || snake(field).ends_with(&format!("_{}", registry_name))
            {
                matched.insert(registry_name.clone());
            }
        }
    }
    matched.into_iter().collect()
}

/// Discover BlockEntityType.Builder registrations and associate them to generated
/// blocks via ModBlocks.FIELD references in the documented NeoForge pattern.
pub fn discover_block_entities(
    roots: &[PathBuf],
    blocks: &[(String, String)],
) -> Result<Vec<BlockEntitySourceRecord>> {
    let registration = Regex::new(
        r#"(?is)(?:BLOCK_ENTITY_TYPES|TILE_ENTITY_TYPES|BLOCKENTITIES)\s*\.\s*register\s*\(\s*"([^"]+)"[\s\S]{0,2200}?BlockEntityType(?:\s*\.\s*Builder)?\s*\.\s*of\s*\(\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*::\s*new\s*,\s*([^)]*)\)"#
    ).expect("static block-entity registration regex");
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();

    for path in java_files(roots) {
        let Ok(source) = fs::read_to_string(&path) else { continue };
        for capture in registration.captures_iter(&source) {
            let Some(raw_id) = capture.get(1).map(|value| value.as_str()) else { continue };
            let Some(raw_class) = capture.get(2).map(|value| value.as_str()) else { continue };
            let raw_blocks = capture.get(3).map(|value| value.as_str()).unwrap_or("");
            let registry_name = normalize_registry(raw_id);
            let source_class = simple_class(raw_class);
            let associated_blocks = mapped_block_ids(raw_blocks, blocks);
            let key = (registry_name.clone(), source_class.clone());
            if !seen.insert(key) { continue; }

            let mut warnings = Vec::new();
            if associated_blocks.is_empty() {
                warnings.push(format!(
                    "Block entity '{}' was registered but its ModBlocks reference did not match a discovered block; it was inventoried but not emitted.",
                    source_class
                ));
            }
            warnings.push(
                "Generated block entity preserves its registry and owning block only; custom NBT persistence, ticking, inventories, menus, capabilities and machine logic remain unported.".to_owned()
            );
            output.push(BlockEntitySourceRecord {
                registry_name,
                source_class,
                source_path: path.to_string_lossy().to_string(),
                associated_blocks,
                warnings,
            });
        }
    }

    output.sort_by(|a, b| a.registry_name.cmp(&b.registry_name).then(a.source_class.cmp(&b.source_class)));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn finds_block_entity_type_and_owning_block_from_neoforge_registration() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/main/java/demo")).unwrap();
        fs::write(root.join("src/main/java/demo/ModBlockEntities.java"), r#"
            public class ModBlockEntities {
                public static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITY_TYPES =
                    DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, MOD_ID);
                public static final Supplier<BlockEntityType<RubyMachineBlockEntity>> RUBY_MACHINE =
                    BLOCK_ENTITY_TYPES.register("ruby_machine", () -> BlockEntityType.Builder.of(
                        RubyMachineBlockEntity::new, ModBlocks.RUBY_MACHINE.get()
                    ).build(null));
            }
        "#).unwrap();

        let found = discover_block_entities(
            &[root.to_path_buf()],
            &[("ruby_machine".to_owned(), "RubyMachineBlock".to_owned())]
        ).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].registry_name, "ruby_machine");
        assert_eq!(found[0].source_class, "RubyMachineBlockEntity");
        assert_eq!(found[0].associated_blocks, vec!["ruby_machine"]);
    }
}
