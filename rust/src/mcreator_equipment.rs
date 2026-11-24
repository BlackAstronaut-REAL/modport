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

fn set_if_present(definition: &mut Map<String, Value>, key: &str, value: Value) {
    if definition.contains_key(key) {
        definition.insert(key.to_owned(), value);
    }
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
                if let Ok(value) = arguments[arguments.len() - 1].trim().trim_end_matches(['f','F']).parse::<f64>() {
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
    name: &str,
    registry: &str,
    mut updates: Map<String, Value>,
) -> Value {
    let mut definition = template_definition(template);
    let display = item_display_name(registry);
    updates.insert("name".to_owned(), json!(display));
    updates.entry("creativeTabs".to_owned()).or_insert_with(|| {
        json!(if kind == "tool" {
            if matches!(updates.get("toolType").and_then(Value::as_str), Some("Sword" | "Bow")) { ["COMBAT"] } else { ["TOOLS"] }
        } else {
            ["MISC"]
        })
    });
    for (key, value) in updates {
        if key == "name" && !definition.contains_key("name") {
            continue;
        }
        definition.insert(key, value);
    }
    json!({
        "_fv":template_version(template, imported_fv),
        "_type":kind,
        "definition":definition,
        "mojmodelbridge_element_name":name
    })
}

/// Build an MCreator item element from recovered source facts and a template element.
pub fn build_item_element(
    template: Option<&Value>,
    simple_class: &str,
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
