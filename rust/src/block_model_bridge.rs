use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

fn snake_case(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            if previous.is_ascii_lowercase() || previous.is_ascii_digit() { output.push('_'); }
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

fn compact_registry(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn read_json(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

fn json_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root).follow_links(false).into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file()
            && entry.path().extension().and_then(|value| value.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("json")))
        .map(|entry| entry.into_path()).collect();
    files.sort();
    files
}

fn asset_namespaces(root: &Path, modid: &str, section: &str) -> Vec<PathBuf> {
    let mut bases = vec![root.join("src/main/resources/assets").join(modid), root.join("assets").join(modid)];
    bases.retain(|base| base.is_dir());
    bases.into_iter().map(|base| base.join(section)).filter(|path| path.is_dir()).collect()
}

fn collect_model_refs(state: &Value) -> Vec<String> {
    let mut refs = Vec::new();
    if let Some(variants) = state.get("variants").and_then(Value::as_object) {
        for variant in variants.values() {
            if let Some(model) = variant.get("model").and_then(Value::as_str) {
                refs.push(model.to_owned());
            } else if let Some(entries) = variant.as_array() {
                for entry in entries {
                    if let Some(model) = entry.get("model").and_then(Value::as_str) { refs.push(model.to_owned()); }
                }
            }
        }
    }
    if let Some(parts) = state.get("multipart").and_then(Value::as_array) {
        for part in parts {
            if let Some(model) = part.get("apply").and_then(|apply| apply.get("model")).and_then(Value::as_str) {
                refs.push(model.to_owned());
            }
        }
    }
    refs
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone()).to_string_lossy().to_lowercase();
        if seen.insert(key) { result.push(path); }
    }
    result
}

/// Preserve namespaced Minecraft block/item texture paths, matching the old exporter.
pub fn mcreator_texture_ref(value: &str) -> String {
    let value = value.trim().replace('\\', "/");
    if value.is_empty() { return String::new(); }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("minecraft:block/") {
        return format!("minecraft:{}", &value["minecraft:block/".len()..]);
    }
    if lower.starts_with("minecraft:item/") {
        return format!("minecraft:{}", &value["minecraft:item/".len()..]);
    }
    value
}

fn texture_value(textures: &Map<String, Value>, names: &[&str]) -> String {
    names.iter().find_map(|name| textures.get(*name).and_then(Value::as_str)).unwrap_or("").to_owned()
}

