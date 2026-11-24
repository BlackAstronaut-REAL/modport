use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ValidationIssue {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub element: String,
    pub registry_name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceValidationReport {
    pub workspace: String,
    pub ok: bool,
    pub stats: BTreeMap<String, Value>,
    pub errors: Vec<ValidationIssue>,
    pub warnings: Vec<ValidationIssue>,
    pub info: Vec<ValidationIssue>,
}

impl WorkspaceValidationReport {
    fn new(workspace: impl Into<String>) -> Self {
        Self { workspace: workspace.into(), ok: true, stats: BTreeMap::new(), errors: Vec::new(), warnings: Vec::new(), info: Vec::new() }
    }

    fn add(&mut self, severity: &str, code: &str, message: impl Into<String>, element: impl Into<String>, registry: impl Into<String>, path: impl Into<String>) {
        let issue = ValidationIssue {
            severity: severity.to_owned(), code: code.to_owned(), message: message.into(),
            element: element.into(), registry_name: registry.into(), path: path.into(),
        };
        match severity { "error" => self.errors.push(issue), "warning" => self.warnings.push(issue), _ => self.info.push(issue) }
        self.ok = self.errors.is_empty();
    }

    fn error(&mut self, code: &str, message: impl Into<String>, element: impl Into<String>, registry: impl Into<String>, path: impl Into<String>) {
        self.add("error", code, message, element, registry, path);
    }

    fn warning(&mut self, code: &str, message: impl Into<String>, element: impl Into<String>, registry: impl Into<String>, path: impl Into<String>) {
        self.add("warning", code, message, element, registry, path);
    }
}

fn resolved_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| if path.is_absolute() { path.to_path_buf() } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
    })
}

fn read_json_object(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    value.as_object()?;
    Some(value)
}

fn python_string(value: Option<&Value>, default: &str) -> String {
    match value {
        None => default.to_owned(),
        Some(Value::Null) => "None".to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(v)) => if *v { "True" } else { "False" }.to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        Some(v) => v.to_string(),
    }
}

fn python_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}

fn python_int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Null => None,
        Value::Bool(v) => Some(if *v { 1 } else { 0 }),
        Value::Number(v) => v.as_i64().or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok())).or_else(|| v.as_f64().map(|n| n as i64)),
        Value::String(v) => v.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn object_value<'a>(value: &'a Value, key: &str) -> Option<&'a Map<String, Value>> {
    value.get(key).and_then(Value::as_object)
}

fn workspace_file(root: &Path) -> Option<PathBuf> {
    let mut selected: Option<(i64, usize, PathBuf)> = None;
    for entry in fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("mcreator") { continue; }
        let Some(data) = read_json_object(&path) else { continue };
        if object_value(&data, "workspaceSettings").is_none() { continue; }
        let version = python_int(data.get("mcreatorVersion")).unwrap_or(0);
        let count = data.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        let candidate = (version, count, path);
        if selected.as_ref().map(|current| (candidate.0, candidate.1) > (current.0, current.1)).unwrap_or(true) {
            selected = Some(candidate);
        }
    }
    selected.map(|v| v.2)
}

fn resource_candidates(root: &Path, modid: &str, definition: &Map<String, Value>) -> Vec<(String, PathBuf)> {
    let textures = root.join("src/main/resources/assets").join(modid).join("textures");
    let mappings: [(&str, &[&str]); 7] = [
        ("texture", &["item", "block"]), ("textureHelmet", &["item"]),
        ("textureBody", &["item"]), ("textureLeggings", &["item"]),
        ("textureBoots", &["item"]), ("customModelTexture", &["projectile", "entity", "entities", "item"]),
        ("mobModelTexture", &["entity", "entities"]),
    ];
    let mut result = Vec::new();
    for (key, directories) in mappings {
        let Some(Value::String(raw)) = definition.get(key) else { continue };
        if raw.trim().is_empty() || raw == "Normal" || raw == "Default" { continue; }
        let filename = if raw.to_ascii_lowercase().ends_with(".png") { raw.clone() } else { format!("{raw}.png") };
        for (index, directory) in directories.iter().enumerate() {
            let group = if index == 0 { key.to_owned() } else { format!("{key}|alternative") };
            result.push((group, textures.join(directory).join(&filename)));
        }
    }
    result
}

