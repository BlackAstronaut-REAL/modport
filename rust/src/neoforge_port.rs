//! First source-driven NeoForge 1.21.1 conversion pass.
//!
//! Accepts source folders or archives, discovers registered item/block classes,
//! copies data/assets, materializes simple native registrations and resources,
//! and records the content that still needs behavioral adapters.

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use walkdir::WalkDir;

use crate::archive::extract_archive;
use crate::metadata::{infer_mod_metadata, ModMetadata};
use crate::mod_graph::{scan_mod_graph, EntityMetadata};
use crate::mcreator_armor::{discover_armor_sources, ArmorSetSource};
use crate::mcreator_block::{discover_block_sources, is_plant_source, source_block_properties, BlockSource};
use crate::mcreator_equipment::{discover_equipment_sources, EquipmentSource};
use crate::mcreator_projectile::{discover_projectile_sources, ProjectileSource};
use crate::neoforge_docs::{default_docs_root, DocSearchResult, NeoForgeDocsIndex};
use crate::neoforge_project::{create_neoforge_project, NeoForgeProjectReport, DEFAULT_NEOFORGE_VERSION};
use crate::resource_bridge::copy_source_resources;
use crate::scan::scan_project;

#[derive(Debug, Clone, Serialize)]
pub struct NativeItemPlan {
    pub registry_name: String,
    pub source_class: String,
    pub kind: String,
    pub stack_size: u32,
    pub source_path: String,
    pub generated_as: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeBlockPlan {
    pub registry_name: String,
    pub source_class: String,
    pub source_path: String,
    pub hardness: f64,
    pub resistance: f64,
    pub light_level: u32,
    pub plant_like: bool,
    pub generated_as: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeEntityPlan {
    pub registry_name: String,
    pub source_class: String,
    pub health: f64,
    pub width: f64,
    pub height: f64,
    pub ai_classes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeArmorPlan {
    pub source_prefix: String,
    pub piece_classes: Vec<String>,
    pub registry_names: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NeoForgePortReport {
    pub conversion_state: String,
    pub behavior_fully_reconstructed: bool,
    pub target_project: NeoForgeProjectReport,
    pub source_metadata: ModMetadata,
    pub source_mod_id: String,
    pub target_mod_id: String,
    pub source_roots: Vec<String>,
    pub source_file_counts: BTreeMap<String, usize>,
    pub items: Vec<NativeItemPlan>,
    pub blocks: Vec<NativeBlockPlan>,
    pub entities: Vec<NativeEntityPlan>,
    pub armor_sets: Vec<NativeArmorPlan>,
    pub projectiles: Vec<String>,
    pub resource_files_copied: usize,
    pub generated_java_files: Vec<String>,
    pub generated_resource_files: Vec<String>,
    pub detected_domains: Vec<String>,
    pub docs_guidance: BTreeMap<String, Vec<DocSearchResult>>,
    pub analysis_file: PathBuf,
    pub warnings: Vec<String>,
}

struct PreparedSources {
    roots: Vec<PathBuf>,
    // Keep extracted JARs alive until every source/bytecode pass has completed.
    _temporary_roots: Vec<TempDir>,
    warnings: Vec<String>,
}

struct ItemRecord {
    id: String,
    class_name: String,
    source_path: String,
    source_text: String,
    stack_size: u32,
    kind: String,
    warnings: Vec<String>,
}

struct BlockRecord {
    id: String,
    class_name: String,
    source_path: String,
    properties: Map<String, Value>,
    plant_like: bool,
    warnings: Vec<String>,
}

pub fn port_to_neoforge(
    input_roots: &[PathBuf],
    output_root: &Path,
    mod_id_override: Option<&str>,
    mod_name_override: Option<&str>,
    package_override: Option<&str>,
    neoforge_version: &str,
    docs_root: Option<&Path>,
    overwrite: bool,
) -> Result<NeoForgePortReport> {
    if input_roots.is_empty() {
        bail!("port-neoforge requires at least one source folder, JAR, ZIP, Java file, or class file");
    }

    let prepared = prepare_sources(input_roots)?;
    let roots = &prepared.roots;
    let scan = scan_project(roots)?;
    let source_metadata = scan.metadata.clone();
    let primary = roots.first().context("No prepared source root")?;
    let graph = scan_mod_graph(primary, &roots[1..])?;

    let equipment = discover_equipment_sources(roots)?;
    let block_sources = discover_block_sources(roots)?;
    let armor_sources = discover_armor_sources(roots)?;
    let projectile_sources = discover_projectile_sources(roots)?;

    let source_mod_id = if !source_metadata.modid.trim().is_empty() {
        source_metadata.modid.trim().to_owned()
    } else {
        infer_namespace(primary).unwrap_or_default()
    };
    let target_mod_id = mod_id_override
        .map(str::to_owned)
        .unwrap_or_else(|| normalize_mod_id(if source_mod_id.is_empty() {
            primary.file_name().and_then(|value| value.to_str()).unwrap_or("converted_mod")
        } else {
            &source_mod_id
        }));
    let target_mod_name = mod_name_override
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| (!source_metadata.name.trim().is_empty()).then(|| source_metadata.name.trim().to_owned()))
        .unwrap_or_else(|| display_name_from_id(&target_mod_id));
    let target_package = package_override
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("com.modport.{}", target_mod_id));

    // Resolve docs and source metadata before touching output. Invalid input
    // should not leave a half-created target project behind.
    let docs = NeoForgeDocsIndex::load(
        docs_root.map(Path::to_path_buf).unwrap_or_else(default_docs_root)
    )?;
    let detected_domains = detect_domains(roots, &scan, &graph, &equipment, &block_sources, &armor_sources, &projectile_sources);
    let mut docs_guidance = BTreeMap::new();
    for domain in &detected_domains {
        let guidance = docs.guide(domain, None, 8);
        if !guidance.is_empty() {
            docs_guidance.insert(domain.clone(), guidance);
        }
    }

    let target_project = create_neoforge_project(
        output_root, &target_mod_id, &target_mod_name, &target_package,
        neoforge_version, overwrite
    )?;

    let mut warnings = prepared.warnings.clone();
    warnings.extend(scan.notes.iter().cloned());
    warnings.extend(graph.warnings.iter().cloned());
    warnings.extend(target_project.warnings.iter().cloned());

    let resources_root = output_root.join("src/main/resources");
    let mut resource_files_copied = 0usize;
    for root in roots {
        if !root.is_dir() { continue; }
        let metadata = infer_mod_metadata(root);
        let source_namespace = if !metadata.modid.trim().is_empty() {
            metadata.modid.trim().to_owned()
        } else {
            source_mod_id.clone()
        };
        match copy_source_resources(
            root, &resources_root,
            (!source_namespace.is_empty()).then_some(source_namespace.as_str()),
            Some(&target_mod_id),
        ) {
            Ok(count) => resource_files_copied += count,
            Err(error) => warnings.push(format!("Resource import from {} failed: {error:#}", root.display())),
        }
    }

    let mut item_records = collect_items(&equipment, &armor_sources, &mut warnings);
    let mut block_records = collect_blocks(&block_sources, &mut warnings);
    deduplicate_items(&mut item_records, &mut warnings);
    deduplicate_blocks(&mut block_records, &mut warnings);

    let generated_java_files = write_native_registries(output_root, &target_package, &target_mod_id, &item_records, &block_records)?;
    let generated_resource_files = write_native_resources(&resources_root, &target_mod_id, &item_records, &block_records)?;

    let items: Vec<NativeItemPlan> = item_records.iter().map(|item| NativeItemPlan {
        registry_name: item.id.clone(),
        source_class: item.class_name.clone(),
        kind: item.kind.clone(),
        stack_size: item.stack_size,
        source_path: item.source_path.clone(),
        generated_as: "vanilla Item registration".to_owned(),
        warnings: item.warnings.clone(),
    }).collect();

    let blocks: Vec<NativeBlockPlan> = block_records.iter().map(|block| NativeBlockPlan {
        registry_name: block.id.clone(),
        source_class: block.class_name.clone(),
        source_path: block.source_path.clone(),
        hardness: block.properties.get("hardness").and_then(Value::as_f64).unwrap_or(1.0),
        resistance: block.properties.get("resistance").and_then(Value::as_f64).unwrap_or(1.0),
        light_level: block.properties.get("light_level").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 15.0) as u32,
        plant_like: block.plant_like,
        generated_as: "vanilla Block + BlockItem registration".to_owned(),
        warnings: block.warnings.clone(),
    }).collect();

    let entities: Vec<NativeEntityPlan> = graph.entities.values().map(|entity: &EntityMetadata| NativeEntityPlan {
        registry_name: normalized_id(&entity.registry_name),
        source_class: entity.class_name.clone(),
        health: entity.health,
        width: entity.width,
        height: entity.height,
        ai_classes: entity.ai_classes.clone(),
        warnings: vec![
            "Entity discovered and described, but custom entity class, attributes, AI and renderer are not emitted in this pass.".to_owned()
        ],
    }).collect();

    let armor_sets: Vec<NativeArmorPlan> = armor_sources.iter().map(|armor: &ArmorSetSource| {
        let mut piece_classes: Vec<String> = armor.fields.values().cloned().collect();
        piece_classes.sort();
        piece_classes.dedup();
        let mut registry_names: Vec<String> = armor.registered_piece_ids.values().map(|value| normalized_id(value)).collect();
        registry_names.sort();
        registry_names.dedup();
        NativeArmorPlan {
            source_prefix: armor.prefix.clone(),
            piece_classes,
            registry_names,
            warnings: vec![
                "Armor pieces are recorded for follow-up conversion; wearable ArmorMaterial and ArmorItem behavior is not yet emitted.".to_owned()
            ],
        }
    }).collect();

    let projectiles: Vec<String> = projectile_sources.iter().map(|source: &ProjectileSource| {
        format!("{} ({})", source.class_name, source.registry_name)
    }).collect();

    if items.is_empty() && blocks.is_empty() {
        warnings.push("No registered Java/bytecode item or block subclasses were recovered. The project contains copied resources and source analysis only.".to_owned());
    }
    if !entities.is_empty() {
        warnings.push(format!("{} entity class(es) were recognized but are not yet registered as native NeoForge entities.", entities.len()));
    }
    if !armor_sets.is_empty() {
        warnings.push(format!("{} armor set(s) were recognized but remain non-wearable until ArmorMaterial/ArmorItem generation is implemented.", armor_sets.len()));
    }
    if !projectiles.is_empty() {
        warnings.push(format!("{} projectile class(es) were recognized but their entity behavior and item bindings are not yet emitted.", projectiles.len()));
    }
    if block_records.iter().any(|block| block.plant_like) {
        warnings.push("Plant-like blocks are currently emitted as simple blocks; crop growth, placement and cross-model behavior remain pending.".to_owned());
    }
    warnings.push("Custom Java methods, event handlers, networking, capabilities, AI goals, block entities, and renderer/model behavior are not copied blindly; they require dedicated API-aware adapters.".to_owned());

    let source_file_counts = scan.root_scans.iter().flat_map(|root| root.file_counts.iter())
        .fold(BTreeMap::<String, usize>::new(), |mut counts, (extension, count)| {
            *counts.entry(extension.clone()).or_insert(0) += count;
            counts
        });
    let analysis_file = output_root.join("modport/port-analysis.json");
    let report = NeoForgePortReport {
        conversion_state: "partial_native_content_export".to_owned(),
        behavior_fully_reconstructed: false,
        target_project,
        source_metadata,
        source_mod_id: source_mod_id.clone(),
        target_mod_id: target_mod_id.clone(),
        source_roots: roots.iter().map(|root| root.to_string_lossy().to_string()).collect(),
        source_file_counts,
        items,
        blocks,
        entities,
        armor_sets,
        projectiles,
        resource_files_copied,
        generated_java_files,
        generated_resource_files,
        detected_domains,
        docs_guidance,
        analysis_file: analysis_file.clone(),
        warnings,
    };

    if let Some(parent) = analysis_file.parent() { fs::create_dir_all(parent)?; }
    fs::write(&analysis_file, format!("{}\n", serde_json::to_string_pretty(&report)?))
        .with_context(|| format!("writing conversion analysis {}", analysis_file.display()))?;

    let status = json!({
        "format_version": 1,
        "target": {
            "loader": "NeoForge",
            "minecraft_version": "1.21.1",
            "neoforge_version": neoforge_version,
            "java": 21
        },
        "source_mod_id": source_mod_id,
        "target_mod_id": target_mod_id,
        "display_name": target_mod_name,
        "package": target_package,
        "conversion_state": "partial_native_content_export",
        "behavior_fully_reconstructed": false,
        "analysis_file": "modport/port-analysis.json",
        "converted": {
            "items": report.items.len(),
            "blocks": report.blocks.len(),
            "resource_files": resource_files_copied
        },
        "recognized_but_pending": {
            "entities": report.entities.len(),
            "armor_sets": report.armor_sets.len(),
            "projectiles": report.projectiles.len()
        },
        "documentation_domains": report.detected_domains,
        "notes": [
            "Registered simple items and blocks are emitted as native NeoForge registrations.",
            "Original assets and data JSON are copied with the mod namespace remapped where possible.",
            "Custom source behavior not explicitly converted is listed as pending in port-analysis.json."
        ]
    });
    fs::write(output_root.join("modport/port-status.json"), format!("{}\n", serde_json::to_string_pretty(&status)?))?;

    Ok(report)
}

fn prepare_sources(inputs: &[PathBuf]) -> Result<PreparedSources> {
    let mut roots = Vec::new();
    let mut temporary_roots = Vec::new();
    let mut warnings = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        if input.is_dir() {
            roots.push(fs::canonicalize(input).unwrap_or_else(|_| input.clone()));
            continue;
        }
        if !input.is_file() { bail!("Source input does not exist: {}", input.display()); }
        let extension = input.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
        let temp = tempfile::tempdir().context("creating temporary source staging directory")?;
        let root = temp.path().join(format!("source_{index}"));
        match extension.as_str() {
            "jar" | "zip" => {
                extract_archive(input, &root).with_context(|| format!("extracting {}", input.display()))?;
            }
            "java" | "class" => {
                fs::create_dir_all(&root)?;
                fs::copy(input, root.join(input.file_name().context("source file has no filename")?))?;
                warnings.push(format!("Only the loose file {} was supplied; companion sources/resources may be missing.", input.display()));
            }
            _ => bail!("Unsupported source input {}; expected a directory, JAR, ZIP, Java or class file", input.display()),
        }
        roots.push(root);
        temporary_roots.push(temp);
    }
    Ok(PreparedSources { roots, _temporary_roots: temporary_roots, warnings })
}

