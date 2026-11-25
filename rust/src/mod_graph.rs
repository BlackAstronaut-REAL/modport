use anyhow::{Context, Result};
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::bytecode::classfile::ConstantValue;
use crate::bytecode::{decode_instructions, parse_method_descriptor, ClassFile, Instruction};
use crate::source::java::{extract_balanced, split_top_level, strip_java_comments, unquote_java_string};

const FILE_LIMIT: usize = 250_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RendererRegistration {
    pub entity_class: String,
    pub renderer_class: String,
    #[serde(default)]
    pub model_class: String,
    #[serde(default)]
    pub model_args: Vec<Value>,
    #[serde(default)]
    pub renderer_args: Vec<Value>,
    #[serde(default = "default_shadow")]
    pub shadow_size: f64,
    #[serde(default = "default_scale")]
    pub visual_scale: f64,
    #[serde(default)]
    pub texture_refs: Vec<String>,
    #[serde(default)]
    pub source_class: String,
    #[serde(default)]
    pub source_offset: usize,
}
fn default_shadow() -> f64 { 0.5 }
fn default_scale() -> f64 { 1.0 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityMetadata {
    pub class_name: String,
    pub display_name: String,
    pub registry_name: String,
    #[serde(default)]
    pub superclass: String,
    #[serde(default = "default_width")]
    pub width: f64,
    #[serde(default = "default_height")]
    pub height: f64,
    #[serde(default = "default_health")]
    pub health: f64,
    #[serde(default = "default_speed")]
    pub movement_speed: f64,
    #[serde(default = "default_attack")]
    pub attack_damage: f64,
    #[serde(default)]
    pub armor: f64,
    #[serde(default = "default_follow")]
    pub follow_range: f64,
    #[serde(default)]
    pub xp: i32,
    #[serde(default)]
    pub fire_immune: bool,
    #[serde(default)]
    pub water_mob: bool,
    #[serde(default)]
    pub flying_mob: bool,
    #[serde(default)]
    pub ai_classes: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}
fn default_width() -> f64 { 0.6 }
fn default_height() -> f64 { 1.8 }
fn default_health() -> f64 { 10.0 }
fn default_speed() -> f64 { 0.3 }
fn default_attack() -> f64 { 3.0 }
fn default_follow() -> f64 { 16.0 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModGraph {
    pub root: PathBuf,
    #[serde(default)]
    pub source_roots: Vec<PathBuf>,
    #[serde(default)]
    pub class_files: BTreeMap<String, PathBuf>,
    #[serde(default)]
    pub java_files: BTreeMap<String, PathBuf>,
    #[serde(default)]
    pub registrations: Vec<RendererRegistration>,
    #[serde(default)]
    pub entities: BTreeMap<String, EntityMetadata>,
    #[serde(default)]
    pub registry_names: BTreeMap<String, String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl ModGraph {
    pub fn model_constructor_args(&self) -> BTreeMap<String, Vec<Value>> {
        let mut result = BTreeMap::new();
        for registration in &self.registrations {
            if !registration.model_class.is_empty() && !registration.model_args.is_empty() {
                result.entry(registration.model_class.clone()).or_insert_with(|| registration.model_args.clone());
            }
        }
        result
    }

    pub fn registrations_by_model(&self) -> BTreeMap<String, Vec<RendererRegistration>> {
        let mut result = BTreeMap::new();
        for registration in &self.registrations {
            if !registration.model_class.is_empty() {
                result.entry(registration.model_class.clone()).or_insert_with(Vec::new).push(registration.clone());
            }
        }
        result
    }

    pub fn to_json(&self) -> Result<Value> { Ok(serde_json::to_value(self)?) }
}

fn simple_name(value: &str) -> &str {
    value.rsplit(['.', '/', '$']).next().unwrap_or(value)
}

fn canonical_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn split_camel_case(value: &str, separator: char) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::with_capacity(value.len() + 8);
    for (index, current) in chars.iter().copied().enumerate() {
        if index > 0 && current.is_ascii_uppercase() {
            let previous = chars[index - 1];
            let next_is_lower = chars.get(index + 1).map(|value| value.is_ascii_lowercase()).unwrap_or(false);
            let boundary = previous.is_ascii_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_is_lower);
            if boundary { output.push(separator); }
        }
        output.push(current);
    }
    output
}

fn class_display_name(class_name: &str) -> String {
    let name = class_name.strip_prefix("Entity").or_else(|| class_name.strip_prefix("Mob")).unwrap_or(class_name);
    let spaced = split_camel_case(name, ' ');
    let cleaned = Regex::new(r"[_$]+").unwrap().replace_all(&spaced, " ");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn registry_slug(value: &str) -> String {
    let name = value.strip_prefix("Entity").or_else(|| value.strip_prefix("Mob")).unwrap_or(value);
    let camel = split_camel_case(name, '_');
    let normalized = Regex::new(r"[^A-Za-z0-9]+").unwrap().replace_all(&camel, "_");
    let value = normalized.trim_matches('_').to_ascii_lowercase();
    if value.is_empty() { "converted_entity".to_owned() } else { value }
}

fn java_scalar(value: &str) -> Value {
    let raw = value.trim();
    if raw == "true" { return Value::Bool(true); }
    if raw == "false" { return Value::Bool(false); }
    if raw.starts_with('"') && raw.ends_with('"') {
        return Value::String(unquote_java_string(raw));
    }
    let cleaned = raw.trim_end_matches(|ch: char| matches!(ch, 'f'|'F'|'d'|'D'|'l'|'L'));
    if let Ok(value) = cleaned.parse::<i64>() { return json_number(value as f64); }
    if let Ok(value) = cleaned.parse::<f64>() {
        if value.is_finite() { return json_number(value); }
    }
    Value::String(raw.to_owned())
}

fn json_number(value: f64) -> Value {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        Value::from(value as i64)
    } else { Value::from(value) }
}

fn read_java(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("Reading Java source {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_owned();
    Ok(strip_java_comments(&text))
}

fn list_source_files(roots: &[PathBuf], extension: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for root in roots {
        if !root.exists() { continue; }
        let paths = if root.is_file() {
            vec![root.clone()]
        } else {
            WalkDir::new(root).follow_links(false).into_iter()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| entry.path().to_path_buf())
                .collect()
        };
        for path in paths {
            if result.len() >= FILE_LIMIT { break; }
            if path.extension().and_then(|v| v.to_str()).map(|v| v.eq_ignore_ascii_case(extension)).unwrap_or(false) {
                let path = canonical_path(&path);
                let key = path.to_string_lossy().to_ascii_lowercase();
                if seen.insert(key) { result.push(path); }
            }
        }
    }
    result.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    result
}

fn source_index(paths: &[PathBuf]) -> BTreeMap<String, Vec<PathBuf>> {
    let mut result = BTreeMap::<String, Vec<PathBuf>>::new();
    for path in paths {
        let key = simple_name(path.file_stem().and_then(|v| v.to_str()).unwrap_or("")).to_owned();
        result.entry(key).or_default().push(path.clone());
    }
    result
}

fn compiled_class_is_entity(
    class_name: &str,
    classes: &BTreeMap<String, crate::compiled_registry::CompiledClassInfo>,
) -> bool {
    const ENTITY_BASES: &[&str] = &[
        "Entity", "EntityLiving", "EntityLivingBase", "EntityCreature", "EntityAgeable",
        "EntityAnimal", "EntityMob", "EntityWaterMob", "EntityFlying",
        "LivingEntity", "Mob", "Monster", "PathfinderMob", "Animal", "TamableAnimal",
        "FlyingMob", "WaterAnimal", "AmbientCreature", "AgeableMob", "MobEntity",
    ];
    let mut current = class_name.to_owned();
    let mut visited = HashSet::new();
    for _ in 0..24 {
        if !visited.insert(current.clone()) { break; }
        let Some(info) = classes.get(simple_name(&current)) else { return false; };
        let parent = simple_name(&info.superclass);
        if ENTITY_BASES.iter().any(|base| parent.eq_ignore_ascii_case(base)) {
            return true;
        }
        if parent.is_empty() { return false; }
        current = info.superclass.clone();
    }
    false
}

fn registry_names(java_paths: &[PathBuf]) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let modern = Regex::new(
        r#"(?s)RegistryObject\s*<\s*EntityType\s*<\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*>\s*>\s*[A-Za-z_$][A-Za-z0-9_$]*\s*=\s*[^;]{0,1200}?\.register\s*\(\s*"([^"]+)""#
    ).unwrap();
    let old_class_first = Regex::new(
        r#"(?i)\b[A-Za-z_$][A-Za-z0-9_$]*(?:Entry|Entity|Projectile|Mob|Eggless|Register)[A-Za-z0-9_$]*\s*\(\s*([A-Za-z_$][A-Za-z0-9_$.]*)\.class\s*,\s*"([^"]+)""#
    ).unwrap();
    let old_string_first = Regex::new(
        r#"(?i)\b[A-Za-z_$][A-Za-z0-9_$]*(?:Entry|Entity|Projectile|Mob|Eggless|Register)[A-Za-z0-9_$]*\s*\(\s*"([^"]+)"\s*,\s*([A-Za-z_$][A-Za-z0-9_$.]*)\.class"#
    ).unwrap();
    for path in java_paths {
        let Ok(text) = read_java(path) else { continue; };
        for found in modern.captures_iter(&text) {
            let entity = simple_name(&found[1]).to_owned();
            result.entry(entity).or_insert_with(|| found[2].to_owned());
        }
        if text.contains(".class") && (text.contains("EntityEntryBuilder") || text.contains("registerModEntity") || text.contains("registerGlobalEntityID")) {
            for found in old_class_first.captures_iter(&text) {
                if !found[2].to_ascii_lowercase().ends_with(".png") && !found[2].contains('/') {
                    result.entry(simple_name(&found[1]).to_owned()).or_insert_with(|| found[2].to_owned());
                }
            }
            for found in old_string_first.captures_iter(&text) {
                if !found[1].to_ascii_lowercase().ends_with(".png") && !found[1].contains('/') {
                    result.entry(simple_name(&found[2]).to_owned()).or_insert_with(|| found[1].to_owned());
                }
            }
        }
    }
    result
}

