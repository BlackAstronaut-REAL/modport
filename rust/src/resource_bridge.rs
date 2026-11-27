use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root).follow_links(false).into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path()).collect();
    files.sort();
    files
}

fn named_directories(root: &Path, name: &str) -> Vec<PathBuf> {
    WalkDir::new(root).follow_links(false).into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_dir() && entry.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|entry| entry.into_path()).collect()
}

fn resource_roots(root: &Path, name: &str) -> Vec<PathBuf> {
    let candidates = if name == "assets" {
        vec![root.join("src/main/resources/assets"), root.join("assets"), root.join("src/main/resources")]
    } else {
        vec![root.join("src/main/resources/data"), root.join("data")]
    };
    let mut roots: Vec<PathBuf> = candidates.into_iter().filter(|p| p.exists()).collect();
    roots.extend(named_directories(root, name));
    roots
}

fn relative_components(path: &Path) -> Vec<String> {
    path.components().filter_map(|component| match component {
        Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
        _ => None,
    }).collect()
}

fn relative_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn rewrite_namespace_string(value: &str, source_modid: &str, target_modid: &str) -> String {
    let source = source_modid.trim().to_lowercase();
    let target = target_modid.trim().to_lowercase();
    if source.is_empty() || target.is_empty() || source == target { return value.to_owned(); }
    let prefix = format!("{source}:");
    if value.to_lowercase().starts_with(&prefix) {
        return format!("{target}{}", &value[source.len()..]);
    }
    let tag_prefix = format!("#{source}:");
    if value.to_lowercase().starts_with(&tag_prefix) {
        return format!("#{target}{}", &value[source.len() + 1..]);
    }
    value.to_owned()
}

fn rewrite_namespace(value: Value, source_modid: &str, target_modid: &str) -> Value {
    match value {
        Value::String(text) => Value::String(rewrite_namespace_string(&text, source_modid, target_modid)),
        Value::Array(values) => Value::Array(values.into_iter().map(|v| rewrite_namespace(v, source_modid, target_modid)).collect()),
        Value::Object(values) => Value::Object(values.into_iter().map(|(k, v)| (k, rewrite_namespace(v, source_modid, target_modid))).collect()),
        other => other,
    }
}


fn rewrite_legacy_translation_key(key: &str, source_modid: &str, target_modid: &str) -> String {
    let mut parts: Vec<String> = key.split('.').map(str::to_owned).collect();
    if parts.len() < 2 { return key.to_owned(); }

    let kind = match parts[0].to_ascii_lowercase().as_str() {
        "item" => "item",
        "tile" | "block" => "block",
        "entity" => "entity",
        "container" => "container",
        _ => return key.to_owned(),
    };
    // Modern language keys are usually <kind>.<namespace>.<path>[.<suffix>].
    // Do not rewrite vanilla/dependency namespaces while migrating legacy
    // <kind>.<unlocalized-name>.name keys.
    if parts.len() >= 4 {
        let namespace = parts[1].split_once(':').map(|(namespace, _)| namespace).unwrap_or(&parts[1]);
        if !namespace.eq_ignore_ascii_case(source_modid)
            && !namespace.eq_ignore_ascii_case(target_modid) {
            return key.to_owned();
        }
    }
    let last = parts.last().map(|value| value.to_ascii_lowercase()).unwrap_or_default();
    let legacy_suffix = matches!(last.as_str(), "name" | "desc" | "description" | "tooltip" | "info");
    if legacy_suffix { parts.pop(); }

    if parts.len() < 2 { return key.to_owned(); }
    let mut identifiers = parts.into_iter().skip(1).collect::<Vec<_>>();
    if identifiers.is_empty() { return key.to_owned(); }

    if let Some(first) = identifiers.first().cloned() {
        if first.eq_ignore_ascii_case(source_modid) && !source_modid.is_empty() {
            identifiers.remove(0);
        } else if let Some((namespace, path)) = first.split_once(':') {
            if namespace.eq_ignore_ascii_case(source_modid) || source_modid.is_empty()
                || namespace.eq_ignore_ascii_case(target_modid) {
                identifiers[0] = path.to_owned();
            }
        }
    }
    if identifiers.is_empty() { return key.to_owned(); }

    let has_legacy_name_suffix = last == "name";
    let has_legacy_description_suffix = matches!(last.as_str(), "desc" | "description" | "tooltip" | "info");
    let had_source_namespace = key.split('.').nth(1).is_some_and(|part| {
        part.eq_ignore_ascii_case(source_modid) || part.eq_ignore_ascii_case(target_modid)
    }) || key.split('.').nth(1).is_some_and(|part| {
        part.split_once(':').is_some_and(|(namespace, _)| {
            namespace.eq_ignore_ascii_case(source_modid) || namespace.eq_ignore_ascii_case(target_modid)
        })
    });

    if has_legacy_name_suffix || has_legacy_description_suffix || had_source_namespace {
        let mut output = format!("{kind}.{target_modid}.{}", identifiers.join("."));
        if has_legacy_description_suffix {
            output.push('.');
            output.push_str(&last);
        }
        output
    } else {
        key.to_owned()
    }
}