fn infer_namespace(root: &Path) -> Option<String> {
    let assets = WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok())
        .find(|entry| entry.file_type().is_dir() && entry.file_name().to_string_lossy().eq_ignore_ascii_case("assets"))?;
    let mut namespaces: Vec<String> = fs::read_dir(assets.path()).ok()?.flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name != "minecraft" && name != "forge" && !name.is_empty())
        .collect();
    namespaces.sort();
    namespaces.into_iter().next()
}

fn normalize_mod_id(value: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
            separator = false;
        } else if !result.is_empty() && !separator {
            result.push('_');
            separator = true;
        }
        if result.len() >= 64 { break; }
    }
    while result.ends_with('_') { result.pop(); }
    if result.is_empty() { result = "converted_mod".to_owned(); }
    if !result.as_bytes()[0].is_ascii_lowercase() { result.insert_str(0, "mod_"); }
    if result.len() == 1 { result.push_str("_mod"); }
    result
}

fn normalized_id(value: &str) -> String {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    value.rsplit(':').next().unwrap_or(value).trim_matches('/').replace('-', "_").to_ascii_lowercase()
}

fn display_name_from_id(value: &str) -> String {
    value.split(['_', '-', '.', ' ']).filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            format!("{}{}", chars.next().unwrap_or_default().to_ascii_uppercase(), chars.as_str())
        }).collect::<Vec<_>>().join(" ")
}

