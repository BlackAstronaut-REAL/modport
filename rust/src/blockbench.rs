use anyhow::{Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::model::{ParsedModel, Part};

const UUID_NAMESPACE: &str = "30f30b00-8b6d-4d33-85d2-a55bc1f487a9";

fn clean_number(value: f64) -> Value {
    if value.abs() < 1e-10 { return json!(0); }
    let rounded = value.round();
    if (value - rounded).abs() < 1e-9 { return json!(rounded as i64); }
    json!((value * 100_000_000.0).round() / 100_000_000.0)
}

fn vector(values: [f64; 3]) -> Value {
    json!([clean_number(values[0]), clean_number(values[1]), clean_number(values[2])])
}

/// Keep Beta 12 UUIDs byte-for-byte stable across Python and Rust output.
pub fn deterministic_uuid(model_name: &str, kind: &str, index: usize, name: &str) -> String {
    let namespace = Uuid::parse_str(UUID_NAMESPACE).expect("fixed UUID namespace is valid");
    Uuid::new_v5(&namespace, format!("{model_name}:{kind}:{index}:{name}").as_bytes()).to_string()
}

fn calculate_origins(parts: &[Part]) -> Result<Vec<[f64; 3]>> {
    let by_name: HashMap<String, usize> = parts.iter().enumerate().map(|(i, p)| (p.name.clone(), i)).collect();
    fn visit(
        index: usize,
        parts: &[Part],
        by_name: &HashMap<String, usize>,
        cache: &mut [Option<[f64; 3]>],
        visiting: &mut HashSet<usize>,
    ) -> Result<[f64; 3]> {
        if let Some(value) = cache[index] { return Ok(value); }
        if !visiting.insert(index) {
            anyhow::bail!("Cyclic model hierarchy involving '{}'", parts[index].name);
        }
        let offset = parts[index].offset;
        let value = if let Some(parent) = parts[index].parent.as_ref().and_then(|name| by_name.get(name)).copied() {
            let parent_origin = visit(parent, parts, by_name, cache, visiting)?;
            [parent_origin[0] - offset[0], parent_origin[1] - offset[1], parent_origin[2] + offset[2]]
        } else {
            [-offset[0], 24.0 - offset[1], offset[2]]
        };
        visiting.remove(&index);
        cache[index] = Some(value);
        Ok(value)
    }

    let mut cache = vec![None; parts.len()];
    let mut visiting = HashSet::new();
    for index in 0..parts.len() { visit(index, parts, &by_name, &mut cache, &mut visiting)?; }
    Ok(cache.into_iter().map(|value| value.unwrap_or([0.0, 24.0, 0.0])).collect())
}

fn embed_textures(model_name: &str, paths: &[PathBuf]) -> Result<Vec<Value>> {
    let mut unique = Vec::<PathBuf>::new();
    let mut seen_paths = HashSet::<PathBuf>::new();
    let mut basename_counts = HashMap::<String, usize>::new();

    for path in paths {
        if !path.is_file() { continue; }
        let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if seen_paths.insert(resolved.clone()) {
            let basename = resolved.file_name().and_then(|v| v.to_str()).unwrap_or("texture.png").to_ascii_lowercase();
            *basename_counts.entry(basename).or_insert(0) += 1;
            unique.push(resolved);
        }
    }

    let mut used_names = HashSet::<String>::new();
    let mut result = Vec::new();
    for (index, path) in unique.iter().enumerate() {
        let original_name = path.file_name().and_then(|v| v.to_str()).unwrap_or("texture.png");
        let lower = original_name.to_ascii_lowercase();
        let mut display_name = if basename_counts.get(&lower).copied().unwrap_or(0) > 1 {
            format!("{}_{}", path.parent().and_then(|v| v.file_name()).and_then(|v| v.to_str()).unwrap_or("textures"), original_name)
        } else {
            original_name.to_owned()
        };
        let initial = display_name.clone();
        let mut suffix = 2usize;
        while !used_names.insert(display_name.to_ascii_lowercase()) {
            let p = std::path::Path::new(&initial);
            display_name = format!("{}_{}{}", p.file_stem().and_then(|v| v.to_str()).unwrap_or("texture"), suffix, p.extension().map(|v| format!(".{}", v.to_string_lossy())).unwrap_or_default());
            suffix += 1;
        }

        let bytes = fs::read(path).with_context(|| format!("Reading embedded texture {}", path.display()))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let mime = match path.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
            "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "webp" => "image/webp", _ => "application/octet-stream",
        };
        let id = deterministic_uuid(model_name, "texture", index, &path.to_string_lossy());
        result.push(json!({
            "name": display_name,
            "folder": "",
            "namespace": "",
            "id": id,
            "uuid": id,
            "particle": false,
            "render_mode": "normal",
            "internal": true,
            "saved": true,
            "selected": index == 0,
            "path": display_name,
            "source": format!("data:{mime};base64,{encoded}"),
            "moj_model_bridge_source": path.to_string_lossy()
        }));
    }
    Ok(result)
}

