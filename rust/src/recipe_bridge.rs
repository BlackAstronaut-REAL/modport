use anyhow::Context;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const VANILLA_BLOCK_NAMES: &[&str] = &[
    "air", "stone", "granite", "diorite", "andesite", "deepslate", "cobbled_deepslate",
    "netherrack", "end_stone", "dirt", "coarse_dirt", "rooted_dirt", "grass_block",
    "podzol", "sand", "red_sand", "gravel", "clay", "snow", "snow_block",
];
const VANILLA_BLOCK_SUFFIXES: &[&str] = &[
    "_block", "_ore", "_log", "_wood", "_planks", "_slab", "_stairs", "_wall",
    "_fence", "_fence_gate", "_door", "_trapdoor", "_button", "_pressure_plate",
    "_sign", "_hanging_sign", "_leaves", "_sapling", "_glass", "_pane", "_wool",
    "_terracotta", "_concrete", "_concrete_powder", "_bricks", "_stone",
    "_cobblestone", "_dirt", "_sand", "_gravel", "_ice", "_coral", "_coral_block",
    "_coral_fan", "_mushroom", "_stem", "_nylium", "_wart", "_torch", "_lantern",
    "_rail", "_bed", "_banner", "_head", "_skull", "_anvil", "_table", "_crop",
    "_roots", "_flower",
];

fn compact_registry(path: &str) -> String {
    path.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub fn looks_like_vanilla_block_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase().replace('-', "_");
    VANILLA_BLOCK_NAMES.contains(&path.as_str())
        || VANILLA_BLOCK_SUFFIXES.iter().any(|suffix| path.ends_with(suffix))
}

/// Map a Minecraft item/block identifier into an MCreator MItemBlock token.
pub fn recipe_ref(
    value: &Value,
    registry_to_element: &BTreeMap<String, String>,
    source_modid: &str,
    _target_modid: &str,
) -> String {
    let Some(raw) = value.as_str() else { return String::new() };
    let raw = raw.trim();
    if raw.is_empty() { return String::new(); }
    if raw.starts_with('#') { return raw.to_owned(); }
    if raw.starts_with("Items.") || raw.starts_with("Blocks.") { return raw.to_owned(); }
    let (namespace, path) = raw.split_once(':').unwrap_or(("minecraft", raw));
    if namespace.eq_ignore_ascii_case(source_modid) {
        let compact = compact_registry(path);
        return registry_to_element.get(&compact)
            .map(|name| format!("CUSTOM:{name}"))
            .unwrap_or_default();
    }
    if namespace.eq_ignore_ascii_case("minecraft") {
        let enum_name = path.to_ascii_uppercase().replace('/', "_").replace('-', "_");
        return format!("{}{enum_name}", if looks_like_vanilla_block_path(path) { "Blocks." } else { "Items." });
    }
    raw.to_owned()
}