/// Infer MCreator's native block render mode and concrete texture references from
/// block model/blockstate JSON. This ports the old _mcreator_block_model_info helper.
fn mcreator_block_model_info_raw(source_roots: &[PathBuf], source_modid: &str, registry: &str) -> Value {
    let registry = snake_case(registry);
    let mut candidates = Vec::new();

    for root in source_roots {
        for base in asset_namespaces(root, source_modid, "models") {
            let block = base.join("block");
            candidates.push(block.join(format!("{registry}.json")));
            candidates.push(block.join(format!("{registry}_inventory.json")));
        }
        for base in asset_namespaces(root, source_modid, "blockstates") {
            let state = base.join(format!("{registry}.json"));
            let Some(payload) = read_json(&state) else { continue };
            for reference in collect_model_refs(&payload) {
                let model_path = reference.split_once(':').map(|(_, path)| path).unwrap_or(&reference).trim_start_matches('/');
                let model_path = model_path.strip_prefix("models/").unwrap_or(model_path);
                for models_root in asset_namespaces(root, source_modid, "models") {
                    candidates.push(models_root.join(format!("{model_path}.json")));
                }
            }
        }
        if candidates.is_empty() {
            for path in json_files(root) {
                let parts: Vec<String> = path.components().filter_map(|part| match part {
                    Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
                    _ => None,
                }).collect();
                let has_block_model_path = parts.windows(2).any(|pair| pair[0] == "models" && pair[1] == "block");
                let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or_default();
                if has_block_model_path && compact_registry(stem) == compact_registry(&registry) { candidates.push(path); }
            }
        }
    }

    for path in unique_paths(candidates).into_iter().filter(|path| path.is_file()) {
        let Some(payload) = read_json(&path) else { continue };
        let parent = payload.get("parent").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        let textures = payload.get("textures").and_then(Value::as_object).cloned().unwrap_or_default();
        let model_path = path.to_string_lossy().into_owned();

        if parent.ends_with("cross") || parent == "minecraft:block/cross" {
            return json!({"renderType":12,"customModelName":"Cross model","texture":texture_value(&textures,&["cross","plant","particle"]),"modelPath":model_path});
        }
        if parent.ends_with("grass_block") || (textures.contains_key("side") && textures.contains_key("top")
            && textures.contains_key("bottom") && textures.contains_key("overlay")) {
            let side = texture_value(&textures,&["side","all"]);
            return json!({"renderType":14,"customModelName":"Grass block","textureTop":texture_value(&textures,&["top"]),
                "textureBottom":texture_value(&textures,&["bottom"]),"texture":side,"textureLeft":side,"textureFront":side,
                "textureRight":side,"textureBack":side,"modelPath":model_path});
        }
        if parent.ends_with("crop") {
            return json!({"renderType":12,"customModelName":"Cross model","texture":texture_value(&textures,&["crop","all","particle"]),"modelPath":model_path});
        }
        if parent.ends_with("flower_pot_cross") {
            return json!({"renderType":12,"customModelName":"Cross model","texture":texture_value(&textures,&["plant","all","particle"]),"modelPath":model_path});
        }
        if parent.ends_with("cube_all") {
            return json!({"renderType":11,"customModelName":"Single texture","texture":texture_value(&textures,&["all","particle"]),"modelPath":model_path});
        }
        if parent.ends_with("cube") {
            let texture = texture_value(&textures,&["all","particle"]);
            let top = texture_value(&textures,&["up","top"]);
            let bottom = texture_value(&textures,&["down","bottom"]);
            let left = texture_value(&textures,&["west"]);
            let front = texture_value(&textures,&["north"]);
            let right = texture_value(&textures,&["east"]);
            let back = texture_value(&textures,&["south"]);
            return json!({"renderType":10,"customModelName":"Normal","texture":texture,
                "textureTop":if top.is_empty(){&texture}else{&top},
                "textureBottom":if bottom.is_empty(){&texture}else{&bottom},
                "textureLeft":if left.is_empty(){&texture}else{&left},
                "textureFront":if front.is_empty(){&texture}else{&front},
                "textureRight":if right.is_empty(){&texture}else{&right},
                "textureBack":if back.is_empty(){&texture}else{&back},"modelPath":model_path});
        }
        if ["up","down","north","south","east","west","side","top","bottom"].iter().any(|key| textures.contains_key(*key)) {
            let side = texture_value(&textures,&["side","north","particle"]);
            let top = texture_value(&textures,&["top","up"]);
            let bottom = texture_value(&textures,&["bottom","down"]);
            let left = texture_value(&textures,&["west"]);
            let front = texture_value(&textures,&["north"]);
            let right = texture_value(&textures,&["east"]);
            let back = texture_value(&textures,&["south"]);
            return json!({"renderType":11,"texture":side,
                "textureTop":mcreator_texture_ref(if top.is_empty(){&side}else{&top}),
                "textureBottom":mcreator_texture_ref(if bottom.is_empty(){&side}else{&bottom}),
                "textureLeft":mcreator_texture_ref(if left.is_empty(){&side}else{&left}),
                "textureFront":mcreator_texture_ref(if front.is_empty(){&side}else{&front}),
                "textureRight":mcreator_texture_ref(if right.is_empty(){&side}else{&right}),
                "textureBack":mcreator_texture_ref(if back.is_empty(){&side}else{&back}),"modelPath":model_path});
        }
    }
    Value::Object(Map::new())
}

/// Normalize block-model texture fields to MCreator-compatible resource references.
pub fn normalize_block_model_info_textures(mut info: Value) -> Value {
    const FIELDS: &[&str] = &[
        "texture", "textureTop", "textureBottom", "textureLeft",
        "textureFront", "textureRight", "textureBack", "itemTexture",
        "particleTexture",
    ];
    if let Some(object) = info.as_object_mut() {
        for field in FIELDS {
            let value = object.get(*field).and_then(Value::as_str).unwrap_or("").to_owned();
            if !value.is_empty() {
                object.insert((*field).to_owned(), Value::String(mcreator_texture_ref(&value)));
            }
        }
    }
    info
}

