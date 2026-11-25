use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct PlantExportRecord {
    pub class_name: String,
    pub element_name: String,
    pub registry_name: String,
    pub texture: String,
    pub source_resources: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlantWorkspaceExportReport {
    pub workspace_file: PathBuf,
    pub discovered: usize,
    pub exported: usize,
    pub skipped: usize,
    pub records: Vec<PlantExportRecord>,
    pub warnings: Vec<String>,
}

fn pascal(value: &str) -> String {
    let mut output = String::new();
    let mut upper = true;
    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() {
            upper = true;
        } else if upper {
            output.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            output.push(ch);
        }
    }
    if output.is_empty() { output = "ConvertedPlant".to_owned(); }
    if output.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        output.insert_str(0, "Plant");
    }
    output
}

fn display_name(registry: &str) -> String {
    registry.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|value| !value.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    let mut value = first.to_ascii_uppercase().to_string();
                    value.extend(chars.map(|ch| ch.to_ascii_lowercase()));
                    value
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn workspace_descriptor(root: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::<(i64, usize, PathBuf)>::new();
    for entry in fs::read_dir(root).with_context(|| format!("Reading workspace {}", root.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("mcreator") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
        if !value.get("workspaceSettings").is_some_and(Value::is_object)
            || !value.get("mod_elements").is_some_and(Value::is_array) {
            continue;
        }
        candidates.push((
            value.get("mcreatorVersion").and_then(Value::as_i64).unwrap_or(0),
            value.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            path,
        ));
    }
    candidates.sort_by_key(|(version, count, path)| (*version, *count, path.to_string_lossy().to_ascii_lowercase()));
    candidates.pop().map(|(_, _, path)| path)
        .ok_or_else(|| anyhow::anyhow!("No valid .mcreator workspace descriptor found in {}", root.display()))
}

fn plant_template(root: &Path) -> Result<Value> {
    let elements = root.join("elements");
    if elements.is_dir() {
        let mut paths: Vec<PathBuf> = fs::read_dir(elements)?.filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
            if value.get("_type").and_then(Value::as_str) == Some("plant")
                && value.get("definition").is_some_and(Value::is_object) {
                return Ok(value);
            }
        }
    }
    serde_json::from_str(include_str!("../../oldVer/mcreator_templates/PlantSchema.mod.json"))
        .context("Parsing embedded MCreator plant schema")
}

/// Build the native MCreator plant schema. Plants are cross-model content by
/// default; referenced block models are still copied for source-state fidelity.
pub fn build_plant_element(
    template: &Value,
    source: &crate::mcreator_block::BlockSource,
    model_info: &Value,
    texture: &str,
    imported_fv: i64,
) -> (Value, Vec<String>) {
    let mut definition = template.get("definition").and_then(Value::as_object).cloned().unwrap_or_default();
    let registry = &source.registry_name;
    let mut warnings = Vec::new();

    definition.insert("renderType".to_owned(), json!(12));
    definition.insert("texture".to_owned(), json!(texture));
    definition.insert("textureBottom".to_owned(), json!(""));
    definition.insert("customModelName".to_owned(), json!("Cross model"));
    definition.insert("itemTexture".to_owned(), json!(""));
    definition.insert("particleTexture".to_owned(), json!(""));
    definition.insert("name".to_owned(), json!(display_name(registry)));
    definition.insert("creativeTabs".to_owned(), json!(["DECORATIONS"]));
    definition.insert("hardness".to_owned(), json!(0.0));
    definition.insert("resistance".to_owned(), json!(0.0));
    definition.insert("luminance".to_owned(), json!(0));
    definition.insert("isSolid".to_owned(), json!(false));
    definition.insert("isWaterloggable".to_owned(), json!(false));
    definition.insert("hasBlockItem".to_owned(), json!(true));
    definition.insert("maxStackSize".to_owned(), json!(64));
    definition.insert("rarity".to_owned(), json!("COMMON"));
    definition.insert("soundOnStep".to_owned(), json!("PLANT"));
    definition.insert("generationType".to_owned(), json!("custom"));
    definition.insert("generateFeature".to_owned(), json!(true));
    definition.insert("frequencyOnChunks".to_owned(), json!(1));
    definition.insert("generateAtAnyHeight".to_owned(), json!(true));
    definition.insert("restrictionBiomes".to_owned(), Value::Array(Vec::new()));
    definition.insert("patchSize".to_owned(), json!(1));
    definition.insert("offsetType".to_owned(), json!("XZ"));

    if let Some(light) = model_info.get("luminance") {
        definition.insert("luminance".to_owned(), light.clone());
    }
    if source.source_text.contains("SoundType.WOOD") {
        definition.insert("soundOnStep".to_owned(), json!("WOOD"));
    } else if source.source_text.contains("SoundType.METAL") {
        definition.insert("soundOnStep".to_owned(), json!("METAL"));
    } else if source.source_text.contains("SoundType.GLASS") {
        definition.insert("soundOnStep".to_owned(), json!("GLASS"));
    }
    if source.source_text.contains("randomTicks") || source.source_text.contains("tickRandomly") {
        definition.insert("forceTicking".to_owned(), json!(true));
    }
    if source.source_text.contains("getDrops") || source.source_text.contains("getDrop") {
        warnings.push("Custom plant drops were detected but are not yet reconstructed into MCreator drop/loot fields.".to_owned());
    }
    if model_info.as_object().is_none_or(|object| object.is_empty()) {
        warnings.push("No plant block model was resolved; using the native cross-model plant schema.".to_owned());
    }
    warnings.push("Plant growth behavior, custom placement rules, and biome-specific growth conditions are not fully reconstructed.".to_owned());

    let version = template.get("_fv").and_then(Value::as_i64).unwrap_or(imported_fv).clamp(0, 89);
    (json!({"_fv":version,"_type":"plant","definition":definition}), warnings)
}

fn placeholder_png() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+iZ1sAAAAASUVORK5CYII="
    ).expect("embedded PNG is valid")
}

