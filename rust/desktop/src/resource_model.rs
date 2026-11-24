use anyhow::{bail, Context, Result};
use mojmodelbridge::viewer::{
    ViewerAnimation, ViewerCube, ViewerNode, ViewerScene,
};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

fn read_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).with_context(|| format!("Reading item model {}", path.display()))?;
    serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
        .with_context(|| format!("Parsing item model {}", path.display()))
}

fn asset_namespace(path: &Path) -> String {
    let components = path.components().filter_map(|component| match component {
        Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
        _ => None,
    }).collect::<Vec<_>>();
    components.windows(2)
        .find(|pair| pair[0].eq_ignore_ascii_case("assets"))
        .map(|pair| pair[1].clone())
        .unwrap_or_else(|| "minecraft".to_owned())
}

fn model_file_index(roots: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut files = HashMap::new();
    for root in roots {
        if !root.is_dir() { continue; }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if !entry.file_type().is_file() { continue; }
            let path = entry.into_path();
            let normalized = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            if normalized.contains("/assets/") && normalized.contains("/models/") {
                files.insert(normalized, path);
            }
        }
    }
    files
}

fn resolve_parent_path(path: &Path, parent: &str, index: &HashMap<String, PathBuf>) -> Option<PathBuf> {
    if parent.starts_with("builtin/") { return None; }
    let (namespace, model) = parent.split_once(':')
        .map(|(namespace, model)| (namespace.to_owned(), model.to_owned()))
        .unwrap_or_else(|| (asset_namespace(path), parent.to_owned()));
    let model = model.trim_start_matches('/').trim_end_matches(".json");
    if model.contains("..") { return None; }
    let path_text = path.to_string_lossy().replace('\\', "/");
    let marker = format!("/assets/{namespace}/models/");
    let asset_root = path_text.to_ascii_lowercase().find(&marker.to_ascii_lowercase())
        .map(|offset| PathBuf::from(&path_text[..offset]));
    let root = asset_root.unwrap_or_else(|| path.parent().unwrap_or(path).to_path_buf());
    let candidate = root.join("assets").join(namespace).join("models").join(format!("{model}.json"));
    if candidate.is_file() { return Some(candidate); }
    let key = candidate.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    index.get(&key).cloned()
}

fn merge_model_chain(
    path: &Path,
    index: &HashMap<String, PathBuf>,
    visiting: &mut BTreeSet<PathBuf>,
    depth: usize,
) -> Result<Value> {
    if depth > 16 {
        bail!("Item model parent chain exceeded 16 models at {}", path.display());
    }
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visiting.insert(canonical.clone()) {
        bail!("Cyclic item model parent chain at {}", path.display());
    }
    let payload = read_json(path)?;
    let parent_ref = payload.get("parent").and_then(Value::as_str).map(str::to_owned);
    let mut parent = if let Some(parent_ref) = parent_ref.as_deref() {
        resolve_parent_path(path, parent_ref, index)
            .map(|parent_path| merge_model_chain(&parent_path, index, visiting, depth + 1))
            .transpose()?
            .unwrap_or_else(|| Value::Object(Map::new()))
    } else {
        Value::Object(Map::new())
    };

    if let (Some(base), Some(child)) = (parent.as_object_mut(), payload.as_object()) {
        for (key, value) in child {
            if key == "textures" {
                let inherited = base.entry("textures").or_insert_with(|| Value::Object(Map::new()));
                if let (Some(inherited), Some(child)) = (inherited.as_object_mut(), value.as_object()) {
                    for (texture, reference) in child {
                        inherited.insert(texture.clone(), reference.clone());
                    }
                }
            } else if key == "elements" && !value.as_array().is_some_and(Vec::is_empty) {
                base.insert(key.clone(), value.clone());
            } else if key != "parent" {
                base.insert(key.clone(), value.clone());
            }
        }
    }
    visiting.remove(&canonical);
    Ok(parent)
}

fn resolve_texture_alias(value: &str, textures: &Map<String, Value>, depth: usize) -> String {
    if depth > 12 { return String::new(); }
    let Some(key) = value.strip_prefix('#') else { return value.to_owned(); };
    let Some(next) = textures.get(key).and_then(Value::as_str) else { return String::new(); };
    resolve_texture_alias(next, textures, depth + 1)
}