/// Resolve a recipe output item and its returned stack count.
pub fn recipe_output_ref(
    recipe: &Value,
    registry_to_element: &BTreeMap<String, String>,
    source_modid: &str,
) -> (String, i64) {
    let raw = recipe.get("result").unwrap_or(&Value::Null);
    let (value, count) = if raw.is_object() {
        let value = raw.get("item").or_else(|| raw.get("id")).unwrap_or(&Value::Null);
        let count = raw.get("count").and_then(|value| {
            value.as_i64()
                .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
                .or_else(|| value.as_f64().map(|n| n as i64))
                .or_else(|| value.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
        }).unwrap_or(1).max(1);
        (value, count)
    } else {
        (raw, 1)
    };
    (recipe_ref(value, registry_to_element, source_modid, source_modid), count)
}

/// Resolve one ingredient expression, preferring its first mappable item alternative.
pub fn recipe_ingredient_ref(
    ingredient: &Value,
    registry_to_element: &BTreeMap<String, String>,
    source_modid: &str,
) -> String {
    if ingredient.is_string() {
        return recipe_ref(ingredient, registry_to_element, source_modid, source_modid);
    }
    if let Some(object) = ingredient.as_object() {
        if let Some(item) = object.get("item") {
            return recipe_ref(item, registry_to_element, source_modid, source_modid);
        }
        if let Some(values) = object.get("items").and_then(Value::as_array) {
            return values.iter()
                .map(|value| recipe_ref(value, registry_to_element, source_modid, source_modid))
                .find(|value| !value.is_empty())
                .unwrap_or_default();
        }
        if let Some(tag) = object.get("tag") {
            let tag = tag.as_str().map(str::to_owned).unwrap_or_else(|| {
                if tag.is_null() { String::new() } else { tag.to_string() }
            });
            return format!("#{tag}");
        }
    }
    if let Some(values) = ingredient.as_array() {
        return values.iter()
            .map(|value| recipe_ingredient_ref(value, registry_to_element, source_modid))
            .find(|value| !value.is_empty())
            .unwrap_or_default();
    }
    String::new()
}

fn base_definition(recipe_name: &str, output_ref: &str, output_count: i64, group: &str) -> Value {
    json!({
        "recipeType":"Crafting",
        "recipeRetstackSize":output_count,
        "group":group,
        "unlockingItems":[],
        "cookingBookCategory":"MISC",
        "xpReward":0.0,
        "cookingTime":200,
        "craftingBookCategory":"MISC",
        "recipeShapeless":false,
        "recipeSlots":["","","","","","","","",""],
        "recipeReturnStack":output_ref,
        "name":recipe_name,
        "namespace":"mod"
    })
}

/// Convert a resource recipe JSON object into an MCreator recipe definition.
/// Unsupported patterns/types and unmappable outputs are surfaced as warnings.
pub fn recipe_mcreator_definition(
    payload: &Value,
    recipe_name: &str,
    registry_to_element: &BTreeMap<String, String>,
    source_modid: &str,
) -> (Option<Value>, Vec<String>) {
    let rtype = payload.get("type").and_then(Value::as_str).unwrap_or("minecraft:crafting_shaped");
    let group = payload.get("group").and_then(Value::as_str).unwrap_or("");
    let (output_ref, output_count) = recipe_output_ref(payload, registry_to_element, source_modid);
    if output_ref.is_empty() {
        return (None, vec![format!(
            "Recipe {recipe_name}: result could not be mapped to an MCreator item/block reference."
        )]);
    }
    let mut definition = base_definition(recipe_name, &output_ref, output_count, group);

    if rtype.ends_with("crafting_shaped") {
        let pattern = payload.get("pattern").and_then(Value::as_array).cloned().unwrap_or_default();
        if pattern.len() > 3 {
            return (None, vec![format!(
                "Recipe {recipe_name}: shaped pattern larger than 3x3 is unsupported by MCreator."
            )]);
        }
        let key = payload.get("key").and_then(Value::as_object);
        if let Some(pattern) = pattern.as_slice().get(..) {
            for (row_index, row) in pattern.iter().enumerate() {
                let Some(row) = row.as_str() else { continue };
                for (column_index, symbol) in row.chars().take(3).enumerate() {
                    if symbol == ' ' { continue; }
                    let Some(ingredient) = key.and_then(|key| key.get(symbol.to_string().as_str())) else {
                        continue;
                    };
                    let reference = recipe_ingredient_ref(ingredient, registry_to_element, source_modid);
                    let slot = row_index * 3 + column_index;
                    if !reference.is_empty() && slot < 9 {
                        definition["recipeSlots"][slot] = Value::String(reference);
                    }
                }
            }
        }
        return (Some(definition), Vec::new());
    }

    if rtype.ends_with("crafting_shapeless") {
        definition["recipeShapeless"] = Value::Bool(true);
        if let Some(ingredients) = payload.get("ingredients").and_then(Value::as_array) {
            for (index, ingredient) in ingredients.iter().take(9).enumerate() {
                definition["recipeSlots"][index] = Value::String(
                    recipe_ingredient_ref(ingredient, registry_to_element, source_modid)
                );
            }
        }
        return (Some(definition), Vec::new());
    }

    let recipe_type = match rtype {
        "minecraft:smelting" => Some("Smelting"),
        "minecraft:blasting" => Some("Blasting"),
        "minecraft:smoking" => Some("Smoking"),
        "minecraft:campfire_cooking" => Some("Campfire cooking"),
        "minecraft:stonecutting" => Some("Stone cutting"),
        "minecraft:smithing" | "minecraft:smithing_transform" | "minecraft:smithing_trim" => Some("Smithing"),
        _ => None,
    };
    if let Some(recipe_type) = recipe_type {
        definition["recipeType"] = Value::String(recipe_type.to_owned());
        let ingredient = recipe_ingredient_ref(
            payload.get("ingredient").unwrap_or(&Value::Null),
            registry_to_element,
            source_modid,
        );
        let (input, returned) = match recipe_type {
            "Smelting" => ("smeltingInputStack", "smeltingReturnStack"),
            "Blasting" => ("blastingInputStack", "blastingReturnStack"),
            "Smoking" => ("smokingInputStack", "smokingReturnStack"),
            "Campfire cooking" => ("campfireCookingInputStack", "campfireCookingReturnStack"),
            "Stone cutting" => ("stoneCuttingInputStack", "stoneCuttingReturnStack"),
            _ => ("", ""),
        };
        if recipe_type == "Smithing" {
            definition["smithingInputStack"] = Value::String(recipe_ingredient_ref(
                payload.get("addition").unwrap_or(&Value::Null), registry_to_element, source_modid
            ));
            definition["smithingInputAdditionStack"] = Value::String(ingredient);
            definition["smithingInputTemplateStack"] = Value::String(recipe_ingredient_ref(
                payload.get("template").unwrap_or(&Value::Null), registry_to_element, source_modid
            ));
            definition["smithingReturnStack"] = Value::String(output_ref);
        } else {
            definition[input] = Value::String(ingredient);
            definition[returned] = Value::String(output_ref);
        }
        return (Some(definition), Vec::new());
    }

    (None, vec![format!(
        "Recipe {recipe_name}: recipe type '{rtype}' is not representable by MCreator's native recipe element."
    )])
}

/// Locate JSON recipe resources under standard assets/data trees.
pub fn recipe_source_files(source_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for root in source_roots {
        if !root.exists() { continue; }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if !entry.file_type().is_file() || !entry.path().extension().and_then(|v| v.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json")) {
                continue;
            }
            let path = entry.path();
            let lower = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            if !lower.contains("/recipes/") || !(lower.contains("/assets/") || lower.contains("/data/")) {
                continue;
            }
            let key = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().to_lowercase();
            if seen.insert(key) { candidates.push(path.to_path_buf()); }
        }
    }
    candidates.sort_by_key(|path| (path.components().count(), path.to_string_lossy().to_ascii_lowercase()));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_map() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("ironingot".to_owned(), "IronIngot".to_owned()),
            ("sword".to_owned(), "IronSword".to_owned()),
        ])
    }

    #[test]
    fn maps_custom_and_vanilla_item_block_references() {
        let map = item_map();
        assert_eq!(recipe_ref(&json!("demo:iron_ingot"), &map, "demo", "converted"), "CUSTOM:IronIngot");
        assert_eq!(recipe_ref(&json!("minecraft:stick"), &map, "demo", "converted"), "Items.STICK");
        assert_eq!(recipe_ref(&json!("minecraft:stone"), &map, "demo", "converted"), "Blocks.STONE");
        assert_eq!(recipe_ref(&json!("other:some_item"), &map, "demo", "converted"), "other:some_item");
        assert_eq!(recipe_ref(&json!("demo:missing"), &map, "demo", "converted"), "");
        assert_eq!(recipe_ref(&json!("#minecraft:planks"), &map, "demo", "converted"), "#minecraft:planks");
    }

    #[test]
    fn maps_shaped_recipe_grid_and_output_count() {
        let payload = json!({
            "type":"minecraft:crafting_shaped",
            "group":"tools",
            "pattern":[" I "," I "," S "],
            "key":{"I":{"item":"demo:iron_ingot"},"S":{"item":"minecraft:stick"}},
            "result":{"item":"demo:sword","count":2}
        });
        let (definition,warnings) = recipe_mcreator_definition(&payload, "IronSwordRecipe", &item_map(), "demo");
        let definition = definition.unwrap();
        assert!(warnings.is_empty());
        assert_eq!(definition["recipeReturnStack"], "CUSTOM:IronSword");
        assert_eq!(definition["recipeRetstackSize"], 2);
        assert_eq!(definition["group"], "tools");
        assert_eq!(definition["recipeSlots"][1], "CUSTOM:IronIngot");
        assert_eq!(definition["recipeSlots"][4], "CUSTOM:IronIngot");
        assert_eq!(definition["recipeSlots"][7], "Items.STICK");
        assert_eq!(definition["recipeSlots"][0], "");
    }

    #[test]
    fn maps_shapeless_and_cooking_recipe_kinds() {
        let shapeless = json!({
            "type":"minecraft:crafting_shapeless","ingredients":[{"tag":"forge:ingots/iron"},"demo:iron_ingot"],
            "result":{"item":"demo:sword"}
        });
        let (definition,_) = recipe_mcreator_definition(&shapeless,"MixedRecipe",&item_map(),"demo");
        let definition = definition.unwrap();
        assert_eq!(definition["recipeShapeless"], true);
        assert_eq!(definition["recipeSlots"][0], "#forge:ingots/iron");
        assert_eq!(definition["recipeSlots"][1], "CUSTOM:IronIngot");

        let cooking = json!({
            "type":"minecraft:smelting","ingredient":{"item":"demo:iron_ingot"},
            "result":{"item":"demo:sword","count":1}
        });
        let (definition,_) = recipe_mcreator_definition(&cooking,"SmeltRecipe",&item_map(),"demo");
        let definition = definition.unwrap();
        assert_eq!(definition["recipeType"], "Smelting");
        assert_eq!(definition["smeltingInputStack"], "CUSTOM:IronIngot");
        assert_eq!(definition["smeltingReturnStack"], "CUSTOM:IronSword");
    }

    #[test]
    fn rejects_oversized_patterns_and_unrepresentable_recipe_types() {
        let oversized = json!({
            "pattern":["XXXX","XXXX","XXXX","XXXX"],
            "key":{"X":{"item":"minecraft:stick"}},
            "result":{"item":"demo:sword"}
        });
        let (definition,warnings) = recipe_mcreator_definition(&oversized,"TooLarge",&item_map(),"demo");
        assert!(definition.is_none());
        assert!(warnings[0].contains("larger than 3x3"));

        let unsupported = json!({
            "type":"minecraft:smithing_transform_plus",
            "result":{"item":"demo:sword"}
        });
        let (definition,warnings) = recipe_mcreator_definition(&unsupported,"Unsupported",&item_map(),"demo");
        assert!(definition.is_none());
        assert!(warnings[0].contains("not representable"));
    }

    #[test]
    fn discovers_recipe_resources_across_assets_and_data_roots() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let data = root.join("data/demo/recipes/tools/sword.json");
        let assets = root.join("assets/demo/recipes/legacy.json");
        fs::create_dir_all(data.parent().unwrap()).unwrap();
        fs::create_dir_all(assets.parent().unwrap()).unwrap();
        fs::write(&data,b"{}").unwrap();
        fs::write(&assets,b"{}").unwrap();
        let paths = recipe_source_files(&[root.to_path_buf()]);
        assert_eq!(paths.len(),2);
        assert!(paths.iter().any(|path| path.ends_with("tools/sword.json")));
        assert!(paths.iter().any(|path| path.ends_with("legacy.json")));
    }
}


