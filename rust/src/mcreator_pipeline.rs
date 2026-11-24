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
    pub validation_ok: bool,
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
        if kind == "armor" {
            if let Some(piece_ids) = descriptor.get("mojmodelbridge_source_piece_ids").and_then(Value::as_object) {
                for piece_registry in piece_ids.values().filter_map(Value::as_str) {
                    let compact_piece = compact_registry(piece_registry);
                    if !compact_piece.is_empty() {
                        map.insert(compact_piece, name.to_owned());
                    }
                }
            }
        }
    }
    Ok(map)
}

fn count_field(value: &Value, field: &str) -> usize {
    match value.get(field) {
        Some(Value::Array(items)) => items.len(),
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0) as usize,
        _ => 0,
    }
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

    let graph_root = source_roots.first()
        .ok_or_else(|| anyhow::anyhow!("export-all requires at least one source root"))?;
    let graph = crate::mod_graph::scan_mod_graph(graph_root, source_roots)?;
    let workspace_bytes = fs::read(&workspace_file)
        .with_context(|| format!("Reading {}", workspace_file.display()))?;
    let workspace_descriptor: Value = serde_json::from_slice(&workspace_bytes)
        .context("Parsing workspace generator settings")?;
    let target_generator = workspace_descriptor
        .get("workspaceSettings")
        .and_then(|settings| settings.get("currentGenerator"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("neoforge-1.21.1")
        .to_owned();
    let entities = crate::mcreator_workspace::export_living_entities_to_workspace_with_modid(
        &graph, &workspace_root, &target_generator, false, Some(&import_modid)
    )?;
    let entity_details = serde_json::to_value(&entities)?;
    if let Some(items) = entity_details.get("warnings").and_then(Value::as_array) {
        warnings.extend(items.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    stages.push(stage(
        "living_entities",
        count_field(&entity_details, "exported_entities"),
        count_field(&entity_details, "skipped_entities"),
        entity_details,
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

    let validation = crate::workspace_validation::validate_workspace(
        &workspace_root,
        None,
        true,
    )?;
    let validation_details = serde_json::to_value(&validation)?;
    let validation_ok = validation_details.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if let Some(errors) = validation_details.get("errors").and_then(Value::as_array) {
        for error in errors {
            let code = error.get("code").and_then(Value::as_str).unwrap_or("validation_error");
            let message = error.get("message").and_then(Value::as_str).unwrap_or("Workspace validation error");
            warnings.push(format!("[{code}] {message}"));
        }
    }
    if let Some(items) = validation_details.get("warnings").and_then(Value::as_array) {
        for warning in items {
            let code = warning.get("code").and_then(Value::as_str).unwrap_or("validation_warning");
            let message = warning.get("message").and_then(Value::as_str).unwrap_or("Workspace validation warning");
            warnings.push(format!("[{code}] {message}"));
        }
    }
    stages.push(stage(
        "validation",
        0,
        0,
        validation_details,
    ));

    Ok(ExportAllReport {
        workspace_file,
        source_modid: import_modid,
        validation_ok,
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
                {"name":"RubyArmor","type":"armor","registry_name":"ruby_armor",
                 "mojmodelbridge_source_piece_ids":{"helmet":"ruby_helmet","body":"ruby_chestplate","leggings":"ruby_leggings","boots":"ruby_boots"}},
                {"name":"OtherElement","type":"procedure","registry_name":"other"}
            ]
        })).unwrap()).unwrap();

        let map = registry_element_map(&descriptor).unwrap();
        assert_eq!(map["rubyingot"], "RubyIngot");
        assert_eq!(map["rubypickaxe"], "RubyPickaxe");
        assert_eq!(map["rubyhelmet"], "RubyArmor");
        assert_eq!(map["rubychestplate"], "RubyArmor");
        assert_eq!(map["rubyleggings"], "RubyArmor");
        assert_eq!(map["rubyboots"], "RubyArmor");
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
    fn export_all_initializes_every_stage_for_an_empty_source_tree() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let source = temp.path().join("source");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(workspace.join("test.mcreator"), serde_json::to_vec(&json!({
            "mcreatorVersion":202600,
            "workspaceSettings":{"modid":"converted"},
            "mod_elements":[],
            "language_map":{"en_us":{}}
        })).unwrap()).unwrap();

        let report = export_all_to_workspace(&[source], &workspace, "demo", 89).unwrap();
        assert_eq!(report.stages.iter().map(|stage| stage.name.as_str()).collect::<Vec<_>>(), vec![
            "equipment","armor","blocks","plants","worldgen","living_entities","projectiles","recipes","validation"
        ]);
        assert!(report.stages.iter().all(|stage| stage.exported == 0 && stage.skipped == 0));
        assert!(report.workspace_file.ends_with("test.mcreator"));
    }

    #[test]
    fn export_all_runs_all_stages_on_an_empty_source_workspace() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source_mod");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        let descriptor = workspace.join("converted.mcreator");
        fs::write(&descriptor, serde_json::to_vec(&json!({
            "mcreatorVersion": 202600,
            "workspaceSettings": {
                "modid": "converted",
                "modElementsPackage": "net.converted",
                "currentGenerator": "neoforge-1.21.1"
            },
            "mod_elements": [],
            "language_map": {"en_us": {}}
        })).unwrap()).unwrap();

        let report = export_all_to_workspace(
            &[source],
            &workspace,
            "examplemod",
            89
        ).unwrap();

        let names: Vec<&str> = report.stages.iter().map(|stage| stage.name.as_str()).collect();
        assert_eq!(names, vec![
            "equipment", "armor", "blocks", "plants",
            "worldgen", "living_entities", "projectiles", "recipes", "validation"
        ]);
        assert!(report.stages.iter().all(|stage| stage.exported == 0 && stage.skipped == 0));
        assert_eq!(report.source_modid, "examplemod");
        assert!(report.validation_ok);
        assert_eq!(report.workspace_file, fs::canonicalize(&descriptor).unwrap());
    }

    #[test]
    fn count_field_supports_numeric_and_record_array_reports() {
        assert_eq!(count_field(&json!({"exported":4}), "exported"), 4);
        assert_eq!(count_field(&json!({"exported":[{},{}]}), "exported"), 2);
        assert_eq!(count_field(&json!({"exported":null}), "exported"), 0);
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
