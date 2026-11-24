use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use crate::model::{ParsedModel, Part, Cube};
use super::java::{eval_int, eval_number, extract_balanced, find_numeric_constants, split_top_level, strip_java_comments, unquote_java_string};
use super::legacy_model::parse_legacy_java_model;

fn method_calls(chain: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut position = 0usize;
    while position < chain.len() {
        let Some(relative) = chain[position..].find('.') else { break; };
        let dot = position + relative;
        let mut cursor = dot + 1;
        while chain.as_bytes().get(cursor).map(|v| v.is_ascii_whitespace()).unwrap_or(false) { cursor += 1; }
        let start = cursor;
        while chain.as_bytes().get(cursor).map(|v| v.is_ascii_alphanumeric() || *v == b'_' || *v == b'$').unwrap_or(false) { cursor += 1; }
        if cursor == start { position = dot + 1; continue; }
        let name = chain[start..cursor].to_owned();
        while chain.as_bytes().get(cursor).map(|v| v.is_ascii_whitespace()).unwrap_or(false) { cursor += 1; }
        if chain.as_bytes().get(cursor).copied() != Some(b'(') { position = cursor; continue; }
        match extract_balanced(chain, cursor, '(', ')') {
            Ok((args, end)) => { result.push((name, args)); position = end; }
            Err(_) => break,
        }
    }
    result
}

fn parse_inflate(expression: &str, constants: &BTreeMap<String, f64>, warnings: &mut Vec<String>) -> Result<f64> {
    let value = expression.trim();
    if value.is_empty() || value.contains("CubeDeformation.NONE") { return Ok(0.0); }
    let re = Regex::new(r"(?:new\s+)?CubeDeformation\s*\(").unwrap();
    if let Some(found) = re.find(value) {
        let open = value[found.start()..found.end()].rfind('(').unwrap() + found.start();
        let (inner, _) = extract_balanced(value, open, '(', ')')?;
        let args = split_top_level(&inner, ',');
        let values: Vec<f64> = args.iter().map(|arg| eval_number(arg, constants)).collect::<Result<_>>()?;
        if values.len() == 1 { return Ok(values[0]); }
        if values.len() >= 3 {
            let min = values[..3].iter().fold(f64::MAX, |a, b| a.min(*b));
            let max = values[..3].iter().fold(f64::MIN, |a, b| a.max(*b));
            if max - min > 1e-7 {
                warnings.push(format!("A per-axis CubeDeformation was reduced to one Blockbench inflate value ({}, {}, {} -> average).", values[0], values[1], values[2]));
            }
            return Ok(values[..3].iter().sum::<f64>() / 3.0);
        }
    }
    eval_number(value, constants)
}

fn parse_builder(builder: &str, constants: &BTreeMap<String, f64>, warnings: &mut Vec<String>, part_name: &str) -> Vec<Cube> {
    let mut cubes = Vec::new();
    let (mut uv_x, mut uv_y, mut mirror, mut cube_number) = (0, 0, false, 0usize);
    for (method, raw_args) in method_calls(builder) {
        let mut args = split_top_level(&raw_args, ',');
        match method.as_str() {
            "texOffs" | "uv" if args.len() >= 2 => {
                if let Ok(value) = eval_int(&args[0], constants) { uv_x = value; }
                if let Ok(value) = eval_int(&args[1], constants) { uv_y = value; }
            }
            "mirror" | "mirrored" => mirror = args.first().map(|v| v.trim() != "false").unwrap_or(true),
            "addBox" | "cuboid" => {
                cube_number += 1;
                let mut name = if cube_number == 1 { format!("{part_name}_cube") } else { format!("{part_name}_cube_{cube_number}") };
                if args.first().map(|v| v.trim().starts_with('"')).unwrap_or(false) {
                    name = unquote_java_string(&args.remove(0));
                    if name.is_empty() { name = format!("{part_name}_cube"); }
                }
                if args.len() < 6 {
                    warnings.push(format!("Skipped an addBox in {part_name}: expected at least 6 dimensions."));
                    continue;
                }
                let dimensions: Vec<f64> = match args[..6].iter().map(|arg| eval_number(arg, constants)).collect() {
                    Ok(values) => values,
                    Err(error) => { warnings.push(format!("Skipped an addBox in {part_name}: {error}")); continue; }
                };
                let (mut inflate, mut cube_mirror) = (0.0, mirror);
                for token in args.iter().skip(6) {
                    let extra = token.trim();
                    if extra == "true" || extra == "false" { cube_mirror = extra == "true"; }
                    else if extra.contains("CubeDeformation") || extra.contains("Dilation") {
                        match parse_inflate(&extra.replace("Dilation", "CubeDeformation"), constants, warnings) {
                            Ok(value) => inflate = value,
                            Err(error) => warnings.push(format!("Could not parse inflation for {part_name}: {error}")),
                        }
                    } else if let Ok(value) = eval_number(extra, constants) { inflate = value; }
                }
                cubes.push(Cube {
                    x: dimensions[0], y: dimensions[1], z: dimensions[2],
                    dx: dimensions[3], dy: dimensions[4], dz: dimensions[5],
                    uv_x, uv_y, mirror: cube_mirror, inflate, name,
                });
            }
            _ => {}
        }
    }
    cubes
}

