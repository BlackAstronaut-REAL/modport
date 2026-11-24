use anyhow::{bail, Context, Result};
use mojmodelbridge::metadata::infer_mod_metadata;
use mojmodelbridge::mod_graph::scan_mod_graph;
use mojmodelbridge::mcreator_armor::discover_armor_sources;
use mojmodelbridge::mcreator_block::{discover_block_sources};
use mojmodelbridge::mcreator_equipment::discover_equipment_sources;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const CATEGORIES: &[&str] = &[
    "Mobs", "Blocks", "Weapons", "Tools", "Armor", "Items", "Food", "Models",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    MobEgg,
    Texture,
    Model,
}

#[derive(Debug, Clone)]
pub struct BrowserEntry {
    pub category: String,
    pub name: String,
    pub detail: String,
    pub icon_kind: IconKind,
    pub icon_path: Option<PathBuf>,
    /// Legacy Java ModelRenderer / ModelBase model source.
    pub model_source: Option<PathBuf>,
    /// Java Edition item/block resource model JSON with explicit cuboid elements.
    pub resource_model_source: Option<PathBuf>,
    /// Additional source textures referenced by a linked resource model.
    pub texture_paths: Vec<PathBuf>,
    pub lore: Vec<String>,
    pub components_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserIndex {
    pub root: Option<PathBuf>,
    pub display_name: String,
    pub entries: Vec<BrowserEntry>,
    /// Project roots, not only texture subdirectories: model/animation texture
    /// references may resolve from any resource namespace underneath them.
    pub texture_roots: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl BrowserIndex {
    pub fn in_category(&self, category: &str) -> Vec<usize> {
        self.entries.iter().enumerate()
            .filter_map(|(index, entry)| (entry.category == category).then_some(index))
            .collect()
    }
}

pub fn scan_mod(root: &Path) -> Result<BrowserIndex> {
    if !root.is_dir() {
        bail!("Choose an extracted mod/project directory. Raw JAR/ZIP browser indexing is not enabled yet.");
    }
    let root = fs::canonicalize(root)
        .with_context(|| format!("Resolving mod directory {}", root.display()))?;
    let metadata = infer_mod_metadata(&root);
    let source_modid = metadata.modid.clone();
    let display_name = if !metadata.name.is_empty() {
        metadata.name
    } else if !source_modid.is_empty() {
        source_modid.clone()
    } else {
        root.file_name().and_then(|value| value.to_str()).unwrap_or("Mod project").to_owned()
    };

    let graph = scan_mod_graph(&root, &[])?;
    let mut all_files = Vec::<PathBuf>::new();
    for entry in WalkDir::new(&root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file() { continue; }
        let path = entry.path();
        if path.components().any(|component| {
            matches!(component.as_os_str().to_string_lossy().to_ascii_lowercase().as_str(),
                ".git" | ".gradle" | "build" | "target" | "node_modules")
        }) {
            continue;
        }
        all_files.push(path.to_path_buf());
        if all_files.len() >= 250_000 { break; }
    }
    all_files.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());

    let source_roots = vec![root.clone()];
    let equipment = discover_equipment_sources(&source_roots)?;
    let blocks = discover_block_sources(&source_roots)?;
    let armor_sets = discover_armor_sources(&source_roots)?;
    let languages = load_language_data(&all_files, &source_modid);
    let mut entries = Vec::<BrowserEntry>::new();
    let mut seen = HashSet::<String>::new();

    let mut add = |entry: BrowserEntry| {
        let key = format!("{}:{}", entry.category.to_ascii_lowercase(), entry.name.to_ascii_lowercase());
        if seen.insert(key) { entries.push(entry); }
    };

    // Registered mobs are taken from the existing registration graph.
    for (entity_name, entity) in &graph.entities {
        let registration = graph.registrations.iter()
            .find(|registration| registration.entity_class == *entity_name);
        let model_source = registration.and_then(|registration| {
            let model = registration.model_class.rsplit(['.', '/', '$']).next().unwrap_or("");
            graph.java_files.get(model).or_else(|| graph.class_files.get(model)).cloned()
        });
        let icon_path = registration.and_then(|registration| {
            find_texture_reference(&all_files, &registration.texture_refs)
        });
        add(BrowserEntry {
            category: "Mobs".to_owned(),
            name: if entity.display_name.is_empty() { humanize(entity_name) } else { entity.display_name.clone() },
            detail: if let Some(registration) = registration {
                if registration.model_class.is_empty() { entity.registry_name.clone() }
                else { format!("{} · {}", entity.registry_name, registration.model_class) }
            } else { entity.registry_name.clone() },
            icon_kind: IconKind::MobEgg,
            icon_path,
            model_source,
            resource_model_source: None,
            texture_paths: Vec::new(),
            lore: Vec::new(),
            components_json: "{}".to_owned(),
        });
    }

    // Registered items/tools are classified from superclass and constructor
    // evidence, never from texture filename substrings.
    for source in &equipment {
        let registry = normalize_registry(&source.registry_name);
        let category = if source.element_type == "tool" {
            if matches!(source.tool_type.as_deref(), Some("Sword" | "Bow" | "Crossbow" | "Trident" | "Staff" | "Dagger" | "Spear")) {
                "Weapons"
            } else {
                "Tools"
            }
        } else if is_food_source(&source.source_text, &source.superclass) {
            "Food"
        } else {
            "Items"
        };
        let texture_refs = equipment_texture_refs(&source.source_text);
        let icon_path = find_texture_reference(&all_files, &texture_refs)
            .or_else(|| find_texture_by_stems(&all_files, &[registry.as_str(), source.class_name.as_str()]));
        let resource_model_source = find_item_model_json(&all_files, &registry);
        let (renderer_model, renderer_textures) = find_held_renderer_model(&all_files, &registry);
        let model_source = renderer_model
            .or_else(|| find_linked_java_model(&all_files, &source.source_text, false));
        let mut lore = extract_tooltip_lines(&source.source_text);
        append_unique(&mut lore, &languages.descriptions_for(&registry));
        let components_json = find_item_components(&all_files, &registry)
            .map(|value| serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned()))
            .unwrap_or_else(|| "{}".to_owned());
        let mut texture_paths = renderer_textures;
        if let Some(path) = resource_model_source.as_ref() {
            append_paths(&mut texture_paths, &resource_model_texture_paths(&all_files, path));
        }
        add(BrowserEntry {
            category: category.to_owned(),
            name: languages.name_for(&registry).unwrap_or_else(|| humanize(&registry)),
            detail: format!("{} · {}", registry, source.class_name),
            icon_kind: IconKind::Texture,
            icon_path,
            model_source,
            resource_model_source,
            texture_paths,
            lore,
            components_json,
        });
    }