#[derive(Debug, Clone, serde::Serialize)]
pub struct RecipeElementExport {
    pub element_name: String,
    pub registry_name: String,
    pub source_path: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecipeExportReport {
    pub exported: Vec<RecipeElementExport>,
    pub warnings: Vec<String>,
    pub copied_recipe_resources: Vec<String>,
}

fn snake_name(value: &str) -> String {
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
    let result = result.trim_matches('_').to_owned();
    if result.is_empty() { "converted_content".to_owned() } else { result }
}

fn pascal_name(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut words = Vec::<String>::new();
    let mut current = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !current.is_empty() { words.push(std::mem::take(&mut current)); }
            continue;
        }
        if ch.is_ascii_uppercase() && !current.is_empty() {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower) {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(ch);
    }
    if !current.is_empty() { words.push(current); }
    let mut result = String::new();
    for word in words {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.push(first.to_ascii_uppercase());
            result.extend(chars);
        }
    }
    if result.is_empty() { result = "ConvertedContent".to_owned(); }
    if result.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        result.insert_str(0, "Content");
    }
    result
}

fn unique_element_name(base: &str, used: &mut std::collections::BTreeSet<String>) -> String {
    let candidate = if base.is_empty() { "ConvertedContent" } else { base };
    let mut name = candidate.to_owned();
    let mut suffix = 2usize;
    while !used.insert(name.to_ascii_lowercase()) {
        name = format!("{candidate}{suffix}");
        suffix += 1;
    }
    name
}