/// Infer a block render type and normalize all returned texture references.
pub fn mcreator_block_model_info(source_roots: &[PathBuf], source_modid: &str, registry: &str) -> Value {
    normalize_block_model_info_textures(mcreator_block_model_info_raw(source_roots, source_modid, registry))
}

fn model_texture_stem(reference: &str) -> String {
    let mut value = reference.trim().replace('\\', "/");
    if let Some((_, path)) = value.split_once(':') { value = path.to_owned(); }
    value = value.trim_start_matches('/').to_owned();
    if value.to_ascii_lowercase().starts_with("textures/") { value = value[9..].to_owned(); }
    value = value.rsplit('/').next().unwrap_or("").to_owned();
    if value.to_ascii_lowercase().ends_with(".png") { value.truncate(value.len() - 4); }
    value
}

fn safe_texture_relative(reference: &str) -> Option<PathBuf> {
    let mut value = reference.trim().replace('\\', "/");
    if let Some((_, path)) = value.split_once(':') { value = path.to_owned(); }
    value = value.trim_start_matches('/').to_owned();
    if value.to_ascii_lowercase().starts_with("textures/") { value = value[9..].to_owned(); }
    if value.to_ascii_lowercase().ends_with(".png") { value.truncate(value.len() - 4); }
    let path = PathBuf::from(value);
    if path.components().any(|part| !matches!(part, Component::Normal(_))) { return None; }
    Some(path)
}

fn model_texture_sources(model_path: &Path, reference: &str) -> Vec<PathBuf> {
    let Some(relative) = safe_texture_relative(reference) else { return Vec::new() };
    let mut candidates = Vec::new();
    let mut cursor = model_path.parent();
    while let Some(directory) = cursor {
        let assets = directory.join("assets");
        if assets.is_dir() {
            if let Ok(namespaces) = fs::read_dir(&assets) {
                let mut namespaces: Vec<PathBuf> = namespaces.filter_map(|entry| entry.ok().map(|v| v.path())).filter(|path| path.is_dir()).collect();
                namespaces.sort();
                for namespace in namespaces {
                    candidates.push(namespace.join("textures").join(&relative).with_extension("png"));
                    candidates.push(assets.join(&relative).with_extension("png"));
                }
            }
            break;
        }
        cursor = directory.parent();
    }
    let target_name = relative.file_name().map(|value| value.to_string_lossy().to_ascii_lowercase() + ".png");
    if let Some(target_name) = target_name {
        let mut roots = vec![model_path.parent().unwrap_or_else(|| Path::new("."))];
        roots.extend(model_path.ancestors().skip(1).take(3));
        for root in roots {
            if !root.is_dir() { continue; }
            for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
                if entry.file_type().is_file() && entry.file_name().to_string_lossy().eq_ignore_ascii_case(&target_name) {
                    candidates.push(entry.into_path());
                }
            }
        }
    }
    unique_paths(candidates).into_iter().filter(|path| path.is_file()).collect()
}

fn copy_file_with_metadata(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
    if fs::canonicalize(source).ok() != fs::canonicalize(target).ok() {
        fs::copy(source, target).with_context(|| format!("Copying {} to {}", source.display(), target.display()))?;
    }
    let source_meta = PathBuf::from(format!("{}.mcmeta", source.display()));
    if source_meta.is_file() {
        let target_meta = PathBuf::from(format!("{}.mcmeta", target.display()));
        if let Some(parent) = target_meta.parent() { fs::create_dir_all(parent)?; }
        fs::copy(&source_meta, &target_meta).with_context(|| format!("Copying {} to {}", source_meta.display(), target_meta.display()))?;
    }
    Ok(())
}