fn renderer_sources(renderer: &str, index: &BTreeMap<String, Vec<PathBuf>>) -> Option<PathBuf> {
    let paths = index.get(renderer)?;
    paths.iter().min_by_key(|path| {
        let text = path.to_string_lossy().to_ascii_lowercase();
        (
            if text.contains("render") { 0usize } else { 1usize },
            path.components().count(),
            text,
        )
    }).cloned()
}

fn referenced_model(text: &str) -> Option<(String, Vec<Value>)> {
    let pattern = Regex::new(r"\bnew\s+([A-Za-z_$][A-Za-z0-9_$.]*(?:Model[A-Za-z0-9_$]*)|Model[A-Za-z0-9_$]+)\s*\(").unwrap();
    for found in pattern.find_iter(text) {
        let captures = pattern.captures(&text[found.start()..found.end()])?;
        let class_name = simple_name(&captures[1]).to_owned();
        let open = text[found.start()..found.end()].rfind('(')? + found.start();
        let (raw, _) = extract_balanced(text, open, '(', ')').ok()?;
        let args = if raw.trim().is_empty() { Vec::new() } else {
            split_top_level(&raw, ',').iter().map(|value| java_scalar(value)).collect()
        };
        return Some((class_name, args));
    }
    None
}

fn png_refs(text: &str) -> Vec<String> {
    let regex = Regex::new(r"(?i)[A-Za-z0-9_./\\-]{1,240}\.png").unwrap();
    let mut refs: Vec<String> = regex.find_iter(text)
        .map(|found| found.as_str().replace('\\', "/"))
        .collect();
    refs.sort_by_key(|value| value.to_ascii_lowercase());
    refs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    refs
}

fn texture_references_in_region(source: &str) -> Vec<String> {
    let mut references = png_refs(source);
    let two_args = Regex::new(
        r#"(?is)(?:new\s+ResourceLocation|ResourceLocation\s*\.\s*(?:fromNamespaceAndPath|tryParse))\s*\(\s*"([^"]+)"\s*,\s*"([^"]+)"#
    ).unwrap();
    for capture in two_args.captures_iter(source) {
        references.push(format!("{}:{}", &capture[1], &capture[2]));
    }
    let one_arg = Regex::new(
        r#"(?is)(?:new\s+ResourceLocation|ResourceLocation\s*\.\s*(?:parse|withDefaultNamespace))\s*\(\s*"([^"]+)"#
    ).unwrap();
    for capture in one_arg.captures_iter(source) {
        references.push(capture[1].replace('\\', "/"));
    }
    let texture_constants = Regex::new(
        r#"(?i)["']((?:[A-Za-z0-9_.-]+:)?(?:textures/)?(?:entity|entities|mob|mobs|models)/[A-Za-z0-9_./-]{1,200})["']"#
    ).unwrap();
    for capture in texture_constants.captures_iter(source) {
        references.push(capture[1].replace('\\', "/"));
    }
    references
}

fn append_unique_texture_refs(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.iter().any(|existing| existing.eq_ignore_ascii_case(&value)) {
            target.push(value);
        }
    }
}

fn source_texture_references(source: &str) -> Vec<String> {
    let mut references = Vec::new();
    // A mob's own texture accessor is more authoritative than unrelated
    // renderer textures such as overlays, eyes, or auxiliary layers.
    let texture_method = Regex::new(
        r"(?s)\b(?:getEntityTexture|getEntityTextureLocation|getTextureLocation|getTextureResource|getTexture)\s*\([^)]*\)\s*\{"
    ).unwrap();
    for method in texture_method.find_iter(source) {
        let Some(open) = source[method.start()..method.end()].rfind('{')
            .map(|offset| method.start() + offset) else { continue; };
        if let Ok((body, _)) = extract_balanced(source, open, '{', '}') {
            append_unique_texture_refs(&mut references, texture_references_in_region(&body));
        }
    }
    append_unique_texture_refs(&mut references, texture_references_in_region(source));
    references
}

fn renderer_info(renderer: &str, index: &BTreeMap<String, Vec<PathBuf>>) -> (String, Vec<Value>, f64, f64, Vec<String>) {
    let Some(path) = renderer_sources(renderer, index) else { return (String::new(), Vec::new(), 0.5, 1.0, Vec::new()); };
    let Ok(text) = read_java(&path) else { return (String::new(), Vec::new(), 0.5, 1.0, Vec::new()); };
    let (model, args) = referenced_model(&text).unwrap_or_default();

    let mut shadow = 0.5;
    if !model.is_empty() {
        if let Some(found) = Regex::new(r",\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+))[fFdD]?\s*\)").unwrap()
            .find(&text)
        {
            let raw = found.as_str().trim_matches(|ch: char| ch == ',' || ch.is_whitespace() || ch == ')');
            shadow = raw.trim_end_matches(|ch: char| matches!(ch, 'f'|'F'|'d'|'D')).parse().unwrap_or(0.5);
        }
    }
    let scale_regex = Regex::new(r"(?:GlStateManager\.[A-Za-z0-9_$]+|glScalef|scale)\s*\(\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+))[fFdD]?\s*,\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+))[fFdD]?\s*,\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+))[fFdD]?").unwrap();
    let visual_scale = scale_regex.captures(&text).and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse::<f64>().ok()).unwrap_or(1.0);
    (model, args, shadow, visual_scale, source_texture_references(&text))
}

fn registry_field_entities(java_paths: &[PathBuf]) -> HashMap<String, String> {
    let patterns = [
        Regex::new(r"(?s)RegistryObject\s*<\s*EntityType\s*<\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*>\s*>\s*([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*[^;]{0,1200}?\.register\s*\(").unwrap(),
        Regex::new(r"(?:static\s+final\s+)?EntityType\s*<\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*>\s*([A-Za-z_$][A-Za-z0-9_$]*)\s*=").unwrap()
    ];
    let mut result = HashMap::new();
    for path in java_paths {
        let Ok(text) = read_java(path) else { continue; };
        for pattern in &patterns {
            for captures in pattern.captures_iter(&text) {
                result.insert(captures[2].to_owned(), simple_name(&captures[1]).to_owned());
            }
        }
    }
    result
}

fn parse_registration_call(
    name: &str,
    raw_args: &str,
    java_path: &Path,
    offset: usize,
    field_entities: &HashMap<String, String>,
    index: &BTreeMap<String, Vec<PathBuf>>,
) -> Option<RendererRegistration> {
    if !matches!(name, "registerEntityRenderingHandler" | "registerRender" | "registerRenderer" | "registerEntityRenderer") { return None; }
    let args = split_top_level(raw_args, ',');
    if args.len() < 2 { return None; }

    let first = args[0].trim();
    let entity = if let Some(class) = first.strip_suffix(".class") {
        simple_name(class).to_owned()
    } else if let Some(field) = first.strip_suffix(".get()") {
        field_entities.get(simple_name(field))?.clone()
    } else {
        return None;
    };

    let factory = args[1].trim();
    let factory_new = Regex::new(r"(?s)(?:->\s*new\s+|new\s+)([A-Za-z_$][A-Za-z0-9_$.]*)").unwrap();
    let method_reference = Regex::new(r"([A-Za-z_$][A-Za-z0-9_$.]*)\s*::\s*new").unwrap();
    let renderer = factory_new.captures(factory)
        .map(|found| simple_name(&found[1]).to_owned())
        .or_else(|| method_reference.captures(factory).map(|found| simple_name(&found[1]).to_owned()))?;
    let (model_class, model_args, mut shadow, mut scale, mut textures) = renderer_info(&renderer, index);

    // If the renderer factory explicitly constructs a model, it is a stronger
    // signal than a potentially ambiguous same-name source file.
    let inline_model = referenced_model(factory);
    let (model_class, model_args) = inline_model.unwrap_or((model_class, model_args));

    let numeric = Regex::new(r"(?i)([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[-+]?\d+)?)[fFdD]?").unwrap();
    let numeric_args: Vec<f64> = args.iter().flat_map(|arg| numeric.find_iter(arg))
        .filter_map(|found| found.as_str().trim_end_matches(|ch: char| matches!(ch, 'f'|'F'|'d'|'D')).parse().ok())
        .collect();
    if numeric_args.len() >= 2 {
        shadow = numeric_args[numeric_args.len()-2];
        scale = numeric_args[numeric_args.len()-1];
    }
    textures.extend(source_texture_references(raw_args));
    textures.sort_by_key(|value| value.to_ascii_lowercase());
    textures.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

    Some(RendererRegistration {
        entity_class: entity,
        renderer_class: renderer,
        model_class,
        model_args,
        renderer_args: args.iter().skip(1).map(|value| Value::String(value.trim().to_owned())).collect(),
        shadow_size: shadow,
        visual_scale: scale,
        texture_refs: textures,
        source_class: java_path.file_name().and_then(|v| v.to_str()).unwrap_or("").to_owned(),
        source_offset: offset,
    })
}