fn import_root_name(modid: &str) -> String {
    let name = modid.trim().replace('-', "_");
    if name.eq_ignore_ascii_case("lostinfinity") || name.eq_ignore_ascii_case("lost_infinity") {
        "LostInfinity".to_owned()
    } else if name.is_empty() {
        "converted_mod".to_owned()
    } else {
        name
    }
}

fn recipe_import_path(modid: &str) -> String {
    format!("~/{}/Recipes", import_root_name(modid))
}

fn recipe_relative_path(path: &Path) -> Option<PathBuf> {
    let components: Vec<_> = path.components().collect();
    let index = components.iter().position(|part| {
        part.as_os_str().to_string_lossy().eq_ignore_ascii_case("recipes")
    })?;
    let relative: PathBuf = components.iter().skip(index + 1).collect();
    if relative.as_os_str().is_empty() { None } else { Some(relative) }
}

fn read_recipe_object(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    value.as_object()?;
    Some(value)
}

/// Emit supported recipe JSON resources as native MCreator recipe elements and
/// append generated descriptors to the loaded workspace definition.
pub fn emit_recipe_elements(
    source_roots: &[PathBuf],
    output_elements: &Path,
    workspace: &mut Value,
    source_modid: &str,
    registry_to_element: &BTreeMap<String, String>,
    imported_fv: i64,
) -> anyhow::Result<RecipeExportReport> {
    let mut report = RecipeExportReport {
        exported: Vec::new(),
        warnings: Vec::new(),
        copied_recipe_resources: Vec::new(),
    };
    let mut used_names = std::collections::BTreeSet::<String>::new();
    let mut existing = workspace.get("mod_elements").and_then(Value::as_array)
        .cloned().unwrap_or_default();
    for item in &existing {
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            used_names.insert(name.to_ascii_lowercase());
        }
    }
    let mut descriptors = Vec::new();
    let mut emitted_registries = std::collections::BTreeSet::<String>::new();
    fs::create_dir_all(output_elements)?;

    for source in recipe_source_files(source_roots) {
        let Some(payload) = read_recipe_object(&source) else { continue };
        let Some(relative) = recipe_relative_path(&source) else { continue };
        let Some(stem) = relative.file_stem().and_then(|value| value.to_str()) else { continue };
        let registry_name = snake_name(stem);
        let element_name = unique_element_name(&format!("{}Recipe", pascal_name(stem)), &mut used_names);
        let (definition, warnings) = recipe_mcreator_definition(
            &payload, &registry_name, registry_to_element, source_modid
        );
        let Some(definition) = definition else {
            report.warnings.extend(warnings);
            continue;
        };

        let version = imported_fv.clamp(0, 89);
        let element = json!({"_fv":version,"_type":"recipe","definition":definition});
        let output = output_elements.join(format!("{element_name}.mod.json"));
        fs::write(&output, format!("{}\n", serde_json::to_string_pretty(&element)?))?;
        descriptors.push(json!({
            "name":element_name,
            "type":"recipe",
            "compiles":true,
            "locked_code":false,
            "registry_name":registry_name,
            "metadata":{"files":[]},
            "path":recipe_import_path(source_modid),
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":source_modid,
            "mojmodelbridge_source_registry":registry_name
        }));
        emitted_registries.insert(registry_name.to_ascii_lowercase());
        report.exported.push(RecipeElementExport {
            element_name,
            registry_name,
            source_path:source.to_string_lossy().replace('\\', "/"),
            warnings,
        });
    }

    let field_map = source_field_registered_names(source_roots, source_modid);
    for recovered in java_recipe_payloads(source_roots, source_modid, &field_map, registry_to_element) {
        let registry_name = snake_name(&recovered.name);
        if emitted_registries.contains(&registry_name.to_ascii_lowercase()) {
            continue;
        }
        let (definition, warnings) = recipe_mcreator_definition(
            &recovered.payload,
            &registry_name,
            registry_to_element,
            source_modid,
        );
        let Some(definition) = definition else {
            report.warnings.extend(warnings);
            continue;
        };
        let element_name = unique_element_name(
            &format!("{}Recipe", pascal_name(&registry_name)),
            &mut used_names,
        );
        let element = json!({"_fv":imported_fv.clamp(0, 89),"_type":"recipe","definition":definition});
        let output = output_elements.join(format!("{element_name}.mod.json"));
        fs::write(&output, format!("{}\n", serde_json::to_string_pretty(&element)?))?;
        descriptors.push(json!({
            "name":element_name,
            "type":"recipe",
            "compiles":true,
            "locked_code":false,
            "registry_name":registry_name,
            "metadata":{"files":[]},
            "path":recipe_import_path(source_modid),
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":source_modid,
            "mojmodelbridge_source_registry":registry_name
        }));
        emitted_registries.insert(registry_name.to_ascii_lowercase());
        report.exported.push(RecipeElementExport {
            element_name,
            registry_name,
            source_path:"<Java recipe registration>".to_owned(),
            warnings:recovered.warnings,
        });
    }

    if !workspace.is_object() {
        *workspace = Value::Object(serde_json::Map::new());
    }
    if !workspace.get("mod_elements").is_some_and(Value::is_array) {
        workspace["mod_elements"] = Value::Array(std::mem::take(&mut existing));
    }
    if let Some(elements) = workspace.get_mut("mod_elements").and_then(Value::as_array_mut) {
        elements.extend(descriptors);
    }
    Ok(report)
}

/// Copy recipe JSON into the target data pack while retaining its path below recipes.
pub fn copy_recipe_resources(
    source_roots: &[PathBuf],
    workspace_root: &Path,
    target_modid: &str,
) -> anyhow::Result<Vec<String>> {
    let target_root = workspace_root.join("src/main/resources/data")
        .join(target_modid).join("recipes");
    let mut outputs = Vec::new();
    for source in recipe_source_files(source_roots) {
        let Some(relative) = recipe_relative_path(&source) else { continue };
        let Some(payload) = read_recipe_object(&source) else { continue };
        let target = target_root.join(relative);
        if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
        fs::write(&target, format!("{}\n", serde_json::to_string_pretty(&payload)?))?;
        outputs.push(target.to_string_lossy().replace('\\', "/"));
    }
    Ok(outputs)
}