fn compact(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn java_field_name(value: &str, prefix: &str) -> String {
    let id = normalized_id(value);
    let mut name = String::new();
    let mut upper = true;
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() {
            name.push(if upper { ch.to_ascii_uppercase() } else { ch.to_ascii_uppercase() });
            upper = false;
        } else { upper = true; }
    }
    if name.is_empty() || name.as_bytes()[0].is_ascii_digit() { name.insert_str(0, "N_"); }
    format!("{prefix}_{}", name)
}

fn collect_items(
    equipment: &[EquipmentSource],
    armor_sources: &[ArmorSetSource],
    warnings: &mut Vec<String>,
) -> Vec<ItemRecord> {
    let stack_regex = Regex::new(r"(?i)(?:setMaxStackSize|stacksTo)\s*\(\s*(\d+)").expect("static regex");
    let mut items = Vec::new();
    for source in equipment {
        let stack_size = stack_regex.captures(&source.source_text).and_then(|capture| capture.get(1))
            .and_then(|value| value.as_str().parse::<u32>().ok()).unwrap_or(64).clamp(1, 64);
        let mut record_warnings = Vec::new();
        if source.element_type == "tool" || source.tool_type.is_some() {
            record_warnings.push("Tool/weapon registry recovered, but its legacy combat stats, mining rules, durability and use behavior are not yet transferred.".to_owned());
        }
        record_warnings.push("Custom item class methods are not copied; this entry is emitted as a basic native Item.".to_owned());
        items.push(ItemRecord {
            id: normalized_id(&source.registry_name),
            class_name: source.class_name.clone(),
            source_path: source.source_path.clone(),
            source_text: source.source_text.clone(),
            stack_size,
            kind: source.tool_type.clone().unwrap_or_else(|| source.element_type.clone()),
            warnings: record_warnings,
        });
    }

    // Keep armor registry IDs visible and loadable as ordinary placeholder
    // items until the 1.21.1 ArmorMaterial and ArmorItem adapter is added.
    for armor in armor_sources {
        for (piece, class) in &armor.fields {
            let Some(registry) = armor.registered_piece_ids.get(class) else { continue };
            let id = normalized_id(registry);
            if id.is_empty() { continue; }
            items.push(ItemRecord {
                id,
                class_name: class.clone(),
                source_path: armor.source_paths.first().cloned().unwrap_or_default(),
                source_text: armor.source_text.clone(),
                stack_size: 1,
                kind: format!("armor_{piece}_placeholder"),
                warnings: vec![
                    "Registered armor item preserved as a basic Item placeholder; it is not wearable until native ArmorMaterial/ArmorItem generation is implemented.".to_owned()
                ],
            });
        }
    }
    for item in &items {
        if item.id.is_empty() {
            warnings.push(format!("Skipped {} because its registry ID was empty.", item.class_name));
        }
    }
    items.retain(|item| !item.id.is_empty());
    items
}