fn resolve_texture_file(roots: &[PathBuf], namespace: &str, reference: &str) -> Option<PathBuf> {
    let reference = reference.trim();
    if reference.is_empty() || reference.starts_with('#') { return None; }
    let (namespace, path) = reference.split_once(':').unwrap_or((namespace, reference));
    let path = path.trim_start_matches('/').strip_prefix("textures/").unwrap_or(path.trim_start_matches('/'));
    let relative = Path::new(path).with_extension("png");
    if relative.components().any(|component| !matches!(component, Component::Normal(_))) {
        return None;
    }
    for root in roots {
        for candidate in [
            root.join("assets").join(namespace).join("textures").join(&relative),
            root.join("src/main/resources/assets").join(namespace).join("textures").join(&relative),
        ] {
            if candidate.is_file() { return Some(candidate); }
        }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if entry.file_type().is_file()
                && entry.path().extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
                && entry.path().to_string_lossy().replace('\\', "/").to_ascii_lowercase()
                    .ends_with(&format!("/assets/{namespace}/textures/{}", relative.to_string_lossy().replace('\\', "/").to_ascii_lowercase()))
            {
                return Some(entry.into_path());
            }
        }
    }
    None
}

fn resolved_face_texture_path(
    face: Option<&Value>,
    textures: &Map<String, Value>,
    roots: &[PathBuf],
    namespace: &str,
) -> Option<PathBuf> {
    let reference = face?.get("texture")?.as_str()?;
    let resolved = resolve_texture_alias(reference, textures, 0);
    resolve_texture_file(roots, namespace, &resolved)
}

fn face_texture_indices(
    element: &Value,
    textures: &Map<String, Value>,
    roots: &[PathBuf],
    namespace: &str,
    texture_paths: &[PathBuf],
) -> [usize; 6] {
    let faces = element.get("faces").and_then(Value::as_object);
    let names = ["east", "west", "up", "down", "north", "south"];
    std::array::from_fn(|index| {
        let Some(face) = faces.and_then(|faces| faces.get(names[index])) else { return 0; };
        resolved_face_texture_path(Some(face), textures, roots, namespace)
            .and_then(|path| texture_paths.iter().position(|candidate| candidate == &path))
            .unwrap_or(0)
    })
}

fn face_uvs(
    element: &Value,
    textures: &Map<String, Value>,
    roots: &[PathBuf],
    namespace: &str,
    texture_paths: &[PathBuf],
    default_texture_size: [u32; 2],
) -> [[f32; 4]; 6] {
    let default = [
        0.0, 0.0,
        default_texture_size[0] as f32,
        default_texture_size[1] as f32,
    ];
    let faces = element.get("faces").and_then(Value::as_object);
    let names = ["east", "west", "up", "down", "north", "south"];
    std::array::from_fn(|index| {
        let Some(face) = faces.and_then(|faces| faces.get(names[index])) else { return default; };
        let Some(uv) = face.get("uv").and_then(Value::as_array).filter(|uv| uv.len() == 4) else {
            return default;
        };
        let values = uv.iter().filter_map(Value::as_f64).map(|value| value as f32).collect::<Vec<_>>();
        if values.len() != 4 { return default; }
        let size = resolved_face_texture_path(Some(face), textures, roots, namespace)
            .and_then(|path| texture_paths.iter().find(|candidate| **candidate == path))
            .and_then(|path| image::image_dimensions(path).ok())
            .unwrap_or((default_texture_size[0], default_texture_size[1]));
        let sx = size.0 as f32 / 16.0;
        let sy = size.1 as f32 / 16.0;
        [values[0] * sx, values[1] * sy, values[2] * sx, values[3] * sy]
    })
}

fn vector3(value: Option<&Value>, default: [f32; 3]) -> [f32; 3] {
    let Some(values) = value.and_then(Value::as_array) else { return default; };
    if values.len() != 3 { return default; }
    std::array::from_fn(|index| values[index].as_f64().map(|value| value as f32).unwrap_or(default[index]))
}

