use regex::Regex;
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Default)]
pub struct ToolMaterialStats {
    pub uses: Option<i64>,
    pub efficiency: Option<f64>,
    pub damage: Option<f64>,
    pub enchantability: Option<i64>,
}

fn item_display_name(registry: &str) -> String {
    let normalized = registry.replace(['-', '.'], "_");
    let chars: Vec<char> = normalized.chars().collect();
    let mut words = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower) {
                words.push(' ');
            }
        }
        if ch == '_' || ch.is_whitespace() {
            words.push(' ');
        } else {
            words.push(ch);
        }
    }
    words.split_whitespace().map(|word| {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => {
                let mut value = first.to_ascii_uppercase().to_string();
                value.extend(chars.map(|ch| ch.to_ascii_lowercase()));
                value
            }
            None => String::new(),
        }
    }).collect::<Vec<_>>().join(" ")
}

fn template_definition(template: Option<&Value>) -> Map<String, Value> {
    let Some(template) = template else { return Map::new() };
    template.get("definition")
        .and_then(Value::as_object)
        .cloned()
        .or_else(|| template.as_object().cloned())
        .unwrap_or_default()
}

fn template_version(template: Option<&Value>, fallback: i64) -> i64 {
    template.and_then(|value| value.get("_fv")).and_then(Value::as_i64)
        .unwrap_or(fallback).clamp(0, 89)
}

fn sanitize_item_template(definition: &mut Map<String, Value>, kind: &str) {
    definition.insert("renderType".to_owned(), json!(0));
    definition.insert("customModelName".to_owned(), json!("Normal"));
    for key in ["texture", "guiTexture", "itemTexture", "particleTexture"] {
        if definition.contains_key(key) {
            definition.insert(key.to_owned(), Value::String(String::new()));
        }
    }
    if kind == "tool" {
        if definition.contains_key("blockingRenderType") {
            definition.insert("blockingRenderType".to_owned(), json!(0));
        }
        if definition.contains_key("blockingModelName") {
            definition.insert("blockingModelName".to_owned(), json!("Normal blocking"));
        }
    }
}

