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
        report.exported.push(RecipeElementExport {
            element_name,
            registry_name,
            source_path:source.to_string_lossy().replace('\\', "/"),
            warnings,
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