/// Read a Minecraft Java Edition cuboid item model. This is deliberately a
/// resource-model adapter, separate from the Java ModelRenderer parser.
fn load_resource_model(path: &Path, roots: &[PathBuf], allow_builtin_block_shape: bool) -> Result<(ViewerScene, Vec<PathBuf>)> {
    let index = model_file_index(roots);
    let mut visiting = BTreeSet::new();
    let payload = merge_model_chain(path, &index, &mut visiting, 0)?;
    let elements = payload.get("elements").and_then(Value::as_array).cloned().unwrap_or_default();
    if elements.is_empty() && !allow_builtin_block_shape {
        bail!("Item model {} has no cuboid elements in its own or inherited definition.", path.display());
    }
    let textures = payload.get("textures").and_then(Value::as_object).cloned().unwrap_or_default();
    let namespace = asset_namespace(path);
    let mut texture_paths = Vec::new();
    let mut referenced = BTreeSet::<String>::new();
    let mut texture_references = textures.values()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for element in &elements {
        if let Some(faces) = element.get("faces").and_then(Value::as_object) {
            texture_references.extend(faces.values()
                .filter_map(|face| face.get("texture").and_then(Value::as_str))
                .map(str::to_owned));
        }
    }
    for value in texture_references {
        let resolved = resolve_texture_alias(&value, &textures, 0);
        if resolved.is_empty() || !referenced.insert(resolved.clone()) { continue; }
        if let Some(texture_path) = resolve_texture_file(roots, &namespace, &resolved) {
            if !texture_paths.contains(&texture_path) {
                texture_paths.push(texture_path);
            }
        }
    }

    let mut warnings = Vec::new();
    let dimensions = texture_paths.first()
        .and_then(|path| image::image_dimensions(path).ok())
        .unwrap_or((16, 16));
    let texture_size = [dimensions.0.max(1), dimensions.1.max(1)];
    let mut nodes = Vec::new();
    if elements.is_empty() {
        let shape = builtin_block_shape(path, roots);
        nodes = builtin_block_nodes(&shape, texture_size);
        if nodes.is_empty() {
            bail!("Block model {} has no explicit elements and uses an unsupported built-in parent.", path.display());
        }
    }

    for (element_index, element) in elements.iter().enumerate() {
        let from = vector3(element.get("from"), [0.0, 0.0, 0.0]);
        let to = vector3(element.get("to"), [16.0, 16.0, 16.0]);
        let rotation = element.get("rotation");
        let origin = vector3(rotation.and_then(|rotation| rotation.get("origin")), [8.0, 8.0, 8.0]);
        let relative_origin = [origin[0] - 8.0, origin[1] - 8.0, origin[2] - 8.0];
        let (mut base_rotation, local_translation) = ([0.0_f32; 3], relative_origin);
        if let Some(rotation) = rotation {
            let angle = rotation.get("angle").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let axis = rotation.get("axis").and_then(Value::as_str).unwrap_or("");
            let index = match axis { "x" => Some(0), "y" => Some(1), "z" => Some(2), _ => None };
            if let Some(index) = index {
                base_rotation[index] = angle.to_radians();
            } else {
                warnings.push(format!("Element {element_index} has an unsupported rotation axis '{axis}'."));
            }
            if rotation.get("rescale").and_then(Value::as_bool).unwrap_or(false) {
                warnings.push(format!("Element {element_index} uses rescale; the preview keeps its unscaled cuboid."));
            }
        }

        let absolute_min = [from[0] - 8.0, from[1] - 8.0, from[2] - 8.0];
        let absolute_max = [to[0] - 8.0, to[1] - 8.0, to[2] - 8.0];
        let min = std::array::from_fn(|axis| absolute_min[axis] - local_translation[axis]);
        let max = std::array::from_fn(|axis| absolute_max[axis] - local_translation[axis]);

        let mut selected_texture = String::new();
        if let Some(face_values) = element.get("faces").and_then(Value::as_object) {
            if let Some(reference) = face_values.values().find_map(|face| face.get("texture").and_then(Value::as_str)) {
                selected_texture = resolve_texture_alias(reference, &textures, 0);
            }
        }
        let texture_size = texture_size;
        let face_uvs = face_uvs(
            element, &textures, roots, &namespace, &texture_paths, texture_size,
        );
        let face_texture_indices = face_texture_indices(
            element, &textures, roots, &namespace, &texture_paths,
        );

        nodes.push(ViewerNode {
            name: element.get("name").and_then(Value::as_str)
                .map(str::to_owned).unwrap_or_else(|| format!("element_{element_index}")),
            parent: None,
            local_translation,
            base_rotation,
            cubes: vec![ViewerCube {
                min,
                max,
                uv_origin: [0.0; 2],
                face_uvs: Some(face_uvs),
                face_texture_indices: Some(face_texture_indices),
                mirror: false,
                name: format!("element_{element_index}"),
            }],
        });
        let _ = selected_texture;
    }

    if texture_paths.len() > 1 {
        warnings.push("The resource model references multiple texture images; each cuboid face uses its declared texture when that reference resolves.".to_owned());
    }
    if texture_paths.is_empty() {
        warnings.push("No referenced texture PNG could be resolved for this item model.".to_owned());
    }
    let scene = ViewerScene {
        name: path.file_stem().and_then(|value| value.to_str()).unwrap_or("ItemModel").to_owned(),
        texture_size,
        texture: texture_paths.first().map(|path| path.to_string_lossy().to_string()).unwrap_or_default(),
        texture_variants: texture_paths.iter().map(|path| path.to_string_lossy().to_string()).collect(),
        nodes,
        animations: Vec::<ViewerAnimation>::new(),
        warnings,
    };
    Ok((scene, texture_paths))
}