fn collect_blocks(sources: &[BlockSource], warnings: &mut Vec<String>) -> Vec<BlockRecord> {
    let mut blocks = Vec::new();
    for source in sources {
        let properties = source_block_properties(source);
        let plant_like = is_plant_source(source);
        let mut block_warnings = Vec::new();
        block_warnings.push("Custom block methods, state properties, interactions, drops and tick behavior are not copied; this entry uses a simple native Block.".to_owned());
        if plant_like {
            block_warnings.push("Plant-like source detected; growth, placement and cross-model rendering are not yet reconstructed.".to_owned());
        }
        let id = normalized_id(&source.registry_name);
        if id.is_empty() {
            warnings.push(format!("Skipped block {} because its registry ID was empty.", source.class_name));
            continue;
        }
        blocks.push(BlockRecord {
            id,
            class_name: source.class_name.clone(),
            source_path: source.source_path.clone(),
            properties,
            plant_like,
            warnings: block_warnings,
        });
    }
    blocks
}

fn deduplicate_items(items: &mut Vec<ItemRecord>, warnings: &mut Vec<String>) {
    items.sort_by(|a, b| a.id.cmp(&b.id).then(a.class_name.cmp(&b.class_name)));
    let mut seen = BTreeSet::new();
    items.retain(|item| {
        if seen.insert(item.id.clone()) { true }
        else {
            warnings.push(format!("Duplicate item registry ID '{}' was found; retained the first sorted source class.", item.id));
            false
        }
    });
}