    // Armor pieces come from the armor set/registry parser. A Java getArmorModel
    // override can link the legacy model that should appear when the piece is selected.
    for set in &armor_sets {
        let armor_model = find_linked_armor_model(&all_files, &set.source_text, &set.prefix);
        for (part, field) in &set.fields {
            let registry = set.registered_piece_ids.get(field)
                .cloned().unwrap_or_else(|| format!("{}_{}", normalize_registry(&set.prefix), part));
            let registry = normalize_registry(&registry);
            let source_text = all_files.iter()
                .find(|path| path.file_stem().and_then(|value| value.to_str())
                    .map(|stem| stem.eq_ignore_ascii_case(field)).unwrap_or(false)
                    && path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java")))
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            let texture_stems = [registry.as_str(), field.as_str(), set.prefix.as_str()];
            let icon_path = find_texture_by_stems(&all_files, &texture_stems);
            let mut lore = extract_tooltip_lines(&source_text);
            append_unique(&mut lore, &languages.descriptions_for(&registry));
            let resource_model_source = find_item_model_json(&all_files, &registry);
            let model_source = armor_model.clone()
                .or_else(|| find_linked_java_model(&all_files, &source_text, true));
            let components_json = find_item_components(&all_files, &registry)
                .map(|value| serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned()))
                .unwrap_or_else(|| "{}".to_owned());
            let mut texture_paths = find_armor_layer_textures(&all_files, &set.prefix, &set.source_text);
            if let Some(model_path) = resource_model_source.as_ref() {
                append_paths(&mut texture_paths, &resource_model_texture_paths(&all_files, model_path));
            }
            add(BrowserEntry {
                category: "Armor".to_owned(),
                name: languages.name_for(&registry).unwrap_or_else(|| humanize(&registry)),
                detail: format!("{} · {} armor set", registry, set.prefix),
                icon_kind: IconKind::Texture,
                icon_path,
                model_source,
                resource_model_source,
                texture_paths,
                lore,
                components_json,
            });
        }
    }

