use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use rayon::prelude::*;

use crate::mcreator_entity::{build_living_entity_element, find_living_entity_template, write_living_entity_element};
use crate::mcreator_model::{
    export_parsed_model_sources, parse_model_source_with_args, McreatorAnimationRecord, ModelSourceExport,
};
use crate::mod_graph::{scan_mod_graph, EntityMetadata, ModGraph, RendererRegistration};
use crate::source_bridge::{copy_animation_metadata, copy_model_sources};
use crate::texture::TextureIndex;

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceEntityRecord {
    pub entity_class: String,
    pub renderer_class: String,
    pub model_class: String,
    pub element_name: String,
    pub registry_name: String,
    pub texture: String,
    pub model_path: String,
    pub animation_path: Option<String>,
    pub animations: Vec<McreatorAnimationRecord>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McreatorWorkspaceExportReport {
    pub workspace_file: PathBuf,
    pub exported_entities: usize,
    pub skipped_entities: usize,
    pub records: Vec<WorkspaceEntityRecord>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct CachedModelExport {
    class_name: String,
    export: ModelSourceExport,
}

fn workspace_file(root: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::<(i64, usize, PathBuf)>::new();
    for entry in fs::read_dir(root).with_context(|| format!("Reading workspace root {}", root.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("mcreator") { continue; }
        let Ok(text) = fs::read_to_string(&path) else { continue; };
        let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { continue; };
        if !value.get("workspaceSettings").map(Value::is_object).unwrap_or(false)
            || !value.get("mod_elements").map(Value::is_array).unwrap_or(false) {
            continue;
        }
        let version = value.get("mcreatorVersion").and_then(Value::as_i64).unwrap_or(0);
        let count = value.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        candidates.push((version, count, path));
    }
    candidates.into_iter().max_by_key(|(version, count, _)| (*version, *count))
        .map(|(_, _, path)| path)
        .ok_or_else(|| anyhow::anyhow!("No valid top-level .mcreator workspace file found in {}", root.display()))
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = value.chars().collect();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            if previous.is_ascii_lowercase() || previous.is_ascii_digit() {
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
    if out.is_empty() { "converted_entity".to_owned() } else { out }
}

fn pascal(value: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() { upper = true; continue; }
        if upper {
            out.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() { out = "ConvertedEntity".to_owned(); }
    if out.chars().next().map(|ch| ch.is_ascii_digit()).unwrap_or(false) { out.insert_str(0, "Entity"); }
    out
}

fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut value = base.to_owned();
    let mut number = 2usize;
    while !used.insert(value.to_ascii_lowercase()) {
        value = format!("{base}_{number}");
        number += 1;
    }
    value
}

fn model_source_path(graph: &ModGraph, model_class: &str) -> Option<PathBuf> {
    graph.java_files.get(model_class).cloned()
        .or_else(|| graph.class_files.get(model_class).cloned())
        .or_else(|| graph.java_files.iter().find(|(name, _)| name.eq_ignore_ascii_case(model_class)).map(|(_, path)| path.clone()))
        .or_else(|| graph.class_files.iter().find(|(name, _)| name.eq_ignore_ascii_case(model_class)).map(|(_, path)| path.clone()))
}

fn fallback_metadata(entity_class: &str, registry: &str) -> EntityMetadata {
    let name = entity_class.strip_prefix("Entity").or_else(|| entity_class.strip_prefix("Mob")).unwrap_or(entity_class);
    EntityMetadata {
        class_name: entity_class.to_owned(),
        display_name: pascal(name),
        registry_name: registry.to_owned(),
        superclass: String::new(),
        width: 0.6,
        height: 1.8,
        health: 10.0,
        movement_speed: 0.3,
        attack_damage: 3.0,
        armor: 0.0,
        follow_range: 16.0,
        xp: 0,
        fire_immune: false,
        water_mob: false,
        flying_mob: false,
        ai_classes: Vec::new(),
        warnings: vec!["Entity metadata was not recovered; conservative default attributes were used.".to_owned()],
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn texture_filename(path: &Path, fallback_registry: &str) -> String {
    let raw = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
    let stem = Path::new(raw).file_stem().and_then(|value| value.to_str()).unwrap_or(fallback_registry);
    let sanitized = slug(stem);
    if sanitized.is_empty() { format!("{fallback_registry}.png") } else { format!("{sanitized}.png") }
}

fn placeholder_png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+iZ1sAAAAASUVORK5CYII="
    ).expect("embedded one-pixel placeholder PNG is valid base64")
}


fn resolve_explicit_texture(reference_values: &[String], roots: &[PathBuf], source_modid: &str) -> Option<PathBuf> {
    for reference in reference_values {
        let mut raw = reference.trim().replace('\\', "/");
        if raw.is_empty() { continue; }
        if let Some(path) = Path::new(&raw).is_file().then(|| PathBuf::from(&raw)) {
            return Some(path);
        }
        if let Some((_, rest)) = raw.split_once(':') {
            if !rest.is_empty() {
                let rest = rest.to_owned();
                raw = rest;
            }
        }
        while raw.starts_with("./") { raw = raw[2..].to_owned(); }
        if let Some(rest) = raw.strip_prefix("assets/") {
            raw = rest.to_owned();
        }
        if let Some(rest) = raw.strip_prefix(&format!("{source_modid}/textures/")) {
            raw = rest.to_owned();
        } else if let Some(rest) = raw.strip_prefix("textures/") {
            raw = rest.to_owned();
        }
        let relative = PathBuf::from(&raw);
        for root in roots {
            let candidates = [
                root.join(&relative),
                root.join("textures").join(&relative),
                root.join("src/main/resources").join(&relative),
                root.join("src/main/resources/assets").join(source_modid).join("textures").join(&relative),
                root.join("assets").join(source_modid).join("textures").join(&relative),
            ];
            if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
                return fs::canonicalize(&path).ok().or(Some(path));
            }
        }
    }
    None
}

fn ensure_texture(
    texture_index: Option<&TextureIndex>,
    source_model: &Path,
    model_class: &str,
    texture_refs: &[String],
    roots: &[PathBuf],
    source_modid: &str,
    workspace_root: &Path,
    modid: &str,
    registry_name: &str,
    overwrite: bool,
) -> Result<(String, String, Vec<String>)> {
    let mut warnings = Vec::new();
    let explicit = resolve_explicit_texture(texture_refs, roots, source_modid);
    let resolved = if let Some(index) = texture_index {
        index.resolve_set(source_model, model_class, explicit.as_deref(), 16)?
    } else {
        Default::default()
    };
    let texture_source = explicit.or(resolved.primary);
    let textures_dir = workspace_root.join("src/main/resources/assets")
        .join(modid).join("textures/entities");
    fs::create_dir_all(&textures_dir)?;
    let texture_name = texture_source.as_deref()
        .map(|path| texture_filename(path, registry_name))
        .unwrap_or_else(|| format!("{registry_name}.png"));
    let mut output_name = texture_name.clone();
    let mut target = textures_dir.join(&output_name);

    if let Some(source) = texture_source.as_ref() {
        if target.exists() && fs::read(&target)? != fs::read(source)? {
            output_name = format!("{registry_name}_{}", texture_name);
            target = textures_dir.join(&output_name);
        }
        if overwrite || !target.exists() {
            fs::copy(source, &target)
                .with_context(|| format!("Copying model texture {} to {}", source.display(), target.display()))?;
            copy_animation_metadata(source, &target)?;
        }
    } else {
        if !target.exists() || overwrite {
            fs::write(&target, placeholder_png())
                .with_context(|| format!("Writing placeholder texture {}", target.display()))?;
        }
        warnings.push(format!("No body texture was resolved for {model_class}; generated placeholder {}.", output_name));
        if !resolved.evidence.is_empty() {
            warnings.extend(resolved.evidence.into_iter().map(|item| format!("Texture resolver: {item}")));
        }
    }

    let relative = relative_path(workspace_root, &target);
    let _ = roots;
    Ok((output_name, relative, warnings))
}

fn update_language(workspace: &mut Value, modid: &str, element_name: &str, registry: &str, display: &str) {
    let workspace_object = workspace.as_object_mut().expect("validated workspace object");
    let language_map = workspace_object.entry("language_map".to_owned())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !language_map.is_object() { *language_map = Value::Object(serde_json::Map::new()); }
    let languages = language_map.as_object_mut().expect("language map initialized");
    let english = languages.entry("en_us".to_owned()).or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !english.is_object() { *english = Value::Object(serde_json::Map::new()); }
    let english = english.as_object_mut().expect("English language map initialized");
    for key in [
        format!("entity.{modid}.{registry}"),
        format!("entity.{modid}.{registry}.name"),
        format!("entity.{modid}.{element_name}"),
        format!("entity.{modid}.{element_name}.name"),
    ] {
        english.insert(key, json!(display));
    }
}

/// Export all renderer-linked entities into an existing MCreator workspace.
/// The workspace is updated only after each entity's model, texture and element
/// files have been generated successfully.
pub fn export_living_entities_to_workspace(
    graph: &ModGraph,
    workspace_root: &Path,
    target_generator: &str,
    overwrite: bool,
) -> Result<McreatorWorkspaceExportReport> {
    export_living_entities_to_workspace_with_modid(
        graph, workspace_root, target_generator, overwrite, None
    )
}

/// Export renderer-linked entities while preserving an explicit source namespace.
/// The standalone exporter derives the namespace from the graph root; export-all
/// supplies the user-selected mod id because its source roots may have arbitrary names.
pub fn export_living_entities_to_workspace_with_modid(
    graph: &ModGraph,
    workspace_root: &Path,
    target_generator: &str,
    overwrite: bool,
    source_modid_override: Option<&str>,
) -> Result<McreatorWorkspaceExportReport> {
    let workspace_root = fs::canonicalize(workspace_root)
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let workspace_path = workspace_file(&workspace_root)?;
    let workspace_text = fs::read_to_string(&workspace_path)
        .with_context(|| format!("Reading {}", workspace_path.display()))?;
    let mut workspace: Value = serde_json::from_str(workspace_text.trim_start_matches('\u{feff}'))
        .with_context(|| format!("Parsing {}", workspace_path.display()))?;
    if !workspace.is_object() || !workspace.get("workspaceSettings").map(Value::is_object).unwrap_or(false)
        || !workspace.get("mod_elements").map(Value::is_array).unwrap_or(false) {
        bail!("Workspace must contain object workspaceSettings and array mod_elements.");
    }

    let settings = workspace.get("workspaceSettings").and_then(Value::as_object).unwrap();
    let modid = settings.get("modid").and_then(Value::as_str).filter(|value| !value.is_empty())
        .unwrap_or("converted_mod").to_owned();
    let package = settings.get("modElementsPackage").and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned).unwrap_or_else(|| format!("net.mojmodelbridge.{modid}"));
    let generator = settings.get("currentGenerator").and_then(Value::as_str)
        .filter(|value| matches!(*value, "neoforge-1.21.1" | "forge-1.20.1"))
        .unwrap_or(target_generator).to_owned();

    let source_modid = source_modid_override
        .map(str::to_owned)
        .unwrap_or_else(|| graph.root.file_name().and_then(|value| value.to_str())
            .map(slug).unwrap_or_else(|| "imported_mod".to_owned()));
    let template = find_living_entity_template(&workspace_root)?.map(|(_, value, _)| value);

    let mut roots = vec![graph.root.clone()];
    roots.extend(graph.source_roots.iter().cloned());
    roots.sort();
    roots.dedup();
    let field_registry_map = crate::recipe_bridge::source_field_registered_names(&roots, &source_modid);
    let localization = crate::localization::LocalizationCatalog::load(&roots, &field_registry_map);
    let shallow_roots = graph.source_roots.clone();
    let texture_index = TextureIndex::new(&roots, &shallow_roots).ok();

    let elements_dir = workspace_root.join("elements");
    let existing_elements: Vec<String> = workspace.get("mod_elements").and_then(Value::as_array)
        .into_iter().flatten()
        .filter_map(|value| value.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let mut used_names: HashSet<String> = existing_elements.into_iter()
        .map(|value| value.to_ascii_lowercase()).collect();
    let mut used_registries: HashSet<String> = workspace.get("mod_elements").and_then(Value::as_array)
        .into_iter().flatten()
        .filter_map(|value| value.get("registry_name").and_then(Value::as_str).map(|name| name.to_ascii_lowercase()))
        .collect();

    let mut model_cache = BTreeMap::<String, CachedModelExport>::new();

    // Discover unique model source + constructor-argument inputs up front and
    // parse them concurrently. The registration loop below still chooses final
    // generated class names and performs all workspace writes in deterministic
    // order, but the expensive Java/class geometry conversion is parallel.
    let mut model_parse_inputs = BTreeMap::<String, (PathBuf, Vec<Value>)>::new();
    for registration in &graph.registrations {
        if registration.entity_class.is_empty() || registration.model_class.is_empty() {
            continue;
        }
        if !overwrite {
            let already_exported = workspace.get("mod_elements").and_then(Value::as_array)
                .into_iter().flatten().any(|descriptor| {
                    descriptor.get("mojmodelbridge_generated").and_then(Value::as_bool) == Some(true)
                        && descriptor.get("mojmodelbridge_source_modid").and_then(Value::as_str) == Some(source_modid.as_str())
                        && descriptor.get("mojmodelbridge_source_entity_class").and_then(Value::as_str)
                            == Some(registration.entity_class.as_str())
                });
            if already_exported {
                continue;
            }
        }
        let Some(source_model) = model_source_path(graph, &registration.model_class) else { continue };
        let parse_key = format!(
            "{}|{}",
            source_model.to_string_lossy(),
            serde_json::to_string(&registration.model_args)?
        );
        model_parse_inputs.entry(parse_key)
            .or_insert_with(|| (source_model, registration.model_args.clone()));
    }
    let parsed_model_results: Vec<(String, Result<Arc<crate::model::ParsedModel>, String>)> =
        model_parse_inputs.par_iter().map(|(parse_key, (source_model, model_args))| {
            let parsed = parse_model_source_with_args(source_model, model_args)
                .map(Arc::new)
                .map_err(|error| format!("{error:#}"));
            (parse_key.clone(), parsed)
        }).collect();
    let parsed_models: BTreeMap<String, Result<Arc<crate::model::ParsedModel>, String>> =
        parsed_model_results.into_iter().collect();

    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let mut skipped = 0usize;

    for registration in &graph.registrations {
        let existing_generated = workspace.get("mod_elements").and_then(Value::as_array)
            .into_iter().flatten().enumerate().find_map(|(index, descriptor)| {
                let matches = descriptor.get("mojmodelbridge_generated").and_then(Value::as_bool) == Some(true)
                    && descriptor.get("mojmodelbridge_source_modid").and_then(Value::as_str) == Some(source_modid.as_str())
                    && descriptor.get("mojmodelbridge_source_entity_class").and_then(Value::as_str)
                        == Some(registration.entity_class.as_str());
                if !matches { return None; }
                Some((
                    index,
                    descriptor.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
                    descriptor.get("registry_name").and_then(Value::as_str).unwrap_or("").to_owned(),
                    descriptor.get("mojmodelbridge_model_class").and_then(Value::as_str).map(str::to_owned),
                ))
            });
        if existing_generated.is_some() && !overwrite {
            skipped += 1;
            warnings.push(format!(
                "{} was already exported by MojModelBridge; skipped it (use --overwrite to refresh it).",
                registration.entity_class
            ));
            continue;
        }
        if registration.entity_class.is_empty() || registration.model_class.is_empty() {
            skipped += 1;
            warnings.push("Skipped a renderer registration with missing entity/model class.".to_owned());
            continue;
        }
        let Some(source_model) = model_source_path(graph, &registration.model_class) else {
            skipped += 1;
            warnings.push(format!("No Java/class model source found for {}; leaving this entity out of the generated workspace.", registration.model_class));
            continue;
        };
        let raw_meta = graph.entities.get(&registration.entity_class)
            .cloned().unwrap_or_else(|| fallback_metadata(&registration.entity_class,
                graph.registry_names.get(&registration.entity_class).map(String::as_str).unwrap_or("")));
        let registry_base = slug(graph.registry_names.get(&registration.entity_class)
            .map(String::as_str).filter(|value| !value.is_empty())
            .unwrap_or(&raw_meta.registry_name));
        let desired_element = pascal(registration.entity_class
            .strip_prefix("Entity").or_else(|| registration.entity_class.strip_prefix("Mob"))
            .unwrap_or(&registration.entity_class));
        let (element_name, registry_name) = if let Some((_, name, registry, _)) = existing_generated.as_ref() {
            (name.clone(), registry.clone())
        } else {
            let mut registry_name = registry_base.clone();
            let mut element_name = desired_element.clone();
            if used_registries.contains(&registry_name.to_ascii_lowercase()) {
                registry_name = slug(&format!("{source_modid}_{registry_base}"));
            }
            if used_names.contains(&element_name.to_ascii_lowercase()) {
                element_name = format!("{}_{}", desired_element, pascal(&source_modid));
            }
            element_name = unique_name(&element_name, &mut used_names);
            if used_registries.contains(&registry_name.to_ascii_lowercase()) {
                registry_name = unique_name(&registry_name, &mut used_registries);
            } else {
                used_registries.insert(registry_name.to_ascii_lowercase());
            }
            (element_name, registry_name)
        };

        let mut requested_model_class = existing_generated.as_ref()
            .and_then(|entry| entry.3.clone())
            .unwrap_or_else(|| pascal(&registration.model_class));
        if existing_generated.as_ref().and_then(|entry| entry.3.as_ref()).is_none()
            && model_cache.values().any(|entry| entry.class_name.eq_ignore_ascii_case(&requested_model_class))
        {
            requested_model_class = pascal(&format!("{} {}", registration.model_class, element_name));
        }
        let source_key = format!("{}|{}|{}|{}", source_model.to_string_lossy(),
            serde_json::to_string(&registration.model_args)?, generator, requested_model_class);
        let cached = if let Some(cached) = model_cache.get(&source_key) {
            cached.clone()
        } else {
            let parse_key = format!(
                "{}|{}",
                source_model.to_string_lossy(),
                serde_json::to_string(&registration.model_args)?
            );
            let parsed_model = parsed_models.get(&parse_key)
                .ok_or_else(|| anyhow::anyhow!("Parallel model preparse did not index {}", source_model.display()))?
                .as_ref()
                .map_err(|error| anyhow::anyhow!(
                    "Could not parse model {}: {error}",
                    source_model.display()
                ))?;
            let generated = export_parsed_model_sources(
                parsed_model.as_ref(), &source_model, &workspace_root, &modid,
                Some(&requested_model_class), &generator, overwrite
            )?;
            let cache = CachedModelExport { class_name: generated.class_name.clone(), export: generated };
            model_cache.insert(source_key, cache.clone());
            cache
        };

        let mut entity_warnings = cached.export.warnings.clone();
        let (texture_name, texture_relative, texture_warnings) = ensure_texture(
            texture_index.as_ref(), &source_model, &registration.model_class,
            &registration.texture_refs, &roots, &source_modid, &workspace_root,
            &modid, &registry_name, overwrite
        )?;
        entity_warnings.extend(texture_warnings);

        let animation_files = cached.export.animation_path.iter().cloned().collect::<Vec<_>>();
        let written_sources = copy_model_sources(
            &workspace_root, &package, std::slice::from_ref(&cached.export.model_path), &animation_files
        )?;
        let mut metadata_files: Vec<String> = written_sources.values().cloned().collect();
        metadata_files.push(texture_relative);

        let display_name = localization.translated_name(&registry_name, &raw_meta.display_name);
        let mut element = build_living_entity_element(
            template.as_ref(), &raw_meta, registration, &cached.class_name,
            &texture_name, &cached.export.animations
        );
        if let Some(definition) = element.get_mut("definition").and_then(Value::as_object_mut) {
            if definition.contains_key("mobName") {
                definition.insert("mobName".to_owned(), json!(display_name));
            }
            if definition.contains_key("mobLabel") && definition.get("mobLabel").and_then(Value::as_str).unwrap_or("").is_empty() {
                definition.insert("mobLabel".to_owned(), json!(display_name));
            }
        }
        let element_path = write_living_entity_element(&elements_dir, &element_name, &element)?;
        metadata_files.push(relative_path(&workspace_root, &element_path));

        let descriptor = json!({
            "name":element_name,
            "type":"livingentity",
            "compiles":true,
            "locked_code":false,
            "registry_name":registry_name,
            "metadata":{"files":metadata_files},
            "path":format!("~/Converted {source_modid}/Mobs"),
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":source_modid,
            "mojmodelbridge_source_registry":registry_base,
            "mojmodelbridge_source_entity_class":registration.entity_class,
            "mojmodelbridge_model_class":cached.class_name
        });
        let descriptors = workspace.get_mut("mod_elements").and_then(Value::as_array_mut)
            .expect("validated mod_elements array");
        if let Some((index, _, _, _)) = existing_generated {
            if let Some(existing) = descriptors.get_mut(index) {
                *existing = descriptor;
            } else {
                descriptors.push(descriptor);
            }
        } else {
            descriptors.push(descriptor);
        }
        update_language(&mut workspace, &modid, &element_name, &registry_name, &display_name);

        records.push(WorkspaceEntityRecord {
            entity_class: registration.entity_class.clone(),
            renderer_class: registration.renderer_class.clone(),
            model_class: cached.class_name,
            element_name,
            registry_name,
            texture: texture_name,
            model_path: cached.export.model_path.to_string_lossy().to_string(),
            animation_path: cached.export.animation_path.map(|path| path.to_string_lossy().to_string()),
            animations: cached.export.animations,
            warnings: entity_warnings.clone(),
        });
        warnings.extend(entity_warnings);
    }

    fs::write(&workspace_path, format!("{}\n", serde_json::to_string_pretty(&workspace)?))
        .with_context(|| format!("Writing updated workspace {}", workspace_path.display()))?;

    Ok(McreatorWorkspaceExportReport {
        workspace_file: workspace_path,
        exported_entities: records.len(),
        skipped_entities: skipped,
        records,
        warnings,
    })
}

pub fn scan_and_export_living_entities(
    mod_root: &Path,
    source_roots: &[PathBuf],
    workspace_root: &Path,
    target_generator: &str,
    overwrite: bool,
) -> Result<McreatorWorkspaceExportReport> {
    let graph = scan_mod_graph(mod_root, source_roots)?;
    export_living_entities_to_workspace(&graph, workspace_root, target_generator, overwrite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_graph::{EntityMetadata, RendererRegistration};
    use tempfile::tempdir;

    #[test]
    fn exports_renderer_linked_entity_into_existing_workspace() {
        let root = tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let source_root = root.path().join("mod");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&source_root).unwrap();
        let model_source = source_root.join("ModelWing.java");
        fs::write(&model_source, r#"
            public class ModelWing {
                public static LayerDefinition createBodyLayer() {
                    MeshDefinition mesh = new MeshDefinition();
                    PartDefinition root = mesh.getRoot();
                    root.addOrReplaceChild("body",
                        CubeListBuilder.create().texOffs(0,0).addBox(-1,-1,-1,2,2,2),
                        PartPose.ZERO);
                    return LayerDefinition.create(mesh,64,32);
                }
            }
        "#).unwrap();
        fs::write(source_root.join("ModelWing.png"), b"png").unwrap();

        let settings = json!({"modid":"demo","currentGenerator":"neoforge-1.21.1","modElementsPackage":"net.demo"});
        let descriptor = json!({"name":"ExampleItem","type":"item","registry_name":"example_item",
            "metadata":{"files":[]}});
        fs::write(workspace.join("demo.mcreator"), serde_json::to_vec(&json!({
            "mcreatorVersion":202600,"workspaceSettings":settings,"mod_elements":[descriptor],
            "language_map":{"en_us":{}}
        })).unwrap()).unwrap();

        let registration = RendererRegistration {
            entity_class:"EntityWingBeast".into(),renderer_class:"RenderWingBeast".into(),
            model_class:"ModelWing".into(),model_args:Vec::new(),renderer_args:Vec::new(),
            shadow_size:0.5,visual_scale:1.0,texture_refs:vec!["ModelWing.png".into()],
            source_class:"RenderRegistry".into(),source_offset:0
        };
        let metadata = EntityMetadata {
            class_name:"EntityWingBeast".into(),display_name:"Wing Beast".into(),registry_name:"wing_beast".into(),
            superclass:"EntityMob".into(),width:0.8,height:1.4,health:25.0,movement_speed:0.3,
            attack_damage:4.0,armor:0.0,follow_range:24.0,xp:20,fire_immune:false,
            water_mob:false,flying_mob:true,ai_classes:vec!["EntityAIWander".into()],warnings:Vec::new()
        };
        let graph = ModGraph {
            root:source_root.clone(),source_roots:Vec::new(),
            class_files:BTreeMap::new(),
            java_files:BTreeMap::from([("ModelWing".into(),model_source)]),
            registrations:vec![registration],
            entities:BTreeMap::from([("EntityWingBeast".into(),metadata)]),
            registry_names:BTreeMap::new(),warnings:Vec::new()
        };
        let report = export_living_entities_to_workspace(&graph,&workspace,"neoforge-1.21.1",true).unwrap();
        assert_eq!(report.exported_entities,1);
        let updated:Value=serde_json::from_slice(&fs::read(workspace.join("demo.mcreator")).unwrap()).unwrap();
        assert_eq!(updated["mod_elements"].as_array().unwrap().len(),2);
        assert_eq!(updated["mod_elements"][1]["type"],"livingentity");
        assert!(workspace.join("elements/WingBeast.mod.json").is_file());
        assert!(workspace.join("src/main/resources/assets/demo/textures/entities/model_wing.png").is_file());
        assert!(workspace.join("src/main/java/net/demo/client/model/ModelWing.java").is_file());

        let skipped = export_living_entities_to_workspace(
            &graph, &workspace, "neoforge-1.21.1", false
        ).unwrap();
        assert_eq!(skipped.exported_entities, 0);
        assert_eq!(skipped.skipped_entities, 1);
        let after_skip: Value = serde_json::from_slice(&fs::read(workspace.join("demo.mcreator")).unwrap()).unwrap();
        assert_eq!(after_skip["mod_elements"].as_array().unwrap().len(), 2);

        let refreshed = export_living_entities_to_workspace(
            &graph, &workspace, "neoforge-1.21.1", true
        ).unwrap();
        assert_eq!(refreshed.exported_entities, 1);
        let after_refresh: Value = serde_json::from_slice(&fs::read(workspace.join("demo.mcreator")).unwrap()).unwrap();
        assert_eq!(after_refresh["mod_elements"].as_array().unwrap().len(), 2);
    }
}
