//! Native NeoForge datapack recipe materialization.
//!
//! Converts recipe JSON resources and legacy Java recipe helper calls into
//! Minecraft 1.21.1 recipe JSON under data/<modid>/recipes.

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::recipe_bridge::{java_recipe_payloads, recipe_source_files, source_field_registered_names};

#[derive(Debug, Clone, Serialize)]
pub struct NativeRecipeReport {
    pub generated: Vec<String>,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn export_native_recipes(
    source_roots: &[PathBuf],
    resources_root: &Path,
    source_modid: &str,
    target_modid: &str,
) -> Result<NativeRecipeReport> {
    let recipe_root = resources_root.join("data").join(target_modid).join("recipes");
    fs::create_dir_all(&recipe_root)?;

    let mut report = NativeRecipeReport { generated: Vec::new(), skipped: Vec::new(), warnings: Vec::new() };
    let mut generated_keys = BTreeSet::new();

    for source in recipe_source_files(source_roots) {
        let Some(relative) = recipe_relative_path(&source) else {
            report.skipped.push(source.to_string_lossy().into_owned());
            continue;
        };
        let Some(key) = safe_recipe_key(&relative) else {
            report.warnings.push(format!("Skipped recipe with unsafe relative path: {}", source.display()));
            continue;
        };
        let bytes = match fs::read(&source) {
            Ok(bytes) => bytes,
            Err(error) => {
                report.warnings.push(format!("Could not read recipe {}: {error}", source.display()));
                continue;
            }
        };
        let parsed = serde_json::from_slice::<Value>(&bytes);
        let Ok(payload) = parsed else {
            report.warnings.push(format!("Skipped invalid recipe JSON: {}", source.display()));
            continue;
        };
        if !payload.is_object() {
            report.warnings.push(format!("Skipped non-object recipe JSON: {}", source.display()));
            continue;
        }

        if !generated_keys.insert(key.clone()) {
            report.warnings.push(format!("Duplicate recipe path '{}' found; retained the first sorted source.", key.display()));
            continue;
        }

        let namespace = source_namespace(&source).unwrap_or_else(|| source_modid.to_owned());
        let migrated = migrate_recipe_payload(payload, &namespace, target_modid, &mut report.warnings);
        let destination = recipe_root.join(&relative);
        write_recipe(&destination, &migrated)?;
        report.generated.push(format!(
            "data/{target_modid}/recipes/{}",
            relative.to_string_lossy().replace('\\', "/")
        ));
    }

    // Recover common 1.7/1.12 register-helper recipes when no JSON file was
    // shipped. The shared Java parser supplies field-to-registry relationships.
    let field_map = source_field_registered_names(source_roots, source_modid);
    let empty_element_map = std::collections::BTreeMap::new();
    for recipe in java_recipe_payloads(source_roots, source_modid, &field_map, &empty_element_map) {
        let key = safe_recipe_key(Path::new(&format!("{}.json", recipe.name)));
        let Some(key) = key else { continue };
        if !generated_keys.insert(key.clone()) {
            report.skipped.push(format!("Java recipe '{}' duplicates an existing JSON recipe", recipe.name));
            continue;
        }
        let payload = rewrite_java_recipe_refs(recipe.payload, target_modid);
        let payload = migrate_recipe_payload(payload, source_modid, target_modid, &mut report.warnings);
        let destination = recipe_root.join(&key);
        write_recipe(&destination, &payload)?;
        report.generated.push(format!(
            "data/{target_modid}/recipes/{}",
            key.to_string_lossy().replace('\\', "/")
        ));
        report.warnings.extend(recipe.warnings.into_iter().map(|warning| format!("{}: {warning}", recipe.name)));
    }

    report.generated.sort();
    report.skipped.sort();
    report.warnings.sort();
    Ok(report)
}

fn recipe_relative_path(path: &Path) -> Option<PathBuf> {
    let components: Vec<_> = path.components().collect();
    let index = components.iter().position(|component| {
        component.as_os_str().to_string_lossy().eq_ignore_ascii_case("recipes")
    })?;
    let relative: PathBuf = components.iter().skip(index + 1).collect();
    if relative.as_os_str().is_empty() { None } else { Some(relative) }
}

fn safe_recipe_key(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() { return None; }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => result.push(value),
            _ => return None,
        }
    }
    if result.extension().and_then(|value| value.to_str()).is_none_or(|value| !value.eq_ignore_ascii_case("json")) {
        return None;
    }
    Some(result)
}

