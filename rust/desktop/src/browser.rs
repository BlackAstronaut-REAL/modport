use anyhow::{bail, Context, Result};
use mojmodelbridge::metadata::infer_mod_metadata;
use mojmodelbridge::mod_graph::{scan_mod_graph, EntityMetadata};
use mojmodelbridge::mcreator_armor::discover_armor_sources;
use mojmodelbridge::mcreator_block::{discover_block_sources, is_plant_source};
use mojmodelbridge::mcreator_equipment::{discover_equipment_sources, source_tool_materials, ToolMaterialStats};
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
    /// In-game block render archetype inferred from blockstate/model JSON.
    pub render_shape: Option<String>,
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
    let tool_materials = source_tool_materials(&source_roots, &source_modid);
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
        let entity_source_path = graph.java_files.get(entity_name)
            .or_else(|| graph.class_files.get(entity_name)).cloned();
        let (mob_lore, mob_components) = mob_component_metadata(entity, entity_source_path.as_deref());
        add(BrowserEntry {
            category: "Mobs".to_owned(),
            name: if entity.display_name.is_empty() { humanize(entity_name) } else { entity.display_name.clone() },
            detail: if let Some(registration) = registration {
                if registration.model_class.is_empty() { entity.registry_name.clone() }
                else { format!("{} · {}", entity.registry_name, registration.model_class) }
            } else { entity.registry_name.clone() },
            icon_kind: IconKind::MobEgg,
            icon_path: icon_path.clone(),
            model_source,
            resource_model_source: None,
            texture_paths: icon_path.into_iter().collect(),
            render_shape: None,
            lore: mob_lore,
            components_json: mob_components,
        });
    }

    // Registered items/tools are classified from superclass and constructor
    // evidence, never from texture filename substrings.
    for source in &equipment {
        let registry = normalize_registry(&source.registry_name);
        let category = if is_weapon_source(source) {
            "Weapons"
        } else if is_food_source(&source.source_text, &source.superclass) {
            "Food"
        } else if source.element_type == "tool" {
            "Tools"
        } else {
            "Items"
        };
        let resource_model_source = find_item_model_json(&all_files, &registry);
        let (renderer_model, renderer_textures) = find_held_renderer_model(&all_files, &registry);
        let texture_refs = equipment_texture_refs(&source.source_text);
        let icon_path = find_texture_reference(&all_files, &texture_refs)
            .or_else(|| renderer_textures.first().cloned())
            .or_else(|| resource_model_source.as_ref()
                .and_then(|path| resource_model_texture_paths(&all_files, path).first().cloned()));
        let model_source = renderer_model
            .or_else(|| find_linked_java_model(&all_files, &source.source_text, false));
        let mut lore = extract_tooltip_lines(&source.source_text);
        append_unique(&mut lore, &languages.descriptions_for(&registry));
        let (source_properties, source_property_lore) =
            item_source_properties(source, tool_materials.get(&compact(&registry)));
        append_unique(&mut lore, &source_property_lore);
        let mut components = find_item_components(&all_files, &registry)
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(object) = components.as_object_mut() {
            object.insert("source_properties".to_owned(), source_properties);
        } else {
            components = serde_json::json!({
                "minecraft_components": components,
                "source_properties": source_properties,
            });
        }
        let components_json = serde_json::to_string_pretty(&components)
            .unwrap_or_else(|_| "{}".to_owned());
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
            render_shape: None,
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
            let mut lore = extract_tooltip_lines(&set.source_text);
            append_unique(&mut lore, &extract_tooltip_lines(&source_text));
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
                render_shape: None,
                lore,
                components_json,
            });
        }
    }

    // Registered blocks and plants are discovered from class ancestry/registry
    // declarations. Their block model JSON is linked for 3D preview when present.
    for source in &blocks {
        let registry = normalize_registry(&source.registry_name);
        let (resource_model_source, blockstate_shape) = find_block_model_json(&all_files, &registry);
        let texture_refs = block_texture_refs(&source.source_text);
        let (legacy_model_source, legacy_model_textures) =
            find_linked_legacy_block_model(&all_files, &source.source_text);
        let mut texture_paths = legacy_model_textures;
        if let Some(path) = resource_model_source.as_ref() {
            append_paths(&mut texture_paths, &resource_model_texture_paths(&all_files, path));
        }
        let icon_path = find_texture_reference(&all_files, &texture_refs)
            .or_else(|| texture_paths.first().cloned());
        let render_shape = resource_model_source.as_ref()
            .map(|path| block_render_shape(path, source))
            .or(blockstate_shape)
            .or_else(|| is_plant_source(source).then(|| "plant".to_owned()))
            .unwrap_or_else(|| if legacy_model_source.is_some() { "custom".to_owned() } else { "cube".to_owned() });
        let mut lore = extract_tooltip_lines(&source.source_text);
        append_unique(&mut lore, &languages.descriptions_for(&registry));
        let components_json = find_item_components(&all_files, &registry)
            .map(|value| serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned()))
            .unwrap_or_else(|| "{}".to_owned());
        add(BrowserEntry {
            category: "Blocks".to_owned(),
            name: languages.name_for(&registry).unwrap_or_else(|| humanize(&registry)),
            detail: format!("{} · {}", registry, source.class_name),
            icon_kind: IconKind::Texture,
            icon_path,
            model_source: legacy_model_source,
            resource_model_source,
            texture_paths,
            render_shape: Some(render_shape),
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
                find_block_model_json(&all_files, &registry).0
            },
            texture_paths: Vec::new(),
            render_shape: if category == "Blocks" { Some("orphan texture".to_owned()) } else { None },
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
            render_shape: None,
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
        let compact_registry = compact(registry);
        let mut variants = vec![compact_registry.clone()];
        for suffix in [
            "ingot", "ore", "block", "item", "tool", "sword", "staff", "bow",
            "helmet", "chestplate", "leggings", "boots",
        ] {
            if compact_registry.ends_with(suffix) && compact_registry.len() > suffix.len() {
                variants.push(compact_registry[..compact_registry.len() - suffix.len()].to_owned());
            }
        }
        let mut result = Vec::new();
        for variant in variants {
            if let Some(values) = self.descriptions.get(&variant) {
                append_unique(&mut result, values);
            }
        }
        result
    }
}