#[cfg(test)]
mod export_tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn emits_recipe_element_and_appends_workspace_descriptor() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let recipe = source.join("data/demo/recipes/tools/sword.json");
        fs::create_dir_all(recipe.parent().unwrap()).unwrap();
        fs::write(&recipe, serde_json::to_vec(&json!({
            "type":"minecraft:crafting_shaped",
            "pattern":[" I "," I "," S "],
            "key":{"I":{"item":"demo:iron_ingot"},"S":{"item":"minecraft:stick"}},
            "result":{"item":"demo:sword","count":1}
        })).unwrap()).unwrap();

        let mut workspace = json!({
            "mod_elements":[{"name":"ExistingItem","type":"item","registry_name":"existing"}]
        });
        let map = BTreeMap::from([
            ("ironingot".to_owned(),"IronIngot".to_owned()),
            ("sword".to_owned(),"IronSword".to_owned())
        ]);
        let output = temp.path().join("workspace/elements");
        let report = emit_recipe_elements(
            &[source], &output, &mut workspace, "demo", &map, 89
        ).unwrap();

        assert_eq!(report.exported.len(), 1);
        assert!(report.warnings.is_empty());
        assert_eq!(report.exported[0].element_name, "SwordRecipe");
        assert_eq!(workspace["mod_elements"].as_array().unwrap().len(), 2);
        let descriptor = &workspace["mod_elements"][1];
        assert_eq!(descriptor["type"], "recipe");
        assert_eq!(descriptor["path"], "~/demo/Recipes");
        assert_eq!(descriptor["mojmodelbridge_source_registry"], "sword");

        let element_path = output.join("SwordRecipe.mod.json");
        let element: Value = serde_json::from_slice(&fs::read(element_path).unwrap()).unwrap();
        assert_eq!(element["_fv"], 89);
        assert_eq!(element["_type"], "recipe");
        assert_eq!(element["definition"]["recipeReturnStack"], "CUSTOM:IronSword");
        assert_eq!(element["definition"]["recipeSlots"][1], "CUSTOM:IronIngot");
    }

    #[test]
    fn preserves_recipe_subfolders_and_normalizes_copied_json() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let recipe = source.join("assets/demo/recipes/tools/sword.json");
        fs::create_dir_all(recipe.parent().unwrap()).unwrap();
        fs::write(&recipe, br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stick"],"result":{"item":"minecraft:stone"}}"#).unwrap();
        let workspace = temp.path().join("workspace");

        let outputs = copy_recipe_resources(&[source], &workspace, "converted").unwrap();
        assert_eq!(outputs.len(), 1);
        let target = workspace.join("src/main/resources/data/converted/recipes/tools/sword.json");
        assert!(target.is_file());
        let copied: Value = serde_json::from_slice(&fs::read(target).unwrap()).unwrap();
        assert_eq!(copied["result"]["item"], "minecraft:stone");
        assert_eq!(copied["type"], "minecraft:crafting_shapeless");
    }

    #[test]
    fn emitted_names_are_unique_against_existing_workspace_elements() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let recipe = source.join("data/demo/recipes/sword.json");
        fs::create_dir_all(recipe.parent().unwrap()).unwrap();
        fs::write(&recipe, br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stick"],"result":{"item":"minecraft:stone"}}"#).unwrap();
        let mut workspace = json!({"mod_elements":[{"name":"SwordRecipe"}]});
        let report = emit_recipe_elements(
            &[source], &temp.path().join("elements"), &mut workspace, "demo", &BTreeMap::new(), 100
        ).unwrap();
        assert_eq!(report.exported[0].element_name, "SwordRecipe2");
        let path = temp.path().join("elements/SwordRecipe2.mod.json");
        let element: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(element["_fv"], 89);
    }
}