fn shape_for_builtin_parent(parent: &str, stem: &str) -> Option<&'static str> {
    let parent = parent.to_ascii_lowercase();
    if ["block/cross", "block/tinted_cross", "block/crop", "block/flower"]
        .iter().any(|value| parent.ends_with(value))
        || ["flower", "sapling", "crop", "fern", "tall_grass", "cross"]
            .iter().any(|value| stem.contains(value))
    {
        Some("plant")
    } else if parent.ends_with("block/template_torch_wall") {
        Some("wall_torch")
    } else if ["block/torch", "block/template_torch"]
        .iter().any(|value| parent.ends_with(value))
        || stem.ends_with("torch")
    {
        Some("torch")
    } else if ["redstone_dust", "redstone_wire", "wire", "carpet", "rail", "repeater", "comparator", "pressure_plate"]
        .iter().any(|value| stem.contains(value))
    {
        Some("floor")
    } else if ["block/cube", "block/cube_all", "block/cube_column", "block/orientable"]
        .iter().any(|value| parent.ends_with(value))
    {
        Some("cube")
    } else {
        None
    }
}

fn builtin_block_shape(path: &Path, roots: &[PathBuf]) -> String {
    let index = model_file_index(roots);
    let mut current = path.to_path_buf();
    let mut visited = BTreeSet::<PathBuf>::new();
    let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();

    for _ in 0..16 {
        let canonical = fs::canonicalize(&current).unwrap_or_else(|_| current.clone());
        if !visited.insert(canonical) { break; }
        let Ok(payload) = read_json(&current) else { break; };
        let Some(parent) = payload.get("parent").and_then(Value::as_str) else { break; };
        if let Some(shape) = shape_for_builtin_parent(parent, &stem) {
            return shape.to_owned();
        }
        let Some(next) = resolve_parent_path(&current, parent, &index) else { break; };
        current = next;
    }
    String::new()
}

fn builtin_cube(
    name: &str,
    min: [f32; 3],
    max: [f32; 3],
    rotation: [f32; 3],
    texture_size: [u32; 2],
) -> ViewerNode {
    let width = texture_size[0].max(1) as f32;
    let height = texture_size[1].max(1) as f32;
    ViewerNode {
        name: name.to_owned(),
        parent: None,
        local_translation: [0.0; 3],
        base_rotation: rotation,
        cubes: vec![ViewerCube {
            min,
            max,
            uv_origin: [0.0; 2],
            face_uvs: Some([[0.0, 0.0, width, height]; 6]),
            face_texture_indices: None,
            mirror: false,
            name: format!("{name}_cube"),
        }],
    }
}