/// Materialize a model JSON texture reference into the MCreator texture tree.
pub fn copy_model_ref_texture_to_workspace(
    model_path: Option<&Path>,
    texture_ref: &str,
    texture_root: &Path,
    output_subdir: &str,
    registry_name: &str,
) -> Result<String> {
    let reference = texture_ref.trim().replace('\\', "/");
    if reference.is_empty() { return Ok(String::new()); }
    if let Some((namespace, resource_path)) = reference.split_once(':') {
        if namespace.eq_ignore_ascii_case("minecraft") {
            let path = resource_path.trim_start_matches('/').strip_prefix("block/")
                .or_else(|| resource_path.trim_start_matches('/').strip_prefix("item/"))
                .unwrap_or(resource_path.trim_start_matches('/'));
            return Ok(format!("minecraft:{path}"));
        }
    }
    let stem = model_texture_stem(&reference);
    let Some(model_path) = model_path else { return Ok(stem) };
    let Some(source) = model_texture_sources(model_path, &reference).into_iter().next() else { return Ok(stem) };
    let target_dir = if output_subdir.is_empty() { texture_root.to_path_buf() } else { texture_root.join(output_subdir) };
    fs::create_dir_all(&target_dir)?;
    let mut target = target_dir.join(format!("{stem}.png"));
    let mut resolved_stem = stem;
    if target.is_file() && fs::read(&target)? != fs::read(&source)? {
        target = target_dir.join(format!("{registry_name}_{resolved_stem}.png"));
        resolved_stem = target.file_stem().and_then(|value| value.to_str()).unwrap_or("").to_owned();
    }
    copy_file_with_metadata(&source, &target)?;
    Ok(resolved_stem)
}


fn rewrite_model_namespace(value: Value, source_modid: &str, target_modid: &str) -> Value {
    match value {
        Value::String(text) => {
            if let Some((namespace, path)) = text.split_once(':') {
                if namespace.eq_ignore_ascii_case(source_modid) {
                    return Value::String(format!("{target_modid}:{path}"));
                }
            }
            Value::String(text)
        }
        Value::Array(values) => Value::Array(values.into_iter()
            .map(|value| rewrite_model_namespace(value, source_modid, target_modid)).collect()),
        Value::Object(values) => Value::Object(values.into_iter()
            .map(|(key, value)| (key, rewrite_model_namespace(value, source_modid, target_modid))).collect()),
        other => other,
    }
}

fn safe_model_reference(reference: &str, source_modid: &str) -> Option<PathBuf> {
    let reference = reference.trim().replace('\\', "/");
    let (namespace, mut path) = reference.split_once(':')
        .map(|(namespace, path)| (Some(namespace), path))
        .unwrap_or((None, reference.as_str()));
    if namespace.is_some_and(|namespace| !namespace.eq_ignore_ascii_case(source_modid)) {
        return None;
    }
    path = path.trim_start_matches('/');
    path = path.strip_prefix("models/").unwrap_or(path);
    path = path.strip_prefix("block/").unwrap_or(path);
    let path = PathBuf::from(path);
    if path.components().any(|part| !matches!(part, Component::Normal(_))) {
        return None;
    }
    Some(path.with_extension("json"))
}