/// Load a workspace, emit recipe elements, copy recipe data and persist its descriptor.
pub fn export_recipes_to_workspace(
    source_roots: &[PathBuf],
    workspace_root: &Path,
    source_modid: &str,
    registry_to_element: &BTreeMap<String, String>,
    imported_fv: i64,
) -> anyhow::Result<RecipeExportReport> {
    let mut candidates = Vec::<(i64, usize, PathBuf, Value)>::new();
    for entry in fs::read_dir(workspace_root)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("mcreator") {
            continue;
        }
        let Some(value) = read_recipe_object(&path) else { continue };
        if !value.get("workspaceSettings").is_some_and(Value::is_object) {
            continue;
        }
        let version = value.get("mcreatorVersion").and_then(Value::as_i64).unwrap_or(0);
        let count = value.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        candidates.push((version, count, path, value));
    }
    candidates.sort_by_key(|(version, count, path, _)| (*version, *count, path.to_string_lossy().to_lowercase()));
    let Some((_, _, descriptor_path, mut workspace)) = candidates.pop() else {
        anyhow::bail!("No readable .mcreator workspace definition found in {}", workspace_root.display());
    };

    let report = emit_recipe_elements(
        source_roots,
        &workspace_root.join("elements"),
        &mut workspace,
        source_modid,
        registry_to_element,
        imported_fv,
    )?;
    let target_modid = workspace.get("workspaceSettings")
        .and_then(|settings| settings.get("modid"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(source_modid);
    let mut report = report;
    report.copied_recipe_resources = copy_recipe_resources(source_roots, workspace_root, target_modid)?;
    fs::write(&descriptor_path, format!("{}\n", serde_json::to_string_pretty(&workspace)?))
        .with_context(|| format!("Writing updated workspace descriptor {}", descriptor_path.display()))?;
    Ok(report)
}


#[cfg(test)]
mod workspace_export_tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn exports_recipe_resources_and_updates_selected_workspace_file() {
        let temp = tempdir().unwrap();
        let workspace_root = temp.path().join("workspace");
        let source_root = temp.path().join("source");
        fs::create_dir_all(&workspace_root).unwrap();
        let recipe = source_root.join("data/demo/recipes/stone_from_dirt.json");
        fs::create_dir_all(recipe.parent().unwrap()).unwrap();
        fs::write(&recipe, br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:dirt"],"result":{"item":"minecraft:stone"}}"#).unwrap();

        let workspace_file = workspace_root.join("test.mcreator");
        fs::write(&workspace_file, serde_json::to_vec(&json!({
            "mcreatorVersion":202600,
            "workspaceSettings":{"modid":"converted"},
            "mod_elements":[]
        })).unwrap()).unwrap();

        let report = export_recipes_to_workspace(
            &[source_root], &workspace_root, "demo", &BTreeMap::new(), 89
        ).unwrap();

        assert_eq!(report.exported.len(), 1);
        assert_eq!(report.exported[0].element_name, "StoneFromDirtRecipe");
        assert_eq!(report.copied_recipe_resources.len(), 1);
        let descriptor: Value = serde_json::from_slice(&fs::read(workspace_file).unwrap()).unwrap();
        assert_eq!(descriptor["mod_elements"][0]["registry_name"], "stone_from_dirt");
        assert_eq!(descriptor["mod_elements"][0]["path"], "~/demo/Recipes");
        assert!(workspace_root.join("elements/StoneFromDirtRecipe.mod.json").is_file());
        assert!(workspace_root.join("src/main/resources/data/converted/recipes/stone_from_dirt.json").is_file());
    }
}


#[derive(Debug, Clone, serde::Serialize)]
pub struct JavaRecipePayload {
    pub name: String,
    pub payload: Value,
    pub warnings: Vec<String>,
}

fn normalize_java_item_ref(expression: &str) -> String {
    let mut expression = expression.trim().trim_matches('"').trim_matches('\'').to_owned();
    if let Some(path) = expression.strip_prefix("minecraft:block/") {
        return format!("minecraft:{path}");
    }
    if let Some(path) = expression.strip_prefix("minecraft:item/") {
        return format!("minecraft:{path}");
    }
    if let Some(tail) = expression.strip_prefix("Items.").or_else(|| expression.strip_prefix("items.")) {
        let mut tail = tail.to_owned();
        for prefix in ["ITEMS.", "items."] {
            while let Some(next) = tail.strip_prefix(prefix) {
                tail = next.to_owned();
            }
        }
        return format!("Items.{}", tail.to_ascii_uppercase().replace('-', "_"));
    }
    if let Some(tail) = expression.strip_prefix("Blocks.").or_else(|| expression.strip_prefix("blocks.")) {
        let mut tail = tail.to_owned();
        for prefix in ["BLOCKS.", "blocks."] {
            while let Some(next) = tail.strip_prefix(prefix) {
                tail = next.to_owned();
            }
        }
        return format!("Blocks.{}", tail.to_ascii_uppercase().replace('-', "_"));
    }
    expression = expression.trim().to_owned();
    expression
}

fn java_call_arguments(source: &str, wanted: &[&str]) -> Vec<(String, String)> {
    use crate::source::java::{extract_balanced, strip_java_comments};

    let source = strip_java_comments(source);
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'"' || bytes[index] == b'\'' {
            let quote = bytes[index];
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else if bytes[index] == quote {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' || bytes[index] == b'$' {
            let name_start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' || bytes[index] == b'$')
            {
                index += 1;
            }
            let name = &source[name_start..index];
            if !wanted.contains(&name) {
                continue;
            }
            let mut open = index;
            while open < bytes.len() && bytes[open].is_ascii_whitespace() {
                open += 1;
            }
            if open >= bytes.len() || bytes[open] != b'(' {
                continue;
            }
            if let Ok((arguments, end)) = extract_balanced(&source, open, '(', ')') {
                result.push((name.to_owned(), arguments));
                index = end;
            }
            continue;
        }
        index += 1;
    }
    result
}

fn java_string_literals(value: &str) -> Vec<String> {
    use crate::source::java::unquote_java_string;

    let bytes = value.as_bytes();
    let mut literals = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
            } else if bytes[index] == b'"' {
                index += 1;
                break;
            } else {
                index += 1;
            }
        }
        literals.push(unquote_java_string(&value[start..index]));
    }
    literals
}

fn terminal_java_identifier(expression: &str) -> String {
    let trimmed = expression.trim().trim_end_matches(|ch: char| ch == ')' || ch == '(');
    let bytes = trimmed.as_bytes();
    let mut start = bytes.len();
    while start > 0 {
        let ch = bytes[start - 1];
        if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'$' {
            start -= 1;
        } else {
            break;
        }
    }
    trimmed[start..].to_owned()
}