fn builtin_block_nodes(shape: &str, texture_size: [u32; 2]) -> Vec<ViewerNode> {
    match shape {
        "cube" => vec![builtin_cube(
            "cube",
            [-8.0, -8.0, -8.0],
            [8.0, 8.0, 8.0],
            [0.0; 3],
            texture_size,
        )],
        "floor" => vec![builtin_cube(
            "floor",
            [-8.0, -8.0, -8.0],
            [8.0, -7.65, 8.0],
            [0.0; 3],
            texture_size,
        )],
        "torch" => vec![builtin_cube(
            "torch",
            [-1.5, -8.0, -1.5],
            [1.5, 3.0, 1.5],
            [0.0; 3],
            texture_size,
        )],
        "wall_torch" => vec![builtin_cube(
            "wall_torch",
            [-1.5, -7.0, -1.5],
            [1.5, 4.0, 1.5],
            [0.32, 0.0, 0.32],
            texture_size,
        )],
        "plant" => vec![
            builtin_cube(
                "plant_front",
                [-7.5, -8.0, -0.18],
                [7.5, 8.0, 0.18],
                [0.0, 45.0_f32.to_radians(), 0.0],
                texture_size,
            ),
            builtin_cube(
                "plant_back",
                [-7.5, -8.0, -0.18],
                [7.5, 8.0, 0.18],
                [0.0, -45.0_f32.to_radians(), 0.0],
                texture_size,
            ),
        ],
        _ => Vec::new(),
    }
}

pub fn load_item_model(path: &Path, roots: &[PathBuf]) -> Result<(ViewerScene, Vec<PathBuf>)> {
    load_resource_model(path, roots, false)
}

pub fn load_block_model(path: &Path, roots: &[PathBuf]) -> Result<(ViewerScene, Vec<PathBuf>)> {
    load_resource_model(path, roots, true)
}

