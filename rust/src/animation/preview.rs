use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::blockbench::deterministic_uuid;
use crate::model::Part;
use super::eval::{evaluate, trig_coefficients};

#[derive(Debug, Clone)]
struct Formula {
    bone: String,
    channel: String,
    axis: String,
    expression: String,
    class: String,
}

type AxisKey = (String, String, String);
type Clip = BTreeMap<String, BTreeMap<String, BTreeMap<String, [f64; 3]>>>;

fn formulas_from_analysis(analysis: &Value) -> Vec<Formula> {
    analysis["formulas"].as_array().into_iter().flatten().filter_map(|row| {
        Some(Formula {
            bone: row["bone"].as_str()?.to_owned(),
            channel: row["channel"].as_str()?.to_owned(),
            axis: row["axis"].as_str()?.to_owned(),
            expression: row["expression"].as_str()?.to_owned(),
            class: row["animation_class"].as_str().unwrap_or("static").to_owned(),
        })
    }).collect()
}

fn axis_index(axis: &str) -> Option<usize> {
    match axis { "x" => Some(0), "y" => Some(1), "z" => Some(2), _ => None }
}

fn clean_number(value: f64) -> Value {
    if value.abs() < 1e-10 { json!(0) }
    else if (value - value.round()).abs() < 1e-9 { json!(value.round() as i64) }
    else { json!((value * 100_000_000.0).round() / 100_000_000.0) }
}

fn base_state(parts: &[Part]) -> HashMap<AxisKey, f64> {
    let mut result = HashMap::new();
    for part in parts {
        for (axis, index) in [("x", 0), ("y", 1), ("z", 2)] {
            result.insert((part.name.clone(), "rotation".to_owned(), axis.to_owned()), part.rotation[index]);
            result.insert((part.name.clone(), "position".to_owned(), axis.to_owned()), part.offset[index]);
        }
    }
    result
}

fn period_for(formulas: &[Formula], variable: &str) -> f64 {
    let mut shortest = f64::INFINITY;
    for formula in formulas {
        for coefficient in trig_coefficients(&formula.expression, variable) {
            let period = std::f64::consts::TAU / (coefficient * 20.0);
            if (0.08..=12.0).contains(&period) {
                shortest = shortest.min(period);
            }
        }
    }
    if shortest.is_finite() { shortest } else { 1.0 }
}

fn runtime_samples(kind: &str, length: f64, count: usize) -> Vec<(f64, HashMap<String, f64>)> {
    let mut samples = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let elapsed = length * index as f64 / count as f64;
        let phase = std::f64::consts::TAU * index as f64 / count as f64;
        let mut variables = HashMap::from([
            ("limbSwing".to_owned(), 0.0),
            ("limbSwingAmount".to_owned(), 0.0),
            ("ageInTicks".to_owned(), 20.0 * elapsed),
            ("netHeadYaw".to_owned(), 0.0),
            ("headPitch".to_owned(), 0.0),
            ("scaleFactor".to_owned(), 1.0),
        ]);
        match kind {
            "walk_cycle" => {
                variables.insert("limbSwing".to_owned(), phase);
                variables.insert("limbSwingAmount".to_owned(), 1.0);
            }
            "idle" => {}
            _ => {}
        }
        samples.push((elapsed, variables));
    }
    samples
}

fn formula_applies(formula: &Formula, kind: &str) -> bool {
    formula.class == "static" || formula.class == "combined" || formula.class == kind
}

fn axis_delta(key: &AxisKey, target: f64, bind: f64) -> f64 {
    let difference = target - bind;
    match key.1.as_str() {
        "rotation" => difference.to_degrees() * if matches!(key.2.as_str(), "x" | "y") { -1.0 } else { 1.0 },
        "position" => difference * if matches!(key.2.as_str(), "x" | "y") { -1.0 } else { 1.0 },
        _ => difference,
    }
}

fn sample_clip(parts: &[Part], formulas: &[Formula], kind: &str, length: f64) -> Clip {
    let bind = base_state(parts);
    let animated: HashSet<AxisKey> = formulas.iter()
        .filter(|formula| formula.class == kind || formula.class == "combined")
        .map(|formula| (formula.bone.clone(), formula.channel.clone(), formula.axis.clone()))
        .collect();

    let mut clip = Clip::new();
    for (time, variables) in runtime_samples(kind, length, 64) {
        let mut state = bind.clone();
        // This follows the method's original assignment order. A later formula
        // can read the updated state of an earlier assignment through its
        // recovered symbolic expression.
        for formula in formulas.iter().filter(|formula| formula_applies(formula, kind)) {
            let key = (formula.bone.clone(), formula.channel.clone(), formula.axis.clone());
            if let Ok(value) = evaluate(&formula.expression, &variables) {
                state.insert(key, value);
            }
        }

        for key in &animated {
            let Some(index) = axis_index(&key.2) else { continue; };
            let target = state.get(key).copied().unwrap_or(0.0);
            let neutral = bind.get(key).copied().unwrap_or(0.0);
            let vector = clip.entry(key.0.clone()).or_default()
                .entry(key.1.clone()).or_default()
                .entry(format!("{time:.8}")).or_insert([0.0; 3]);
            vector[index] = axis_delta(key, target, neutral);
        }
    }
    clip
}