fn rewrite_language_payload(value: Value, source_modid: &str, target_modid: &str) -> Value {
    match value {
        Value::Object(values) => {
            let mut output = serde_json::Map::new();
            for (key, value) in values {
                let rewritten = rewrite_legacy_translation_key(&key, source_modid, target_modid);
                output.entry(rewritten).or_insert_with(|| {
                    rewrite_language_payload(value, source_modid, target_modid)
                });
            }
            Value::Object(output)
        }
        Value::Array(values) => Value::Array(values.into_iter()
            .map(|value| rewrite_language_payload(value, source_modid, target_modid)).collect()),
        other => other,
    }
}

fn decode_legacy_lang_value(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' { output.push(ch); continue; }
        match chars.next() {
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('"') => output.push('"'),
            Some('\\') => output.push('\\'),
            Some('u') => {
                let digits: String = chars.by_ref().take(4).collect();
                if let Ok(code) = u32::from_str_radix(&digits, 16) {
                    if let Some(decoded) = char::from_u32(code) { output.push(decoded); }
                }
            }
            Some(other) => { output.push('\\'); output.push(other); }
            None => output.push('\\'),
        }
    }
    output
}

fn merge_legacy_lang_file(
    source: &Path,
    destination: &Path,
    source_modid: &str,
    target_modid: &str,
) -> Result<()> {
    let bytes = fs::read(source).with_context(|| format!("Reading legacy language file {}", source.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut values = if destination.is_file() {
        serde_json::from_slice::<Value>(&fs::read(destination)?)
            .ok().and_then(|value| value.as_object().cloned()).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = rewrite_legacy_translation_key(key.trim(), source_modid, target_modid);
        values.entry(key).or_insert_with(|| Value::String(decode_legacy_lang_value(value.trim())));
    }
    write_json(destination, &Value::Object(values))
}

fn relative_texture_map(textures_root: &Path) -> BTreeMap<String, String> {
    let mut output = BTreeMap::new();
    if !textures_root.is_dir() { return output; }
    for entry in WalkDir::new(textures_root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file()
            || !entry.path().extension().and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png")) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(textures_root) else { continue };
        let key = relative.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        output.entry(key).or_insert_with(|| relative.to_string_lossy().replace('\\', "/"));
    }
    output
}

fn normalize_texture_resource(
    reference: &str,
    source_modid: &str,
    target_modid: &str,
    texture_map: &BTreeMap<String, String>,
) -> Option<String> {
    if reference.trim().is_empty() || reference.trim_start().starts_with('#') { return None; }
    let reference = reference.trim().replace('\\', "/");
    let (namespace, raw_path) = reference.split_once(':').unwrap_or((target_modid, reference.as_str()));
    if namespace != target_modid && !namespace.eq_ignore_ascii_case(source_modid) {
        return None;
    }
    let mut path = raw_path.trim_start_matches('/').to_owned();
    if let Some(index) = path.to_ascii_lowercase().rfind("textures/") {
        path = path[index + "textures/".len()..].to_owned();
    }
    if path.to_ascii_lowercase().ends_with(".png") {
        path.truncate(path.len() - 4);
    }
    let lower_path = path.to_ascii_lowercase();
    let mut candidates = vec![lower_path.clone()];
    if let Some(rest) = lower_path.strip_prefix("items/") {
        candidates.push(format!("item/{rest}"));
    } else if let Some(rest) = lower_path.strip_prefix("item/") {
        candidates.push(format!("items/{rest}"));
    }
    if let Some(rest) = lower_path.strip_prefix("blocks/") {
        candidates.push(format!("block/{rest}"));
    } else if let Some(rest) = lower_path.strip_prefix("block/") {
        candidates.push(format!("blocks/{rest}"));
    }
    for candidate in candidates {
        let key = format!("{}.png", candidate).to_ascii_lowercase();
        if let Some(actual) = texture_map.get(&key) {
            let without_extension = actual.strip_suffix(".png").or_else(|| actual.strip_suffix(".PNG")).unwrap_or(actual);
            return Some(format!("{target_modid}:{}", without_extension.to_ascii_lowercase()));
        }
    }
    Some(format!("{target_modid}:{}", lower_path))
}

fn normalize_texture_values(
    value: &mut Value,
    source_modid: &str,
    target_modid: &str,
    texture_map: &BTreeMap<String, String>,
) -> bool {
    let mut changed = false;
    match value {
        Value::Object(values) => {
            for (key, child) in values.iter_mut() {
                if key.eq_ignore_ascii_case("textures") {
                    if let Value::Object(textures) = child {
                        for texture in textures.values_mut() {
                            if let Some(reference) = texture.as_str() {
                                if let Some(normalized) = normalize_texture_resource(reference, source_modid, target_modid, texture_map) {
                                    if normalized != reference {
                                        *texture = Value::String(normalized);
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                } else if (key.eq_ignore_ascii_case("texture") || key.eq_ignore_ascii_case("particle"))
                    && child.is_string() {
                    if let Some(reference) = child.as_str() {
                        if let Some(normalized) = normalize_texture_resource(reference, source_modid, target_modid, texture_map) {
                            if normalized != reference {
                                *child = Value::String(normalized);
                                changed = true;
                            }
                        }
                    }
                }
                changed |= normalize_texture_values(child, source_modid, target_modid, texture_map);
            }
        }
        Value::Array(values) => {
            for child in values { changed |= normalize_texture_values(child, source_modid, target_modid, texture_map); }
        }
        _ => {}
    }
    changed
}

/// Repair texture ResourceLocations in copied block/item model JSON. Legacy
/// Forge assets commonly vary in path casing, use a textures/ prefix or .png
/// suffix in references, or mix singular/plural item and block texture paths.
pub fn normalize_model_texture_references(
    output_resources: &Path,
    source_modid: &str,
    target_modid: &str,
) -> Result<usize> {
    let namespace_root = output_resources.join("assets").join(target_modid);
    let model_root = namespace_root.join("models");
    let textures_root = namespace_root.join("textures");
    let texture_map = relative_texture_map(&textures_root);
    if !model_root.is_dir() || texture_map.is_empty() { return Ok(0); }

    let mut files = Vec::new();
    for entry in WalkDir::new(&model_root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json")) {
            files.push(entry.into_path());
        }
    }
    files.sort();
    // Minecraft resource paths are case-sensitive. When a model refers to a
    // lower-case ResourceLocation but the legacy asset file is mixed-case, copy
    // the original file and animation sidecar to the normalized lower-case path.
    for (lower_key, actual_relative) in &texture_map {
        if lower_key == actual_relative { continue; }
        let source = textures_root.join(actual_relative);
        let destination = textures_root.join(lower_key);
        if !destination.is_file() {
            if let Some(parent) = destination.parent() { fs::create_dir_all(parent)?; }
            fs::copy(&source, &destination)
                .with_context(|| format!("Normalizing texture path {} to {}", source.display(), destination.display()))?;
        }
        let source_meta = PathBuf::from(format!("{}.mcmeta", source.to_string_lossy()));
        let destination_meta = PathBuf::from(format!("{}.mcmeta", destination.to_string_lossy()));
        if source_meta.is_file() && !destination_meta.is_file() {
            fs::copy(&source_meta, &destination_meta)?;
        }
    }

    let mut rewritten = 0usize;
    for path in files {
        let Some(mut value) = read_json(&path) else { continue };
        if normalize_texture_values(&mut value, source_modid, target_modid, &texture_map) {
            write_json(&path, &value)?;
            rewritten += 1;
        }
    }
    Ok(rewritten)
}

fn rewrite_sound_json(value: Value, source_modid: &str, target_modid: &str) -> Value {
    match value {
        Value::String(text) => {
            let mut output = text;
            if let Some((namespace, path)) = output.split_once(':') {
                if namespace.eq_ignore_ascii_case(source_modid.trim()) {
                    output = format!("{}:{path}", target_modid.trim().to_lowercase());
                }
            }
            if let Some(path) = output.strip_prefix("sounds/") {
                output = format!("sounds/{}", path.to_lowercase());
            } else if let Some((namespace, path)) = output.split_once(':') {
                if namespace.eq_ignore_ascii_case(target_modid.trim()) && path.starts_with("sounds/") {
                    output = format!("{namespace}:sounds/{}", &path[7..].to_lowercase());
                }
            }
            Value::String(output)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(|v| rewrite_sound_json(v, source_modid, target_modid)).collect()),
        Value::Object(values) => Value::Object(values.into_iter().map(|(k, v)| (k, rewrite_sound_json(v, source_modid, target_modid))).collect()),
        other => other,
    }
}

fn migrate_legacy_recipe_result(value: Value) -> Value {
    match value {
        Value::Object(mut object) => {
            for child in object.values_mut() {
                *child = migrate_legacy_recipe_result(std::mem::take(child));
            }
            if let Some(Value::Object(result)) = object.get_mut("result") {
                if !result.contains_key("id") {
                    if let Some(item) = result.remove("item") {
                        result.insert("id".to_owned(), item);
                    }
                }
            }
            Value::Object(object)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(migrate_legacy_recipe_result).collect()),
        other => other,
    }
}

fn rewrite_biome_modifier_type(value: Value) -> Value {
    match value {
        Value::String(text) if text.eq_ignore_ascii_case("forge:add_features") => Value::String("neoforge:add_features".to_owned()),
        Value::Array(values) => Value::Array(values.into_iter().map(rewrite_biome_modifier_type).collect()),
        Value::Object(values) => Value::Object(values.into_iter().map(|(k, v)| (k, rewrite_biome_modifier_type(v))).collect()),
        other => other,
    }
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(value)?))
        .with_context(|| format!("Writing imported resource {}", path.display()))
}

fn copy_if_missing(source: &Path, destination: &Path) -> Result<bool> {
    if !source.is_file() || destination.exists() { return Ok(false); }
    if let Some(parent) = destination.parent() { fs::create_dir_all(parent)?; }
    fs::copy(source, destination)
        .with_context(|| format!("Copying {} to {}", source.display(), destination.display()))?;
    Ok(true)
}

fn remap_first_component(relative: &Path, source_modid: &str, target_modid: &str) -> PathBuf {
    let parts = relative_components(relative);
    if parts.len() < 2 || !parts[0].eq_ignore_ascii_case(source_modid.trim()) {
        return relative.to_path_buf();
    }
    let mut output = PathBuf::from(target_modid.trim());
    for part in parts.iter().skip(1) { output.push(part); }
    output
}

fn lowercase_sound_path(relative: &Path) -> PathBuf {
    let parts = relative_components(relative);
    if parts.len() < 3 || !parts[1].eq_ignore_ascii_case("sounds") { return relative.to_path_buf(); }
    let mut output = PathBuf::new();
    for part in &parts[..parts.len() - 1] { output.push(part); }
    output.push(parts.last().unwrap().to_lowercase());
    output
}

fn contains_generated_data_directory(relative: &Path) -> bool {
    let generated = ["dimension", "dimension_type", "biome", "biomes"];
    relative_components(relative).iter().skip(1)
        .any(|part| generated.contains(&part.to_lowercase().as_str()))
}

/// Native port of oldVer/mcreator_source_bridge.py::_copy_source_resources.
pub fn copy_source_resources(
    source_root: &Path,
    output_resources: &Path,
    source_modid: Option<&str>,
    target_modid: Option<&str>,
) -> Result<usize> {
    let source_modid = source_modid.unwrap_or("").trim();
    let target_modid = target_modid.unwrap_or("").trim();
    let mut copied = 0usize;
    let mut seen = HashSet::<String>::new();

    for assets_root in resource_roots(source_root, "assets") {
        for source in files_under(&assets_root) {
            let Ok(relative) = source.strip_prefix(&assets_root) else { continue };
            if !seen.insert(relative_key(relative)) { continue; }
            let mut relative = relative.to_path_buf();
            if !source_modid.is_empty() && !target_modid.is_empty() {
                relative = remap_first_component(&relative, source_modid, target_modid);
            }
            let relative_parts = relative_components(&relative);
            let is_language_file = relative_parts.len() >= 3
                && relative_parts[1].eq_ignore_ascii_case("lang");
            if is_language_file
                && source.extension().and_then(|v| v.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("lang")) {
                let locale = source.file_stem().and_then(|value| value.to_str()).unwrap_or("en_us").to_ascii_lowercase();
                let mut language_relative = relative.clone();
                language_relative.set_file_name(format!("{locale}.json"));
                let destination = output_resources.join("assets").join(language_relative);
                merge_legacy_lang_file(&source, &destination, source_modid, target_modid)?;
                copied += 1;
                continue;
            }
            let mut destination = output_resources.join("assets").join(&relative);
            if source.extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("json"))
                && !source_modid.is_empty() && !target_modid.is_empty() {
                if let Some(mut payload) = read_json(&source) {
                    payload = rewrite_namespace(payload, source_modid, target_modid);
                    if is_language_file {
                        payload = rewrite_language_payload(payload, source_modid, target_modid);
                    }
                    if relative.file_name().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("sounds.json"))
                        && payload.is_object() {
                        payload = rewrite_sound_json(payload, source_modid, target_modid);
                    }
                    write_json(&destination, &payload)?;
                    copied += 1;
                    continue;
                }
            }
            let parts = relative_components(&relative);
            if parts.len() >= 3 && parts[1].eq_ignore_ascii_case("sounds")
                && source.extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("ogg")) {
                relative = lowercase_sound_path(&relative);
                destination = output_resources.join("assets").join(&relative);
            }
            if copy_if_missing(&source, &destination)? { copied += 1; }
        }
    }

    seen.clear();
    for data_root in resource_roots(source_root, "data") {
        for source in files_under(&data_root) {
            let Ok(relative) = source.strip_prefix(&data_root) else { continue };
            if !seen.insert(relative_key(relative)) || contains_generated_data_directory(relative) { continue; }
            let mut relative = relative.to_path_buf();
            let parts = relative_components(&relative);
            if !source_modid.is_empty() && !target_modid.is_empty()
                && parts.first().is_some_and(|part| part.eq_ignore_ascii_case(source_modid)) {
                relative = remap_first_component(&relative, source_modid, target_modid);
            }
            let parts = relative_components(&relative);
            if !target_modid.is_empty() && parts.len() >= 3
                && parts[1].eq_ignore_ascii_case("forge") && parts[2].eq_ignore_ascii_case("biome_modifier") {
                let mut migrated = PathBuf::from(target_modid);
                migrated.push("neoforge");
                migrated.push("biome_modifier");
                for part in parts.iter().skip(3) { migrated.push(part); }
                relative = migrated;
            }
            let destination = output_resources.join("data").join(&relative);
            if source.extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("json"))
                && !source_modid.is_empty() && !target_modid.is_empty() {
                if let Some(payload) = read_json(&source) {
                    let mut payload = rewrite_biome_modifier_type(rewrite_namespace(payload, source_modid, target_modid));
                    let relative_parts = relative_components(&relative);
                    if relative_parts.len() >= 2 && relative_parts[1].eq_ignore_ascii_case("recipes") {
                        payload = migrate_legacy_recipe_result(payload);
                    }
                    write_json(&destination, &payload)?;
                    copied += 1;
                    continue;
                }
            }
            if copy_if_missing(&source, &destination)? { copied += 1; }
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn remaps_asset_namespaces_and_sound_sample_paths() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let output = temp.path().join("out");
        let namespace = source.join("assets/chaospersists");
        fs::create_dir_all(namespace.join("sounds/entity")).unwrap();
        fs::create_dir_all(namespace.join("models/item")).unwrap();
        fs::write(namespace.join("sounds/entity/ROAR.OGG"), b"OggS").unwrap();
        fs::write(namespace.join("models/item/ore.json"), serde_json::to_vec(&json!({
            "parent":"chaospersists:item/generated",
            "texture":"#chaospersists:items/ore",
            "vanilla":"minecraft:item/generated"
        })).unwrap()).unwrap();
        fs::write(namespace.join("sounds.json"), serde_json::to_vec(&json!({
            "chaospersists.entity.roar":{"sounds":[{"name":"chaospersists:sounds/Entity/ROAR"}]}
        })).unwrap()).unwrap();

        let copied = copy_source_resources(&source, &output, Some("chaospersists"), Some("targetmod")).unwrap();
        assert!(copied >= 3);
        assert!(output.join("assets/targetmod/sounds/entity/roar.ogg").is_file());
        let model: Value = serde_json::from_slice(&fs::read(output.join("assets/targetmod/models/item/ore.json")).unwrap()).unwrap();
        assert_eq!(model["parent"], "targetmod:item/generated");
        assert_eq!(model["texture"], "#targetmod:items/ore");
        assert_eq!(model["vanilla"], "minecraft:item/generated");
        let sounds: Value = serde_json::from_slice(&fs::read(output.join("assets/targetmod/sounds.json")).unwrap()).unwrap();
        assert_eq!(sounds["chaospersists.entity.roar"]["sounds"][0]["name"], "targetmod:sounds/entity/roar");
    }

    #[test]
    fn migrates_legacy_recipe_result_item_to_modern_id_and_remaps_namespace() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let output = temp.path().join("out");
        let recipe_dir = source.join("data/oldmod/recipes");
        fs::create_dir_all(&recipe_dir).unwrap();
        fs::write(recipe_dir.join("ruby.json"), serde_json::to_vec(&json!({
            "type": "minecraft:crafting_shaped",
            "pattern": ["R"],
            "key": {"R": {"item": "oldmod:ruby_ore"}},
            "result": {"item": "oldmod:ruby", "count": 2}
        })).unwrap()).unwrap();

        copy_source_resources(&source, &output, Some("oldmod"), Some("newmod")).unwrap();
        let migrated: Value = serde_json::from_slice(
            &fs::read(output.join("data/newmod/recipes/ruby.json")).unwrap()
        ).unwrap();
        assert_eq!(migrated["key"]["R"]["item"], "newmod:ruby_ore");
        assert_eq!(migrated["result"]["id"], "newmod:ruby");
        assert_eq!(migrated["result"]["count"], 2);
        assert!(migrated["result"].get("item").is_none());
    }

    #[test]
    fn skips_generated_data_and_migrates_forge_biome_modifiers() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let output = temp.path().join("out");
        let generated = source.join("data/demo/dimension_type");
        let modifier = source.join("data/demo/forge/biome_modifier");
        let feature = source.join("data/demo/worldgen/placed_feature");
        fs::create_dir_all(&generated).unwrap();
        fs::create_dir_all(&modifier).unwrap();
        fs::create_dir_all(&feature).unwrap();
        fs::write(generated.join("dimension.json"), b"{\"ignored\":true}").unwrap();
        fs::write(modifier.join("add_ore.json"), serde_json::to_vec(&json!({
            "type":"forge:add_features","features":["demo:ore"]
        })).unwrap()).unwrap();
        fs::write(feature.join("ore.json"), serde_json::to_vec(&json!({"feature":"demo:ore"})).unwrap()).unwrap();

        let copied = copy_source_resources(&source, &output, Some("demo"), Some("newmod")).unwrap();
        assert_eq!(copied, 2);
        assert!(!output.join("data/newmod/dimension_type/dimension.json").exists());
        let migrated: Value = serde_json::from_slice(&fs::read(output.join("data/newmod/neoforge/biome_modifier/add_ore.json")).unwrap()).unwrap();
        assert_eq!(migrated["type"], "neoforge:add_features");
        assert_eq!(migrated["features"][0], "newmod:ore");
        let worldgen: Value = serde_json::from_slice(&fs::read(output.join("data/newmod/worldgen/placed_feature/ore.json")).unwrap()).unwrap();
        assert_eq!(worldgen["feature"], "newmod:ore");
    }

    #[test]
    fn preserves_existing_plain_files_and_copies_animated_texture_metadata() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let output = temp.path().join("out");
        let src_assets = source.join("assets/demo/textures/item");
        let dst_assets = output.join("assets/demo/textures/item");
        fs::create_dir_all(&src_assets).unwrap();
        fs::create_dir_all(&dst_assets).unwrap();
        fs::write(src_assets.join("icon.png"), b"new-png").unwrap();
        fs::write(src_assets.join("icon.png.mcmeta"), b"{\"animation\":{}}").unwrap();
        fs::write(dst_assets.join("icon.png"), b"existing-png").unwrap();

        copy_source_resources(&source, &output, Some("demo"), Some("demo")).unwrap();
        assert_eq!(fs::read(dst_assets.join("icon.png")).unwrap(), b"existing-png");
        assert_eq!(fs::read(dst_assets.join("icon.png.mcmeta")).unwrap(), b"{\"animation\":{}}");
    }

    #[test]
    fn rewrites_resource_location_values_but_not_object_keys() {
        let value = json!({"#demo:old_tag":"demo:item/a","unchanged":"minecraft:stone"});
        let result = rewrite_namespace(value, "demo", "other");
        assert_eq!(result["#demo:old_tag"], "other:item/a");
        assert_eq!(rewrite_namespace_string("#demo:old_tag", "demo", "other"), "#other:old_tag");
        assert_eq!(result["unchanged"], "minecraft:stone");
    }

    #[test]
    fn migrates_legacy_lang_names_to_target_namespace_json() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let output = temp.path().join("out");
        let language = source.join("assets/oldmod/lang/en_US.lang");
        fs::create_dir_all(language.parent().unwrap()).unwrap();
        fs::write(&language,
            "item.ruby_sword.name=Ruby Sword\nitem.ruby_sword.desc=§6Legendary weapon\n"
        ).unwrap();

        copy_source_resources(&source, &output, Some("oldmod"), Some("newmod")).unwrap();
        let migrated_path = output.join("assets/newmod/lang/en_us.json");
        assert!(migrated_path.is_file());
        let migrated: Value = serde_json::from_slice(&fs::read(migrated_path).unwrap()).unwrap();
        assert_eq!(migrated["item.newmod.ruby_sword"], "Ruby Sword");
        assert_eq!(migrated["item.newmod.ruby_sword.desc"], "§6Legendary weapon");
    }

    #[test]
    fn rewrites_legacy_json_language_keys_and_keeps_vanilla_keys() {
        let input = json!({
            "item.oldmod.ruby_sword.name": "Ruby Sword",
            "tile.ruby_ore.name": "Ruby Ore",
            "item.ruby_staff.description": "A staff",
            "item.minecraft.diamond.name": "Diamond"
        });
        let output = rewrite_language_payload(input, "oldmod", "newmod");
        assert_eq!(output["item.newmod.ruby_sword"], "Ruby Sword");
        assert_eq!(output["block.newmod.ruby_ore"], "Ruby Ore");
        assert_eq!(output["item.newmod.ruby_staff.description"], "A staff");
        assert_eq!(output["item.minecraft.diamond.name"], "Diamond");
    }

    #[test]
    fn normalizes_namespaced_model_textures_against_copied_files() {
        let temp = tempdir().unwrap();
        let resources = temp.path();
        let texture = resources.join("assets/newmod/textures/items/RubySword.PNG");
        fs::create_dir_all(texture.parent().unwrap()).unwrap();
        fs::write(&texture, b"texture").unwrap();

        let model = resources.join("assets/newmod/models/item/ruby_sword.json");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&model, serde_json::to_vec(&json!({
            "parent":"minecraft:item/handheld",
            "textures":{"layer0":"oldmod:Textures/Items/RUBYSWORD.PNG","particle":"minecraft:item/diamond"}
        })).unwrap()).unwrap();

        assert_eq!(normalize_model_texture_references(resources, "oldmod", "newmod").unwrap(), 1);
        let rewritten: Value = serde_json::from_slice(&fs::read(model).unwrap()).unwrap();
        assert_eq!(rewritten["textures"]["layer0"], "newmod:items/rubysword");
        assert_eq!(rewritten["textures"]["particle"], "minecraft:item/diamond");
        assert!(resources.join("assets/newmod/textures/items/rubysword.png").is_file());
    }

}
