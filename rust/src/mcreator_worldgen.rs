use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct WorldgenElementRecord {
    pub element_name: String,
    pub registry_name: String,
    pub element_type: String,
    pub source_path: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldgenWorkspaceExportReport {
    pub workspace_file: PathBuf,
    pub biomes_discovered: usize,
    pub dimensions_discovered: usize,
    pub biomes_exported: usize,
    pub dimensions_exported: usize,
    pub skipped: usize,
    pub records: Vec<WorldgenElementRecord>,
    pub warnings: Vec<String>,
}

fn snake_name(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower) {
                output.push('_');
            }
        }
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else if !output.is_empty() && !output.ends_with('_') {
            output.push('_');
        }
    }
    let output = output.trim_matches('_').to_owned();
    if output.is_empty() { "converted_content".to_owned() } else { output }
}

fn pascal_name(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut words = Vec::<String>::new();
    let mut current = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if ch.is_ascii_uppercase() && !current.is_empty() {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower) {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    let mut result = String::new();
    for word in words {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.push(first.to_ascii_uppercase());
            result.extend(chars);
        }
    }
    if result.is_empty() {
        result = "ConvertedContent".to_owned();
    }
    if result.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        result.insert_str(0, "Content");
    }
    result
}

fn workspace_descriptor(root: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::<(i64, usize, PathBuf)>::new();
    for entry in fs::read_dir(root).with_context(|| format!("Reading workspace root {}", root.display()))? {
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

fn template_for(root: &Path, element_type: &str) -> Result<Value> {
    let elements = root.join("elements");
    if elements.is_dir() {
        let mut paths: Vec<PathBuf> = fs::read_dir(&elements)?.filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
            if value.get("_type").and_then(Value::as_str) == Some(element_type)
                && value.get("definition").is_some_and(Value::is_object) {
                return Ok(value);
            }
        }
    }
    let embedded = match element_type {
        "biome" => include_str!("../../oldVer/mcreator_templates/BiomeSchema.mod.json"),
        "dimension" => include_str!("../../oldVer/mcreator_templates/DimensionSchema.mod.json"),
        _ => anyhow::bail!("Unsupported worldgen element type {element_type}"),
    };
    serde_json::from_str(embedded).context("Parsing embedded MCreator worldgen schema")
}

fn merge_definition(template: &Value, updates: &Value, name: &str) -> Value {
    let mut definition = template.get("definition").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(updates) = updates.as_object() {
        for (key, value) in updates {
            definition.insert(key.clone(), value.clone());
        }
    }
    definition.insert("name".to_owned(), json!(name));
    Value::Object(definition)
}

fn unique_element_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut candidate = base.to_owned();
    let mut suffix = 2usize;
    while !used.insert(candidate.to_ascii_lowercase()) {
        candidate = format!("{base}{suffix}");
        suffix += 1;
    }
    candidate
}

fn source_modid_for_root(root: &Path, supplied: &str) -> String {
    if !supplied.trim().is_empty() {
        return supplied.trim().to_owned();
    }
    root.file_name().and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("converted_mod")
        .to_owned()
}

fn source_biomes(source_roots: &[PathBuf], source_modid: &str) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    for root in source_roots {
        for (registry, value) in crate::worldgen_bridge::resource_biome_definitions(root, source_modid) {
            result.entry(registry).or_insert(value);
        }
    }
    result
}

fn source_dimensions(
    source_roots: &[PathBuf],
    source_modid: &str,
) -> (BTreeMap<String, Value>, BTreeMap<String, Value>) {
    let mut dimensions = BTreeMap::new();
    let mut dimension_types = BTreeMap::new();
    for root in source_roots {
        let (root_dimensions, root_types) = crate::worldgen_bridge::resource_dimension_definitions(root, source_modid);
        for (registry, value) in root_dimensions {
            dimensions.entry(registry).or_insert(value);
        }
        for (registry, value) in root_types {
            dimension_types.entry(registry).or_insert(value);
        }
    }
    (dimensions, dimension_types)
}