fn source_namespace(path: &Path) -> Option<String> {
    let components: Vec<String> = path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        }).collect();
    let recipe = components.iter().position(|part| part.eq_ignore_ascii_case("recipes"))?;
    if recipe >= 2 && (components[recipe - 2].eq_ignore_ascii_case("assets")
        || components[recipe - 2].eq_ignore_ascii_case("data")) {
        Some(components[recipe - 1].clone())
    } else {
        None
    }
}

fn write_recipe(path: &Path, payload: &Value) -> Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(payload)?))
        .with_context(|| format!("writing NeoForge recipe {}", path.display()))
}

fn migrate_recipe_payload(
    value: Value,
    source_modid: &str,
    target_modid: &str,
    warnings: &mut Vec<String>,
) -> Value {
    match value {
        Value::Object(mut object) => {
            // Namespace references are rewritten throughout ingredient, result,
            // predicate and advancement-like nested fields without changing unrelated IDs.
            for (key, child) in object.iter_mut() {
                *child = migrate_recipe_payload(std::mem::take(child), source_modid, target_modid, warnings);
                if key == "result" {
                    if let Value::Object(result) = child {
                        if !result.contains_key("id") {
                            if let Some(item) = result.remove("item") {
                                result.insert("id".to_owned(), item);
                            }
                        }
                    }
                }
            }

            if let Some(Value::String(kind)) = object.get_mut("type") {
                if kind.eq_ignore_ascii_case("forge:ore_shaped") {
                    *kind = "minecraft:crafting_shaped".to_owned();
                    warnings.push("Migrated forge:ore_shaped to minecraft:crafting_shaped.".to_owned());
                } else if kind.eq_ignore_ascii_case("forge:ore_shapeless") {
                    *kind = "minecraft:crafting_shapeless".to_owned();
                    warnings.push("Migrated forge:ore_shapeless to minecraft:crafting_shapeless.".to_owned());
                }
            }

            if object.get("type").and_then(Value::as_str).is_some_and(|kind| kind.eq_ignore_ascii_case("forge:ore_dict")) {
                let ore = object.get("ore").and_then(Value::as_str).unwrap_or("");
                let tag = ore_dictionary_tag(ore);
                let mut ingredient = Map::new();
                ingredient.insert("tag".to_owned(), json!(tag));
                warnings.push(format!("Migrated Forge ore-dictionary ingredient '{ore}' to tag '{tag}'."));
                return Value::Object(ingredient);
            }

            Value::Object(object)
        }
        Value::Array(values) => Value::Array(values.into_iter()
            .map(|value| migrate_recipe_payload(value, source_modid, target_modid, warnings))
            .collect()),
        Value::String(text) => Value::String(rewrite_namespace(&text, source_modid, target_modid)),
        other => other,
    }
}

fn ore_dictionary_tag(ore: &str) -> String {
    let mappings = [
        ("ingot", "ingots"), ("nugget", "nuggets"), ("gem", "gems"),
        ("dust", "dusts"), ("ore", "ores"), ("block", "storage_blocks"),
        ("plank", "planks"), ("log", "logs"), ("slab", "slabs"),
        ("stair", "stairs"), ("stone", "stones"),
    ];
    for (prefix, category) in mappings {
        if let Some(material) = ore.strip_prefix(prefix) {
            if !material.is_empty() {
                return format!("c:{category}/{}", camel_to_snake(material));
            }
        }
    }
    if ore.eq_ignore_ascii_case("stickWood") { return "c:rods/wooden".to_owned(); }
    if ore.eq_ignore_ascii_case("treeSapling") { return "c:saplings".to_owned(); }
    format!("c:legacy_ore_dict/{}", camel_to_snake(ore))
}

fn camel_to_snake(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut result = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            if previous.is_ascii_lowercase() || previous.is_ascii_digit() { result.push('_'); }
        }
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else if !result.is_empty() && !result.ends_with('_') {
            result.push('_');
        }
    }
    result.trim_matches('_').to_owned()
}