fn outliner_node(index: usize, parts: &[Part], group_ids: &[String], cube_ids: &[Vec<String>], by_name: &HashMap<String, usize>) -> Value {
    let mut children: Vec<Value> = cube_ids[index].iter().map(|id| json!(id)).collect();
    let mut nested: Vec<usize> = parts[index].children.iter().filter_map(|name| by_name.get(name).copied()).collect();
    nested.sort_by_key(|child| parts[*child].source_index);
    for child in nested { children.push(outliner_node(child, parts, group_ids, cube_ids, by_name)); }
    json!({"uuid": group_ids[index], "isOpen": true, "children": children})
}

/// Write the native Blockbench 5.x modded-entity document using Beta 12's
/// coordinate conversion and stable UUID naming scheme.
pub fn model_to_bbmodel(model: &mut ParsedModel, source_file: &str, texture_paths: &[PathBuf]) -> Result<Value> {
    let origins = calculate_origins(&model.parts)?;
    let textures = embed_textures(&model.class_name, texture_paths)?;
    let by_name: HashMap<String, usize> = model.parts.iter().enumerate().map(|(i, p)| (p.name.clone(), i)).collect();

    let mut elements = Vec::<Value>::new();
    let mut groups = Vec::<Value>::new();
    let mut group_ids = Vec::<String>::new();
    let mut cube_ids = Vec::<Vec<String>>::new();
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];

    for (part_index, part) in model.parts.iter_mut().enumerate() {
        let origin = origins[part_index];
        let group_id = deterministic_uuid(&model.class_name, "group", part_index, &part.name);
        part.group_uuid = group_id.clone();
        group_ids.push(group_id.clone());

        let rotation = [
            -part.rotation[0].to_degrees(),
            -part.rotation[1].to_degrees(),
            part.rotation[2].to_degrees(),
        ];
        groups.push(json!({
            "uuid": group_id,
            "export": true,
            "locked": false,
            "origin": vector(origin),
            "rotation": vector(rotation),
            "color": part_index % 8,
            "name": part.name,
            "children": [],
            "reset": false,
            "shade": true,
            "mirror_uv": false,
            "selected": false,
            "visibility": true,
            "autouv": 0,
            "isOpen": true,
            "primary_selected": false
        }));

        let mut own_cubes = Vec::<String>::new();
        for (cube_index, cube) in part.cubes.iter().enumerate() {
            // Exact inverse of Beta 12's Blockbench flip-Y conversion.
            let to_x = origin[0] - cube.x;
            let from_x = to_x - cube.dx;
            let to_y = origin[1] - cube.y;
            let from_y = to_y - cube.dy;
            let from_z = origin[2] + cube.z;
            let to_z = from_z + cube.dz;
            for (axis, pair) in [(from_x, to_x), (from_y, to_y), (from_z, to_z)].iter().enumerate() {
                minimum[axis] = minimum[axis].min(pair.0).min(pair.1);
                maximum[axis] = maximum[axis].max(pair.0).max(pair.1);
            }

            let cube_id = deterministic_uuid(&model.class_name, &format!("cube:{part_index}"), cube_index, &cube.name);
            own_cubes.push(cube_id.clone());
            let texture_index = if textures.is_empty() { Value::Null } else { json!(0) };
            let faces = json!({
                "north": {"uv": [0,0,0,0], "texture": texture_index},
                "east":  {"uv": [0,0,0,0], "texture": texture_index},
                "south": {"uv": [0,0,0,0], "texture": texture_index},
                "west":  {"uv": [0,0,0,0], "texture": texture_index},
                "up":    {"uv": [0,0,0,0], "texture": texture_index},
                "down":  {"uv": [0,0,0,0], "texture": texture_index}
            });
            elements.push(json!({
                "name": cube.name,
                "box_uv": true,
                "rescale": false,
                "locked": false,
                "light_emission": 0,
                "render_order": "default",
                "allow_mirror_modeling": true,
                "from": [clean_number(from_x), clean_number(from_y), clean_number(from_z)],
                "to": [clean_number(to_x), clean_number(to_y), clean_number(to_z)],
                "autouv": 0,
                "color": part_index % 8,
                "origin": vector(origin),
                "uv_offset": [cube.uv_x, cube.uv_y],
                "mirror_uv": cube.mirror,
                "inflate": clean_number(cube.inflate),
                "faces": faces,
                "type": "cube",
                "uuid": cube_id
            }));
        }
        cube_ids.push(own_cubes);
    }

    let mut root_parts: Vec<usize> = model.parts.iter().enumerate()
        .filter(|(_, part)| part.parent.as_ref().and_then(|name| by_name.get(name)).is_none())
        .map(|(index, _)| index).collect();
    root_parts.sort_by_key(|index| model.parts[*index].source_index);
    let outliner: Vec<Value> = root_parts.iter()
        .map(|index| outliner_node(*index, &model.parts, &group_ids, &cube_ids, &by_name))
        .collect();

    if !minimum[0].is_finite() {
        minimum = [-8.0, 0.0, -8.0];
        maximum = [8.0, 16.0, 8.0];
    }
    let size = [maximum[0] - minimum[0], maximum[1] - minimum[1], maximum[2] - minimum[2]];
    let center = [(minimum[0] + maximum[0]) / 2.0, (minimum[1] + maximum[1]) / 2.0, (minimum[2] + maximum[2]) / 2.0];
    let visible_box = [
        1.0f64.max(size[0] / 16.0).max(size[2] / 16.0),
        1.0f64.max(size[1] / 16.0),
        (center[1] - 24.0) / 16.0,
    ];
    model.model_bounds = json!({
        "min": vector(minimum), "max": vector(maximum), "size": vector(size),
        "center": vector(center), "visible_box": vector(visible_box)
    });

    let bridge_fields = json!({
        "version": crate::VERSION,
        "render_instance_sources": model.render_instances,
        "source_file": source_file,
        "source_format": model.source_format,
        "primary_texture": if model.primary_texture.is_empty() { Value::Null } else { json!(model.primary_texture) },
        "texture_variants": model.texture_variants,
        "auxiliary_textures_skipped": model.auxiliary_textures,
        "variant_model_outputs": model.variant_model_outputs,
        "constructor_args": model.constructor_args,
        "scalar_fields": model.scalar_fields,
        "model_bounds": model.model_bounds,
        "linked_entities": model.linked_entities,
        "linked_renderers": model.linked_renderers,
        "notes": "Native Rust model serializer. Procedural animation source and analysis are preserved in sidecar files; Blockbench keyframe sampling is pending."
    });

    Ok(json!({
        "meta": {"format_version": "5.0", "model_format": "modded_entity", "box_uv": true},
        "name": model.class_name,
        "model_identifier": model.class_name,
        "geometry_name": model.class_name,
        "modded_entity_entity_class": "Entity",
        "modded_entity_flip_y": true,
        "visible_box": vector(visible_box),
        "variable_placeholders": "",
        "variable_placeholder_buttons": [],
        "timeline_setups": [],
        "unhandled_root_fields": {"moj_model_bridge": bridge_fields},
        "resolution": {"width": model.texture_width, "height": model.texture_height},
        "elements": elements,
        "groups": groups,
        "outliner": outliner,
        "textures": textures,
        "animations": model.blockbench_animations
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cube, ParsedModel, Part};

    #[test]
    fn beta12_uuid_namespace_is_reproducible() {
        assert_eq!(
            deterministic_uuid("ModelTest", "group", 0, "body"),
            deterministic_uuid("ModelTest", "group", 0, "body")
        );
        assert_ne!(
            deterministic_uuid("ModelTest", "group", 0, "body"),
            deterministic_uuid("ModelTest", "group", 1, "body")
        );
    }

    #[test]
    fn serializes_nested_parts_and_preserves_beta12_coordinates() {
        let mut model = ParsedModel::new("ModelSerializer".into(), 64, 32, vec![
            Part {
                name: "body".into(), variable: "body".into(), parent_variable: "root".into(),
                offset: [0.0, 0.0, 0.0], rotation: [0.0; 3],
                cubes: vec![Cube { x: -1.0, y: 0.0, z: -1.0, dx: 2.0, dy: 3.0, dz: 2.0, uv_x: 4, uv_y: 8, mirror: false, inflate: 0.25, name: "body_cube".into() }],
                parent: None, children: vec!["head".into()], source_index: 0, source_name: "body".into(),
                render_instance_index: 0, render_instance_spec: Value::Null, origin: [0.0,24.0,0.0], group_uuid: String::new()
            },
            Part {
                name: "head".into(), variable: "head".into(), parent_variable: "body".into(),
                offset: [0.0, -6.0, 0.0], rotation: [0.0; 3], cubes: Vec::new(),
                parent: Some("body".into()), children: Vec::new(), source_index: 1, source_name: "head".into(),
                render_instance_index: 0, render_instance_spec: Value::Null, origin: [0.0,24.0,0.0], group_uuid: String::new()
            }
        ], Vec::new());
        let output = model_to_bbmodel(&mut model, "ModelSerializer.java", &[]).unwrap();
        assert_eq!(output["elements"].as_array().unwrap().len(), 1);
        assert_eq!(output["elements"][0]["from"], json!([-1, 21, -1]));
        assert_eq!(output["elements"][0]["to"], json!([1, 24, 1]));
        assert_eq!(output["elements"][0]["uv_offset"], json!([4, 8]));
        assert_eq!(output["outliner"][0]["children"].as_array().unwrap().len(), 2);
        assert_eq!(output["visible_box"][0], 1.0);
    }
}
