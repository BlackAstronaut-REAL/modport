use anyhow::{Context, Result};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const ARMOR_PARTS: [(&str, &[&str]); 4] = [
    ("helmet", &["helmet", "helm", "head"]),
    ("body", &["body", "chest", "chestplate", "torso"]),
    ("leggings", &["leggings", "legs", "pants"]),
    ("boots", &["boots", "boot", "feet", "shoes"]),
];

#[derive(Debug, Clone, Serialize)]
pub struct ArmorSetSource {
    pub prefix: String,
    pub registry_name: String,
    pub fields: BTreeMap<String, String>,
    pub registered_piece_ids: BTreeMap<String, String>,
    pub source_paths: Vec<String>,
    #[serde(skip)]
    pub source_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArmorExportRecord {
    pub prefix: String,
    pub element_name: String,
    pub registry_name: String,
    pub piece_textures: BTreeMap<String, String>,
    pub armor_texture_prefix: String,
    pub source_paths: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArmorWorkspaceExportReport {
    pub workspace_file: PathBuf,
    pub discovered: usize,
    pub exported: usize,
    pub skipped: usize,
    pub records: Vec<ArmorExportRecord>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct ArmorGroup {
    fields: BTreeMap<String, String>,
    paths: BTreeSet<PathBuf>,
    texts: Vec<String>,
}

fn simple_name(value: &str) -> String {
    value.rsplit('.').next().unwrap_or(value).rsplit('$').next().unwrap_or(value).to_owned()
}

fn snake(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower) {
                out.push('_');
            }
        }
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_owned();
    if out.is_empty() { "converted_armor".to_owned() } else { out }
}

fn pascal(value: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() {
            upper = true;
            continue;
        }
        if upper {
            out.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() { out = "ConvertedArmor".to_owned(); }
    if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out.insert_str(0, "Armor");
    }
    out
}

fn normalize_stem(value: &str) -> String {
    value.trim().replace('\\', "/").rsplit('/').next().unwrap_or(value)
        .trim_end_matches(".png").to_ascii_lowercase()
}

fn normalize_registry(value: &str) -> String {
    let mut value = value.trim().trim_matches('"').trim_matches('\'').to_owned();
    if let Some((_, path)) = value.split_once(':') {
        value = path.to_owned();
    }
    value.trim_matches('/').replace('/', "_").replace('-', "_").to_ascii_lowercase()
}

fn json_string(value: &Value, key: &str, default: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or(default).to_owned()
}

fn java_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in roots {
        if !root.is_dir() { continue; }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("java")) {
                files.push(entry.into_path());
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn strip_comments(source: &str) -> String {
    crate::source::java::strip_java_comments(source)
}

fn field_part(field: &str) -> Option<(&'static str, String)> {
    let lower = field.to_ascii_lowercase();
    let suffixes = [
        ("helmet", "helmet"),
        ("helm", "helmet"),
        ("head", "helmet"),
        ("body", "body"),
        ("chestplate", "body"),
        ("chest", "body"),
        ("torso", "body"),
        ("leggings", "leggings"),
        ("legs", "leggings"),
        ("pants", "leggings"),
        ("boots", "boots"),
        ("boot", "boots"),
        ("feet", "boots"),
        ("shoes", "boots"),
    ];
    for (suffix, part) in suffixes {
        if lower.ends_with(suffix) && field.len() > suffix.len() {
            let prefix = field[..field.len() - suffix.len()].trim_end_matches(['_', '-']);
            if !prefix.is_empty() {
                return Some((part, prefix.to_owned()));
            }
        }
    }
    None
}

fn group_armor_fields(source: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut groups = BTreeMap::<String, BTreeMap<String, String>>::new();
    let field_regex = Regex::new(
        r"\b(?:[A-Za-z_$][A-Za-z0-9_$.]*Armor|ItemArmor)\s+([A-Za-z0-9_]+)\s*;"
    ).unwrap();
    for capture in field_regex.captures_iter(source) {
        let Some(field) = capture.get(1).map(|value| value.as_str()) else { continue };
        if let Some((part, prefix)) = field_part(field) {
            groups.entry(prefix).or_default().insert(part.to_owned(), field.to_owned());
        }
    }

    // The legacy engine also groups nested ArmorSet containers that expose no
    // top-level four-piece declarations. Keep the inferred piece aliases explicit.
    let set_regex = Regex::new(r"\b[A-Za-z0-9_.$]*ArmorSet\s+([A-Za-z0-9_]+?)(?:Set)?\s*;").unwrap();
    for capture in set_regex.captures_iter(source) {
        let Some(prefix) = capture.get(1).map(|value| value.as_str()) else { continue };
        let fields = groups.entry(prefix.to_owned()).or_default();
        fields.entry("helmet".to_owned()).or_insert_with(|| format!("{prefix}_helmet"));
        fields.entry("body".to_owned()).or_insert_with(|| format!("{prefix}_chestplate"));
        fields.entry("leggings".to_owned()).or_insert_with(|| format!("{prefix}_leggings"));
        fields.entry("boots".to_owned()).or_insert_with(|| format!("{prefix}_boots"));
    }
    groups
}

fn quoted_values(source: &str) -> Vec<String> {
    let regex = Regex::new(r#"\"([^\"\\]*(?:\\.[^\"\\]*)*)\""#).unwrap();
    regex.captures_iter(source)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
        .collect()
}

fn registration_calls(source: &str) -> Vec<(String, String)> {
    use crate::source::java::extract_balanced;
    let source = strip_comments(source);
    let bytes = source.as_bytes();
    let mut output = Vec::new();
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
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' || bytes[index] == 36) {
                index += 1;
            }
            let name = &source[start..index];
            if name != "registerItem" && name != "registerBlock" {
                continue;
            }
            let mut open = index;
            while open < bytes.len() && bytes[open].is_ascii_whitespace() { open += 1; }
            if open >= bytes.len() || bytes[open] != b'(' { continue; }
            if let Ok((arguments, end)) = extract_balanced(&source, open, '(', ')') {
                output.push((name.to_owned(), arguments));
                index = end;
            }
            continue;
        }
        index += 1;
    }
    output
}