fn regex_number(text: &str, patterns: &[&str]) -> Option<f64> {
    for pattern in patterns {
        let regex = Regex::new(pattern).ok()?;
        if let Some(captures) = regex.captures(text) {
            if let Some(value) = captures.get(1).and_then(|matched| matched.as_str().parse::<f64>().ok()) {
                if value.is_finite() {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn integer_after_call(text: &str, names: &[&str]) -> Option<i64> {
    for name in names {
        let pattern = format!(r"\b{}\s*\(\s*(-?\d+)\s*\)", regex::escape(name));
        if let Some(value) = regex_number(text, &[&pattern]) {
            return Some(value as i64);
        }
    }
    None
}

fn source_field_number(text: &str, names: &[&str]) -> Option<f64> {
    for name in names {
        let pattern = format!(r"\b{}\s*=\s*(-?\d+(?:\.\d+)?)[fFdD]?\s*;", regex::escape(name));
        if let Some(value) = regex_number(text, &[&pattern]) {
            return Some(value);
        }
    }
    None
}

fn method_return_number(text: &str, names: &[&str]) -> Option<f64> {
    for name in names {
        let pattern = format!(
            r"\b{}\s*\([^)]*\)\s*\{{\s*return\s+(-?\d+(?:\.\d+)?)[fFdD]?\s*;",
            regex::escape(name)
        );
        if let Some(value) = regex_number(text, &[&pattern]) {
            return Some(value);
        }
    }
    None
}

fn food_updates(text: &str, superclass: &str) -> Map<String, Value> {
    let mut updates = Map::new();
    let lower = superclass.to_ascii_lowercase();
    let modern_food = text.contains("FoodProperties.Builder") || text.contains(".food(");
    let legacy_food = lower.contains("itemfood") || lower.contains("itemseedfood");
    if !modern_food && !legacy_food {
        return updates;
    }

    updates.insert("isFood".to_owned(), json!(true));
    if let Some(value) = regex_number(text, &[r"\.nutrition\s*\(\s*(\d+)\s*\)"]) {
        updates.insert("nutritionalValue".to_owned(), json!(value as i64));
    } else if legacy_food {
        let super_call = Regex::new(r"super\s*\(([^;]*)\)").unwrap();
        if let Some(captures) = super_call.captures(text) {
            let arguments = crate::source::java::split_top_level(
                captures.get(1).map(|m| m.as_str()).unwrap_or(""), ','
            );
            if lower.contains("itemseedfood") && arguments.len() >= 2 {
                if let Ok(value) = arguments[arguments.len() - 2].trim().parse::<i64>() {
                    updates.insert("nutritionalValue".to_owned(), json!(value));
                }
                if let Ok(value) = arguments[arguments.len() - 1].trim().trim_end_matches(|ch| ch == 'f' || ch == 'F').parse::<f64>() {
                    if value.is_finite() { updates.insert("saturation".to_owned(), json!(value)); }
                }
            } else if arguments.len() >= 2 {
                if let Ok(value) = arguments[0].trim().parse::<i64>() {
                    updates.insert("nutritionalValue".to_owned(), json!(value));
                }
                if let Ok(value) = arguments[1].trim().trim_end_matches(['f','F']).parse::<f64>() {
                    if value.is_finite() { updates.insert("saturation".to_owned(), json!(value)); }
                }
            }
        }
    }

    if let Some(value) = regex_number(text, &[r"\.saturationMod\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?\s*\)"]) {
        updates.insert("saturation".to_owned(), json!(value));
    }
    if text.contains(".alwaysEat()") || text.contains("setAlwaysEdible") {
        updates.insert("isAlwaysEdible".to_owned(), json!(true));
    }
    if text.contains(".meat()") {
        updates.insert("isMeat".to_owned(), json!(true));
    }
    updates
}

fn ranged_updates(text: &str, definition: &Map<String, Value>) -> Map<String, Value> {
    let mut updates = Map::new();
    let is_bow = text.contains("UseAnim.BOW") || Regex::new(r"(?s)getUseAnimation\s*\([^)]*\)\s*\{[^{}]*BOW")
        .is_ok_and(|regex| regex.is_match(text));
    let is_crossbow = text.contains("UseAnim.CROSSBOW") || Regex::new(r"(?s)getUseAnimation\s*\([^)]*\)\s*\{[^{}]*CROSSBOW")
        .is_ok_and(|regex| regex.is_match(text));
    if !is_bow && !is_crossbow {
        return updates;
    }

    updates.insert("enableRanged".to_owned(), json!(true));
    updates.insert("shootConstantly".to_owned(), json!(false));
    updates.insert("rangedItemChargesPower".to_owned(), json!(true));
    updates.insert("projectileDisableAmmoCheck".to_owned(), json!(false));
    updates.insert("animation".to_owned(), json!(if is_bow { "bow" } else { "crossbow" }));
    updates.insert("projectile".to_owned(), json!("Arrow"));
    if definition.contains_key("useDuration") {
        if let Some(duration) = regex_number(text, &[r"\bgetUseDuration\s*\([^)]*\)\s*\{\s*return\s+(\d+)\s*;"]) {
            updates.insert("useDuration".to_owned(), json!(duration as i64));
        }
    }
    if let Ok(regex) = Regex::new(r"\bnew\s+([A-Za-z_$][A-Za-z0-9_$]*(?:Arrow|Projectile|Bolt|Shot))\s*\(") {
        if let Some(captures) = regex.captures(text) {
            if let Some(class_name) = captures.get(1) {
                updates.insert("mojmodelbridge_source_projectile_class".to_owned(), json!(class_name.as_str()));
            }
        }
    }
    updates
}

fn finish_element(
    kind: &str,
    template: Option<&Value>,
    imported_fv: i64,
    _name: &str,
    registry: &str,
    mut updates: Map<String, Value>,
) -> Value {
    let mut definition = template_definition(template);
    let display = item_display_name(registry);
    updates.insert("name".to_owned(), json!(display));
    let default_tabs = if kind == "tool" {
        if matches!(updates.get("toolType").and_then(Value::as_str), Some("Sword" | "Bow")) {
            json!(["COMBAT"])
        } else {
            json!(["TOOLS"])
        }
    } else {
        json!(["MISC"])
    };
    updates.entry("creativeTabs".to_owned()).or_insert(default_tabs);
    for (key, value) in updates {
        if key == "name" && !definition.contains_key("name") {
            continue;
        }
        definition.insert(key, value);
    }
    json!({
        "_fv":template_version(template, imported_fv),
        "_type":kind,
        "definition":definition
    })
}

/// Build an MCreator item element from recovered source facts and a template element.
pub fn build_item_element(
    template: Option<&Value>,
    _simple_class: &str,
    superclass: &str,
    source_text: &str,
    registry: &str,
    texture: &str,
    imported_fv: i64,
) -> (Value, Vec<String>) {
    let mut definition = template_definition(template);
    sanitize_item_template(&mut definition, "item");
    let stack = integer_after_call(source_text, &["func_77625_d", "setMaxStackSize"])
        .unwrap_or(64).clamp(1, 64);
    let durability = integer_after_call(source_text, &["func_77656_e", "setMaxDamage"])
        .unwrap_or(0).max(0);

    let mut updates = Map::new();
    updates.insert("name".to_owned(), json!(item_display_name(registry)));
    updates.insert("texture".to_owned(), json!(texture));
    updates.insert("stackSize".to_owned(), json!(stack));
    updates.insert("damageCount".to_owned(), json!(durability));
    updates.insert("creativeTabs".to_owned(), json!(["MISC"]));
    if definition.contains_key("guiTexture") {
        updates.insert("guiTexture".to_owned(), json!(texture));
    }
    updates.extend(food_updates(source_text, superclass));
    updates.extend(ranged_updates(source_text, &definition));
    if updates.get("isFood").and_then(Value::as_bool) == Some(true) {
        updates.insert("creativeTabs".to_owned(), json!(["FOOD"]));
    }

    let mut warnings = Vec::new();
    if updates.get("isFood").and_then(Value::as_bool) == Some(true)
        && !updates.contains_key("nutritionalValue") {
        warnings.push("Food was detected but exact nutrition/saturation values could not be recovered; verify the generated food properties.".to_owned());
    }
    let mut element = finish_element("item", template, imported_fv, simple_class, registry, updates);
    if let Some(definition) = element.get_mut("definition").and_then(Value::as_object_mut) {
        definition.entry("renderType".to_owned()).or_insert(json!(0));
        definition.entry("customModelName".to_owned()).or_insert(json!("Normal"));
    }
    (element, warnings)
}

/// Classify a source item subclass as one of MCreator's native tool types.
pub fn classify_tool(superclass: &str, simple_class: &str) -> Option<String> {
    let parent = superclass.to_ascii_lowercase();
    let simple = simple_class.to_ascii_lowercase();
    let mappings = [
        ("itemaxe", "Axe"), ("itemsword", "Sword"), ("itempickaxe", "Pickaxe"),
        ("itemspade", "Shovel"), ("itemshovel", "Shovel"), ("itemhoe", "Hoe"),
        ("itemshears", "Shears"), ("itemfishingrod", "Fishing Rod"), ("itembow", "Bow"),
        ("pickaxeitem", "Pickaxe"), ("sworditem", "Sword"), ("axeitem", "Axe"),
        ("shovelitem", "Shovel"), ("hoeitem", "Hoe"), ("bowitem", "Bow"),
        ("crossbowitem", "Bow"), ("fishingroditem", "Fishing Rod"), ("shearsitem", "Shears"),
    ];
    if let Some((_, kind)) = mappings.iter().find(|(token, _)| parent.contains(token)) {
        return Some((*kind).to_owned());
    }
    if !parent.is_empty() && parent != "item" && parent != "java.lang.object" && !parent.contains("item") {
        return None;
    }
    for (suffix, kind) in [
        ("pickaxe", "Pickaxe"), ("fishingrod", "Fishing Rod"), ("shears", "Shears"),
        ("shovel", "Shovel"), ("spade", "Shovel"), ("sword", "Sword"),
        ("axe", "Axe"), ("hoe", "Hoe"), ("bow", "Bow"),
    ] {
        if simple.ends_with(suffix) {
            return Some(kind.to_owned());
        }
    }
    None
}

/// Recover durability, attack damage, efficiency, enchantability and attack speed.
pub fn build_tool_element(
    template: Option<&Value>,
    simple_class: &str,
    superclass: &str,
    source_text: &str,
    registry: &str,
    texture: &str,
    tool_type: &str,
    material: Option<&ToolMaterialStats>,
    imported_fv: i64,
) -> (Value, Vec<String>) {
    let mut definition = template_definition(template);
    sanitize_item_template(&mut definition, "tool");
    let mut warnings = Vec::new();

    let mut durability = integer_after_call(source_text, &["func_77656_e", "setMaxDamage"]);
    if durability.is_none() {
        durability = regex_number(source_text, &[r"\.durability\s*\(\s*(-?\d+)\s*\)"]).map(|v| v as i64);
    }
    let mut damage = source_field_number(source_text, &[
        "weaponDamage","WEAPON_DAMAGE","attackDamage","ATTACK_DAMAGE","damageVsEntity","DAMAGE","damage"
    ]).or_else(|| method_return_number(source_text, &["getDamageVsEntity","getAttackDamage","func_150931_i"]));
    let mut efficiency = material.and_then(|stats| stats.efficiency).unwrap_or(4.0);
    let mut enchantability = method_return_number(source_text, &["getEnchantmentValue","getItemEnchantability"])
        .map(|value| value as i64)
        .or_else(|| material.and_then(|stats| stats.enchantability))
        .unwrap_or(10);

    if durability.is_none() {
        durability = material.and_then(|stats| stats.uses);
    }
    if damage.is_none() {
        if let Some(material_damage) = material.and_then(|stats| stats.damage) {
            let offset = match tool_type {
                "Sword" => 4.0, "Axe" => 3.0, "Pickaxe" => 2.0, "Shovel" => 1.0,
                _ => 0.0
            };
            damage = Some(material_damage + offset);
        }
    }
    let default_damage = match tool_type {
        "Sword" => 4.0, "Axe" => 5.0, "Pickaxe" => 3.0, "Shovel" => 2.0,
        "Hoe" => 1.0, "Bow" => 2.0, _ => 3.0
    };
    let damage = damage.unwrap_or_else(|| {
        warnings.push(format!(
            "Tool attack damage was not explicit; used a {} baseline of {} instead of guessing from durability/use duration.",
            tool_type.to_ascii_lowercase(), default_damage
        ));
        default_damage
    });
    let durability = durability.unwrap_or_else(|| {
        warnings.push("Tool durability could not be recovered exactly; defaulted to 250 uses.".to_owned());
        250
    }).clamp(1, i64::MAX);
    if material.is_none() {
        efficiency = 4.0;
    }

    let mut attack_speed = 1.0;
    if let Ok(regex) = Regex::new(r"(?s)super\s*\([^;]*?,\s*(-?\d+(?:\.\d+)?)[fF]?\s*,\s*(?:new\s+)?Properties") {
        if let Some(captures) = regex.captures(source_text) {
            if let Some(raw) = captures.get(1).and_then(|m| m.as_str().parse::<f64>().ok()) {
                attack_speed = if raw > 0.0 { (4.0 - raw).max(0.0) } else { (4.0 + raw).max(0.0) };
            }
        }
    }

    let combat = matches!(tool_type, "Sword" | "Bow");
    let mut updates = Map::new();
    updates.insert("toolType".to_owned(), json!(tool_type));
    updates.insert("texture".to_owned(), json!(texture));
    updates.insert("name".to_owned(), json!(item_display_name(registry)));
    updates.insert("creativeTabs".to_owned(), json!(if combat { ["COMBAT"] } else { ["TOOLS"] }));
    updates.insert("usageCount".to_owned(), json!(durability));
    updates.insert("damageVsEntity".to_owned(), json!(damage.max(0.0)));
    updates.insert("efficiency".to_owned(), json!(efficiency.max(0.0)));
    updates.insert("attackSpeed".to_owned(), json!(attack_speed));
    updates.insert("enchantability".to_owned(), json!(enchantability.max(0)));
    updates.insert("repairItems".to_owned(), json!([]));
    if definition.contains_key("guiTexture") {
        updates.insert("guiTexture".to_owned(), json!(texture));
    }
    warnings.push("Material-specific mining speed, repair ingredient, and special behavior were not fully reconstructed.".to_owned());

    let mut element = finish_element("tool", template, imported_fv, simple_class, registry, updates);
    if let Some(definition) = element.get_mut("definition").and_then(Value::as_object_mut) {
        definition.entry("renderType".to_owned()).or_insert(json!(0));
        definition.entry("customModelName".to_owned()).or_insert(json!("Normal"));
        if definition.contains_key("blockingRenderType") {
            definition.insert("blockingRenderType".to_owned(), json!(0));
        }
        if definition.contains_key("blockingModelName") {
            definition.insert("blockingModelName".to_owned(), json!("Normal blocking"));
        }
    }
    (element, warnings)
}



use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct EquipmentSource {
    pub class_name: String,
    pub superclass: String,
    pub registry_name: String,
    pub element_type: String,
    pub source_path: String,
    #[serde(skip)]
    pub source_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquipmentExportRecord {
    pub class_name: String,
    pub element_name: String,
    pub registry_name: String,
    pub element_type: String,
    pub texture: String,
    pub source_path: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquipmentWorkspaceExportReport {
    pub workspace_file: PathBuf,
    pub discovered: usize,
    pub exported: usize,
    pub skipped: usize,
    pub records: Vec<EquipmentExportRecord>,
    pub warnings: Vec<String>,
}

fn simple_java_name(value: &str) -> String {
    value.rsplit(['.', '
    use super::*;

    #[test]
    fn builds_native_item_schema_with_stack_and_damage_recovery() {
        let (element, warnings) = build_item_element(
            None, "ItemCrystal", "Item", "setMaxStackSize(16); setMaxDamage(32);",
            "crystal", "crystal.png", 89
        );
        assert!(warnings.is_empty());
        assert_eq!(element["_type"], "item");
        assert_eq!(element["_fv"], 89);
        assert_eq!(element["definition"]["name"], "Crystal");
        assert_eq!(element["definition"]["stackSize"], 16);
        assert_eq!(element["definition"]["damageCount"], 32);
        assert_eq!(element["definition"]["texture"], "crystal.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
    }

    #[test]
    fn caps_stack_size_and_preserves_template_specific_fields() {
        let template = json!({
            "_fv":88,"_type":"item",
            "definition":{
                "name":"Old","texture":"old.png","guiTexture":"old.png",
                "renderType":7,"customModelName":"OldModel",
                "creativeTabs":["BLOCKS"],"templateMarker":{"keep":true}
            }
        });
        let (element, _) = build_item_element(Some(&template), "ItemBigStack", "Item",
            "setMaxStackSize(128);", "big_stack", "new.png", 89);
        assert_eq!(element["_fv"], 88);
        assert_eq!(element["definition"]["stackSize"], 64);
        assert_eq!(element["definition"]["texture"], "new.png");
        assert_eq!(element["definition"]["guiTexture"], "new.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
        assert_eq!(element["definition"]["templateMarker"]["keep"], true);
    }

    #[test]
    fn recognizes_modern_food_properties_and_food_creative_tab() {
        let source = r#"
            public class ExampleFood extends Item {
                public ExampleFood() {
                    super(new Item.Properties().food(new FoodProperties.Builder()
                        .nutrition(6).saturationMod(0.8F).meat().alwaysEat().build()));
                }
            }
        "#;
        let (element, warnings) = build_item_element(None, "ExampleFood", "Item", source, "example_food", "food.png", 89);
        assert!(warnings.is_empty());
        assert_eq!(element["definition"]["isFood"], true);
        assert_eq!(element["definition"]["nutritionalValue"], 6);
        assert_eq!(element["definition"]["saturation"], 0.8);
        assert_eq!(element["definition"]["isMeat"], true);
        assert_eq!(element["definition"]["isAlwaysEdible"], true);
        assert_eq!(element["definition"]["creativeTabs"], json!(["FOOD"]));
    }

    #[test]
    fn recognizes_native_bow_behavior_and_projectile_trace() {
        let source = r#"
            public class OldBow extends Item {
                public UseAnim getUseAnimation() { return UseAnim.BOW; }
                public int getUseDuration() { return 72; }
                public void releaseUsing() { new CustomArrow(); }
            }
        "#;
        let (element, _) = build_item_element(None, "OldBow", "Item", source, "old_bow", "bow.png", 89);
        let definition = &element["definition"];
        assert_eq!(definition["enableRanged"], true);
        assert_eq!(definition["animation"], "bow");
        assert_eq!(definition["useDuration"], 72);
        assert_eq!(definition["projectile"], "Arrow");
        assert_eq!(definition["mojmodelbridge_source_projectile_class"], "CustomArrow");
    }

    #[test]
    fn classifies_legacy_and_modern_tools() {
        assert_eq!(classify_tool("ItemSword", "Example"), Some("Sword".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.PickaxeItem", "Custom"), Some("Pickaxe".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.Item", "ExampleBow"), Some("Bow".to_owned()));
        assert_eq!(classify_tool("LivingEntity", "EntityDragon"), None);
    }

    #[test]
    fn recovers_tool_durability_damage_enchantability_and_attack_speed() {
        let source = r#"
            public class TestSword extends ItemSword {
                private static final float weaponDamage = 7.0F;
                public int getEnchantmentValue() { return 14; }
                public TestSword() { super(2, -2.4F, new Properties()); }
            }
            setMaxDamage(900);
        "#;
        let material = ToolMaterialStats { efficiency:Some(8.0), ..Default::default() };
        let (element, warnings) = build_tool_element(
            None,"TestSword","ItemSword",source,"test_sword","test_sword.png","Sword",Some(&material),89
        );
        assert_eq!(element["definition"]["toolType"], "Sword");
        assert_eq!(element["definition"]["usageCount"], 900);
        assert_eq!(element["definition"]["damageVsEntity"], 7.0);
        assert_eq!(element["definition"]["efficiency"], 8.0);
        assert_eq!(element["definition"]["enchantability"], 14);
        assert_eq!(element["definition"]["attackSpeed"], 1.6);
        assert_eq!(element["definition"]["creativeTabs"], json!(["COMBAT"]));
        assert!(warnings.iter().any(|warning| warning.contains("repair ingredient")));
    }

    #[test]
    fn defaults_unrecovered_tool_stats_conservatively() {
        let (element, warnings) = build_tool_element(
            None,"UnknownPickaxe","Item","", "unknown_pickaxe", "", "Pickaxe", None,89
        );
        assert_eq!(element["definition"]["usageCount"], 250);
        assert_eq!(element["definition"]["damageVsEntity"], 3.0);
        assert_eq!(element["definition"]["efficiency"], 4.0);
        assert_eq!(element["definition"]["enchantability"], 10);
        assert_eq!(warnings.len(), 3);
    }
}
]).next().unwrap_or(value).to_owned()
}

fn normalize_registry(value: &str) -> String {
    let mut value = value.trim().trim_matches('"').trim_matches('\'').to_owned();
    if let Some((_, path)) = value.split_once(':') {
        value = path.to_owned();
    }
    value.trim_matches('/').replace('-', "_").replace('/', "_").to_ascii_lowercase()
}

fn valid_registry(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-' | '/'))
        && !value.replace('_', "").chars().all(|ch| ch.is_ascii_digit())
}

fn java_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("java"))
            {
                files.push(entry.into_path());
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn source_class_headers(text: &str) -> Vec<(String, String, bool)> {
    let Ok(regex) = Regex::new(
        r"\b(?:(abstract|final)\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>{}]*>)?\s+extends\s+([A-Za-z_$][A-Za-z0-9_$.]*)"
    ) else { return Vec::new() };
    regex.captures_iter(text).filter_map(|capture| {
        Some((
            capture.get(2)?.as_str().to_owned(),
            simple_java_name(capture.get(3)?.as_str()),
            capture.get(1).is_some_and(|value| value.as_str() == "abstract"),
        ))
    }).collect()
}

fn source_constructor_assignments(text: &str) -> Vec<(String, String, String)> {
    let patterns = [
        r"\b([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:(?:\([A-Za-z_$][\w.$<>?, \[\]]*\)\s*)|\(*\s*)new\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\(([^;]{0,1600})",
        r"\b(?:public|protected|private|static|final|volatile|transient|\s)+[A-Za-z_$][\w.$<>?, \[\]]*\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:(?:\([A-Za-z_$][\w.$<>?, \[\]]*\)\s*)|\(*\s*)new\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\(([^;]{0,1600})",
    ];
    let mut found = BTreeMap::<(String, String), (String, String, String)>::new();
    for pattern in patterns {
        let Ok(regex) = Regex::new(pattern) else { continue };
        for capture in regex.captures_iter(text) {
            let Some(field) = capture.get(1).map(|value| value.as_str()) else { continue };
            let Some(class) = capture.get(2).map(|value| simple_java_name(value.as_str())) else { continue };
            let Some(args) = capture.get(3).map(|value| value.as_str()) else { continue };
            found.entry((field.to_owned(), class.clone()))
                .or_insert((field.to_owned(), class, args.to_owned()));
        }
    }
    found.into_values().collect()
}

fn quoted_values(text: &str) -> Vec<String> {
    let Ok(regex) = Regex::new(r#"\"([^\"\\]*(?:\\.[^\"\\]*)*)\""#) else { return Vec::new() };
    regex.captures_iter(text).filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned())).collect()
}

fn registry_from_constructor_args(args: &str) -> Option<String> {
    let values = quoted_values(args);
    if values.is_empty() {
        return None;
    }
    let raw = if args.contains("ResourceLocation") { values.last()? } else { values.first()? };
    let name = normalize_registry(raw);
    valid_registry(&name).then_some(name)
}

fn java_call_arguments(source: &str, names: &[&str]) -> Vec<(String, String)> {
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
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' || bytes[index] == b'
    use super::*;

    #[test]
    fn builds_native_item_schema_with_stack_and_damage_recovery() {
        let (element, warnings) = build_item_element(
            None, "ItemCrystal", "Item", "setMaxStackSize(16); setMaxDamage(32);",
            "crystal", "crystal.png", 89
        );
        assert!(warnings.is_empty());
        assert_eq!(element["_type"], "item");
        assert_eq!(element["_fv"], 89);
        assert_eq!(element["definition"]["name"], "Crystal");
        assert_eq!(element["definition"]["stackSize"], 16);
        assert_eq!(element["definition"]["damageCount"], 32);
        assert_eq!(element["definition"]["texture"], "crystal.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
    }

    #[test]
    fn caps_stack_size_and_preserves_template_specific_fields() {
        let template = json!({
            "_fv":88,"_type":"item",
            "definition":{
                "name":"Old","texture":"old.png","guiTexture":"old.png",
                "renderType":7,"customModelName":"OldModel",
                "creativeTabs":["BLOCKS"],"templateMarker":{"keep":true}
            }
        });
        let (element, _) = build_item_element(Some(&template), "ItemBigStack", "Item",
            "setMaxStackSize(128);", "big_stack", "new.png", 89);
        assert_eq!(element["_fv"], 88);
        assert_eq!(element["definition"]["stackSize"], 64);
        assert_eq!(element["definition"]["texture"], "new.png");
        assert_eq!(element["definition"]["guiTexture"], "new.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
        assert_eq!(element["definition"]["templateMarker"]["keep"], true);
    }

    #[test]
    fn recognizes_modern_food_properties_and_food_creative_tab() {
        let source = r#"
            public class ExampleFood extends Item {
                public ExampleFood() {
                    super(new Item.Properties().food(new FoodProperties.Builder()
                        .nutrition(6).saturationMod(0.8F).meat().alwaysEat().build()));
                }
            }
        "#;
        let (element, warnings) = build_item_element(None, "ExampleFood", "Item", source, "example_food", "food.png", 89);
        assert!(warnings.is_empty());
        assert_eq!(element["definition"]["isFood"], true);
        assert_eq!(element["definition"]["nutritionalValue"], 6);
        assert_eq!(element["definition"]["saturation"], 0.8);
        assert_eq!(element["definition"]["isMeat"], true);
        assert_eq!(element["definition"]["isAlwaysEdible"], true);
        assert_eq!(element["definition"]["creativeTabs"], json!(["FOOD"]));
    }

    #[test]
    fn recognizes_native_bow_behavior_and_projectile_trace() {
        let source = r#"
            public class OldBow extends Item {
                public UseAnim getUseAnimation() { return UseAnim.BOW; }
                public int getUseDuration() { return 72; }
                public void releaseUsing() { new CustomArrow(); }
            }
        "#;
        let (element, _) = build_item_element(None, "OldBow", "Item", source, "old_bow", "bow.png", 89);
        let definition = &element["definition"];
        assert_eq!(definition["enableRanged"], true);
        assert_eq!(definition["animation"], "bow");
        assert_eq!(definition["useDuration"], 72);
        assert_eq!(definition["projectile"], "Arrow");
        assert_eq!(definition["mojmodelbridge_source_projectile_class"], "CustomArrow");
    }

    #[test]
    fn classifies_legacy_and_modern_tools() {
        assert_eq!(classify_tool("ItemSword", "Example"), Some("Sword".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.PickaxeItem", "Custom"), Some("Pickaxe".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.Item", "ExampleBow"), Some("Bow".to_owned()));
        assert_eq!(classify_tool("LivingEntity", "EntityDragon"), None);
    }

    #[test]
    fn recovers_tool_durability_damage_enchantability_and_attack_speed() {
        let source = r#"
            public class TestSword extends ItemSword {
                private static final float weaponDamage = 7.0F;
                public int getEnchantmentValue() { return 14; }
                public TestSword() { super(2, -2.4F, new Properties()); }
            }
            setMaxDamage(900);
        "#;
        let material = ToolMaterialStats { efficiency:Some(8.0), ..Default::default() };
        let (element, warnings) = build_tool_element(
            None,"TestSword","ItemSword",source,"test_sword","test_sword.png","Sword",Some(&material),89
        );
        assert_eq!(element["definition"]["toolType"], "Sword");
        assert_eq!(element["definition"]["usageCount"], 900);
        assert_eq!(element["definition"]["damageVsEntity"], 7.0);
        assert_eq!(element["definition"]["efficiency"], 8.0);
        assert_eq!(element["definition"]["enchantability"], 14);
        assert_eq!(element["definition"]["attackSpeed"], 1.6);
        assert_eq!(element["definition"]["creativeTabs"], json!(["COMBAT"]));
        assert!(warnings.iter().any(|warning| warning.contains("repair ingredient")));
    }

    #[test]
    fn defaults_unrecovered_tool_stats_conservatively() {
        let (element, warnings) = build_tool_element(
            None,"UnknownPickaxe","Item","", "unknown_pickaxe", "", "Pickaxe", None,89
        );
        assert_eq!(element["definition"]["usageCount"], 250);
        assert_eq!(element["definition"]["damageVsEntity"], 3.0);
        assert_eq!(element["definition"]["efficiency"], 4.0);
        assert_eq!(element["definition"]["enchantability"], 10);
        assert_eq!(warnings.len(), 3);
    }
}
 {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' || bytes[index] == b'
    use super::*;

    #[test]
    fn builds_native_item_schema_with_stack_and_damage_recovery() {
        let (element, warnings) = build_item_element(
            None, "ItemCrystal", "Item", "setMaxStackSize(16); setMaxDamage(32);",
            "crystal", "crystal.png", 89
        );
        assert!(warnings.is_empty());
        assert_eq!(element["_type"], "item");
        assert_eq!(element["_fv"], 89);
        assert_eq!(element["definition"]["name"], "Crystal");
        assert_eq!(element["definition"]["stackSize"], 16);
        assert_eq!(element["definition"]["damageCount"], 32);
        assert_eq!(element["definition"]["texture"], "crystal.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
    }

    #[test]
    fn caps_stack_size_and_preserves_template_specific_fields() {
        let template = json!({
            "_fv":88,"_type":"item",
            "definition":{
                "name":"Old","texture":"old.png","guiTexture":"old.png",
                "renderType":7,"customModelName":"OldModel",
                "creativeTabs":["BLOCKS"],"templateMarker":{"keep":true}
            }
        });
        let (element, _) = build_item_element(Some(&template), "ItemBigStack", "Item",
            "setMaxStackSize(128);", "big_stack", "new.png", 89);
        assert_eq!(element["_fv"], 88);
        assert_eq!(element["definition"]["stackSize"], 64);
        assert_eq!(element["definition"]["texture"], "new.png");
        assert_eq!(element["definition"]["guiTexture"], "new.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
        assert_eq!(element["definition"]["templateMarker"]["keep"], true);
    }

    #[test]
    fn recognizes_modern_food_properties_and_food_creative_tab() {
        let source = r#"
            public class ExampleFood extends Item {
                public ExampleFood() {
                    super(new Item.Properties().food(new FoodProperties.Builder()
                        .nutrition(6).saturationMod(0.8F).meat().alwaysEat().build()));
                }
            }
        "#;
        let (element, warnings) = build_item_element(None, "ExampleFood", "Item", source, "example_food", "food.png", 89);
        assert!(warnings.is_empty());
        assert_eq!(element["definition"]["isFood"], true);
        assert_eq!(element["definition"]["nutritionalValue"], 6);
        assert_eq!(element["definition"]["saturation"], 0.8);
        assert_eq!(element["definition"]["isMeat"], true);
        assert_eq!(element["definition"]["isAlwaysEdible"], true);
        assert_eq!(element["definition"]["creativeTabs"], json!(["FOOD"]));
    }

    #[test]
    fn recognizes_native_bow_behavior_and_projectile_trace() {
        let source = r#"
            public class OldBow extends Item {
                public UseAnim getUseAnimation() { return UseAnim.BOW; }
                public int getUseDuration() { return 72; }
                public void releaseUsing() { new CustomArrow(); }
            }
        "#;
        let (element, _) = build_item_element(None, "OldBow", "Item", source, "old_bow", "bow.png", 89);
        let definition = &element["definition"];
        assert_eq!(definition["enableRanged"], true);
        assert_eq!(definition["animation"], "bow");
        assert_eq!(definition["useDuration"], 72);
        assert_eq!(definition["projectile"], "Arrow");
        assert_eq!(definition["mojmodelbridge_source_projectile_class"], "CustomArrow");
    }

    #[test]
    fn classifies_legacy_and_modern_tools() {
        assert_eq!(classify_tool("ItemSword", "Example"), Some("Sword".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.PickaxeItem", "Custom"), Some("Pickaxe".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.Item", "ExampleBow"), Some("Bow".to_owned()));
        assert_eq!(classify_tool("LivingEntity", "EntityDragon"), None);
    }

    #[test]
    fn recovers_tool_durability_damage_enchantability_and_attack_speed() {
        let source = r#"
            public class TestSword extends ItemSword {
                private static final float weaponDamage = 7.0F;
                public int getEnchantmentValue() { return 14; }
                public TestSword() { super(2, -2.4F, new Properties()); }
            }
            setMaxDamage(900);
        "#;
        let material = ToolMaterialStats { efficiency:Some(8.0), ..Default::default() };
        let (element, warnings) = build_tool_element(
            None,"TestSword","ItemSword",source,"test_sword","test_sword.png","Sword",Some(&material),89
        );
        assert_eq!(element["definition"]["toolType"], "Sword");
        assert_eq!(element["definition"]["usageCount"], 900);
        assert_eq!(element["definition"]["damageVsEntity"], 7.0);
        assert_eq!(element["definition"]["efficiency"], 8.0);
        assert_eq!(element["definition"]["enchantability"], 14);
        assert_eq!(element["definition"]["attackSpeed"], 1.6);
        assert_eq!(element["definition"]["creativeTabs"], json!(["COMBAT"]));
        assert!(warnings.iter().any(|warning| warning.contains("repair ingredient")));
    }

    #[test]
    fn defaults_unrecovered_tool_stats_conservatively() {
        let (element, warnings) = build_tool_element(
            None,"UnknownPickaxe","Item","", "unknown_pickaxe", "", "Pickaxe", None,89
        );
        assert_eq!(element["definition"]["usageCount"], 250);
        assert_eq!(element["definition"]["damageVsEntity"], 3.0);
        assert_eq!(element["definition"]["efficiency"], 4.0);
        assert_eq!(element["definition"]["enchantability"], 10);
        assert_eq!(warnings.len(), 3);
    }
}
)
            {
                index += 1;
            }
            let name = &source[start..index];
            if !names.contains(&name) {
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

fn terminal_field_name(value: &str) -> String {
    let value = value.trim();
    let tail = value.rsplit('.').next().unwrap_or(value).trim();
    if tail.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '
    use super::*;

    #[test]
    fn builds_native_item_schema_with_stack_and_damage_recovery() {
        let (element, warnings) = build_item_element(
            None, "ItemCrystal", "Item", "setMaxStackSize(16); setMaxDamage(32);",
            "crystal", "crystal.png", 89
        );
        assert!(warnings.is_empty());
        assert_eq!(element["_type"], "item");
        assert_eq!(element["_fv"], 89);
        assert_eq!(element["definition"]["name"], "Crystal");
        assert_eq!(element["definition"]["stackSize"], 16);
        assert_eq!(element["definition"]["damageCount"], 32);
        assert_eq!(element["definition"]["texture"], "crystal.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
    }

    #[test]
    fn caps_stack_size_and_preserves_template_specific_fields() {
        let template = json!({
            "_fv":88,"_type":"item",
            "definition":{
                "name":"Old","texture":"old.png","guiTexture":"old.png",
                "renderType":7,"customModelName":"OldModel",
                "creativeTabs":["BLOCKS"],"templateMarker":{"keep":true}
            }
        });
        let (element, _) = build_item_element(Some(&template), "ItemBigStack", "Item",
            "setMaxStackSize(128);", "big_stack", "new.png", 89);
        assert_eq!(element["_fv"], 88);
        assert_eq!(element["definition"]["stackSize"], 64);
        assert_eq!(element["definition"]["texture"], "new.png");
        assert_eq!(element["definition"]["guiTexture"], "new.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
        assert_eq!(element["definition"]["templateMarker"]["keep"], true);
    }

    #[test]
    fn recognizes_modern_food_properties_and_food_creative_tab() {
        let source = r#"
            public class ExampleFood extends Item {
                public ExampleFood() {
                    super(new Item.Properties().food(new FoodProperties.Builder()
                        .nutrition(6).saturationMod(0.8F).meat().alwaysEat().build()));
                }
            }
        "#;
        let (element, warnings) = build_item_element(None, "ExampleFood", "Item", source, "example_food", "food.png", 89);
        assert!(warnings.is_empty());
        assert_eq!(element["definition"]["isFood"], true);
        assert_eq!(element["definition"]["nutritionalValue"], 6);
        assert_eq!(element["definition"]["saturation"], 0.8);
        assert_eq!(element["definition"]["isMeat"], true);
        assert_eq!(element["definition"]["isAlwaysEdible"], true);
        assert_eq!(element["definition"]["creativeTabs"], json!(["FOOD"]));
    }

    #[test]
    fn recognizes_native_bow_behavior_and_projectile_trace() {
        let source = r#"
            public class OldBow extends Item {
                public UseAnim getUseAnimation() { return UseAnim.BOW; }
                public int getUseDuration() { return 72; }
                public void releaseUsing() { new CustomArrow(); }
            }
        "#;
        let (element, _) = build_item_element(None, "OldBow", "Item", source, "old_bow", "bow.png", 89);
        let definition = &element["definition"];
        assert_eq!(definition["enableRanged"], true);
        assert_eq!(definition["animation"], "bow");
        assert_eq!(definition["useDuration"], 72);
        assert_eq!(definition["projectile"], "Arrow");
        assert_eq!(definition["mojmodelbridge_source_projectile_class"], "CustomArrow");
    }

    #[test]
    fn classifies_legacy_and_modern_tools() {
        assert_eq!(classify_tool("ItemSword", "Example"), Some("Sword".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.PickaxeItem", "Custom"), Some("Pickaxe".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.Item", "ExampleBow"), Some("Bow".to_owned()));
        assert_eq!(classify_tool("LivingEntity", "EntityDragon"), None);
    }

    #[test]
    fn recovers_tool_durability_damage_enchantability_and_attack_speed() {
        let source = r#"
            public class TestSword extends ItemSword {
                private static final float weaponDamage = 7.0F;
                public int getEnchantmentValue() { return 14; }
                public TestSword() { super(2, -2.4F, new Properties()); }
            }
            setMaxDamage(900);
        "#;
        let material = ToolMaterialStats { efficiency:Some(8.0), ..Default::default() };
        let (element, warnings) = build_tool_element(
            None,"TestSword","ItemSword",source,"test_sword","test_sword.png","Sword",Some(&material),89
        );
        assert_eq!(element["definition"]["toolType"], "Sword");
        assert_eq!(element["definition"]["usageCount"], 900);
        assert_eq!(element["definition"]["damageVsEntity"], 7.0);
        assert_eq!(element["definition"]["efficiency"], 8.0);
        assert_eq!(element["definition"]["enchantability"], 14);
        assert_eq!(element["definition"]["attackSpeed"], 1.6);
        assert_eq!(element["definition"]["creativeTabs"], json!(["COMBAT"]));
        assert!(warnings.iter().any(|warning| warning.contains("repair ingredient")));
    }

    #[test]
    fn defaults_unrecovered_tool_stats_conservatively() {
        let (element, warnings) = build_tool_element(
            None,"UnknownPickaxe","Item","", "unknown_pickaxe", "", "Pickaxe", None,89
        );
        assert_eq!(element["definition"]["usageCount"], 250);
        assert_eq!(element["definition"]["damageVsEntity"], 3.0);
        assert_eq!(element["definition"]["efficiency"], 4.0);
        assert_eq!(element["definition"]["enchantability"], 10);
        assert_eq!(warnings.len(), 3);
    }
}
) {
        tail.to_owned()
    } else {
        String::new()
    }
}

fn registration_registry(args: &[String]) -> Option<String> {
    if args.len() < 2 {
        return None;
    }
    let values = quoted_values(&args[1]);
    let raw = values.last()?;
    let registry = normalize_registry(raw);
    valid_registry(&registry).then_some(registry)
}

fn collect_source_registries(
    texts: &[String],
) -> BTreeMap<String, BTreeSet<String>> {
    use crate::source::java::split_top_level;

    let mut field_classes = BTreeMap::<String, String>::new();
    let mut class_registries = BTreeMap::<String, BTreeSet<String>>::new();
    for text in texts {
        for (field, class, args) in source_constructor_assignments(text) {
            field_classes.insert(field, class.clone());
            if let Some(registry) = registry_from_constructor_args(&args) {
                class_registries.entry(class).or_default().insert(registry);
            }
        }
    }

    let combined = texts.join("\n");
    for (call, raw_args) in java_call_arguments(&combined, &["registerItem", "registerBlock"]) {
        let args = split_top_level(&raw_args, ',');
        if args.len() < 2 {
            continue;
        }
        let field = terminal_field_name(&args[0]);
        let Some(class) = field_classes.get(&field) else { continue };
        let Some(registry) = registration_registry(&args) else { continue };
        class_registries.entry(class.clone()).or_default().insert(registry);
        let _ = call;
    }

    let modern = Regex::new(
        r#"(?s)\b(?:ITEMS|BLOCKS|ITEM_REGISTRY|BLOCK_REGISTRY)\s*\.\s*register\s*\(\s*"([^"]+)"\s*,\s*([^;]{0,1600})"#
    ).unwrap();
    let class_new = Regex::new(r"\bnew\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\(").unwrap();
    let class_constructor = Regex::new(r"\b([A-Za-z_$][A-Za-z0-9_$.]*)\s*::\s*new\b").unwrap();
    for capture in modern.captures_iter(&combined) {
        let registry = normalize_registry(capture.get(1).map(|value| value.as_str()).unwrap_or(""));
        if !valid_registry(&registry) {
            continue;
        }
        let body = capture.get(2).map(|value| value.as_str()).unwrap_or("");
        let mut classes = BTreeSet::new();
        classes.extend(class_new.captures_iter(body).filter_map(|item| item.get(1).map(|value| simple_java_name(value.as_str()))));
        classes.extend(class_constructor.captures_iter(body).filter_map(|item| item.get(1).map(|value| simple_java_name(value.as_str()))));
        for class in classes {
            if !matches!(class.as_str(), "Object" | "Item" | "Block" | "EntityType") {
                class_registries.entry(class).or_default().insert(registry.clone());
            }
        }
    }
    class_registries
}

fn superclass_is_item(
    superclass: &str,
    class_parents: &BTreeMap<String, String>,
    depth: usize,
) -> bool {
    if depth > 12 {
        return false;
    }
    let parent = simple_java_name(superclass).to_ascii_lowercase();
    if parent == "item" || parent.contains("item") {
        return true;
    }
    class_parents.get(&simple_java_name(superclass))
        .is_some_and(|next| superclass_is_item(next, class_parents, depth + 1))
}

fn equipment_kind(class: &str, superclass: &str, parents: &BTreeMap<String, String>) -> Option<String> {
    if superclass.to_ascii_lowercase().contains("itemarmor")
        || class.to_ascii_lowercase().ends_with("armor")
    {
        return None;
    }
    if let Some(tool) = classify_tool(superclass, class) {
        return Some("tool".to_owned() + "|" + &tool);
    }
    if superclass_is_item(superclass, parents, 0) {
        return Some("item".to_owned());
    }
    None
}

/// Discover concrete Java-registered item/tool subclasses. Compiled-only registrations
/// are not yet inferred here; the bytecode registry scanner remains a follow-up.
pub fn discover_equipment_sources(source_roots: &[PathBuf]) -> Result<Vec<EquipmentSource>> {
    use crate::source::java::strip_java_comments;

    let files = java_files(source_roots);
    let mut texts = Vec::<String>::new();
    let mut class_paths = BTreeMap::<String, PathBuf>::new();
    let mut class_texts = BTreeMap::<String, String>::new();
    let mut class_parents = BTreeMap::<String, String>::new();
    let mut abstract_classes = HashSet::<String>::new();

    for path in &files {
        let Ok(bytes) = fs::read(path) else { continue };
        let text = strip_java_comments(&String::from_utf8_lossy(&bytes));
        for (class, parent, is_abstract) in source_class_headers(&text) {
            class_paths.entry(class.clone()).or_insert_with(|| path.clone());
            class_texts.entry(class.clone()).or_insert_with(|| text.clone());
            class_parents.insert(class.clone(), parent);
            if is_abstract {
                abstract_classes.insert(class);
            }
        }
        texts.push(text);
    }

    let registry_map = collect_source_registries(&texts);
    let mut output = Vec::new();
    let mut seen = BTreeSet::<(String, String)>::new();
    for (class, registries) in registry_map {
        if abstract_classes.contains(&class) {
            continue;
        }
        let Some(source_text) = class_texts.get(&class) else { continue };
        let Some(source_path) = class_paths.get(&class) else { continue };
        let Some(superclass) = class_parents.get(&class) else { continue };
        let Some(kind) = equipment_kind(&class, superclass, &class_parents) else { continue };
        let (element_type, tool_type) = if let Some(tool_type) = kind.strip_prefix("tool|") {
            ("tool".to_owned(), Some(tool_type.to_owned()))
        } else {
            (kind, None)
        };

        for registry in registries {
            if !seen.insert((class.clone(), registry.clone())) {
                continue;
            }
            let mut text = source_text.clone();
            if let Some(tool_type) = &tool_type {
                // The source superclass may be a custom base; the final type is
                // already resolved from the class chain and passed explicitly.
                let _ = tool_type;
            }
            output.push(EquipmentSource {
                class_name: class.clone(),
                superclass: superclass.clone(),
                registry_name: registry,
                element_type,
                source_path: source_path.to_string_lossy().replace('\\', "/"),
                source_text: std::mem::take(&mut text),
            });
        }
    }
    output.sort_by(|a, b| a.registry_name.cmp(&b.registry_name).then(a.class_name.cmp(&b.class_name)));
    Ok(output)
}

fn unique_equipment_element_name(class_name: &str, registry: &str) -> String {
    let stripped = class_name.strip_prefix("Item").unwrap_or(class_name);
    let compact_class: String = stripped.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect();
    let compact_registry: String = registry.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect();
    if !compact_class.is_empty() && compact_registry.ends_with(&compact_class) {
        return stripped.to_owned();
    }
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in registry.chars() {
        if !ch.is_ascii_alphanumeric() {
            if !current.is_empty() { words.push(std::mem::take(&mut current)); }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() { words.push(current); }
    let mut name = String::new();
    for word in words {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            name.push(first.to_ascii_uppercase());
            name.extend(chars);
        }
    }
    if name.is_empty() { name = "ConvertedItem".to_owned(); }
    if name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        name.insert_str(0, "Item");
    }
    name
}

fn texture_stem_from_reference(reference: &str) -> Option<String> {
    let mut value = reference.trim().replace('\\', "/");
    if value.is_empty() {
        return None;
    }
    if let Some((_, path)) = value.split_once(':') {
        value = path.to_owned();
    }
    let value = value.trim_start_matches('/');
    let value = value.strip_prefix("textures/").unwrap_or(value);
    Path::new(value).file_stem().and_then(|stem| stem.to_str()).map(str::to_owned)
}

fn declared_equipment_texture_refs(source_text: &str) -> Vec<String> {
    let patterns = [
        r#"\b(?:setTextureName|func_111206_d|registerIcon|func_94245_a)\s*\(\s*"([^"]+)""#,
        r#"new\s+ResourceLocation\s*\(\s*"([^"]+)"\s*,\s*"([^"]+)""#,
    ];
    let mut refs = Vec::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let Ok(regex) = Regex::new(pattern) else { continue };
        for capture in regex.captures_iter(source_text) {
            let raw = if index == 1 {
                capture.get(2).map(|value| value.as_str())
            } else {
                capture.get(1).map(|value| value.as_str())
            };
            if let Some(raw) = raw {
                if !refs.iter().any(|value| value.eq_ignore_ascii_case(raw)) {
                    refs.push(raw.to_owned());
                }
            }
        }
    }
    refs
}

fn texture_candidates(source_roots: &[PathBuf], source_modid: &str, source: &EquipmentSource) -> Vec<PathBuf> {
    let mut stems = Vec::<String>::new();
    for reference in declared_equipment_texture_refs(&source.source_text) {
        if let Some(stem) = texture_stem_from_reference(&reference) {
            stems.push(stem);
        }
    }
    stems.push(source.registry_name.clone());
    stems.push(source.class_name.clone());
    if let Some(stripped) = source.class_name.strip_prefix("Item") {
        stems.push(stripped.to_owned());
    }
    let stems: BTreeSet<String> = stems.into_iter().map(|value| value.to_ascii_lowercase()).collect();
    let mut candidates = Vec::<PathBuf>::new();

    for root in source_roots {
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if !entry.file_type().is_file()
                || !entry.path().extension().and_then(|value| value.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
            {
                continue;
            }
            let Some(stem) = entry.path().file_stem().and_then(|value| value.to_str()) else { continue };
            if stems.contains(&stem.to_ascii_lowercase()) {
                candidates.push(entry.into_path());
            }
        }
        let namespace = root.join("assets").join(source_modid).join("textures");
        if namespace.exists() {
            for stem in &stems {
                for folder in ["item", "items"] {
                    let path = namespace.join(folder).join(format!("{stem}.png"));
                    if path.is_file() {
                        candidates.push(path);
                    }
                }
            }
        }
    }

    candidates.sort_by_key(|path| {
        let text = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        let preferred = if text.contains("/textures/item/") || text.contains("/textures/items/") { 0 } else { 1 };
        (preferred, text.len(), text)
    });
    let mut seen = BTreeSet::new();
    candidates.retain(|path| {
        let key = fs::canonicalize(path).unwrap_or_else(|_| path.clone()).to_string_lossy().to_ascii_lowercase();
        seen.insert(key)
    });
    candidates
}

fn placeholder_item_png() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+iZ1sAAAAASUVORK5CYII="
    ).expect("embedded item placeholder PNG is valid base64")
}

fn workspace_descriptor_file(root: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::<(i64, usize, PathBuf)>::new();
    for entry in fs::read_dir(root).with_context(|| format!("Reading workspace {}", root.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("mcreator") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
        if !value.get("workspaceSettings").is_some_and(Value::is_object)
            || !value.get("mod_elements").is_some_and(Value::is_array)
        {
            continue;
        }
        let version = value.get("mcreatorVersion").and_then(Value::as_i64).unwrap_or(0);
        let count = value.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        candidates.push((version, count, path));
    }
    candidates.sort_by_key(|(version, count, path)| (*version, *count, path.to_string_lossy().to_ascii_lowercase()));
    candidates.pop().map(|(_, _, path)| path)
        .ok_or_else(|| anyhow::anyhow!("No valid top-level .mcreator workspace file found in {}", root.display()))
}

fn load_equipment_template(root: &Path, element_type: &str) -> Result<Value> {
    let embedded = match element_type {
        "tool" => include_str!("../../oldVer/mcreator_templates/ToolSchema.mod.json"),
        _ => include_str!("../../oldVer/mcreator_templates/ItemSchema.mod.json"),
    };
    let entries = root.join("elements");
    if entries.is_dir() {
        let mut paths: Vec<PathBuf> = fs::read_dir(&entries)?.filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
            if value.get("_type").and_then(Value::as_str) == Some(element_type)
                && value.get("definition").is_some_and(Value::is_object)
            {
                return Ok(value);
            }
        }
    }
    serde_json::from_str(embedded).context("Parsing embedded MCreator equipment schema")
}

fn equipment_language_entry(workspace: &mut Value, modid: &str, kind: &str, registry: &str, element_name: &str, display_name: &str) {
    if !workspace.get("language_map").is_some_and(Value::is_object) {
        workspace["language_map"] = json!({});
    }
    if !workspace["language_map"].get("en_us").is_some_and(Value::is_object) {
        workspace["language_map"]["en_us"] = json!({});
    }
    let language = workspace["language_map"]["en_us"].as_object_mut().expect("language map initialized");
    let prefix = if kind == "tool" { "item" } else { "item" };
    for key in [
        format!("{prefix}.{modid}.{registry}"),
        format!("{prefix}.{modid}.{registry}.name"),
        format!("{prefix}.{modid}.{element_name}"),
        format!("{prefix}.{modid}.{element_name}.name"),
    ] {
        language.insert(key, json!(display_name));
    }
}

fn equipment_source_modid(source_root: &Path, supplied: &str) -> String {
    if !supplied.trim().is_empty() {
        return supplied.trim().to_owned();
    }
    source_root.file_name().and_then(|value| value.to_str()).filter(|value| !value.is_empty())
        .unwrap_or("converted_mod").to_owned()
}

fn compact_name(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    let candidate = if base.is_empty() { "ConvertedItem" } else { base };
    let mut name = candidate.to_owned();
    let mut suffix = 2usize;
    while !used.insert(name.to_ascii_lowercase()) {
        name = format!("{candidate}{suffix}");
        suffix += 1;
    }
    name
}

/// Discover registered item/tool classes, copy source item textures and write
/// native MCreator element JSON plus workspace descriptors/language entries.
pub fn export_equipment_to_workspace(
    source_roots: &[PathBuf],
    workspace_root: &Path,
    source_modid: &str,
    imported_fv: i64,
) -> Result<EquipmentWorkspaceExportReport> {
    let workspace_root = fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let workspace_file = workspace_descriptor_file(&workspace_root)?;
    let bytes = fs::read(&workspace_file).with_context(|| format!("Reading {}", workspace_file.display()))?;
    let mut workspace: Value = serde_json::from_slice(&bytes).context("Parsing MCreator workspace descriptor")?;
    let target_modid = workspace.get("workspaceSettings").and_then(|settings| settings.get("modid"))
        .and_then(Value::as_str).filter(|value| !value.trim().is_empty()).unwrap_or("converted_mod").to_owned();
    let import_modid = equipment_source_modid(source_roots.first().map(PathBuf::as_path).unwrap_or(Path::new(".")), source_modid);

    let discovered = discover_equipment_sources(source_roots)?;
    let mut report = EquipmentWorkspaceExportReport {
        workspace_file:workspace_file.clone(),
        discovered:discovered.len(),
        exported:0,
        skipped:0,
        records:Vec::new(),
        warnings:Vec::new(),
    };

    let mut used_names = HashSet::<String>::new();
    let mut existing_registry = HashSet::<(String,String)>::new();
    for descriptor in workspace.get("mod_elements").and_then(Value::as_array).into_iter().flatten() {
        if let Some(name) = descriptor.get("name").and_then(Value::as_str) {
            used_names.insert(name.to_ascii_lowercase());
        }
        let kind = descriptor.get("type").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        let registry = descriptor.get("registry_name").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        if !kind.is_empty() && !registry.is_empty() {
            existing_registry.insert((kind, registry));
        }
    }

    let output_elements = workspace_root.join("elements");
    let texture_dir = workspace_root.join("src/main/resources/assets").join(&target_modid).join("textures/item");
    fs::create_dir_all(&output_elements)?;
    fs::create_dir_all(&texture_dir)?;

    let mut templates = BTreeMap::<String, Value>::new();
    let mut copied_texture_names = BTreeSet::<String>::new();
    for source in &discovered {
        let kind = source.element_type.as_str();
        let registry = normalize_registry(&source.registry_name);
        if existing_registry.contains(&(kind.to_owned(), registry.clone())) {
            report.skipped += 1;
            continue;
        }
        let template = if let Some(template) = templates.get(kind) {
            template.clone()
        } else {
            let template = load_equipment_template(&workspace_root, kind)?;
            templates.insert(kind.to_owned(), template.clone());
            template
        };

        let candidates = texture_candidates(source_roots, &import_modid, source);
        let texture_source = candidates.first().cloned();
        let mut texture_name = registry.clone();
        let target_texture = if let Some(path) = texture_source.as_ref() {
            let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or(&registry);
            texture_name = stem.to_owned();
            let mut target = texture_dir.join(format!("{texture_name}.png"));
            if target.exists() && fs::read(&target)? != fs::read(path)? {
                texture_name = format!("{registry}_{texture_name}");
                target = texture_dir.join(format!("{texture_name}.png"));
            }
            fs::copy(path, &target).with_context(|| format!("Copying item texture {} to {}", path.display(), target.display()))?;
            crate::source_bridge::copy_animation_metadata(path, &target)?;
            target
        } else {
            let target = texture_dir.join(format!("{texture_name}.png"));
            if !target.exists() {
                fs::write(&target, placeholder_item_png())?;
            }
            report.warnings.push(format!("No source icon texture found for {}; generated placeholder '{}'.", source.class_name, target.file_name().unwrap_or_default().to_string_lossy()));
            target
        };

        let element_name = unique_name(
            &unique_equipment_element_name(&source.class_name, &registry),
            &mut used_names
        );
        let template_ref = Some(&template);
        let (element, mut warnings) = if kind == "tool" {
            let tool_type = source.class_name.to_ascii_lowercase();
            let tool_type = classify_tool(&source.superclass, &source.class_name)
                .or_else(|| {
                    ["Sword","Axe","Pickaxe","Shovel","Hoe","Bow","Fishing Rod","Shears"]
                        .into_iter().find(|kind| tool_type.ends_with(&kind.to_ascii_lowercase()))
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "Sword".to_owned());
            build_tool_element(
                template_ref,&source.class_name,&source.superclass,&source.source_text,
                &registry,&texture_name,&tool_type,None,imported_fv
            )
        } else {
            build_item_element(
                template_ref,&source.class_name,&source.superclass,&source.source_text,
                &registry,&texture_name,imported_fv
            )
        };
        if texture_source.is_none() {
            warnings.push("Source texture could not be recovered; verify the generated placeholder before release.".to_owned());
        }

        let element_path = output_elements.join(format!("{element_name}.mod.json"));
        fs::write(&element_path, format!("{}\n", serde_json::to_string_pretty(&element)?))
            .with_context(|| format!("Writing {}", element_path.display()))?;
        let texture_relative = target_texture.strip_prefix(&workspace_root).unwrap_or(&target_texture)
            .to_string_lossy().replace('\\', "/");
        let mut metadata_files = vec![texture_relative];
        let meta = PathBuf::from(format!("{}.mcmeta", target_texture.display()));
        if meta.is_file() {
            metadata_files.push(meta.strip_prefix(&workspace_root).unwrap_or(&meta).to_string_lossy().replace('\\', "/"));
        }
        let path_group = format!("~/{}/{}", import_modid.replace('-', "_"), if kind == "tool" { "Tools" } else { "Items" });
        let descriptor = json!({
            "name":element_name,
            "type":kind,
            "compiles":true,
            "locked_code":false,
            "registry_name":registry,
            "metadata":{"files":metadata_files},
            "path":path_group,
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":import_modid,
            "mojmodelbridge_source_class":source.class_name,
            "mojmodelbridge_source_path":source.source_path
        });
        workspace["mod_elements"].as_array_mut().expect("validated workspace mod_elements").push(descriptor);
        let display = item_display_name(&registry);
        equipment_language_entry(&mut workspace,&target_modid,kind,&registry,&element_name,&display);

        report.exported += 1;
        report.records.push(EquipmentExportRecord {
            class_name:source.class_name.clone(),
            element_name,
            registry_name:registry,
            element_type:kind.to_owned(),
            texture:texture_name,
            source_path:source.source_path.clone(),
            warnings,
        });
        existing_registry.insert((kind.to_owned(), source.registry_name.to_ascii_lowercase()));
    }

    fs::write(&workspace_file, format!("{}\n", serde_json::to_string_pretty(&workspace)?))
        .with_context(|| format!("Writing updated workspace {}", workspace_file.display()))?;
    let _ = copied_texture_names;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_native_item_schema_with_stack_and_damage_recovery() {
        let (element, warnings) = build_item_element(
            None, "ItemCrystal", "Item", "setMaxStackSize(16); setMaxDamage(32);",
            "crystal", "crystal.png", 89
        );
        assert!(warnings.is_empty());
        assert_eq!(element["_type"], "item");
        assert_eq!(element["_fv"], 89);
        assert_eq!(element["definition"]["name"], "Crystal");
        assert_eq!(element["definition"]["stackSize"], 16);
        assert_eq!(element["definition"]["damageCount"], 32);
        assert_eq!(element["definition"]["texture"], "crystal.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
    }

    #[test]
    fn caps_stack_size_and_preserves_template_specific_fields() {
        let template = json!({
            "_fv":88,"_type":"item",
            "definition":{
                "name":"Old","texture":"old.png","guiTexture":"old.png",
                "renderType":7,"customModelName":"OldModel",
                "creativeTabs":["BLOCKS"],"templateMarker":{"keep":true}
            }
        });
        let (element, _) = build_item_element(Some(&template), "ItemBigStack", "Item",
            "setMaxStackSize(128);", "big_stack", "new.png", 89);
        assert_eq!(element["_fv"], 88);
        assert_eq!(element["definition"]["stackSize"], 64);
        assert_eq!(element["definition"]["texture"], "new.png");
        assert_eq!(element["definition"]["guiTexture"], "new.png");
        assert_eq!(element["definition"]["renderType"], 0);
        assert_eq!(element["definition"]["customModelName"], "Normal");
        assert_eq!(element["definition"]["templateMarker"]["keep"], true);
    }

    #[test]
    fn recognizes_modern_food_properties_and_food_creative_tab() {
        let source = r#"
            public class ExampleFood extends Item {
                public ExampleFood() {
                    super(new Item.Properties().food(new FoodProperties.Builder()
                        .nutrition(6).saturationMod(0.8F).meat().alwaysEat().build()));
                }
            }
        "#;
        let (element, warnings) = build_item_element(None, "ExampleFood", "Item", source, "example_food", "food.png", 89);
        assert!(warnings.is_empty());
        assert_eq!(element["definition"]["isFood"], true);
        assert_eq!(element["definition"]["nutritionalValue"], 6);
        assert_eq!(element["definition"]["saturation"], 0.8);
        assert_eq!(element["definition"]["isMeat"], true);
        assert_eq!(element["definition"]["isAlwaysEdible"], true);
        assert_eq!(element["definition"]["creativeTabs"], json!(["FOOD"]));
    }

    #[test]
    fn recognizes_native_bow_behavior_and_projectile_trace() {
        let source = r#"
            public class OldBow extends Item {
                public UseAnim getUseAnimation() { return UseAnim.BOW; }
                public int getUseDuration() { return 72; }
                public void releaseUsing() { new CustomArrow(); }
            }
        "#;
        let (element, _) = build_item_element(None, "OldBow", "Item", source, "old_bow", "bow.png", 89);
        let definition = &element["definition"];
        assert_eq!(definition["enableRanged"], true);
        assert_eq!(definition["animation"], "bow");
        assert_eq!(definition["useDuration"], 72);
        assert_eq!(definition["projectile"], "Arrow");
        assert_eq!(definition["mojmodelbridge_source_projectile_class"], "CustomArrow");
    }

    #[test]
    fn classifies_legacy_and_modern_tools() {
        assert_eq!(classify_tool("ItemSword", "Example"), Some("Sword".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.PickaxeItem", "Custom"), Some("Pickaxe".to_owned()));
        assert_eq!(classify_tool("net.minecraft.world.item.Item", "ExampleBow"), Some("Bow".to_owned()));
        assert_eq!(classify_tool("LivingEntity", "EntityDragon"), None);
    }

    #[test]
    fn recovers_tool_durability_damage_enchantability_and_attack_speed() {
        let source = r#"
            public class TestSword extends ItemSword {
                private static final float weaponDamage = 7.0F;
                public int getEnchantmentValue() { return 14; }
                public TestSword() { super(2, -2.4F, new Properties()); }
            }
            setMaxDamage(900);
        "#;
        let material = ToolMaterialStats { efficiency:Some(8.0), ..Default::default() };
        let (element, warnings) = build_tool_element(
            None,"TestSword","ItemSword",source,"test_sword","test_sword.png","Sword",Some(&material),89
        );
        assert_eq!(element["definition"]["toolType"], "Sword");
        assert_eq!(element["definition"]["usageCount"], 900);
        assert_eq!(element["definition"]["damageVsEntity"], 7.0);
        assert_eq!(element["definition"]["efficiency"], 8.0);
        assert_eq!(element["definition"]["enchantability"], 14);
        assert_eq!(element["definition"]["attackSpeed"], 1.6);
        assert_eq!(element["definition"]["creativeTabs"], json!(["COMBAT"]));
        assert!(warnings.iter().any(|warning| warning.contains("repair ingredient")));
    }

    #[test]
    fn defaults_unrecovered_tool_stats_conservatively() {
        let (element, warnings) = build_tool_element(
            None,"UnknownPickaxe","Item","", "unknown_pickaxe", "", "Pickaxe", None,89
        );
        assert_eq!(element["definition"]["usageCount"], 250);
        assert_eq!(element["definition"]["damageVsEntity"], 3.0);
        assert_eq!(element["definition"]["efficiency"], 4.0);
        assert_eq!(element["definition"]["enchantability"], 10);
        assert_eq!(warnings.len(), 3);
    }
}