fn source_attribute_number(source: &str, attribute_names: &[&str]) -> Option<f64> {
    let names = attribute_names.iter()
        .map(|name| regex::escape(name))
        .collect::<Vec<_>>()
        .join("|");
    let number = r"([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)[fFdD]?";

    // AttributeSupplier.Builder.add(Attributes.MAX_HEALTH, 80.0D)
    let builder = Regex::new(&format!(
        r"(?is)\b(?:add|put)\s*\(\s*(?:SharedMonsterAttributes|Attributes|Attribute)\s*\.\s*(?:{names})\b\s*,\s*{number}"
    )).ok()?;
    if let Some(value) = builder.captures(source).and_then(|capture| capture.get(1)) {
        if let Ok(value) = value.as_str().parse::<f64>() {
            return Some(value);
        }
    }

    // getEntityAttribute(SharedMonsterAttributes.maxHealth).setBaseValue(120.0D)
    let setter = Regex::new(&format!(
        r"(?is)(?:SharedMonsterAttributes|Attributes|Attribute)\s*\.\s*(?:{names})\b.{{0,240}}?(?:setBaseValue|setValue|set)\s*\(\s*{number}"
    )).ok()?;
    setter.captures(source).and_then(|capture| capture.get(1))
        .and_then(|value| value.as_str().parse::<f64>().ok())
}

fn source_numeric_value(source: &str, pattern: &str) -> Option<f64> {
    Regex::new(pattern).ok()?.captures(source)
        .and_then(|capture| capture.get(1))
        .and_then(|value| value.as_str().trim_end_matches(|ch: char| matches!(ch, 'f'|'F'|'d'|'D')).parse::<f64>().ok())
}

fn parse_entity_source_metadata(path: &Path, registry: Option<&str>) -> Result<EntityMetadata> {
    let text = read_java(path)?;
    let class_re = Regex::new(r"\bclass\s+([A-Za-z_$][\w$]*)(?:\s+extends\s+([A-Za-z_$][\w$.]*))?").unwrap();
    let capture = class_re.captures(&text);
    let class_name = capture.as_ref().map(|v| simple_name(&v[1]).to_owned())
        .unwrap_or_else(|| path.file_stem().and_then(|v| v.to_str()).unwrap_or("").to_owned());
    let superclass = capture.and_then(|v| v.get(2).map(|m| simple_name(m.as_str()).to_owned())).unwrap_or_default();

    let mut metadata = EntityMetadata {
        class_name: class_name.clone(),
        display_name: class_display_name(registry.unwrap_or(&class_name)),
        registry_name: registry.map(str::to_owned).unwrap_or_else(|| registry_slug(&class_name)),
        superclass: superclass.clone(),
        width: default_width(),
        height: default_height(),
        health: default_health(),
        movement_speed: default_speed(),
        attack_damage: default_attack(),
        armor: 0.0,
        follow_range: default_follow(),
        xp: 0,
        fire_immune: text.contains("fireImmune") || text.contains("fire_immune"),
        water_mob: ["WaterMob","Squid","Fish"].iter().any(|v| superclass.contains(v)),
        flying_mob: ["Flying","Ambient","Bat"].iter().any(|v| superclass.contains(v)),
        ai_classes: Vec::new(),
        warnings: Vec::new(),
    };

    let numbers = [
        (r"\b(?:setSize|func_70105_a)\s*\(\s*([-+]?\d+(?:\.\d*)?[fFdD]?)\s*,\s*([-+]?\d+(?:\.\d*)?[fFdD]?)\s*\)", 0usize),
        (r"\b(?:maxHealth|setMaxHealth)\s*(?:=|\()\s*([-+]?\d+(?:\.\d*)?[fFdD]?)", 1usize),
        (r"\b(?:movementSpeed|setMovementSpeed)\s*(?:=|\()\s*([-+]?\d+(?:\.\d*)?[fFdD]?)", 2usize),
        (r"\b(?:attackDamage|setAttackDamage)\s*(?:=|\()\s*([-+]?\d+(?:\.\d*)?[fFdD]?)", 3usize),
        (r"\b(?:followRange|setFollowRange)\s*(?:=|\()\s*([-+]?\d+(?:\.\d*)?[fFdD]?)", 4usize),
        (r"\b(?:armor|setArmor)\s*(?:=|\()\s*([-+]?\d+(?:\.\d*)?[fFdD]?)", 5usize),
    ];
    for (pattern, field) in numbers {
        let regex = Regex::new(pattern).unwrap();
        if let Some(found) = regex.captures(&text) {
            let values: Vec<f64> = found.iter().skip(1).flatten()
                .filter_map(|value| value.as_str().trim_end_matches(|ch: char| matches!(ch,'f'|'F'|'d'|'D')).parse().ok()).collect();
            match field {
                0 if values.len() == 2 => { metadata.width = values[0]; metadata.height = values[1]; }
                1 if !values.is_empty() => metadata.health = values[0],
                2 if !values.is_empty() => metadata.movement_speed = values[0],
                3 if !values.is_empty() => metadata.attack_damage = values[0],
                4 if !values.is_empty() => metadata.follow_range = values[0],
                5 if !values.is_empty() => metadata.armor = values[0],
                _ => {}
            }
        }
    }
    // Modern and legacy attribute builders use a common attribute token
    // followed by add()/setBaseValue(), rather than assigning the numeric
    // property directly. Preserve explicit values when these patterns exist.
    if let Some(value) = source_attribute_number(&text, &["MAX_HEALTH", "maxHealth", "MAX_HEALTH"]) {
        metadata.health = value;
    }
    if let Some(value) = source_attribute_number(&text, &["MOVEMENT_SPEED", "movementSpeed", "MOVEMENT_SPEED"]) {
        metadata.movement_speed = value;
    }
    if let Some(value) = source_attribute_number(&text, &["ATTACK_DAMAGE", "attackDamage", "ATTACK_DAMAGE"]) {
        metadata.attack_damage = value;
    }
    if let Some(value) = source_attribute_number(&text, &["FOLLOW_RANGE", "followRange", "FOLLOW_RANGE"]) {
        metadata.follow_range = value;
    }
    if let Some(value) = source_attribute_number(&text, &["ARMOR", "armor"]) {
        metadata.armor = value;
    }
    if let Some(value) = source_attribute_number(&text, &["ARMOR_TOUGHNESS", "armorToughness"]) {
        metadata.armor = metadata.armor.max(value);
    }
    if let Some(value) = source_numeric_value(
        &text,
        r"(?is)\b(?:experienceValue|xpValue|setExperiencePoints)\s*(?:=|\()\s*(-?\d+)",
    ) {
        metadata.xp = value.round() as i32;
    }
    let ai = Regex::new(r"new\s+(EntityAI[A-Za-z0-9_$]+|[A-Za-z0-9_$]*Goal)\s*\(").unwrap();
    metadata.ai_classes = ai.captures_iter(&text).map(|v| v[1].to_owned()).collect::<HashSet<_>>().into_iter().collect();
    metadata.ai_classes.sort();
    Ok(metadata)
}

/// Scan mixed source/compiled roots and build the first native entity →
/// renderer → model graph. Source registrations cover legacy FML calls,
/// RegistryObject.get() lambdas and renderer constructor references.

#[derive(Debug, Clone)]
enum CompiledValue {
    Class(String),
    Number(f64),
    Text(String),
    Object(usize),
    Field(String),
    Null,
}

#[derive(Debug, Clone, Default)]
struct CompiledObject {
    class_name: String,
    arguments: Vec<CompiledValue>,
}

fn compiled_pop(stack: &mut Vec<CompiledValue>) -> CompiledValue {
    stack.pop().unwrap_or(CompiledValue::Null)
}

fn compiled_pop_n(stack: &mut Vec<CompiledValue>, count: usize) -> Vec<CompiledValue> {
    let missing = count.saturating_sub(stack.len());
    let mut values = vec![CompiledValue::Null; missing];
    let split = stack.len().saturating_sub(count);
    values.extend(stack.drain(split..));
    values
}

fn compiled_u16(bytes: &[u8], index: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(index)?, *bytes.get(index + 1)?]))
}

fn compiled_constant(class: &ClassFile, instruction: &Instruction) -> Option<CompiledValue> {
    let op = instruction.opcode;
    let number = match op {
        0x02 => Some(-1.0),
        0x03..=0x08 => Some((op - 0x03) as f64),
        0x09..=0x0a => Some((op - 0x09) as f64),
        0x0b..=0x0d => Some((op - 0x0b) as f64),
        0x0e..=0x0f => Some((op - 0x0e) as f64),
        0x10 => instruction.operands.first().map(|value| *value as i8 as f64),
        0x11 => instruction.operands.get(0..2).map(|bytes| i16::from_be_bytes([bytes[0], bytes[1]]) as f64),
        _ => None,
    };
    if let Some(value) = number { return Some(CompiledValue::Number(value)); }
    let index = match op {
        0x12 => *instruction.operands.first()? as u16,
        0x13 | 0x14 => compiled_u16(&instruction.operands, 0)?,
        _ => return None,
    };
    match class.constant(index)? {
        ConstantValue::Integer(value) => Some(CompiledValue::Number(value as f64)),
        ConstantValue::Float(value) => Some(CompiledValue::Number(value as f64)),
        ConstantValue::Long(value) => Some(CompiledValue::Number(value as f64)),
        ConstantValue::Double(value) => Some(CompiledValue::Number(value)),
        ConstantValue::String(value) => Some(CompiledValue::Text(value)),
        ConstantValue::Class(value) => Some(CompiledValue::Class(value)),
    }
}