/// Native port of oldVer/workspace_validator.py.
pub fn validate_workspace(workspace_root: &Path, workspace_override: Option<Value>, write_reports: bool) -> Result<WorkspaceValidationReport> {
    let root = resolved_path(workspace_root);
    let selected_workspace = workspace_file(&root);
    let workspace = match workspace_override {
        Some(value) => value,
        None => {
            let Some(path) = selected_workspace.as_ref() else {
                let mut report = WorkspaceValidationReport::new(root.display().to_string());
                report.error("workspace_missing", "No readable .mcreator workspace definition was found.", "", "", root.display().to_string());
                return Ok(report);
            };
            let Some(value) = read_json_object(path) else {
                let mut report = WorkspaceValidationReport::new(path.display().to_string());
                report.error("workspace_invalid", "Workspace JSON is not an object.", "", "", "");
                return Ok(report);
            };
            value
        }
    };
    if !workspace.is_object() {
        let mut report = WorkspaceValidationReport::new(selected_workspace.as_ref().unwrap_or(&root).display().to_string());
        report.error("workspace_invalid", "Workspace JSON is not an object.", "", "", "");
        return Ok(report);
    }

    let report_workspace = selected_workspace.as_ref().unwrap_or(&root).display().to_string();
    let mut report = WorkspaceValidationReport::new(report_workspace);
    let modid_value = object_value(&workspace, "workspaceSettings").and_then(|v| v.get("modid"));
    let modid = modid_value.filter(|v| python_truthy(v)).map(|v| python_string(Some(v), "converted_mod")).unwrap_or_else(|| "converted_mod".to_owned());

    let descriptors: Vec<&Map<String, Value>> = workspace.get("mod_elements").and_then(Value::as_array)
        .into_iter().flatten().filter_map(Value::as_object).collect();
    report.stats.insert("descriptor_count".to_owned(), json!(descriptors.len()));
    let mut type_counts = BTreeMap::<String, usize>::new();
    for descriptor in &descriptors {
        *type_counts.entry(python_string(descriptor.get("type"), "unknown")).or_insert(0) += 1;
    }
    report.stats.insert("element_type_counts".to_owned(), serde_json::to_value(type_counts)?);

    let mut descriptor_keys = BTreeMap::<(String, String), Vec<String>>::new();
    let mut name_counts = BTreeMap::<String, usize>::new();
    let mut custom_biomes = BTreeSet::<String>::new();
    for descriptor in &descriptors {
        let name = python_string(descriptor.get("name"), "");
        let registry = python_string(descriptor.get("registry_name"), "");
        let kind = python_string(descriptor.get("type"), "");
        descriptor_keys.entry((kind.to_lowercase(), registry.to_lowercase())).or_default().push(name.clone());
        *name_counts.entry(name.to_lowercase()).or_insert(0) += 1;
        if kind == "biome" && !name.is_empty() { custom_biomes.insert(name.clone()); }
        if name.is_empty() { report.error("element_name_empty", "Workspace element descriptor has no name.", "", registry.clone(), ""); }
        if !registry.is_empty() && registry != registry.to_lowercase() {
            report.error("registry_uppercase", "Modern registry paths must be lowercase.", name.clone(), registry.clone(), "");
        }
        if !registry.is_empty() && !registry.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-')) {
            report.error("registry_invalid", "Registry path contains invalid characters.", name.clone(), registry.clone(), "");
        }
    }
    for ((kind, registry), names) in descriptor_keys {
        if !registry.is_empty() && names.len() > 1 {
            report.error("duplicate_same_type_registry", format!("{} {} descriptors use registry path '{}'.", names.len(), kind, registry), "", registry, "");
        }
    }
    for (name, count) in name_counts {
        if !name.is_empty() && count > 1 {
            report.error("duplicate_element_name", format!("{} descriptors use element name '{}'.", count, name), name, "", "");
        }
    }

    let mut loaded_elements = BTreeSet::<String>::new();
    let mut explicit_item_paths = BTreeSet::<String>::new();
    let mut automatic_block_item_paths = BTreeSet::<String>::new();
    let mut missing_element_files = 0usize;
    let mut empty_creative_tabs = 0usize;
    let mut animated_textures = 0usize;

    for descriptor in &descriptors {
        let name = python_string(descriptor.get("name"), "");
        let kind = python_string(descriptor.get("type"), "");
        let registry = python_string(descriptor.get("registry_name"), "");
        let path = root.join("elements").join(format!("{name}.mod.json"));
        let Some(data) = read_json_object(&path) else {
            missing_element_files += 1;
            report.error("element_file_missing", "Descriptor has no readable .mod.json file.", name.clone(), registry.clone(), path.display().to_string());
            continue;
        };
        loaded_elements.insert(name.clone());
        let file_kind = python_string(data.get("_type"), "");
        if !file_kind.is_empty() && file_kind != kind {
            report.error("element_type_mismatch", format!("Descriptor type '{}' differs from file type '{}'.", kind, file_kind), name.clone(), registry.clone(), path.display().to_string());
        }
        let definition = object_value(&data, "definition").cloned().unwrap_or_default();
        if definition.is_empty() {
            report.error("definition_missing", "Element JSON has no non-empty definition object; MCreator will report a missing mod definition.", name.clone(), registry.clone(), path.display().to_string());
        }
        let normalized_kind = python_string(data.get("_type"), "");
        if !normalized_kind.trim().is_empty() && normalized_kind.to_lowercase() != kind.to_lowercase() {
            report.error("element_type_mismatch", format!("Element file _type '{}' does not match descriptor type '{}'.", normalized_kind, kind), name.clone(), registry.clone(), path.display().to_string());
        }
        let fv = if data.get("_fv").is_none() { Some(0) } else { python_int(data.get("_fv")) };
        match fv {
            Some(v) if v > 89 => report.error("element_format_too_new", format!("Element format FV{} is newer than the supported MCreator format FV89.", v), name.clone(), registry.clone(), path.display().to_string()),
            None => report.error("element_format_invalid", "Element _fv is not an integer.", name.clone(), registry.clone(), path.display().to_string()),
            _ => {}
        }

        let render_type = python_int(definition.get("renderType")).unwrap_or(0);
        let custom_model = python_string(definition.get("customModelName"), "");
        if (kind == "item" || kind == "tool") && render_type == 0 && custom_model.is_empty() {
            report.error("native_item_model_missing", "Native item/tool renderType=0 requires customModelName='Normal'.", name.clone(), registry.clone(), path.display().to_string());
        }
        if (kind == "block" || kind == "plant") && custom_model.is_empty() {
            report.warning("block_model_missing", "Block/plant customModelName is empty; native render modes may fail to resolve a model.", name.clone(), registry.clone(), path.display().to_string());
        }
        if matches!(kind.as_str(), "item" | "tool" | "armor" | "block" | "plant" | "livingentity")
            && definition.get("creativeTabs").and_then(Value::as_array).is_some_and(Vec::is_empty) {
            empty_creative_tabs += 1;
            report.warning("creative_tabs_empty", "Content is not assigned to a creative tab.", name.clone(), registry.clone(), "");
        }
        if (kind == "item" || kind == "tool") && !registry.is_empty() { explicit_item_paths.insert(registry.to_lowercase()); }
        let has_block_item = definition.get("hasBlockItem").map(python_truthy).unwrap_or(true);
        if (kind == "block" || kind == "plant") && !registry.is_empty() && has_block_item {
            automatic_block_item_paths.insert(registry.to_lowercase());
        }
        if kind == "dimension" {
            match definition.get("biomesInDimension").and_then(Value::as_array) {
                None => report.error("dimension_biomes_empty", "Dimension has no surface biome; generated multi-noise biome sources cannot be empty.", name.clone(), registry.clone(), ""),
                Some(biomes) if biomes.is_empty() => report.error("dimension_biomes_empty", "Dimension has no surface biome; generated multi-noise biome sources cannot be empty.", name.clone(), registry.clone(), ""),
                Some(biomes) => for biome in biomes {
                    if let Some(custom) = biome.as_str().and_then(|s| s.strip_prefix("CUSTOM:")) {
                        if !custom_biomes.contains(custom) {
                            report.error("dimension_custom_biome_missing", format!("Dimension references missing custom biome 'CUSTOM:{}'.", custom), name.clone(), registry.clone(), "");
                        }
                    }
                }
            }
        }
        if kind == "projectile" && python_string(definition.get("projectileItem"), "").trim().is_empty() {
            report.warning("projectile_item_unbound", "Projectile has no bound item; it cannot be selected by normal ranged-item generation.", name.clone(), registry.clone(), "");
        }
        if let Some(files) = object_value(descriptor, "metadata").and_then(|m| m.get("files")).and_then(Value::as_array) {
            for rel in files {
                let relative = python_string(Some(rel), "");
                if !root.join(&relative).exists() {
                    report.error("metadata_file_missing", "Workspace metadata references a missing file.", name.clone(), registry.clone(), relative);
                }
            }
        }

        let mut grouped = BTreeMap::<String, Vec<PathBuf>>::new();
        for (key, candidate) in resource_candidates(&root, &modid, &definition) {
            grouped.entry(key.split('|').next().unwrap_or(&key).to_owned()).or_default().push(candidate);
        }
        for (key, candidates) in grouped {
            if !candidates.iter().any(|p| p.exists()) {
                report.warning("texture_reference_missing", format!("Texture field '{}' does not resolve to a copied PNG.", key), name.clone(), registry.clone(),
                    candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" | "));
            } else {
                for candidate in &candidates {
                    if candidate.exists() {
                        let mut sidecar = candidate.file_name().unwrap_or_default().to_os_string();
                        sidecar.push(".mcmeta");
                        if candidate.with_file_name(sidecar).exists() { animated_textures += 1; break; }
                    }
                }
            }
        }
    }

    for registry in explicit_item_paths.intersection(&automatic_block_item_paths) {
        report.error("block_item_registry_collision", "An explicit item and an automatic BlockItem would register the same item path. Disable hasBlockItem on the block/plant.", "", registry.clone(), "");
    }
    report.stats.insert("loaded_element_files".to_owned(), json!(loaded_elements.len()));
    report.stats.insert("missing_element_files".to_owned(), json!(missing_element_files));
    report.stats.insert("empty_creative_tabs".to_owned(), json!(empty_creative_tabs));
    report.stats.insert("animated_texture_references".to_owned(), json!(animated_textures));
    report.stats.insert("error_count".to_owned(), json!(report.errors.len()));
    report.stats.insert("warning_count".to_owned(), json!(report.warnings.len()));
    report.ok = report.errors.is_empty();

    if write_reports {
        let json_path = root.join("MOJMODELBRIDGE_WORKSPACE_VALIDATION.json");
        let text_path = root.join("MOJMODELBRIDGE_WORKSPACE_VALIDATION.txt");
        fs::write(&json_path, format!("{}\n", serde_json::to_string_pretty(&report)?))
            .with_context(|| format!("Writing validation report {}", json_path.display()))?;
        let mut lines = vec![
            format!("Workspace: {}", report.workspace),
            format!("Result: {}", if report.ok { "PASS" } else { "FAIL" }),
            format!("Errors: {}", report.errors.len()),
            format!("Warnings: {}", report.warnings.len()),
            String::new(),
        ];
        for (heading, issues) in [("ERRORS", &report.errors), ("WARNINGS", &report.warnings), ("INFO", &report.info)] {
            lines.push(heading.to_owned());
            lines.push("-".repeat(heading.len()));
            if issues.is_empty() { lines.push("None".to_owned()); }
            for issue in issues {
                let context = [&issue.element, &issue.registry_name, &issue.path].into_iter()
                    .filter(|v| !v.is_empty()).cloned().collect::<Vec<_>>().join(", ");
                lines.push(format!("[{}] {}{}", issue.code, issue.message,
                    if context.is_empty() { String::new() } else { format!(" ({context})") }));
            }
            lines.push(String::new());
        }
        fs::write(&text_path, lines.join("\n"))
            .with_context(|| format!("Writing validation report {}", text_path.display()))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn element_file(root: &Path, name: &str, kind: &str, definition: Value) {
        let dir = root.join("elements");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.mod.json")), serde_json::to_vec(&json!({"_type":kind,"_fv":89,"definition":definition})).unwrap()).unwrap();
    }

    #[test]
    fn missing_workspace_definition_is_reported() {
        let root = tempdir().unwrap();
        let report = validate_workspace(root.path(), None, false).unwrap();
        assert!(!report.ok);
        assert_eq!(report.errors[0].code, "workspace_missing");
    }

    #[test]
    fn validates_collisions_metadata_textures_and_item_models() {
        let root = tempdir().unwrap();
        element_file(root.path(), "ExampleItem", "item", json!({"renderType":0,"customModelName":"","creativeTabs":[],"texture":"missing_icon"}));
        element_file(root.path(), "ExampleBlock", "block", json!({"creativeTabs":[],"hasBlockItem":true}));
        let workspace = json!({
            "workspaceSettings":{"modid":"demo"},
            "mod_elements":[
                {"name":"ExampleItem","type":"item","registry_name":"shared"},
                {"name":"ExampleBlock","type":"block","registry_name":"shared","metadata":{"files":["src/main/resources/missing.png"]}}
            ]
        });
        let report = validate_workspace(root.path(), Some(workspace), false).unwrap();
        let errors: BTreeSet<_> = report.errors.iter().map(|v| v.code.as_str()).collect();
        let warnings: BTreeSet<_> = report.warnings.iter().map(|v| v.code.as_str()).collect();
        assert!(errors.contains("native_item_model_missing"));
        assert!(errors.contains("metadata_file_missing"));
        assert!(errors.contains("block_item_registry_collision"));
        assert!(warnings.contains("block_model_missing"));
        assert!(warnings.contains("creative_tabs_empty"));
        assert!(warnings.contains("texture_reference_missing"));
        assert_eq!(report.stats["descriptor_count"], 2);
        assert_eq!(report.stats["loaded_element_files"], 2);
    }

    #[test]
    fn dimension_custom_biome_references_are_checked() {
        let root = tempdir().unwrap();
        element_file(root.path(), "TestDimension", "dimension", json!({"biomesInDimension":["CUSTOM:MissingBiome"]}));
        let workspace = json!({"workspaceSettings":{"modid":"demo"},"mod_elements":[{"name":"TestDimension","type":"dimension","registry_name":"test_dimension"}]});
        let report = validate_workspace(root.path(), Some(workspace), false).unwrap();
        assert!(report.errors.iter().any(|v| v.code == "dimension_custom_biome_missing"));
    }

    #[test]
    fn discovers_highest_version_root_workspace_and_writes_reports() {
        let root = tempdir().unwrap();
        let low = json!({"mcreatorVersion":202500,"workspaceSettings":{"modid":"demo"},"mod_elements":[]});
        let high = json!({"mcreatorVersion":202600,"workspaceSettings":{"modid":"demo"},"mod_elements":[]});
        fs::write(root.path().join("low.mcreator"), serde_json::to_vec(&low).unwrap()).unwrap();
        fs::write(root.path().join("high.mcreator"), serde_json::to_vec(&high).unwrap()).unwrap();
        let report = validate_workspace(root.path(), None, true).unwrap();
        assert!(report.ok);
        assert!(report.workspace.ends_with("high.mcreator"));
        assert!(root.path().join("MOJMODELBRIDGE_WORKSPACE_VALIDATION.json").is_file());
        assert!(root.path().join("MOJMODELBRIDGE_WORKSPACE_VALIDATION.txt").is_file());
    }
}