fn dimension_type_for<'a>(
    payload: &Value,
    types: &'a BTreeMap<String, Value>,
    source_modid: &str,
    dimension_registry: &str,
) -> Option<&'a Value> {
    let referenced = payload.get("type")
        .and_then(Value::as_str)
        .map(|value| crate::worldgen_bridge::resource_namespaced_registry(&json!(value), source_modid))
        .filter(|value| !value.is_empty());
    referenced.as_ref().and_then(|key| types.get(key))
        .or_else(|| types.get(dimension_registry))
}

fn dimension_biome_fallback(payload: &Value, source_modid: &str, biomes: &BTreeMap<String, Value>) -> String {
    if let Some(entries) = payload.get("generator")
        .and_then(|value| value.get("biome_source"))
        .and_then(|value| value.get("biomes"))
        .and_then(Value::as_array) {
        for entry in entries {
            if let Some(value) = entry.get("biome") {
                let registry = crate::worldgen_bridge::resource_namespaced_registry(value, source_modid);
                if !registry.is_empty() {
                    return registry;
                }
            }
        }
    }
    biomes.keys().next().cloned().unwrap_or_default()
}

fn localization_entry(workspace: &mut Value, category: &str, modid: &str, registry: &str, element_name: &str, display: &str) {
    if !workspace.get("language_map").is_some_and(Value::is_object) {
        workspace["language_map"] = json!({});
    }
    if !workspace["language_map"].get("en_us").is_some_and(Value::is_object) {
        workspace["language_map"]["en_us"] = json!({});
    }
    let language = workspace["language_map"]["en_us"].as_object_mut().expect("language map initialized");
    language.insert(format!("{category}.{modid}.{registry}"), json!(display));
    language.insert(format!("{category}.{modid}.{element_name}"), json!(display));
}

fn display_name(registry: &str) -> String {
    pascal_name(registry)
}

fn write_worldgen_element(
    output_elements: &Path,
    element_name: &str,
    element_type: &str,
    version: i64,
    definition: Value,
) -> Result<()> {
    let element = json!({
        "_fv":version.clamp(0, 89),
        "_type":element_type,
        "definition":definition
    });
    fs::create_dir_all(output_elements)?;
    let path = output_elements.join(format!("{element_name}.mod.json"));
    fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&element)?))
        .with_context(|| format!("Writing worldgen element {}", path.display()))
}

fn append_descriptor(
    workspace: &mut Value,
    element_name: &str,
    element_type: &str,
    registry: &str,
    source_modid: &str,
) {
    let group = if element_type == "biome" { "Biomes" } else { "Dimensions" };
    workspace["mod_elements"].as_array_mut().expect("workspace mod_elements array").push(json!({
        "name":element_name,
        "type":element_type,
        "compiles":true,
        "locked_code":false,
        "registry_name":registry,
        "metadata":{"files":[]},
        "path":format!("~/{}/{}",source_modid.replace('-', "_"),group),
        "mojmodelbridge_generated":true,
        "mojmodelbridge_source_modid":source_modid,
        "mojmodelbridge_source_registry":registry,
        "mojmodelbridge_source_resource":true
    }));
}