    // Registered blocks and plants are discovered from class ancestry/registry
    // declarations. Their block model JSON is linked for 3D preview when present.
    for source in &blocks {
        let registry = normalize_registry(&source.registry_name);
        let icon_path = find_texture_reference(&all_files, &block_texture_refs(&source.source_text))
            .or_else(|| find_texture_by_stems(&all_files, &[registry.as_str(), source.class_name.as_str()]));
        let resource_model_source = find_block_model_json(&all_files, &registry);
        let mut lore = extract_tooltip_lines(&source.source_text);
        append_unique(&mut lore, &languages.descriptions_for(&registry));
        let components_json = find_item_components(&all_files, &registry)
            .map(|value| serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned()))
            .unwrap_or_else(|| "{}".to_owned());
        let texture_paths = resource_model_source.as_ref()
            .map(|path| resource_model_texture_paths(&all_files, path))
            .unwrap_or_default();
        add(BrowserEntry {
            category: "Blocks".to_owned(),
            name: languages.name_for(&registry).unwrap_or_else(|| humanize(&registry)),
            detail: format!("{} · {}", registry, source.class_name),
            icon_kind: IconKind::Texture,
            icon_path,
            model_source: None,
            resource_model_source,
            texture_paths,
            lore,
            components_json,
        });
    }

    // Orphaned texture assets remain visible, but are placed in neutral Items
    // / Blocks buckets instead of guessed tool/weapon/food buckets.
    for path in &all_files {
        if !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png")) { continue; }
        let components: Vec<String> = path.components()
            .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
            .collect();
        let Some(texture_index) = components.iter().position(|part| part == "textures") else { continue; };
        let Some(kind) = components.get(texture_index + 1).map(String::as_str) else { continue; };
        let category = match kind {
            "block" | "blocks" => "Blocks",
            "item" | "items" => "Items",
            _ => continue,
        };
        let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("texture");
        let registry = normalize_registry(stem);
        let lore = languages.descriptions_for(&registry);
        let components_json = find_item_components(&all_files, &registry)
            .map(|value| serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned()))
            .unwrap_or_else(|| "{}".to_owned());
        add(BrowserEntry {
            category: category.to_owned(),
            name: languages.name_for(&registry).unwrap_or_else(|| humanize(stem)),
            detail: path.strip_prefix(&root).unwrap_or(path).to_string_lossy().to_string(),
            icon_kind: IconKind::Texture,
            icon_path: Some(path.clone()),
            model_source: None,
            resource_model_source: if category == "Items" {
                find_item_model_json(&all_files, &registry)
            } else {
                find_block_model_json(&all_files, &registry)
            },
            texture_paths: Vec::new(),
            lore,
            components_json,
        });
    }

    for path in &all_files {
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else { continue; };
        if !extension.eq_ignore_ascii_case("java") && !extension.eq_ignore_ascii_case("class") { continue; }
        let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("");
        if !stem.to_ascii_lowercase().starts_with("model") { continue; }
        let linked = graph.registrations.iter().any(|registration| {
            registration.model_class.rsplit(['.', '/', '$']).next()
                .map(|name| name.eq_ignore_ascii_case(stem)).unwrap_or(false)
        });
        if linked { continue; }
        add(BrowserEntry {
            category: "Models".to_owned(),
            name: humanize(stem.strip_prefix("Model").unwrap_or(stem)),
            detail: path.strip_prefix(&root).unwrap_or(path).to_string_lossy().to_string(),
            icon_kind: IconKind::Model,
            icon_path: None,
            model_source: Some(path.clone()),
            resource_model_source: None,
            texture_paths: Vec::new(),
            lore: Vec::new(),
            components_json: "{}".to_owned(),
        });
    }

    drop(add);
    entries.sort_by(|a, b| {
        category_order(&a.category).cmp(&category_order(&b.category))
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    let mut warnings = graph.warnings;
    if all_files.len() >= 250_000 {
        warnings.push("The browser scan reached its 250,000-file safety limit; some assets may not be listed.".to_owned());
    }
    if entries.is_empty() {
        warnings.push("No supported entity registrations, item/block textures, or model source files were found.".to_owned());
    }

    Ok(BrowserIndex {
        root: Some(root.clone()),
        display_name,
        entries,
        texture_roots: vec![root],
        warnings,
    })
}

#[derive(Debug, Default)]
struct LanguageData {
    names: BTreeMap<String, String>,
    descriptions: BTreeMap<String, Vec<String>>,
}

impl LanguageData {
    fn name_for(&self, registry: &str) -> Option<String> {
        self.names.get(&compact(registry)).cloned()
    }

    fn descriptions_for(&self, registry: &str) -> Vec<String> {
        self.descriptions.get(&compact(registry)).cloned().unwrap_or_default()
    }
}

fn load_language_data(files: &[PathBuf], modid: &str) -> LanguageData {
    let mut result = LanguageData::default();
    for path in files {
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
        if extension.eq_ignore_ascii_case("lang") {
            if !path.file_name().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase().contains("en_us") {
                continue;
            }
            let Ok(text) = fs::read_to_string(path) else { continue; };
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') { continue; }
                let Some((key, value)) = line.split_once('=') else { continue; };
                add_language_value(&mut result, key, value);
            }
        } else if path.file_name().and_then(|value| value.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("en_us.json"))
        {
            let Ok(bytes) = fs::read(path) else { continue; };
            let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else { continue; };
            if let Some(object) = payload.as_object() {
                for (key, value) in object {
                    if let Some(value) = value.as_str() {
                        add_language_value(&mut result, key, value);
                    }
                }
            }
        }
    }

    // Older Forge mods often provide no en_US resource and call the language
    // registry directly from Java source.
    let field_registry = source_field_registry_names(files);
    for path in files.iter().filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java"))) {
        let Ok(text) = fs::read_to_string(path) else { continue; };
        let name_call = Regex::new(r#"addNameForObject\s*\(\s*([A-Za-z_$][A-Za-z0-9_$]*)\s*,\s*["'](?:en_US|en_us)["']\s*,\s*["']((?:\\.|[^"'\\])*)["']\s*\)"#).unwrap();
        for capture in name_call.captures_iter(&text) {
            let field = &capture[1];
            let display = decode_java_string(&capture[2]);
            if let Some(registries) = field_registry.get(field) {
                for registry in registries {
                    result.names.entry(compact(registry)).or_insert_with(|| display.clone());
                }
            }
        }
        let localization = Regex::new(r#"addStringLocalization\s*\(\s*["']([^"']+)["']\s*,\s*["'](?:en_US|en_us)["']\s*,\s*["']((?:\\.|[^"'\\])*)["']\s*\)"#).unwrap();
        for capture in localization.captures_iter(&text) {
            add_language_value(&mut result, &capture[1], &decode_java_string(&capture[2]));
        }
    }

    if !modid.is_empty() {
        let _ = modid;
    }
    result
}

fn add_language_value(target: &mut LanguageData, key: &str, value: &str) {
    if value.trim().is_empty() { return; }
    let normalized = key.to_ascii_lowercase();
    let compact_key = compact(&normalized);
    let tokens = normalized.split(['.', ':']).collect::<Vec<_>>();
    let is_description = tokens.iter().any(|token| {
        matches!(*token, "desc" | "description" | "tooltip" | "info" | "lore")
    });
    let suffix = tokens.last().copied().unwrap_or("");
    let stem = if matches!(suffix, "name" | "desc" | "description" | "tooltip" | "info" | "lore") {
        tokens.get(tokens.len().saturating_sub(2)).copied().unwrap_or("")
    } else {
        suffix
    };
    if is_description {
        let key = compact(stem);
        if !key.is_empty() {
            let values = target.descriptions.entry(key).or_default();
            if !values.iter().any(|old| old == value.trim()) {
                values.push(value.trim().to_owned());
            }
        }
    } else {
        for token in tokens.iter().rev().take(3) {
            let key = compact(token);
            if !key.is_empty() {
                target.names.entry(key).or_insert_with(|| value.trim().to_owned());
            }
        }
        if !compact_key.is_empty() {
            target.names.entry(compact_key).or_insert_with(|| value.trim().to_owned());
        }
    }
}

fn source_field_registry_names(files: &[PathBuf]) -> BTreeMap<String, Vec<String>> {
    let mut fields = BTreeMap::<String, Vec<String>>::new();
    let assign = Regex::new(r"\b([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*new\s+[A-Za-z_$][A-Za-z0-9_$.]*\s*\([^;]*?\)").unwrap();
    let registration = Regex::new(r#"(?:registerItem|registerBlock)\s*\(\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*,\s*"([^"]+)""#).unwrap();
    for path in files.iter().filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java"))) {
        let Ok(text) = fs::read_to_string(path) else { continue; };
        let known = assign.captures_iter(&text).filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned())).collect::<HashSet<_>>();
        for capture in registration.captures_iter(&text) {
            let field = capture[1].rsplit('.').next().unwrap_or(&capture[1]);
            if known.contains(field) {
                fields.entry(field.to_owned()).or_default().push(normalize_registry(&capture[2]));
            }
        }
    }
    fields
}