fn deduplicate_blocks(blocks: &mut Vec<BlockRecord>, warnings: &mut Vec<String>) {
    blocks.sort_by(|a, b| a.id.cmp(&b.id).then(a.class_name.cmp(&b.class_name)));
    let mut seen = BTreeSet::new();
    blocks.retain(|block| {
        if seen.insert(block.id.clone()) { true }
        else {
            warnings.push(format!("Duplicate block registry ID '{}' was found; retained the first sorted source class.", block.id));
            false
        }
    });
}

fn write_native_registries(
    output_root: &Path,
    package: &str,
    mod_id: &str,
    items: &[ItemRecord],
    blocks: &[BlockRecord],
) -> Result<Vec<String>> {
    let java_root = output_root.join("src/main/java").join(package.replace('.', "/")).join("registry");
    fs::create_dir_all(&java_root)?;
    let blocks_text = generate_blocks_java(package, mod_id, blocks);
    let items_text = generate_items_java(package, mod_id, items, blocks);
    fs::write(java_root.join("ModBlocks.java"), blocks_text)?;
    fs::write(java_root.join("ModItems.java"), items_text)?;
    Ok(vec![
        format!("src/main/java/{}/registry/ModBlocks.java", package.replace('.', "/")),
        format!("src/main/java/{}/registry/ModItems.java", package.replace('.', "/")),
    ])
}

