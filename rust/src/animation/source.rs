use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::model::Part;
use crate::source::java::{extract_balanced, find_numeric_constants, split_top_level, strip_java_comments};

#[derive(Debug, Clone)]
struct JavaMethod {
    params: Vec<String>,
    body: String,
    source: String,
}

#[derive(Debug, Clone)]
enum Event {
    Local { name: String, operator: String, expression: String },
    Assignment { bone: String, field: String, operator: String, expression: String },
    Chain { targets: Vec<(String, String)>, expression: String },
    Helper { name: String, arguments: Vec<String> },
}

#[derive(Debug, Clone)]
struct Formula {
    bone: String,
    channel: String,
    axis: String,
    expression: String,
    animation_class: String,
}

#[derive(Debug, Clone)]
pub struct SourceAnimation {
    pub analysis: Value,
    pub blockbench_animations: Vec<Value>,
    pub source: String,
    pub logic: String,
    pub warnings: Vec<String>,
}

fn collect_methods(source: &str) -> HashMap<String, Vec<JavaMethod>> {
    let pattern = Regex::new(
        r"(?m)^[ \t]*(?:(?:public|protected|private|static|final|synchronized)\s+)*[A-Za-z_$][\w$<>\[\].?, ]*\s+([A-Za-z_$][\w$]*)\s*\(([^;{}]*)\)\s*(?:throws\s+[^{]+)?\{"
    ).unwrap();
    let parameter_pattern = Regex::new(r"([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:\[\])?\s*$").unwrap();
    let annotation_pattern = Regex::new(r"@\w+(?:\([^)]*\))?\s*").unwrap();
    let mut methods = HashMap::<String, Vec<JavaMethod>>::new();
    let mut cursor = 0usize;

    while cursor < source.len() {
        let Some(found) = pattern.find(&source[cursor..]) else { break; };
        let start = cursor + found.start();
        let end_header = cursor + found.end();
        let capture = pattern.captures(&source[start..end_header]).unwrap();
        let name = capture[1].to_owned();
        let raw_parameters = capture[2].to_owned();
        let Some(open) = source[start..end_header].rfind('{').map(|index| start + index) else {
            cursor = end_header;
            continue;
        };
        let Ok((body, end)) = extract_balanced(source, open, '{', '}') else {
            cursor = end_header;
            continue;
        };

        let params = if raw_parameters.trim().is_empty() {
            Vec::new()
        } else {
            split_top_level(&raw_parameters, ',').into_iter().filter_map(|raw| {
                let cleaned = annotation_pattern.replace_all(&raw, "").trim().to_owned();
                parameter_pattern.captures(&cleaned).map(|capture| capture[1].to_owned())
            }).collect()
        };

        methods.entry(name).or_default().push(JavaMethod {
            params,
            body,
            source: source[start..end].to_owned(),
        });
        cursor = end;
    }
    methods
}

fn canonical_part(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase).collect()
}

fn channel_field(field: &str) -> Option<(&'static str, &'static str)> {
    match field {
        "field_78795_f" | "rotateAngleX" | "xRot" => Some(("rotation", "x")),
        "field_78796_g" | "rotateAngleY" | "yRot" => Some(("rotation", "y")),
        "field_78808_h" | "rotateAngleZ" | "zRot" => Some(("rotation", "z")),
        "field_78800_c" | "rotationPointX" | "x" => Some(("position", "x")),
        "field_78797_d" | "rotationPointY" | "y" => Some(("position", "y")),
        "field_78798_e" | "rotationPointZ" | "z" => Some(("position", "z")),
        _ => None,
    }
}

fn number_text(value: f64) -> String {
    if value.abs() < 1e-10 { "0".to_owned() }
    else if (value - value.round()).abs() < 1e-9 { (value.round() as i64).to_string() }
    else { format!("{value:.10}") }
}