fn field_registry_map(source: &str) -> BTreeMap<String, String> {
    use crate::source::java::split_top_level;
    let mut result = BTreeMap::new();
    for (_call, args) in registration_calls(source) {
        let args = split_top_level(&args, ',');
        if args.len() < 2 { continue; }
        let field = args[0].trim().rsplit('.').next().unwrap_or("").trim();
        if field.is_empty() { continue; }
        let values = quoted_values(&args[1]);
        let Some(raw) = values.last() else { continue };
        let registry = normalize_registry(raw);
        if !registry.is_empty() {
            result.insert(field.to_owned(), registry);
        }
    }

    // Modernized source may assign an item reference via BuiltInRegistries rather
    // than an explicit registration helper.
    let modern = Regex::new(
        r#"(?s)\b([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:\([^;]+?\)\s*)?BuiltInRegistries\.ITEM\.get\s*\(\s*ResourceLocation\.fromNamespaceAndPath\s*\(\s*[^,]+,\s*"([^"]+)""#
    ).unwrap();
    for capture in modern.captures_iter(source) {
        let field = capture.get(1).unwrap().as_str();
        let registry = normalize_registry(capture.get(2).unwrap().as_str());
        if !registry.is_empty() { result.insert(field.to_owned(), registry); }
    }
    result
}

fn armor_group_registry(prefix: &str, fields: &BTreeMap<String, String>, ids: &BTreeMap<String, String>) -> String {
    // Piece ids are the strongest signal for the material prefix. Strip the part
    // suffix, but keep a stable single armor registry rather than registering four items.
    for part in ["helmet", "body", "leggings", "boots"] {
        if let Some(field) = fields.get(part) {
            if let Some(registry) = ids.get(field) {
                let lower = registry.to_ascii_lowercase();
                for (_, aliases) in ARMOR_PARTS {
                    for suffix in aliases {
                        let suffix = format!("_{suffix}");
                        if lower.ends_with(&suffix) {
                            return format!("{}_armor", lower.trim_end_matches(&suffix).trim_end_matches('_'));
                        }
                    }
                }
            }
        }
    }
    format!("{}_armor", snake(prefix))
}

