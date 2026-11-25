use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::blockbench::deterministic_uuid;
use crate::model::ParsedModel;

/// Renderer-neutral geometry for one Minecraft ModelBox/Blockbench cube.
/// Coordinates are local to the owning part's pivot and use Blockbench axes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewerCube {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub uv_origin: [f32; 2],
    /// Optional per-face UV rectangles in source texture pixels. Minecraft
    /// resource item models use independent face UVs rather than a ModelBox net.
    pub face_uvs: Option<[[f32; 4]; 6]>,
    /// Texture index per face in ViewerScene.texture_variants. None uses the
    /// scene's active texture, as with legacy ModelRenderer meshes.
    #[serde(default)]
    pub face_texture_indices: Option<[usize; 6]>,
    /// Which of the six Minecraft resource-model faces actually exist.
    /// None preserves legacy cuboids, which render all six faces.
    #[serde(default)]
    pub face_visibility: Option<[bool; 6]>,
    /// Per-face clockwise UV rotations in degrees, following Minecraft's
    /// resource-model face convention (the down face rotates counterclockwise).
    #[serde(default)]
    pub face_uv_rotations: Option<[u16; 6]>,
    /// Render as a two-sided textured plane rather than a six-sided cuboid.
    #[serde(default)]
    pub crossed_plane: bool,
    pub mirror: bool,
    pub name: String,
}