fn parse_pose(expression: &str, constants: &BTreeMap<String, f64>, warnings: &mut Vec<String>, part_name: &str) -> ([f64; 3], [f64; 3]) {
    if expression.contains("PartPose.ZERO") || expression.contains("ModelTransform.NONE") { return ([0.0; 3], [0.0; 3]); }
    let re = Regex::new(r"(?:PartPose|ModelTransform)\s*\.\s*(offsetAndRotation|offset|rotation|of|pivot)\s*\(").unwrap();
    let Some(found) = re.find(expression) else {
        warnings.push(format!("Unknown PartPose for {part_name}; used zero transform: {}", expression.chars().take(100).collect::<String>()));
        return ([0.0; 3], [0.0; 3]);
    };
    let open = expression[found.start()..found.end()].rfind('(').unwrap() + found.start();
    let Ok((inner, _)) = extract_balanced(expression, open, '(', ')') else {
        warnings.push(format!("Could not read PartPose for {part_name}; used zero transform."));
        return ([0.0; 3], [0.0; 3]);
    };
    let values: Result<Vec<f64>> = split_top_level(&inner, ',').iter().map(|v| eval_number(v, constants)).collect();
    let Ok(values) = values else {
        warnings.push(format!("Could not parse PartPose for {part_name}; used zero transform."));
        return ([0.0; 3], [0.0; 3]);
    };
    let method = found.as_str();
    if method.contains("offsetAndRotation") || method.contains(".of") {
        if values.len() >= 6 { return ([values[0], values[1], values[2]], [values[3], values[4], values[5]]); }
    } else if method.contains(".offset") || method.contains(".pivot") {
        if values.len() >= 3 { return ([values[0], values[1], values[2]], [0.0; 3]); }
    } else if method.contains(".rotation") && values.len() >= 3 {
        return ([0.0; 3], [values[0], values[1], values[2]]);
    }
    warnings.push(format!("Incomplete PartPose for {part_name}; used zero transform."));
    ([0.0; 3], [0.0; 3])
}

fn preserve_animation_methods(source: &str, class_name: &str) -> (Value, String, String, Vec<String>) {
    let mut methods = Vec::<(String, String)>::new();
    let mut warnings = Vec::new();
    for name in ["setupAnim", "setAngles", "renderToBuffer", "render"] {
        let prefix = r"(?m)^[ \t]*(?:(?:public|protected|private|static|final|synchronized)\s+)*[\w<>\[\],.? ]+\b";
        let pattern = Regex::new(&format!(r"{prefix}{name}\s*\([^;{{]*\)\s*(?:throws\s+[^\{{]+)?\{{")).unwrap();
        if let Some(found) = pattern.find(source) {
            let open = source[found.start()..found.end()].rfind('{').unwrap() + found.start();
            match extract_balanced(source, open, '{', '}') {
                Ok((_body, end)) => methods.push((name.to_owned(), source[found.start()..end].to_owned())),
                Err(error) => warnings.push(format!("Could not preserve {name}: {error}")),
            }
        }
    }
    if methods.is_empty() { return (json!({}), String::new(), String::new(), warnings); }
    let names: Vec<String> = methods.iter().map(|(name, _)| name.clone()).collect();
    let mut logic = format!("// Extracted by MojModelBridge native Rust from {class_name}.\n// Procedural methods are preserved verbatim for target-side adaptation.\n\n");
    for (name, body) in &methods { logic.push_str(&format!("// --- {name} ---\n{body}\n\n")); }
    let analysis = json!({
        "source_format":"modern_mojmap_java",
        "method":names.first(),
        "methods":names,
        "mode":"preserve_java",
        "note":"Procedural animation/render methods are preserved as Java. Native formula sampling and Blockbench keyframe baking are pending."
    });
    (analysis, logic, "setupAnim/renderToBuffer".to_owned(), warnings)
}