fn compiled_local(instruction: &Instruction) -> Option<(usize, bool)> {
    let op = instruction.opcode;
    let load = match op {
        0x19 => instruction.operands.first().map(|value| *value as usize),
        0x2a..=0x2d => Some((op - 0x2a) as usize),
        _ => None,
    };
    if let Some(index) = load { return Some((index, true)); }
    let store = match op {
        0x3a => instruction.operands.first().map(|value| *value as usize),
        0x4b..=0x4e => Some((op - 0x4b) as usize),
        _ => None,
    };
    if let Some(index) = store { return Some((index, false)); }
    if op == 0xc4 && instruction.operands.len() >= 3 {
        let modified = instruction.operands[0];
        let index = compiled_u16(&instruction.operands, 1)? as usize;
        if modified == 0x19 { return Some((index, true)); }
        if modified == 0x3a { return Some((index, false)); }
    }
    None
}

fn compiled_value_json(value: &CompiledValue, objects: &[CompiledObject], depth: usize) -> Value {
    if depth > 8 { return Value::Null; }
    match value {
        CompiledValue::Class(name) => Value::String(simple_name(name).to_owned()),
        CompiledValue::Number(number) => json_number(*number),
        CompiledValue::Text(text) => Value::String(text.clone()),
        CompiledValue::Field(field) => Value::String(field.clone()),
        CompiledValue::Object(index) => {
            let Some(object) = objects.get(*index) else { return Value::Null; };
            serde_json::json!({
                "class_name": simple_name(&object.class_name),
                "args": object.arguments.iter()
                    .map(|argument| compiled_value_json(argument, objects, depth + 1))
                    .collect::<Vec<_>>()
            })
        }
        CompiledValue::Null => Value::Null,
    }
}

fn compiled_registration_from_call(
    method_name: &str,
    arguments: &[CompiledValue],
    objects: &[CompiledObject],
    source_class: &str,
    offset: usize,
    texture_refs: &[String],
) -> Option<RendererRegistration> {
    if !matches!(method_name,
        "registerEntityRenderingHandler" | "registerRender" |
        "registerRenderer" | "registerEntityRenderer"
    ) {
        return None;
    }
    let entity_class = match arguments.first()? {
        CompiledValue::Class(name) => simple_name(name).to_owned(),
        _ => return None,
    };
    let renderer_index = match arguments.get(1)? {
        CompiledValue::Object(index) => *index,
        _ => return None,
    };
    let renderer = objects.get(renderer_index)?;
    let model = renderer.arguments.iter().find_map(|argument| {
        let CompiledValue::Object(index) = argument else { return None; };
        let object = objects.get(*index)?;
        simple_name(&object.class_name).starts_with("Model").then_some((*index, object))
    });
    let (model_class, model_args) = if let Some((_index, model)) = model {
        (
            simple_name(&model.class_name).to_owned(),
            model.arguments.iter()
                .map(|argument| compiled_value_json(argument, objects, 0))
                .collect(),
        )
    } else {
        (String::new(), Vec::new())
    };
    let renderer_args: Vec<Value> = renderer.arguments.iter()
        .map(|argument| compiled_value_json(argument, objects, 0))
        .collect();
    let numeric: Vec<f64> = renderer.arguments.iter().filter_map(|argument| {
        if let CompiledValue::Number(value) = argument { Some(*value) } else { None }
    }).collect();
    let shadow_size = numeric.first().copied().unwrap_or(0.5);
    let visual_scale = if numeric.len() >= 2 { *numeric.last().unwrap() } else { 1.0 };

    Some(RendererRegistration {
        entity_class,
        renderer_class: simple_name(&renderer.class_name).to_owned(),
        model_class,
        model_args,
        renderer_args,
        shadow_size,
        visual_scale,
        texture_refs: texture_refs.to_vec(),
        source_class: source_class.to_owned(),
        source_offset: offset,
    })
}

fn class_texture_references(class: &ClassFile) -> Vec<String> {
    let pattern = Regex::new(r"(?i)[A-Za-z0-9_./\\-]{1,240}\.png").unwrap();
    let mut references = HashSet::new();
    for entry in &class.constant_pool {
        let text = match entry.as_ref() {
            Some(crate::bytecode::ConstantPoolEntry::Utf8(value)) => Some(value.as_str()),
            _ => None,
        };
        if let Some(text) = text {
            for found in pattern.find_iter(text) {
                references.insert(found.as_str().replace('\\', "/"));
            }
            let resource_path = Regex::new(
                r"(?i)(?:textures/)?(?:entity|entities|mob|mobs|models)/[A-Za-z0-9_./-]{1,200}"
            ).unwrap();
            for found in resource_path.find_iter(text) {
                references.insert(found.as_str().replace('\\', "/"));
            }
        }
    }
    let mut output: Vec<String> = references.into_iter().collect();
    output.sort_by_key(|value| value.to_ascii_lowercase());
    output
}

fn parse_compiled_registration_class(path: &Path) -> Result<Vec<RendererRegistration>> {
    let bytes = fs::read(path).with_context(|| format!("Reading compiled registry {}", path.display()))?;
    let class = ClassFile::parse(&bytes)?;
    parse_compiled_registration_class_file(&class, path)
}

fn parse_compiled_registration_class_file(class: &ClassFile, path: &Path) -> Result<Vec<RendererRegistration>> {
    let class_name = class.this_name()
        .map(|name| simple_name(name).to_owned())
        .unwrap_or_else(|| path.file_stem().and_then(|value| value.to_str()).unwrap_or("Unknown").to_owned());
    let textures = class_texture_references(class);
    let mut registrations = Vec::new();

    for method in &class.methods {
        let Some(code) = method.code.as_ref() else { continue; };
        let instructions = match decode_instructions(&code.code) {
            Ok(instructions) => instructions,
            Err(_) => continue,
        };
        if !instructions.iter().any(|instruction| {
            matches!(instruction.opcode, 0xb6..=0xb9)
                && compiled_u16(&instruction.operands, 0)
                    .and_then(|index| class.member_reference(index))
                    .map(|(_, name, _)| matches!(name.as_str(),
                        "registerEntityRenderingHandler" | "registerRender" |
                        "registerRenderer" | "registerEntityRenderer"
                    ))
                    .unwrap_or(false)
        }) {
            continue;
        }

        let descriptor = match parse_method_descriptor(&method.descriptor) {
            Ok(descriptor) => descriptor,
            Err(_) => continue,
        };
        let is_static = method.access_flags & 0x0008 != 0;
        let mut locals = HashMap::<usize, CompiledValue>::new();
        if !is_static { locals.insert(0, CompiledValue::Null); }
        let mut slot = if is_static { 0 } else { 1 };
        for parameter in &descriptor.parameters {
            locals.insert(slot, CompiledValue::Null);
            slot += parameter.slots();
        }
        let mut stack = Vec::<CompiledValue>::new();
        let mut objects = Vec::<CompiledObject>::new();

        for instruction in &instructions {
            let op = instruction.opcode;
            if op == 0x00 { continue; }

            if let Some((index, load)) = compiled_local(instruction) {
                if load {
                    stack.push(locals.get(&index).cloned().unwrap_or(CompiledValue::Null));
                } else {
                    locals.insert(index, compiled_pop(&mut stack));
                }
                continue;
            }

            if (0x02..=0x14).contains(&op) {
                if let Some(value) = compiled_constant(class, instruction) {
                    stack.push(value);
                }
                continue;
            }

            match op {
                0x01 => stack.push(CompiledValue::Null),
                0xbb => {
                    let index = compiled_u16(&instruction.operands, 0).unwrap_or(0);
                    let name = class.class_name(index).unwrap_or("java/lang/Object").to_owned();
                    let object_index = objects.len();
                    objects.push(CompiledObject { class_name: name, arguments: Vec::new() });
                    stack.push(CompiledValue::Object(object_index));
                }
                0x57 => { let _ = compiled_pop(&mut stack); }
                0x58 => { let _ = compiled_pop(&mut stack); let _ = compiled_pop(&mut stack); }
                0x59 => {
                    if let Some(value) = stack.last().cloned() { stack.push(value); }
                }
                0x5a => {
                    if stack.len() >= 2 {
                        let first = compiled_pop(&mut stack);
                        let second = compiled_pop(&mut stack);
                        stack.push(first.clone());
                        stack.push(second);
                        stack.push(first);
                    }
                }
                0x5c => {
                    if stack.len() >= 2 {
                        let first = stack[stack.len() - 2].clone();
                        let second = stack[stack.len() - 1].clone();
                        stack.push(first);
                        stack.push(second);
                    } else if let Some(value) = stack.last().cloned() {
                        stack.push(value);
                    }
                }
                0x5f => {
                    if stack.len() >= 2 {
                        let len = stack.len();
                        stack.swap(len - 1, len - 2);
                    }
                }
                0xb2 => {
                    let index = compiled_u16(&instruction.operands, 0).unwrap_or(0);
                    let field = class.member_reference(index)
                        .map(|(owner, name, _)| format!("{}.{}", simple_name(&owner), name))
                        .unwrap_or_default();
                    stack.push(CompiledValue::Field(field));
                }
                0xb4 => {
                    let _ = compiled_pop(&mut stack);
                    stack.push(CompiledValue::Null);
                }
                0xb3 => { let _ = compiled_pop(&mut stack); }
                0xb5 => { let _ = compiled_pop(&mut stack); let _ = compiled_pop(&mut stack); }
                0xb6..=0xb9 => {
                    let cp_index = compiled_u16(&instruction.operands, 0).unwrap_or(0);
                    let Some((owner, name, descriptor)) = class.member_reference(cp_index) else { continue; };
                    let parsed = match parse_method_descriptor(&descriptor) {
                        Ok(parsed) => parsed,
                        Err(_) => continue,
                    };
                    let arguments = compiled_pop_n(&mut stack, parsed.parameters.len());
                    let receiver = if op == 0xb8 { CompiledValue::Null } else { compiled_pop(&mut stack) };

                    if name == "<init>" {
                        if let CompiledValue::Object(index) = receiver {
                            if let Some(object) = objects.get_mut(index) {
                                object.class_name = owner;
                                object.arguments = arguments;
                            }
                        }
                    } else if let Some(registration) = compiled_registration_from_call(
                        &name,
                        &arguments,
                        &objects,
                        &format!("{class_name}.class"),
                        instruction.offset,
                        &textures,
                    ) {
                        registrations.push(registration);
                    } else if !matches!(parsed.return_type, crate::bytecode::JvmType::Void) {
                        stack.push(CompiledValue::Null);
                    }
                }
                0xba => {
                    let cp_index = compiled_u16(&instruction.operands, 0).unwrap_or(0);
                    let argument_count = class.invoke_dynamic(cp_index)
                        .and_then(|(_, _, descriptor)| parse_method_descriptor(&descriptor).ok())
                        .map(|descriptor| descriptor.parameters.len())
                        .unwrap_or(0);
                    let _ = compiled_pop_n(&mut stack, argument_count);
                    stack.push(CompiledValue::Null);
                }
                0xc0 | 0xc1 => {}
                0xac..=0xb1 | 0xbf => break,
                _ => {}
            }
        }
    }
    Ok(registrations)
}