fn normalize_expression(expression: &str, substitutions: &HashMap<String, String>) -> String {
    let casts = Regex::new(r"\((?:float|double|int|long)\)\s*").unwrap();
    let suffix = Regex::new(r"([0-9])([fFdD])\b").unwrap();
    let mut result = expression.trim().to_owned();
    result = casts.replace_all(&result, "").to_string();
    result = result.replace("MathHelper.func_76134_b", "cos")
        .replace("MathHelper.cos", "cos")
        .replace("MathHelper.func_76126_a", "sin")
        .replace("MathHelper.sin", "sin")
        .replace("MathHelper.func_76135_e", "abs")
        .replace("MathHelper.abs", "abs")
        .replace("Math.toRadians", "radians")
        .replace("Math.toDegrees", "degrees")
        .replace("Math.pow", "pow")
        .replace("Math.PI", &std::f64::consts::PI.to_string())
        .replace("this.", "");
    result = suffix.replace_all(&result, "$1").to_string();

    let mut pairs: Vec<(&String, &String)> = substitutions.iter().collect();
    pairs.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    for (name, value) in pairs {
        if name.is_empty() { continue; }
        let token = Regex::new(&format!(r"\b{}\b", regex::escape(name))).unwrap();
        result = token.replace_all(&result, format!("({value})").as_str()).to_string();
    }
    result
}

fn is_dynamic(expression: &str) -> bool {
    ["limbSwing", "limbSwingAmount", "ageInTicks", "netHeadYaw", "headPitch"]
        .iter().any(|name| expression.contains(name))
}

fn classify(expression: &str) -> &'static str {
    let head = expression.contains("netHeadYaw") || expression.contains("headPitch");
    let walk = expression.contains("limbSwing") || expression.contains("limbSwingAmount");
    let idle = expression.contains("ageInTicks");
    if head && (walk || idle) { "combined" }
    else if head { "head_tracking" }
    else if walk && idle { "combined" }
    else if walk { "walk_cycle" }
    else if idle { "idle" }
    else { "static" }
}

fn resolve_part(raw: &str, canonical: &HashMap<String, String>, names: &BTreeSet<String>) -> String {
    let value = raw.trim().strip_prefix("this.").unwrap_or(raw.trim());
    canonical.get(&canonical_part(value)).cloned()
        .or_else(|| names.contains(value).then(|| value.to_owned()))
        .unwrap_or_else(|| value.to_owned())
}