fn extract_tooltip_lines(source: &str) -> Vec<String> {
    const FORMAT_CODES: &[(&str, &str)] = &[
        ("BLACK", "§0"), ("DARK_BLUE", "§1"), ("DARK_GREEN", "§2"),
        ("DARK_AQUA", "§3"), ("DARK_RED", "§4"), ("DARK_PURPLE", "§5"),
        ("GOLD", "§6"), ("GRAY", "§7"), ("DARK_GRAY", "§8"),
        ("BLUE", "§9"), ("GREEN", "§a"), ("AQUA", "§b"), ("RED", "§c"),
        ("LIGHT_PURPLE", "§d"), ("YELLOW", "§e"), ("WHITE", "§f"),
        ("BOLD", "§l"), ("STRIKETHROUGH", "§m"), ("UNDERLINE", "§n"),
        ("ITALIC", "§o"), ("RESET", "§r"),
    ];
    let calls = Regex::new(r"(?s)\b(?:tooltip|list|par\d+List)\s*\.\s*add\s*\((.*?)\)\s*;").unwrap();
    let literals = Regex::new(r#""((?:\\.|[^"\\])*)""#).unwrap();
    let mut result = Vec::new();
    for capture in calls.captures_iter(source) {
        let mut expression = capture[1].to_owned();
        for (name, code) in FORMAT_CODES {
            for class in ["TextFormatting", "EnumChatFormatting", "ChatFormatting", "Formatting"] {
                expression = expression.replace(&format!("{class}.{name}"), &format!("\"{code}\""));
            }
        }
        let mut value = String::new();
        for literal in literals.captures_iter(&expression) {
            value.push_str(&decode_java_string(&literal[1]));
        }
        let value = value.trim().to_owned();
        if !value.is_empty() && !result.contains(&value) { result.push(value); }
    }
    result
}

fn decode_java_string(value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => result.push('\n'),
            Some('r') => result.push('\r'),
            Some('t') => result.push('\t'),
            Some('b') => result.push('\u{0008}'),
            Some('f') => result.push('\u{000c}'),
            Some('"') => result.push('"'),
            Some('\'') => result.push('\''),
            Some('\\') => result.push('\\'),
            Some('u') => {
                let digits = chars.by_ref().take(4).collect::<String>();
                if let Ok(code) = u32::from_str_radix(&digits, 16) {
                    if let Some(decoded) = char::from_u32(code) { result.push(decoded); }
                }
            }
            Some(other) => result.push(other),
            None => result.push('\\'),
        }
    }
    result
}