/// Export resource-only biome, dimension and dimension-type JSON into native
/// MCreator elements in an existing workspace descriptor.
pub fn export_worldgen_to_workspace(
    source_roots: &[PathBuf],
    workspace_root: &Path,
    source_modid: &str,
    imported_fv: i64,
) -> Result<WorldgenWorkspaceExportReport> {
    let workspace_root = fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let workspace_file = workspace_descriptor(&workspace_root)?;
    let bytes = fs::read(&workspace_file).with_context(|| format!("Reading {}", workspace_file.display()))?;
    let mut workspace: Value = serde_json::from_slice(&bytes).context("Parsing MCreator workspace descriptor")?;
    let target_modid = workspace.get("workspaceSettings").and_then(|settings| settings.get("modid"))
        .and_then(Value::as_str).filter(|value| !value.trim().is_empty()).unwrap_or("converted_mod").to_owned();
    let import_modid = source_roots.first()
        .map(|root| source_modid_for_root(root, source_modid))
        .unwrap_or_else(|| source_modid_for_root(&workspace_root, source_modid));

    let biomes = source_biomes(source_roots, &import_modid);
    let (dimensions, dimension_types) = source_dimensions(source_roots, &import_modid);
    let biome_template = template_for(&workspace_root, "biome")?;
    let dimension_template = template_for(&workspace_root, "dimension")?;
    let mut report = WorldgenWorkspaceExportReport {
        workspace_file:workspace_file.clone(),
        biomes_discovered:biomes.len(),
        dimensions_discovered:dimensions.len(),
        biomes_exported:0,
        dimensions_exported:0,
        skipped:0,
        records:Vec::new(),
        warnings:Vec::new(),
    };

    let mut existing = BTreeSet::<(String,String)>::new();
    let mut used_names = HashSet::<String>::new();
    for item in workspace.get("mod_elements").and_then(Value::as_array).into_iter().flatten() {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        let registry = item.get("registry_name").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        if !kind.is_empty() && !registry.is_empty() {
            existing.insert((kind,registry));
        }
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            used_names.insert(name.to_ascii_lowercase());
        }
    }

    let output_elements = workspace_root.join("elements");
    for (registry,payload) in &biomes {
        if existing.contains(&("biome".to_owned(),registry.to_ascii_lowercase())) {
            report.skipped += 1;
            continue;
        }
        let updates = crate::worldgen_bridge::resource_biome_updates(payload);
        let name = display_name(registry);
        let definition = merge_definition(&biome_template,&updates,&name);
        let element_name = unique_element_name(&format!("{name}Biome"),&mut used_names);
        write_worldgen_element(
            &output_elements,&element_name,"biome",template_version(&biome_template,imported_fv),definition
        )?;
        append_descriptor(&mut workspace,&element_name,"biome",registry,&import_modid);
        localization_entry(&mut workspace,"biome",&target_modid,registry,&element_name,&name);
        report.biomes_exported += 1;
        report.records.push(WorldgenElementRecord {
            element_name,
            registry_name:registry.clone(),
            element_type:"biome".to_owned(),
            source_path:format!("data/{import_modid}/worldgen/biome/{registry}.json"),
            warnings:Vec::new(),
        });
        existing.insert(("biome".to_owned(),registry.to_ascii_lowercase()));
    }

    for (registry,payload) in &dimensions {
        if existing.contains(&("dimension".to_owned(),registry.to_ascii_lowercase())) {
            report.skipped += 1;
            continue;
        }
        let dtype = dimension_type_for(payload,&dimension_types,&import_modid,registry)
            .cloned().unwrap_or_else(|| Value::Object(Map::new()));
        if dtype.as_object().is_some_and(Map::is_empty) {
            report.warnings.push(format!("Dimension {registry} has no matching dimension_type JSON; conservative type defaults were used."));
        }
        let biome_fallback = dimension_biome_fallback(payload,&import_modid,&biomes);
        let updates = crate::worldgen_bridge::resource_dimension_updates(
            payload,&dtype,&biome_fallback,&import_modid
        );
        let name = display_name(registry);
        let definition = merge_definition(&dimension_template,&updates,&name);
        let element_name = unique_element_name(&format!("{name}Dimension"),&mut used_names);
        write_worldgen_element(
            &output_elements,&element_name,"dimension",template_version(&dimension_template,imported_fv),definition
        )?;
        append_descriptor(&mut workspace,&element_name,"dimension",registry,&import_modid);
        localization_entry(&mut workspace,"dimension",&target_modid,registry,&element_name,&name);
        report.dimensions_exported += 1;
        report.records.push(WorldgenElementRecord {
            element_name,
            registry_name:registry.clone(),
            element_type:"dimension".to_owned(),
            source_path:format!("data/{import_modid}/dimension/{registry}.json"),
            warnings:Vec::new(),
        });
        existing.insert(("dimension".to_owned(),registry.to_ascii_lowercase()));
    }

    fs::write(&workspace_file,format!("{}\n",serde_json::to_string_pretty(&workspace)?))
        .with_context(||format!("Writing workspace descriptor {}",workspace_file.display()))?;
    Ok(report)
}

