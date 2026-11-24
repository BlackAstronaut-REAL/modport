use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ExportStageResult {
    pub name: String,
    pub exported: usize,
    pub skipped: usize,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportAllReport {
    pub workspace_file: PathBuf,
    pub source_modid: String,
    pub stages: Vec<ExportStageResult>,
    pub warnings: Vec<String>,
}

fn selected_workspace(root: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::<(i64, usize, PathBuf)>::new();
    for entry in fs::read_dir(root).with_context(|| format!("Reading workspace root {}", root.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("mcreator") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
        if !value.get("workspaceSettings").is_some_and(Value::is_object)
            || !value.get("mod_elements").is_some_and(Value::is_array) {
            continue;
        }
        let version = value.get("mcreatorVersion").and_then(Value::as_i64).unwrap_or(0);
        let count = value.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        candidates.push((version, count, path));
    }
    candidates.sort_by_key(|(version,count,path)| (*version,*count,path.to_string_lossy().to_ascii_lowercase()));
    candidates.pop().map(|(_,_,path)| path)
        .ok_or_else(|| anyhow::anyhow!("No valid .mcreator workspace descriptor found in {}", root.display()))
}

fn compact_registry(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn registry_element_map(workspace_file: &Path) -> Result<BTreeMap<String,String>> {
    let bytes = fs::read(workspace_file).with_context(|| format!("Reading {}", workspace_file.display()))?;
    let workspace: Value = serde_json::from_slice(&bytes).context("Parsing workspace descriptor")?;
    let mut map = BTreeMap::new();
    for descriptor in workspace.get("mod_elements").and_then(Value::as_array).into_iter().flatten() {
        let kind = descriptor.get("type").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "item" | "tool" | "block" | "plant" | "armor") {
            continue;
        }
        let registry = descriptor.get("registry_name").and_then(Value::as_str).unwrap_or("");
        let name = descriptor.get("name").and_then(Value::as_str).unwrap_or("");
        let compact = compact_registry(registry);
        if !compact.is_empty() && !name.is_empty() {
            map.insert(compact, name.to_owned());
        }
    }
    Ok(map)
}

fn count_field(value: &Value, field: &str) -> usize {
    value.get(field).and_then(Value::as_u64).unwrap_or(0) as usize
}

fn stage(name: &str, exported: usize, skipped: usize, details: Value) -> ExportStageResult {
    ExportStageResult {
        name: name.to_owned(),
        exported,
        skipped,
        details,
    }
}

/// Run the native workspace exporters in a dependency-aware order.
/// This intentionally uses the same stage functions as the individual CLI commands.
pub fn export_all_to_workspace(
    source_roots: &[PathBuf],
    workspace_root: &Path,
    source_modid: &str,
    imported_fv: i64,
) -> Result<ExportAllReport> {
    let workspace_root = fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let workspace_file = selected_workspace(&workspace_root)?;
    let import_modid = if !source_modid.trim().is_empty() {
        source_modid.trim().to_owned()
    } else {
        source_roots.first().and_then(|root| root.file_name()).and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("converted_mod")
            .to_owned()
    };

    let mut stages = Vec::<ExportStageResult>::new();
    let mut warnings = Vec::<String>::new();

    let equipment = crate::mcreator_equipment::export_equipment_to_workspace(
        source_roots, &workspace_root, &import_modid, imported_fv
    )?;
    warnings.extend(equipment.warnings.iter().cloned());
    for record in &equipment.records {
        warnings.extend(record.warnings.iter().map(|warning| format!("{}: {warning}", record.class_name)));
    }
    stages.push(stage(
        "equipment",
        equipment.exported,
        equipment.skipped,
        serde_json::to_value(&equipment)?,
    ));

    let armor = crate::mcreator_armor::export_armor_to_workspace(
        source_roots, &workspace_root, &import_modid, imported_fv
    )?;
    warnings.extend(armor.warnings.iter().cloned());
    for record in &armor.records {
        warnings.extend(record.warnings.iter().map(|warning| format!("{}: {warning}", record.prefix)));
    }
    stages.push(stage(
        "armor",
        armor.exported,
        armor.skipped,
        serde_json::to_value(&armor)?,
    ));

    let blocks = crate::mcreator_block::export_blocks_to_workspace(
        source_roots, &workspace_root, &import_modid, imported_fv
    )?;
    warnings.extend(blocks.warnings.iter().cloned());
    for record in &blocks.records {
        warnings.extend(record.warnings.iter().map(|warning| format!("{}: {warning}", record.class_name)));
    }
    stages.push(stage(
        "blocks",
        blocks.exported,
        blocks.skipped,
        serde_json::to_value(&blocks)?,
    ));

    let plants = crate::mcreator_plant::export_plants_to_workspace(
        source_roots, &workspace_root, &import_modid, imported_fv
    )?;
    warnings.extend(plants.warnings.iter().cloned());
    for record in &plants.records {
        warnings.extend(record.warnings.iter().map(|warning| format!("{}: {warning}", record.class_name)));
    }
    stages.push(stage(
        "plants",
        plants.exported,
        plants.skipped,
        serde_json::to_value(&plants)?,
    ));

    let worldgen = crate::mcreator_worldgen::export_worldgen_to_workspace(
        source_roots, &workspace_root, &import_modid, imported_fv
    )?;
    warnings.extend(worldgen.warnings.iter().cloned());
    for record in &worldgen.records {
        warnings.extend(record.warnings.iter().map(|warning| format!("{}: {warning}", record.registry_name)));
    }
    stages.push(stage(
        "worldgen",
        worldgen.biomes_exported + worldgen.dimensions_exported,
        worldgen.skipped,
        serde_json::to_value(&worldgen)?,
    ));

    let entities = crate::mcreator_workspace::export_living_entities_to_workspace(
        source_roots, &workspace_root, &import_modid, imported_fv
    )?;
    warnings.extend(entities.warnings.iter().cloned());
    stages.push(stage(
        "living_entities",
        entities.exported,
        entities.skipped,
        serde_json::to_value(&entities)?,
    ));

    let projectiles = crate::mcreator_projectile::export_projectiles_to_workspace(
        source_roots, &workspace_root, &import_modid, imported_fv
    )?;
    warnings.extend(projectiles.warnings.iter().cloned());
    for record in &projectiles.records {
        warnings.extend(record.warnings.iter().map(|warning| format!("{}: {warning}", record.class_name)));
    }
    stages.push(stage(
        "projectiles",
        projectiles.exported,
        projectiles.skipped,
        serde_json::to_value(&projectiles)?,
    ));

    // Recipe references are resolved only after item/tool/block elements exist.
    let registry_to_element = registry_element_map(&workspace_file)?;
    let recipes = crate::recipe_bridge::export_recipes_to_workspace(
        source_roots, &workspace_root, &import_modid, &registry_to_element, imported_fv
    )?;
    warnings.extend(recipes.warnings.iter().cloned());
    for record in &recipes.exported {
        warnings.extend(record.warnings.iter().map(|warning| format!("{}: {warning}", record.element_name)));
    }
    stages.push(stage(
        "recipes",
        recipes.exported.len(),
        0,
        serde_json::to_value(&recipes)?,
    ));

    Ok(ExportAllReport {
        workspace_file,
        source_modid: import_modid,
        stages,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn builds_compact_recipe_registry_map_from_existing_workspace_elements() {
        let temp = tempdir().unwrap();
        let descriptor = temp.path().join("test.mcreator");
        fs::write(&descriptor, serde_json::to_vec(&json!({
            "workspaceSettings":{"modid":"converted"},
            "mod_elements":[
                {"name":"RubyIngot","type":"item","registry_name":"ruby_ingot"},
                {"name":"RubyPickaxe","type":"tool","registry_name":"ruby_pickaxe"},
                {"name":"OtherElement","type":"procedure","registry_name":"other"}
            ]
        })).unwrap()).unwrap();

        let map = registry_element_map(&descriptor).unwrap();
        assert_eq!(map["rubyingot"], "RubyIngot");
        assert_eq!(map["rubypickaxe"], "RubyPickaxe");
        assert!(!map.contains_key("other"));
    }

    #[test]
    fn selects_highest_version_valid_workspace_descriptor() {
        let temp = tempdir().unwrap();
        let low = temp.path().join("a.mcreator");
        let high = temp.path().join("b.mcreator");
        fs::write(&low, serde_json::to_vec(&json!({
            "mcreatorVersion":202500,"workspaceSettings":{"modid":"x"},"mod_elements":[]
        })).unwrap()).unwrap();
        fs::write(&high, serde_json::to_vec(&json!({
            "mcreatorVersion":202600,"workspaceSettings":{"modid":"x"},"mod_elements":[]
        })).unwrap()).unwrap();
        assert_eq!(selected_workspace(temp.path()).unwrap(), high);
    }

    #[test]
    fn stage_serialization_keeps_skipped_and_exported_counts() {
        let value = json!({"exported":3,"skipped":2});
        let result = stage("blocks", count_field(&value,"exported"), count_field(&value,"skipped"), value);
        assert_eq!(result.name, "blocks");
        assert_eq!(result.exported, 3);
        assert_eq!(result.skipped, 2);
    }
}