fn compiled_registry_names(compiled: &crate::compiled_registry::CompiledRegistryIndex) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let markers = ["registerModEntity", "registerGlobalEntityID"];
    for info in compiled.classes.values() {
        let Some(class) = compiled.parsed_class_paths.get(&info.path) else { continue };
        let class = class.as_ref();
        let has_marker = class.constant_pool.iter().flatten().any(|entry| {
            matches!(entry, crate::bytecode::ConstantPoolEntry::Utf8(value)
                if markers.iter().any(|marker| value == marker))
        });
        if !has_marker { continue; }
        for method in &class.methods {
            let Some(code) = method.code.as_ref() else { continue; };
            let Ok(instructions) = decode_instructions(&code.code) else { continue; };
            let mut recent_class = String::new();
            let mut recent_string = String::new();
            for instruction in &instructions {
                if (0x12..=0x14).contains(&instruction.opcode) {
                    match compiled_constant(class, instruction) {
                        Some(CompiledValue::Class(value)) => recent_class = simple_name(&value).to_owned(),
                        Some(CompiledValue::Text(value)) => recent_string = value,
                        _ => {}
                    }
                }
                if !matches!(instruction.opcode, 0xb6..=0xb9) { continue; }
                let Some(index) = compiled_u16(&instruction.operands, 0) else { continue; };
                let Some((_, name, _)) = class.member_reference(index) else { continue; };
                if matches!(name.as_str(), "registerModEntity" | "registerGlobalEntityID")
                    && !recent_class.is_empty()
                    && !recent_string.is_empty()
                {
                    result.entry(recent_class.clone()).or_insert_with(|| recent_string.clone());
                }
            }
        }
    }
    result
}


fn entity_attribute_name(field: &str) -> Option<&'static str> {
    match field {
        "field_111267_a" | "maxHealth" | "MAX_HEALTH" | "GENERIC_MAX_HEALTH" => Some("health"),
        "field_111263_d" | "movementSpeed" | "MOVEMENT_SPEED" | "GENERIC_MOVEMENT_SPEED" => Some("movement_speed"),
        "field_111264_e" | "attackDamage" | "ATTACK_DAMAGE" | "GENERIC_ATTACK_DAMAGE" => Some("attack_damage"),
        "field_111265_b" | "followRange" | "FOLLOW_RANGE" | "GENERIC_FOLLOW_RANGE" => Some("follow_range"),
        "armor" | "ARMOR" | "ARMOR_VALUE" => Some("armor"),
        _ => None,
    }
}

fn default_compiled_entity_metadata(class_name: &str, registry: Option<&str>) -> EntityMetadata {
    EntityMetadata {
        class_name: class_name.to_owned(),
        display_name: class_display_name(registry.unwrap_or(class_name)),
        registry_name: registry.map(str::to_owned).unwrap_or_else(|| registry_slug(class_name)),
        superclass: String::new(),
        width: default_width(),
        height: default_height(),
        health: default_health(),
        movement_speed: default_speed(),
        attack_damage: default_attack(),
        armor: 0.0,
        follow_range: default_follow(),
        xp: 0,
        fire_immune: false,
        water_mob: false,
        flying_mob: false,
        ai_classes: Vec::new(),
        warnings: Vec::new(),
    }
}

fn parse_entity_class_metadata(path: &Path, class_name: &str, registry: Option<&str>) -> EntityMetadata {
    let parsed = fs::read(path).ok().and_then(|bytes| ClassFile::parse(&bytes).ok());
    let Some(class) = parsed else {
        let mut metadata = default_compiled_entity_metadata(class_name, registry);
        metadata.warnings.push(
            "Compiled entity class could not be parsed; numeric attributes and superclass traits remain defaults.".to_owned()
        );
        return metadata;
    };
    parse_entity_class_metadata_from_class(&class, class_name, registry)
}