fn template_version(template: &Value, fallback: i64) -> i64 {
    template.get("_fv").and_then(Value::as_i64).unwrap_or(fallback).clamp(0,89)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn exports_resource_biome_and_dimension_json_to_workspace_elements() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let workspace = temp.path().join("workspace");
        let biome = source.join("data/demo/worldgen/biome/ruby.json");
        let dimension = source.join("data/demo/dimension/ruby_realm.json");
        let dimension_type = source.join("data/demo/dimension_type/ruby_realm.json");
        for path in [&biome,&dimension,&dimension_type] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        fs::write(&biome,br#"{
            "temperature":0.8,"downfall":0.2,
            "effects":{"ambient_sound":"demo:ambient"},
            "spawners":{"monster":[{"type":"demo:beast","minCount":2,"maxCount":3,"weight":10}]}
        }"#).unwrap();
        fs::write(&dimension,br#"{
            "type":"demo:ruby_realm",
            "generator":{"biome_source":{"biomes":[{"biome":"demo:ruby"}]},
            "settings":{"sea_level":48,"ore_veins_enabled":false}}
        }"#).unwrap();
        fs::write(&dimension_type,br#"{
            "natural":true,"bed_works":false,"has_skylight":false,
            "ambient_light":0.2,"ultrawarm":true,"fixed_time":18000
        }"#).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("test.mcreator"),serde_json::to_vec(&json!({
            "mcreatorVersion":202600,"workspaceSettings":{"modid":"converted"},
            "mod_elements":[],"language_map":{"en_us":{}}
        })).unwrap()).unwrap();

        let report = export_worldgen_to_workspace(&[source],&workspace,"demo",89).unwrap();
        assert_eq!(report.biomes_discovered,1);
        assert_eq!(report.dimensions_discovered,1);
        assert_eq!(report.biomes_exported,1);
        assert_eq!(report.dimensions_exported,1);

        let biome_element:Value=serde_json::from_slice(&fs::read(workspace.join("elements/RubyBiome.mod.json")).unwrap()).unwrap();
        assert_eq!(biome_element["_type"],"biome");
        assert_eq!(biome_element["definition"]["temperature"],0.8);
        assert_eq!(biome_element["definition"]["spawnEntries"][0]["entity"],"demo:beast");

        let dimension_element:Value=serde_json::from_slice(&fs::read(workspace.join("elements/RubyRealmDimension.mod.json")).unwrap()).unwrap();
        assert_eq!(dimension_element["_type"],"dimension");
        assert_eq!(dimension_element["definition"]["biomesInDimension"],json!(["CUSTOM:Ruby"]));
        assert_eq!(dimension_element["definition"]["seaLevel"],48);
        assert_eq!(dimension_element["definition"]["generateOreVeins"],false);
        assert_eq!(dimension_element["definition"]["doesWaterVaporize"],true);

        let updated:Value=serde_json::from_slice(&fs::read(workspace.join("test.mcreator")).unwrap()).unwrap();
        assert_eq!(updated["mod_elements"].as_array().unwrap().len(),2);
        assert_eq!(updated["language_map"]["en_us"]["biome.converted.ruby"],"Ruby");
        assert_eq!(updated["language_map"]["en_us"]["dimension.converted.ruby_realm"],"Ruby Realm");
    }

    #[test]
    fn skips_existing_worldgen_elements_and_uses_template_fields() {
        let temp=tempdir().unwrap();
        let source=temp.path().join("source");
        let workspace=temp.path().join("workspace");
        let biome=source.join("data/demo/worldgen/biome/old.json");
        fs::create_dir_all(biome.parent().unwrap()).unwrap();
        fs::write(&biome,br#"{"temperature":0.6,"downfall":0.1,"effects":{},"spawners":{}}"#).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("test.mcreator"),serde_json::to_vec(&json!({
            "mcreatorVersion":202600,"workspaceSettings":{"modid":"converted"},
            "mod_elements":[{"name":"ExistingBiome","type":"biome","registry_name":"old"}]
        })).unwrap()).unwrap();

        let report=export_worldgen_to_workspace(&[source],&workspace,"demo",89).unwrap();
        assert_eq!(report.biomes_discovered,1);
        assert_eq!(report.biomes_exported,0);
        assert_eq!(report.skipped,1);
        assert!(!workspace.join("elements/OldBiome.mod.json").exists());
    }
}