fn generate_blocks_java(package: &str, _mod_id: &str, blocks: &[BlockRecord]) -> String {
    let mut body = format!(r#"package {package}.registry;

import {package}.NeoForgeMod;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredRegister;

public final class ModBlocks {{
    public static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(NeoForgeMod.MOD_ID);
"#);
    for block in blocks {
        let field = java_field_name(&block.id, "BLOCK");
        let hardness = block.properties.get("hardness").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 3600000.0);
        let resistance = block.properties.get("resistance").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 3600000.0);
        let light = block.properties.get("light_level").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 15.0) as u32;
        let light_property = if light > 0 {
            format!(".lightLevel(state -> {light})")
        } else {
            String::new()
        };
        body.push_str(&format!(
            r#"    public static final DeferredBlock<Block> {field} = BLOCKS.registerSimpleBlock("{id}", BlockBehaviour.Properties.of().strength({hardness}F, {resistance}F){light_property});
"#,
            id = block.id
        ));
    }
    body.push_str(r#"
    private ModBlocks() {}

    public static void register(IEventBus modEventBus) {
        BLOCKS.register(modEventBus);
    }
}
"#);
    body
}
fn generate_items_java(package: &str, _mod_id: &str, items: &[ItemRecord], blocks: &[BlockRecord]) -> String {
    let mut body = format!(r#"package {package}.registry;

import {package}.NeoForgeMod;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

public final class ModItems {{
    public static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(NeoForgeMod.MOD_ID);
"#);
    for item in items {
        let field = java_field_name(&item.id, "ITEM");
        body.push_str(&format!(
            r#"    public static final DeferredItem<Item> {field} = ITEMS.registerSimpleItem("{id}", new Item.Properties().stacksTo({stack_size}));
"#,
            id = item.id,
            stack_size = item.stack_size
        ));
    }
    for block in blocks {
        let field = java_field_name(&block.id, "BLOCK_ITEM");
        let block_field = java_field_name(&block.id, "BLOCK");
        body.push_str(&format!(
            r#"    public static final DeferredItem<BlockItem> {field} = ITEMS.register("{id}", () -> new BlockItem(ModBlocks.{block_field}.get(), new Item.Properties()));
"#,
            id = block.id
        ));
    }
    body.push_str(r#"
    private ModItems() {}

    public static void register(IEventBus modEventBus) {
        ITEMS.register(modEventBus);
    }
}
"#);
    body
}
fn write_native_resources(
    resources_root: &Path,
    mod_id: &str,
    items: &[ItemRecord],
    blocks: &[BlockRecord],
) -> Result<Vec<String>> {
    let namespace_root = resources_root.join("assets").join(mod_id);
    let textures_root = namespace_root.join("textures");
    let mut written = Vec::new();

    let lang = namespace_root.join("lang/en_us.json");
    if let Some(parent) = lang.parent() { fs::create_dir_all(parent)?; }
    let mut localization = if lang.is_file() {
        serde_json::from_slice::<Value>(&fs::read(&lang)?).ok()
            .and_then(|value| value.as_object().cloned()).unwrap_or_default()
    } else { Map::new() };

    for item in items {
        let key = format!("item.{mod_id}.{}", item.id);
        localization.entry(key).or_insert_with(|| json!(display_name_from_id(&item.id)));
        let model_path = namespace_root.join(format!("models/item/{}.json", item.id));
        if !model_path.exists() {
            let texture = find_texture_reference(&textures_root, mod_id, &item.id, &item.class_name)
                .unwrap_or_else(|| "minecraft:item/barrier".to_owned());
            write_json_file(&model_path, &json!({
                "parent": "minecraft:item/generated",
                "textures": {"layer0": texture}
            }))?;
            written.push(relative_to(resources_root, &model_path));
        }
    }

    for block in blocks {
        let key = format!("block.{mod_id}.{}", block.id);
        localization.entry(key).or_insert_with(|| json!(display_name_from_id(&block.id)));

        let block_state = namespace_root.join(format!("blockstates/{}.json", block.id));
        if !block_state.exists() {
            write_json_file(&block_state, &json!({"variants":{"":{"model":format!("{mod_id}:block/{}", block.id)}}}))?;
            written.push(relative_to(resources_root, &block_state));
        }

        let block_model = namespace_root.join(format!("models/block/{}.json", block.id));
        if !block_model.exists() {
            let texture = find_texture_reference(&textures_root, mod_id, &block.id, &block.class_name)
                .unwrap_or_else(|| "minecraft:block/stone".to_owned());
            write_json_file(&block_model, &json!({
                "parent": "minecraft:block/cube_all",
                "textures": {"all": texture}
            }))?;
            written.push(relative_to(resources_root, &block_model));
        }

        let item_model = namespace_root.join(format!("models/item/{}.json", block.id));
        if !item_model.exists() {
            write_json_file(&item_model, &json!({"parent":format!("{mod_id}:block/{}", block.id)}))?;
            written.push(relative_to(resources_root, &item_model));
        }
    }

    write_json_file(&lang, &Value::Object(localization))?;
    written.push(relative_to(resources_root, &lang));
    written.sort();
    written.dedup();
    Ok(written)
}

fn find_texture_reference(textures_root: &Path, mod_id: &str, registry_id: &str, class_name: &str) -> Option<String> {
    if !textures_root.is_dir() { return None; }
    let wanted = BTreeSet::from([compact(registry_id), compact(class_name)]);
    let mut candidates: Vec<PathBuf> = WalkDir::new(textures_root).follow_links(false).into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("png")))
        .filter(|entry| wanted.contains(&compact(entry.path().file_stem().and_then(|value| value.to_str()).unwrap_or(""))))
        .map(|entry| entry.into_path()).collect();
    candidates.sort_by_key(|path| {
        let stem = compact(path.file_stem().and_then(|value| value.to_str()).unwrap_or(""));
        (stem != compact(registry_id), path.to_string_lossy().to_ascii_lowercase())
    });
    let path = candidates.first()?;
    let relative = path.strip_prefix(textures_root).ok()?.with_extension("");
    Some(format!("{mod_id}:{}", relative.to_string_lossy().replace('\\', "/")))
}
fn write_json_file(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(value)?))
        .with_context(|| format!("writing resource JSON {}", path.display()))
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn detect_domains(
    roots: &[PathBuf],
    scan: &crate::scan::ProjectScan,
    graph: &crate::mod_graph::ModGraph,
    items: &[EquipmentSource],
    blocks: &[BlockSource],
    armor: &[ArmorSetSource],
    projectiles: &[ProjectileSource],
) -> Vec<String> {
    let mut domains = BTreeSet::from(["project_setup".to_owned(), "registries".to_owned()]);
    if !items.is_empty() { domains.insert("items".to_owned()); }
    if !blocks.is_empty() { domains.insert("blocks".to_owned()); }
    if !armor.is_empty() { domains.insert("items".to_owned()); domains.insert("enchantments".to_owned()); }
    if !graph.entities.is_empty() || !projectiles.is_empty() { domains.insert("entities".to_owned()); }
    if !graph.registrations.is_empty() || !scan.merged_model_roots.is_empty() {
        domains.insert("models".to_owned());
        domains.insert("rendering".to_owned());
    }
    if !scan.merged_resource_roots.is_empty() { domains.insert("resources".to_owned()); }
    for root in roots {
        if !root.is_dir() { continue; }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if !entry.file_type().is_file() { continue; }
            let path = entry.path().to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            if path.contains("/recipes/") { domains.insert("recipes".to_owned()); }
            if path.contains("/loot_tables/") || path.contains("/loottables/") { domains.insert("loot_tables".to_owned()); }
            if path.contains("/worldgen/") || path.contains("/biomes/") || path.contains("/dimensions/") {
                domains.insert("worldgen".to_owned());
            }
            if path.contains("/network/") || path.contains("/packets/") || path.contains("/payloads/") {
                domains.insert("networking".to_owned());
            }
            if path.contains("/blockentities/") || path.contains("/block_entities/") {
                domains.insert("blockentities".to_owned());
            }
        }
    }
    domains.into_iter().collect()
}

