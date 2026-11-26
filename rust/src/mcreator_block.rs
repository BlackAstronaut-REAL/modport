use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct BlockSource {
    pub class_name: String,
    pub superclass: String,
    pub registry_name: String,
    pub source_path: String,
    #[serde(skip)]
    pub source_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockExportRecord {
    pub class_name: String,
    pub element_name: String,
    pub registry_name: String,
    pub render_type: i64,
    pub textures: BTreeMap<String, String>,
    pub source_resources: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockWorkspaceExportReport {
    pub workspace_file: PathBuf,
    pub discovered: usize,
    pub exported: usize,
    pub skipped: usize,
    pub records: Vec<BlockExportRecord>,
    pub warnings: Vec<String>,
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
    if out.is_empty() { "converted_block".to_owned() } else { out }
}

fn pascal(value: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() {
            upper = true;
        } else if upper {
            out.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() { out = "ConvertedBlock".to_owned(); }
    if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out.insert_str(0, "Block");
    }
    out
}

fn normalize_registry(value: &str) -> String {
    let mut value = value.trim().trim_matches('"').trim_matches('\'').to_owned();
    if let Some((_, path)) = value.split_once(':') {
        value = path.to_owned();
    }
    value.trim_matches('/').replace('/', "_").replace('-', "_").to_ascii_lowercase()
}

fn compact(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn files_java(roots: &[PathBuf]) -> Vec<PathBuf> {
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

fn class_headers(source: &str) -> Vec<(String, String)> {
    let regex = Regex::new(
        r"\b(?:(?:public|protected|private|abstract|final|static)\s+)*class\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>{}]*>)?\s+extends\s+([A-Za-z_$][A-Za-z0-9_$.]*)"
    ).unwrap();
    regex.captures_iter(source).filter_map(|capture| {
        Some((capture.get(1)?.as_str().to_owned(), simple_name(capture.get(2)?.as_str())))
    }).collect()
}

fn constructor_assignments(source: &str) -> Vec<(String, String)> {
    let regex = Regex::new(
        r"\b([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:(?:\([A-Za-z_$][\w.$<>?, \[\]]*\)\s*)|\(*\s*)new\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\("
    ).unwrap();
    regex.captures_iter(source).filter_map(|capture| {
        Some((capture.get(1)?.as_str().to_owned(), simple_name(capture.get(2)?.as_str())))
    }).collect()
}

fn java_registration_calls(source: &str, wanted: &[&str]) -> Vec<(String, String)> {
    use crate::source::java::extract_balanced;
    let source = strip_comments(source);
    let bytes = source.as_bytes();
    let mut found = Vec::new();
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
            if !wanted.contains(&name) { continue; }
            let mut open = index;
            while open < bytes.len() && bytes[open].is_ascii_whitespace() { open += 1; }
            if open >= bytes.len() || bytes[open] != b'(' { continue; }
            if let Ok((arguments, end)) = extract_balanced(&source, open, '(', ')') {
                found.push((name.to_owned(), arguments));
                index = end;
            }
            continue;
        }
        index += 1;
    }
    found
}

fn quoted_values(source: &str) -> Vec<String> {
    let regex = Regex::new(r#"\"([^\"\\]*(?:\\.[^\"\\]*)*)\""#).unwrap();
    regex.captures_iter(source)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
        .collect()
}

fn field_registry_map(source: &str) -> BTreeMap<String, String> {
    use crate::source::java::split_top_level;
    let mut result = BTreeMap::new();
    for (_call, raw_args) in java_registration_calls(source, &["registerBlock", "registerItem"]) {
        let args = split_top_level(&raw_args, ',');
        if args.len() < 2 { continue; }
        let field = args[0].trim().rsplit('.').next().unwrap_or("").trim();
        let values = quoted_values(&args[1]);
        let Some(raw_registry) = values.last() else { continue };
        let registry = normalize_registry(raw_registry);
        if !field.is_empty() && !registry.is_empty() {
            result.insert(field.to_owned(), registry);
        }
    }
    result
}

fn modern_block_registrations(source: &str) -> Vec<(String, String)> {
    let regex = Regex::new(
        r#"(?s)\b(?:BLOCKS|BLOCK_REGISTRY)\s*\.\s*register\s*\(\s*"([^"]+)"\s*,\s*([^;]{0,1600})"#
    ).unwrap();
    let constructor = Regex::new(r"\bnew\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\(").unwrap();
    let factory = Regex::new(r"\b([A-Za-z_$][A-Za-z0-9_$.]*)\s*::\s*new\b").unwrap();
    let mut result = Vec::new();
    for capture in regex.captures_iter(source) {
        let registry = normalize_registry(capture.get(1).unwrap().as_str());
        let body = capture.get(2).map(|value| value.as_str()).unwrap_or("");
        if registry.is_empty() { continue; }
        let mut classes = BTreeSet::new();
        classes.extend(constructor.captures_iter(body).filter_map(|c| c.get(1).map(|v| simple_name(v.as_str()))));
        classes.extend(factory.captures_iter(body).filter_map(|c| c.get(1).map(|v| simple_name(v.as_str()))));
        for class in classes {
            result.push((class, registry.clone()));
        }
    }
    result
}

pub fn is_plant_source(source: &BlockSource) -> bool {
    let superclass = source.superclass.to_ascii_lowercase();
    let class = source.class_name.to_ascii_lowercase();
    [
        "blockflower", "blockbush", "blockcrops", "blocksapling",
        "blocktallgrass", "blockplant", "blockreed", "flowerblock",
        "saplingblock", "cropblock", "bushblock", "tallgrassblock",
    ].iter().any(|token| superclass.contains(token))
        || ["plant", "sapling", "flower", "bush", "crop", "reed", "tallgrass"]
            .iter().any(|token| class.contains(token))
}

fn is_block_class(superclass: &str, parents: &BTreeMap<String, String>, depth: usize) -> bool {
    if depth > 12 { return false; }
    let parent = simple_name(superclass);
    let lower = parent.to_ascii_lowercase();

    // Legacy Forge mods frequently introduce intermediate bases named
    // BlockBase, BlockOre, BlockFluid, BlockContainer, etc. They do not end
    // with "Block", so the former suffix-only check silently discarded every
    // registered subclass that extended one of these bases.
    if lower.starts_with("itemblock")
        || lower.contains("blockentity")
        || lower.contains("blockstate")
        || lower.contains("blockpos")
        || lower.contains("blockbehaviour")
        || lower.contains("blockbehavior")
    {
        return false;
    }
    if lower == "block"
        || lower.starts_with("block")
        || lower.ends_with("block")
        || ["blockflower","blockbush","blockcrops","blocksapling","blocktallgrass","blockplant","blockreed","flowerblock","saplingblock","cropblock","bushblock","tallgrassblock"].iter().any(|token| lower.contains(token))
    {
        return true;
    }
    parents.get(&parent).is_some_and(|next| is_block_class(next, parents, depth + 1))
}

/// Resolve a Block constructor or registered static field from one argument
/// of a legacy registerBlock call. Some mods register inline instances, while
/// others wrap them in helper expressions or pass name/instance in reverse order.
fn registered_block_class(expression: &str, field_classes: &BTreeMap<String, String>) -> Option<String> {
    let constructor = Regex::new(r"\\bnew\\s+([A-Za-z_$][A-Za-z0-9_$.]*)\\s*\\(").ok()?;
    if let Some(class) = constructor.captures(expression)
        .and_then(|capture| capture.get(1))
        .map(|value| simple_name(value.as_str()))
    {
        return Some(class);
    }

    let class_literal = Regex::new(r"\\b([A-Za-z_$][A-Za-z0-9_$.]*)\\s*\\.\\s*class\\b").ok()?;
    if let Some(class) = class_literal.captures(expression)
        .and_then(|capture| capture.get(1))
        .map(|value| simple_name(value.as_str()))
    {
        return Some(class);
    }

    let field = expression.trim().trim_end_matches(';').rsplit('.').next()?.trim();
    field_classes.get(field).cloned()
}

/// Recover registrations whose block instance is passed directly rather than
/// first being assigned to a static field. The normal field map remains useful
/// for legacy drop recovery, so this augments rather than replaces it.
fn legacy_block_registrations(
    source: &str,
    field_classes: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    use crate::source::java::split_top_level;
    let mut output = Vec::new();
    for (_, raw_arguments) in java_registration_calls(source, &["registerBlock"]) {
        let arguments = split_top_level(&raw_arguments, ',');
        let registry = arguments.iter()
            .flat_map(|argument| quoted_values(argument))
            .find_map(|value| {
                let normalized = normalize_registry(&value);
                (!normalized.is_empty()).then_some(normalized)
            });
        let Some(registry) = registry else { continue };
        let class_name = arguments.iter()
            .find_map(|argument| registered_block_class(argument, field_classes));
        if let Some(class_name) = class_name {
            output.push((class_name, registry));
        }
    }
    output
}

/// Discover registered Java Block subclasses from legacy GameRegistry and modern BLOCKS.register calls.
pub fn discover_block_sources(source_roots: &[PathBuf]) -> Result<Vec<BlockSource>> {/// Discover registered Java Block subclasses from legacy GameRegistry and modern BLOCKS.register calls.
pub fn discover_block_sources(source_roots: &[PathBuf]) -> Result<Vec<BlockSource>> {
    let files = files_java(source_roots);
    let mut texts = Vec::<String>::new();
    let mut class_texts = BTreeMap::<String, String>::new();
    let mut class_paths = BTreeMap::<String, PathBuf>::new();
    let mut parents = BTreeMap::<String, String>::new();
    let mut field_classes = BTreeMap::<String, String>::new();

    for path in &files {
        let Ok(bytes) = fs::read(path) else { continue };
        let text = strip_comments(&String::from_utf8_lossy(&bytes));
        for (class, parent) in class_headers(&text) {
            class_texts.entry(class.clone()).or_insert_with(|| text.clone());
            class_paths.entry(class.clone()).or_insert_with(|| path.clone());
            parents.insert(class, parent);
        }
        for (field, class) in constructor_assignments(&text) {
            field_classes.insert(field, class);
        }
        texts.push(text);
    }

    let combined = texts.join("\n");
    let compiled = crate::compiled_registry::scan_compiled_registrations(source_roots)?;
    for (class, info) in &compiled.classes {
        class_paths.entry(class.clone()).or_insert_with(|| info.path.clone());
        parents.entry(class.clone()).or_insert_with(|| info.superclass.clone());
    }
    let field_ids = field_registry_map(&combined);
    let mut class_ids = BTreeMap::<String, BTreeSet<String>>::new();

    for (field, class) in &field_classes {
        if let Some(registry) = field_ids.get(field) {
            class_ids.entry(class.clone()).or_default().insert(registry.clone());
        }
    }
    for (class, registry) in legacy_block_registrations(&combined, &field_classes) {
        class_ids.entry(class).or_default().insert(registry);
    }
    for (class, registry) in modern_block_registrations(&combined) {
        class_ids.entry(class).or_default().insert(registry);
    }
    for (class, registries) in &compiled.registries {
        class_ids.entry(class.clone()).or_default().extend(registries.iter().cloned());
    }

    let mut output = Vec::new();
    for (class, registries) in class_ids {
        let Some(superclass) = parents.get(&class) else { continue };
        if !is_block_class(superclass, &parents, 0) { continue; }
        let source_text = class_texts.get(&class).cloned().unwrap_or_default();
        let Some(source_path) = class_paths.get(&class) else { continue };
        for registry in registries {
            let source = BlockSource {
                class_name:class.clone(),
                superclass:superclass.clone(),
                registry_name:registry,
                source_path:source_path.to_string_lossy().replace('\\', "/"),
                source_text:source_text.clone(),
            };
            output.push(source);
        }
    }
    output.sort_by(|a,b| a.registry_name.cmp(&b.registry_name).then(a.class_name.cmp(&b.class_name)));
    Ok(output)
}

fn definition_template(root: &Path) -> Result<Value> {
    let elements = root.join("elements");
    if elements.is_dir() {
        let mut candidates: Vec<PathBuf> = fs::read_dir(elements)?.filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect();
        candidates.sort();
        for path in candidates {
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
            if value.get("_type").and_then(Value::as_str) == Some("block")
                && value.get("definition").is_some_and(Value::is_object) {
                return Ok(value);
            }
        }
    }
    serde_json::from_str(include_str!("../assets/mcreator_templates/BlockSchema.mod.json"))
        .context("Parsing embedded MCreator block schema")
}

fn workspace_file(root: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::<(i64,usize,PathBuf)>::new();
    for entry in fs::read_dir(root)? {
        let path=entry?.path();
        if path.extension().and_then(|value|value.to_str()) != Some("mcreator") { continue; }
        let Ok(bytes)=fs::read(&path) else {continue};
        let Ok(value)=serde_json::from_slice::<Value>(&bytes) else {continue};
        if !value.get("workspaceSettings").is_some_and(Value::is_object)
            || !value.get("mod_elements").is_some_and(Value::is_array) { continue; }
        candidates.push((
            value.get("mcreatorVersion").and_then(Value::as_i64).unwrap_or(0),
            value.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            path
        ));
    }
    candidates.sort_by_key(|(version,count,path)|(*version,*count,path.to_string_lossy().to_ascii_lowercase()));
    candidates.pop().map(|(_,_,path)|path)
        .ok_or_else(||anyhow::anyhow!("No valid .mcreator workspace file in {}",root.display()))
}

fn number_after(source: &str, patterns: &[&str]) -> Option<f64> {
    for pattern in patterns {
        let regex=Regex::new(pattern).ok()?;
        if let Some(capture)=regex.captures(source) {
            if let Some(value)=capture.get(1).and_then(|m|m.as_str().parse::<f64>().ok()) {
                if value.is_finite() {return Some(value);}
            }
        }
    }
    None
}

fn block_sound_type(source_text: &str) -> Option<&'static str> {
    if source_text.contains("woodSoundType") {
        return Some("WOOD");
    }
    let pattern = Regex::new(
        r"(?:net\.minecraft\.world\.level\.block\.)?SoundType\.([A-Za-z0-9_]+)",
    ).ok()?;
    let raw = pattern.captures(source_text)?.get(1)?.as_str().to_ascii_uppercase();
    Some(match raw.as_str() {
        "STONE" => "STONE",
        "GRASS" | "WET_GRASS" => "GRASS",
        "WOOD" => "WOOD",
        "LEAVES" => "LEAVES",
        "GLASS" => "GLASS",
        "METAL" => "METAL",
        "PISTON" => "PISTON",
        "CLOTH" => "WOOL",
        "SAND" => "SAND",
        "SNOW" => "SNOW",
        "LADDER" => "LADDER",
        "ANVIL" => "ANVIL",
        "SLIME_BLOCK" => "SLIME",
        "HONEY_BLOCK" => "HONEY",
        "NETHERRACK" => "NETHERRACK",
        "NETHER_BRICKS" => "NETHER_BRICKS",
        "BONE_BLOCK" => "BONE",
        "NETHERITE_BLOCK" => "NETHERITE",
        "BAMBOO" | "BAMBOO_SAPLING" => "BAMBOO",
        "CROP" | "FLOWERING_AZALEA" | "FUNGUS" => "PLANT",
        "ROOTED_DIRT" => "GROUND",
        "SCULK" => "SCULK",
        "CANDLE" => "CANDLE",
        "AMETHYST" => "AMETHYST",
        "COPPER" => "COPPER",
        "POWDER_SNOW" => "POWDER_SNOW",
        _ => "STONE",
    })
}



fn compact_sound_token(value: &str) -> String {
    value.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

/// Port of the old exporter sound inventory scoring. MCreator workspaces
/// store imported sound events as sound_elements; retain each exact element
/// name so the target schema can bind it without inventing a resource ID.
fn infer_block_sound_event(source_text: &str, registry: &str, workspace: &Value) -> Option<String> {
    let inventory = workspace.get("sound_elements").and_then(Value::as_array)?;
    let source_compact = compact_sound_token(source_text);
    let registry_compact = compact_sound_token(registry);
    let role_terms = ["step", "walk", "foot", "stone", "grass", "wood", "metal", "block"];
    let mut best: Option<(i32, String)> = None;

    for item in inventory {
        let Some(name) = item.get("name").and_then(Value::as_str)
            .map(str::trim).filter(|name| !name.is_empty()) else { continue };
        let candidate_compact = compact_sound_token(name);
        let candidate_lower = name.to_ascii_lowercase();
        let mut score = 0;

        if !registry_compact.is_empty()
            && (candidate_compact.contains(&registry_compact)
                || registry_compact.contains(&candidate_compact))
        {
            score += 8;
        }
        if role_terms.iter().any(|term| source_compact.contains(term)) {
            score += 5;
        }
        if role_terms.iter().any(|term| candidate_lower.contains(term)) {
            score += 4;
        }
        let shared_characters = candidate_compact.chars().collect::<BTreeSet<_>>()
            .into_iter().filter(|ch| source_compact.contains(*ch)).count();
        score += shared_characters.min(5) as i32;

        if best.as_ref().is_none_or(|(best_score, _)| score > *best_score) {
            best = Some((score, name.to_owned()));
        }
    }
    best.map(|(_, name)| name)
}

fn apply_block_sound_bindings(
    definition: &mut Map<String, Value>,
    source_text: &str,
    registry: &str,
    workspace: &Value,
) {
    let has_inventory = workspace.get("sound_elements").and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty());
    if !has_inventory {
        return;
    }

    let sound = block_sound_type(source_text)
        .map(str::to_owned)
        .or_else(|| infer_block_sound_event(source_text, registry, workspace))
        .unwrap_or_else(|| "STONE".to_owned());
    definition.insert("soundOnStep".to_owned(), json!(sound));
    definition.insert("isCustomSoundType".to_owned(), json!(false));
    for key in ["breakSound", "fallSound", "hitSound", "placeSound", "stepSound"] {
        definition.insert(key.to_owned(), json!(""));
    }
}


/// Recover the statically identifiable ItemStack emitted by a block's drop
/// routine. Dynamic loot tables remain untouched instead of guessing an item.
fn extract_block_drop(
    source_text: &str,
    field_registry_map: &BTreeMap<String, BTreeSet<String>>,
) -> Option<(String, i64)> {
    if source_text.contains("Collections.emptyList") {
        return Some((String::new(), 0));
    }

    // Balance the constructor arguments instead of relying on a fragile regex:
    // ItemStack references in old Java sources often contain qualified fields
    // or nested count expressions.
    let constructor = Regex::new(r"\bnew\s+ItemStack\s*\(").ok()?;
    let found = constructor.find(source_text)?;
    let open = source_text[found.start()..found.end()].rfind('(')? + found.start();
    let (raw_arguments, _) =
        crate::source::java::extract_balanced(source_text, open, '(', ')').ok()?;
    let arguments = crate::source::java::split_top_level(&raw_arguments, ',');
    let raw_reference = arguments.first()?.trim();
    if raw_reference.is_empty() {
        return None;
    }
    let count = match arguments.get(1) {
        Some(value) => value.trim().parse::<i64>().ok()?.max(0),
        None => 1,
    };

    let field = raw_reference.rsplit('.').next().unwrap_or(raw_reference);
    if field.eq_ignore_ascii_case("this") || field.eq_ignore_ascii_case("self") {
        return Some((String::new(), count));
    }

    // Keep vanilla references canonical. A Block used as an ItemStack drop
    // maps to MCreator's item token with the same registry path.
    if let Some((owner, name)) = raw_reference.split_once('.') {
        if owner.eq_ignore_ascii_case("Items") || owner.eq_ignore_ascii_case("Blocks") {
            return Some((format!("Items.{name}"), count));
        }
    }

    let registries = field_registry_map.iter()
        .find(|(registered_field, _)| registered_field.eq_ignore_ascii_case(field))
        .map(|(_, registries)| registries)?;
    let registry = registries.iter().next()?;
    let token = registry.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_uppercase() } else { '_' })
        .collect::<String>();
    Some((format!("Items.{token}"), count))
}

fn apply_block_drop_updates(
    definition: &mut Map<String, Value>,
    source_text: &str,
    field_registry_map: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    let Some((drop, amount)) = extract_block_drop(source_text, field_registry_map) else {
        return false;
    };
    definition.insert("customDrop".to_owned(), json!(drop));
    definition.insert("dropAmount".to_owned(), json!(amount));
    definition.insert("useLootTableForDrops".to_owned(), json!(false));
    true
}

/// Extract source-authored block settings for the desktop content editor.
/// These values mirror the fields used by the native MCreator block builder,
/// but remain separate metadata so the browser does not present them as lore.
pub fn source_block_properties(source: &BlockSource) -> Map<String, Value> {
    let text = &source.source_text;
    let mut properties = Map::new();

    let hardness = number_after(text, &[
        r"\.strength\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
        r"\.hardness\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
        r"\bsetHardness\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
    ]).unwrap_or(1.0).max(0.0);
    let resistance = number_after(text, &[
        r"\.strength\s*\(\s*-?\d+(?:\.\d+)?[fFdD]?\s*,\s*(-?\d+(?:\.\d+)?)[fFdD]?",
        r"\.resistance\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
        r"\bsetResistance\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
    ]).unwrap_or(1.0).max(0.0);
    let light = number_after(text, &[
        r"\.lightLevel\s*\(\s*\(\s*\)\s*->\s*(?:\(\s*)?(-?\d+)",
        r"\bsetLightLevel\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
    ]).unwrap_or(0.0).max(0.0);

    properties.insert("hardness".to_owned(), json!(hardness));
    properties.insert("resistance".to_owned(), json!(resistance));
    properties.insert("light_level".to_owned(), json!(light));
    let harvest_level = number_after(text, &[
        r#"(?i)\bsetHarvestLevel\s*\(\s*["'][^"']+["']\s*,\s*(\d+)"#,
        r#"(?i)\bharvestLevel\s*\(\s*(\d+)"#,
    ]).unwrap_or(0.0).clamp(0.0, 10.0) as u32;
    properties.insert("harvest_level".to_owned(), json!(harvest_level));
    properties.insert(
        "has_gravity".to_owned(),
        json!(
            text.contains("SandBlock")
                || text.contains("FallingBlock")
                || source.superclass.contains("FallingBlock")
                || source.class_name.contains("Sand")
        ),
    );
    properties.insert(
        "waterloggable".to_owned(),
        json!(text.contains("waterlogged") || text.contains("WATERLOGGED")),
    );
    properties.insert("has_block_item".to_owned(), json!(true));
    properties.insert("max_stack_size".to_owned(), json!(64));
    properties.insert(
        "requires_correct_tool".to_owned(),
        json!(text.contains("setHarvestLevel") || text.contains("requiresCorrectToolForDrops")),
    );
    properties.insert(
        "random_ticks".to_owned(),
        json!(text.contains("randomTicks") || text.contains("tickRandomly")),
    );
    properties.insert(
        "replaceable".to_owned(),
        json!(text.contains("replaceable") || text.contains("REPLACEABLE")),
    );
    if let Some(value) = number_after(text, &[r"\bsetLightOpacity\s*\(\s*(\d+)\s*\)"]) {
        properties.insert("light_opacity".to_owned(), json!(value as i64));
    }

    if let Some(sound) = block_sound_type(text) {
        properties.insert("step_sound".to_owned(), json!(sound));
    }

    let destroy_tool = if text.contains("BlockTags.MINEABLE_WITH_PICKAXE") || text.contains("pickaxe") {
        Some("Pickaxe")
    } else if text.contains("BlockTags.MINEABLE_WITH_AXE") || text.contains("axe") {
        Some("Axe")
    } else if text.contains("BlockTags.MINEABLE_WITH_SHOVEL") || text.contains("shovel") {
        Some("Shovel")
    } else {
        None
    };
    if let Some(tool) = destroy_tool {
        properties.insert("destroy_tool".to_owned(), json!(tool));
    }
    if let Some((drop, amount)) = extract_block_drop(text, &BTreeMap::new()) {
        properties.insert("custom_drop".to_owned(), json!(drop));
        properties.insert("drop_amount".to_owned(), json!(amount));
        properties.insert("use_loot_table_for_drops".to_owned(), json!(false));
    }
    properties
}

fn block_display(registry: &str) -> String {
    registry.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars=word.chars();
            match chars.next() {
                Some(first) => {
                    let mut out=first.to_ascii_uppercase().to_string();
                    out.extend(chars.map(|ch|ch.to_ascii_lowercase()));
                    out
                },
                None => String::new()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn placeholder_png() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+iZ1sAAAAASUVORK5CYII="
    ).expect("embedded PNG is valid")
}

/// Build a native MCreator block element from recovered source properties and model-derived fields.
pub fn build_block_element(
    template: &Value,
    source: &BlockSource,
    model_info: &Value,
    texture_fields: &BTreeMap<String,String>,
    imported_fv: i64,
    field_registry_map: &BTreeMap<String, BTreeSet<String>>,
) -> (Value, Vec<String>) {
    let mut definition=template.get("definition").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut warnings=Vec::new();
    let registry=&source.registry_name;

    definition.insert("name".to_owned(),json!(block_display(registry)));
    definition.insert("creativeTabs".to_owned(),json!(["BUILDING_BLOCKS"]));
    definition.insert("hardness".to_owned(),json!(number_after(&source.source_text,&[
        r"\.strength\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
        r"\.hardness\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
        r"\bsetHardness\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?"
    ]).unwrap_or(1.0).max(0.0)));
    definition.insert("resistance".to_owned(),json!(number_after(&source.source_text,&[
        r"\.strength\s*\(\s*-?\d+(?:\.\d+)?[fFdD]?\s*,\s*(-?\d+(?:\.\d+)?)[fFdD]?",
        r"\.resistance\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?",
        r"\bsetResistance\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?"
    ]).unwrap_or(1.0).max(0.0)));
    let light=number_after(&source.source_text,&[
        r"\.lightLevel\s*\(\s*\(\s*\)\s*->\s*(?:\(\s*)?(-?\d+)",
        r"\bsetLightLevel\s*\(\s*(-?\d+(?:\.\d+)?)[fFdD]?"
    ]).unwrap_or(0.0);
    definition.insert("luminance".to_owned(),json!({"fixedValue":light.max(0.0)}));
    definition.insert("hasGravity".to_owned(),json!(source.source_text.contains("SandBlock") || source.source_text.contains("FallingBlock")));
    definition.insert("isWaterloggable".to_owned(),json!(source.source_text.contains("waterlogged") || source.source_text.contains("WATERLOGGED")));
    definition.insert("hasBlockItem".to_owned(),json!(true));
    definition.insert("maxStackSize".to_owned(),json!(64));
    definition.insert("rarity".to_owned(),json!("COMMON"));

    for key in ["renderType","customModelName","texture","textureTop","textureBottom","textureLeft","textureFront","textureRight","textureBack"] {
        if let Some(value)=model_info.get(key) {
            definition.insert(key.to_owned(),value.clone());
        }
    }
    for (key,value) in texture_fields {
        definition.insert(key.clone(),json!(value));
    }
    if let Some(texture)=texture_fields.get("texture") {
        definition.insert("itemTexture".to_owned(),json!(texture));
        definition.insert("particleTexture".to_owned(),json!(texture));
    }

    if source.source_text.contains("setLightOpacity") {
        if let Some(value)=number_after(&source.source_text,&[r"\bsetLightOpacity\s*\(\s*(\d+)\s*\)"]) {
            definition.insert("lightOpacity".to_owned(),json!(value as i64));
        }
    }
    if source.source_text.contains("setHarvestLevel") || source.source_text.contains("requiresCorrectToolForDrops") {
        definition.insert("requiresCorrectTool".to_owned(),json!(true));
    }
    if source.source_text.contains("BlockTags.MINEABLE_WITH_PICKAXE") || source.source_text.contains("pickaxe") {
        definition.insert("destroyTool".to_owned(),json!("Pickaxe"));
    }
    if source.source_text.contains("BlockTags.MINEABLE_WITH_AXE") || source.source_text.contains("axe") {
        definition.insert("destroyTool".to_owned(),json!("Axe"));
    }
    if source.source_text.contains("BlockTags.MINEABLE_WITH_SHOVEL") || source.source_text.contains("shovel") {
        definition.insert("destroyTool".to_owned(),json!("Shovel"));
    }
    if source.source_text.contains("randomTicks") || source.source_text.contains("tickRandomly") {
        definition.insert("tickRandomly".to_owned(),json!(true));
    }
    if source.source_text.contains("replaceable") || source.source_text.contains("REPLACEABLE") {
        definition.insert("isReplaceable".to_owned(),json!(true));
    }
    if model_info.as_object().is_none_or(Map::is_empty) {
        warnings.push("No block model/blockstate JSON could be resolved; template render defaults were retained.".to_owned());
    }
    let has_static_drop = apply_block_drop_updates(&mut definition, &source.source_text, field_registry_map);
    if !has_static_drop && (source.source_text.contains("getDrops") || source.source_text.contains("getDrop")) {
        warnings.push("Custom block drop behavior was detected but could not be reduced to a static ItemStack; its loot behavior needs manual reconstruction.".to_owned());
    }

    let fv=template.get("_fv").and_then(Value::as_i64).unwrap_or(imported_fv).clamp(0,89);
    (json!({"_fv":fv,"_type":"block","definition":definition}),warnings)
}

/// Export registered Java block classes with blockstate/model JSON and texture resources.
pub fn export_blocks_to_workspace(
    source_roots:&[PathBuf],
    workspace_root:&Path,
    source_modid:&str,
    imported_fv:i64
)->Result<BlockWorkspaceExportReport>{
    let workspace_root=fs::canonicalize(workspace_root).unwrap_or_else(|_|workspace_root.to_path_buf());
    let workspace_file=workspace_file(&workspace_root)?;
    let bytes=fs::read(&workspace_file)?;
    let mut workspace:Value=serde_json::from_slice(&bytes)?;
    let target_modid=workspace.get("workspaceSettings").and_then(|v|v.get("modid")).and_then(Value::as_str)
        .filter(|s|!s.trim().is_empty()).unwrap_or("converted_mod").to_owned();
    let discovered: Vec<BlockSource> = discover_block_sources(source_roots)?
        .into_iter()
        .filter(|source| !is_plant_source(source))
        .collect();
    let template=definition_template(&workspace_root)?;
    let field_registry_map = crate::recipe_bridge::source_field_registered_names(source_roots, source_modid);
    let localization = crate::localization::LocalizationCatalog::load(source_roots, &field_registry_map);
    let creative_tabs = crate::localization::CreativeTabCatalog::load(source_roots);

    let mut report=BlockWorkspaceExportReport{
        workspace_file:workspace_file.clone(),discovered:discovered.len(),exported:0,skipped:0,records:Vec::new(),warnings:Vec::new()
    };
    let mut existing=HashSet::<String>::new();
    let mut used=HashSet::<String>::new();
    for item in workspace.get("mod_elements").and_then(Value::as_array).into_iter().flatten(){
        if item.get("type").and_then(Value::as_str)==Some("block"){
            existing.insert(item.get("registry_name").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase());
        }
        if let Some(name)=item.get("name").and_then(Value::as_str){used.insert(name.to_ascii_lowercase());}
    }

    let texture_root=workspace_root.join("src/main/resources/assets").join(&target_modid).join("textures");
    let output_elements=workspace_root.join("elements");
    fs::create_dir_all(&texture_root)?;
    fs::create_dir_all(&output_elements)?;

    for source in discovered{
        let registry=source.registry_name.to_ascii_lowercase();
        if existing.contains(&registry){report.skipped+=1;continue;}
        let copied=crate::block_model_bridge::copy_source_block_model_resources(source_roots,source_modid,&target_modid,&registry,&workspace_root)?;
        let info=crate::block_model_bridge::mcreator_block_model_info(source_roots,source_modid,&registry);
        let model_path=info.get("modelPath").and_then(Value::as_str).map(Path::new);
        let mut textures=BTreeMap::<String,String>::new();
        for key in ["texture","textureTop","textureBottom","textureLeft","textureFront","textureRight","textureBack"]{
            let raw=info.get(key).and_then(Value::as_str).unwrap_or("");
            if raw.is_empty(){continue;}
            let resolved=crate::block_model_bridge::copy_model_ref_texture_to_workspace(
                model_path,raw,&texture_root,"block",&registry
            )?;
            if !resolved.starts_with("minecraft:") {
                let target=texture_root.join("block").join(format!("{resolved}.png"));
                if !target.is_file() {
                    fs::create_dir_all(target.parent().unwrap())?;
                    fs::write(&target,placeholder_png())?;
                    report.warnings.push(format!("Model texture '{}' for {} could not be copied; generated placeholder '{}'.",raw,registry,target.file_name().unwrap_or_default().to_string_lossy()));
                }
            }
            textures.insert(key.to_owned(),resolved);
        }

        if !textures.contains_key("texture") {
            let raw_stem=registry.clone();
            let target=texture_root.join("block").join(format!("{raw_stem}.png"));
            if !target.exists(){fs::write(&target,placeholder_png())?;}
            textures.insert("texture".to_owned(),raw_stem.clone());
            report.warnings.push(format!("No usable block texture resolved for {}; generated placeholder icon.",registry));
        }

        let name_base=source.class_name.strip_prefix("Block").unwrap_or(&source.class_name);
        let mut element_name=pascal(name_base);
        if element_name.is_empty(){element_name=pascal(&registry);}
        let base=element_name.clone();
        let mut suffix=2;
        while !used.insert(element_name.to_ascii_lowercase()){
            element_name=format!("{base}{suffix}");
            suffix+=1;
        }

        let (mut element,mut warnings)=build_block_element(&template,&source,&info,&textures,imported_fv,&field_registry_map);
        let display = localization.translated_name(&registry, &block_display(&registry));
        if let Some(definition) = element.get_mut("definition").and_then(Value::as_object_mut) {
            apply_block_sound_bindings(definition, &source.source_text, &source.registry_name, &workspace);
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
        if source.source_text.trim().is_empty() {
            warnings.push("Block class and registry ID were recovered from compiled bytecode only; source-defined state properties, hardness, drops, and special behavior may need manual reconstruction.".to_owned());
        }
        let element_path=output_elements.join(format!("{element_name}.mod.json"));
        fs::write(&element_path,format!("{}\n",serde_json::to_string_pretty(&element)?))
            .with_context(||format!("Writing block element {}",element_path.display()))?;
        let mut metadata=BTreeSet::<String>::new();
        for path in &copied{
            metadata.insert(path.strip_prefix(&workspace_root).unwrap_or(path).to_string_lossy().replace('\\',"/"));
        }
        for value in textures.values(){
            if value.starts_with("minecraft:"){continue;}
            let path=texture_root.join("block").join(format!("{value}.png"));
            if path.is_file(){
                metadata.insert(path.strip_prefix(&workspace_root).unwrap_or(&path).to_string_lossy().replace('\\',"/"));
                let sidecar=PathBuf::from(format!("{}.mcmeta",path.display()));
                if sidecar.is_file(){
                    metadata.insert(sidecar.strip_prefix(&workspace_root).unwrap_or(&sidecar).to_string_lossy().replace('\\',"/"));
                }
            }
        }
        let descriptor=json!({
            "name":element_name,"type":"block","compiles":true,"locked_code":false,
            "registry_name":registry,
            "metadata":{"files":metadata.iter().cloned().collect::<Vec<_>>()},
            "path":format!("~/{}/Blocks",source_modid.replace('-',"_")),
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":source_modid,
            "mojmodelbridge_source_class":source.class_name,
            "mojmodelbridge_source_path":source.source_path
        });
        workspace["mod_elements"].as_array_mut().expect("workspace mod_elements array").push(descriptor);
        if !workspace.get("language_map").is_some_and(Value::is_object){workspace["language_map"]=json!({});}
        if !workspace["language_map"].get("en_us").is_some_and(Value::is_object){workspace["language_map"]["en_us"]=json!({});}
        let language=workspace["language_map"]["en_us"].as_object_mut().unwrap();
        language.insert(format!("block.{target_modid}.{registry}"),json!(display));
        language.insert(format!("block.{target_modid}.{registry}.name"),json!(display));
        language.insert(format!("block.{target_modid}.{element_name}"),json!(display));
        language.insert(format!("block.{target_modid}.{element_name}.name"),json!(display));

        report.exported+=1;
        report.warnings.extend(warnings.clone());
        report.records.push(BlockExportRecord{
            class_name:source.class_name,element_name,registry_name:registry,
            render_type:info.get("renderType").and_then(Value::as_i64).unwrap_or(0),
            textures,source_resources:metadata.into_iter().collect(),warnings
        });
        existing.insert(source.registry_name.to_ascii_lowercase());
    }

    fs::write(&workspace_file,format!("{}\n",serde_json::to_string_pretty(&workspace)?))
        .with_context(||format!("Writing workspace descriptor {}",workspace_file.display()))?;
    Ok(report)
}

#[cfg(test)]
mod tests{
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discovers_legacy_and_modern_registered_block_classes(){
        let temp=tempdir().unwrap();
        let root=temp.path();
        fs::write(root.join("Registry.java"),r#"
            public class Registry {
                public static Block DEMO = new DemoBlock();
                public static void register(){ GameRegistry.registerBlock(DEMO,"demo_block"); }
            }
            class DemoBlock extends Block { }
            class ModernBlock extends Block { }
            class ModernRegistry {
                static final RegistryObject<Block> MODERN = BLOCKS.register("modern",()->new ModernBlock());
            }
        "#).unwrap();
        let found=discover_block_sources(&[root.to_path_buf()]).unwrap();
        assert_eq!(found.len(),2);
        assert!(found.iter().any(|b|b.class_name=="DemoBlock"&&b.registry_name=="demo_block"));
        assert!(found.iter().any(|b|b.class_name=="ModernBlock"&&b.registry_name=="modern"));
    }

    #[test]
    fn plant_like_subclasses_are_not_emitted_as_regular_blocks() {
        let plant = BlockSource {
            class_name: "RubyFlower".to_owned(),
            superclass: "BlockFlower".to_owned(),
            registry_name: "ruby_flower".to_owned(),
            source_path: "RubyFlower.java".to_owned(),
            source_text: String::new(),
        };
        let ordinary = BlockSource {
            class_name: "RubyOre".to_owned(),
            superclass: "Block".to_owned(),
            registry_name: "ruby_ore".to_owned(),
            source_path: "RubyOre.java".to_owned(),
            source_text: String::new(),
        };
        assert!(is_plant_source(&plant));
        assert!(!is_plant_source(&ordinary));
    }

    #[test]
    fn excludes_plant_subclasses_from_block_elements() {
        let temp=tempdir().unwrap();
        let root=temp.path();
        fs::write(root.join("Registry.java"),r#"
            public class Registry {
                public static Block FLOWER = new RubyFlower();
                public static void register() {
                    GameRegistry.registerBlock(FLOWER, "ruby_flower");
                }
            }
            class RubyFlower extends BlockFlower { }
        "#).unwrap();
        let found=discover_block_sources(&[root.to_path_buf()]).unwrap();
        assert_eq!(found.len(),1);
        assert!(is_plant_source(&found[0]));
    }

    #[test]
    fn exposes_source_block_properties_as_editable_metadata() {
        let source = BlockSource {
            class_name: "GlowStoneBlock".to_owned(),
            superclass: "FallingBlock".to_owned(),
            registry_name: "glow_stone".to_owned(),
            source_path: "GlowStoneBlock.java".to_owned(),
            source_text: r#"
                new BlockBehaviour.Properties()
                    .strength(3.5F, 12.0F)
                    .lightLevel(() -> 11)
                    .requiresCorrectToolForDrops()
                    .randomTicks()
                    .sound(SoundType.METAL);
                setLightOpacity(4);
                BlockTags.MINEABLE_WITH_PICKAXE;
                waterlogged = true;
            "#.to_owned(),
        };
        let properties = source_block_properties(&source);
        assert_eq!(properties["hardness"], 3.5);
        assert_eq!(properties["resistance"], 12.0);
        assert_eq!(properties["light_level"], 11.0);
        assert_eq!(properties["has_gravity"], true);
        assert_eq!(properties["requires_correct_tool"], true);
        assert_eq!(properties["random_ticks"], true);
        assert_eq!(properties["step_sound"], "METAL");
        assert_eq!(properties["light_opacity"], 4);
        assert_eq!(properties["destroy_tool"], "Pickaxe");
    }

    #[test]
    fn infers_block_properties_and_model_textures_into_template(){
        let source=BlockSource{
            class_name:"DemoBlock".to_owned(),superclass:"Block".to_owned(),registry_name:"demo_block".to_owned(),
            source_path:"DemoBlock.java".to_owned(),
            source_text:"new BlockBehaviour.Properties().strength(2.5F, 6.0F).lightLevel(() -> 12).requiresCorrectToolForDrops();".to_owned()
        };
        let template:Value=serde_json::from_str(include_str!("../assets/mcreator_templates/BlockSchema.mod.json")).unwrap();
        let info=json!({"renderType":11,"customModelName":"Single texture","texture":"demo_block","modelPath":"block.json"});
        let textures=BTreeMap::from([("texture".to_owned(),"demo_block".to_owned())]);
        let (element,warnings)=build_block_element(&template,&source,&info,&textures,89,&BTreeMap::new());
        assert!(warnings.is_empty());
        assert_eq!(element["_type"],"block");
        assert_eq!(element["definition"]["hardness"],2.5);
        assert_eq!(element["definition"]["resistance"],6.0);
        assert_eq!(element["definition"]["luminance"]["fixedValue"],12.0);
        assert_eq!(element["definition"]["renderType"],11);
        assert_eq!(element["definition"]["texture"],"demo_block");
        assert_eq!(element["definition"]["requiresCorrectTool"],true);
    }

    #[test]
    fn infers_block_sound_event_and_normalizes_explicit_sound_types() {
        let workspace = json!({
            "sound_elements": [
                {"name": "ambient_wind"},
                {"name": "ruby_ore_step"},
                {"name": "stone_break"}
            ]
        });
        let mut definition = Map::new();
        for key in ["breakSound", "fallSound", "hitSound", "placeSound", "stepSound"] {
            definition.insert(key.to_owned(), json!("CUSTOM:old"));
        }
        apply_block_sound_bindings(
            &mut definition,
            "Ruby ore block walking",
            "ruby_ore",
            &workspace,
        );
        assert_eq!(definition["soundOnStep"], "ruby_ore_step");
        assert_eq!(definition["isCustomSoundType"], false);
        for key in ["breakSound", "fallSound", "hitSound", "placeSound", "stepSound"] {
            assert_eq!(definition[key], "");
        }

        apply_block_sound_bindings(
            &mut definition,
            ".sound(SoundType.CLOTH)",
            "ruby_ore",
            &workspace,
        );
        assert_eq!(definition["soundOnStep"], "WOOL");
    }

    #[test]
    fn maps_legacy_and_modern_block_sound_types_to_mcreator_values() {
        for (source, expected) in [
            ("SoundType.CLOTH", "WOOL"),
            ("SoundType.SLIME_BLOCK", "SLIME"),
            ("SoundType.BAMBOO_SAPLING", "BAMBOO"),
            ("SoundType.FLOWERING_AZALEA", "PLANT"),
            ("SoundType.ROOTED_DIRT", "GROUND"),
            ("SoundType.WET_GRASS", "GRASS"),
            ("SoundType.POWDER_SNOW", "POWDER_SNOW"),
        ] {
            assert_eq!(block_sound_type(source), Some(expected), "{source}");
        }
        assert_eq!(block_sound_type("woodSoundType"), Some("WOOD"));
        assert_eq!(block_sound_type("SoundType.UNKNOWN"), Some("STONE"));
        assert_eq!(block_sound_type("no sound configured"), None);
    }

    #[test]
    fn recovers_static_custom_block_drops_and_explicit_empty_drops() {
        let item_source = BlockSource {
            class_name: "RubyOre".to_owned(),
            superclass: "Block".to_owned(),
            registry_name: "ruby_ore".to_owned(),
            source_path: "RubyOre.java".to_owned(),
            source_text: "return new ItemStack(RUBY, 3);".to_owned(),
        };
        let registry = BTreeMap::from([(
            "RUBY".to_owned(),
            BTreeSet::from(["ruby_gem".to_owned()]),
        )]);
        let template: Value = serde_json::from_str(include_str!(
            "../assets/mcreator_templates/BlockSchema.mod.json"
        )).unwrap();
        let (element, warnings) = build_block_element(
            &template,
            &item_source,
            &json!({"renderType": 11, "customModelName": "Single texture", "texture": "ruby_ore"}),
            &BTreeMap::new(),
            89,
            &registry,
        );
        assert!(warnings.is_empty());
        assert_eq!(element["definition"]["customDrop"], "Items.RUBY_GEM");
        assert_eq!(element["definition"]["dropAmount"], 3);
        assert_eq!(element["definition"]["useLootTableForDrops"], false);

        let empty_source = BlockSource {
            source_text: "return Collections.emptyList();".to_owned(),
            ..item_source
        };
        let (empty, warnings) = build_block_element(
            &template,
            &empty_source,
            &json!({"renderType": 11, "customModelName": "Single texture", "texture": "ruby_ore"}),
            &BTreeMap::new(),
            89,
            &BTreeMap::new(),
        );
        assert!(warnings.is_empty());
        assert_eq!(empty["definition"]["customDrop"], "");
        assert_eq!(empty["definition"]["dropAmount"], 0);
        assert_eq!(empty["definition"]["useLootTableForDrops"], false);
    }

    #[test]
    fn exports_registered_block_with_blockstate_model_texture_and_manifest(){
        let temp=tempdir().unwrap();
        let source=temp.path().join("source");
        let workspace=temp.path().join("workspace");
        let assets=source.join("assets/demo");
        for directory in [
            assets.join("blockstates"),assets.join("models/block"),assets.join("textures/block")
        ]{fs::create_dir_all(&directory).unwrap();}
        fs::write(source.join("Registry.java"),r#"
            public class Registry {
                public static Block DEMO = new DemoBlock();
                public static void register(){ GameRegistry.registerBlock(DEMO,"demo_block"); }
            }
            class DemoBlock extends Block { }
        "#).unwrap();
        fs::write(assets.join("blockstates/demo_block.json"),br#"{"variants":{"":{"model":"demo:block/demo_block"}}}"#).unwrap();
        fs::write(assets.join("models/block/demo_block.json"),br#"{"parent":"minecraft:block/cube_all","textures":{"all":"demo:block/demo_block"}}"#).unwrap();
        fs::write(assets.join("textures/block/demo_block.png"),b"png").unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("test.mcreator"),serde_json::to_vec(&json!({
            "mcreatorVersion":202600,"workspaceSettings":{"modid":"converted"},"mod_elements":[]
        })).unwrap()).unwrap();

        let report=export_blocks_to_workspace(&[source],&workspace,"demo",89).unwrap();
        assert_eq!(report.discovered,1);
        assert_eq!(report.exported,1);
        let element:Value=serde_json::from_slice(&fs::read(workspace.join("elements/DemoBlock.mod.json")).unwrap()).unwrap();
        assert_eq!(element["_type"],"block");
        assert_eq!(element["definition"]["renderType"],11);
        assert_eq!(element["definition"]["texture"],"demo_block");
        assert!(workspace.join("src/main/resources/assets/converted/blockstates/demo_block.json").is_file());
        assert!(workspace.join("src/main/resources/assets/converted/models/block/demo_block.json").is_file());
        assert!(workspace.join("src/main/resources/assets/converted/textures/block/demo_block.png").is_file());
    }
}
