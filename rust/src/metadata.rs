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
    /// Minecraft target declared by mcmod.info or a loader dependency.
    pub minecraft_version: String,
    /// Forge loader version/range when it can be recovered from metadata.
    pub forge_version: String,
    /// NeoForge loader version/range when the mod declares the NeoForge dependency.
    pub neoforge_version: String,
    /// Loader family inferred from the metadata file and dependency declarations.
    pub loader_family: String,
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
        minecraft_version: first(object.get("mcversion").or_else(|| object.get("minecraftVersion")).or_else(|| object.get("minecraft_version"))),
        forge_version: first(object.get("forgeVersion").or_else(|| object.get("forge_version"))),
        neoforge_version: first(object.get("neoforgeVersion").or_else(|| object.get("neoforge_version"))),
        loader_family: first(object.get("loader").or_else(|| object.get("loaderFamily").or_else(|| object.get("loader_family")))),
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

pub(crate) fn dependency_version_range(text: &str, dependency_id: &str) -> String {
    let mut in_dependency = false;
    let mut current_mod_id = String::new();
    let mut current_range = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("[[") || (line.starts_with('[') && line.ends_with(']')) {
            if in_dependency && current_mod_id.eq_ignore_ascii_case(dependency_id) && !current_range.is_empty() {
                return current_range;
            }
            in_dependency = line.starts_with("[[dependencies.");
            current_mod_id.clear();
            current_range.clear();
            continue;
        }
        if !in_dependency { continue; }
        if current_mod_id.is_empty() {
            current_mod_id = toml_string(line, "modId");
        }
        if current_range.is_empty() {
            current_range = toml_string(line, "versionRange");
        }
    }
    if in_dependency && current_mod_id.eq_ignore_ascii_case(dependency_id) { current_range } else { String::new() }
}

fn version_from_range(value: &str) -> String {
    Regex::new(r"(?i)(?:\[|\()\s*(\d+\.\d+(?:\.\d+)?)")
        .ok()
        .and_then(|regex| regex.captures(value))
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
        .unwrap_or_default()
}

fn from_mods_toml(path: &Path) -> Option<ModMetadata> {
    let text = fs::read_to_string(path).ok()?;
    let modid = toml_string(&text, "modId");
    if modid.is_empty() { return None; }
    let author_text = toml_string(&text, "authors");
    let minecraft_range = dependency_version_range(&text, "minecraft");
    let forge_range = dependency_version_range(&text, "forge");
    let neoforge_range = dependency_version_range(&text, "neoforge");
    let is_neoforge_file = path.file_name().and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("neoforge.mods.toml"));
    Some(ModMetadata {
        modid, name: toml_string(&text, "displayName"), version: toml_string(&text, "version"),
        minecraft_version: version_from_range(&minecraft_range),
        forge_version: version_from_range(&forge_range),
        neoforge_version: version_from_range(&neoforge_range),
        loader_family: if is_neoforge_file || !neoforge_range.is_empty() { "NeoForge".to_owned() }
            else if !forge_range.is_empty() { "Forge".to_owned() } else { String::new() },
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
    if primary.minecraft_version.is_empty() { primary.minecraft_version = fallback.minecraft_version; }
    if primary.forge_version.is_empty() { primary.forge_version = fallback.forge_version; }
    if primary.neoforge_version.is_empty() { primary.neoforge_version = fallback.neoforge_version; }
    if primary.loader_family.is_empty() { primary.loader_family = fallback.loader_family; }
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
 
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcmod_info_minecraft_version_is_not_confused_with_mod_version() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mcmod.info");
        fs::write(&path, r#"[{"modid":"sample","name":"Sample","version":"4.2.0","mcversion":"1.7.10"}]"#).unwrap();
        let metadata = from_json_metadata(&path, false).unwrap();
        assert_eq!(metadata.version, "4.2.0");
        assert_eq!(metadata.minecraft_version, "1.7.10");
    }

    #[test]
    fn mods_toml_minecraft_and_forge_dependencies_are_recovered() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mods.toml");
        fs::write(&path, r#"
modLoader="javafml"
loaderVersion="[14,)"
[[mods]]
modId="sample"
version="1.0"
displayName="Sample"
[[dependencies.sample]]
modId="forge"
mandatory=true
versionRange="[14.23.5.2854,)"
ordering="NONE"
side="BOTH"
[[dependencies.sample]]
modId="minecraft"
mandatory=true
versionRange="[1.12.2,1.13)"
ordering="NONE"
side="BOTH"
"#).unwrap();
        let metadata = from_mods_toml(&path).unwrap();
        assert_eq!(metadata.minecraft_version, "1.12.2");
        assert_eq!(metadata.forge_version, "14.23.5");
    }

    #[test]
    fn neoforge_mods_toml_recovers_minecraft_and_loader_versions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("neoforge.mods.toml");
        fs::write(&path, r#"
modLoader="javafml"
loaderVersion="[4,)"
[[mods]]
modId="sample"
version="1.0"
displayName="Sample"
[[dependencies.sample]]
modId="neoforge"
type="required"
versionRange="[21.1.100,)"
[[dependencies.sample]]
modId="minecraft"
type="required"
versionRange="[1.21.1,1.22)"
"#).unwrap();
        let metadata = from_mods_toml(&path).unwrap();
        assert_eq!(metadata.minecraft_version, "1.21.1");
        assert_eq!(metadata.neoforge_version, "21.1.100");
        assert_eq!(metadata.loader_family, "NeoForge");
    }

    #[test]
    fn dependency_version_range_is_scoped_to_dependency_table() {
        let text = r#"
[[dependencies.sample]]
modId="forge"
versionRange="[14.23.5,)"
[[dependencies.sample]]
modId="minecraft"
versionRange="[1.12.2,1.13)"
"#;
        assert_eq!(dependency_version_range(text, "minecraft"), "[1.12.2,1.13)");
        assert_eq!(dependency_version_range(text, "forge"), "[14.23.5,)");
    }
}