fn extract_method_body(source: &str, method_names: &[&str]) -> Option<(String, String)> {
    for name in method_names {
        let escaped = regex::escape(name);
        let pattern = Regex::new(&format!(
            r"(?m)^[ \t]*(?:(?:public|protected|private|static|final|synchronized)\s+)*[\w<>\[\],.? ]+\b{escaped}\s*\([^;{{}}]*\)\s*(?:throws\s+[^{{}}]+)?\{{"
        )).ok()?;
        let Some(found) = pattern.find(source) else { continue; };
        let open = source[found.start()..found.end()].rfind('{')? + found.start();
        let Ok((body, _)) = extract_balanced(source, open, '{', '}') else { continue; };
        return Some(((*name).to_owned(), body));
    }
    None
}

fn canonical_part_name(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase).collect()
}

fn resolve_rendered_parts(
    body: &str,
    aliases: &HashMap<String, String>,
) -> BTreeSet<String> {
    let call_pattern = Regex::new(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*render\s*\(").unwrap();
    call_pattern.captures_iter(body).filter_map(|capture| {
        let raw = capture.get(1)?.as_str();
        aliases.get(&canonical_part_name(raw)).cloned()
    }).collect()
}

/// Port of Beta 12's conservative explicit render-loop detector. A loop is
/// expanded only when it renders a known model part and changes the angle used
/// by the render transform. Unrelated animation loops are left untouched.
fn extract_modern_render_instances(
    source: &str,
    class_name: &str,
    parts: &[Part],
    constants: &BTreeMap<String, f64>,
) -> (BTreeMap<String, Vec<Value>>, Vec<Value>, Vec<String>) {
    let Some((method_name, body)) = extract_method_body(source, &["renderToBuffer", "render"]) else {
        return (BTreeMap::new(), Vec::new(), Vec::new());
    };

    let mut aliases = HashMap::<String, String>::new();
    for part in parts {
        for alias in [&part.name, &part.variable] {
            aliases.entry(canonical_part_name(alias)).or_insert_with(|| part.name.clone());
        }
    }

    let loop_pattern = Regex::new(r"for\s*\(([^{}]*)\)\s*\{").unwrap();
    let increment_pattern = Regex::new(r"\b([A-Za-z_$][\w$]*)\s*\+=\s*([^;]+);").unwrap();
    let axis_pattern = Regex::new(r"Axis\.([XYZ])P\.rotation(Degrees)?\(\s*([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*)\s*\)").unwrap();
    let direct_rotation_pattern = Regex::new(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.(xRot|yRot|zRot)\s*=\s*([^;]+);").unwrap();
    let position_pattern = Regex::new(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.(x|y|z)\s*=\s*([^;]+);").unwrap();

    let mut render_instances = BTreeMap::<String, Vec<Value>>::new();
    let mut warnings = Vec::<String>::new();
    let mut cursor = 0usize;

    while cursor < body.len() {
        let Some(found) = loop_pattern.find(&body[cursor..]) else { break; };
        let start = cursor + found.start();
        let end_header = cursor + found.end();
        let header = loop_pattern.captures(&body[start..end_header])
            .and_then(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
            .unwrap_or_default();
        let open = body[start..end_header].rfind('{').map(|value| start + value);
        let Some(open) = open else { cursor = end_header; continue; };
        let Ok((loop_body, loop_end)) = extract_balanced(&body, open, '{', '}') else {
            cursor = end_header;
            continue;
        };
        cursor = loop_end;

        let header_parts = split_top_level(&header, ';');
        if header_parts.len() < 2 { continue; }
        let condition = header_parts[1].trim();
        let inclusive = Regex::new(r"<=\s*([^;]+)$").unwrap();
        let exclusive = Regex::new(r"<\s*([^;]+)$").unwrap();
        let count = if let Some(capture) = inclusive.captures(condition) {
            eval_int(&capture[1], constants).ok().map(|value| value + 1)
        } else if let Some(capture) = exclusive.captures(condition) {
            eval_int(&capture[1], constants).ok()
        } else {
            None
        };
        let Some(count) = count.filter(|value| *value >= 2 && *value <= 512) else { continue; };

        let rendered = resolve_rendered_parts(&loop_body, &aliases);
        if rendered.is_empty() { continue; }

        let increments: BTreeMap<String, String> = increment_pattern.captures_iter(&loop_body)
            .map(|capture| (capture[1].to_owned(), capture[2].trim().to_owned()))
            .collect();

        let mut chosen_axis: Option<char> = None;
        let mut chosen_variable = String::new();
        let mut chosen_step = None::<f64>;

        for capture in axis_pattern.captures_iter(&loop_body) {
            let axis = capture[1].chars().next().unwrap_or('Z');
            let degrees = capture.get(2).is_some();
            let variable = capture[3].rsplit('.').next().unwrap_or(&capture[3]);
            let Some(increment) = increments.get(variable) else { continue; };
            let Ok(mut step) = eval_number(increment, constants) else { continue; };
            if degrees { step = step.to_radians(); }
            chosen_axis = Some(axis);
            chosen_variable = variable.to_owned();
            chosen_step = Some(step);
            break;
        }

        if chosen_axis.is_none() {
            for capture in direct_rotation_pattern.captures_iter(&loop_body) {
                let raw_part = &capture[1];
                let field = &capture[2];
                let expression = capture[3].trim();
                let resolved = aliases.get(&canonical_part_name(raw_part));
                if resolved.is_none() { continue; }
                for (variable, increment) in &increments {
                    if expression.trim_start_matches("this.").trim() != variable { continue; }
                    let Ok(step) = eval_number(increment, constants) else { continue; };
                    chosen_axis = Some(match field { "xRot" => 'X', "yRot" => 'Y', _ => 'Z' });
                    chosen_variable = variable.clone();
                    chosen_step = Some(step);
                    break;
                }
                if chosen_axis.is_some() { break; }
            }
        }

        let axis_index = match chosen_axis.unwrap_or('Z') { 'X' => 0, 'Y' => 1, _ => 2 };
        let step = chosen_step.unwrap_or(0.0);

        let runtime_transforms: Vec<Value> = direct_rotation_pattern.captures_iter(&loop_body)
            .filter_map(|capture| {
                let part = aliases.get(&canonical_part_name(&capture[1]))?;
                Some(json!({
                    "part": part,
                    "axis": capture[2],
                    "expression": capture[3].trim()
                }))
            }).collect();
        let position_assignments: Vec<Value> = position_pattern.captures_iter(&loop_body)
            .filter_map(|capture| {
                let part = aliases.get(&canonical_part_name(&capture[1]))?;
                Some(json!({
                    "part": part,
                    "axis": capture[2],
                    "expression": capture[3].trim()
                }))
            }).collect();

        for part_name in rendered {
            let instances = render_instances.entry(part_name.clone()).or_default();
            if !instances.is_empty() && instances.len() != count as usize {
                warnings.push(format!(
                    "Render multiplicity for '{part_name}' in {class_name} was encountered more than once; instance copies were appended in source order."
                ));
            }
            for instance_index in 0..count as usize {
                let mut rotation = [0.0f64; 3];
                if chosen_axis.is_some() {
                    rotation[axis_index] = instance_index as f64 * step;
                }
                instances.push(json!({
                    "rotation": rotation,
                    "source_method": method_name,
                    "instance_index": instance_index,
                    "instance_count": count,
                    "runtime_transforms": runtime_transforms,
                    "part_assignments": runtime_transforms,
                    "position_assignments": position_assignments
                }));
            }
        }

        let _ = chosen_variable;
    }

    // Persistent PoseStack counters (e.g. Rotator.rf1 += 2 after its render
    // loop) remain a separate runtime transform, not baked into static copies.
    let mut runtime_rotations = Vec::<Value>::new();
    let global_increments: BTreeMap<String, String> = increment_pattern.captures_iter(&body)
        .map(|capture| (capture[1].to_owned(), capture[2].trim().to_owned()))
        .collect();
    for capture in axis_pattern.captures_iter(&body) {
        let axis = capture[1].to_owned();
        let degrees = capture.get(2).is_some();
        let variable = capture[3].rsplit('.').next().unwrap_or(&capture[3]).to_owned();
        let Some(increment) = global_increments.get(&variable) else { continue; };
        if increment_pattern.find_iter(&body).filter(|found| {
            increment_pattern.captures(found.as_str()).map(|entry| entry[1] == variable).unwrap_or(false)
        }).count() != 1 { continue; }
        let after = &body[capture.get(0).unwrap().end()..];
        let Some(loop_match) = loop_pattern.find(after) else { continue; };
        let loop_open = after[loop_match.start()..loop_match.end()].rfind('{').map(|value| loop_match.start() + value);
        let Some(loop_open) = loop_open else { continue; };
        let Ok((loop_body, loop_end)) = extract_balanced(after, loop_open, '{', '}') else { continue; };
        let post_loop = &after[loop_end..];
        if !increment_pattern.captures_iter(post_loop).any(|entry| entry[1] == variable) { continue; }
        let rendered = resolve_rendered_parts(&loop_body, &aliases);
        let selected: Vec<String> = rendered.into_iter().filter(|name| render_instances.contains_key(name)).collect();
        if selected.is_empty() { continue; }
        if let Ok(speed) = eval_number(increment, constants) {
            runtime_rotations.push(json!({
                "axis": axis,
                "degrees": degrees,
                "speed": speed,
                "parts": selected,
                "variable": variable
            }));
        }
    }

    (render_instances, runtime_rotations, warnings)
}

/// Port of Beta 12's repeated helper-call detector for groups such as eight
/// crab legs, where a helper overwrites ModelPart.x/y/z before rendering it.
fn extract_modern_helper_render_instances(
    source: &str,
    parts: &[Part],
    constants: &BTreeMap<String, f64>,
) -> BTreeMap<String, Vec<Value>> {
    let method_pattern = Regex::new(
        r"(?m)(?:public|protected|private)\s+(?:static\s+)?(?:final\s+)?[A-Za-z_$][\w.$<>,?\[\] ]*\s+([A-Za-z_$][\w$]*)\s*\(([^)]*)\)\s*\{"
    ).unwrap();
    let call_pattern = Regex::new(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\(([^;{}]*)\)\s*;").unwrap();
    let render_pattern = Regex::new(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*render\s*\(").unwrap();
    let position_pattern = Regex::new(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.(x|y|z)\s*=\s*([^;]+);").unwrap();
    let parameter_pattern = Regex::new(r"([A-Za-z_$][\w$]*)\s*$").unwrap();

    let mut definitions = HashMap::<String, (Vec<String>, String)>::new();
    let mut cursor = 0usize;
    while cursor < source.len() {
        let Some(found) = method_pattern.find(&source[cursor..]) else { break; };
        let start = cursor + found.start();
        let end_header = cursor + found.end();
        let capture = method_pattern.captures(&source[start..end_header]).unwrap();
        let name = capture[1].to_owned();
        let raw_parameters = capture[2].to_owned();
        let open = source[start..end_header].rfind('{').map(|value| start + value);
        let Some(open) = open else { cursor = end_header; continue; };
        let Ok((body, end)) = extract_balanced(source, open, '{', '}') else {
            cursor = end_header;
            continue;
        };
        let parameters = if raw_parameters.trim().is_empty() {
            Vec::new()
        } else {
            split_top_level(&raw_parameters, ',').iter().map(|raw| {
                parameter_pattern.captures(raw.trim()).map(|capture| capture[1].to_owned()).unwrap_or_default()
            }).collect()
        };
        definitions.insert(name, (parameters, body));
        cursor = end;
    }
    if definitions.is_empty() { return BTreeMap::new(); }

    let known: HashMap<String, String> = parts.iter().flat_map(|part| {
        [
            (canonical_part_name(&part.name), part.name.clone()),
            (canonical_part_name(&part.variable), part.name.clone())
        ]
    }).collect();
    let mut output = BTreeMap::<String, Vec<Value>>::new();

    for (outer_name, (_outer_parameters, outer_body)) in &definitions {
        let calls: Vec<(String, String)> = call_pattern.captures_iter(outer_body)
            .map(|capture| (capture[1].to_owned(), capture[2].to_owned()))
            .collect();
        let mut groups = BTreeMap::<String, Vec<String>>::new();
        for (callee, arguments) in calls {
            if callee == *outer_name { continue; }
            if let Some((_parameters, nested_body)) = definitions.get(&callee) {
                let has_known_render = render_pattern.captures_iter(nested_body).any(|capture| {
                    known.contains_key(&canonical_part_name(&capture[1]))
                });
                if has_known_render {
                    groups.entry(callee).or_default().push(arguments);
                }
            }
        }

        for (callee, group) in groups {
            if group.len() < 2 { continue; }
            let Some((parameters, nested_body)) = definitions.get(&callee) else { continue; };
            let rendered: BTreeSet<String> = render_pattern.captures_iter(nested_body)
                .filter_map(|capture| known.get(&canonical_part_name(&capture[1])).cloned())
                .collect();
            if rendered.is_empty() { continue; }

            for (instance_index, raw_arguments) in group.iter().enumerate() {
                let arguments = if raw_arguments.trim().is_empty() { Vec::new() } else { split_top_level(raw_arguments, ',') };
                let mapping: HashMap<String, String> = parameters.iter().zip(arguments.iter())
                    .map(|(parameter, argument)| (parameter.clone(), argument.trim().to_owned()))
                    .collect();
                let mut assignments = HashMap::<String, HashMap<String, f64>>::new();
                for capture in position_pattern.captures_iter(nested_body) {
                    let Some(part) = known.get(&canonical_part_name(&capture[1])).cloned() else { continue; };
                    let mut expression = capture[3].trim().to_owned();
                    for (parameter, argument) in &mapping {
                        let token = Regex::new(&format!(r"\b{}\b", regex::escape(parameter))).unwrap();
                        expression = token.replace_all(&expression, argument.as_str()).to_string();
                    }
                    let Ok(value) = eval_number(&expression, constants) else { continue; };
                    assignments.entry(part).or_default().insert(capture[2].to_owned(), value);
                }
                if assignments.is_empty() { continue; }

                for part in &rendered {
                    let mut spec = json!({
                        "instance_index": instance_index,
                        "instance_count": group.len(),
                        "source_method": outer_name,
                        "helper_method": callee
                    });
                    if let Some(position) = assignments.get(part) {
                        if let (Some(x), Some(y), Some(z)) = (position.get("x"), position.get("y"), position.get("z")) {
                            spec["position"] = json!([x, y, z]);
                        }
                    }
                    output.entry(part.clone()).or_default().push(spec);
                }
            }
        }
    }

    output
}

/// Reconstruct modern Mojang-mapped PartDefinition / ModelPartData source.
pub fn parse_modern_java_model(text: &str, source_name: &str) -> Result<ParsedModel> {
    let clean = strip_java_comments(text);
    let class_re = Regex::new(r"\b(?:public\s+)?(?:final\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap();
    let class_name = class_re.captures(&clean).and_then(|c| c.get(1).map(|m| m.as_str().to_owned()))
        .unwrap_or_else(|| Path::new(source_name).file_stem().and_then(|v| v.to_str()).unwrap_or("Model").to_owned());
    if !clean.contains("addOrReplaceChild") && !clean.contains("addChild") {
        bail!("No PartDefinition.addOrReplaceChild(...) or ModelPartData.addChild(...) declarations found in {source_name}.");
    }

    let constants = find_numeric_constants(&clean);
    let (mut texture_width, mut texture_height) = (64i32, 32i32);
    let layer_re = Regex::new(r"LayerDefinition\s*\.\s*create\s*\(").unwrap();
    for found in layer_re.find_iter(&clean) {
        let open = clean[found.start()..found.end()].rfind('(').unwrap() + found.start();
        if let Ok((inner, _)) = extract_balanced(&clean, open, '(', ')') {
            let args = split_top_level(&inner, ',');
            if args.len() >= 3 {
                if let Ok(value) = eval_int(&args[args.len() - 2], &constants) { texture_width = value; }
                if let Ok(value) = eval_int(&args[args.len() - 1], &constants) { texture_height = value; }
                break;
            }
        }
    }
    if texture_width == 64 && texture_height == 32 && !clean.contains("LayerDefinition") {
        let width_re = Regex::new(r"\btexWidth\s*=\s*([^;]+);").unwrap();
        let height_re = Regex::new(r"\btexHeight\s*=\s*([^;]+);").unwrap();
        if let (Some(w), Some(h)) = (width_re.captures(&clean), height_re.captures(&clean)) {
            if let (Ok(w), Ok(h)) = (eval_int(&w[1], &constants), eval_int(&h[1], &constants)) { texture_width = w; texture_height = h; }
        }
    }

    let mut root_variables: HashSet<String> = HashSet::from(["root".to_owned(), "partdefinition".to_owned(), "modelPartData".to_owned()]);
    let root_re = Regex::new(r"(?:PartDefinition|ModelPartData)\s+([A-Za-z_$][\w$]*)\s*=\s*[A-Za-z_$][\w$]*\s*\.\s*getRoot\s*\(\s*\)").unwrap();
    for capture in root_re.captures_iter(&clean) { root_variables.insert(capture[1].to_owned()); }

    let mut builders = HashMap::<String, String>::new();
    let builder_re = Regex::new(r"CubeListBuilder\s+([A-Za-z_$][\w$]*)\s*=").unwrap();
    for capture in builder_re.captures_iter(&clean) {
        let whole = capture.get(0).unwrap();
        if let Some(end) = clean[whole.end()..].find(';') {
            builders.insert(capture[1].to_owned(), clean[whole.end()..whole.end() + end].trim().to_owned());
        }
    }

    let call_re = Regex::new(r"([A-Za-z_$][A-Za-z0-9_$]*)\s*\.\s*(?:addOrReplaceChild|addChild)\s*\(").unwrap();
    let assign_re = Regex::new(r"(?:PartDefinition|ModelPartData)?\s*([A-Za-z_$][\w$]*)\s*=\s*$").unwrap();
    let mut parts = Vec::<Part>::new();
    let mut parent_variables = Vec::<String>::new();
    let mut warnings = Vec::new();
    for found in call_re.find_iter(&clean) {
        let capture = call_re.captures(&clean[found.start()..found.end()]).unwrap();
        let parent_variable = capture[1].to_owned();
        let open = clean[found.start()..found.end()].rfind('(').unwrap() + found.start();
        let (raw_args, _) = extract_balanced(&clean, open, '(', ')').with_context(|| format!("Reading child declaration in {class_name}"))?;
        let args = split_top_level(&raw_args, ',');
        if args.len() < 3 {
            warnings.push(format!("Skipped child call near byte {}: expected three arguments.", found.start()));
            continue;
        }
        let statement_start = clean[..found.start()].rfind(|c| c == ';' || c == '{' || c == '}').map(|v| v + 1).unwrap_or(0);
        let variable = assign_re.captures(&clean[statement_start..found.start()]).map(|v| v[1].to_owned())
            .unwrap_or_else(|| format!("__part_{}", parts.len()));
        let name = unquote_java_string(&args[0]);
        let builder = builders.get(args[1].trim()).cloned().unwrap_or_else(|| args[1].trim().to_owned());
        let mut local_warnings = Vec::new();
        let (offset, rotation) = parse_pose(&args[2], &constants, &mut local_warnings, &name);
        let cubes = parse_builder(&builder, &constants, &mut local_warnings, &name);
        warnings.extend(local_warnings);
        parts.push(Part {
            name: name.clone(), variable, parent_variable: parent_variable.clone(),
            offset, rotation, cubes, parent: None, children: Vec::new(),
            source_index: parts.len(), source_name: name, render_instance_index: 0,
            render_instance_spec: Value::Null, origin: [0.0, 24.0, 0.0], group_uuid: String::new(),
        });
        parent_variables.push(parent_variable);
    }
    if parts.is_empty() { bail!("No modern model parts were recovered from {source_name}."); }

    let variable_indices: HashMap<String, usize> = parts.iter().enumerate().map(|(i, p)| (p.variable.clone(), i)).collect();
    let names: Vec<String> = parts.iter().map(|p| p.name.clone()).collect();
    for i in 0..parts.len() {
        if let Some(parent) = variable_indices.get(&parent_variables[i]).copied() {
            if parent != i {
                parts[i].parent = Some(names[parent].clone());
                let child_name = parts[i].name.clone();
                parts[parent].children.push(child_name);
            }
        } else if !root_variables.contains(&parent_variables[i]) && !parent_variables[i].to_ascii_lowercase().ends_with("root") {
            warnings.push(format!("Parent variable '{}' for '{}' was not resolved; treated as a root part.", parent_variables[i], parts[i].name));
        }
    }

    let (mut render_instances, render_runtime_rotations, render_warnings) =
        extract_modern_render_instances(&clean, &class_name, &parts, &constants);
    let helper_instances = extract_modern_helper_render_instances(&clean, &parts, &constants);
    for (name, specs) in helper_instances {
        render_instances.insert(name, specs);
    }
    warnings.extend(render_warnings);

    let (animation_analysis, animation_logic, animation_source, animation_warnings) = preserve_animation_methods(&clean, &class_name);
    warnings.extend(animation_warnings);
    if parts.iter().map(|part| part.cubes.len()).sum::<usize>() == 0 {
        warnings.push("Parts were found, but no CubeListBuilder.addBox calls were converted.".to_owned());
    }
    if texture_width <= 0 || texture_height <= 0 {
        warnings.push(format!("Invalid texture resolution {texture_width}x{texture_height}; falling back to 64 x 32."));
        texture_width = 64;
        texture_height = 32;
    }
    let mut model = ParsedModel::new(class_name, texture_width as u32, texture_height as u32, parts, warnings);
    model.source_format = "modern_mojmap_java".to_owned();
    model.animation_analysis = animation_analysis;
    model.animation_logic = animation_logic;
    model.animation_source = animation_source;
    model.render_instances = render_instances;
    model.render_runtime_rotations = render_runtime_rotations;
    Ok(model)
}

/// Detect and parse either modern Mojmap or legacy ModelRenderer Java source.
pub fn parse_java_model(text: &str, source_name: &str, constructor_args: &[Value]) -> Result<ParsedModel> {
    let clean = strip_java_comments(text);
    let modern = ["PartDefinition", "ModelPartData", "CubeListBuilder", "LayerDefinition"].iter().any(|marker| clean.contains(marker));
    if modern {
        parse_modern_java_model(text, source_name)
    } else {
        parse_legacy_java_model(text, source_name, constructor_args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_repeated_render_loop_as_per_instance_rotations() {
        let source = r#"
        public class ModelRepeatedRender {
            public static LayerDefinition createBodyLayer() {
                MeshDefinition mesh = new MeshDefinition();
                PartDefinition root = mesh.getRoot();
                PartDefinition wing = root.addOrReplaceChild("wing",
                    CubeListBuilder.create().addBox(-1, -1, -1, 2, 2, 2),
                    PartPose.ZERO);
                return LayerDefinition.create(mesh, 64, 32);
            }
            public void renderToBuffer(PoseStack poseStack, VertexConsumer vertex,
                                       int light, int overlay, float red, float green,
                                       float blue, float alpha) {
                float angle = 0.0F;
                for (int i = 0; i < 4; i++) {
                    poseStack.mulPose(Axis.ZP.rotationDegrees(angle));
                    this.wing.render(poseStack, vertex, light, overlay);
                    angle += 90.0F;
                }
            }
        }
        "#;
        let model = parse_java_model(source, "ModelRepeatedRender.java", &[]).unwrap();
        let instances = model.render_instances.get("wing").expect("render loop should be detected");
        assert_eq!(instances.len(), 4);
        let rotations: Vec<f64> = instances.iter()
            .map(|spec| spec["rotation"][2].as_f64().unwrap())
            .collect();
        assert!((rotations[0] - 0.0).abs() < 1e-8);
        assert!((rotations[1] - std::f64::consts::FRAC_PI_2).abs() < 1e-8);
        assert!((rotations[2] - std::f64::consts::PI).abs() < 1e-8);
        assert!((rotations[3] - 3.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-8);
    }

    #[test]
    fn parses_modern_geometry_hierarchy_pose_and_uv() {
        let source = r#"
        public class ModelModern {
            private static final float SCALE = 0.5F;
            public static LayerDefinition createBodyLayer() {
                MeshDefinition mesh = new MeshDefinition();
                PartDefinition root = mesh.getRoot();
                PartDefinition body = root.addOrReplaceChild("body",
                    CubeListBuilder.create().texOffs(0, 0).addBox(-4, 0, -2, 8, 12, 4, new CubeDeformation(SCALE)),
                    PartPose.offset(0, 0, 0));
                body.addOrReplaceChild("head",
                    CubeListBuilder.create().texOffs(32, 0).addBox(-4, -8, -4, 8, 8, 8),
                    PartPose.offset(0, -6, 0));
                return LayerDefinition.create(mesh, 64, 64);
            }
            public void setupAnim(Entity entity, float a, float b, float c, float d, float e) {
                this.head.xRot = e;
            }
        }
        "#;
        let model = parse_java_model(source, "ModelModern.java", &[]).unwrap();
        assert_eq!(model.parts.len(), 2);
        assert_eq!((model.texture_width, model.texture_height), (64, 64));
        assert_eq!(model.parts[1].parent.as_deref(), Some("body"));
        assert_eq!(model.parts[0].children, vec!["head"]);
        assert_eq!(model.parts[1].cubes[0].uv_x, 32);
        assert!((model.parts[0].cubes[0].inflate - 0.5).abs() < 1e-8);
        assert!(model.animation_logic.contains("setupAnim"));
    }
}