fn process_body(
    body: &str,
    initial_values: &HashMap<String, String>,
    methods: &HashMap<String, Vec<JavaMethod>>,
    canonical: &HashMap<String, String>,
    part_names: &BTreeSet<String>,
    formulas: &mut BTreeMap<(String, String, String), Formula>,
    warnings: &mut Vec<String>,
    depth: usize,
) {
    if depth > 4 {
        warnings.push("Java animation helper recursion exceeded four levels.".to_owned());
        return;
    }

    let local_pattern = Regex::new(
        r"(?:\b(?:final\s+)?(?:float|double|int|boolean)\s+)?\b([A-Za-z_$][\w$]*)\s*(=|\+=|-=)\s*([^;]+);"
    ).unwrap();
    let assignment_pattern = Regex::new(
        r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*([A-Za-z_$][\w$]*)\s*(=|\+=|-=)\s*([^;]+);"
    ).unwrap();
    let chain_pattern = Regex::new(
        r"((?:(?:this\.)?[A-Za-z_$][\w$]*\s*\.\s*[A-Za-z_$][\w$]*\s*=\s*){2,})([^;]+);"
    ).unwrap();
    let target_pattern = Regex::new(
        r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*([A-Za-z_$][\w$]*)\s*=\s*"
    ).unwrap();
    let call_pattern = Regex::new(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\(").unwrap();

    let chain_ranges: Vec<(usize, usize)> = chain_pattern.find_iter(body)
        .map(|found| (found.start(), found.end())).collect();
    let mut events = Vec::<(usize, u8, Event)>::new();

    for capture in local_pattern.captures_iter(body) {
        let Some(found) = capture.get(0) else { continue; };
        let previous = if found.start() == 0 { '\0' } else { body[..found.start()].chars().last().unwrap_or('\0') };
        let name = capture[1].to_owned();
        if previous == '.' || part_names.contains(&name) || channel_field(&name).is_some() {
            continue;
        }
        if chain_ranges.iter().any(|(start, end)| *start <= found.start() && found.start() < *end) {
            continue;
        }
        events.push((found.start(), 0, Event::Local {
            name,
            operator: capture[2].to_owned(),
            expression: capture[3].trim().to_owned(),
        }));
    }

    for capture in chain_pattern.captures_iter(body) {
        let Some(found) = capture.get(0) else { continue; };
        let prefix = capture.get(1).map(|v| v.as_str()).unwrap_or("");
        let targets = target_pattern.captures_iter(prefix).map(|entry| {
            (entry[1].to_owned(), entry[2].to_owned())
        }).collect::<Vec<_>>();
        if targets.len() >= 2 {
            events.push((found.start(), 1, Event::Chain {
                targets,
                expression: capture[2].trim().to_owned(),
            }));
        }
    }

    for capture in assignment_pattern.captures_iter(body) {
        let Some(found) = capture.get(0) else { continue; };
        if chain_ranges.iter().any(|(start, end)| *start <= found.start() && found.start() < *end) {
            continue;
        }
        let bone = resolve_part(&capture[1], canonical, part_names);
        if !part_names.contains(&bone) || channel_field(&capture[2]).is_none() {
            continue;
        }
        events.push((found.start(), 2, Event::Assignment {
            bone,
            field: capture[2].to_owned(),
            operator: capture[3].to_owned(),
            expression: capture[4].trim().to_owned(),
        }));
    }

    for capture in call_pattern.captures_iter(body) {
        let name = capture[1].to_owned();
        if !methods.contains_key(&name) || matches!(name.as_str(), "sin" | "cos" | "abs" | "super") {
            continue;
        }
        let Some(found) = capture.get(0) else { continue; };
        let open = found.end().saturating_sub(1);
        let Ok((raw_args, _)) = extract_balanced(body, open, '(', ')') else { continue; };
        let arguments = if raw_args.trim().is_empty() { Vec::new() } else { split_top_level(&raw_args, ',') };
        events.push((found.start(), 3, Event::Helper {
            name,
            arguments: arguments.into_iter().map(|value| value.trim().to_owned()).collect(),
        }));
    }

    events.sort_by_key(|(position, priority, _)| (*position, *priority));
    let mut values = initial_values.clone();

    for (_, _, event) in events {
        match event {
            Event::Local { name, operator, expression } => {
                let mut value = normalize_expression(&expression, &values);
                if operator != "=" {
                    if let Some(previous) = values.get(&name) {
                        let op = if operator == "+=" { "+" } else { "-" };
                        value = format!("({previous}) {op} ({value})");
                    }
                }
                if let Some(previous) = values.get(&name) {
                    if is_dynamic(previous) && !is_dynamic(&value) && operator == "=" {
                        continue;
                    }
                }
                values.insert(name, value);
            }
            Event::Assignment { bone, field, operator, expression } => {
                let Some((channel, axis)) = channel_field(&field) else { continue; };
                let mut value = normalize_expression(&expression, &values);
                if value.contains("instanceof") || value.contains("null") || value.contains("new ") {
                    continue;
                }
                let key = (bone.clone(), channel.to_owned(), axis.to_owned());
                if operator != "=" {
                    if let Some(previous) = formulas.get(&key) {
                        let op = if operator == "+=" { "+" } else { "-" };
                        value = format!("({}) {op} ({value})", previous.expression);
                    }
                }
                formulas.insert(key, Formula {
                    bone, channel: channel.to_owned(), axis: axis.to_owned(),
                    animation_class: classify(&value).to_owned(), expression: value,
                });
            }
            Event::Chain { targets, expression } => {
                let value = normalize_expression(&expression, &values);
                for (raw_bone, field) in targets.into_iter().rev() {
                    let bone = resolve_part(&raw_bone, canonical, part_names);
                    if !part_names.contains(&bone) { continue; }
                    let Some((channel, axis)) = channel_field(&field) else { continue; };
                    formulas.insert((bone.clone(), channel.to_owned(), axis.to_owned()), Formula {
                        bone, channel: channel.to_owned(), axis: axis.to_owned(),
                        animation_class: classify(&value).to_owned(), expression: value.clone(),
                    });
                }
            }
            Event::Helper { name, arguments } => {
                let Some(overloads) = methods.get(&name) else { continue; };
                let Some(helper) = overloads.iter().find(|method| method.params.len() == arguments.len()) else {
                    continue;
                };
                let mut helper_values = values.clone();
                for (parameter, argument) in helper.params.iter().zip(arguments.iter()) {
                    helper_values.insert(parameter.clone(), normalize_expression(argument, &values));
                }
                process_body(
                    &helper.body, &helper_values, methods, canonical, part_names,
                    formulas, warnings, depth + 1
                );
            }
        }
    }
}

/// Recover common Java ModelPart/ModelRenderer assignments as symbolic formulas.
/// This intentionally leaves unsupported branches in the preserved Java sidecar.
pub fn extract_java_animation(
    source: &str,
    class_name: &str,
    parts: &[Part],
) -> Result<Option<SourceAnimation>> {
    let clean = strip_java_comments(source);
    let methods = collect_methods(&clean);
    let preferred = ["func_78087_a", "setRotationAngles", "setupAnim", "setAngles", "func_78088_a", "render", "renderToBuffer"];
    if !preferred.iter().any(|name| methods.contains_key(*name)) {
        return Ok(None);
    }

    let part_names: BTreeSet<String> = parts.iter().flat_map(|part| {
        [part.name.clone(), part.variable.clone(), part.source_name.clone()]
    }).filter(|name| !name.is_empty()).collect();
    let mut canonical = HashMap::<String, String>::new();
    for part in parts {
        for name in [&part.name, &part.variable, &part.source_name] {
            if !name.is_empty() {
                canonical.entry(canonical_part(name)).or_insert_with(|| part.name.clone());
            }
        }
    }

    let semantic = ["limbSwing", "limbSwingAmount", "ageInTicks", "netHeadYaw", "headPitch", "scaleFactor"];
    let numeric = find_numeric_constants(&clean);
    let mut scalar_values: HashMap<String, String> = numeric.iter()
        .filter(|(_, value)| value.is_finite())
        .map(|(name, value)| (name.clone(), number_text(*value)))
        .collect();

    let declared_float = Regex::new(
        r"\b(?:private|protected|public)?\s*(?:final\s+)?(?:float|double)\s+([A-Za-z_$][\w$]*)\s*(?:;|=)"
    ).unwrap();
    for capture in declared_float.captures_iter(&clean) {
        scalar_values.entry(capture[1].to_owned()).or_insert_with(|| "1.0".to_owned());
    }

    // setupAnim often caches its clock arguments in instance fields consumed by
    // renderToBuffer. Carry those aliases into the render method's environment.
    let field_cache_pattern = Regex::new(
        r"(?:this\.)?([A-Za-z_$][\w$]*)\s*=\s*(?:this\.)?([A-Za-z_$][\w$]*)\s*;"
    ).unwrap();
    for methods_for_name in methods.get("setupAnim").into_iter().flatten() {
        let mut param_values = HashMap::new();
        for (index, parameter) in methods_for_name.params.iter().skip(1).enumerate() {
            if let Some(value) = semantic.get(index) {
                param_values.insert(parameter.clone(), (*value).to_owned());
            }
        }
        for capture in field_cache_pattern.captures_iter(&methods_for_name.body) {
            if let Some(value) = param_values.get(&capture[2]) {
                scalar_values.insert(capture[1].to_owned(), value.clone());
            }
        }
    }

    let mut merged = BTreeMap::<(String, String, String, String), Formula>::new();
    let mut source_names = Vec::new();
    let mut source_bodies = Vec::<String>::new();
    let mut warnings = Vec::new();
    for name in preferred {
        let Some(candidates) = methods.get(name) else { continue; };
        for method in candidates {
            let mut substitutions = scalar_values.clone();
            for value in semantic { substitutions.insert(value.to_owned(), value.to_owned()); }

            if matches!(name, "setupAnim" | "setAngles" | "setRotationAngles" | "func_78087_a") {
                for (index, parameter) in method.params.iter().skip(1).enumerate() {
                    if let Some(value) = semantic.get(index) {
                        substitutions.insert(parameter.clone(), (*value).to_owned());
                    }
                }
            }

            let before = merged.len();
            let mut formulas = BTreeMap::new();
            process_body(
                &method.body, &substitutions, &methods, &canonical, &part_names,
                &mut formulas, &mut warnings, 0
            );
            for (_, formula) in formulas {
                let key = (
                    formula.bone.clone(), formula.channel.clone(),
                    formula.axis.clone(), formula.expression.clone()
                );
                merged.entry(key).or_insert(formula);
            }
            if merged.len() > before || !method.source.is_empty() {
                source_names.push(name.to_owned());
                if !source_bodies.contains(&method.source) {
                    source_bodies.push(method.source.clone());
                }
            }
        }
    }

    if merged.is_empty() {
        return Ok(None);
    }

    // Part-to-part formulas inherit the source animation class of their referenced
    // channel. Keep this conservative; unsupported references remain sidecar-only.
    let formulas: Vec<Formula> = merged.into_values().collect();
    let mut formula_json = Vec::new();
    let mut by_bone = serde_json::Map::new();
    let mut classification = BTreeMap::<String, BTreeSet<String>>::new();
    for formula in &formulas {
        formula_json.push(json!({
            "bone": formula.bone, "channel": formula.channel, "axis": formula.axis,
            "expression": formula.expression, "animation_class": formula.animation_class
        }));
        let mut channels = by_bone.get(&formula.bone).and_then(Value::as_object).cloned().unwrap_or_default();
        let values = channels.get(&formula.channel).and_then(Value::as_array).cloned().unwrap_or_default();
        let mut values = values;
        values.push(json!(formula.expression));
        channels.insert(formula.channel.clone(), Value::Array(values));
        by_bone.insert(formula.bone.clone(), Value::Object(channels));
        if formula.animation_class != "static" {
            classification.entry(formula.bone.clone()).or_default().insert(formula.animation_class.clone());
        }
    }
    let classification_json = classification.into_iter().map(|(bone, classes)| {
        (bone, json!(classes.into_iter().collect::<Vec<_>>()))
    }).collect::<serde_json::Map<String, Value>>();

    let mut analysis = json!({
        "source_format": "java_source",
        "source_method": source_names.join(", "),
        "formula_count": formulas.len(),
        "bones": by_bone,
        "classification": classification_json,
        "formulas": formula_json,
        "warnings": warnings.clone(),
    });
    let clips = crate::animation::preview::bake_blockbench_animations(class_name, parts, &analysis);
    analysis["baked_blockbench_clips"] = json!(clips.iter().filter_map(|clip| clip["name"].as_str()).collect::<Vec<_>>());
    Ok(Some(SourceAnimation {
        analysis,
        blockbench_animations: clips,
        source: source_names.join(", "),
        logic: source_bodies.join("\n\n"),
        warnings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Cube;

    fn sample_part() -> Part {
        Part {
            name: "wing".into(), variable: "wing".into(), parent_variable: "root".into(),
            offset: [0.0; 3], rotation: [0.0; 3],
            cubes: vec![Cube {
                x: 0.0, y: 0.0, z: 0.0, dx: 1.0, dy: 1.0, dz: 1.0,
                uv_x: 0, uv_y: 0, mirror: false, inflate: 0.0, name: "wing_cube".into()
            }],
            parent: None, children: Vec::new(), source_index: 0, source_name: "wing".into(),
            render_instance_index: 0, render_instance_spec: Value::Null,
            origin: [0.0, 24.0, 0.0], group_uuid: String::new(),
        }
    }

    #[test]
    fn recovers_setup_anim_formula_and_bakes_walk_clip() {
        let source = r#"
            public class ModelWing {
                public void setupAnim(Entity entity, float limbSwing, float limbSwingAmount,
                                      float ageInTicks, float netHeadYaw, float headPitch) {
                    this.wing.xRot = Mth.sin(limbSwing * 0.5F) * limbSwingAmount;
                    this.wing.zRot = Mth.cos(ageInTicks * 0.1F) * 0.25F;
                }
            }
        "#;
        let result = extract_java_animation(source, "ModelWing", &[sample_part()]).unwrap().unwrap();
        assert_eq!(result.analysis["formula_count"], 2);
        assert_eq!(result.analysis["formulas"][0]["animation_class"], "walk_cycle");
        assert_eq!(result.analysis["formulas"][1]["animation_class"], "idle");
        assert!(!result.blockbench_animations.is_empty());
        assert_eq!(result.blockbench_animations[0]["name"], "animation.model.walk");
    }

    #[test]
    fn tracks_local_reassignments_and_helper_methods_in_source_order() {
        let source = r#"
            public class ModelHelperAnim {
                private void animateWing(float amount) {
                    float angle = amount * 0.25F;
                    this.wing.xRot = angle;
                }
                public void setupAnim(Entity entity, float limbSwing, float limbSwingAmount,
                                      float ageInTicks, float netHeadYaw, float headPitch) {
                    float amount = Mth.sin(limbSwing);
                    this.animateWing(amount);
                    amount = Mth.cos(ageInTicks);
                    this.wing.zRot = amount;
                }
            }
        "#;
        let result = extract_java_animation(source, "ModelHelperAnim", &[sample_part()]).unwrap().unwrap();
        let formulas = result.analysis["formulas"].as_array().unwrap();
        assert!(formulas.iter().any(|formula| formula["axis"] == "x"
            && formula["expression"].as_str().unwrap().contains("limbSwing")));
        assert!(formulas.iter().any(|formula| formula["axis"] == "z"
            && formula["expression"].as_str().unwrap().contains("ageInTicks")));
    }
}