fn discover_armor_sources(source_roots: &[PathBuf]) -> Result<Vec<ArmorSetSource>> {
    let files = java_files(source_roots);
    let mut groups = BTreeMap::<String, ArmorGroup>::new();

    for path in files {
        let Ok(bytes) = fs::read(&path) else { continue };
        let source_text = strip_comments(&String::from_utf8_lossy(&bytes));
        let found = group_armor_fields(&source_text);
        if found.is_empty() { continue; }

        for (prefix, fields) in found {
            let group = groups.entry(prefix).or_default();
            for (part, field) in fields {
                group.fields.insert(part, field);
            }
            group.paths.insert(path.clone());
            group.texts.push(source_text.clone());
        }
    }

    let mut results = Vec::new();
    let combined_registry_source = all_texts.join("\n");
    for (prefix, group) in groups {
        let ids = field_registry_map(&combined_registry_source);
        let registry_name = armor_group_registry(&prefix, &group.fields, &ids);
        results.push(ArmorSetSource {
            prefix,
            registry_name,
            fields:group.fields,
            registered_piece_ids:ids,
            source_paths:group.paths.iter().map(|path| path.to_string_lossy().replace('\\', "/")).collect(),
            source_text:combined,
        });
    }
    results.sort_by(|a,b| a.registry_name.cmp(&b.registry_name));
    Ok(results)
}

fn armor_piece_stems(prefix: &str, field_name: &str, part: &str, registered_id: Option<&str>) -> Vec<String> {
    let mut stems = Vec::new();
    if let Some(id) = registered_id {
        stems.push(normalize_stem(id));
    }
    stems.push(normalize_stem(field_name));
    let base = snake(prefix);
    if let Some((_, aliases)) = ARMOR_PARTS.iter().find(|(name,_)| *name == part) {
        for alias in *aliases {
            stems.push(format!("{base}_{alias}"));
        }
    }
    let mut seen = BTreeSet::new();
    stems.retain(|stem| !stem.is_empty() && seen.insert(stem.to_ascii_lowercase()));
    stems
}

fn texture_matches(source_roots: &[PathBuf], stems: &[String], part: &str) -> Vec<PathBuf> {
    let aliases = ARMOR_PARTS.iter().find(|(name,_)| *name == part).map(|(_,aliases)| *aliases).unwrap_or(&[]);
    let mut exact = Vec::new();
    let mut fallback = Vec::<(i32, PathBuf)>::new();

    for root in source_roots {
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if !entry.file_type().is_file()
                || !path.extension().and_then(|value| value.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("png")) {
                continue;
            }
            let full = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            if !full.contains("/textures/item/") && !full.contains("/textures/items/") {
                continue;
            }
            let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            if stems.iter().any(|expected| stem == expected.to_ascii_lowercase()) {
                exact.push(path.to_path_buf());
                continue;
            }
            let compact: String = stem.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect();
            let part_suffix = aliases.iter().any(|alias| {
                let suffix: String = alias.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect();
                compact.ends_with(&suffix)
            });
            if !part_suffix { continue; }

            let mut score = 0;
            for expected in stems {
                let base: String = expected.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect();
                if base.is_empty() { continue; }
                let base_without_part = aliases.iter().find_map(|alias| {
                    let suffix: String = alias.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect();
                    base.strip_suffix(&suffix).map(str::to_owned)
                }).unwrap_or_else(|| base.clone());
                if compact.starts_with(&base_without_part) {
                    score = score.max(75 + base_without_part.len().min(12) as i32);
                }
            }
            if score > 0 { fallback.push((score, path.to_path_buf())); }
        }
    }

    exact.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    if !exact.is_empty() { return exact; }
    fallback.sort_by(|a,b| b.0.cmp(&a.0).then_with(|| a.1.to_string_lossy().to_ascii_lowercase().cmp(&b.1.to_string_lossy().to_ascii_lowercase())));
    fallback.into_iter().map(|(_,path)| path).collect()
}

fn armor_layer_stems(prefix: &str, layer: usize) -> Vec<String> {
    let base = snake(prefix);
    vec![
        format!("{base}_layer_{layer}"),
        format!("{base}__layer_{layer}"),
        format!("{base}_{layer}"),
        format!("{base}layer{layer}"),
    ]
}

fn armor_layer_candidates(source_roots: &[PathBuf], stems: &[String]) -> Vec<PathBuf> {
    let exact_stems: BTreeSet<String> = stems.iter().map(|value| value.to_ascii_lowercase()).collect();
    let mut paths = Vec::new();
    for root in source_roots {
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if !entry.file_type().is_file()
                || !path.extension().and_then(|value| value.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("png")) {
                continue;
            }
            let full = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            if !full.contains("/models/armor/") && !full.contains("/textures/armor/") && !full.contains("/armor/") {
                continue;
            }
            let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            if exact_stems.contains(&stem) { paths.push(path.to_path_buf()); }
        }
    }
    paths.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    paths
}