fn is_food_source(source: &str, superclass: &str) -> bool {
    let parent = superclass.to_ascii_lowercase();
    parent.contains("itemfood") || parent.contains("itemseedfood")
        || source.contains("FoodProperties.Builder")
        || source.contains(".food(")
        || source.contains("setAlwaysEdible")
        || source.contains("setHealAmount")
        || source.contains("setPotionEffect")
}

fn equipment_texture_refs(source: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let single = Regex::new(r#"\b(?:setTextureName|func_111206_d|registerIcon|func_94245_a)\s*\(\s*"([^"]+)""#).unwrap();
    for capture in single.captures_iter(source) {
        refs.push(capture[1].to_owned());
    }
    let location = Regex::new(r#"new\s+ResourceLocation\s*\(\s*"([^"]+)"\s*,\s*"([^"]+)""#).unwrap();
    for capture in location.captures_iter(source) {
        refs.push(format!("{}:{}", &capture[1], &capture[2]));
    }
    refs.sort();
    refs.dedup();
    refs
}

fn block_texture_refs(source: &str) -> Vec<String> {
    equipment_texture_refs(source)
}

fn find_texture_by_stems(files: &[PathBuf], stems: &[&str]) -> Option<PathBuf> {
    let wanted = stems.iter().filter(|stem| !stem.is_empty()).map(|stem| compact(stem)).collect::<HashSet<_>>();
    files.iter().find(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
            && path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/textures/")
            && path.file_stem().and_then(|value| value.to_str())
                .map(|stem| wanted.contains(&compact(stem))).unwrap_or(false)
    }).cloned()
}

fn find_texture_reference(files: &[PathBuf], references: &[String]) -> Option<PathBuf> {
    for reference in references {
        let cleaned = reference.replace('\\', "/");
        let suffix = cleaned.rsplit(':').next().unwrap_or(&cleaned).trim_start_matches('/');
        let suffix = if suffix.to_ascii_lowercase().ends_with(".png") { suffix.to_owned() }
            else { format!("{suffix}.png") };
        if let Some(found) = files.iter().find(|path| {
            path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase())
        }) {
            return Some(found.clone());
        }
        if let Some(found) = find_texture_by_stems(files, &[Path::new(&suffix).file_stem().and_then(|value| value.to_str()).unwrap_or("")]) {
            return Some(found);
        }
    }
    None
}