/// Copy exact source blockstate and referenced block-model JSON into the target
/// workspace resource tree while remapping only the imported mod namespace.
pub fn copy_source_block_model_resources(
    source_roots: &[PathBuf],
    source_modid: &str,
    target_modid: &str,
    registry: &str,
    workspace_root: &Path,
) -> Result<Vec<PathBuf>> {
    let registry = snake_case(registry);
    let output_assets = workspace_root.join("src/main/resources/assets").join(target_modid);
    let mut outputs = Vec::new();
    let mut seen_outputs = BTreeSet::new();

    for root in source_roots {
        let bases = [
            root.join("src/main/resources/assets").join(source_modid),
            root.join("assets").join(source_modid),
        ];
        for base in bases {
            if !base.is_dir() {
                continue;
            }
            let blockstate = base.join("blockstates").join(format!("{registry}.json"));
            let mut wanted_models = BTreeSet::new();
            if let Some(payload) = read_json(&blockstate) {
                for reference in collect_model_refs(&payload) {
                    wanted_models.insert(reference);
                }
                let rewritten = rewrite_model_namespace(payload, source_modid, target_modid);
                let destination = output_assets.join("blockstates").join(format!("{registry}.json"));
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&destination, format!("{}\n", serde_json::to_string_pretty(&rewritten)?))
                    .with_context(|| format!("Writing copied blockstate {}", destination.display()))?;
                let key = destination.to_string_lossy().to_ascii_lowercase();
                if seen_outputs.insert(key) {
                    outputs.push(destination);
                }
            }

            wanted_models.insert(registry.clone());
            let models_root = base.join("models/block");
            for reference in wanted_models {
                let Some(relative) = safe_model_reference(&reference, source_modid) else {
                    continue;
                };
                let source = models_root.join(&relative);
                if !source.is_file() {
                    continue;
                }
                let Some(payload) = read_json(&source) else {
                    continue;
                };
                let rewritten = rewrite_model_namespace(payload, source_modid, target_modid);
                let destination = output_assets.join("models/block").join(&relative);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&destination, format!("{}\n", serde_json::to_string_pretty(&rewritten)?))
                    .with_context(|| format!("Writing copied block model {}", destination.display()))?;
                let key = destination.to_string_lossy().to_ascii_lowercase();
                if seen_outputs.insert(key) {
                    outputs.push(destination);
                }
            }
        }
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn copies_blockstate_and_all_variant_multipart_models_with_namespace_remapping() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("src");
        let assets = source.join("assets/demo");
        let blockstate = assets.join("blockstates/duct.json");
        fs::create_dir_all(blockstate.parent().unwrap()).unwrap();
        let blockstate_json = json!({
            "variants": {
                "facing=north": {"model":"demo:block/duct"},
                "facing=south": [{"model":"demo:block/duct_alt","y":180}]
            },
            "multipart": [{"apply":{"model":"demo:block/duct_joint"}}]
        });
        fs::write(&blockstate, serde_json::to_vec(&blockstate_json).unwrap()).unwrap();
        for name in ["duct", "duct_alt", "duct_joint"] {
            let path = assets.join(format!("models/block/{name}.json"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, serde_json::to_vec(&json!({
                "parent":"minecraft:block/cube_all",
                "textures":{"all":"demo:block/duct"}
            })).unwrap()).unwrap();
        }

        let output = temp.path().join("workspace");
        let copied = copy_source_block_model_resources(
            &[source], "demo", "converted", "duct", &output
        ).unwrap();
        assert_eq!(copied.len(), 4);
        let target_state = output.join("src/main/resources/assets/converted/blockstates/duct.json");
        let state: Value = serde_json::from_slice(&fs::read(target_state).unwrap()).unwrap();
        assert_eq!(state["variants"]["facing=north"]["model"], "converted:block/duct");
        assert_eq!(state["variants"]["facing=south"][0]["model"], "converted:block/duct_alt");

        let target_model = output.join("src/main/resources/assets/converted/models/block/duct.json");
        let model: Value = serde_json::from_slice(&fs::read(target_model).unwrap()).unwrap();
        assert_eq!(model["parent"], "minecraft:block/cube_all");
        assert_eq!(model["textures"]["all"], "converted:block/duct");
        assert!(output.join("src/main/resources/assets/converted/models/block/duct_alt.json").is_file());
        assert!(output.join("src/main/resources/assets/converted/models/block/duct_joint.json").is_file());
    }

    #[test]
    fn grass_block_parent_maps_to_native_render_type() {
        let temp = tempdir().unwrap();
        let model = temp.path().join("src/assets/demo/models/block/grass.json");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&model, serde_json::to_vec(&json!({"parent":"minecraft:block/grass_block",
            "textures":{"side":"demo:block/grass","top":"demo:block/grass_top","bottom":"demo:block/dirt","overlay":"minecraft:block/grass_side_overlay"}})).unwrap()).unwrap();
        let info = mcreator_block_model_info(&[temp.path().join("src")], "demo", "grass");
        assert_eq!(info["renderType"], 14);
        assert_eq!(info["textureTop"], "demo:block/grass_top");
    }

    #[test]
    fn cube_all_uses_single_texture_render_type() {
        let temp = tempdir().unwrap();
        let model = temp.path().join("src/assets/demo/models/block/ore.json");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&model, serde_json::to_vec(&json!({"parent":"minecraft:block/cube_all","textures":{"all":"demo:block/ore","particle":"demo:block/ore"}})).unwrap()).unwrap();
        let info = mcreator_block_model_info(&[temp.path().join("src")], "demo", "ore");
        assert_eq!(info["renderType"], 11);
        assert_eq!(info["customModelName"], "Single texture");
    }

    #[test]
    fn plant_cross_parent_maps_to_cross_model_type() {
        let temp = tempdir().unwrap();
        let model = temp.path().join("src/assets/demo/models/block/flower.json");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&model, serde_json::to_vec(&json!({"parent":"minecraft:block/cross","textures":{"cross":"demo:block/flower"}})).unwrap()).unwrap();
        let info = mcreator_block_model_info(&[temp.path().join("src")], "demo", "flower");
        assert_eq!(info["renderType"], 12);
        assert_eq!(info["customModelName"], "Cross model");
    }

    #[test]
    fn blockstate_variants_resolve_referenced_models() {
        let temp = tempdir().unwrap();
        let assets = temp.path().join("src/assets/demo");
        let blockstate = assets.join("blockstates/ore.json");
        let model = assets.join("models/block/ore_variant.json");
        fs::create_dir_all(blockstate.parent().unwrap()).unwrap();
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&blockstate, br#"{"variants":{"":{"model":"demo:block/ore_variant"}}}"#).unwrap();
        fs::write(&model, br#"{"parent":"minecraft:block/cube_all","textures":{"all":"demo:block/ore"}}"#).unwrap();
        let info = mcreator_block_model_info(&[temp.path().join("src")], "demo", "ore");
        assert_eq!(info["customModelName"], "Single texture");
        assert!(info["modelPath"].as_str().unwrap().ends_with("ore_variant.json"));
    }

    #[test]
    fn model_texture_is_copied_with_animation_metadata() {
        let temp = tempdir().unwrap();
        let src_root = temp.path().join("src");
        let model = src_root.join("assets/demo/models/block/foo.json");
        let texture = src_root.join("assets/demo/textures/block/foo.png");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::create_dir_all(texture.parent().unwrap()).unwrap();
        fs::write(&model, b"{}").unwrap();
        fs::write(&texture, b"pngdata").unwrap();
        fs::write(format!("{}.mcmeta", texture.display()), b"{\"animation\":{}}").unwrap();
        let output = temp.path().join("out");
        let stem = copy_model_ref_texture_to_workspace(Some(&model), "demo:block/foo", &output, "block", "foo").unwrap();
        assert_eq!(stem, "foo");
        assert_eq!(fs::read(output.join("block/foo.png")).unwrap(), b"pngdata");
        assert_eq!(fs::read(output.join("block/foo.png.mcmeta")).unwrap(), b"{\"animation\":{}}");
    }

    #[test]
    fn normalizes_all_mcreator_texture_fields_and_keeps_unknown_fields() {
        let info = json!({
            "texture":"minecraft:block/stone",
            "textureTop":"demo:block/top",
            "textureBack":"minecraft:item/stick",
            "modelPath":"source/models/block/test.json",
            "extra":"preserved"
        });
        let normalized = normalize_block_model_info_textures(info);
        assert_eq!(normalized["texture"], "minecraft:stone");
        assert_eq!(normalized["textureTop"], "demo:block/top");
        assert_eq!(normalized["textureBack"], "minecraft:stick");
        assert_eq!(normalized["modelPath"], "source/models/block/test.json");
        assert_eq!(normalized["extra"], "preserved");
    }

    #[test]
    fn minecraft_textures_remain_namespaced() {
        assert_eq!(copy_model_ref_texture_to_workspace(None, "minecraft:block/diamond_ore", Path::new("."), "block", "ore").unwrap(), "minecraft:diamond_ore");
    }

    #[test]
    fn collisions_receive_registry_prefixed_texture_stem() {
        let temp = tempdir().unwrap();
        let src_root = temp.path().join("src");
        let model = src_root.join("assets/demo/models/block/foo.json");
        let texture = src_root.join("assets/demo/textures/block/foo.png");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::create_dir_all(texture.parent().unwrap()).unwrap();
        fs::write(&model, b"{}").unwrap();
        fs::write(&texture, b"source-texture").unwrap();
        let output = temp.path().join("out/block");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("foo.png"), b"different-existing-texture").unwrap();
        let stem = copy_model_ref_texture_to_workspace(Some(&model), "demo:block/foo", &temp.path().join("out"), "block", "ore").unwrap();
        assert_eq!(stem, "ore_foo");
        assert_eq!(fs::read(output.join("ore_foo.png")).unwrap(), b"source-texture");
    }
}