fn parse_entity_class_metadata_from_class(class: &ClassFile, class_name: &str, registry: Option<&str>) -> EntityMetadata {
    let mut metadata = default_compiled_entity_metadata(class_name, registry);
    metadata.superclass = class.super_name().map(simple_name).unwrap_or("").to_owned();
    let superclass = metadata.superclass.to_ascii_lowercase();
    metadata.water_mob = ["watermob", "squid", "fish"].iter().any(|token| superclass.contains(token));
    metadata.flying_mob = ["flying", "ambient", "bat"].iter().any(|token| superclass.contains(token));
    metadata.fire_immune = class.constant_pool.iter().filter_map(|entry| match entry.as_ref() {
        Some(crate::bytecode::ConstantPoolEntry::Utf8(value)) => Some(value.as_str()),
        _ => None,
    }).any(|value| value.contains("fireImmune") || value.contains("field_70178_ae"));

    let mut found_attributes = HashSet::<&'static str>::new();
    let mut size_values = Vec::<(f64, f64)>::new();
    let mut ai_classes = HashSet::<String>::new();

    for method in &class.methods {
        let Some(code) = method.code.as_ref() else { continue; };
        let Ok(instructions) = decode_instructions(&code.code) else { continue; };
        let mut stack = Vec::<CompiledValue>::new();
        let mut objects = Vec::<CompiledObject>::new();
        let mut current_attribute: Option<&'static str> = None;
        let mut recent_numeric: Option<f64> = None;

        for instruction in &instructions {
            let op = instruction.opcode;
            if op == 0x00 { continue; }

            if let Some((index, load)) = compiled_local(instruction) {
                if load {
                    stack.push(CompiledValue::Null);
                } else {
                    let _ = compiled_pop(&mut stack);
                }
                let _ = index;
                continue;
            }
            if matches!(op, 0x15..=0x18 | 0x1a..=0x29) {
                stack.push(CompiledValue::Null);
                continue;
            }
            if matches!(op, 0x36..=0x39 | 0x3b..=0x4a) {
                let _ = compiled_pop(&mut stack);
                continue;
            }

            if (0x02..=0x14).contains(&op) {
                if let Some(value) = compiled_constant(class, instruction) {
                    if let CompiledValue::Number(number) = value {
                        recent_numeric = Some(number);
                        stack.push(CompiledValue::Number(number));
                    } else {
                        stack.push(value);
                    }
                }
                continue;
            }

            match op {
                0x01 => stack.push(CompiledValue::Null),
                0xbb => {
                    let index = compiled_u16(&instruction.operands, 0).unwrap_or(0);
                    let name = class.class_name(index).unwrap_or("java/lang/Object").to_owned();
                    let simple = simple_name(&name);
                    if simple.starts_with("EntityAI") || simple.ends_with("Goal") {
                        ai_classes.insert(simple.to_owned());
                    }
                    let object_index = objects.len();
                    objects.push(CompiledObject { class_name: name, arguments: Vec::new() });
                    stack.push(CompiledValue::Object(object_index));
                }
                0x57 => { let _ = compiled_pop(&mut stack); }
                0x58 => { let _ = compiled_pop(&mut stack); let _ = compiled_pop(&mut stack); }
                0x59 => {
                    if let Some(value) = stack.last().cloned() { stack.push(value); }
                }
                0x5a => {
                    if stack.len() >= 2 {
                        let first = compiled_pop(&mut stack);
                        let second = compiled_pop(&mut stack);
                        stack.push(first.clone());
                        stack.push(second);
                        stack.push(first);
                    }
                }
                0x5c => {
                    if stack.len() >= 2 {
                        let first = stack[stack.len() - 2].clone();
                        let second = stack[stack.len() - 1].clone();
                        stack.push(first);
                        stack.push(second);
                    } else if let Some(value) = stack.last().cloned() {
                        stack.push(value);
                    }
                }
                0xb2 => {
                    let index = compiled_u16(&instruction.operands, 0).unwrap_or(0);
                    if let Some((_, name, _)) = class.member_reference(index) {
                        current_attribute = entity_attribute_name(&name).or(current_attribute);
                        stack.push(CompiledValue::Field(name));
                    } else {
                        stack.push(CompiledValue::Null);
                    }
                }
                0xb4 => {
                    let _ = compiled_pop(&mut stack);
                    stack.push(CompiledValue::Null);
                }
                0xb3 => { let _ = compiled_pop(&mut stack); }
                0xb5 => {
                    let value = compiled_pop(&mut stack);
                    let _owner = compiled_pop(&mut stack);
                    if let Some((_, field, _)) = class.member_reference(compiled_u16(&instruction.operands, 0).unwrap_or(0)) {
                        if matches!(field.as_str(), "field_70728_aV" | "xpReward" | "experienceReward") {
                            if let CompiledValue::Number(number) = value {
                                metadata.xp = number as i32;
                                recent_numeric = Some(number);
                            } else if let Some(number) = recent_numeric {
                                metadata.xp = number as i32;
                            }
                        }
                    }
                }
                0xb6..=0xb9 => {
                    let index = compiled_u16(&instruction.operands, 0).unwrap_or(0);
                    let Some((owner, name, descriptor)) = class.member_reference(index) else { continue; };
                    let Ok(parsed) = parse_method_descriptor(&descriptor) else { continue; };
                    let arguments = compiled_pop_n(&mut stack, parsed.parameters.len());
                    let receiver = if op == 0xb8 { CompiledValue::Null } else { compiled_pop(&mut stack) };

                    if name == "<init>" {
                        if let CompiledValue::Object(object_index) = receiver {
                            if let Some(object) = objects.get_mut(object_index) {
                                object.class_name = owner;
                                object.arguments = arguments;
                            }
                        }
                    } else if matches!(name.as_str(), "setSize" | "func_70105_a") && arguments.len() >= 2 {
                        let numbers: Vec<f64> = arguments.iter().filter_map(|value| {
                            if let CompiledValue::Number(number) = value { Some(*number) } else { None }
                        }).collect();
                        if numbers.len() >= 2 {
                            size_values.push((numbers[numbers.len() - 2], numbers[numbers.len() - 1]));
                        }
                    } else if matches!(name.as_str(), "setBaseValue" | "func_111128_a" | "add")
                        && current_attribute.is_some()
                    {
                        let value = arguments.last().and_then(|argument| {
                            if let CompiledValue::Number(number) = argument { Some(*number) } else { None }
                        });
                        if let (Some(attribute), Some(number)) = (current_attribute, value) {
                            match attribute {
                                "health" => metadata.health = number,
                                "movement_speed" => metadata.movement_speed = number,
                                "attack_damage" => metadata.attack_damage = number,
                                "follow_range" => metadata.follow_range = number,
                                "armor" => metadata.armor = number,
                                _ => {}
                            }
                            found_attributes.insert(attribute);
                        }
                        current_attribute = None;
                    } else if !matches!(parsed.return_type, crate::bytecode::JvmType::Void) {
                        stack.push(CompiledValue::Null);
                    }
                }
                0xba => {
                    let index = compiled_u16(&instruction.operands, 0).unwrap_or(0);
                    let count = class.invoke_dynamic(index)
                        .and_then(|(_, _, descriptor)| parse_method_descriptor(&descriptor).ok())
                        .map(|descriptor| descriptor.parameters.len())
                        .unwrap_or(0);
                    let _ = compiled_pop_n(&mut stack, count);
                    stack.push(CompiledValue::Null);
                }
                0xc0 | 0xc1 => {}
                0xac..=0xb1 | 0xbf => break,
                _ => {}
            }
        }
    }

    let has_size_assignment = !size_values.is_empty();
    if let Some((width, height)) = size_values.iter().copied()
        .max_by(|a, b| (a.0 * a.1).total_cmp(&(b.0 * b.1)))
    {
        metadata.width = width;
        metadata.height = height;
    }
    metadata.ai_classes = ai_classes.into_iter().collect();
    metadata.ai_classes.sort();

    for (attribute, label) in [
        ("health", "maximum health"),
        ("attack_damage", "attack damage"),
        ("movement_speed", "movement speed"),
    ] {
        if !found_attributes.contains(attribute) {
            metadata.warnings.push(format!(
                "{} was not assigned directly in this compiled entity class; the MCreator value is a safe fallback and may come from a superclass or custom stats helper.",
                label[..1].to_ascii_uppercase() + &label[1..]
            ));
        }
    }
    if !has_size_assignment {
        metadata.warnings.push("Entity dimensions were not assigned directly in the compiled class; default dimensions remain in use.".to_owned());
    }
    metadata.warnings.push(
        "Entity source was unavailable; compiled constructor and attribute bytecode were used. Unrecognized fields remain at defaults.".to_owned()
    );
    metadata
}

