use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
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