/// A named model part. The local translation moves this part's pivot from its
/// parent pivot; cube coordinates remain local to this part's own pivot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewerNode {
    pub name: String,
    pub parent: Option<usize>,
    pub local_translation: [f32; 3],
    /// Blockbench-space Euler angles, in radians.
    pub base_rotation: [f32; 3],
    pub cubes: Vec<ViewerCube>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AnimationChannel {
    Position,
    Rotation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewerKeyframe {
    pub time: f32,
    /// Position deltas are model units; rotation deltas are degrees.
    pub value: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewerTrack {
    pub node: usize,
    pub channel: AnimationChannel,
    pub keyframes: Vec<ViewerKeyframe>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewerAnimation {
    pub name: String,
    pub length: f32,
    pub looping: bool,
    pub tracks: Vec<ViewerTrack>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct NodePose {
    pub translation: [f32; 3],
    /// Blockbench-space Euler angles, in radians.
    pub rotation: [f32; 3],
}

/// CPU-side scene contract shared by the conversion engine and any renderer.
/// It deliberately contains no GPU handles, UI state, window types or parser
/// state. ParsedModel remains the only model/animation parsing input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewerScene {
    pub name: String,
    pub texture_size: [u32; 2],
    pub texture: String,
    pub texture_variants: Vec<String>,
    pub nodes: Vec<ViewerNode>,
    pub animations: Vec<ViewerAnimation>,
    pub warnings: Vec<String>,
}

impl ViewerScene {
    pub fn from_parsed_model(model: &ParsedModel) -> Result<Self> {
        if model.parts.is_empty() {
            bail!("Cannot create a viewer scene: the parsed model has no parts.");
        }

        // This is the same pivot calculation used by the Blockbench exporter.
        // Sharing it prevents the preview and exported model from drifting.
        let origins = crate::blockbench::calculate_origins(&model.parts)?;
        let by_name: HashMap<&str, usize> = model.parts.iter().enumerate()
            .map(|(index, part)| (part.name.as_str(), index))
            .collect();

        let mut nodes = Vec::with_capacity(model.parts.len());
        let mut node_ids = HashMap::<String, usize>::new();
        for (index, part) in model.parts.iter().enumerate() {
            node_ids.insert(
                deterministic_uuid(&model.class_name, "group", index, &part.name),
                index,
            );

            let parent = part.parent.as_deref().and_then(|name| by_name.get(name).copied());
            let local_translation: [f32; 3] = if let Some(parent_index) = parent {
                [
                    (origins[index][0] - origins[parent_index][0]) as f32,
                    (origins[index][1] - origins[parent_index][1]) as f32,
                    (origins[index][2] - origins[parent_index][2]) as f32,
                ]
            } else {
                origins[index].map(|value| value as f32)
            };

            let cubes = part.cubes.iter().map(|cube| {
                ViewerCube {
                    min: [
                        (-cube.x - cube.dx - cube.inflate) as f32,
                        (-cube.y - cube.dy - cube.inflate) as f32,
                        (cube.z - cube.inflate) as f32,
                    ],
                    max: [
                        (-cube.x + cube.inflate) as f32,
                        (-cube.y + cube.inflate) as f32,
                        (cube.z + cube.dz + cube.inflate) as f32,
                    ],
                    uv_origin: [cube.uv_x as f32, cube.uv_y as f32],
                    face_uvs: None,
                    face_texture_indices: None,
                    face_visibility: None,
                    face_uv_rotations: None,
                    crossed_plane: false,
                    mirror: cube.mirror,
                    name: cube.name.clone(),
                }
            }).collect();

            nodes.push(ViewerNode {
                name: part.name.clone(),
                parent,
                local_translation,
                base_rotation: [
                    -part.rotation[0] as f32,
                    -part.rotation[1] as f32,
                    part.rotation[2] as f32,
                ],
                cubes,
            });
        }

        let animations = model.blockbench_animations.iter()
            .filter_map(|animation| parse_animation(animation, &node_ids))
            .collect::<Vec<_>>();

        let mut warnings = model.warnings.clone();
        if model.blockbench_animations.is_empty() && !model.animation_analysis.is_null() {
            warnings.push("Animation analysis exists, but no preview clips have been baked for this model yet.".to_owned());
        }
        if nodes.iter().all(|node| node.cubes.is_empty()) {
            warnings.push("The model hierarchy was recovered, but it contains no visible cubes.".to_owned());
        }

        Ok(Self {
            name: model.class_name.clone(),
            texture_size: [model.texture_width.max(1), model.texture_height.max(1)],
            texture: model.primary_texture.clone(),
            texture_variants: model.texture_variants.clone(),
            nodes,
            animations,
            warnings,
        })
    }

    /// Return a pose for every node at an animation time. Keyframes are
    /// interpolated linearly; unsupported interpolation modes remain usable
    /// with a predictable linear approximation.
    pub fn sample_pose(&self, animation_index: usize, elapsed: f32) -> Vec<NodePose> {
        let mut pose: Vec<NodePose> = self.nodes.iter().map(|node| NodePose {
            translation: node.local_translation,
            rotation: node.base_rotation,
        }).collect();

        let Some(animation) = self.animations.get(animation_index) else {
            return pose;
        };
        let time = if animation.length > 0.0 {
            if animation.looping {
                elapsed.rem_euclid(animation.length)
            } else {
                elapsed.clamp(0.0, animation.length)
            }
        } else {
            0.0
        };

        for track in &animation.tracks {
            let Some(node_pose) = pose.get_mut(track.node) else { continue; };
            let value = sample_track(&track.keyframes, time);
            match track.channel {
                AnimationChannel::Position => {
                    for axis in 0..3 { node_pose.translation[axis] += value[axis]; }
                }
                AnimationChannel::Rotation => {
                    for axis in 0..3 {
                        node_pose.rotation[axis] += value[axis].to_radians();
                    }
                }
            }
        }
        pose
    }
}

fn parse_animation(value: &Value, node_ids: &HashMap<String, usize>) -> Option<ViewerAnimation> {
    let name = value.get("name")?.as_str()?.to_owned();
    let length = value.get("length").and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as f32;
    let looping = value.get("loop").and_then(Value::as_str)
        .map(|mode| mode.eq_ignore_ascii_case("loop") || mode.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let animators = value.get("animators")?.as_object()?;
    let mut tracks = Vec::new();

    for (uuid, animator) in animators {
        let Some(&node) = node_ids.get(uuid) else { continue; };
        let Some(keyframes) = animator.get("keyframes").and_then(Value::as_array) else { continue; };
        for channel in [AnimationChannel::Position, AnimationChannel::Rotation] {
            let expected = match channel {
                AnimationChannel::Position => "position",
                AnimationChannel::Rotation => "rotation",
            };
            let mut values = keyframes.iter().filter_map(|frame| {
                if !frame.get("channel")?.as_str()?.eq_ignore_ascii_case(expected) {
                    return None;
                }
                let time = frame.get("time")?.as_f64()? as f32;
                let point = frame.get("data_points")?.as_array()?.first()?;
                Some(ViewerKeyframe {
                    time,
                    value: [
                        point.get("x")?.as_f64()? as f32,
                        point.get("y")?.as_f64()? as f32,
                        point.get("z")?.as_f64()? as f32,
                    ],
                })
            }).collect::<Vec<_>>();
            values.sort_by(|a, b| a.time.total_cmp(&b.time));
            if !values.is_empty() {
                tracks.push(ViewerTrack { node, channel, keyframes: values });
            }
        }
    }

    Some(ViewerAnimation { name, length, looping, tracks })
}

fn sample_track(keyframes: &[ViewerKeyframe], time: f32) -> [f32; 3] {
    let Some(first) = keyframes.first() else { return [0.0; 3]; };
    if time <= first.time {
        return if first.time <= 0.0 { first.value } else { [0.0; 3] };
    }
    for pair in keyframes.windows(2) {
        let left = &pair[0];
        let right = &pair[1];
        if time <= right.time {
            let span = right.time - left.time;
            if span <= f32::EPSILON { return right.value; }
            let amount = ((time - left.time) / span).clamp(0.0, 1.0);
            return std::array::from_fn(|axis| {
                left.value[axis] + (right.value[axis] - left.value[axis]) * amount
            });
        }
    }
    keyframes.last().map(|last| last.value).unwrap_or([0.0; 3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cube, ParsedModel, Part};
    use serde_json::json;

    fn part(name: &str, parent: Option<&str>, offset: [f64; 3], rotation: [f64; 3]) -> Part {
        Part {
            name: name.to_owned(), variable: name.to_owned(), parent_variable: "root".to_owned(),
            offset, rotation,
            cubes: vec![Cube {
                x: 1.0, y: 2.0, z: 3.0, dx: 4.0, dy: 5.0, dz: 6.0,
                uv_x: 8, uv_y: 4, mirror: true, inflate: 0.25, name: format!("{name}_cube"),
            }],
            parent: parent.map(str::to_owned), children: Vec::new(),
            source_index: 0, source_name: name.to_owned(), render_instance_index: 0,
            render_instance_spec: Value::Null, origin: [0.0, 24.0, 0.0],
            group_uuid: String::new(),
        }
    }

    #[test]
    fn scene_uses_the_exporters_pivot_and_cube_coordinate_conventions() {
        let model = ParsedModel::new(
            "ModelScene".to_owned(), 64, 32,
            vec![
                part("body", None, [2.0, 3.0, 4.0], [0.0, 0.0, 0.0]),
                part("head", Some("body"), [1.0, 5.0, 2.0], [0.1, 0.2, 0.3]),
            ],
            Vec::new(),
        );
        let scene = ViewerScene::from_parsed_model(&model).unwrap();
        assert_eq!(scene.nodes[0].local_translation, [-2.0, 21.0, 4.0]);
        assert_eq!(scene.nodes[1].local_translation, [-1.0, -5.0, 2.0]);
        assert_eq!(scene.nodes[0].cubes[0].min, [-5.25, -7.25, 2.75]);
        assert_eq!(scene.nodes[0].cubes[0].max, [-0.75, -1.75, 9.25]);
        assert!(scene.nodes[0].cubes[0].mirror);
        assert_eq!(scene.nodes[0].cubes[0].uv_origin, [8.0, 4.0]);
    }

    #[test]
    fn samples_position_and_rotation_from_existing_blockbench_clips() {
        let mut model = ParsedModel::new(
            "ModelAnimated".to_owned(), 64, 32,
            vec![part("body", None, [0.0; 3], [0.0; 3])],
            Vec::new(),
        );
        let uuid = deterministic_uuid("ModelAnimated", "group", 0, "body");
        model.blockbench_animations = vec![json!({
            "name": "animation.model.walk",
            "loop": "loop",
            "length": 2,
            "animators": {
                uuid: {"keyframes": [
                    {"channel":"rotation","time":0,"data_points":[{"x":0,"y":0,"z":0}]},
                    {"channel":"rotation","time":2,"data_points":[{"x":90,"y":0,"z":0}]},
                    {"channel":"position","time":0,"data_points":[{"x":0,"y":0,"z":0}]},
                    {"channel":"position","time":2,"data_points":[{"x":0,"y":4,"z":0}]}
                ]}
            }
        })];
        let scene = ViewerScene::from_parsed_model(&model).unwrap();
        assert_eq!(scene.animations.len(), 1);
        let pose = scene.sample_pose(0, 1.0);
        assert!((pose[0].rotation[0] - 45.0f32.to_radians()).abs() < 1e-5);
        assert!((pose[0].translation[1] - 26.0).abs() < 1e-5);
    }
}