fn rewrite_namespace(value: &str, source_modid: &str, target_modid: &str) -> String {
    let source = source_modid.trim();
    let target = target_modid.trim();
    if source.is_empty() || target.is_empty() || source.eq_ignore_ascii_case(target) { return value.to_owned(); }
    if let Some(path) = value.strip_prefix(&format!("{source}:")) {
        return format!("{target}:{path}");
    }
    if let Some(path) = value.strip_prefix(&format!("#{source}:")) {
        return format!("#{target}:{path}");
    }
    value.to_owned()
}

fn rewrite_java_recipe_refs(value: Value, target_modid: &str) -> Value {
    match value {
        Value::String(text) => {
            if let Some(registry) = text.strip_prefix("CUSTOM:") {
                Value::String(format!("{target_modid}:{}", camel_to_snake(registry)))
            } else if let Some(path) = text.strip_prefix("Items.") {
                Value::String(format!("minecraft:{}", path.to_ascii_lowercase()))
            } else if let Some(path) = text.strip_prefix("Blocks.") {
                let path = path.split('#').next().unwrap_or(path);
                Value::String(format!("minecraft:{}", path.to_ascii_lowercase()))
            } else {
                Value::String(text)
            }
        }
        Value::Array(values) => Value::Array(values.into_iter().map(|value| rewrite_java_recipe_refs(value, target_modid)).collect()),
        Value::Object(values) => Value::Object(values.into_iter().map(|(key, value)| {
            (key, rewrite_java_recipe_refs(value, target_modid))
        }).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn migrates_forge_ore_dictionary_recipe_json_and_result_schema() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("source");
        fs::create_dir_all(root.join("assets/oldmod/recipes")).unwrap();
        fs::write(root.join("assets/oldmod/recipes/ruby.json"), br#"{
            "type":"forge:ore_shaped",
            "pattern":["I"],
            "key":{"I":{"type":"forge:ore_dict","ore":"ingotIron"}},
            "result":{"item":"oldmod:ruby","count":2}
        }"#).unwrap();

        let report = export_native_recipes(
            &[root], &temp.path().join("resources"), "oldmod", "newmod"
        ).unwrap();
        assert_eq!(report.generated.len(), 1);
        let output: Value = serde_json::from_slice(
            &fs::read(temp.path().join("resources/data/newmod/recipes/ruby.json")).unwrap()
        ).unwrap();
        assert_eq!(output["type"], "minecraft:crafting_shaped");
        assert_eq!(output["key"]["I"]["tag"], "c:ingots/iron");
        assert_eq!(output["result"]["id"], "newmod:ruby");
        assert!(output["result"].get("item").is_none());
    }

    #[test]
    fn recovers_java_helper_recipe_calls_into_modern_datapack_json() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("source");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Items.java"), r#"
            public class Items {
                public static final Item RUBY_DUST = new RubyDust();
                public static final Item RUBY_SWORD = new RubySword();
                public static void register() {
                    GameRegistry.registerItem(RUBY_DUST, "ruby_dust");
                    GameRegistry.registerItem(RUBY_SWORD, "ruby_sword");
                }
                public void recipes() {
                    addShapelessRecipe(cpId("ruby_sword_from_dust"), 0, RUBY_SWORD, RUBY_DUST, Items.REDSTONE);
                }
            }
            public class RubyDust extends Item {}
            public class RubySword extends Item {}
        "#).unwrap();

        let report = export_native_recipes(
            &[root], &temp.path().join("resources"), "source", "converted"
        ).unwrap();
        assert_eq!(report.generated.len(), 1);
        let output: Value = serde_json::from_slice(
            &fs::read(temp.path().join("resources/data/converted/recipes/ruby_dust_from_redstone.json")).unwrap()
        ).unwrap();
        assert_eq!(output["type"], "minecraft:crafting_shapeless");
        assert_eq!(output["ingredients"][0]["item"], "converted:ruby_dust");
        assert_eq!(output["ingredients"][1]["item"], "minecraft:redstone");
        assert_eq!(output["result"]["id"], "converted:ruby_sword");
    }
}