fn find_item_model_json(files: &[PathBuf], registry: &str) -> Option<PathBuf> {
    let wanted = compact(registry);
    let mut candidates = files.iter().filter(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/models/item/")
    }).collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        let stem = compact(path.file_stem().and_then(|value| value.to_str()).unwrap_or(""));
        (if stem == wanted { 0 } else { 1 }, path.to_string_lossy().to_ascii_lowercase())
    });
    for path in candidates {
        let stem = compact(path.file_stem().and_then(|value| value.to_str()).unwrap_or(""));
        if stem != wanted { continue; }
        let Ok(bytes) = fs::read(path) else { continue; };
        let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else { continue; };
        let parent = payload.get("parent").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        let standard = ["item/generated", "item/handheld", "builtin/generated", "builtin/handheld"];
        let custom = payload.get("elements").and_then(Value::as_array).is_some_and(|elements| !elements.is_empty())
            || payload.get("display").is_some_and(Value::is_object)
            || (!parent.is_empty() && !standard.iter().any(|value| parent.ends_with(value)));
        if custom { return Some(path.clone()); }
    }
    None
}

fn find_block_model_json(files: &[PathBuf], registry: &str) -> Option<PathBuf> {
    let wanted = compact(registry);
    let mut candidates = files.iter().filter(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/models/block/")
            && path.file_stem().and_then(|value| value.to_str()).map(|stem| compact(stem) == wanted).unwrap_or(false)
    }).cloned().collect::<Vec<_>>();
    candidates.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    candidates.into_iter().next()
}

fn append_paths(target: &mut Vec<PathBuf>, values: &[PathBuf]) {
    for value in values {
        if !target.contains(value) { target.push(value.clone()); }
    }
}

fn find_armor_layer_textures(files: &[PathBuf], prefix: &str, source: &str) -> Vec<PathBuf> {
    let prefix = compact(prefix.trim_end_matches("Armor").trim_end_matches("armor"));
    let mut candidates = Vec::<PathBuf>::new();
    let mut references = Vec::<String>::new();
    let resource_location = Regex::new(r#"["']([^"'\s]+(?:textures/)?(?:models/armor|armor)/[^"'\s]+\.png)["']"#).unwrap();
    for capture in resource_location.captures_iter(source) {
        references.push(capture[1].to_owned());
    }

    for reference in &references {
        if let Some(path) = find_texture_reference(files, std::slice::from_ref(reference)) {
            if !candidates.contains(&path) { candidates.push(path); }
        }
    }
    let mut matching = files.iter().filter(|path| {
        if !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png")) {
            return false;
        }
        let normalized = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        let armor_directory = normalized.contains("/textures/models/armor/")
            || normalized.contains("/textures/armor/");
        if !armor_directory { return false; }
        let stem = compact(path.file_stem().and_then(|value| value.to_str()).unwrap_or(""));
        prefix.is_empty() || stem.contains(&prefix)
    }).cloned().collect::<Vec<_>>();
    matching.sort_by_key(|path| {
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
        let layer = if name.contains("layer_1") || name.contains("layer1") { 0 }
            else if name.contains("layer_2") || name.contains("layer2") { 1 }
            else { 2 };
        (layer, path.to_string_lossy().to_ascii_lowercase())
    });
    append_paths(&mut candidates, &matching);
    candidates
}

