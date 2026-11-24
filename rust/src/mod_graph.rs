use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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

fn class_display_name(class_name: &str) -> String {
    let name = class_name.strip_prefix("Entity").or_else(|| class_name.strip_prefix("Mob")).unwrap_or(class_name);
    let camel = Regex::new(r"(?<=[a-z0-9])(?=[A-Z])").unwrap();
    let spaced = camel.replace_all(name, " ");
    let cleaned = Regex::new(r"[_$]+").unwrap().replace_all(&spaced, " ");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn registry_slug(value: &str) -> String {
    let name = value.strip_prefix("Entity").or_else(|| value.strip_prefix("Mob")).unwrap_or(value);
    let camel = Regex::new(r"(?<=[a-z0-9])(?=[A-Z])").unwrap().replace_all(name, "_");
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
    (model, args, shadow, visual_scale, png_refs(&text))
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
    textures.extend(png_refs(raw_args));
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
    let ai = Regex::new(r"new\s+(EntityAI[A-Za-z0-9_$]+|[A-Za-z0-9_$]*Goal)\s*\(").unwrap();
    metadata.ai_classes = ai.captures_iter(&text).map(|v| v[1].to_owned()).collect::<HashSet<_>>().into_iter().collect();
    metadata.ai_classes.sort();
    Ok(metadata)
}

/// Scan mixed source/compiled roots and build the first native entity →
/// renderer → model graph. Source registrations cover legacy FML calls,
/// RegistryObject.get() lambdas and renderer constructor references.
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
    let class_paths = list_source_files(&roots, "class");
    let java_paths = list_source_files(&roots, "java");
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
    let registry = registry_names(&java_paths);
    let field_entities = registry_field_entities(&java_paths);
    let registration_names = ["registerEntityRenderingHandler","registerRender","registerRenderer","registerEntityRenderer"];
    let mut registrations = Vec::new();
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

    // Preserve exact entity class -> renderer links while removing repeated
    // discovery of the same source registration.
    let mut seen = HashSet::new();
    registrations.retain(|reg| seen.insert((
        reg.entity_class.clone(), reg.renderer_class.clone(), reg.model_class.clone(),
        format!("{:?}", reg.model_args), reg.source_class.clone(), reg.source_offset,
    )));
    registrations.sort_by_key(|reg| (reg.entity_class.to_ascii_lowercase(), reg.renderer_class.to_ascii_lowercase(), reg.source_offset));

    let mut entities = BTreeMap::new();
    for registration in &registrations {
        if entities.contains_key(&registration.entity_class) { continue; }
        let source = java_files.get(&registration.entity_class)
            .or_else(|| java_files.get(&simple_name(&registration.entity_class)));
        let metadata = if let Some(path) = source {
            parse_entity_source_metadata(path, registry.get(&registration.entity_class).map(String::as_str))
                .unwrap_or_else(|_| EntityMetadata {
                    class_name: registration.entity_class.clone(),
                    display_name: class_display_name(&registration.entity_class),
                    registry_name: registry.get(&registration.entity_class).cloned().unwrap_or_else(|| registry_slug(&registration.entity_class)),
                    superclass: String::new(), width: default_width(), height: default_height(),
                    health: default_health(), movement_speed: default_speed(), attack_damage: default_attack(),
                    armor: 0.0, follow_range: default_follow(), xp: 0, fire_immune: false, water_mob: false,
                    flying_mob: false, ai_classes: Vec::new(),
                    warnings: vec!["Entity source metadata was not fully parsed; defaults remain in use.".to_owned()]
                })
        } else {
            EntityMetadata {
                class_name: registration.entity_class.clone(),
                display_name: class_display_name(&registration.entity_class),
                registry_name: registry.get(&registration.entity_class).cloned().unwrap_or_else(|| registry_slug(&registration.entity_class)),
                superclass: String::new(), width: default_width(), height: default_height(),
                health: default_health(), movement_speed: default_speed(), attack_damage: default_attack(),
                armor: 0.0, follow_range: default_follow(), xp: 0, fire_immune: false, water_mob: false,
                flying_mob: false, ai_classes: Vec::new(),
                warnings: vec!["Entity class source was not found; metadata defaults remain in use.".to_owned()]
            }
        };
        entities.insert(registration.entity_class.clone(), metadata);
    }

    let mut warnings = Vec::new();
    if registrations.is_empty() {
        warnings.push("No supported source renderer registrations were found. Compiled class registration interpretation is not yet wired into this graph stage.".to_owned());
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
    use tempfile::tempdir;

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