/// Read registry names associated with static item/block fields from Java registration calls.
pub fn source_field_registered_names(
    source_roots: &[PathBuf],
    source_modid: &str,
) -> BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut java_files = Vec::new();
    for root in source_roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("java"))
            {
                java_files.push(entry.into_path());
            }
        }
    }
    java_files.sort();

    let mut combined = String::new();
    for path in java_files {
        if let Ok(bytes) = fs::read(path) {
            combined.push_str(&String::from_utf8_lossy(&bytes));
            combined.push('\n');
        }
    }

    let mut field_map = BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    use crate::source::java::split_top_level;
    for (_call, raw_args) in java_call_arguments(&combined, &["registerItem", "registerBlock"]) {
        let args = split_top_level(&raw_args, ',');
        if args.len() < 2 {
            continue;
        }
        let field = terminal_java_identifier(&args[0]);
        if field.is_empty() {
            continue;
        }
        let literals = java_string_literals(&args[1]);
        let Some(raw_registry) = literals.last() else {
            continue;
        };
        let registry = if let Some((namespace, path)) = raw_registry.split_once(':') {
            if !namespace.eq_ignore_ascii_case(source_modid) {
                continue;
            }
            path
        } else {
            raw_registry.as_str()
        };
        let registry = snake_name(registry);
        if !registry.is_empty() {
            field_map.entry(field).or_default().insert(registry);
        }
    }
    field_map
}

/// Resolve one Java ItemStack/Ingredient expression to an MCreator item token.
pub fn java_recipe_item_ref(
    expression: &str,
    field_map: &BTreeMap<String, std::collections::BTreeSet<String>>,
    registry_to_element: &BTreeMap<String, String>,
    source_modid: &str,
) -> String {
    use crate::source::java::split_top_level;

    let mut expression = normalize_java_item_ref(expression);
    for wrapper in ["new ItemStack", "Ingredient.of"] {
        if let Some(position) = expression.find(wrapper) {
            if let Some(open_rel) = expression[position..].find('(') {
                let open = position + open_rel;
                if let Ok((arguments, _)) = crate::source::java::extract_balanced(&expression, open, '(', ')') {
                    if let Some(first) = split_top_level(&arguments, ',').first() {
                        expression = normalize_java_item_ref(first);
                    }
                }
            }
        }
    }

    if let Some(position) = expression.to_ascii_lowercase().find("createvanillaspawnegg") {
        if let Some(open_rel) = expression[position..].find('(') {
            let open = position + open_rel;
            if let Ok((arguments, _)) = crate::source::java::extract_balanced(&expression, open, '(', ')') {
                if let Some(value) = split_top_level(&arguments, ',').first() {
                    let entity = value.trim().trim_matches('"').trim_matches('\'');
                    if !entity.is_empty() {
                        let path = entity.split(':').last().unwrap_or(entity);
                        return format!("Items.{}_SPAWN_EGG", path.to_ascii_uppercase().replace('-', "_"));
                    }
                }
            }
        }
    }

    let normalized = normalize_java_item_ref(&expression);
    if normalized == "AIR" {
        return "Items.AIR".to_owned();
    }
    let parts: Vec<&str> = normalized.split('.').collect();
    if let Some(name) = parts.last().copied() {
        if !name.is_empty()
            && parts.iter().all(|part| {
                !part.is_empty() && part.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
            })
        {
            if let Some(registries) = field_map.get(name) {
                for registry in registries {
                    if let Some(element) = registry_to_element.get(&compact_registry(registry)) {
                        return format!("CUSTOM:{element}");
                    }
                }
                if let Some(registry) = registries.iter().next() {
                    return format!("CUSTOM:{registry}");
                }
            }
            if normalized.starts_with("Items.") {
                return format!("Items.{}", name.to_ascii_uppercase());
            }
            if normalized.starts_with("Blocks.") {
                return format!("Blocks.{}#0", name.to_ascii_uppercase());
            }
        }
    }

    if normalized.starts_with("Items.") || normalized.starts_with("Blocks.") {
        return normalized;
    }
    let _ = source_modid;
    String::new()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct JavaRecipePayload {
    pub name: String,
    pub payload: Value,
    pub warnings: Vec<String>,
}

/// Recover addShapedRecipe/addShapelessRecipe Java helper calls when recipe JSON is absent.
pub fn java_recipe_payloads(
    source_roots: &[PathBuf],
    source_modid: &str,
    field_map: &BTreeMap<String, std::collections::BTreeSet<String>>,
    registry_to_element: &BTreeMap<String, String>,
) -> Vec<JavaRecipePayload> {
    let mut java_files = Vec::new();
    for root in source_roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("java"))
            {
                java_files.push(entry.into_path());
            }
        }
    }
    java_files.sort();

    let mut combined = String::new();
    for path in java_files {
        if let Ok(bytes) = fs::read(path) {
            combined.push_str(&String::from_utf8_lossy(&bytes));
            combined.push('\n');
        }
    }

    use crate::source::java::{split_top_level, unquote_java_string};
    let mut output = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (call_name, raw_args) in java_call_arguments(&combined, &["addShapelessRecipe", "addShapedRecipe"]) {
        let args = split_top_level(&raw_args, ',');
        if args.len() < 4 {
            continue;
        }

        let first_arg = &args[0];
        let recipe_id = if let Some(position) = first_arg.find("cpId") {
            let tail = &first_arg[position..];
            let Some(start) = tail.find('"') else { continue };
            let literal = &tail[start..];
            let Some(end) = literal[1..].find('"') else { continue };
            unquote_java_string(&literal[..end + 2])
        } else {
            let literals = java_string_literals(first_arg);
            literals.into_iter().next().unwrap_or_default()
        };
        let name = snake_name(&recipe_id);
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }

        let output_ref = java_recipe_item_ref(&args[2], field_map, registry_to_element, source_modid);
        if output_ref.is_empty() {
            continue;
        }

        let mut warnings = Vec::new();
        let payload = if call_name == "addShapelessRecipe" {
            let mut ingredients = Vec::new();
            for argument in args.iter().skip(3) {
                let reference = java_recipe_item_ref(argument, field_map, registry_to_element, source_modid);
                if !reference.is_empty() {
                    ingredients.push(json!({"item":reference}));
                }
            }
            if ingredients.is_empty() {
                continue;
            }
            json!({
                "type":"minecraft:crafting_shapeless",
                "ingredients":ingredients,
                "result":{"item":output_ref,"count":1}
            })
        } else {
            let mut patterns = Vec::new();
            let mut index = 3usize;
            while index < args.len() {
                let value = args[index].trim();
                if value.starts_with('"') && value.ends_with('"') {
                    let decoded = unquote_java_string(value);
                    if !decoded.is_empty() && decoded.chars().count() <= 3 {
                        patterns.push(decoded);
                        index += 1;
                        continue;
                    }
                }
                break;
            }

            let mut key = serde_json::Map::new();
            while index + 1 < args.len() {
                let token = args[index].trim();
                let value = args[index + 1].trim();
                index += 2;
                let Some(start) = token.find('\'') else {
                    continue;
                };
                let Some(end_rel) = token[start + 1..].find('\'') else {
                    continue;
                };
                let character = &token[start + 1..start + 1 + end_rel];
                let reference = java_recipe_item_ref(value, field_map, registry_to_element, source_modid);
                if !reference.is_empty() {
                    key.insert(character.to_owned(), json!({"item":reference}));
                }
            }
            if patterns.is_empty() || key.is_empty() {
                continue;
            }
            json!({
                "type":"minecraft:crafting_shaped",
                "pattern":patterns,
                "key":key,
                "result":{"item":output_ref,"count":1}
            })
        };
        output.push(JavaRecipePayload { name, payload, warnings:std::mem::take(&mut warnings) });
    }
    output
}

