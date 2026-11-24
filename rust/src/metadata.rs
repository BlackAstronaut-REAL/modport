use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModMetadata {
    pub modid: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: String,
    pub website: String,
    pub license: String,
    pub source: String,
}

impl ModMetadata {
    pub fn author_text(&self) -> String {
        self.authors.iter().filter(|v| !v.is_empty()).cloned().collect::<Vec<_>>().join(", ")
    }
}

fn first(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn authors(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) => s.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        Some(Value::Array(values)) => values.iter().filter_map(|v| match v {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
            Value::Object(_) => {
                let name = first(v.get("name"));
                if name.is_empty() { None } else { Some(name) }
            }
            _ => None,
        }).collect(),
        _ => Vec::new(),
    }
}

fn candidate_files(root: &Path, names: &[&str]) -> Vec<PathBuf> {
    let mut result: Vec<PathBuf> = WalkDir::new(root).follow_links(false).into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| names.iter().any(|name| entry.file_name().to_string_lossy().eq_ignore_ascii_case(name)))
        .map(|entry| entry.path().to_path_buf())
        .collect();
    result.sort_by(|a,b| a.components().count().cmp(&b.components().count())
        .then_with(|| a.to_string_lossy().to_lowercase().cmp(&b.to_string_lossy().to_lowercase())));
    result
}

fn from_json_metadata(path: &Path, fabric: bool) -> Option<ModMetadata> {
    let payload: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let object = if let Value::Array(entries) = &payload {
        entries.iter().find(|entry| entry.is_object())?
    } else { &payload };
    let contact = object.get("contact").and_then(Value::as_object);
    Some(ModMetadata {
        modid: first(object.get(if fabric { "id" } else { "modid" }).or_else(|| object.get("id"))),
        name: first(object.get("name")),
        version: first(object.get("version")),
        authors: authors(object.get("authors").or_else(|| object.get("authorList")).or_else(|| object.get("author")).or_else(|| object.get("contributors"))),
        description: first(object.get("description")),
        website: first(object.get("url").or_else(|| object.get("website")).or_else(|| contact.and_then(|c| c.get("homepage")))),
        license: first(object.get("license")),
        source: path.to_string_lossy().to_string(),
    })
}

fn toml_string(text: &str, key: &str) -> String {
    let escaped = regex::escape(key);
    Regex::new(&format!(r#"(?m)^\s*{escaped}\s*=\s*["']([^"']*)["']"#))
        .ok().and_then(|re| re.captures(text))
        .and_then(|c| c.get(1)).map(|m| m.as_str().trim().to_owned()).unwrap_or_default()
}

fn from_mods_toml(path: &Path) -> Option<ModMetadata> {
    let text = fs::read_to_string(path).ok()?;
    let modid = toml_string(&text, "modId");
    if modid.is_empty() { return None; }
    let author_text = toml_string(&text, "authors");
    Some(ModMetadata {
        modid, name: toml_string(&text, "displayName"), version: toml_string(&text, "version"),
        authors: authors(Some(&Value::String(author_text))),
        description: toml_string(&text, "description"), website: toml_string(&text, "displayURL"),
        license: toml_string(&text, "license"), source: path.to_string_lossy().to_string(),
    })
}

fn from_java(root: &Path) -> Option<ModMetadata> {
    let pattern = Regex::new(r"(?s)@Mod\s*\((.*?)\)").ok()?;
    let mut best: Option<ModMetadata> = None;
    for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() || entry.path().extension().and_then(|x| x.to_str()) != Some("java") { continue; }
        let Ok(text) = fs::read_to_string(entry.path()) else { continue; };
        for block in pattern.captures_iter(&text).filter_map(|c| c.get(1)) {
            let value = |key: &str| {
                let re = Regex::new(&format!(r#"\b{}\s*=\s*["']([^"']+)["']"#, regex::escape(key))).ok();
                re.and_then(|r| r.captures(block.as_str()))
                    .and_then(|c| c.get(1)).map(|m| m.as_str().trim().to_owned()).unwrap_or_default()
            };
            let modid = {
                let id = value("modid");
                if id.is_empty() { value("value") } else { id }
            };
            if modid.is_empty() { continue; }
            let candidate = ModMetadata {
                modid, name: value("name"), version: value("version"),
                source: entry.path().to_string_lossy().to_string(), ..Default::default()
            };
            if best.is_none() || (!candidate.name.is_empty() && best.as_ref().unwrap().name.is_empty()) {
                best = Some(candidate);
            }
        }
    }
    best
}

fn merge(mut primary: ModMetadata, fallback: Option<ModMetadata>) -> ModMetadata {
    let Some(fallback) = fallback else { return primary; };
    if primary.modid.is_empty() { primary.modid = fallback.modid; }
    if primary.name.is_empty() { primary.name = fallback.name; }
    if primary.version.is_empty() { primary.version = fallback.version; }
    if primary.authors.is_empty() { primary.authors = fallback.authors; }
    if primary.description.is_empty() { primary.description = fallback.description; }
    if primary.website.is_empty() { primary.website = fallback.website; }
    if primary.license.is_empty() { primary.license = fallback.license; }
    if primary.source.is_empty() { primary.source = fallback.source; }
    primary
}

/// Recover metadata in the same source-priority order as Beta 12:
/// mcmod.info, Fabric metadata, mods.toml, Java @Mod, then assets namespace.
pub fn infer_mod_metadata(root: &Path) -> ModMetadata {
    let mut result = ModMetadata::default();
    for path in candidate_files(root, &["mcmod.info"]) {
        result = merge(result, from_json_metadata(&path, false));
        if !result.modid.is_empty() && !result.name.is_empty() { break; }
    }
    if result.modid.is_empty() {
        for path in candidate_files(root, &["fabric.mod.json"]) {
            result = merge(result, from_json_metadata(&path, true));
            if !result.modid.is_empty() { break; }
        }
    }
    if result.modid.is_empty() {
        for path in candidate_files(root, &["mods.toml", "neoforge.mods.toml"]) {
            result = merge(result, from_mods_toml(&path));
            if !result.modid.is_empty() { break; }
        }
    }
    result = merge(result, from_java(root));
    if result.modid.is_empty() {
        let mut namespaces = Vec::new();
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_dir() && entry.file_name().to_string_lossy().eq_ignore_ascii_case("assets") {
                if let Ok(children) = fs::read_dir(entry.path()) {
                    for child in children.flatten().filter(|child| child.path().is_dir()) {
                        let name = child.file_name().to_string_lossy().to_string();
                        if Regex::new(r"^[a-z0-9_.-]+$").unwrap().is_match(&name) {
                            namespaces.push(name);
                        }
                    }
                }
            }
        }
        namespaces.sort_by_key(|value: &String| (matches!(value.as_str(), "minecraft" | "forge"), value.len(), value.clone()));
        if let Some(namespace) = namespaces.first() { result.modid = namespace.clone(); }
    }
    result
}
