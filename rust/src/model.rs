use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Geometry primitive mirrored from the Beta 12 model pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cube {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
    #[serde(default)]
    pub uv_x: i32,
    #[serde(default)]
    pub uv_y: i32,
    #[serde(default)]
    pub mirror: bool,
    #[serde(default)]
    pub inflate: f64,
    #[serde(default = "default_cube_name")]
    pub name: String,
}

fn default_cube_name() -> String { "cube".to_owned() }

/// A hierarchical model bone / ModelRenderer part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Part {
    pub name: String,
    pub variable: String,
    pub parent_variable: String,
    #[serde(default)]
    pub offset: [f64; 3],
    #[serde(default)]
    pub rotation: [f64; 3],
    #[serde(default)]
    pub cubes: Vec<Cube>,
    /// Parent is a stable name instead of an owning pointer, keeping the IR serializable.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default)]
    pub source_index: usize,
    #[serde(default)]
    pub source_name: String,
    #[serde(default)]
    pub render_instance_index: usize,
    #[serde(default)]
    pub render_instance_spec: Value,
    #[serde(default = "default_origin")]
    pub origin: [f64; 3],
    #[serde(default)]
    pub group_uuid: String,
}

fn default_origin() -> [f64; 3] { [0.0, 24.0, 0.0] }

/// Engine-neutral model IR. Source-specific evidence stays attached so exporters
/// do not have to guess how a value was recovered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedModel {
    pub class_name: String,
    pub texture_width: u32,
    pub texture_height: u32,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub animation_analysis: Value,
    #[serde(default)]
    pub animation_logic: String,
    #[serde(default)]
    pub baked_animation_json: Value,
    #[serde(default)]
    pub blockbench_animations: Vec<Value>,
    #[serde(default)]
    pub animation_source: String,
    #[serde(default)]
    pub animation_resource: String,
    #[serde(default)]
    pub animation_resource_path: String,
    #[serde(default = "default_source_format")]
    pub source_format: String,
    #[serde(default)]
    pub primary_texture: String,
    #[serde(default)]
    pub texture_variants: Vec<String>,
    #[serde(default)]
    pub auxiliary_textures: Vec<String>,
    #[serde(default)]
    pub texture_evidence: Vec<String>,
    #[serde(default)]
    pub variant_model_outputs: Vec<String>,
    #[serde(default)]
    pub constructor_args: Vec<Value>,
    #[serde(default)]
    pub scalar_fields: BTreeMap<String, f64>,
    #[serde(default)]
    pub model_bounds: Value,
    #[serde(default)]
    pub linked_entities: Vec<String>,
    #[serde(default)]
    pub linked_renderers: Vec<String>,
    #[serde(default)]
    pub cached_cube_count: Option<usize>,
    #[serde(default)]
    pub render_instances: BTreeMap<String, Vec<Value>>,
    #[serde(default)]
    pub default_render_parts: Vec<String>,
    #[serde(default)]
    pub render_runtime_rotations: Vec<Value>,
}

fn default_source_format() -> String { "modern_mojmap_java".to_owned() }

impl ParsedModel {
    /// Match the legacy cube-count rule, including expanded render instances.
    pub fn cube_count(&self) -> usize {
        if let Some(cached) = self.cached_cube_count {
            return cached;
        }
        // Before expansion, the instance table carries multiplicity. After
        // expansion, each copy is a real Part and must not be multiplied again.
        if !self.render_instances.is_empty()
            && !self.parts.iter().any(|part| part.render_instance_index > 0)
        {
            return self.parts.iter().map(|part| {
                let instances = self.render_instances.get(&part.name)
                    .map(|items| items.len()).unwrap_or(0).max(1);
                part.cubes.len() * instances
            }).sum();
        }
        self.parts.iter().map(|part| part.cubes.len()).sum()
    }
}


fn vec3_from_json(value: &Value) -> Option<[f64; 3]> {
    let values = value.as_array()?;
    if values.len() != 3 { return None; }
    Some([values[0].as_f64()?, values[1].as_f64()?, values[2].as_f64()?])
}