fn resource_model_texture_paths(files: &[PathBuf], model: &Path) -> Vec<PathBuf> {
    let Ok(bytes) = fs::read(model) else { return Vec::new(); };
    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else { return Vec::new(); };
    let Some(textures) = payload.get("textures").and_then(Value::as_object) else { return Vec::new(); };
    let mut paths = Vec::new();
    for reference in textures.values().filter_map(Value::as_str) {
        if reference.starts_with('#') { continue; }
        let suffix = reference.split_once(':').map(|(_, path)| path).unwrap_or(reference);
        let suffix = suffix.trim_start_matches("textures/").trim_start_matches('/');
        let suffix = if suffix.to_ascii_lowercase().ends_with(".png") { suffix.to_owned() }
            else { format!("{suffix}.png") };
        if let Some(found) = files.iter().find(|path| {
            path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase())
        }) {
            if !paths.contains(found) { paths.push(found.clone()); }
        }
    }
    paths
}

fn find_model_source_by_class(files: &[PathBuf], class_name: &str) -> Option<PathBuf> {
    let simple = class_name.rsplit(['.', '/', '$']).next().unwrap_or(class_name);
    files.iter().filter(|path| {
        path.file_stem().and_then(|value| value.to_str()).is_some_and(|stem| stem.eq_ignore_ascii_case(simple))
            && path.extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("java") || extension.eq_ignore_ascii_case("class")
            })
    }).min_by_key(|path| {
        let priority = if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java")) { 0 } else { 1 };
        (priority, path.to_string_lossy().to_ascii_lowercase())
    }).cloned()
}

fn find_held_renderer_model(files: &[PathBuf], registry: &str) -> (Option<PathBuf>, Vec<PathBuf>) {
    let field_registries = source_field_registry_names(files);
    let mut source_by_class = BTreeMap::<String, String>::new();
    for path in files.iter().filter(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java"))
    }) {
        let Some(class_name) = path.file_stem().and_then(|value| value.to_str()) else { continue; };
        if let Ok(text) = fs::read_to_string(path) {
            source_by_class.entry(class_name.to_owned()).or_insert(text);
        }
    }

    let target = compact(registry);
    let registration = Regex::new(
        r#"(?s)(?:MinecraftForgeClient\s*\.\s*)?registerItemRenderer\s*\(\s*(?:[A-Za-z_$][A-Za-z0-9_$]*\s*\.\s*)*([A-Za-z_$][A-Za-z0-9_$]*)\s*,\s*new\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\("#
    ).unwrap();
    let model_pattern = Regex::new(r"\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(").unwrap();
    let texture_pattern = Regex::new(r#"(?i)["']([^"'\s]+(?:\.png|/textures/[^"'\s]+))["']"#).unwrap();
    let mut model_source = None;
    let mut textures = Vec::new();

    for text in source_by_class.values() {
        for capture in registration.captures_iter(text) {
            let field = &capture[1];
            let renderer_class = capture[2].rsplit('.').next().unwrap_or(&capture[2]);
            let registry_match = field_registries.get(field).is_some_and(|registries| {
                registries.iter().any(|name| compact(name) == target)
            });
            if !registry_match { continue; }

            let Some(renderer_text) = source_by_class.get(renderer_class) else { continue; };
            if model_source.is_none() {
                if let Some(model) = model_pattern.captures(renderer_text).and_then(|capture| {
                    find_model_source_by_class(files, &capture[1])
                }) {
                    model_source = Some(model);
                }
            }
            for texture in texture_pattern.captures_iter(renderer_text) {
                let reference = texture[1].replace('\\', "/");
                if let Some(path) = find_texture_reference(files, std::slice::from_ref(&reference)) {
                    if !textures.contains(&path) { textures.push(path); }
                }
            }
        }
    }
    (model_source, textures)
}

