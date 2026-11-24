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
        if !self.render_instances.is_empty() {
            return self.parts.iter().map(|part| {
                let instances = self.render_instances.get(&part.name)
                    .map(|items| items.len()).unwrap_or(0).max(1);
                part.cubes.len() * instances
            }).sum();
        }
        self.parts.iter().map(|part| part.cubes.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