/// Build a preview for legacy blocks that use a vanilla render shape but do not
/// ship modern blockstate/model JSON resources.
pub fn synthetic_block_preview(
    name: &str,
    shape: &str,
    texture: Option<PathBuf>,
) -> Result<(ViewerScene, Vec<PathBuf>)> {
    let texture_paths = texture.into_iter().collect::<Vec<_>>();
    let texture_size = texture_paths.first()
        .and_then(|path| image::image_dimensions(path).ok())
        .unwrap_or((16, 16));
    let texture_size = [texture_size.0.max(1), texture_size.1.max(1)];
    let nodes = builtin_block_nodes(shape, texture_size);
    if nodes.is_empty() {
        bail!("Unsupported synthetic legacy block render shape '{shape}'.");
    }
    let scene = ViewerScene {
        name: name.to_owned(),
        texture_size,
        texture: texture_paths.first().map(|path| path.to_string_lossy().to_string()).unwrap_or_default(),
        texture_variants: texture_paths.iter().map(|path| path.to_string_lossy().to_string()).collect(),
        nodes,
        animations: Vec::<ViewerAnimation>::new(),
        warnings: vec![format!("Preview uses the inferred legacy '{shape}' render shape because this block has no resource model JSON.")],
    };
    Ok((scene, texture_paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn maps_each_custom_block_face_to_its_own_texture_and_uv_scale() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let model_dir = root.join("assets/demo/models/block");
        let texture_dir = root.join("assets/demo/textures/block");
        fs::create_dir_all(&model_dir).unwrap();
        fs::create_dir_all(&texture_dir).unwrap();

        let top = texture_dir.join("top.png");
        let side = texture_dir.join("side.png");
        let bottom = texture_dir.join("bottom.png");
        RgbaImage::new(32, 16).save(&top).unwrap();
        RgbaImage::new(16, 16).save(&side).unwrap();
        RgbaImage::new(16, 16).save(&bottom).unwrap();

        let model = model_dir.join("custom_pillar.json");
        fs::write(&model, r##"{
            "textures": {
                "top": "demo:block/top",
                "side": "demo:block/side",
                "bottom": "demo:block/bottom"
            },
            "elements": [{
                "from": [0,0,0],
                "to": [16,16,16],
                "faces": {
                    "up": {"uv":[0,0,16,16],"texture":"#top"},
                    "down": {"uv":[0,0,16,16],"texture":"#bottom"},
                    "north": {"uv":[0,0,16,16],"texture":"#side"},
                    "south": {"uv":[0,0,16,16],"texture":"#side"},
                    "east": {"uv":[0,0,16,16],"texture":"#side"},
                    "west": {"uv":[0,0,16,16],"texture":"#side"}
                }
            }]
        }"##).unwrap();

        let (scene, paths) = load_block_model(&model, &[root.to_path_buf()]).unwrap();
        let cube = &scene.nodes[0].cubes[0];
        let map = cube.face_texture_indices.unwrap();
        let index = |path: &Path| paths.iter().position(|candidate| candidate == path).unwrap();
        assert_eq!(map[2], index(&top));
        assert_eq!(map[3], index(&bottom));
        assert_eq!(map[0], index(&side));
        assert_eq!(map[1], index(&side));
        assert_eq!(map[4], index(&side));
        assert_eq!(map[5], index(&side));

        let uv = cube.face_uvs.unwrap();
        assert_eq!(uv[2], [0.0, 0.0, 32.0, 16.0]);
        assert_eq!(uv[3], [0.0, 0.0, 16.0, 16.0]);
        assert_eq!(uv[4], [0.0, 0.0, 16.0, 16.0]);
    }

    #[test]
    fn renders_torch_and_wall_torch_as_vertical_models() {
        let (torch, _) = synthetic_block_preview("Torch", "torch", None).unwrap();
        let torch_cube = &torch.nodes[0].cubes[0];
        assert!(torch_cube.max[1] > torch_cube.min[1] + 8.0);
        assert!(torch_cube.max[0] - torch_cube.min[0] < 4.0);

        let (wall, _) = synthetic_block_preview("Wall Torch", "wall_torch", None).unwrap();
        let wall_cube = &wall.nodes[0].cubes[0];
        assert!(wall_cube.max[1] > wall_cube.min[1] + 8.0);
        assert_ne!(wall.nodes[0].base_rotation[2], 0.0);
    }

    #[test]
    fn synthesizes_legacy_block_shape_previews_with_source_texture() {
        let temp = tempfile::tempdir().unwrap();
        let texture = temp.path().join("stone.png");
        RgbaImage::new(32, 32).save(&texture).unwrap();

        let (scene, textures) = synthetic_block_preview("Ancient Stone", "cube", Some(texture.clone())).unwrap();
        assert_eq!(scene.name, "Ancient Stone");
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.nodes[0].cubes[0].min, [-8.0, -8.0, -8.0]);
        assert_eq!(scene.texture_size, [32, 32]);
        assert_eq!(textures, vec![texture]);

        let (plant, _) = synthetic_block_preview("Blue Flower", "plant", None).unwrap();
        assert_eq!(plant.nodes.len(), 2);
    }

    #[test]
    fn renders_vanilla_cube_all_and_cross_plant_block_models() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let model_dir = root.join("assets/demo/models/block");
        let texture_dir = root.join("assets/demo/textures/block");
        fs::create_dir_all(&model_dir).unwrap();
        fs::create_dir_all(&texture_dir).unwrap();
        let texture_path = texture_dir.join("flower.png");
        RgbaImage::new(16, 16).save(&texture_path).unwrap();

        let cube_path = model_dir.join("stone_like.json");
        fs::write(&cube_path, r#"{"parent":"minecraft:block/cube_all","textures":{"all":"demo:block/flower"}}"#).unwrap();
        let (cube_scene, cube_textures) = load_block_model(&cube_path, &[root.to_path_buf()]).unwrap();
        assert_eq!(cube_scene.nodes.len(), 1);
        assert_eq!(cube_scene.nodes[0].cubes[0].min, [-8.0, -8.0, -8.0]);
        assert_eq!(cube_textures, vec![texture_path.clone()]);

        let plant_path = model_dir.join("flower_cross.json");
        fs::write(&plant_path, r#"{"parent":"minecraft:block/cross","textures":{"cross":"demo:block/flower"}}"#).unwrap();
        let (plant_scene, plant_textures) = load_block_model(&plant_path, &[root.to_path_buf()]).unwrap();
        assert_eq!(plant_scene.nodes.len(), 2);
        assert_eq!(plant_scene.nodes[0].name, "plant_front");
        assert_eq!(plant_scene.nodes[1].name, "plant_back");
        assert_eq!(plant_scene.nodes[0].base_rotation[1], 45.0_f32.to_radians());
        assert_eq!(plant_textures, vec![texture_path]);
    }

    #[test]
    fn follows_custom_parent_chain_to_builtin_cross_shape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let models = root.join("assets/demo/models/block");
        let textures = root.join("assets/demo/textures/block");
        fs::create_dir_all(&models).unwrap();
        fs::create_dir_all(&textures).unwrap();
        let texture_path = textures.join("flower.png");
        RgbaImage::new(16, 16).save(&texture_path).unwrap();
        let base = models.join("shared_plant.json");
        fs::write(&base, r#"{"parent":"minecraft:block/cross","textures":{"cross":"demo:block/flower"}}"#).unwrap();
        let derived = models.join("custom_flower.json");
        fs::write(&derived, r#"{"parent":"demo:block/shared_plant"}"#).unwrap();

        let (scene, resolved_textures) = load_block_model(&derived, &[root.to_path_buf()]).unwrap();
        assert_eq!(scene.nodes.len(), 2);
        assert_eq!(resolved_textures, vec![texture_path]);
    }

    #[test]
    fn renders_flat_floor_block_models_as_thin_cuboids() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let model_dir = root.join("assets/demo/models/block");
        fs::create_dir_all(&model_dir).unwrap();
        let model_path = model_dir.join("redstone_dust.json");
        fs::write(&model_path, r#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/redstone_dust_line"}}"#).unwrap();

        let (scene, _) = load_block_model(&model_path, &[root.to_path_buf()]).unwrap();
        let cube = &scene.nodes[0].cubes[0];
        assert_eq!(cube.min[1], -8.0);
        assert_eq!(cube.max[1], -7.65);
    }

    #[test]
    fn parses_item_model_geometry_and_resolves_texture_face_uvs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let model_path = root.join("assets/demo/models/item/ancient_sword.json");
        let texture_path = root.join("assets/demo/textures/item/ancient_sword.png");
        fs::create_dir_all(model_path.parent().unwrap()).unwrap();
        fs::create_dir_all(texture_path.parent().unwrap()).unwrap();
        RgbaImage::new(32, 32).save(&texture_path).unwrap();
        fs::write(&model_path, r##"{
            "textures":{"layer0":"demo:item/ancient_sword"},
            "elements":[{
                "name":"blade",
                "from":[6,0,7],
                "to":[10,16,9],
                "faces":{
                    "north":{"uv":[6,0,10,16],"texture":"#layer0"},
                    "south":{"uv":[6,0,10,16],"texture":"#layer0"},
                    "east":{"uv":[6,0,9,16],"texture":"#layer0"},
                    "west":{"uv":[6,0,9,16],"texture":"#layer0"},
                    "up":{"uv":[6,0,10,2],"texture":"#layer0"},
                    "down":{"uv":[6,0,10,2],"texture":"#layer0"}
                }
            }]
        }"##).unwrap();

        let (scene, textures) = load_item_model(&model_path, &[root.to_path_buf()]).unwrap();
        assert_eq!(textures, vec![texture_path]);
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.nodes[0].name, "blade");
        assert_eq!(scene.nodes[0].cubes.len(), 1);
        let cube = &scene.nodes[0].cubes[0];
        assert_eq!(cube.face_uvs.unwrap()[0], [12.0, 0.0, 18.0, 32.0]);
        assert_eq!(scene.texture_size, [32, 32]);
    }

    #[test]
    fn inherits_parent_item_model_textures_and_elements() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let models = root.join("assets/demo/models/item");
        let textures = root.join("assets/demo/textures/item");
        fs::create_dir_all(&models).unwrap();
        fs::create_dir_all(&textures).unwrap();
        let texture_path = textures.join("shared.png");
        RgbaImage::new(16, 16).save(&texture_path).unwrap();
        let parent = models.join("base_tool.json");
        fs::write(&parent, r##"{
            "textures":{"layer0":"demo:item/shared"},
            "elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{
                "north":{"texture":"#layer0"}
            }}]
        }"##).unwrap();
        let child = models.join("custom_tool.json");
        fs::write(&child, r#"{"parent":"demo:item/base_tool"}"#).unwrap();

        let (scene, found_textures) = load_item_model(&child, &[root.to_path_buf()]).unwrap();
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(found_textures, vec![texture_path]);
    }
}