pub fn scan_mod_graph(root: &Path, source_roots: &[PathBuf]) -> Result<ModGraph> {
    let root = canonical_path(root);
    let roots = {
        let mut result = vec![root.clone()];
        for candidate in source_roots {
            let path = canonical_path(candidate);
            if !result.contains(&path) { result.push(path); }
        }
        result
    };
    let mut class_paths = list_source_files(&roots, "class");
    let java_paths = list_source_files(&roots, "java");
    let compiled_content = crate::compiled_registry::scan_compiled_registrations(&roots)?;
    for info in compiled_content.classes.values() {
        if !class_paths.contains(&info.path) {
            class_paths.push(info.path.clone());
        }
    }
    class_paths.sort();
    class_paths.dedup();
    let mut class_files = BTreeMap::<String, PathBuf>::new();
    for path in &class_paths {
        let class_name = simple_name(path.file_stem().and_then(|v| v.to_str()).unwrap_or("")).to_owned();
        let replace = class_files.get(&class_name).map(|old| {
            old.file_stem().map(|v| v.to_string_lossy().contains('$')).unwrap_or(false)
                && !path.file_stem().map(|v| v.to_string_lossy().contains('$')).unwrap_or(false)
        }).unwrap_or(true);
        if replace { class_files.insert(class_name, path.clone()); }
    }
    let mut java_files = BTreeMap::<String, PathBuf>::new();
    for path in &java_paths {
        let class_name = simple_name(path.file_stem().and_then(|v| v.to_str()).unwrap_or("")).to_owned();
        java_files.entry(class_name).or_insert_with(|| path.clone());
    }

    let index = source_index(&java_paths);
    // Entity registry IDs are separate from the broader item/block registry map.
    let mut entity_registry = compiled_registry_names(&compiled_content);
    for (entity, name) in registry_names(&java_paths) {
        entity_registry.insert(entity, name);
    }
    // The compiled generic registry scanner also sees modern RegistryObject
    // registrations. Keep only classes whose bytecode hierarchy is entity-like.
    for (class, names) in &compiled_content.registries {
        if !compiled_class_is_entity(class, &compiled_content.classes) { continue; }
        if let Some(name) = names.iter().next() {
            entity_registry.entry(class.clone()).or_insert_with(|| name.clone());
        }
    }
    let mut registry = entity_registry.clone();
    for (class, names) in &compiled_content.registries {
        if let Some(name) = names.iter().next() {
            registry.entry(class.clone()).or_insert_with(|| name.clone());
        }
    }
    for (entity, name) in &entity_registry {
        registry.insert(entity.clone(), name.clone());
    }
    let field_entities = registry_field_entities(&java_paths);
    let mut registrations = Vec::new();
    let mut warnings = compiled_content.warnings.clone();
    for path in &java_paths {
        let text = match read_java(path) { Ok(text) => text, Err(error) => {
            // Keep the scan alive when one source file is unreadable.
            let _ = error;
            continue;
        }};
        let call_pattern = Regex::new(r"\b(registerEntityRenderingHandler|registerRender|registerRenderer|registerEntityRenderer)\s*\(").unwrap();
        for found in call_pattern.find_iter(&text) {
            let open = text[found.start()..found.end()].rfind('(').unwrap() + found.start();
            let Ok((raw_args, _end)) = extract_balanced(&text, open, '(', ')') else { continue; };
            let name = call_pattern.captures(&text[found.start()..found.end()]).map(|v| v[1].to_owned()).unwrap_or_default();
            if let Some(registration) = parse_registration_call(&name, &raw_args, path, found.start(), &field_entities, &index) {
                registrations.push(registration);
            }
        }
    }

    // Decompiled source is not always available. Recover legacy class-literal
    // registrations directly from JVM Code attributes when the class constant
    // pool advertises one of the known renderer-registration methods.
    let registration_markers = [
        b"registerEntityRenderingHandler".as_slice(),
        b"registerRender".as_slice(),
        b"registerRenderer".as_slice(),
        b"registerEntityRenderer".as_slice(),
    ];
    let compiled_registrations: Vec<(PathBuf, Result<Vec<RendererRegistration>, String>)> =
        class_paths.par_iter().filter_map(|path| {
            let class = compiled_content.parsed_class_paths.get(path)?;
            let has_marker = class.constant_pool.iter().flatten().any(|entry| {
                matches!(entry, crate::bytecode::ConstantPoolEntry::Utf8(value)
                    if registration_markers.iter().any(|marker| value.as_bytes() == *marker))
            });
            if !has_marker { return None; }
            Some((path.clone(), parse_compiled_registration_class_file(class.as_ref(), path)
                .map_err(|error| format!("{error:#}"))))
        }).collect();
    let mut compiled_registrations = compiled_registrations;
    compiled_registrations.sort_by_key(|(path, _)| path.to_string_lossy().to_ascii_lowercase());
    for (path, parsed) in compiled_registrations {
        match parsed {
            Ok(found) => registrations.extend(found),
            Err(error) => warnings.push(format!(
                "Could not recover compiled renderer registrations from {}: {error}",
                path.display()
            )),
        }
    }

    // Add texture constants from the renderer class itself as well as references
    // that may live in the registration/bootstrap class.
    for registration in &mut registrations {
        if let Some(path) = class_files.get(&registration.renderer_class) {
            if let Some(class) = compiled_content.parsed_class_paths.get(path) {
                registration.texture_refs.extend(class_texture_references(class.as_ref()));
                registration.texture_refs.sort_by_key(|value| value.to_ascii_lowercase());
                registration.texture_refs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            }
        }
    }

    // Merge bytecode/source duplicates without collapsing genuine constructor
    // variants that use different model arguments, shadow sizes or visual scales.
    let mut seen = HashSet::new();
    registrations.retain(|reg| seen.insert((
        reg.entity_class.clone(),
        reg.renderer_class.clone(),
        reg.model_class.clone(),
        format!("{:?}", reg.model_args),
        reg.shadow_size.to_bits(),
        reg.visual_scale.to_bits(),
    )));
    registrations.sort_by_key(|reg| (
        reg.entity_class.to_ascii_lowercase(),
        reg.renderer_class.to_ascii_lowercase(),
        reg.source_offset
    ));

    let mut entities = BTreeMap::new();
    let mut entity_names: HashSet<String> = entity_registry.keys().cloned().collect();
    entity_names.extend(registrations.iter().map(|registration| registration.entity_class.clone()));
    for entity_name in entity_names {
        let entity_registry_name = entity_registry.get(&entity_name).or_else(|| registry.get(&entity_name));
        let source = java_files.get(&entity_name).or_else(|| java_files.get(simple_name(&entity_name)));
        let compiled = class_files.get(&entity_name).or_else(|| class_files.get(simple_name(&entity_name)));
        let metadata = if let Some(path) = source {
            parse_entity_source_metadata(path, entity_registry_name.map(String::as_str))
                .unwrap_or_else(|_| EntityMetadata {
                    class_name: entity_name.clone(), display_name: class_display_name(&entity_name),
                    registry_name: entity_registry_name.cloned().unwrap_or_else(|| registry_slug(&entity_name)),
                    superclass: String::new(), width: default_width(), height: default_height(),
                    health: default_health(), movement_speed: default_speed(), attack_damage: default_attack(),
                    armor: 0.0, follow_range: default_follow(), xp: 0, fire_immune: false, water_mob: false,
                    flying_mob: false, ai_classes: Vec::new(),
                    warnings: vec!["Entity source metadata was not fully parsed; defaults remain in use.".to_owned()]
                })
        } else if let Some(path) = compiled {
            if let Some(class) = compiled_content.parsed_class_paths.get(path) {
                parse_entity_class_metadata_from_class(
                    class.as_ref(), &entity_name, entity_registry_name.map(String::as_str)
                )
            } else {
                parse_entity_class_metadata(path, &entity_name, entity_registry_name.map(String::as_str))
            }
        } else {
            EntityMetadata {
                class_name: entity_name.clone(), display_name: class_display_name(&entity_name),
                registry_name: entity_registry_name.cloned().unwrap_or_else(|| registry_slug(&entity_name)),
                superclass: String::new(), width: default_width(), height: default_height(),
                health: default_health(), movement_speed: default_speed(), attack_damage: default_attack(),
                armor: 0.0, follow_range: default_follow(), xp: 0, fire_immune: false, water_mob: false,
                flying_mob: false, ai_classes: Vec::new(),
                warnings: vec!["Entity class source and compiled class were not found; metadata defaults remain in use.".to_owned()]
            }
        };
        entities.insert(entity_name, metadata);
    }

    if registrations.is_empty() {
        warnings.push("No supported source or compiled renderer registrations were found in the supplied roots.".to_owned());
    }

    Ok(ModGraph {
        root,
        source_roots: roots,
        class_files,
        java_files,
        registrations,
        entities,
        registry_names: registry,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn recovers_compiled_legacy_registry_name_from_class_and_string_constants() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("LegacyEntityRegistry.class"),
            include_bytes!("../tests/fixtures/LegacyEntityRegistry.class")
        ).unwrap();

        let graph = scan_mod_graph(root, &[]).unwrap();
        assert_eq!(graph.registry_names.get("EntityRedAnt").map(String::as_str), Some("red_ant"));
    }

    #[test]
    fn discovers_renderer_registration_from_a_compiled_only_mod_jar() {
        let temp = tempdir().unwrap();
        let archive_path = temp.path().join("compiled-only.jar");
        {
            let file = fs::File::create(&archive_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer.start_file(
                "example/LegacyRenderRegistry.class",
                zip::write::FileOptions::default(),
            ).unwrap();
            writer.write_all(include_bytes!("../tests/fixtures/LegacyRenderRegistry.class")).unwrap();
            writer.finish().unwrap();
        }

        let graph = scan_mod_graph(&archive_path, &[]).unwrap();
        assert_eq!(graph.registrations.len(), 1);
        assert_eq!(graph.registrations[0].entity_class, "EntityPig");
        assert_eq!(graph.registrations[0].renderer_class, "RenderPig");
        assert_eq!(graph.registrations[0].model_class, "ModelPig");
        assert!(graph.class_files.contains_key("LegacyRenderRegistry"));
        assert!(graph.class_files["LegacyRenderRegistry"].is_file());
    }

    #[test]
    fn connects_compiled_entity_class_literal_to_renderer_and_model_constructors() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("LegacyRenderRegistry.class"),
            include_bytes!("../tests/fixtures/LegacyRenderRegistry.class")
        ).unwrap();

        let graph = scan_mod_graph(root, &[]).unwrap();
        assert_eq!(graph.registrations.len(), 1);
        let registration = &graph.registrations[0];
        assert_eq!(registration.entity_class, "EntityPig");
        assert_eq!(registration.renderer_class, "RenderPig");
        assert_eq!(registration.model_class, "ModelPig");
        assert_eq!(registration.model_args, vec![json_number(0.25)]);
        assert!((registration.shadow_size - 0.4).abs() < 1e-6);
        assert_eq!(registration.visual_scale, 1.0);
        assert_eq!(graph.entities["EntityPig"].class_name, "EntityPig");
    }

    #[test]
    fn recovers_compiled_entity_dimensions_attributes_xp_and_ai() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("EntityStatFixture.class");
        let fixture = include_bytes!("../tests/fixtures/EntityStatFixture.class");
        assert_eq!(&fixture[..4], &[0xca, 0xfe, 0xba, 0xbe], "fixture must be a valid JVM class file");
        let parsed_class = ClassFile::parse(fixture).expect("compiled entity fixture should parse");
        assert!(parsed_class.methods.iter().any(|method| method.name == "init" && method.code.is_some()),
            "fixture should expose the init method Code attribute");
        fs::write(&path, fixture).unwrap();

        let metadata = parse_entity_class_metadata(&path, "EntityStatFixture", None);
        assert!((metadata.width - 1.2).abs() < 1e-5, "metadata recovered: {metadata:#?}");
        assert!((metadata.height - 1.4).abs() < 1e-5);
        assert!((metadata.health - 42.0).abs() < 1e-8);
        assert!((metadata.movement_speed - 0.28).abs() < 1e-8);
        assert!((metadata.attack_damage - 7.0).abs() < 1e-8);
        assert!((metadata.follow_range - 24.0).abs() < 1e-8);
        assert_eq!(metadata.xp, 12);
        assert!(metadata.fire_immune);
        assert!(metadata.flying_mob);
        assert!(metadata.ai_classes.contains(&"EntityAIAttackMelee".to_owned()));
    }

    #[test]
    fn prioritizes_the_entity_texture_accessor_over_renderer_overlays() {
        let source = r#"
            static ResourceLocation SHADOW = new ResourceLocation("examplemod", "textures/entity/shadow.png");
            public ResourceLocation getEntityTexture(Entity entity) {
                return new ResourceLocation("examplemod", "textures/entity/storm_drake");
            }
        "#;
        let references = source_texture_references(source);
        assert_eq!(references.first().map(String::as_str),
            Some("examplemod:textures/entity/storm_drake"));
        assert!(references.iter().any(|value| value == "examplemod:textures/entity/shadow.png"));
    }

    #[test]
    fn recovers_mob_texture_resource_locations_without_png_suffixes() {
        let references = source_texture_references(r#"
            public ResourceLocation getEntityTexture(Entity entity) {
                return new ResourceLocation("examplemod", "textures/entity/storm_drake");
            }
        "#);
        assert!(references.iter().any(|value| value == "examplemod:textures/entity/storm_drake"));

        let references = source_texture_references(r#"
            public ResourceLocation getTextureLocation(Entity entity) {
                return ResourceLocation.fromNamespaceAndPath("examplemod", "entity/stone_golem");
            }
        "#);
        assert!(references.iter().any(|value| value == "examplemod:entity/stone_golem"));
    }

    #[test]
    fn parses_modern_and_legacy_entity_attribute_builder_values() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("EntityAttributeFixture.java");
        fs::write(&path, r#"
            public class EntityAttributeFixture extends EntityMob {
                public void applyAttributes() {
                    getEntityAttribute(SharedMonsterAttributes.maxHealth).setBaseValue(120.0D);
                    getEntityAttribute(SharedMonsterAttributes.movementSpeed).setBaseValue(0.34D);
                    getEntityAttribute(SharedMonsterAttributes.attackDamage).setBaseValue(16.0D);
                    getEntityAttribute(SharedMonsterAttributes.followRange).setBaseValue(48.0D);
                    getEntityAttribute(SharedMonsterAttributes.armor).setBaseValue(10.0D);
                    this.experienceValue = 32;
                }
            }
        "#).unwrap();
        let metadata = parse_entity_source_metadata(&path, Some("attribute_fixture")).unwrap();
        assert_eq!(metadata.health, 120.0);
        assert_eq!(metadata.movement_speed, 0.34);
        assert_eq!(metadata.attack_damage, 16.0);
        assert_eq!(metadata.follow_range, 48.0);
        assert_eq!(metadata.armor, 10.0);
        assert_eq!(metadata.xp, 32);
    }

    #[test]
    fn parses_mojang_attribute_builder_values() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("EntityModernAttributes.java");
        fs::write(&path, r#"
            public class EntityModernAttributes extends Monster {
                AttributeSupplier.Builder createAttributes() {
                    return Mob.createMobAttributes()
                        .add(Attributes.MAX_HEALTH, 80.0D)
                        .add(Attributes.MOVEMENT_SPEED, 0.28D)
                        .add(Attributes.ATTACK_DAMAGE, 12.0D)
                        .add(Attributes.FOLLOW_RANGE, 32.0D)
                        .add(Attributes.ARMOR, 6.0D);
                }
            }
        "#).unwrap();
        let metadata = parse_entity_source_metadata(&path, Some("modern_attributes")).unwrap();
        assert_eq!(metadata.health, 80.0);
        assert_eq!(metadata.movement_speed, 0.28);
        assert_eq!(metadata.attack_damage, 12.0);
        assert_eq!(metadata.follow_range, 32.0);
        assert_eq!(metadata.armor, 6.0);
    }

    #[test]
    fn recognizes_compiled_entity_subclasses_but_not_item_subclasses() {
        use crate::compiled_registry::CompiledClassInfo;
        let classes = BTreeMap::from([
            ("CustomMobBase".to_owned(), CompiledClassInfo {
                class_name: "CustomMobBase".to_owned(),
                superclass: "net/minecraft/world/entity/monster/Monster".to_owned(),
                access_flags: 0,
                path: PathBuf::new(),
            }),
            ("FrostBeast".to_owned(), CompiledClassInfo {
                class_name: "FrostBeast".to_owned(),
                superclass: "demo/CustomMobBase".to_owned(),
                access_flags: 0,
                path: PathBuf::new(),
            }),
            ("AncientSword".to_owned(), CompiledClassInfo {
                class_name: "AncientSword".to_owned(),
                superclass: "net/minecraft/world/item/SwordItem".to_owned(),
                access_flags: 0,
                path: PathBuf::new(),
            }),
        ]);
        assert!(compiled_class_is_entity("FrostBeast", &classes));
        assert!(!compiled_class_is_entity("AncientSword", &classes));
    }

    #[test]
    fn discovers_registered_entity_without_renderer_registration() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/main/java/demo/entity")).unwrap();
        fs::write(root.join("src/main/java/demo/entity/EntityNoRenderer.java"), r#"
            public class EntityNoRenderer extends EntityMob {
                public void applyAttributes() {
                    getEntityAttribute(SharedMonsterAttributes.maxHealth).setBaseValue(36.0D);
                }
            }
        "#).unwrap();
        fs::write(root.join("src/main/java/demo/entity/EntityRegistry.java"), r#"
            public class EntityRegistry {
                RegistryObject<EntityType<EntityNoRenderer>> NO_RENDERER =
                    ENTITY_TYPES.register("no_renderer", () -> EntityType.Builder.of(EntityNoRenderer::new, MobCategory.MONSTER).build("no_renderer"));
            }
        "#).unwrap();
        let graph = scan_mod_graph(root, &[]).unwrap();
        let entity = graph.entities.get("EntityNoRenderer")
            .expect("registered mob must be discovered without a renderer registration");
        assert_eq!(entity.registry_name, "no_renderer");
        assert_eq!(entity.health, 36.0);
        assert!(graph.registrations.is_empty());
    }

    #[test]
    fn connects_registry_entity_renderer_and_model_constructor() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/main/java/demo/entity")).unwrap();
        fs::create_dir_all(root.join("src/main/java/demo/client")).unwrap();
        fs::write(root.join("src/main/java/demo/entity/EntityFireBeetle.java"), r#"
            package demo.entity;
            public class EntityFireBeetle extends EntityMob {
                public EntityFireBeetle() {
                    setSize(1.2F, 1.4F);
                    maxHealth = 42.0D;
                    movementSpeed = 0.28D;
                    attackDamage = 7.0D;
                    tasks.addTask(1, new EntityAIAttackMelee(this, 1.0D, true));
                }
            }
        "#).unwrap();
        fs::write(root.join("src/main/java/demo/entity/EntityRegistry.java"), r#"
            public class EntityRegistry {
                RegistryObject<EntityType<EntityFireBeetle>> FIRE_BEETLE =
                    ENTITY_TYPES.register("fire_beetle", () -> EntityType.Builder.of(EntityFireBeetle::new, MobCategory.MONSTER).build("fire_beetle"));
            }
        "#).unwrap();
        fs::write(root.join("src/main/java/demo/client/ModelFireBeetle.java"), "public class ModelFireBeetle {}").unwrap();
        fs::write(root.join("src/main/java/demo/client/RenderFireBeetle.java"), r#"
            public class RenderFireBeetle extends MobRenderer {
                public RenderFireBeetle(Context ctx) {
                    super(ctx, new ModelFireBeetle(0.25F), 0.4F);
                }
            }
        "#).unwrap();
        fs::write(root.join("src/main/java/demo/client/ClientRegistration.java"), r#"
            public class ClientRegistration {
                void register() {
                    event.registerEntityRenderer(EntityRegistry.FIRE_BEETLE.get(), RenderFireBeetle::new);
                }
            }
        "#).unwrap();

        let graph = scan_mod_graph(root, &[]).unwrap();
        assert_eq!(graph.registrations.len(), 1);
        let registration = &graph.registrations[0];
        assert_eq!(registration.entity_class, "EntityFireBeetle");
        assert_eq!(registration.renderer_class, "RenderFireBeetle");
        assert_eq!(registration.model_class, "ModelFireBeetle");
        assert_eq!(registration.model_args, vec![json_number(0.25)]);
        assert!((registration.shadow_size - 0.4).abs() < 1e-8);
        let metadata = &graph.entities["EntityFireBeetle"];
        assert_eq!(metadata.registry_name, "fire_beetle");
        assert_eq!((metadata.width, metadata.height), (1.2, 1.4));
        assert_eq!(metadata.health, 42.0);
        assert!(metadata.ai_classes.contains(&"EntityAIAttackMelee".to_owned()));
    }

    #[test]
    fn connects_legacy_class_literal_registration_and_registry_names() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/EntityRedAnt.java"), "public class EntityRedAnt extends EntityMob {}").unwrap();
        fs::write(root.join("src/RenderRedAnt.java"), "public class RenderRedAnt { ModelRedAnt model = new ModelRedAnt(0.1F); }").unwrap();
        fs::write(root.join("src/ModelRedAnt.java"), "public class ModelRedAnt {}").unwrap();
        fs::write(root.join("src/RenderRegistry.java"), r#"
            public class RenderRegistry {
                void register() {
                    RenderingRegistry.registerEntityRenderingHandler(EntityRedAnt.class, new RenderRedAnt());
                }
            }
        "#).unwrap();
        let graph = scan_mod_graph(root, &[]).unwrap();
        assert_eq!(graph.registrations.len(), 1);
        assert_eq!(graph.registrations[0].entity_class, "EntityRedAnt");
        assert_eq!(graph.registrations[0].renderer_class, "RenderRedAnt");
        assert_eq!(graph.registrations[0].model_class, "ModelRedAnt");
    }
}