fn find_linked_java_model(files: &[PathBuf], source: &str, armor: bool) -> Option<PathBuf> {
    if source.is_empty() { return None; }
    if armor && !source.contains("getArmorModel") { return None; }
    if !armor && !source.contains("IItemRenderer") && !source.contains("renderItem") && !source.contains("getArmorModel") {
        return None;
    }
    let pattern = Regex::new(r"\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(").unwrap();
    for capture in pattern.captures_iter(source) {
        if let Some(path) = find_model_source_by_class(files, &capture[1]) {
            return Some(path);
        }
    }
    None
}

fn find_linked_armor_model(files: &[PathBuf], source: &str, prefix: &str) -> Option<PathBuf> {
    let mut sources = Vec::new();
    if !source.is_empty() { sources.push(source.to_owned()); }
    let prefix = compact(prefix);
    for path in files.iter().filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java"))) {
        if !path.file_stem().and_then(|value| value.to_str()).map(|stem| compact(stem).contains(&prefix)).unwrap_or(false) {
            continue;
        }
        if let Ok(text) = fs::read_to_string(path) {
            if text.contains("getArmorModel") { sources.push(text); }
        }
    }
    for text in sources {
        if !text.contains("getArmorModel") { continue; }
        if let Some(method) = Regex::new(r"\bgetArmorModel\s*\([^)]*\)\s*\{").unwrap().find(&text) {
            let body_start = method.end();
            let body = text[body_start..].split_once('}').map(|(body, _)| body).unwrap_or("");
            if let Some(capture) = Regex::new(r"\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(").unwrap().captures(body) {
                if let Some(path) = find_model_source_by_class(files, &capture[1]) {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn find_item_components(files: &[PathBuf], registry: &str) -> Option<Value> {
    let wanted = compact(registry);
    for path in files.iter().filter(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && (path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/items/")
                || path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/item_components/"))
    }) {
        if !path.file_stem().and_then(|value| value.to_str()).map(|stem| compact(stem) == wanted).unwrap_or(false) {
            continue;
        }
        let Ok(bytes) = fs::read(path) else { continue; };
        let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else { continue; };
        if let Some(components) = payload.get("components") {
            return Some(components.clone());
        }
        if payload.as_object().is_some_and(|object| object.keys().any(|key| key.starts_with("minecraft:"))) {
            return Some(payload);
        }
    }
    None
}

fn append_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.contains(value) { target.push(value.clone()); }
    }
}

fn normalize_registry(value: &str) -> String {
    let mut value = value.trim().trim_matches('"').trim_matches('\'').to_owned();
    if let Some((_, path)) = value.split_once(':') { value = path.to_owned(); }
    value.trim_matches('/').replace('-', "_").replace('/', "_").to_ascii_lowercase()
}

fn compact(value: &str) -> String {
    value.chars().filter(|character| character.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn category_order(category: &str) -> usize {
    CATEGORIES.iter().position(|candidate| *candidate == category).unwrap_or(usize::MAX)
}

fn humanize(value: &str) -> String {
    let chars: Vec<char> = value.replace('_', " ").replace('-', " ").chars().collect();
    let mut output = String::with_capacity(chars.len() + 8);
    for (index, ch) in chars.iter().copied().enumerate() {
        if index > 0 && ch.is_ascii_uppercase() && chars[index - 1].is_ascii_lowercase() {
            output.push(' ');
        }
        output.push(ch);
    }
    output.split_whitespace().map(|word| {
        let mut chars = word.chars();
        chars.next().map(|first| first.to_uppercase().to_string() + chars.as_str()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_extraction_preserves_legacy_formatting_and_unicode_literals() {
        let lines = extract_tooltip_lines(r#"
            tooltip.add(TextFormatting.GOLD + "A golden blade");
            par1List.add(EnumChatFormatting.ITALIC + "Forged\nbright");
        "#);
        assert_eq!(lines[0], "§6A golden blade");
        assert_eq!(lines[1], "§oForged\nbright");
    }

    #[test]
    fn registry_category_logic_does_not_use_texture_filename_guesses() {
        assert_eq!(category_order("Weapons"), 2);
        assert_eq!(category_order("Items"), 5);
        assert_eq!(compact("red_ant"), "redant");
    }

    #[test]
    fn humanizes_registry_and_model_names() {
        assert_eq!(humanize("red_ant"), "Red Ant");
        assert_eq!(humanize("FireBeetle"), "Fire Beetle");
        assert_eq!(humanize("ModelTriffid"), "Model Triffid");
    }
}