/// Expand source-confirmed repeated render calls into real model parts. This
/// mirrors Beta 12 behavior: the first render updates the original part and
/// each subsequent render becomes a transformed clone.
pub fn expand_render_instances(model: &mut ParsedModel) {
    if model.render_instances.is_empty()
        || model.parts.iter().any(|part| part.render_instance_index > 0)
    {
        return;
    }

    let source_parts = std::mem::take(&mut model.parts);
    let mut expanded = Vec::with_capacity(source_parts.len());
    for mut part in source_parts {
        if part.source_name.is_empty() {
            part.source_name = part.name.clone();
        }
        let specs = model.render_instances.get(&part.name).cloned().unwrap_or_default();
        if specs.is_empty() {
            expanded.push(part);
            continue;
        }

        if let Some(first) = specs.first() {
            if let Some(position) = first.get("position").and_then(vec3_from_json) {
                part.offset = position;
            }
            if let Some(rotation) = first.get("rotation").and_then(vec3_from_json) {
                for axis in 0..3 {
                    part.rotation[axis] += rotation[axis];
                }
            }
        }
        expanded.push(part.clone());

        for (instance_index, spec) in specs.iter().enumerate().skip(1) {
            let rotation = spec.get("rotation").and_then(vec3_from_json).unwrap_or([0.0; 3]);
            let position = spec.get("position").and_then(vec3_from_json).unwrap_or(part.offset);
            let mut clone = part.clone();
            clone.name = format!("{}_{}", part.name, instance_index + 1);
            clone.variable = format!("{}_{}", part.variable, instance_index + 1);
            clone.parent_variable = part.parent_variable.clone();
            clone.offset = position;
            clone.rotation = [
                part.rotation[0] + rotation[0],
                part.rotation[1] + rotation[1],
                part.rotation[2] + rotation[2],
            ];
            clone.cubes = part.cubes.clone();
            clone.parent = part.parent.clone();
            clone.children = part.children.clone();
            clone.source_index = part.source_index;
            clone.source_name = part.source_name.clone();
            clone.render_instance_index = instance_index;
            clone.render_instance_spec = spec.clone();
            expanded.push(clone);
        }
    }
    model.parts = expanded;
    model.cached_cube_count = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_render_instances_into_transformed_part_copies() {
        let mut model = ParsedModel::new("ModelRotator".into(), 64, 32, vec![Part {
            name: "wing".into(), variable: "wing".into(), parent_variable: "root".into(),
            offset: [1.0, 2.0, 3.0], rotation: [0.0; 3],
            cubes: vec![Cube {
                x: 0.0, y: 0.0, z: 0.0, dx: 2.0, dy: 2.0, dz: 2.0,
                uv_x: 0, uv_y: 0, mirror: false, inflate: 0.0, name: "wing_cube".into()
            }],
            parent: None, children: Vec::new(), source_index: 4, source_name: "wing".into(),
            render_instance_index: 0, render_instance_spec: Value::Null,
            origin: [0.0, 24.0, 0.0], group_uuid: String::new()
        }], Vec::new());
        model.render_instances.insert("wing".into(), vec![
            serde_json::json!({"rotation":[0.0,0.0,0.0],"instance_index":0,"instance_count":3}),
            serde_json::json!({"rotation":[0.0,0.0,1.0],"position":[4.0,5.0,6.0],"instance_index":1,"instance_count":3}),
            serde_json::json!({"rotation":[0.0,0.0,2.0],"instance_index":2,"instance_count":3})
        ]);

        assert_eq!(model.cube_count(), 3);
        expand_render_instances(&mut model);
        assert_eq!(model.parts.len(), 3);
        assert_eq!(model.parts[1].name, "wing_2");
        assert_eq!(model.parts[2].name, "wing_3");
        assert_eq!(model.parts[1].offset, [4.0, 5.0, 6.0]);
        assert_eq!(model.parts[1].rotation, [0.0, 0.0, 1.0]);
        assert_eq!(model.parts[2].rotation, [0.0, 0.0, 2.0]);
        assert_eq!(model.parts[2].source_name, "wing");
        assert_eq!(model.parts[2].render_instance_index, 2);
        assert_eq!(model.cube_count(), 3);
        expand_render_instances(&mut model);
        assert_eq!(model.parts.len(), 3);
    }

    #[test]
    fn cube_count_matches_legacy_render_instance_semantics() {
        let model = ParsedModel {
            class_name: "Example".into(),
            texture_width: 64,
            texture_height: 32,
            parts: vec![Part {
                name: "body".into(),
                variable: "body".into(),
                parent_variable: "root".into(),
                cubes: vec![Cube {
                    x: 0.0, y: 0.0, z: 0.0, dx: 1.0, dy: 2.0, dz: 3.0,
                    uv_x: 0, uv_y: 0, mirror: false, inflate: 0.0, name: "cube".into(),
                }; 3],
                offset: [0.0; 3], rotation: [0.0; 3], parent: None,
                children: vec![], source_index: 0, source_name: String::new(),
                render_instance_index: 0, render_instance_spec: Value::Null,
                origin: [0.0,24.0,0.0], group_uuid: String::new(),
            }],
            warnings: vec![], animation_analysis: Value::Null, animation_logic: String::new(),
            baked_animation_json: Value::Null, blockbench_animations: vec![],
            animation_source: String::new(), animation_resource: String::new(),
            animation_resource_path: String::new(), source_format: default_source_format(),
            primary_texture: String::new(), texture_variants: vec![], auxiliary_textures: vec![],
            texture_evidence: vec![], variant_model_outputs: vec![], constructor_args: vec![],
            scalar_fields: BTreeMap::new(), model_bounds: Value::Null, linked_entities: vec![],
            linked_renderers: vec![], cached_cube_count: None,
            render_instances: BTreeMap::from([("body".into(), vec![Value::Null; 8])]),
            default_render_parts: vec![], render_runtime_rotations: vec![],
        };
        assert_eq!(model.cube_count(), 24);
    }
}


impl ParsedModel {
    pub fn new(class_name: String, texture_width: u32, texture_height: u32, parts: Vec<Part>, warnings: Vec<String>) -> Self {
        Self {
            class_name, texture_width, texture_height, parts, warnings,
            animation_analysis: serde_json::json!({}),
            animation_logic: String::new(),
            baked_animation_json: serde_json::json!({}),
            blockbench_animations: Vec::new(),
            animation_source: String::new(),
            animation_resource: String::new(),
            animation_resource_path: String::new(),
            source_format: default_source_format(),
            primary_texture: String::new(),
            texture_variants: Vec::new(),
            auxiliary_textures: Vec::new(),
            texture_evidence: Vec::new(),
            variant_model_outputs: Vec::new(),
            constructor_args: Vec::new(),
            scalar_fields: BTreeMap::new(),
            model_bounds: serde_json::json!({}),
            linked_entities: Vec::new(),
            linked_renderers: Vec::new(),
            cached_cube_count: None,
            render_instances: BTreeMap::new(),
            default_render_parts: Vec::new(),
            render_runtime_rotations: Vec::new(),
        }
    }
}