fn has_motion(clip: &Clip) -> bool {
    clip.values().any(|channels| channels.values().any(|keys| {
        keys.values().any(|value| value.iter().any(|component| component.abs() > 1e-6))
    }))
}

fn group_uuid(class_name: &str, parts: &[Part], bone: &str) -> Option<String> {
    parts.iter().enumerate().find(|(_, part)| part.name == bone)
        .map(|(index, part)| deterministic_uuid(class_name, "group", index, &part.name))
}

fn blockbench_animation(
    class_name: &str,
    parts: &[Part],
    name: &str,
    length: f64,
    clip: &Clip,
    selected: bool,
) -> Value {
    let mut animators = Map::new();
    for (bone, channels) in clip {
        let Some(part_uuid) = group_uuid(class_name, parts, bone) else { continue; };
        let mut keyframes = Vec::new();
        for channel in ["rotation", "position"] {
            let Some(keys) = channels.get(channel) else { continue; };
            for (time_text, vector) in keys {
                let time = time_text.parse::<f64>().unwrap_or(0.0);
                let uuid = deterministic_uuid(
                    class_name,
                    &format!("keyframe:{name}:{bone}:{channel}:{part_uuid}"),
                    (time * 1_000_000.0).round() as usize,
                    &format!("{bone}:{channel}:{time:.8}:{part_uuid}"),
                );
                keyframes.push(json!({
                    "uuid": uuid,
                    "channel": channel,
                    "data_points": [{
                        "x": clean_number(vector[0]),
                        "y": clean_number(vector[1]),
                        "z": clean_number(vector[2])
                    }],
                    "time": clean_number(time),
                    "color": -1,
                    "interpolation": "linear",
                    "bezier_linked": true,
                    "bezier_left_time": [-0.1, -0.1, -0.1],
                    "bezier_left_value": [0, 0, 0],
                    "bezier_right_time": [0.1, 0.1, 0.1],
                    "bezier_right_value": [0, 0, 0]
                }));
            }
        }
        if !keyframes.is_empty() {
            animators.insert(part_uuid, json!({
                "name": bone,
                "type": "bone",
                "keyframes": keyframes
            }));
        }
    }
    json!({
        "name": name,
        "loop": "loop",
        "override": false,
        "length": clean_number(length),
        "snapping": 24,
        "selected": selected,
        "anim_time_update": "",
        "blend_weight": "",
        "start_delay": 0,
        "loop_delay": 0,
        "animators": animators,
        "moj_model_bridge": {
            "source_class": "legacy_procedural",
            "source_animated_axes_are_absolute": true,
            "blockbench_keyframes_are_additive_deltas": true,
            "animation_is_rebased_to_static_bind_pose": true
        }
    })
}

/// Sample recovered JVM animation expressions and serialize native Blockbench
/// walk/idle clips. Unsupported expressions are omitted from affected axes.
pub fn bake_blockbench_animations(class_name: &str, parts: &[Part], analysis: &Value) -> Vec<Value> {
    let formulas = formulas_from_analysis(analysis);
    if formulas.is_empty() { return Vec::new(); }

    let mut result = Vec::new();
    for (kind, name, variable) in [
        ("walk_cycle", "animation.model.walk", "limbSwing"),
        ("idle", "animation.model.idle", "ageInTicks"),
    ] {
        let relevant: Vec<Formula> = formulas.iter()
            .filter(|formula| formula.class == kind || formula.class == "combined")
            .cloned().collect();
        if relevant.is_empty() { continue; }
        let length = period_for(&relevant, variable);
        let clip = sample_clip(parts, &formulas, kind, length);
        if has_motion(&clip) {
            let selected = result.is_empty();
            result.push(blockbench_animation(class_name, parts, name, length, &clip, selected));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ParsedModel, Part};
    use serde_json::Value;

    #[test]
    fn bakes_a_looping_walk_clip_as_bind_pose_deltas() {
        let model = ParsedModel::new("ModelPreview".into(), 64, 32, vec![Part {
            name: "body".into(), variable: "body".into(), parent_variable: "root".into(),
            offset: [0.0; 3], rotation: [0.0; 3], cubes: Vec::new(), parent: None,
            children: Vec::new(), source_index: 0, source_name: "body".into(),
            render_instance_index: 0, render_instance_spec: Value::Null,
            origin: [0.0, 24.0, 0.0], group_uuid: String::new()
        }], Vec::new());
        let analysis = json!({"formulas": [{
            "bone": "body", "channel": "rotation", "axis": "x",
            "expression": "sin(limbSwing) * limbSwingAmount",
            "animation_class": "walk_cycle"
        }]});
        let clips = bake_blockbench_animations(&model.class_name, &model.parts, &analysis);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0]["loop"], "loop");
        assert_eq!(clips[0]["name"], "animation.model.walk");
        let animator = clips[0]["animators"].as_object().unwrap().values().next().unwrap();
        assert!(animator["keyframes"].as_array().unwrap().len() >= 32);
        assert_eq!(animator["keyframes"][0]["data_points"][0]["x"], 0);
    }
}