fn copy_texture(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
    fs::copy(source, target).with_context(|| format!("Copying {} to {}", source.display(), target.display()))?;
    crate::source_bridge::copy_animation_metadata(source, target)?;
    Ok(())
}

fn placeholder_png() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+iZ1sAAAAASUVORK5CYII="
    ).expect("embedded PNG placeholder")
}

fn armor_template(workspace_root: &Path) -> Result<Value> {
    let elements = workspace_root.join("elements");
    if elements.is_dir() {
        let mut paths: Vec<PathBuf> = fs::read_dir(elements)?.filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
            if value.get("_type").and_then(Value::as_str) == Some("armor")
                && value.get("definition").is_some_and(Value::is_object) {
                return Ok(value);
            }
        }
    }
    serde_json::from_str(include_str!("../../oldVer/mcreator_templates/ArmorSchema.mod.json"))
        .context("Parsing embedded MCreator armor schema")
}

fn find_workspace_descriptor(root: &Path) -> Result<PathBuf> {
    let mut paths = Vec::<(i64, usize, PathBuf)>::new();
    for entry in fs::read_dir(root).with_context(|| format!("Reading workspace root {}", root.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("mcreator") { continue; }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
        if !value.get("workspaceSettings").is_some_and(Value::is_object)
            || !value.get("mod_elements").is_some_and(Value::is_array) {
            continue;
        }
        let version = value.get("mcreatorVersion").and_then(Value::as_i64).unwrap_or(0);
        let count = value.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        paths.push((version,count,path));
    }
    paths.sort_by_key(|(version,count,path)| (*version,*count,path.to_string_lossy().to_ascii_lowercase()));
    paths.pop().map(|(_,_,path)|path)
        .ok_or_else(|| anyhow::anyhow!("No valid MCreator workspace descriptor found in {}",root.display()))
}

fn unique_element_name(prefix: &str, used: &mut HashSet<String>) -> String {
    let base = format!("{}Armor", pascal(prefix));
    let mut candidate = base.clone();
    let mut suffix = 2;
    while !used.insert(candidate.to_ascii_lowercase()) {
        candidate = format!("{base}{suffix}");
        suffix += 1;
    }
    candidate
}

fn update_language(workspace: &mut Value, modid: &str, registry: &str, display: &str) {
    if !workspace.get("language_map").is_some_and(Value::is_object) {
        workspace["language_map"] = json!({});
    }
    if !workspace["language_map"].get("en_us").is_some_and(Value::is_object) {
        workspace["language_map"]["en_us"] = json!({});
    }
    let language = workspace["language_map"]["en_us"].as_object_mut().unwrap();
    language.insert(format!("item.{modid}.{registry}"), json!(display));
    for part in ["helmet","body","leggings","boots"] {
        let name = format!("{} {}", display, match part {"body"=>"Chestplate",_=>part});
        language.insert(format!("item.{modid}.{registry}_{part}"), json!(name));
        language.insert(format!("item.{modid}.{registry}_{part}.name"), json!(name));
    }
}

fn armor_display_name(prefix: &str) -> String {
    let registry = snake(prefix);
    registry.split('_').filter(|value| !value.is_empty()).map(|part| {
        let mut chars = part.chars();
        match chars.next() {
            Some(first) => {
                let mut result = first.to_ascii_uppercase().to_string();
                result.extend(chars);
                result
            }
            None => String::new(),
        }
    }).collect::<Vec<_>>().join(" ")
}

/// Build one native MCreator armor element from the four source-piece fields.
pub fn build_armor_element(
    template: Option<&Value>,
    prefix: &str,
    registry: &str,
    pieces: &BTreeMap<String,String>,
    piece_textures: &BTreeMap<String,String>,
    armor_texture_prefix: &str,
    stats: Option<&BTreeMap<String,f64>>,
    imported_fv: i64,
) -> (Value, Vec<String>) {
    let template = template.cloned().unwrap_or_else(|| {
        serde_json::from_str(include_str!("../../oldVer/mcreator_templates/ArmorSchema.mod.json"))
            .expect("embedded armor template is valid JSON")
    });
    let mut definition = template.get("definition").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut warnings = Vec::new();
    let display = armor_display_name(prefix);

    definition.insert("creativeTabs".to_owned(), json!(["COMBAT"]));
    definition.insert("armorTextureFile".to_owned(), json!(armor_texture_prefix));
    definition.insert("maxDamage".to_owned(), json!(stats.and_then(|s|s.get("durability")).copied().unwrap_or(25.0).max(1.0) as i64));
    definition.insert("damageValueHelmet".to_owned(), json!(stats.and_then(|s|s.get("helmet")).copied().unwrap_or(6.0) as i64));
    definition.insert("damageValueBody".to_owned(), json!(stats.and_then(|s|s.get("body")).copied().unwrap_or(8.0) as i64));
    definition.insert("damageValueLeggings".to_owned(), json!(stats.and_then(|s|s.get("leggings")).copied().unwrap_or(6.0) as i64));
    definition.insert("damageValueBoots".to_owned(), json!(stats.and_then(|s|s.get("boots")).copied().unwrap_or(6.0) as i64));
    definition.insert("enchantability".to_owned(), json!(stats.and_then(|s|s.get("enchantability")).copied().unwrap_or(10.0) as i64));
    definition.insert("toughness".to_owned(), json!(stats.and_then(|s|s.get("toughness")).copied().unwrap_or(0.0)));
    definition.insert("rarity".to_owned(), json!("COMMON"));
    definition.insert("repairItems".to_owned(), Value::Array(Vec::new()));
    definition.insert("name".to_owned(), json!(display));

    for (part, aliases) in ARMOR_PARTS {
        let enabled = pieces.contains_key(part);
        let title = match part {
            "helmet" => "Helmet",
            "body" => "Body",
            "leggings" => "Leggings",
            "boots" => "Boots",
            _ => part,
        };
        definition.insert(format!("enable{}", title), json!(enabled));
        definition.insert(format!("texture{}", title), json!(piece_textures.get(part).cloned().unwrap_or_default()));
        let piece_name = match part { "body" => format!("{display} Chestplate"), _ => format!("{display} {}", title) };
        definition.insert(format!("{}Name", part), json!(if enabled { piece_name } else { String::new() }));
        if !enabled {
            definition.insert(format!("texture{}", title), json!(""));
        }
        let _ = aliases;
        if enabled && !piece_textures.contains_key(part) {
            warnings.push(format!("Armor piece {part} was registered but no corresponding texture was resolved."));
        }
    }

    let fv = template.get("_fv").and_then(Value::as_i64).unwrap_or(imported_fv).clamp(0,89);
    let element = json!({"_fv":fv,"_type":"armor","definition":definition});
    let _ = registry;
    if stats.is_none() {
        warnings.push("Armor defense values, durability, enchantability, and special effects use conservative defaults.".to_owned());
    }
    (element,warnings)
}

/// Export grouped Java armor sets into MCreator armor elements and resource files.
pub fn export_armor_to_workspace(
    source_roots: &[PathBuf],
    workspace_root: &Path,
    source_modid: &str,
    imported_fv: i64,
) -> Result<ArmorWorkspaceExportReport> {
    let workspace_root = fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let workspace_file = find_workspace_descriptor(&workspace_root)?;
    let bytes = fs::read(&workspace_file)?;
    let mut workspace: Value = serde_json::from_slice(&bytes)?;
    let target_modid = json_string(workspace.get("workspaceSettings").unwrap_or(&Value::Null),"modid","converted_mod");
    let discovered = discover_armor_sources(source_roots)?;

    let mut report = ArmorWorkspaceExportReport {
        workspace_file:workspace_file.clone(),discovered:discovered.len(),exported:0,skipped:0,records:Vec::new(),warnings:Vec::new()
    };
    let mut existing = BTreeSet::<String>::new();
    let mut used_names = HashSet::<String>::new();
    for entry in workspace.get("mod_elements").and_then(Value::as_array).into_iter().flatten() {
        let kind = json_string(entry,"type","");
        let registry = json_string(entry,"registry_name","").to_ascii_lowercase();
        if kind.eq_ignore_ascii_case("armor") { existing.insert(registry); }
        let name=json_string(entry,"name","");
        if !name.is_empty() { used_names.insert(name.to_ascii_lowercase()); }
    }

    let output_elements = workspace_root.join("elements");
    let item_texture_dir = workspace_root.join("src/main/resources/assets").join(&target_modid).join("textures/item");
    let layer_texture_dir = workspace_root.join("src/main/resources/assets").join(&target_modid).join("textures/models/armor");
    let legacy_layer_dir = workspace_root.join("src/main/resources/assets").join(&target_modid).join("textures/armor");
    fs::create_dir_all(&output_elements)?;
    fs::create_dir_all(&item_texture_dir)?;
    fs::create_dir_all(&layer_texture_dir)?;
    fs::create_dir_all(&legacy_layer_dir)?;

    let mut template_cache = None::<Value>;
    for set in discovered {
        if existing.contains(&set.registry_name) {
            report.skipped += 1;
            continue;
        }
        let template = if let Some(template) = template_cache.as_ref() {
            template.clone()
        } else {
            let template = armor_template(&workspace_root)?;
            template_cache = Some(template.clone());
            template
        };
        let mut piece_textures = BTreeMap::<String,String>::new();
        let mut metadata_files = BTreeSet::<String>::new();
        let mut warnings = Vec::new();

        for (part, _aliases) in ARMOR_PARTS {
            let Some(field) = set.fields.get(part) else { continue };
            let registered_id = set.registered_piece_ids.get(field).map(String::as_str);
            let stems = armor_piece_stems(&set.prefix, field, part, registered_id);
            let candidates = texture_matches(source_roots,&stems,part);
            let desired = stems.first().cloned().unwrap_or_else(|| format!("{}_{}",snake(&set.prefix),part));
            let (texture_stem,source_path) = if let Some(source) = candidates.first() {
                let mut stem = source.file_stem().and_then(|value|value.to_str()).unwrap_or(&desired).to_owned();
                let mut target = item_texture_dir.join(format!("{stem}.png"));
                if target.exists() && fs::read(&target)? != fs::read(source)? {
                    stem=format!("{}_{}",set.registry_name,stem);
                    target=item_texture_dir.join(format!("{stem}.png"));
                }
                copy_texture(source,&target)?;
                (stem,Some((source.clone(),target)))
            } else {
                let stem=desired;
                let target=item_texture_dir.join(format!("{stem}.png"));
                if !target.exists() { fs::write(&target,placeholder_png())?; }
                warnings.push(format!("No exact armor item texture found for {part}; generated placeholder '{}.png'.",stem));
                (stem,None)
            };
            piece_textures.insert(part.to_owned(),texture_stem.clone());
            let target=item_texture_dir.join(format!("{texture_stem}.png"));
            metadata_files.insert(target.strip_prefix(&workspace_root).unwrap_or(&target).to_string_lossy().replace('\\',"/"));
            let meta=PathBuf::from(format!("{}.mcmeta",target.display()));
            if meta.is_file() {
                metadata_files.insert(meta.strip_prefix(&workspace_root).unwrap_or(&meta).to_string_lossy().replace('\\',"/"));
            }
            let _ = source_path;
        }

        let mut layer_one = None;
        let mut layer_two = None;
        let mut layer_prefix = snake(&set.prefix);
        let mut bases = vec![snake(&set.prefix)];
        for value in piece_textures.values() {
            let mut stem=value.clone();
            for (_,aliases) in ARMOR_PARTS {
                for alias in aliases {
                    if stem.ends_with(alias) {
                        stem.truncate(stem.len()-alias.len());
                        stem=stem.trim_end_matches('_').to_owned();
                    }
                }
            }
            if !stem.is_empty() { bases.push(stem); }
        }
        bases.sort();
        bases.dedup();
        let mut candidates_one=Vec::new();
        let mut candidates_two=Vec::new();
        for base in &bases {
            candidates_one.extend(armor_layer_candidates(source_roots,&armor_layer_stems(base,1)));
            candidates_two.extend(armor_layer_candidates(source_roots,&armor_layer_stems(base,2)));
        }
        candidates_one.dedup();
        candidates_two.dedup();
        if let Some(source)=candidates_one.first() {
            let stem=source.file_stem().and_then(|value|value.to_str()).unwrap_or(&layer_prefix);
            if let Some(stripped)=stem.strip_suffix("__layer_1") {
                layer_prefix=format!("{stripped}_");
            } else if let Some(stripped)=stem.strip_suffix("_layer_1") {
                layer_prefix=stripped.to_owned();
            } else if let Some(stripped)=stem.strip_suffix("_1") {
                layer_prefix=stripped.to_owned();
            } else if let Some(stripped)=stem.strip_suffix("layer1") {
                layer_prefix=stripped.trim_end_matches('_').to_owned();
            }
            layer_one=Some(source.clone());
        }
        if let Some(source)=candidates_two.first() { layer_two=Some(source.clone()); }

        for (number,source) in [(1,layer_one.as_ref()),(2,layer_two.as_ref())] {
            if let Some(source)=source {
                let target=layer_texture_dir.join(format!("{layer_prefix}_layer_{number}.png"));
                copy_texture(source,&target)?;
                metadata_files.insert(target.strip_prefix(&workspace_root).unwrap_or(&target).to_string_lossy().replace('\\',"/"));
                let alias=legacy_layer_dir.join(source.file_name().unwrap_or_default());
                copy_texture(source,&alias)?;
                metadata_files.insert(alias.strip_prefix(&workspace_root).unwrap_or(&alias).to_string_lossy().replace('\\',"/"));
            }
        }
        if layer_one.is_none() {
            warnings.push(format!("No legacy armor layer-1 texture was found for {}; armorTextureFile uses '{}'.",set.prefix,layer_prefix));
        }

        let element_name=unique_element_name(&set.prefix,&mut used_names);
        let stats=None;
        let (element,mut builder_warnings)=build_armor_element(
            Some(&template),&set.prefix,&set.registry_name,&set.fields,&piece_textures,&layer_prefix,stats,imported_fv
        );
        warnings.append(&mut builder_warnings);
        let path=output_elements.join(format!("{element_name}.mod.json"));
        fs::write(&path,format!("{}\n",serde_json::to_string_pretty(&element)?))
            .with_context(||format!("Writing armor element {}",path.display()))?;
        let descriptor=json!({
            "name":element_name,"type":"armor","compiles":true,"locked_code":false,
            "registry_name":set.registry_name,
            "metadata":{"files":metadata_files.iter().cloned().collect::<Vec<_>>()},
            "path":format!("~/{}/Armor",source_modid.replace('-',"_")),
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":source_modid,
            "mojmodelbridge_source_prefix":set.prefix,
            "mojmodelbridge_source_paths":set.source_paths
        });
        workspace["mod_elements"].as_array_mut().expect("workspace has mod_elements").push(descriptor);
        update_language(&mut workspace,&target_modid,&set.registry_name,&armor_display_name(&set.prefix));
        report.exported += 1;
        report.records.push(ArmorExportRecord {
            prefix:set.prefix,element_name,registry_name:set.registry_name,piece_textures,
            armor_texture_prefix:layer_prefix,source_paths:set.source_paths,warnings
        });
    }

    fs::write(&workspace_file,format!("{}\n",serde_json::to_string_pretty(&workspace)?))
        .with_context(||format!("Writing MCreator workspace {}",workspace_file.display()))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn groups_legacy_armor_fields_into_one_set() {
        let source = r#"
            public class ModItems {
                public static ItemArmor rubyHelmet;
                public static ItemArmor rubyChestplate;
                public static ItemArmor rubyLegs;
                public static ItemArmor rubyBoots;
            }
        "#;
        let groups=group_armor_fields(source);
        assert_eq!(groups.len(),1);
        assert_eq!(groups["ruby"]["helmet"],"rubyHelmet");
        assert_eq!(groups["ruby"]["body"],"rubyChestplate");
        assert_eq!(groups["ruby"]["leggings"],"rubyLegs");
        assert_eq!(groups["ruby"]["boots"],"rubyBoots");
    }

    #[test]
    fn groups_nested_armor_set_container() {
        let groups=group_armor_fields("public ArmorSet crystalSet;");
        assert_eq!(groups["crystal"].len(),4);
        assert_eq!(groups["crystal"]["body"],"crystal_chestplate");
    }

    #[test]
    fn builds_mcreator_armor_element_with_part_toggles_and_default_stats() {
        let fields=BTreeMap::from([
            ("helmet".to_owned(),"rubyHelmet".to_owned()),
            ("body".to_owned(),"rubyChestplate".to_owned()),
            ("leggings".to_owned(),"rubyLegs".to_owned()),
            ("boots".to_owned(),"rubyBoots".to_owned())
        ]);
        let textures=BTreeMap::from([
            ("helmet".to_owned(),"ruby_helmet".to_owned()),
            ("body".to_owned(),"ruby_chestplate".to_owned()),
            ("leggings".to_owned(),"ruby_legs".to_owned()),
            ("boots".to_owned(),"ruby_boots".to_owned())
        ]);
        let (element,warnings)=build_armor_element(None,"ruby","ruby_armor",&fields,&textures,"ruby",None,89);
        assert_eq!(element["_type"],"armor");
        assert_eq!(element["definition"]["enableHelmet"],true);
        assert_eq!(element["definition"]["enableBody"],true);
        assert_eq!(element["definition"]["enableLeggings"],true);
        assert_eq!(element["definition"]["enableBoots"],true);
        assert_eq!(element["definition"]["armorTextureFile"],"ruby");
        assert_eq!(element["definition"]["maxDamage"],25);
        assert_eq!(element["definition"]["damageValueBody"],8);
        assert_eq!(element["definition"]["textureHelmet"],"ruby_helmet");
        assert!(warnings.iter().any(|w|w.contains("conservative defaults")));
    }

    #[test]
    fn exports_armor_set_icons_and_layer_textures_into_workspace() {
        let temp=tempdir().unwrap();
        let source=temp.path().join("source");
        let workspace=temp.path().join("workspace");
        for directory in [
            source.join("assets/demo/textures/item"),
            source.join("assets/demo/textures/models/armor")
        ] { fs::create_dir_all(directory).unwrap(); }
        let fields=[
            ("rubyHelmet","registerItem(ModItems.rubyHelmet, \"ruby_helmet\");"),
            ("rubyChestplate","registerItem(ModItems.rubyChestplate, \"ruby_chestplate\");"),
            ("rubyLegs","registerItem(ModItems.rubyLegs, \"ruby_leggings\");"),
            ("rubyBoots","registerItem(ModItems.rubyBoots, \"ruby_boots\");")
        ];
        let declarations=fields.iter().map(|(field,_)|format!("public static ItemArmor {field};")).collect::<Vec<_>>().join("\n");
        let calls=fields.iter().map(|(_,call)|call.to_string()).collect::<Vec<_>>().join("\n");
        fs::write(source.join("ModItems.java"),format!("public class ModItems {{ {declarations} public static void register() {{ {calls} }} }}")).unwrap();

        for name in ["ruby_helmet","ruby_chestplate","ruby_leggings","ruby_boots"] {
            fs::write(source.join(format!("assets/demo/textures/item/{name}.png")),b"png").unwrap();
        }
        fs::write(source.join("assets/demo/textures/models/armor/ruby_layer_1.png"),b"layer1").unwrap();
        fs::write(source.join("assets/demo/textures/models/armor/ruby_layer_2.png"),b"layer2").unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("test.mcreator"),serde_json::to_vec(&json!({
            "mcreatorVersion":202600,"workspaceSettings":{"modid":"converted"},"mod_elements":[]
        })).unwrap()).unwrap();

        let report=export_armor_to_workspace(&[source],&workspace,"demo",89).unwrap();
        assert_eq!(report.discovered,1);
        assert_eq!(report.exported,1);
        let element_path=workspace.join("elements/RubyArmor.mod.json");
        let element:Value=serde_json::from_slice(&fs::read(element_path).unwrap()).unwrap();
        assert_eq!(element["_type"],"armor");
        assert_eq!(element["definition"]["enableHelmet"],true);
        assert_eq!(element["definition"]["textureBoots"],"ruby_boots");
        assert!(workspace.join("src/main/resources/assets/converted/textures/models/armor/ruby_layer_1.png").is_file());
        let updated:Value=serde_json::from_slice(&fs::read(workspace.join("test.mcreator")).unwrap()).unwrap();
        assert_eq!(updated["mod_elements"][0]["registry_name"],"ruby_armor");
        assert_eq!(updated["mod_elements"][0]["type"],"armor");
    }

    #[test]
    fn does_not_cross_assign_boot_texture_to_helmet() {
        let temp=tempdir().unwrap();
        let source=temp.path().join("source");
        let root=source.join("assets/demo/textures/item");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ruby_boots.png"),b"boots").unwrap();
        let candidates=texture_matches(&[source],&[
            "ruby_helmet".to_owned(),"RubyHelmet".to_owned()
        ],"helmet");
        assert!(candidates.is_empty());
    }
}