#[cfg(test)]
mod java_recipe_tests {
    use super::*;

    #[test]
    fn normalizes_wrapped_vanilla_and_custom_java_recipe_references() {
        let field_map = BTreeMap::from([
            ("IRON_INGOT".to_owned(), std::collections::BTreeSet::from(["iron_ingot".to_owned()]))
        ]);
        let registry = BTreeMap::from([("ironingot".to_owned(),"IronIngot".to_owned())]);
        assert_eq!(java_recipe_item_ref("Items.ITEMS.STICK", &field_map, &registry, "demo"), "Items.STICK");
        assert_eq!(java_recipe_item_ref("new ItemStack(Blocks.BLOCKS.STONE, 1)", &field_map, &registry, "demo"), "Blocks.STONE#0");
        assert_eq!(java_recipe_item_ref("Ingredient.of(Items.IRON_INGOT)", &field_map, &registry, "demo"), "CUSTOM:IronIngot");
        assert_eq!(java_recipe_item_ref(r#"Items.createVanillaSpawnEgg("minecraft:zombie")"#, &field_map, &registry, "demo"), "Items.ZOMBIE_SPAWN_EGG");
    }

    #[test]
    fn recovers_java_shaped_and_shapeless_recipe_helper_calls() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("Recipes.java"), r#"
            // addShapelessRecipe(cpId("ignored"), 0, Items.STICK, Items.STONE);
            public void register() {
              addShapelessRecipe(cpId("magic_dust"), 0, Items.STICK,
                  Items.REDSTONE, Ingredient.of(Items.GLOWSTONE_DUST));
              addShapedRecipe(cpId("test_sword"), 0, Items.DIAMOND_SWORD,
                  " D ", " D ", " S ", 'D', Items.DIAMOND, 'S', Items.STICK);
            }
        "#).unwrap();
        let field_map = BTreeMap::new();
        let registry = BTreeMap::new();
        let recovered = java_recipe_payloads(&[source], "demo", &field_map, &registry);
        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].name, "magic_dust");
        assert_eq!(recovered[0].payload["type"], "minecraft:crafting_shapeless");
        assert_eq!(recovered[1].name, "test_sword");
        assert_eq!(recovered[1].payload["type"], "minecraft:crafting_shaped");
        assert_eq!(recovered[1].payload["pattern"][0], " D ");
        assert_eq!(recovered[1].payload["key"]["D"]["item"], "Items.DIAMOND");
    }

    #[test]
    fn derives_custom_field_registry_names_from_registration_calls() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("ModItems.java"), r#"
            public final class ModItems {
                public static void register() {
                    registerItem(ModItems.IRON_INGOT, "demo:iron_ingot");
                    registerBlock(ModBlocks.COPPER_ORE, "copper_ore");
                }
            }
        "#).unwrap();
        let fields = source_field_registered_names(&[source], "demo");
        assert_eq!(fields["IRON_INGOT"].iter().next().unwrap(), "iron_ingot");
        assert_eq!(fields["COPPER_ORE"].iter().next().unwrap(), "copper_ore");
    }

    #[test]
    fn emitter_recovers_java_recipes_when_resource_json_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("ModItems.java"), r#"
            public class ModItems {
                public static void register() {
                    registerItem(ModItems.IRON_INGOT, "iron_ingot");
                    addShapelessRecipe(cpId("iron_dust"), 0, ModItems.IRON_INGOT,
                        Items.REDSTONE, Ingredient.of(Items.GLOWSTONE_DUST));
                }
            }
        "#).unwrap();
        let mut workspace = serde_json::json!({"mod_elements":[]});
        let registry_map = BTreeMap::from([("ironingot".to_owned(), "IronIngot".to_owned())]);
        let output = temp.path().join("elements");
        let report = emit_recipe_elements(&[source], &output, &mut workspace, "demo", &registry_map, 89).unwrap();
        assert_eq!(report.exported.len(), 1);
        assert_eq!(report.exported[0].registry_name, "iron_dust");
        let element: Value = serde_json::from_slice(&std::fs::read(output.join("IronDustRecipe.mod.json")).unwrap()).unwrap();
        assert_eq!(element["definition"]["recipeReturnStack"], "CUSTOM:IronIngot");
        assert_eq!(element["definition"]["recipeSlots"][0], "Items.REDSTONE");
        assert_eq!(element["definition"]["recipeSlots"][1], "Items.GLOWSTONE_DUST");
    }
}