fn normalized_id_field(id: &str) -> String {
    java_field_name(id, "FIELD")
}

fn write_dummy_keep() {}

fn display_name(_metadata: &ModMetadata) -> String { String::new() }

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::io::Write;

    #[test]
    fn ports_registered_items_blocks_and_resources_from_java_source() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("old_mod");
        let output = temp.path().join("converted");
        fs::create_dir_all(source.join("src/main/java/demo")).unwrap();
        fs::create_dir_all(source.join("src/main/resources/assets/oldmod/textures/item")).unwrap();
        fs::create_dir_all(source.join("src/main/resources/assets/oldmod/textures/block")).unwrap();
        fs::create_dir_all(source.join("src/main/resources/data/oldmod/recipes")).unwrap();
        fs::write(source.join("mcmod.info"), r#"{"modid":"oldmod","name":"Old Mod","version":"1.2"}"#).unwrap();
        fs::write(source.join("src/main/java/demo/ModItems.java"), r#"
            public class ModItems {
                public static final Item RUBY = new RubyItem();
                public static void register() { GameRegistry.registerItem(RUBY, "ruby"); }
            }
            public class RubyItem extends Item {
                public RubyItem() { setMaxStackSize(16); }
            }
        "#).unwrap();
        fs::write(source.join("src/main/java/demo/ModBlocks.java"), r#"
            public class ModBlocks {
                public static final Block RUBY_ORE = new RubyOre();
                public static void register() { GameRegistry.registerBlock(RUBY_ORE, "ruby_ore"); }
            }
            public class RubyOre extends Block {
                public RubyOre() { setHardness(3.0F); setResistance(5.0F); }
            }
        "#).unwrap();
        fs::write(source.join("src/main/resources/assets/oldmod/textures/item/ruby.png"), b"item png").unwrap();
        fs::write(source.join("src/main/resources/assets/oldmod/textures/block/ruby_ore.png"), b"block png").unwrap();
        fs::write(source.join("src/main/resources/data/oldmod/recipes/ruby.json"),
            br#"{"type":"minecraft:crafting_shaped","result":{"item":"oldmod:ruby"}}"#).unwrap();

        let report = port_to_neoforge(
            &[source], &output, None, None, None, DEFAULT_NEOFORGE_VERSION, None, false
        ).unwrap();

        assert_eq!(report.source_mod_id, "oldmod");
        assert_eq!(report.target_mod_id, "oldmod");
        assert_eq!(report.items.iter().map(|entry| entry.registry_name.as_str()).collect::<Vec<_>>(), vec!["ruby"]);
        assert_eq!(report.blocks.iter().map(|entry| entry.registry_name.as_str()).collect::<Vec<_>>(), vec!["ruby_ore"]);
        assert!(report.resource_files_copied >= 3);
        assert!(output.join("src/main/java/com/modport/oldmod/registry/ModItems.java").is_file());
        assert!(output.join("src/main/resources/assets/oldmod/models/item/ruby.json").is_file());
        assert!(output.join("src/main/resources/assets/oldmod/blockstates/ruby_ore.json").is_file());
        let items_java = fs::read_to_string(output.join("src/main/java/com/modport/oldmod/registry/ModItems.java")).unwrap();
        assert!(items_java.contains("registerSimpleItem(\"ruby\""));
        assert!(items_java.contains("new BlockItem"));
        let status: Value = serde_json::from_slice(&fs::read(output.join("modport/port-status.json")).unwrap()).unwrap();
        assert_eq!(status["behavior_fully_reconstructed"], false);
        assert_eq!(status["converted"]["items"], 1);
        assert_eq!(status["converted"]["blocks"], 1);
    }

    #[test]
    fn extracts_a_jar_then_imports_its_assets() {
        let temp = tempdir().unwrap();
        let jar = temp.path().join("source.jar");
        {
            let file = fs::File::create(&jar).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer.start_file("mcmod.info", zip::write::FileOptions::default()).unwrap();
            writer.write_all(br#"{"modid":"jar_mod","name":"Jar Mod"}"#).unwrap();
            writer.start_file("assets/jar_mod/textures/item/gem.png", zip::write::FileOptions::default()).unwrap();
            writer.write_all(b"png").unwrap();
            writer.finish().unwrap();
        }
        let output = temp.path().join("out");
        let report = port_to_neoforge(
            &[jar], &output, None, None, None, DEFAULT_NEOFORGE_VERSION, None, false
        ).unwrap();
        assert_eq!(report.source_mod_id, "jar_mod");
        assert!(output.join("src/main/resources/assets/jar_mod/textures/item/gem.png").is_file());
    }

    #[test]
    fn refuses_invalid_ids_without_creating_a_project() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let output = temp.path().join("out");
        assert!(port_to_neoforge(
            &[source], &output, Some("Bad-ID"), None, None,
            DEFAULT_NEOFORGE_VERSION, None, false
        ).is_err());
        assert!(!output.exists());
    }
}
