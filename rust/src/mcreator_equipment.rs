use crate::bytecode::{decode_instructions, parse_method_descriptor, ClassFile, ConstantValue, Instruction, JvmType, MemberInfo};
use rayon::prelude::*;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct ToolMaterialStats {
    pub uses: Option<i64>,
    pub efficiency: Option<f64>,
    pub damage: Option<f64>,
    pub enchantability: Option<i64>,
}

fn bytecode_number_constant(class: &ClassFile, instruction: &Instruction) -> Option<f64> {
    let opcode = instruction.opcode;
    match opcode {
        0x02..=0x08 => Some((opcode as i32 - 0x03) as f64),
        0x09 => Some(0.0),
        0x0a => Some(1.0),
        0x0b..=0x0d => Some((opcode - 0x0b) as f64),
        0x0e => Some(0.0),
        0x0f => Some(1.0),
        0x10 => instruction.operands.first().map(|value| *value as i8 as f64),
        0x11 => {
            let bytes = [*instruction.operands.first()?, *instruction.operands.get(1)?];
            Some(i16::from_be_bytes(bytes) as f64)
        }
        0x12..=0x14 => {
            let index = if opcode == 0x12 {
                *instruction.operands.first()? as u16
            } else {
                u16::from_be_bytes([*instruction.operands.first()?, *instruction.operands.get(1)?])
            };
            match class.constant(index)? {
                ConstantValue::Integer(value) => Some(value as f64),
                ConstantValue::Float(value) => Some(value as f64),
                ConstantValue::Long(value) => Some(value as f64),
                ConstantValue::Double(value) => Some(value),
                _ => None,
            }
        }
        _ => None,
    }
}

fn bytecode_method_number(class: &ClassFile, method: &MemberInfo) -> Option<f64> {
    let code = method.code.as_ref()?;
    let instructions = decode_instructions(&code.code).ok()?;
    if instructions.iter().any(|instruction| {
        matches!(instruction.opcode, 0x99..=0xab | 0xc6 | 0xc7)
    }) {
        return None;
    }

    let mut stack = Vec::<Option<f64>>::new();
    for instruction in &instructions {
        let opcode = instruction.opcode;
        if matches!(opcode, 0x01..=0x14) {
            stack.push(bytecode_number_constant(class, instruction));
            continue;
        }
        match opcode {
            0x00 => {}
            0x15..=0x2d => stack.push(None),
            0x2e..=0x35 => {
                stack.pop();
                stack.pop();
                stack.push(None);
            }
            0x36..=0x4e => { stack.pop(); }
            0x4f..=0x56 => {
                stack.pop();
                stack.pop();
                stack.pop();
            }
            0x57 => { stack.pop(); }
            0x59 => {
                if let Some(value) = stack.last().copied() {
                    stack.push(value);
                }
            }
            0x60..=0x73 => {
                let right = stack.pop().flatten();
                let left = stack.pop().flatten();
                let operation = (opcode - 0x60) / 4;
                let result = match (left, right) {
                    (Some(left), Some(right)) => match operation {
                        0 => Some(left + right),
                        1 => Some(left - right),
                        2 => Some(left * right),
                        3 if right != 0.0 => Some(left / right),
                        4 if right != 0.0 => Some(left % right),
                        _ => None,
                    },
                    _ => None,
                };
                stack.push(result);
            }
            0x74..=0x77 => {
                let value = stack.pop().flatten().map(|value| -value);
                stack.push(value);
            }
            0x78..=0x83 => {
                stack.pop();
                stack.pop();
                stack.push(None);
            }
            0x84 => {}
            0x85..=0x93 => {}
            0x94..=0x98 => {
                stack.pop();
                stack.pop();
                stack.push(None);
            }
            0xb2 => stack.push(None),
            0xb3 => { stack.pop(); }
            0xb4 => {
                stack.pop();
                stack.push(None);
            }
            0xb5 => {
                stack.pop();
                stack.pop();
            }
            0xb6..=0xb9 => {
                let index = u16::from_be_bytes([
                    *instruction.operands.first()?,
                    *instruction.operands.get(1)?,
                ]);
                let (_, _, descriptor) = class.member_reference(index)?;
                let descriptor = parse_method_descriptor(&descriptor).ok()?;
                for _ in 0..descriptor.parameters.len() {
                    stack.pop();
                }
                if opcode != 0xb8 {
                    stack.pop();
                }
                if !matches!(descriptor.return_type, JvmType::Void) {
                    stack.push(None);
                }
            }
            0xac..=0xaf => return stack.pop().flatten(),
            0xb0 | 0xb1 => return None,
            _ => {}
        }
    }
    None
}