fn plant_texture_candidates(source_roots: &[PathBuf], source: &crate::mcreator_block::BlockSource) -> Vec<PathBuf> {
    let mut stems = BTreeSet::<String>::new();
    stems.insert(source.registry_name.to_ascii_lowercase());
    stems.insert(source.class_name.to_ascii_lowercase());
    for root in source_roots {
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if !entry.file_type().is_file()
                || !path.extension().and_then(|value| value.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("png")) {
                continue;
            }
            let full = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            if !full.contains("/textures/block/") && !full.contains("/textures/blocks/") {
                continue;
            }
            let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            if stems.contains(&stem) {
                stems.insert(stem.clone());
            }
        }
    }
    let mut paths = Vec::new();
    for root in source_roots {
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if !entry.file_type().is_file()
                || !path.extension().and_then(|value| value.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("png")) {
                continue;
            }
            let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            if stems.contains(&stem) {
                paths.push(path.to_path_buf());
            }
        }
    }
    paths.sort_by_key(|path| {
        let full = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        let preferred = if full.contains("/textures/block/") || full.contains("/textures/blocks/") { 0 } else { 1 };
        (preferred, full.len(), full)
    });
    let mut seen = BTreeSet::new();
    paths.retain(|path| {
        let key = fs::canonicalize(path).unwrap_or_else(|_| path.clone()).to_string_lossy().to_lowercase();
        seen.insert(key)
    });
    paths
}

fn unique_plant_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut candidate = if base.is_empty() { "ConvertedPlant".to_owned() } else { base.to_owned() };
    let mut suffix = 2usize;
    while !used.insert(candidate.to_ascii_lowercase()) {
        candidate = format!("{base}{suffix}");
        suffix += 1;
    }
    candidate
}