fn format_stat(value: f64) -> String {
    if value.fract().abs() < 0.0001 {
        format!("{value:.0}")
    } else {
        format!("{value:.3}").trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

fn has_special_ability_evidence(source: &str, ai_classes: &[String]) -> bool {
    const TOKENS: &[&str] = &[
        "specialability", "special_attack", "specialattack", "performrangedattack",
        "rangedattack", "launchprojectile", "fireball", "breathattack", "dragonbreath",
        "teleport", "summon", "transform", "regenerate", "regeneration", "shield",
        "healself", "chargeattack", "roar", "stomp", "freeze", "lightning",
        "poison", "sonic", "laser", "beamattack", "dashattack", "explosionattack",
    ];
    let lowered = source.to_ascii_lowercase();
    if TOKENS.iter().any(|token| lowered.contains(token)) {
        return true;
    }
    const VANILLA_GOALS: &[&str] = &[
        "entityaimeleeattack", "entityairangedattack", "entityaifollowowner",
        "entityaifollowparent", "entityaipanic", "entityaiwander", "entityaiwatchclosest",
        "entityailookidle", "entityaiswimming", "entityaisit", "entityaieatgrass",
        "entityaitempt", "entityaimate", "entityaibeg", "entityaidefendvillage",
        "entityaidefendmob", "entityaicreeperswell", "entityaiswell",
        "entityairunaroundlikecrazy",
    ];
    ai_classes.iter().any(|goal| {
        let name = compact(goal);
        !VANILLA_GOALS.iter().any(|vanilla| name == *vanilla)
            && (name.contains("attack") || name.contains("ability")
                || name.contains("projectile") || name.contains("special")
                || name.contains("breath") || name.contains("teleport")
                || name.contains("summon") || name.contains("charge"))
    })
}

fn mob_component_metadata(entity: &EntityMetadata, source_path: Option<&Path>) -> (Vec<String>, String) {
    let source = source_path.and_then(|path| {
        if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java")) {
            fs::read_to_string(path).ok()
        } else {
            fs::read(path).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        }
    }).unwrap_or_default();
    let special = has_special_ability_evidence(&source, &entity.ai_classes);
    let mut lore = vec![
        format!("Health: {}", format_stat(entity.health)),
        format!("Attack damage: {}", format_stat(entity.attack_damage)),
        format!("Armor: {}", format_stat(entity.armor)),
        format!("Movement speed: {}", format_stat(entity.movement_speed)),
        format!("Follow range: {}", format_stat(entity.follow_range)),
        format!("Size: {} × {}", format_stat(entity.width), format_stat(entity.height)),
        format!("Experience: {}", entity.xp),
        format!("Special abilities: {}", if special { "true" } else { "false" }),
    ];
    if entity.fire_immune { lore.push("Fire immune: true".to_owned()); }
    if entity.water_mob { lore.push("Water mob: true".to_owned()); }
    if entity.flying_mob { lore.push("Flying mob: true".to_owned()); }
    if !entity.ai_classes.is_empty() {
        lore.push(format!("AI goals: {}", entity.ai_classes.join(", ")));
    }
    let components = serde_json::json!({
        "entity": {
            "registry": entity.registry_name,
            "class": entity.class_name,
            "superclass": entity.superclass,
        },
        "attributes": {
            "max_health": entity.health,
            "attack_damage": entity.attack_damage,
            "armor": entity.armor,
            "movement_speed": entity.movement_speed,
            "follow_range": entity.follow_range,
        },
        "dimensions": {
            "width": entity.width,
            "height": entity.height,
        },
        "experience": entity.xp,
        "flags": {
            "fire_immune": entity.fire_immune,
            "water_mob": entity.water_mob,
            "flying_mob": entity.flying_mob,
        },
        "ai_goals": entity.ai_classes,
        "special_abilities": special,
    });
    (lore, serde_json::to_string_pretty(&components).unwrap_or_else(|_| "{}".to_owned()))
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

fn is_weapon_source(source: &mojmodelbridge::mcreator_equipment::EquipmentSource) -> bool {
    if matches!(source.tool_type.as_deref(), Some("Sword" | "Bow" | "Crossbow" | "Trident" | "Staff" | "Dagger" | "Spear" | "Mace")) {
        return true;
    }
    let evidence = format!(
        "{} {} {}",
        source.class_name.to_ascii_lowercase(),
        source.superclass.to_ascii_lowercase(),
        source.registry_name.to_ascii_lowercase(),
    );
    [
        "itemsword", "itembow", "itemcrossbow", "itemtrident",
        "sword", "bow", "crossbow", "trident", "dagger", "spear",
        "weapon", "battleaxe", "waraxe", "mace",
    ].iter().any(|token| evidence.contains(token))
}

fn numeric_source_call(source: &str, methods: &[&str]) -> Option<f64> {
    let alternatives = methods.iter().map(|name| regex::escape(name)).collect::<Vec<_>>().join("|");
    let number = r"([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)[fFdD]?";
    let direct = Regex::new(&format!(
        r"(?is)(?:\b(?:{alternatives})\s*\(\s*|\b(?:{alternatives})\s*=\s*){number}"
    )).ok()?;
    if let Some(value) = direct.captures(source).and_then(|capture| capture.get(1)) {
        if let Ok(value) = value.as_str().parse::<f64>() {
            return Some(value);
        }
    }

    // Legacy item stats such as getItemEnchantability() are often implemented
    // as a no-argument method returning a constant rather than a field/setter.
    for method in methods {
        let declaration = Regex::new(&format!(
            r"(?is)\b(?:public|protected|private)?\s*(?:static\s+)?(?:final\s+)?(?:int|float|double|long)\s+{}\s*\([^)]*\)\s*\{{",
            regex::escape(method)
        )).ok()?;
        let Some(found) = declaration.find(source) else { continue; };
        let Some(body) = method_body(source, found.start()) else { continue; };
        let returned = Regex::new(&format!(r"(?is)\breturn\s+{number}\s*;")).ok()?;
        if let Some(value) = returned.captures(body).and_then(|capture| capture.get(1)) {
            if let Ok(value) = value.as_str().parse::<f64>() {
                return Some(value);
            }
        }
    }
    None
}

fn item_source_properties(
    source: &mojmodelbridge::mcreator_equipment::EquipmentSource,
    material: Option<&ToolMaterialStats>,
) -> (Value, Vec<String>) {
    let text = &source.source_text;
    let mut properties = serde_json::Map::new();
    let mut lore = Vec::new();
    let fields: &[(&str, &[&str], &str)] = &[
        ("max_stack_size", &["setMaxStackSize", "stacksTo", "maxStackSize", "getItemStackLimit"], "Stack size"),
        ("max_damage", &["setMaxDamage", "durability", "maxDamage", "getMaxDamage"], "Durability"),
        ("attack_damage", &["setAttackDamage", "attackDamage", "getDamageVsEntity"], "Attack damage"),
        ("enchantability", &["getItemEnchantability", "getEnchantability", "getEnchantmentValue", "enchantability"], "Enchantability"),
        ("nutrition", &["setHealAmount", "nutrition"], "Food nutrition"),
        ("saturation", &["setSaturationModifier", "saturationMod", "saturation"], "Food saturation"),
    ];
    for (key, names, label) in fields {
        if let Some(value) = numeric_source_call(text, names) {
            properties.insert((*key).to_owned(), serde_json::json!(value));
            lore.push(format!("{label}: {}", format_stat(value)));
        }
    }

    if let Some(material) = material {
        if let Some(uses) = material.uses {
            properties.entry("max_damage".to_owned()).or_insert_with(|| serde_json::json!(uses));
        }
        if let Some(efficiency) = material.efficiency {
            properties.insert("efficiency".to_owned(), serde_json::json!(efficiency));
            lore.push(format!("Mining efficiency: {}", format_stat(efficiency)));
        }
        if let Some(level) = material.enchantability {
            properties.entry("enchantability".to_owned()).or_insert_with(|| serde_json::json!(level));
        }
        if let Some(harvest) = material.harvest_level {
            properties.insert("harvest_level".to_owned(), serde_json::json!(harvest));
            lore.push(format!("Harvest level: {}", format_stat(harvest as f64)));
        }
        if let Some(damage) = material.damage {
            let offset = match source.tool_type.as_deref() {
                Some("Sword") => 4.0,
                Some("Axe") => 3.0,
                Some("Pickaxe") => 2.0,
                Some("Shovel") => 1.0,
                Some("Hoe") => 0.0,
                _ => 0.0,
            };
            properties.entry("attack_damage".to_owned())
                .or_insert_with(|| serde_json::json!(damage + offset));
        }
        if let Some(value) = properties.get("max_damage").and_then(Value::as_f64) {
            if !lore.iter().any(|line| line.starts_with("Durability:")) {
                lore.push(format!("Durability: {}", format_stat(value)));
            }
        }
        if let Some(value) = properties.get("enchantability").and_then(Value::as_i64) {
            if !lore.iter().any(|line| line.starts_with("Enchantability:")) {
                lore.push(format!("Enchantability: {value}"));
            }
        }
        if let Some(value) = properties.get("attack_damage").and_then(Value::as_f64) {
            if !lore.iter().any(|line| line.starts_with("Attack damage:")) {
                lore.push(format!("Attack damage: {}", format_stat(value)));
            }
        }
    }

    if let Some(capture) = Regex::new(r"(?i)\b(?:Rarity|EnumRarity)\s*\.\s*(COMMON|UNCOMMON|RARE|EPIC)").unwrap()
        .captures(text)
    {
        let rarity = capture[1].to_ascii_lowercase();
        properties.insert("rarity".to_owned(), serde_json::json!(rarity));
        lore.push(format!("Rarity: {}", rarity));
    }
    if let Some(tool_type) = source.tool_type.as_deref() {
        properties.insert("tool_type".to_owned(), serde_json::json!(tool_type));
        if !lore.iter().any(|line| line.starts_with("Tool type:")) {
            lore.push(format!("Tool type: {tool_type}"));
        }
    }

    let is_food = is_food_source(text, &source.superclass);
    properties.insert("is_food".to_owned(), serde_json::json!(is_food));
    if is_food {
        if let Some(capture) = Regex::new(r"(?is)\bsuper\s*\(\s*(-?\d+)\s*,\s*(-?\d+(?:\.\d+)?)[fFdD]?\s*(?:,\s*(true|false))?").unwrap()
            .captures(text)
        {
            if !properties.contains_key("nutrition") {
                if let Ok(value) = capture[1].parse::<i64>() {
                    properties.insert("nutrition".to_owned(), serde_json::json!(value));
                }
            }
            if !properties.contains_key("saturation") {
                if let Ok(value) = capture[2].parse::<f64>() {
                    properties.insert("saturation".to_owned(), serde_json::json!(value));
                }
            }
            if let Some(meat) = capture.get(3) {
                properties.insert("is_meat".to_owned(), serde_json::json!(meat.as_str().eq_ignore_ascii_case("true")));
            }
        }
        if let Some(value) = properties.get("nutrition").and_then(Value::as_i64) {
            lore.push(format!("Food nutrition: {value}"));
        }
        if let Some(value) = properties.get("saturation").and_then(Value::as_f64) {
            lore.push(format!("Food saturation: {}", format_stat(value)));
        }
    }
    let always_edible = text.contains("alwaysEat()") || text.contains("setAlwaysEdible") || text.contains("func_77848_i");
    if always_edible {
        properties.insert("always_edible".to_owned(), serde_json::json!(true));
        lore.push("Always edible: true".to_owned());
    }
    if text.contains(".meat()") {
        properties.insert("is_meat".to_owned(), serde_json::json!(true));
    }

    let no_repair = text.contains("setNoRepair") || text.contains("setNoRepair()");
    if no_repair {
        properties.insert("repairable".to_owned(), serde_json::json!(false));
        lore.push("Repairable: false".to_owned());
    }

    let ranged_animation = if text.contains("UseAnim.BOW") || text.contains("EnumAction.bow") {
        Some("bow")
    } else if text.contains("UseAnim.CROSSBOW") {
        Some("crossbow")
    } else if text.contains("UseAnim.SPEAR") {
        Some("spear")
    } else {
        None
    };
    if let Some(animation) = ranged_animation {
        properties.insert("ranged_animation".to_owned(), serde_json::json!(animation));
        properties.insert("enable_ranged".to_owned(), serde_json::json!(true));
        lore.push(format!("Ranged item: true ({animation})"));
        if let Some(duration) = numeric_source_call(text, &["getUseDuration"]) {
            properties.insert("use_duration".to_owned(), serde_json::json!(duration));
        }
        if let Some(projectile) = Regex::new(r"\bnew\s+([A-Za-z_$][A-Za-z0-9_$]*(?:Arrow|Projectile|Bolt|Shot))\s*\(").unwrap()
            .captures(text)
        {
            properties.insert("projectile_class".to_owned(), serde_json::json!(&projectile[1]));
        }
    } else {
        properties.insert("enable_ranged".to_owned(), serde_json::json!(false));
    }

    (Value::Object(properties), lore)
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

fn declared_texture_refs(source: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let png = Regex::new(r#"(?i)"([^"\n]{1,240}\.png)""#).unwrap();
    for capture in png.captures_iter(source) {
        refs.push(capture[1].replace('\\', "/"));
    }
    let single = Regex::new(r#"(?i)\b(?:setTextureName|func_111206_d|setBlockTextureName|func_149658_d|registerIcon|func_94245_a|registerBlockIcons)\s*\([^)]*?["']([^"']+)["']"#).unwrap();
    for capture in single.captures_iter(source) {
        refs.push(capture[1].replace('\\', "/"));
    }
    let location = Regex::new(r#"new\s+ResourceLocation\s*\(\s*"([^"]+)"\s*(?:,\s*"([^"]+)")?"#).unwrap();
    for capture in location.captures_iter(source) {
        refs.push(if let Some(path) = capture.get(2) {
            format!("{}:{}", &capture[1], path.as_str())
        } else {
            capture[1].to_owned()
        });
    }
    let declared = Regex::new(r#"(?i)\b(?:texture|textureName|icon|iconName|sprite|spriteName)\s*(?:=|:)\s*"([^"]+)""#).unwrap();
    for capture in declared.captures_iter(source) {
        refs.push(capture[1].replace('\\', "/"));
    }
    refs.sort_by_key(|value| value.to_ascii_lowercase());
    refs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    refs
}

fn equipment_texture_refs(source: &str) -> Vec<String> {
    declared_texture_refs(source)
}

fn block_texture_refs(source: &str) -> Vec<String> {
    declared_texture_refs(source)
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

fn asset_namespace(path: &Path) -> String {
    let components = path.components().filter_map(|component| match component {
        std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
        _ => None,
    }).collect::<Vec<_>>();
    components.windows(2)
        .find(|pair| pair[0].eq_ignore_ascii_case("assets"))
        .map(|pair| pair[1].clone())
        .unwrap_or_else(|| "minecraft".to_owned())
}

fn resolve_blockstate_model(files: &[PathBuf], state_path: &Path) -> Option<PathBuf> {
    let payload = fs::read(state_path).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())?;
    let mut references = Vec::<String>::new();
    if let Some(variants) = payload.get("variants").and_then(Value::as_object) {
        for variant in variants.values() {
            if let Some(reference) = variant.get("model").and_then(Value::as_str) {
                references.push(reference.to_owned());
            } else if let Some(array) = variant.as_array() {
                for choice in array {
                    if let Some(reference) = choice.get("model").and_then(Value::as_str) {
                        references.push(reference.to_owned());
                    }
                }
            }
        }
    }
    if let Some(parts) = payload.get("multipart").and_then(Value::as_array) {
        for part in parts {
            let apply = part.get("apply").unwrap_or(part);
            if let Some(reference) = apply.get("model").and_then(Value::as_str) {
                references.push(reference.to_owned());
            } else if let Some(array) = apply.as_array() {
                for choice in array {
                    if let Some(reference) = choice.get("model").and_then(Value::as_str) {
                        references.push(reference.to_owned());
                    }
                }
            }
        }
    }
    references.sort();
    references.dedup();
    let namespace = asset_namespace(state_path);
    for reference in references {
        let (model_namespace, model) = reference.split_once(':')
            .map(|(namespace, model)| (namespace, model))
            .unwrap_or((namespace.as_str(), reference.as_str()));
        let model = model.trim_start_matches('/').trim_start_matches("block/").trim_end_matches(".json");
        let suffix = format!("/assets/{model_namespace}/models/block/{model}.json").to_ascii_lowercase();
        if let Some(found) = files.iter().find(|path| {
            path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().ends_with(&suffix)
        }) {
            return Some(found.clone());
        }
    }
    None
}

fn find_block_model_json(files: &[PathBuf], registry: &str) -> (Option<PathBuf>, Option<String>) {
    let wanted = compact(registry);
    let mut state_files = files.iter().filter(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/blockstates/")
            && path.file_stem().and_then(|value| value.to_str())
                .is_some_and(|stem| compact(stem) == wanted)
    }).cloned().collect::<Vec<_>>();
    state_files.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    for state_path in state_files {
        if let Some(model_path) = resolve_blockstate_model(files, &state_path) {
            let shape = block_render_shape_from_model(&model_path);
            return (Some(model_path), Some(shape));
        }
    }

    let mut candidates = files.iter().filter(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/models/block/")
            && path.file_stem().and_then(|value| value.to_str())
                .is_some_and(|stem| compact(stem) == wanted)
    }).cloned().collect::<Vec<_>>();
    candidates.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    let model = candidates.into_iter().next();
    let shape = model.as_deref().map(block_render_shape_from_model);
    (model, shape)
}

fn block_render_shape_from_model(path: &Path) -> String {
    let payload = fs::read(path).ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    let parent = payload.get("parent").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();

    if ["block/cross", "block/tinted_cross", "block/crop", "block/flower"]
        .iter().any(|candidate| parent.ends_with(candidate))
        || ["flower", "sapling", "crop", "tall_grass", "fern", "cross"]
            .iter().any(|candidate| stem.contains(candidate))
    {
        return "plant".to_owned();
    }
    if ["block/torch", "block/template_torch", "block/template_torch_wall"]
        .iter().any(|candidate| parent.ends_with(candidate))
        || ["redstone_dust", "wire", "carpet", "rail", "repeater", "comparator", "pressure_plate"]
            .iter().any(|candidate| stem.contains(candidate))
    {
        return "floor".to_owned();
    }
    if parent.ends_with("block/cube") || parent.ends_with("block/cube_all")
        || parent.ends_with("block/cube_column") || parent.ends_with("block/orientable")
    {
        return "cube".to_owned();
    }
    let elements = payload.get("elements").and_then(Value::as_array);
    if let Some(elements) = elements.filter(|elements| !elements.is_empty()) {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for element in elements {
            if let Some(from) = element.get("from").and_then(Value::as_array) {
                if let Some(value) = from.get(1).and_then(Value::as_f64) { low = low.min(value); }
            }
            if let Some(to) = element.get("to").and_then(Value::as_array) {
                if let Some(value) = to.get(1).and_then(Value::as_f64) { high = high.max(value); }
            }
        }
        if low.is_finite() && high.is_finite() && high - low <= 1.25 {
            return "floor".to_owned();
        }
        return "custom".to_owned();
    }
    if parent.ends_with("block/cube") || parent.ends_with("block/cube_all")
        || parent.ends_with("block/cube_column") || parent.ends_with("block/orientable")
    {
        return "cube".to_owned();
    }
    "custom".to_owned()
}

fn block_render_shape(path: &Path, source: &mojmodelbridge::mcreator_block::BlockSource) -> String {
    let shape = block_render_shape_from_model(path);
    if shape == "custom" && is_plant_source(source) { "plant".to_owned() } else { shape }
}

fn append_paths(target: &mut Vec<PathBuf>, values: &[PathBuf]) {
    for value in values {
        if !target.contains(value) { target.push(value.clone()); }
    }
}

fn find_armor_layer_textures(files: &[PathBuf], prefix: &str, source: &str) -> Vec<PathBuf> {
    let normalized_prefix = compact(prefix.trim_end_matches("Armor").trim_end_matches("armor"));
    let mut result = Vec::<PathBuf>::new();
    for reference in declared_texture_refs(source) {
        if let Some(path) = find_texture_reference(files, std::slice::from_ref(&reference)) {
            if path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().contains("/armor/")
                && !result.contains(&path)
            {
                result.push(path);
            }
        }
    }

    let mut candidates = files.iter().filter(|path| {
        if !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png")) {
            return false;
        }
        let normalized = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        if !normalized.contains("/textures/models/armor/")
            && !normalized.contains("/textures/armor/")
        {
            return false;
        }
        let stem = compact(path.file_stem().and_then(|value| value.to_str()).unwrap_or(""));
        normalized_prefix.is_empty() || stem.contains(&normalized_prefix)
    }).cloned().collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        let file = path.file_name().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
        let layer = if file.contains("layer_1") || file.contains("layer1") { 0 }
            else if file.contains("layer_2") || file.contains("layer2") { 1 }
            else { 2 };
        (layer, path.to_string_lossy().to_ascii_lowercase())
    });
    append_paths(&mut result, &candidates);
    result
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

fn java_render_id(source: &str, method_name: &str) -> Option<String> {
    let method = Regex::new(&format!(r"(?s)\b{}\s*\([^)]*\)\s*\{{", regex::escape(method_name))).ok()?;
    let found = method.find(source)?;
    let body = method_body(source, found.start())?;
    let returned = Regex::new(r"\breturn\s+([A-Za-z_$][A-Za-z0-9_$.]*|\d+)\s*;").ok()?;
    let token = returned.captures(body)?.get(1)?.as_str().to_owned();
    if token.parse::<i64>().is_ok() {
        return Some(token);
    }
    let constant = Regex::new(&format!(
        r"(?m)\b{}\s*=\s*(-?\d+)\s*;",
        regex::escape(token.rsplit('.').next().unwrap_or(&token))
    )).ok()?;
    constant.captures(source).and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn find_linked_legacy_block_model(files: &[PathBuf], block_source: &str) -> (Option<PathBuf>, Vec<PathBuf>) {
    let model_pattern = Regex::new(r"\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(").unwrap();
    let mut candidates = Vec::<String>::new();

    // Some legacy blocks directly instantiate their model in the block class.
    if block_source.contains("getRenderType") || block_source.contains("renderBlock")
        || block_source.contains("BlockEntityRenderer") || block_source.contains("getModel")
    {
        candidates.push(block_source.to_owned());
    }

    let block_render_id = java_render_id(block_source, "getRenderType");
    for path in files.iter().filter(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java"))
    }) {
        let Ok(text) = fs::read_to_string(path) else { continue; };
        let is_legacy_handler = text.contains("renderWorldBlock")
            || text.contains("renderInventoryBlock")
            || text.contains("ISimpleBlockRenderingHandler")
            || text.contains("BlockEntityRenderer");
        if !is_legacy_handler { continue; }
        let renderer_id = java_render_id(&text, "getRenderId");
        if block_render_id.is_some() && renderer_id == block_render_id {
            candidates.push(text);
        } else if text.contains("BlockEntityRenderer")
            && (text.contains("BlockEntityType") || text.contains("BlockEntity"))
        {
            candidates.push(text);
        }
    }

    for text in candidates {
        let search_regions = ["renderWorldBlock", "renderInventoryBlock", "render", "getModel"]
            .iter()
            .filter_map(|method| {
                let pattern = Regex::new(&format!(r"\b{}\s*\([^)]*\)\s*\{{", regex::escape(method))).ok()?;
                let found = pattern.find(&text)?;
                method_body(&text, found.start()).map(str::to_owned)
            }).collect::<Vec<_>>();
        let mut model = None;
        for region in search_regions.iter().chain(std::iter::once(&text)) {
            if let Some(found) = model_pattern.captures(region).and_then(|capture| {
                find_model_source_by_class(files, &capture[1])
            }) {
                model = Some(found);
                break;
            }
        }
        if let Some(model) = model {
            let textures = declared_texture_refs(&text).into_iter()
                .filter_map(|reference| find_texture_reference(files, std::slice::from_ref(&reference)))
                .collect::<Vec<_>>();
            return (Some(model), textures);
        }
    }
    (None, Vec::new())
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

fn armor_model_owner_key(path: &Path) -> String {
    let mut stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("").to_owned();
    if stem.to_ascii_lowercase().starts_with("item") {
        stem = stem[4..].to_owned();
    }
    for suffix in ["helmet", "chestplate", "leggings", "boots", "armor"] {
        if stem.to_ascii_lowercase().ends_with(suffix) {
            stem.truncate(stem.len() - suffix.len());
            break;
        }
    }
    compact(&stem)
}

fn method_body(text: &str, method_start: usize) -> Option<&str> {
    let open = text[method_start..].find('{')? + method_start;
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut quote = 0_u8;
    let mut escaped = false;
    for index in open..bytes.len() {
        let ch = bytes[index];
        if quote != 0 {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == quote {
                quote = 0;
            }
            continue;
        }
        if ch == b'"' || ch == b'\'' {
            quote = ch;
        } else if ch == b'{' {
            depth += 1;
        } else if ch == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(&text[open + 1..index]);
            }
        }
    }
    None
}

fn find_linked_armor_model(files: &[PathBuf], source: &str, prefix: &str) -> Option<PathBuf> {
    let target = compact(prefix);
    let method_pattern = Regex::new(r"\bgetArmorModel\s*\([^)]*\)\s*\{").unwrap();
    let model_pattern = Regex::new(r"\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(").unwrap();
    let mut candidates = Vec::<String>::new();
    if !source.is_empty() {
        candidates.push(source.to_owned());
    }
    for path in files.iter().filter(|path| {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java"))
    }) {
        if armor_model_owner_key(path) != target { continue; }
        let Ok(text) = fs::read_to_string(path) else { continue; };
        if text.contains("getArmorModel") {
            candidates.push(text);
        }
    }

    for text in candidates {
        let Some(method) = method_pattern.find(&text) else { continue; };
        let body = method_body(&text, method.start()).unwrap_or("");
        let model = model_pattern.captures(body)
            .or_else(|| model_pattern.captures(&text));
        if let Some(model) = model {
            if let Some(path) = find_model_source_by_class(files, &model[1]) {
                return Some(path);
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
    fn discovers_registered_custom_item_renderer_model_and_texture() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/main/java/demo")).unwrap();
        fs::create_dir_all(root.join("src/main/resources/assets/demo/textures/items")).unwrap();
        fs::write(root.join("src/main/java/demo/ModItems.java"), r#"
            class ModItems {
                static Item BIG_BLADE = new Item();
                void register() { GameRegistry.registerItem(BIG_BLADE, "big_blade"); }
            }
        "#).unwrap();
        fs::write(root.join("src/main/java/demo/ClientProxy.java"), r#"
            class ClientProxy {
                void register() { MinecraftForgeClient.registerItemRenderer(ModItems.BIG_BLADE, new BigBladeRenderer()); }
            }
        "#).unwrap();
        fs::write(root.join("src/main/java/demo/BigBladeRenderer.java"), r#"
            class BigBladeRenderer implements IItemRenderer {
                ModelBigBlade model = new ModelBigBlade();
                String texture = "demo:textures/items/big_blade.png";
            }
        "#).unwrap();
        fs::write(root.join("src/main/java/demo/ModelBigBlade.java"), "class ModelBigBlade {}").unwrap();
        let skin = root.join("src/main/resources/assets/demo/textures/items/big_blade.png");
        fs::write(&skin, b"png").unwrap();

        let files = vec![
            root.join("src/main/java/demo/ModItems.java"),
            root.join("src/main/java/demo/ClientProxy.java"),
            root.join("src/main/java/demo/BigBladeRenderer.java"),
            root.join("src/main/java/demo/ModelBigBlade.java"),
            skin.clone(),
        ];
        let (model, textures) = find_held_renderer_model(&files, "big_blade");
        assert_eq!(model.and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().to_string())).as_deref(), Some("ModelBigBlade"));
        assert!(textures.contains(&skin));
    }

    #[test]
    fn discovers_armor_model_and_both_armor_layer_textures() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let armor_source = root.join("ItemDragonArmor.java");
        fs::write(&armor_source, r#"
            class ItemDragonArmor {
                ModelDragonArmor model = new ModelDragonArmor();
                ModelBiped getArmorModel() { return model; }
            }
        "#).unwrap();
        let model_path = root.join("ModelDragonArmor.java");
        fs::write(&model_path, "class ModelDragonArmor {}").unwrap();
        let texture_dir = root.join("assets/demo/textures/models/armor");
        fs::create_dir_all(&texture_dir).unwrap();
        let layer_one = texture_dir.join("dragon_layer_1.png");
        let layer_two = texture_dir.join("dragon_layer_2.png");
        fs::write(&layer_one, b"png").unwrap();
        fs::write(&layer_two, b"png").unwrap();
        let files = vec![armor_source, model_path.clone(), layer_one.clone(), layer_two.clone()];

        let model = find_linked_armor_model(&files, "", "Dragon");
        assert_eq!(model, Some(model_path));
        let textures = find_armor_layer_textures(&files, "Dragon", "");
        assert_eq!(textures, vec![layer_one, layer_two]);
    }

    #[test]
    fn recovers_legacy_and_modern_food_properties() {
        let legacy = mojmodelbridge::mcreator_equipment::EquipmentSource {
            class_name: "ItemRoastBeast".to_owned(),
            superclass: "ItemFood".to_owned(),
            registry_name: "roast_beast".to_owned(),
            element_type: "item".to_owned(),
            tool_type: None,
            source_path: String::new(),
            source_text: "public ItemRoastBeast() { super(8, 0.8F, true); setAlwaysEdible(); }".to_owned(),
        };
        let (legacy_properties, _) = item_source_properties(&legacy, None);
        assert_eq!(legacy_properties["nutrition"], 8);
        assert_eq!(legacy_properties["saturation"], 0.8);
        assert_eq!(legacy_properties["is_meat"], true);
        assert_eq!(legacy_properties["always_edible"], true);

        let modern = mojmodelbridge::mcreator_equipment::EquipmentSource {
            class_name: "ItemBerryPie".to_owned(),
            superclass: "Item".to_owned(),
            registry_name: "berry_pie".to_owned(),
            element_type: "item".to_owned(),
            tool_type: None,
            source_path: String::new(),
            source_text: "new Item.Properties().food(new FoodProperties.Builder().nutrition(6).saturationMod(0.7F).alwaysEat().meat().build())".to_owned(),
        };
        let (modern_properties, _) = item_source_properties(&modern, None);
        assert_eq!(modern_properties["nutrition"], 6);
        assert_eq!(modern_properties["saturation"], 0.7);
        assert_eq!(modern_properties["always_edible"], true);
        assert_eq!(modern_properties["is_meat"], true);
    }

    #[test]
    fn extracts_legacy_item_method_return_properties() {
        let source = mojmodelbridge::mcreator_equipment::EquipmentSource {
            class_name: "ItemAncientBlade".to_owned(),
            superclass: "ItemSword".to_owned(),
            registry_name: "ancient_blade".to_owned(),
            element_type: "tool".to_owned(),
            tool_type: Some("Sword".to_owned()),
            source_path: String::new(),
            source_text: r#"
                public int getItemEnchantability() { return 15; }
                public int getMaxDamage() { return 1800; }
                public int getItemStackLimit() { return 1; }
                public float getDamageVsEntity() { return 13.5F; }
            "#.to_owned(),
        };
        let (properties, lore) = item_source_properties(&source);
        assert_eq!(properties["enchantability"], 15.0);
        assert_eq!(properties["max_damage"], 1800.0);
        assert_eq!(properties["max_stack_size"], 1.0);
        assert_eq!(properties["attack_damage"], 13.5);
        assert!(lore.iter().any(|line| line == "Enchantability: 15"));
        assert!(lore.iter().any(|line| line == "Durability: 1800"));
    }

    #[test]
    fn reads_item_components_from_resource_item_definition() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("assets/demo/items/ancient_sword.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"components":{"minecraft:custom_name":{"text":"Ancient Sword","color":"gold"},"minecraft:lore":[{"text":"Old magic","italic":true}]}}"#).unwrap();
        let components = find_item_components(&[path], "ancient_sword").unwrap();
        assert_eq!(components["minecraft:custom_name"]["text"], "Ancient Sword");
        assert_eq!(components["minecraft:lore"][0]["text"], "Old magic");
    }

    #[test]
    fn classifies_weapons_from_registered_type_and_superclass_evidence() {
        let mut source = mojmodelbridge::mcreator_equipment::EquipmentSource {
            class_name: "CustomBlade".to_owned(),
            registry_name: "ancient_blade".to_owned(),
            superclass: "net.minecraft.item.ItemSword".to_owned(),
            element_type: "item".to_owned(),
            tool_type: None,
            source_path: String::new(),
            source_text: String::new(),
        };
        assert!(is_weapon_source(&source));

        source.superclass = "net.minecraft.item.ItemAxe".to_owned();
        source.class_name = "CustomAxe".to_owned();
        source.registry_name = "iron_axe".to_owned();
        source.element_type = "tool".to_owned();
        assert!(!is_weapon_source(&source));

        source.class_name = "ItemBattleAxe".to_owned();
        source.registry_name = "battle_axe".to_owned();
        assert!(is_weapon_source(&source));
    }

    #[test]
    fn blockstate_variant_resolves_actual_in_game_model_and_render_shape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let state = root.join("assets/demo/blockstates/living_flower.json");
        let model = root.join("assets/demo/models/block/flower_cross.json");
        fs::create_dir_all(state.parent().unwrap()).unwrap();
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&state, r#"{"variants":{"":{"model":"demo:block/flower_cross"}}}"#).unwrap();
        fs::write(&model, r#"{"parent":"minecraft:block/cross","textures":{"cross":"demo:block/flower"}}"#).unwrap();
        let files = vec![state, model.clone()];

        let (resolved, shape) = find_block_model_json(&files, "living_flower");
        assert_eq!(resolved, Some(model.clone()));
        assert_eq!(shape.as_deref(), Some("plant"));
        assert_eq!(block_render_shape_from_model(&model), "plant");
    }

    #[test]
    fn links_legacy_block_render_id_to_renderer_model_and_texture() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/main/java/demo")).unwrap();
        fs::create_dir_all(root.join("assets/demo/textures/blocks")).unwrap();
        let block_source = root.join("src/main/java/demo/BlockPizza.java");
        let renderer_source = root.join("src/main/java/demo/PizzaBlockRenderer.java");
        let model_source = root.join("src/main/java/demo/ModelPizza.java");
        let texture = root.join("assets/demo/textures/blocks/pizza.png");
        fs::write(&block_source, "class BlockPizza extends Block { int getRenderType() { return 47; } }").unwrap();
        fs::write(&renderer_source, r#"
            class PizzaBlockRenderer implements ISimpleBlockRenderingHandler {
                int getRenderId() { return 47; }
                void renderWorldBlock() { ModelPizza model = new ModelPizza(); }
                String texture = "demo:textures/blocks/pizza.png";
            }
        "#).unwrap();
        fs::write(&model_source, "class ModelPizza {}").unwrap();
        fs::write(&texture, b"png").unwrap();
        let files = vec![block_source, renderer_source, model_source.clone(), texture.clone()];
        let (model, textures) = find_linked_legacy_block_model(
            &files,
            "class BlockPizza extends Block { int getRenderType() { return 47; } }",
        );
        assert_eq!(model, Some(model_source));
        assert_eq!(textures, vec![texture]);
    }

    #[test]
    fn texture_references_follow_source_resource_locations_not_file_stems() {
        let references = declared_texture_refs(r#"
            item.setTextureName("demo:items/ancient_blade");
            icon = iconRegister.registerIcon("demo:ancient_blade_icon");
            ResourceLocation texture = new ResourceLocation("demo", "textures/entity/ancient_blade.png");
            String other = "demo:textures/items/alternate.png";
        "#);
        assert!(references.iter().any(|value| value == "demo:items/ancient_blade"));
        assert!(references.iter().any(|value| value == "demo:textures/entity/ancient_blade.png"));
        assert!(references.iter().any(|value| value == "demo:textures/items/alternate.png"));

        let temp = tempfile::tempdir().unwrap();
        let item = temp.path().join("assets/demo/textures/items/ancient_blade.png");
        let entity = temp.path().join("assets/demo/textures/entity/ancient_blade.png");
        fs::create_dir_all(item.parent().unwrap()).unwrap();
        fs::create_dir_all(entity.parent().unwrap()).unwrap();
        fs::write(&item, b"png").unwrap();
        fs::write(&entity, b"png").unwrap();
        let files = vec![item.clone(), entity.clone()];
        assert_eq!(
            find_texture_reference(&files, &references),
            Some(item),
            "first explicit source reference should determine the item texture"
        );
        assert_eq!(
            find_texture_reference(&files, &["demo:textures/entity/ancient_blade.png".to_owned()]),
            Some(entity)
        );
    }

    #[test]
    fn mob_metadata_includes_attributes_ai_and_special_ability_boolean() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("EntityStormDrake.java");
        fs::write(&source, r#"
            public class EntityStormDrake extends EntityMob {
                public void performRangedAttack(EntityLivingBase target, float power) {}
                public void breatheLightning() {}
            }
        "#).unwrap();
        let entity = EntityMetadata {
            class_name: "EntityStormDrake".to_owned(),
            display_name: "Storm Drake".to_owned(),
            registry_name: "storm_drake".to_owned(),
            superclass: "EntityMob".to_owned(),
            width: 1.4,
            height: 2.8,
            health: 120.0,
            movement_speed: 0.32,
            attack_damage: 14.0,
            armor: 8.0,
            follow_range: 48.0,
            xp: 25,
            fire_immune: false,
            water_mob: false,
            flying_mob: true,
            ai_classes: vec!["EntityAIAttackMelee".to_owned()],
            warnings: Vec::new(),
        };
        let (lore, components) = mob_component_metadata(&entity, Some(&source));
        assert!(lore.iter().any(|line| line == "Health: 120"));
        assert!(lore.iter().any(|line| line == "Attack damage: 14"));
        assert!(lore.iter().any(|line| line == "Special abilities: true"));
        let json: Value = serde_json::from_str(&components).unwrap();
        assert_eq!(json["attributes"]["max_health"], 120.0);
        assert_eq!(json["attributes"]["attack_damage"], 14.0);
        assert_eq!(json["special_abilities"], true);
        assert_eq!(json["ai_goals"][0], "EntityAIAttackMelee");
    }

    #[test]
    fn humanizes_registry_and_model_names() {
        assert_eq!(humanize("red_ant"), "Red Ant");
        assert_eq!(humanize("FireBeetle"), "Fire Beetle");
        assert_eq!(humanize("ModelTriffid"), "Model Triffid");
    }
}