/// Synthesize only named numeric getters from compiled classes. This reuses the
/// source-property parser downstream, without guessing from unrelated constants
/// or conflating a long use-duration getter with attack damage.
fn compiled_equipment_getter_stubs(class: &ClassFile) -> String {
    let getters: &[(&str, &[&str])] = &[
        ("getMaxDamage", &["getMaxDamage", "func_77612_l"]),
        ("getItemStackLimit", &["getItemStackLimit", "func_77639_j"]),
        ("getDamageVsEntity", &["getDamageVsEntity", "func_150931_i", "getAttackDamage"]),
        ("getItemEnchantability", &["getItemEnchantability", "getEnchantmentValue", "func_77619_b"]),
        ("getHealAmount", &["getHealAmount", "func_150905_g"]),
        ("getSaturationModifier", &["getSaturationModifier", "func_150906_h"]),
    ];
    let mut output = String::new();
    for method in &class.methods {
        let Some((canonical, _aliases)) = getters.iter()
            .find(|(_, aliases)| aliases.contains(&method.name.as_str()))
        else {
            continue;
        };
        let Some(value) = bytecode_method_number(class, method) else { continue; };
        output.push_str(&format!(
            "public double {canonical}() {{ return {:.8}; }}\n",
            value
        ));
    }

    // Compiled-only legacy items frequently keep their icon path solely in a
    // constant-pool String, not in a recoverable Java source file.
    for index in 1..class.constant_pool.len().min(u16::MAX as usize + 1) {
        let Some(ConstantValue::String(value)) = class.constant(index as u16) else { continue; };
        let normalized = value.replace('\\', "/");
        if normalized.to_ascii_lowercase().ends_with(".png") {
            if let Ok(literal) = serde_json::to_string(&normalized) {
                output.push_str(&format!("// TextureRef {literal}\n"));
            }
        }
    }

    // Forge 1.7/1.8 item icons are commonly registered inside registerIcons
    // using a plain resource key such as "modid:items/bertha" (no .png suffix).
    for method in &class.methods {
        let Some(code) = method.code.as_ref() else { continue; };
        let Ok(instructions) = decode_instructions(&code.code) else { continue; };
        let has_icon_registration = matches!(method.name.as_str(), "registerIcons" | "func_94581_a")
            || instructions.iter().any(|instruction| {
                if !matches!(instruction.opcode, 0xb6..=0xb9) || instruction.operands.len() < 2 {
                    return false;
                }
                let index = u16::from_be_bytes([instruction.operands[0], instruction.operands[1]]);
                class.member_reference(index).is_some_and(|(_, name, _)| {
                    matches!(name.as_str(), "registerIcon" | "func_94245_a")
                })
            });
        if !has_icon_registration { continue; }
        for instruction in &instructions {
            let constant_index = match instruction.opcode {
                0x12 => instruction.operands.first().copied().map(u16::from),
                0x13 | 0x14 if instruction.operands.len() >= 2 =>
                    Some(u16::from_be_bytes([instruction.operands[0], instruction.operands[1]])),
                _ => None,
            };
            let Some(constant_index) = constant_index else { continue; };
            let Some(ConstantValue::String(value)) = class.constant(constant_index) else { continue; };
            if value.trim().is_empty() { continue; }
            if let Ok(literal) = serde_json::to_string(&value.replace('\\', "/")) {
                output.push_str(&format!("registerIcon({literal});\n"));
            }
        }
    }
    output
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
    if let Some(value) = method_return_number(text, &["getHealAmount", "func_150905_g"]) {
        updates.insert("nutritionalValue".to_owned(), json!(value.round() as i64));
    } else if let Some(value) = regex_number(text, &[r"\.nutrition\s*\(\s*(\d+)\s*\)"]) {
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

    if let Some(value) = method_return_number(text, &["getSaturationModifier", "func_150906_h"]) {
        updates.insert("saturation".to_owned(), json!(value));
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
    if let Some(duration) = regex_number(text, &[r"\bgetUseDuration\s*\([^)]*\)\s*\{\s*return\s+(\d+)\s*;"]) {
        updates.insert("useDuration".to_owned(), json!(duration as i64));
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
        definition.insert("renderType".to_owned(), json!(0));
        definition.insert("customModelName".to_owned(), json!("Normal"));
    }
    (element, warnings)
}

/// Classify a source item subclass as one of MCreator's native tool types.
pub fn classify_tool(superclass: &str, simple_class: &str) -> Option<String> {
    let parent = superclass.to_ascii_lowercase();
    let simple = simple_class.to_ascii_lowercase();
    let item_style_class = simple.starts_with("item");
    let mappings = [
        ("itemaxe", "Axe"), ("itemsword", "Sword"), ("itempickaxe", "Pickaxe"),
        ("itemspade", "Shovel"), ("itemshovel", "Shovel"), ("itemhoe", "Hoe"),
        ("itemshears", "Shears"), ("itemfishingrod", "Fishing Rod"), ("itembow", "Bow"),
        ("itemmeleeweapon", "Sword"), ("itemweapon", "Sword"), ("weaponitem", "Sword"),
        ("itemrangedweapon", "Bow"), ("rangedweaponitem", "Bow"), ("itemranged", "Bow"),
        ("itemgun", "Bow"), ("itemblaster", "Bow"), ("itemlauncher", "Bow"), ("itemcannon", "Bow"),
        ("pickaxeitem", "Pickaxe"), ("sworditem", "Sword"), ("axeitem", "Axe"),
        ("shovelitem", "Shovel"), ("hoeitem", "Hoe"), ("bowitem", "Bow"),
        ("crossbowitem", "Bow"), ("fishingroditem", "Fishing Rod"), ("shearsitem", "Shears"),
    ];
    if let Some((_, kind)) = mappings.iter().find(|(token, _)| parent.contains(token)) {
        return Some((*kind).to_owned());
    }
    // Older mods often insert a custom ItemBase/WeaponBase between their concrete
    // class and Minecraft's Item. Keep the item-style class-name fallback even
    // when that intermediate parent is not present in the supplied roots.
    if !parent.is_empty() && parent != "item" && parent != "java.lang.object"
        && !parent.contains("item") && !item_style_class
    {
        return None;
    }
    for (suffix, kind) in [
        ("fishingrod", "Fishing Rod"), ("crossbow", "Bow"), ("blaster", "Bow"),
        ("launcher", "Bow"), ("cannon", "Bow"), ("rifle", "Bow"), ("pistol", "Bow"),
        ("shotgun", "Bow"), ("sniper", "Bow"), ("zapper", "Bow"), ("raygun", "Bow"),
        ("raygun", "Bow"), ("laser", "Bow"), ("bazooka", "Bow"), ("rocketlauncher", "Bow"),
        ("gun", "Bow"), ("pickaxe", "Pickaxe"), ("shears", "Shears"),
        ("shovel", "Shovel"), ("spade", "Shovel"),
        ("greatsword", "Sword"), ("longsword", "Sword"), ("shortsword", "Sword"),
        ("claymore", "Sword"), ("scimitar", "Sword"), ("cutlass", "Sword"),
        ("saber", "Sword"), ("sabre", "Sword"), ("maul", "Sword"),
        ("warhammer", "Sword"), ("battleaxe", "Axe"),
        ("sword", "Sword"), ("dagger", "Sword"), ("knife", "Sword"), ("katana", "Sword"),
        ("blade", "Sword"), ("spear", "Sword"), ("pike", "Sword"), ("trident", "Sword"),
        ("staff", "Sword"), ("wand", "Sword"), ("mace", "Sword"), ("club", "Sword"),
        ("fist", "Sword"), ("claw", "Sword"), ("gauntlet", "Sword"), ("knuckle", "Sword"),
        ("whip", "Sword"), ("flail", "Sword"), ("halberd", "Sword"), ("rapier", "Sword"),
        ("hammer", "Sword"), ("scythe", "Sword"), ("weapon", "Sword"),
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
        definition.insert("renderType".to_owned(), json!(0));
        definition.insert("customModelName".to_owned(), json!("Normal"));
        if definition.contains_key("blockingRenderType") {
            definition.insert("blockingRenderType".to_owned(), json!(0));
        }
        if definition.contains_key("blockingModelName") {
            definition.insert("blockingModelName".to_owned(), json!("Normal blocking"));
        }
    }
    (element, warnings)
}



use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct EquipmentSource {
    pub class_name: String,
    pub superclass: String,
    pub registry_name: String,
    pub element_type: String,
    pub tool_type: Option<String>,
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
    value.rsplit('.').next().unwrap_or(value).rsplit('$').next().unwrap_or(value).to_owned()
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
        r"\b((?:(?:public|protected|private|static|final|abstract)\s+)*)class\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>{}]*>)?\s+extends\s+([A-Za-z_$][A-Za-z0-9_$.]*)"
    ) else { return Vec::new() };
    regex.captures_iter(text).filter_map(|capture| {
        let modifiers = capture.get(1).map(|value| value.as_str()).unwrap_or("");
        Some((
            capture.get(2)?.as_str().to_owned(),
            simple_java_name(capture.get(3)?.as_str()),
            modifiers.split_whitespace().any(|part| part == "abstract"),
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
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' || bytes[index] == 36 {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' || bytes[index] == 36)
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
    if tail.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch as u32 == 36) {
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

fn collect_source_registries(texts: &[String]) -> BTreeMap<String, BTreeSet<String>> {
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

fn superclass_is_item(superclass: &str, class_parents: &BTreeMap<String, String>, depth: usize) -> bool {
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

fn inherited_tool_type(class: &str, superclass: &str, parents: &BTreeMap<String, String>) -> Option<String> {
    let mut current = superclass.to_owned();
    for _ in 0..12 {
        if let Some(kind) = classify_tool(&current, class) {
            return Some(kind);
        }
        let simple = simple_java_name(&current);
        let Some(next) = parents.get(&simple) else { break };
        current = next.clone();
    }
    classify_tool(superclass, class)
}

fn custom_weapon_tool_type(class: &str, source_text: &str) -> Option<String> {
    let normalized_name = normalize_texture_stem(class);
    let text = source_text.to_ascii_lowercase();
    if normalized_name == "bow" || normalized_name.ends_with("_bow")
        || normalized_name.ends_with("_crossbow")
        || text.contains("useanim.bow") || text.contains("useanim.crossbow")
    {
        return Some("Bow".to_owned());
    }
    // Legacy ranged weapons do not always use ItemBow or put "gun" in the
    // concrete class name. Look for projectile-launch behavior paired with
    // a standard item-use entry point, including SRG-era Minecraft methods.
    let projectile_behavior = [
        "entityarrow", "entityprojectile", "setthrowableheading", "shootprojectile",
        "fireprojectile", "spawnentity", "onplayerstoppedusing", "func_77615_a",
        "getprojectile", "projectileitem",
    ].iter().any(|token| text.contains(token));
    let ranged_item_use = [
        "onitemrightclick", "func_77659_a", "getmaxitemuseduration",
        "getitemuseaction", "onplayerstoppedusing", "func_77615_a",
    ].iter().any(|token| text.contains(token));
    if projectile_behavior && ranged_item_use {
        return Some("Bow".to_owned());
    }
    let damage_signal = [
        "getdamagevsentity", "func_150931_i", "weapondamage",
        "damagevsentity", "attackdamage", "getattackdamage",
    ].iter().any(|token| text.contains(token));
    let combat_signal = [
        "hitentity", "onleftclickentity", "onentityswing", "attackentityfrom",
        "hurtenemy", "onattack", "onentityhit", "getdamagevsentity", "func_150931_i",
    ].iter().any(|token| text.contains(token));
    if damage_signal && combat_signal {
        return Some("Sword".to_owned());
    }
    let attack_attribute_signal = [
        "attackdamage", "generic.attack_damage", "generic_attack_damage",
        "weapondamage", "getdamagevsentity", "func_150931_i",
    ].iter().any(|token| text.contains(token));
    let melee_signal = [
        "hitentity", "onleftclickentity", "onentityswing", "onitemattack",
        "hurtentity", "meleeattack", "dealmeleedamage",
    ].iter().any(|token| text.contains(token));
    let named_weapon_base = [
        "itemweapon", "meleeweapon", "weaponbase", "weaponitem", "combatitem",
        "itemmelee", "meleeitem",
    ].iter().any(|token| text.contains(token));
    if attack_attribute_signal && (melee_signal || named_weapon_base) {
        return Some("Sword".to_owned());
    }
    None
}

fn equipment_kind(
    class: &str,
    superclass: &str,
    parents: &BTreeMap<String, String>,
    source_text: &str,
) -> Option<(String, Option<String>)> {
    let simple = class.to_ascii_lowercase();
    let parent = simple_java_name(superclass).to_ascii_lowercase();
    if (parent.is_empty() || parent == "object" || parent == "java.lang.object")
        && (simple.ends_with("init") || simple.ends_with("registry") || simple.ends_with("registrar"))
    {
        return None;
    }
    if superclass.to_ascii_lowercase().contains("itemarmor")
        || class.to_ascii_lowercase().ends_with("armor")
    {
        return None;
    }
    if let Some(tool) = inherited_tool_type(class, superclass, parents) {
        return Some(("tool".to_owned(), Some(tool)));
    }
    let item_like = superclass_is_item(superclass, parents, 0) || simple.starts_with("item");
    if item_like {
        if let Some(tool) = custom_weapon_tool_type(class, source_text) {
            return Some(("tool".to_owned(), Some(tool)));
        }
        return Some(("item".to_owned(), None));
    }
    None
}

fn parse_material_numbers(arguments: &[String]) -> Option<[f64; 5]> {
    let numeric = Regex::new(r"^\s*(?:\(\s*float\s*\)\s*)?(-?\d+(?:\.\d+)?)[fFdD]?\s*$").ok()?;
    let mut values = Vec::<f64>::new();
    for argument in arguments {
        if let Some(capture) = numeric.captures(argument) {
            let value = capture.get(1)?.as_str().parse::<f64>().ok()?;
            if value.is_finite() {
                values.push(value);
            }
        }
    }
    if values.len() < 5 {
        return None;
    }
    let tail = &values[values.len() - 5..];
    Some([tail[0], tail[1], tail[2], tail[3], tail[4]])
}

fn material_stats_from_values(values: [f64; 5]) -> ToolMaterialStats {
    ToolMaterialStats {
        uses: Some(values[1].round() as i64),
        efficiency: Some(values[2]),
        damage: Some(values[3]),
        enchantability: Some(values[4].round() as i64),
    }
}

/// Recover legacy get_*stats objects and addToolMaterial fields, then associate
/// each material with registered Item/Tool fields that pass it to constructors.
fn compiled_equipment_field_name(class: &ClassFile, instruction: &Instruction) -> Option<String> {
    if !matches!(instruction.opcode, 0xb2 | 0xb3) { return None; }
    let index = u16::from_be_bytes([
        *instruction.operands.first()?,
        *instruction.operands.get(1)?,
    ]);
    class.member_reference(index).map(|(_, name, _)| name)
}

fn compiled_equipment_new_class(class: &ClassFile, instruction: &Instruction) -> Option<String> {
    if instruction.opcode != 0xbb { return None; }
    let index = u16::from_be_bytes([
        *instruction.operands.first()?,
        *instruction.operands.get(1)?,
    ]);
    class.class_name(index).map(|name| name.rsplit('/').next().unwrap_or(name).to_owned())
}

fn compiled_tool_numbers_before(
    class: &ClassFile,
    instructions: &[Instruction],
    call_index: usize,
) -> Vec<f64> {
    let start = call_index.saturating_sub(80);
    let mut values = Vec::new();
    for instruction in instructions[start..call_index].iter().rev() {
        if let Some(value) = bytecode_number_constant(class, instruction) {
            if values.len() < 5 { values.push(value); }
        }
        if values.len() == 5 { break; }
    }
    values.reverse();
    values
}

fn compiled_tool_stats_from_numbers(values: &[f64]) -> Option<ToolMaterialStats> {
    if values.len() < 5 { return None; }
    Some(ToolMaterialStats {
        uses: Some(values[1].round() as i64),
        efficiency: Some(values[2]),
        damage: Some(values[3]),
        enchantability: Some(values[4].round() as i64),
    })
}

fn compiled_next_static_field(
    class: &ClassFile,
    instructions: &[Instruction],
    start: usize,
    limit: usize,
) -> Option<String> {
    instructions.iter().skip(start + 1).take(limit).find_map(|instruction| {
        if instruction.opcode != 0xb3 { return None; }
        compiled_equipment_field_name(class, instruction)
    })
}

/// Recover get_weaponstats -> addToolMaterial -> registered Item field links
/// from classfiles. This is the compiled-only counterpart to source_tool_materials.
pub fn compiled_tool_materials(source_roots: &[PathBuf]) -> Result<BTreeMap<String, ToolMaterialStats>> {
    let compiled = crate::compiled_registry::scan_compiled_registrations(source_roots)?;
    let class_infos: Vec<_> = compiled.classes.values().collect();
    let per_class: Vec<BTreeMap<String, ToolMaterialStats>> = class_infos.par_iter().map(|info| {
        let Some(class) = compiled.parsed_class_paths.get(&info.path) else {
            return BTreeMap::new();
        };
        let class = class.as_ref();
        let mut class_results = BTreeMap::<String, ToolMaterialStats>::new();

        // These maps deliberately outlive an individual method. Old Forge mods
        // often initialize stat objects, register tool materials, instantiate
        // their items, and register those items in separate lifecycle methods.
        let mut stat_fields = BTreeMap::<String, ToolMaterialStats>::new();
        let mut material_fields = BTreeMap::<String, ToolMaterialStats>::new();
        let mut class_item_fields = BTreeMap::<String, String>::new();
        let mut class_item_materials = BTreeMap::<String, ToolMaterialStats>::new();

        for method in &class.methods {
            let Some(code) = method.code.as_ref() else { continue; };
            let Ok(instructions) = decode_instructions(&code.code) else { continue; };

            // First pass: recover helper-generated weapon-stat objects.
            for (index, instruction) in instructions.iter().enumerate() {
                if !matches!(instruction.opcode, 0xb6..=0xb9) { continue; }
                let Some(operands) = instruction.operands.get(0..2) else { continue; };
                let cp_index = u16::from_be_bytes([operands[0], operands[1]]);
                let Some((_, method_name, _)) = class.member_reference(cp_index) else { continue; };
                if !Regex::new(r"(?i)(?:get|load)_?weaponstats$")
                    .unwrap().is_match(&method_name) {
                    continue;
                }
                let values = compiled_tool_numbers_before(&class, &instructions, index);
                let target = compiled_next_static_field(&class, &instructions, index, 12);
                if let (Some(stats), Some(target)) = (
                    compiled_tool_stats_from_numbers(&values),
                    target,
                ) {
                    stat_fields.insert(target.clone(), stats.clone());
                    class_results.entry(compact_name(&target)).or_insert(stats);
                }
            }

            // Second pass: bind stat objects (or direct numeric arguments) to
            // addToolMaterial fields, allowing the stat object to have been
            // created by a different method in the same owner class.
            for (index, instruction) in instructions.iter().enumerate() {
                if !matches!(instruction.opcode, 0xb6..=0xb9) { continue; }
                let Some(operands) = instruction.operands.get(0..2) else { continue; };
                let cp_index = u16::from_be_bytes([operands[0], operands[1]]);
                let Some((_, method_name, _)) = class.member_reference(cp_index) else { continue; };
                if method_name != "addToolMaterial" { continue; }

                let target = compiled_next_static_field(&class, &instructions, index, 12);
                let start = index.saturating_sub(60);
                let referenced = instructions[start..index].iter().rev()
                    .filter_map(|prior| {
                        if prior.opcode != 0xb2 { return None; }
                        compiled_equipment_field_name(&class, prior)
                    })
                    .find_map(|field| stat_fields.get(&field).cloned());
                let stats = referenced.or_else(|| {
                    compiled_tool_stats_from_numbers(&compiled_tool_numbers_before(&class, &instructions, index))
                });
                if let (Some(target), Some(stats)) = (target, stats) {
                    material_fields.insert(target.clone(), stats.clone());
                    class_results.entry(compact_name(&target)).or_insert(stats);
                }
            }

            // Third pass: keep item field -> concrete class links even when the
            // material is assigned in another method.
            for (index, instruction) in instructions.iter().enumerate() {
                if instruction.opcode != 0xb3 { continue; }
                let Some(target) = compiled_equipment_field_name(&class, instruction) else { continue; };
                let start = index.saturating_sub(180);
                let mut new_index = None;
                for probe in (start..index).rev() {
                    if instructions[probe].opcode == 0xbb {
                        new_index = Some(probe);
                        break;
                    }
                    if instructions[probe].opcode == 0xb3 {
                        break;
                    }
                }
                let Some(new_index) = new_index else { continue; };
                let Some(item_class) = compiled_equipment_new_class(&class, &instructions[new_index]) else {
                    continue;
                };
                class_item_fields.insert(target, item_class);
            }
        }

        // Fourth pass: recover which material fields each constructed item uses.
        // Scan every method so constructors and material definitions need not share
        // a single <clinit> method or source lifecycle callback.
        for method in &class.methods {
            let Some(code) = method.code.as_ref() else { continue; };
            let Ok(instructions) = decode_instructions(&code.code) else { continue; };
            for (index, instruction) in instructions.iter().enumerate() {
                if instruction.opcode != 0xb3 { continue; }
                let Some(target) = compiled_equipment_field_name(&class, instruction) else { continue; };
                if !class_item_fields.contains_key(&target) { continue; }
                let start = index.saturating_sub(180);
                let stats = instructions[start..index].iter().rev()
                    .filter_map(|prior| {
                        if prior.opcode != 0xb2 { return None; }
                        compiled_equipment_field_name(&class, prior)
                    })
                    .find_map(|field| material_fields.get(&field).cloned());
                if let Some(stats) = stats {
                    class_item_materials.insert(target, stats);
                }
            }
        }

        // Registry identifiers are indexed by concrete class in the shared
        // compiled registry pass, so construction and registration can live in
        // separate methods without losing the relationship.
        for (item_field, item_class) in class_item_fields {
            // The constructor's material field is the strongest association. Only
            // use a single-material fallback when the bytecode lacks that reference.
            let stats = class_item_materials.get(&item_field).cloned()
                .or_else(|| {
                    if material_fields.len() == 1 {
                        material_fields.values().next().cloned()
                    } else {
                        None
                    }
                });
            let Some(stats) = stats else { continue; };
            if let Some(registries) = compiled.registries.get(&item_class) {
                for registry in registries {
                    let key = compact_name(&normalize_registry(registry));
                    if !key.is_empty() {
                        class_results.insert(key, stats.clone());
                    }
                }
            }
            class_results.entry(compact_name(&item_class)).or_insert(stats);
        }
        class_results
    }).collect();
    let mut results = BTreeMap::<String, ToolMaterialStats>::new();
    for partial in per_class {
        results.extend(partial);
    }
    Ok(results)
}

pub fn source_tool_materials(
    source_roots: &[PathBuf],
    source_modid: &str,
) -> BTreeMap<String, ToolMaterialStats> {
    use crate::source::java::split_top_level;

    let files = java_files(source_roots);
    let texts: Vec<String> = files.par_iter()
        .filter_map(|path| fs::read(path).ok().map(|bytes| {
            crate::source::java::strip_java_comments(&String::from_utf8_lossy(&bytes))
        }))
        .collect();
    let combined = texts.join("\n");
    let stat_call = Regex::new(
        r"(?s)\b([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:this\s*\.\s*)?(?:get|load)_[A-Za-z0-9_]*stats\s*\((.*?)\)\s*;"
    ).expect("static tool stat expression");
    let mut stat_objects = BTreeMap::<String, ToolMaterialStats>::new();
    for capture in stat_call.captures_iter(&combined) {
        let field = capture.get(1).map(|value| value.as_str()).unwrap_or("");
        let arguments = split_top_level(capture.get(2).map(|value| value.as_str()).unwrap_or(""), ',');
        if let Some(values) = parse_material_numbers(&arguments) {
            stat_objects.insert(field.to_owned(), material_stats_from_values(values));
        }
    }

    let material_call = Regex::new(
        r"(?s)\b([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:EnumHelper\s*\.\s*)?addToolMaterial\s*\((.*?)\)\s*;"
    ).expect("addToolMaterial expression");
    let mut material_fields = BTreeMap::<String, ToolMaterialStats>::new();
    for capture in material_call.captures_iter(&combined) {
        let field = capture.get(1).map(|value| value.as_str()).unwrap_or("");
        let raw = capture.get(2).map(|value| value.as_str()).unwrap_or("");
        let arguments = split_top_level(raw, ',');
        let referenced = Regex::new(r"\b([A-Za-z_$][A-Za-z0-9_$]*)\s*\.\s*(?:harvestLevel|maxUses|efficiency|damage|enchantability)\b")
            .expect("tool stat field reference")
            .captures_iter(raw)
            .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
            .collect::<Vec<_>>();
        let stats = referenced.iter().rev().find_map(|name| stat_objects.get(name).cloned())
            .or_else(|| parse_material_numbers(&arguments).map(material_stats_from_values));
        if let Some(stats) = stats {
            material_fields.insert(field.to_owned(), stats);
        }
    }

    let registered_fields = crate::recipe_bridge::source_field_registered_names(source_roots, source_modid);
    let mut results = BTreeMap::<String, ToolMaterialStats>::new();
    let assignments = texts.iter().flat_map(|text| source_constructor_assignments(text)).collect::<Vec<_>>();
    for (field, _class, arguments) in &assignments {
        let tokens = Regex::new(r"\b[A-Za-z_$][A-Za-z0-9_$]*\b")
            .expect("Java identifier")
            .find_iter(arguments)
            .map(|matched| matched.as_str())
            .collect::<BTreeSet<_>>();
        let Some(stats) = tokens.iter().find_map(|token| material_fields.get(*token)) else {
            continue;
        };
        if let Some(registries) = registered_fields.get(field) {
            for registry in registries {
                results.insert(compact_name(registry), stats.clone());
            }
        }
    }

    // Modern deferred-register item factories usually expose the material token
    // directly in their constructor lambda rather than through a static field assignment.
    let modern = Regex::new(
        r#"(?s)\b(?:ITEMS|itemRegistry|REGISTRY)\s*\.\s*register\s*\(\s*"([^"]+)"\s*,\s*\(\s*\)\s*->\s*new\s+[A-Za-z_$][A-Za-z0-9_$.]*\s*\(([^)]*)\)"#
    ).expect("modern item registry factory");
    for capture in modern.captures_iter(&combined) {
        let registry = normalize_registry(capture.get(1).map(|value| value.as_str()).unwrap_or(""));
        let arguments = capture.get(2).map(|value| value.as_str()).unwrap_or("");
        let tokens = Regex::new(r"\b[A-Za-z_$][A-Za-z0-9_$]*\b")
            .expect("Java identifier")
            .find_iter(arguments)
            .map(|matched| matched.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(stats) = tokens.iter().find_map(|token| material_fields.get(*token)) {
            results.insert(compact_name(&registry), stats.clone());
        }
    }

    // Fallback for older fields whose names encode a material, e.g. rubyPickaxe
    // resolving to ruby_stats when constructor args are obscured by helper calls.
    for (field, registries) in &registered_fields {
        let normalized = compact_name(field);
        let candidate = stat_objects.iter()
            .filter_map(|(name, stats)| {
                let key = compact_name(name.trim_end_matches("_stats"));
                if !key.is_empty() && normalized.contains(&key) {
                    Some((key.len(), stats))
                } else {
                    None
                }
            })
            .max_by_key(|(length, _)| *length)
            .map(|(_, stats)| stats.clone());
        if let Some(stats) = candidate {
            for registry in registries {
                results.entry(compact_name(registry)).or_insert_with(|| stats.clone());
            }
        }
    }
    results
}

/// Discover concrete source- and bytecode-registered item/tool subclasses.
/// Compiled-only classes also contribute any named numeric getters that can be
/// safely evaluated from their JVM method bodies.
pub fn discover_equipment_sources(source_roots: &[PathBuf]) -> Result<Vec<EquipmentSource>> {
    use crate::source::java::strip_java_comments;

    let files = java_files(source_roots);
    let mut texts = Vec::<String>::new();
    let mut class_paths = BTreeMap::<String, PathBuf>::new();
    let mut class_texts = BTreeMap::<String, String>::new();
    let mut class_parents = BTreeMap::<String, String>::new();
    let mut abstract_classes = HashSet::<String>::new();

    let parsed_sources: Vec<(std::path::PathBuf, String, Vec<(String, String, bool)>)> = files.par_iter()
        .filter_map(|path| {
            let bytes = fs::read(path).ok()?;
            let text = strip_java_comments(&String::from_utf8_lossy(&bytes));
            let headers = source_class_headers(&text);
            Some((path.clone(), text, headers))
        })
        .collect();
    // Rayon preserves indexed-iterator order for collect(), so merge ordering and
    // duplicate-class precedence remain the same as the original sorted file list.
    for (path, text, headers) in parsed_sources {
        for (class, parent, is_abstract) in headers {
            class_paths.entry(class.clone()).or_insert_with(|| path.clone());
            class_texts.entry(class.clone()).or_insert_with(|| text.clone());
            class_parents.insert(class.clone(), parent);
            if is_abstract {
                abstract_classes.insert(class);
            }
        }
        texts.push(text);
    }

    let compiled = crate::compiled_registry::scan_compiled_registrations(source_roots)?;
    let mut compiled_texts = HashMap::<String, String>::new();
    // Build the simple-name fallback map from retained classes in one linear pass.
    for parsed in compiled.parsed_classes.values() {
        let internal = parsed.this_name().unwrap_or("");
        let simple = internal.rsplit('/').next().unwrap_or("").rsplit('$').next().unwrap_or("");
        if !simple.is_empty() {
            compiled_texts.entry(simple.to_owned())
                .or_insert_with(|| compiled_equipment_getter_stubs(parsed));
        }
    }
    for (class, info) in &compiled.classes {
        class_paths.entry(class.clone()).or_insert_with(|| info.path.clone());
        class_parents.entry(class.clone()).or_insert_with(|| info.superclass.clone());
        if info.access_flags & 0x0400 != 0 {
            abstract_classes.insert(class.clone());
        }
    }

    let mut registry_map = collect_source_registries(&texts);
    for (class, registries) in &compiled.registries {
        registry_map.entry(class.clone()).or_default().extend(registries.iter().cloned());
    }
    let mut output = Vec::new();
    let mut seen = BTreeSet::<(String, String)>::new();
    for (class, registries) in registry_map {
        if abstract_classes.contains(&class) {
            continue;
        }
        let Some(source_path) = class_paths.get(&class) else { continue };
        let mut source_text = class_texts.get(&class).cloned()
            .filter(|text| !text.trim().is_empty())
            .or_else(|| compiled_texts.get(&class).cloned())
            .unwrap_or_default();
        let Some(superclass) = class_parents.get(&class) else { continue };

        // Weapon/item behavior and icon paths are frequently declared once in
        // an abstract/custom base, while concrete Lost Infinity-style subclasses
        // only select stats or constructor arguments. Include the ancestor chain
        // for classification and texture discovery without changing registry IDs.
        let mut parent_name = simple_java_name(superclass);
        let mut visited_parents = HashSet::<String>::new();
        for _ in 0..12 {
            if parent_name.is_empty() || !visited_parents.insert(parent_name.clone()) {
                break;
            }
            if let Some(parent_source) = class_texts.get(&parent_name)
                .filter(|text| !text.trim().is_empty())
                .or_else(|| compiled_texts.get(&parent_name))
            {
                source_text.push_str("\n");
                source_text.push_str(parent_source);
            }
            let Some(next) = class_parents.get(&parent_name) else { break };
            parent_name = simple_java_name(next);
        }
        let Some((element_type, tool_type)) = equipment_kind(&class, superclass, &class_parents, &source_text) else { continue };

        for registry in registries {
            if !seen.insert((class.clone(), registry.clone())) {
                continue;
            }
            output.push(EquipmentSource {
                class_name: class.clone(),
                superclass: superclass.clone(),
                registry_name: registry,
                element_type: element_type.clone(),
                tool_type: tool_type.clone(),
                source_path: source_path.to_string_lossy().replace('\\', "/"),
                source_text: source_text.clone(),
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

fn normalize_texture_stem(value: &str) -> String {
    let mut output = String::new();
    let mut previous = '\0';
    for ch in value.chars() {
        if ch.is_ascii_uppercase() {
            if previous.is_ascii_lowercase() || previous.is_ascii_digit() {
                output.push('_');
            }
            output.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else if !output.is_empty() && !output.ends_with('_') {
            output.push('_');
        }
        previous = ch;
    }
    output.trim_matches('_').to_owned()
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
    // Old renderer/item sources frequently keep the icon path only as a string
    // constant (or in a ResourceLocation), not as an explicit setTextureName call.
    let patterns = [
        r#"(?i)"([^"\n]+\.png)""#,
        r#"\b(?:setTextureName|func_111206_d|registerIcon|func_94245_a)\s*\(\s*"([^"]+)""#,
        r#"new\s+ResourceLocation\s*\(\s*"([^"]+)"(?:\s*,\s*"([^"]+)")?"#,
    ];
    let mut refs = Vec::<String>::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let Ok(regex) = Regex::new(pattern) else { continue };
        for capture in regex.captures_iter(source_text) {
            let raw = if index == 2 {
                capture.get(2).or_else(|| capture.get(1)).map(|value| value.as_str())
            } else {
                capture.get(1).map(|value| value.as_str())
            };
            let Some(raw) = raw else { continue };
            if !refs.iter().any(|value| value.eq_ignore_ascii_case(raw)) {
                refs.push(raw.to_owned());
            }
        }
    }
    refs
}

#[derive(Debug, Clone)]
struct EquipmentTextureFile {
    path: PathBuf,
    normalized_stem: String,
    compact_stem: String,
    lower_path: String,
    root_priority: usize,
}

#[derive(Debug, Clone, Default)]
struct EquipmentTextureIndex {
    files: Vec<EquipmentTextureFile>,
    /// Exact normalized and compact item texture names -> indexes into files.
    by_normalized_stem: HashMap<String, Vec<usize>>,
    by_compact_stem: HashMap<String, Vec<usize>>,
    item_model_refs: HashMap<String, Vec<String>>,
}

impl EquipmentTextureIndex {
    fn new(source_roots: &[PathBuf]) -> Self {
        let mut index = Self::default();
        let mut seen = HashSet::<String>::new();

        for (root_priority, root) in source_roots.iter().enumerate() {
            if !root.is_dir() {
                continue;
            }
            for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
                if extension.eq_ignore_ascii_case("png") {
                    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                    let key = canonical.to_string_lossy().to_ascii_lowercase();
                    if seen.insert(key.clone()) {
                        let normalized_stem = normalize_texture_stem(
                            path.file_stem().and_then(|value| value.to_str()).unwrap_or("")
                        );
                        let compact_stem = compact_name(&normalized_stem);
                        index.files.push(EquipmentTextureFile {
                            path: canonical,
                            normalized_stem,
                            compact_stem,
                            lower_path: key,
                            root_priority,
                        });
                    }
                } else if extension.eq_ignore_ascii_case("json")
                    && path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/models/item/")
                {
                    let Ok(bytes) = fs::read(path) else { continue };
                    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else { continue };
                    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else { continue };
                    let key = compact_name(stem);
                    if key.is_empty() {
                        continue;
                    }
                    let Some(textures) = payload.get("textures").and_then(Value::as_object) else { continue };
                    let reference = textures.get("layer0").and_then(Value::as_str)
                        .or_else(|| textures.values().find_map(Value::as_str));
                    if let Some(reference) = reference.filter(|value| !value.trim().is_empty()) {
                        let refs = index.item_model_refs.entry(key).or_default();
                        if !refs.iter().any(|value| value.eq_ignore_ascii_case(reference)) {
                            refs.push(reference.to_owned());
                        }
                    }
                }
            }
        }
        index.files.sort_by(|a, b| {
            a.root_priority.cmp(&b.root_priority)
                .then_with(|| a.lower_path.cmp(&b.lower_path))
        });
        // Resolve the common exact and separator-insensitive paths through maps.
        // The old resolver scanned every PNG for every registered item even
        // though the source tree itself had already been indexed once.
        for (file_index, file) in index.files.iter().enumerate() {
            if !file.normalized_stem.is_empty() {
                index.by_normalized_stem.entry(file.normalized_stem.clone())
                    .or_default().push(file_index);
            }
            if !file.compact_stem.is_empty() {
                index.by_compact_stem.entry(file.compact_stem.clone())
                    .or_default().push(file_index);
            }
        }
        index
    }

    fn candidates(&self, stems: &[String]) -> Vec<PathBuf> {
        let normalized_targets: BTreeSet<String> = stems.iter()
            .map(|stem| normalize_texture_stem(stem))
            .filter(|stem| !stem.is_empty())
            .collect();
        let compact_targets: BTreeSet<String> = normalized_targets.iter()
            .map(|stem| compact_name(stem))
            .filter(|stem| !stem.is_empty())
            .collect();
        if normalized_targets.is_empty() {
            return Vec::new();
        }

        let mut candidate_indices = BTreeSet::<usize>::new();
        for target in &normalized_targets {
            if let Some(indices) = self.by_normalized_stem.get(target) {
                candidate_indices.extend(indices.iter().copied());
            }
        }
        for target in &compact_targets {
            if let Some(indices) = self.by_compact_stem.get(target) {
                candidate_indices.extend(indices.iter().copied());
            }
        }

        let mut scored = Vec::<(i32, usize, String, PathBuf)>::new();
        for file_index in candidate_indices {
            let file = &self.files[file_index];
            let exact = normalized_targets.contains(&file.normalized_stem);
            let compact = !exact && compact_targets.contains(&file.compact_stem);
            let mut score = if exact { 1200 } else if compact { 1100 } else { 0 };
            if file.lower_path.contains("/textures/item/") || file.lower_path.contains("/textures/items/") {
                score += 60;
            }
            scored.push((score, file.root_priority, file.lower_path.clone(), file.path.clone()));
        }

        // Fuzzy prefix matching is intentionally a last resort. It used to run
        // over the complete PNG list for every item even when an exact name
        // already existed; exact/compact indexed matches always outrank it.
        if scored.is_empty() {
            for file in &self.files {
                let mut score = 0;
                for target in &compact_targets {
                    let shortest = file.compact_stem.len().min(target.len());
                    let longest = file.compact_stem.len().max(target.len());
                    if shortest >= 6 && longest > 0 && shortest as f64 / longest as f64 >= 0.82
                        && (file.compact_stem.starts_with(target) || target.starts_with(&file.compact_stem))
                    {
                        score = score.max(500 - longest.saturating_sub(shortest) as i32 * 20);
                    }
                }
                if score == 0 {
                    continue;
                }
                if file.lower_path.contains("/textures/item/") || file.lower_path.contains("/textures/items/") {
                    score += 60;
                }
                scored.push((score, file.root_priority, file.lower_path.clone(), file.path.clone()));
            }
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2)));
        scored.into_iter().map(|(_, _, _, path)| path).collect()
    }
}

fn texture_candidates(index: &EquipmentTextureIndex, source: &EquipmentSource) -> Vec<PathBuf> {
    let mut stems = Vec::<String>::new();
    let mut model_keys = BTreeSet::from([
        compact_name(&source.registry_name),
        compact_name(&source.class_name),
    ]);
    if let Some(stripped) = source.class_name.strip_prefix("Item") {
        model_keys.insert(compact_name(stripped));
    }
    for key in model_keys {
        if let Some(model_refs) = index.item_model_refs.get(&key) {
            stems.extend(model_refs.iter().filter_map(|reference| texture_stem_from_reference(reference)));
        }
    }
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
    index.candidates(&stems)
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
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { continue };
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

fn equipment_language_entry(workspace: &mut Value, modid: &str, registry: &str, element_name: &str, display_name: &str) {
    if !workspace.get("language_map").is_some_and(Value::is_object) {
        workspace["language_map"] = json!({});
    }
    if !workspace["language_map"].get("en_us").is_some_and(Value::is_object) {
        workspace["language_map"]["en_us"] = json!({});
    }
    let language = workspace["language_map"]["en_us"].as_object_mut().expect("language map initialized");
    for key in [
        format!("item.{modid}.{registry}"),
        format!("item.{modid}.{registry}.name"),
        format!("item.{modid}.{element_name}"),
        format!("item.{modid}.{element_name}.name"),
    ] {
        language.insert(key, json!(display_name));
    }
}

fn source_import_folder(modid: &str, kind: &str) -> String {
    let normalized = modid.trim().replace('-', "_");
    let root = if normalized.eq_ignore_ascii_case("lostinfinity") || normalized.eq_ignore_ascii_case("lost_infinity") {
        "LostInfinity".to_owned()
    } else if normalized.is_empty() {
        "converted_mod".to_owned()
    } else {
        normalized
    };
    format!("~/{}/{}", root, if kind == "tool" { "Tools" } else { "Items" })
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

/// Discover registered Java Item/Tool subclasses, materialize their item textures,
/// write element JSON and update the existing workspace descriptor/language map.
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
    if !workspace.get("workspaceSettings").is_some_and(Value::is_object)
        || !workspace.get("mod_elements").is_some_and(Value::is_array)
    {
        bail!("Workspace must contain object workspaceSettings and array mod_elements.");
    }
    let target_modid = workspace.get("workspaceSettings").and_then(|settings| settings.get("modid"))
        .and_then(Value::as_str).filter(|value| !value.trim().is_empty()).unwrap_or("converted_mod").to_owned();
    let import_modid = if !source_modid.trim().is_empty() {
        source_modid.trim().to_owned()
    } else {
        source_roots.first().and_then(|root| root.file_name()).and_then(|value| value.to_str())
            .unwrap_or("converted_mod").to_owned()
    };
    let field_registry_map = crate::recipe_bridge::source_field_registered_names(source_roots, &import_modid);
    let localization = crate::localization::LocalizationCatalog::load(source_roots, &field_registry_map);
    let creative_tabs = crate::localization::CreativeTabCatalog::load(source_roots);

    let discovered = discover_equipment_sources(source_roots)?;
    // Reuse one icon/model-resource index for the whole equipment batch. The
    // previous per-item WalkDir made texture matching scale as item_count × files.
    let texture_index = EquipmentTextureIndex::new(source_roots);
    let mut tool_materials = compiled_tool_materials(source_roots)?;
    // Decompiled source takes precedence over bytecode-derived material values.
    tool_materials.extend(source_tool_materials(source_roots, &import_modid));
    let mut report = EquipmentWorkspaceExportReport {
        workspace_file:workspace_file.clone(), discovered:discovered.len(), exported:0, skipped:0,
        records:Vec::new(), warnings:Vec::new(),
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

        let candidates = texture_candidates(&texture_index, source);
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
        let (mut element, mut warnings) = if kind == "tool" {
            let tool_type = source.tool_type.as_deref().unwrap_or("Sword");
            build_tool_element(
                Some(&template),&source.class_name,&source.superclass,&source.source_text,
                &registry,&texture_name,tool_type,tool_materials.get(&compact_name(&registry)),imported_fv
            )
        } else {
            build_item_element(
                Some(&template),&source.class_name,&source.superclass,&source.source_text,
                &registry,&texture_name,imported_fv
            )
        };
        let display = localization.translated_name(&registry, &item_display_name(&registry));
        if let Some(definition) = element.get_mut("definition").and_then(Value::as_object_mut) {
            if definition.contains_key("name") {
                definition.insert("name".to_owned(), json!(display));
            }
            if let Some(tab) = creative_tabs.resolve(&source.class_name, &source.source_text) {
                definition.insert("creativeTabs".to_owned(), json!([tab]));
            }
            let information = localization.item_information(&source.source_text, &registry);
            if !information.is_empty() && definition.contains_key("specialInformation") {
                definition.insert("specialInformation".to_owned(), json!({"fixedValue": information}));
            }
        }
        if source.source_path.to_ascii_lowercase().ends_with(".class") {
            warnings.push("Registration and getter constants were recovered from compiled bytecode only; constructor behaviors and source-declared texture paths may need manual reconstruction.".to_owned());
        }
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
        let descriptor = json!({
            "name":element_name,
            "type":kind,
            "compiles":true,
            "locked_code":false,
            "registry_name":registry,
            "metadata":{"files":metadata_files},
            "path":source_import_folder(&import_modid,kind),
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":import_modid,
            "mojmodelbridge_source_class":source.class_name,
            "mojmodelbridge_source_path":source.source_path
        });
        workspace["mod_elements"].as_array_mut().expect("validated workspace mod_elements").push(descriptor);
        equipment_language_entry(&mut workspace,&target_modid,&registry,&element_name,&display);

        report.exported += 1;
        report.records.push(EquipmentExportRecord {
            class_name:source.class_name.clone(),element_name,registry_name:registry.clone(),
            element_type:kind.to_owned(),texture:texture_name,source_path:source.source_path.clone(),warnings
        });
        existing_registry.insert((kind.to_owned(),registry));
    }

    fs::write(&workspace_file, format!("{}\n", serde_json::to_string_pretty(&workspace)?))
        .with_context(|| format!("Writing updated workspace {}", workspace_file.display()))?;
    Ok(report)
}

#[cfg(test)]
mod equipment_discovery_tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn discovers_legacy_registered_items_and_tools_from_public_java_classes() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("ModItems.java"), r#"
            public class ModItems {
                public static final Item IRON_SWORD = new ExampleSword();
                public static final Item CRYSTAL = new CrystalItem();
                public static void register() {
                    GameRegistry.registerItem(ModItems.IRON_SWORD, "iron_sword");
                    GameRegistry.registerItem(ModItems.CRYSTAL, "crystal");
                }
            }
            public class ExampleSword extends ItemSword { }
            public class CrystalItem extends Item { }
        "#).unwrap();

        let found = discover_equipment_sources(&[root.to_path_buf()]).unwrap();
        assert_eq!(found.len(),2);
        assert!(found.iter().any(|source| source.class_name == "ExampleSword" && source.element_type == "tool" && source.tool_type.as_deref() == Some("Sword") && source.registry_name == "iron_sword"));
        assert!(found.iter().any(|source| source.class_name == "CrystalItem" && source.element_type == "item" && source.registry_name == "crystal"));
    }

    #[test]
    fn discovers_modern_deferred_register_item_factories() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("ModItems.java"), r#"
            public class ModItems {
                public static final RegistryObject<Item> RUBY = ITEMS.register("ruby", () -> new RubyItem());
            }
            public class RubyItem extends Item { }
        "#).unwrap();
        let found = discover_equipment_sources(&[root.to_path_buf()]).unwrap();
        assert_eq!(found.len(),1);
        assert_eq!(found[0].registry_name,"ruby");
        assert_eq!(found[0].class_name,"RubyItem");
    }

    #[test]
    fn exports_registered_item_into_workspace_with_texture_and_manifest() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&workspace_root).unwrap();
        fs::create_dir_all(source.join("assets/demo/textures/items")).unwrap();
        fs::write(source.join("assets/demo/textures/items/example.png"), b"test-png").unwrap();
        fs::write(source.join("ModItems.java"), r#"
            public class ModItems {
                public static final Item EXAMPLE = new ExampleItem();
                public static void register() {
                    GameRegistry.registerItem(ModItems.EXAMPLE, "example");
                }
            }
            public class ExampleItem extends Item {
                public ExampleItem() { setTextureName("demo:items/example"); setMaxStackSize(16); }
            }
        "#).unwrap();
        let descriptor_path = workspace_root.join("test.mcreator");
        fs::write(&descriptor_path, serde_json::to_vec(&json!({
            "mcreatorVersion":202600,
            "workspaceSettings":{"modid":"converted","modElementsPackage":"example.converted","currentGenerator":"neoforge-1.21.1"},
            "mod_elements":[]
        })).unwrap()).unwrap();

        let report = export_equipment_to_workspace(&[source], &workspace_root, "demo", 89).unwrap();
        assert_eq!(report.discovered,1);
        assert_eq!(report.exported,1);
        assert_eq!(report.records[0].element_name,"Example");
        let element_path = workspace_root.join("elements/Example.mod.json");
        let element: Value = serde_json::from_slice(&fs::read(element_path).unwrap()).unwrap();
        assert_eq!(element["_type"],"item");
        assert_eq!(element["definition"]["stackSize"],16);
        assert_eq!(element["definition"]["texture"],"example");
        assert!(workspace_root.join("src/main/resources/assets/converted/textures/item/example.png").is_file());
        let updated: Value = serde_json::from_slice(&fs::read(descriptor_path).unwrap()).unwrap();
        assert_eq!(updated["mod_elements"][0]["registry_name"],"example");
        assert_eq!(updated["mod_elements"][0]["metadata"]["files"][0],"src/main/resources/assets/converted/textures/item/example.png");
        assert_eq!(updated["language_map"]["en_us"]["item.converted.example.name"],"Example");
    }

    #[test]
    fn recognizes_registered_weapon_from_inherited_custom_item_base() {
        let temp = tempdir().unwrap();
        let source = temp.path();
        fs::write(source.join("ItemWeaponBase.java"), r#"
            public abstract class ItemWeaponBase extends Item {
                protected float weaponDamage = 24.0F;
                public float getDamageVsEntity() { return weaponDamage; }
                public void hitEntity() { }
            }
        "#).unwrap();
        fs::write(source.join("ItemAncientBlade.java"), r#"
            public class ItemAncientBlade extends ItemWeaponBase { }
        "#).unwrap();
        fs::write(source.join("Registry.java"), r#"
            public class Registry {
                public static ItemAncientBlade ancientBlade = new ItemAncientBlade();
                public static void register() {
                    GameRegistry.registerItem(ancientBlade, "ancient_blade");
                }
            }
        "#).unwrap();

        let found = discover_equipment_sources(&[source.to_path_buf()]).unwrap();
        let weapon = found.iter().find(|item| item.registry_name == "ancient_blade").unwrap();
        assert_eq!(weapon.element_type, "tool");
        assert_eq!(weapon.tool_type.as_deref(), Some("Sword"));
        assert!(weapon.source_text.contains("hitEntity"));
        assert!(weapon.source_text.contains("weaponDamage"));
    }

    #[test]
    fn identifies_unusual_ranged_and_melee_weapon_subclasses() {
        assert_eq!(classify_tool("Item", "ArcZapper"), Some("Bow".to_owned()));
        assert_eq!(classify_tool("Item", "SelectionSniper"), Some("Bow".to_owned()));
        assert_eq!(classify_tool("Item", "DualityGauntlet"), Some("Sword".to_owned()));
        assert_eq!(classify_tool("Item", "QuantumClaymore"), Some("Sword".to_owned()));

        let ranged_source = r#"
            public ItemStack onItemRightClick(World world, EntityPlayer player, EnumHand hand) {
                EntityArrow projectile = new EntityArrow(world, player);
                world.spawnEntity(projectile);
                return player.getHeldItem(hand);
            }
        "#;
        assert_eq!(
            custom_weapon_tool_type("ItemPulseDevice", ranged_source),
            Some("Bow".to_owned())
        );
        let melee_source = r#"
            public float getAttackDamage() { return 18.0F; }
            public void onEntitySwing(ItemStack stack, EntityLivingBase target) { }
        "#;
        assert_eq!(
            custom_weapon_tool_type("ItemUnknownRelic", melee_source),
            Some("Sword".to_owned())
        );
    }

    #[test]
    fn discovers_custom_item_base_weapons_and_keeps_them_as_tools() {
        let parents = BTreeMap::from([
            ("WeaponBase".to_owned(), "Object".to_owned()),
        ]);
        assert_eq!(
            equipment_kind("ItemLostInfinitySword", "WeaponBase", &parents, ""),
            Some(("tool".to_owned(), Some("Sword".to_owned())))
        );
        assert_eq!(
            equipment_kind(
                "ItemBigBertha",
                "Item",
                &BTreeMap::new(),
                "float weaponDamage = 20.0F; public void hitEntity() {}"
            ),
            Some(("tool".to_owned(), Some("Sword".to_owned())))
        );
        assert_eq!(
            equipment_kind("ItemRegistry", "Object", &BTreeMap::new(), ""),
            None
        );
    }

    #[test]
    fn resolves_standalone_item_icon_from_models_item_layer0() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(source.join("assets/demo/models/item")).unwrap();
        fs::create_dir_all(source.join("assets/demo/textures/items")).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(source.join("assets/demo/textures/items/plain_icon.png"), b"standalone-icon").unwrap();
        fs::write(source.join("assets/demo/models/item/PlainItem.json"), serde_json::to_vec(&json!({
            "parent":"minecraft:item/generated",
            "textures":{"layer0":"demo:item/plain_icon"}
        })).unwrap()).unwrap();
        fs::write(source.join("ModItems.java"), r#"
            public class ModItems {
                public static final Item PLAIN = new PlainItem();
                public static void register() {
                    GameRegistry.registerItem(PLAIN, "plain_icon");
                }
            }
            public class PlainItem extends Item { }
        "#).unwrap();
        let descriptor = workspace.join("test.mcreator");
        fs::write(&descriptor, serde_json::to_vec(&json!({
            "mcreatorVersion":202600,
            "workspaceSettings":{"modid":"converted","modElementsPackage":"demo.converted","currentGenerator":"neoforge-1.21.1"},
            "mod_elements":[]
        })).unwrap()).unwrap();

        let report = export_equipment_to_workspace(&[source], &workspace, "demo", 89).unwrap();
        assert_eq!(report.exported, 1);
        assert_eq!(report.records[0].texture, "plain_icon");
        assert!(workspace.join("src/main/resources/assets/converted/textures/item/plain_icon.png").is_file());
        assert!(!report.warnings.iter().any(|warning| warning.contains("No source icon texture found")));
    }

    #[test]
    fn recovers_compiled_only_icon_register_literals() {
        let class = ClassFile {
            minor_version:0,
            major_version:50,
            access_flags:0,
            this_class:0,
            super_class:0,
            interfaces:Vec::new(),
            constant_pool:vec![
                None,
                Some(crate::bytecode::ConstantPoolEntry::Utf8("modid:items/bertha".to_owned())),
                Some(crate::bytecode::ConstantPoolEntry::String { string_index:1 }),
            ],
            fields:Vec::new(),
            methods:vec![MemberInfo {
                name:"registerIcons".to_owned(),
                code:Some(crate::bytecode::CodeAttribute {
                    code:vec![0x12, 0x02, 0xb1],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            attributes:Vec::new(),
            bootstrap_methods:Vec::new(),
        };
        let stubs = compiled_equipment_getter_stubs(&class);
        let references = declared_equipment_texture_refs(&stubs);
        assert!(references.iter().any(|value| value == "modid:items/bertha"));
        assert!(references.iter().any(|value| texture_stem_from_reference(value).as_deref() == Some("bertha")));
    }

    #[test]
    fn recovers_compiled_only_item_texture_string_constants() {
        let class = ClassFile {
            minor_version:0,
            major_version:52,
            access_flags:0,
            this_class:0,
            super_class:0,
            interfaces:Vec::new(),
            constant_pool:vec![
                None,
                Some(crate::bytecode::ConstantPoolEntry::Utf8("textures/items/bertha.png".to_owned())),
                Some(crate::bytecode::ConstantPoolEntry::String { string_index:1 }),
            ],
            fields:Vec::new(),
            methods:Vec::new(),
            attributes:Vec::new(),
            bootstrap_methods:Vec::new(),
        };
        let stubs = compiled_equipment_getter_stubs(&class);
        let references = declared_equipment_texture_refs(&stubs);
        assert_eq!(references, vec!["textures/items/bertha.png"]);
    }

    #[test]
    fn resolves_enumhelper_tool_material_to_registered_item() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("ModItems.java"), r#"
            public class ModItems {
                public static ToolMaterial RUBY = EnumHelper.addToolMaterial("RUBY", 3, 850, 9.0F, 4.0F, 17);
                public static final Item RUBY_PICKAXE = new RubyPickaxe(RUBY);
                public static void register() {
                    GameRegistry.registerItem(ModItems.RUBY_PICKAXE, "ruby_pickaxe");
                }
            }
            public class RubyPickaxe extends ItemPickaxe { }
        "#).unwrap();

        let materials = source_tool_materials(&[root.to_path_buf()], "demo");
        let ruby = &materials["rubypickaxe"];
        assert_eq!(ruby.uses, Some(850));
        assert_eq!(ruby.efficiency, Some(9.0));
        assert_eq!(ruby.damage, Some(4.0));
        assert_eq!(ruby.enchantability, Some(17));
    }

    #[test]
    fn existing_workspace_equipment_registry_is_not_duplicated() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&workspace_root).unwrap();
        fs::write(source.join("ModItems.java"), r#"
            public class ModItems {
                public static final Item EXAMPLE = new ExampleItem();
                public static void register() { GameRegistry.registerItem(EXAMPLE, "example"); }
            }
            public class ExampleItem extends Item { }
        "#).unwrap();
        let descriptor_path = workspace_root.join("test.mcreator");
        fs::write(&descriptor_path, serde_json::to_vec(&json!({
            "mcreatorVersion":202600,"workspaceSettings":{"modid":"converted"},
            "mod_elements":[{"name":"ExistingExample","type":"item","registry_name":"example"}]
        })).unwrap()).unwrap();

        let report = export_equipment_to_workspace(&[source], &workspace_root, "demo", 89).unwrap();
        assert_eq!(report.discovered,1);
        assert_eq!(report.exported,0);
        assert_eq!(report.skipped,1);
        let updated: Value = serde_json::from_slice(&fs::read(descriptor_path).unwrap()).unwrap();
        assert_eq!(updated["mod_elements"].as_array().unwrap().len(),1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_compiled_weapon_stat_tables_to_tool_material_properties() {
        let stats = compiled_tool_stats_from_numbers(&[3.0, 2400.0, 9.0, 5.0, 18.0]).unwrap();
        assert_eq!(stats.uses, Some(2400));
        assert_eq!(stats.efficiency, Some(9.0));
        assert_eq!(stats.damage, Some(5.0));
        assert_eq!(stats.enchantability, Some(18));
        assert!(compiled_tool_stats_from_numbers(&[2400.0, 9.0, 5.0]).is_none());
    }

    #[test]
    fn recovers_compiled_numeric_getters_by_method_identity() {
        let class = ClassFile {
            minor_version: 0, major_version: 52, access_flags: 0,
            this_class: 0, super_class: 0, interfaces: Vec::new(),
            constant_pool: Vec::new(), fields: Vec::new(), methods: Vec::new(),
            attributes: Vec::new(), bootstrap_methods: Vec::new(),
        };
        let make_method = |name: &str, code: Vec<u8>| MemberInfo {
            name: name.to_owned(),
            code: Some(crate::bytecode::CodeAttribute { code, ..Default::default() }),
            ..Default::default()
        };
        let use_duration = make_method("getMaxItemUseDuration", vec![0x11, 0x1c, 0x20, 0xac]);
        let damage = make_method("getDamageVsEntity", vec![0x10, 15, 0xac]);
        let durability = make_method("getMaxDamage", vec![0x11, 0x03, 0x84, 0xac]);

        assert_eq!(bytecode_method_number(&class, &use_duration), Some(7200.0));
        assert_eq!(bytecode_method_number(&class, &damage), Some(15.0));
        assert_eq!(bytecode_method_number(&class, &durability), Some(900.0));
        assert_ne!(
            bytecode_method_number(&class, &use_duration),
            bytecode_method_number(&class, &damage),
        );
    }

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
    fn recovers_compiled_food_getters_without_decompiled_constructor_source() {
        let source = r#"
            public class CookedMeat extends ItemFood {
                public double getHealAmount() { return 8.0; }
                public double getSaturationModifier() { return 0.8; }
            }
        "#;
        let (element, warnings) = build_item_element(
            None, "CookedMeat", "ItemFood", source, "cooked_meat", "meat.png", 89
        );
        assert!(warnings.is_empty());
        assert_eq!(element["definition"]["isFood"], true);
        assert_eq!(element["definition"]["nutritionalValue"], 8);
        assert_eq!(element["definition"]["saturation"], 0.8);
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