/// Export registered flower/bush/crop/sapling block subclasses as MCreator plant elements.
pub fn export_plants_to_workspace(
    source_roots: &[PathBuf],
    workspace_root: &Path,
    source_modid: &str,
    imported_fv: i64,
) -> Result<PlantWorkspaceExportReport> {
    let workspace_root = fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let workspace_file = workspace_descriptor(&workspace_root)?;
    let bytes = fs::read(&workspace_file)?;
    let mut workspace: Value = serde_json::from_slice(&bytes)?;
    let target_modid = workspace.get("workspaceSettings").and_then(|value| value.get("modid"))
        .and_then(Value::as_str).filter(|value| !value.trim().is_empty()).unwrap_or("converted_mod").to_owned();

    let discovered_all = crate::mcreator_block::discover_block_sources(source_roots)?;
    let discovered: Vec<_> = discovered_all.into_iter()
        .filter(crate::mcreator_block::is_plant_source)
        .collect();
    let template = plant_template(&workspace_root)?;
    let field_registry_map = crate::recipe_bridge::source_field_registered_names(source_roots, source_modid);
    let localization = crate::localization::LocalizationCatalog::load(source_roots, &field_registry_map);
    let mut report = PlantWorkspaceExportReport {
        workspace_file: workspace_file.clone(),
        discovered: discovered.len(),
        exported: 0,
        skipped: 0,
        records: Vec::new(),
        warnings: Vec::new(),
    };

    let mut existing = BTreeSet::<String>::new();
    let mut used_names = HashSet::<String>::new();
    for descriptor in workspace.get("mod_elements").and_then(Value::as_array).into_iter().flatten() {
        if descriptor.get("type").and_then(Value::as_str) == Some("plant") {
            existing.insert(descriptor.get("registry_name").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase());
        }
        if let Some(name) = descriptor.get("name").and_then(Value::as_str) {
            used_names.insert(name.to_ascii_lowercase());
        }
    }

    let output_elements = workspace_root.join("elements");
    let texture_root = workspace_root.join("src/main/resources/assets").join(&target_modid).join("textures");
    fs::create_dir_all(&output_elements)?;
    fs::create_dir_all(&texture_root)?;

    for source in discovered {
        let registry = source.registry_name.to_ascii_lowercase();
        if existing.contains(&registry) {
            report.skipped += 1;
            continue;
        }

        let copied = crate::block_model_bridge::copy_source_block_model_resources(
            source_roots, source_modid, &target_modid, &registry, &workspace_root
        )?;
        let model_info = crate::block_model_bridge::mcreator_block_model_info(source_roots, source_modid, &registry);
        let raw = model_info.get("texture").and_then(Value::as_str).unwrap_or("");
        let model_path = model_info.get("modelPath").and_then(Value::as_str).map(Path::new);
        let mut texture = if !raw.is_empty() {
            crate::block_model_bridge::copy_model_ref_texture_to_workspace(
                model_path, raw, &texture_root, "block", &registry
            )?
        } else {
            String::new()
        };

        if texture.is_empty() || texture.starts_with("minecraft:") {
            let candidates = plant_texture_candidates(source_roots, &source);
            if let Some(path) = candidates.first() {
                texture = path.file_stem().and_then(|value| value.to_str()).unwrap_or(&registry).to_owned();
                let target = texture_root.join("block").join(format!("{texture}.png"));
                if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
                fs::copy(path, &target).with_context(|| format!("Copying plant texture {} to {}", path.display(), target.display()))?;
                crate::source_bridge::copy_animation_metadata(path, &target)?;
            }
        }
        if texture.is_empty() || texture.starts_with("minecraft:") {
            texture = registry.clone();
            let target = texture_root.join("block").join(format!("{texture}.png"));
            if !target.exists() {
                if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
                fs::write(&target, placeholder_png())?;
            }
            report.warnings.push(format!("No source plant icon texture was resolved for {}; generated placeholder.", registry));
        }

        let class_base = source.class_name
            .strip_prefix("Block").or_else(|| source.class_name.strip_prefix("Plant"))
            .unwrap_or(&source.class_name);
        let element_name = unique_plant_name(&pascal(class_base), &mut used_names);
        let (mut element, warnings) = build_plant_element(&template, &source, &model_info, &texture, imported_fv);
        let display = localization.translated_name(&registry, &display_name(&registry));
        if let Some(definition) = element.get_mut("definition").and_then(Value::as_object_mut) {
            if definition.contains_key("name") {
                definition.insert("name".to_owned(), json!(display));
            }
            let information = localization.item_information(&source.source_text, &registry);
            if !information.is_empty() && definition.contains_key("specialInformation") {
                definition.insert("specialInformation".to_owned(), json!({"fixedValue": information}));
            }
        }
        let element_path = output_elements.join(format!("{element_name}.mod.json"));
        fs::write(&element_path, format!("{}\n", serde_json::to_string_pretty(&element)?))
            .with_context(|| format!("Writing plant element {}", element_path.display()))?;

        let mut metadata = BTreeSet::<String>::new();
        for path in &copied {
            metadata.insert(path.strip_prefix(&workspace_root).unwrap_or(path).to_string_lossy().replace('\\', "/"));
        }
        let texture_path = texture_root.join("block").join(format!("{texture}.png"));
        if texture_path.is_file() {
            metadata.insert(texture_path.strip_prefix(&workspace_root).unwrap_or(&texture_path).to_string_lossy().replace('\\', "/"));
            let sidecar = PathBuf::from(format!("{}.mcmeta", texture_path.display()));
            if sidecar.is_file() {
                metadata.insert(sidecar.strip_prefix(&workspace_root).unwrap_or(&sidecar).to_string_lossy().replace('\\', "/"));
            }
        }

        let descriptor = json!({
            "name":element_name,
            "type":"plant",
            "compiles":true,
            "locked_code":false,
            "registry_name":registry,
            "metadata":{"files":metadata.iter().cloned().collect::<Vec<_>>()},
            "path":format!("~/{}/Plants",source_modid.replace('-', "_")),
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":source_modid,
            "mojmodelbridge_source_class":source.class_name,
            "mojmodelbridge_source_path":source.source_path
        });
        workspace["mod_elements"].as_array_mut().expect("workspace mod_elements array").push(descriptor);

        if !workspace.get("language_map").is_some_and(Value::is_object) {
            workspace["language_map"] = json!({});
        }
        if !workspace["language_map"].get("en_us").is_some_and(Value::is_object) {
            workspace["language_map"]["en_us"] = json!({});
        }
        workspace["language_map"]["en_us"].as_object_mut().unwrap()
            .insert(format!("block.{target_modid}.{registry}"), json!(display.clone()));

        report.exported += 1;
        report.warnings.extend(warnings.clone());
        report.records.push(PlantExportRecord {
            class_name:source.class_name,
            element_name,
            registry_name:registry.clone(),
            texture,
            source_resources:metadata.into_iter().collect(),
            warnings,
        });
        existing.insert(registry);
    }

    fs::write(&workspace_file, format!("{}\n", serde_json::to_string_pretty(&workspace)?))
        .with_context(|| format!("Writing updated workspace {}", workspace_file.display()))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn builds_cross_model_plant_schema_with_growth_defaults() {
        let source = crate::mcreator_block::BlockSource {
            class_name:"RubyFlower".to_owned(),
            superclass:"BlockFlower".to_owned(),
            registry_name:"ruby_flower".to_owned(),
            source_path:"RubyFlower.java".to_owned(),
            source_text:String::new(),
        };
        let template:Value=serde_json::from_str(include_str!("../../oldVer/mcreator_templates/PlantSchema.mod.json")).unwrap();
        let info=json!({"renderType":12,"customModelName":"Cross model","texture":"demo:block/ruby_flower"});
        let (element,warnings)=build_plant_element(&template,&source,&info,"ruby_flower",89);
        assert_eq!(element["_type"],"plant");
        assert_eq!(element["definition"]["renderType"],12);
        assert_eq!(element["definition"]["customModelName"],"Cross model");
        assert_eq!(element["definition"]["creativeTabs"],json!(["DECORATIONS"]));
        assert_eq!(element["definition"]["generateFeature"],true);
        assert_eq!(element["definition"]["generateAtAnyHeight"],true);
        assert_eq!(element["definition"]["texture"],"ruby_flower");
        assert!(!warnings.is_empty());
    }

    #[test]
    fn exports_registered_flower_as_plant_element_with_texture() {
        let temp=tempdir().unwrap();
        let source=temp.path().join("source");
        let workspace=temp.path().join("workspace");
        let assets=source.join("assets/demo");
        fs::create_dir_all(assets.join("textures/block")).unwrap();
        fs::write(source.join("Registry.java"),r#"
            public class Registry {
                public static Block FLOWER = new RubyFlower();
                public static void register() { GameRegistry.registerBlock(FLOWER, "ruby_flower"); }
            }
            class RubyFlower extends BlockFlower { }
        "#).unwrap();
        fs::write(assets.join("textures/block/ruby_flower.png"),b"png").unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("test.mcreator"),serde_json::to_vec(&json!({
            "mcreatorVersion":202600,"workspaceSettings":{"modid":"converted"},"mod_elements":[]
        })).unwrap()).unwrap();

        let report=export_plants_to_workspace(&[source],&workspace,"demo",89).unwrap();
        assert_eq!(report.discovered,1);
        assert_eq!(report.exported,1);
        let element:Value=serde_json::from_slice(&fs::read(workspace.join("elements/RubyFlower.mod.json")).unwrap()).unwrap();
        assert_eq!(element["_type"],"plant");
        assert_eq!(element["definition"]["texture"],"ruby_flower");
        assert!(workspace.join("src/main/resources/assets/converted/textures/block/ruby_flower.png").is_file());
    }
}
