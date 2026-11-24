use anyhow::{bail, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::model::{Cube, ParsedModel, Part};
use super::java::{eval_int, eval_number, extract_balanced, find_numeric_constants, split_top_level, strip_java_comments, unquote_java_string};

#[derive(Clone, Copy)]
struct BipedPart {
    variable: &'static str,
    name: &'static str,
    offset: [f64; 3],
    cube: [f64; 10], // x,y,z,dx,dy,dz,u,v,inflate,mirror-as-number
}

const BIPED_PARTS: &[BipedPart] = &[
    BipedPart { variable:"field_78116_c", name:"head", offset:[0.0,0.0,0.0], cube:[-4.0,-8.0,-4.0,8.0,8.0,8.0,0.0,0.0,0.0,0.0] },
    BipedPart { variable:"bipedHead", name:"head", offset:[0.0,0.0,0.0], cube:[-4.0,-8.0,-4.0,8.0,8.0,8.0,0.0,0.0,0.0,0.0] },
    BipedPart { variable:"field_178720_f", name:"hat", offset:[0.0,0.0,0.0], cube:[-4.0,-8.0,-4.0,8.0,8.0,8.0,32.0,0.0,0.5,0.0] },
    BipedPart { variable:"bipedHeadwear", name:"hat", offset:[0.0,0.0,0.0], cube:[-4.0,-8.0,-4.0,8.0,8.0,8.0,32.0,0.0,0.5,0.0] },
    BipedPart { variable:"field_78115_e", name:"body", offset:[0.0,0.0,0.0], cube:[-4.0,0.0,-2.0,8.0,12.0,4.0,16.0,16.0,0.0,0.0] },
    BipedPart { variable:"bipedBody", name:"body", offset:[0.0,0.0,0.0], cube:[-4.0,0.0,-2.0,8.0,12.0,4.0,16.0,16.0,0.0,0.0] },
    BipedPart { variable:"field_178724_i", name:"right_arm", offset:[-5.0,2.0,0.0], cube:[-3.0,-2.0,-2.0,4.0,12.0,4.0,40.0,16.0,0.0,0.0] },
    BipedPart { variable:"bipedRightArm", name:"right_arm", offset:[-5.0,2.0,0.0], cube:[-3.0,-2.0,-2.0,4.0,12.0,4.0,40.0,16.0,0.0,0.0] },
    BipedPart { variable:"field_178723_h", name:"left_arm", offset:[5.0,2.0,0.0], cube:[-1.0,-2.0,-2.0,4.0,12.0,4.0,32.0,48.0,0.0,1.0] },
    BipedPart { variable:"bipedLeftArm", name:"left_arm", offset:[5.0,2.0,0.0], cube:[-1.0,-2.0,-2.0,4.0,12.0,4.0,32.0,48.0,0.0,1.0] },
    BipedPart { variable:"field_178722_k", name:"right_leg", offset:[-1.9,12.0,0.0], cube:[-2.0,0.0,-2.0,4.0,12.0,4.0,0.0,16.0,0.0,0.0] },
    BipedPart { variable:"bipedRightLeg", name:"right_leg", offset:[-1.9,12.0,0.0], cube:[-2.0,0.0,-2.0,4.0,12.0,4.0,0.0,16.0,0.0,0.0] },
    BipedPart { variable:"field_178721_j", name:"left_leg", offset:[1.9,12.0,0.0], cube:[-2.0,0.0,-2.0,4.0,12.0,4.0,16.0,48.0,0.0,1.0] },
    BipedPart { variable:"bipedLeftLeg", name:"left_leg", offset:[1.9,12.0,0.0], cube:[-2.0,0.0,-2.0,4.0,12.0,4.0,16.0,48.0,0.0,1.0] },
];

struct ModelBuilder {
    parts: Vec<Part>,
    by_variable: HashMap<String, usize>,
    uv_by_variable: HashMap<String, (i32, i32)>,
    mirror_by_variable: HashMap<String, bool>,
    aliases: HashMap<String, String>,
    biped_inflate: f64,
}

impl ModelBuilder {
    fn new(biped_inflate: f64) -> Self {
        Self {
            parts: Vec::new(), by_variable: HashMap::new(),
            uv_by_variable: HashMap::new(), mirror_by_variable: HashMap::new(),
            aliases: HashMap::new(), biped_inflate,
        }
    }

    fn ensure_part(&mut self, raw_variable: &str) -> usize {
        let variable = raw_variable.strip_prefix("this.").unwrap_or(raw_variable).to_owned();
        if let Some(index) = self.by_variable.get(&variable) { return *index; }

        let biped = BIPED_PARTS.iter().find(|part| part.variable == variable);
        let display_name = biped.map(|part| part.name).unwrap_or(variable.as_str()).to_owned();
        let mut part = Part {
            name: display_name.clone(),
            variable: variable.clone(),
            parent_variable: "root".to_owned(),
            offset: [0.0; 3],
            rotation: [0.0; 3],
            cubes: Vec::new(),
            parent: None,
            children: Vec::new(),
            source_index: self.parts.len(),
            source_name: String::new(),
            render_instance_index: 0,
            render_instance_spec: Value::Null,
            origin: [0.0,24.0,0.0],
            group_uuid: String::new(),
        };
        if let Some(base) = biped {
            part.offset = base.offset;
            let c = base.cube;
            part.cubes.push(Cube {
                x:c[0], y:c[1], z:c[2], dx:c[3], dy:c[4], dz:c[5],
                uv_x:c[6] as i32, uv_y:c[7] as i32,
                mirror:c[9] != 0.0, inflate:c[8] + self.biped_inflate,
                name:format!("{display_name}_base"),
            });
        }
        let index = self.parts.len();
        self.parts.push(part);
        self.by_variable.insert(variable, index);
        index
    }

    fn resolve_alias(&self, raw: &str) -> String {
        let value = raw.strip_prefix("this.").unwrap_or(raw);
        self.aliases.get(value).cloned().unwrap_or_else(|| value.to_owned())
    }

    fn link_child(&mut self, parent_index: usize, child_variable: &str) {
        let child_index = self.ensure_part(child_variable);
        if child_index == parent_index { return; }
        let parent_name = self.parts[parent_index].name.clone();
        let child_name = self.parts[child_index].name.clone();
        if let Some(old_parent) = self.parts[child_index].parent.clone() {
            if let Some(old_index) = self.parts.iter().position(|part| part.name == old_parent) {
                self.parts[old_index].children.retain(|name| name != &child_name);
            }
        }
        self.parts[child_index].parent = Some(parent_name.clone());
        self.parts[child_index].parent_variable = self.parts[parent_index].variable.clone();
        if !self.parts[parent_index].children.contains(&child_name) {
            self.parts[parent_index].children.push(child_name);
        }
    }
}

#[derive(Debug)]
struct Event {
    position: usize,
    kind: &'static str,
    variable: String,
    raw_args: String,
}

fn regex(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|error| anyhow::anyhow!("Invalid internal parser pattern {pattern}: {error}"))
}

fn source_class_name(clean: &str, source_name: &str) -> String {
    regex(r"\b(?:public\s+)?(?:final\s+)?class\s+([A-Za-z_$][\w$]*)")
        .unwrap().captures(clean).and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| Path::new(source_name).file_stem().and_then(|v| v.to_str()).unwrap_or("Model").to_owned())
}

fn constructor_body(
    clean: &str,
    class_name: &str,
    constructor_args: &[Value],
) -> (String, BTreeMap<String, f64>) {
    let pattern = regex(&format!(r"\b(?:public|protected|private)?\s*{}\s*\(([^)]*)\)\s*\{{", regex::escape(class_name))).unwrap();
    let mut candidates: Vec<(usize, String, Vec<String>)> = Vec::new();

    for capture in pattern.captures_iter(clean) {
        let whole = capture.get(0).unwrap();
        let params = capture.get(1).map(|m| m.as_str()).unwrap_or("");
        if let Ok((body, _)) = extract_balanced(clean, whole.end() - 1, '{', '}') {
            let names = if params.trim().is_empty() { Vec::new() } else {
                split_top_level(params, ',').iter().filter_map(|raw| {
                    regex(r"[A-Za-z_$][\w$]*").unwrap().find_iter(raw).last().map(|m| m.as_str().to_owned())
                }).collect()
            };
            candidates.push((names.len(), body, names));
        }
    }
    if candidates.is_empty() { return (clean.to_owned(), BTreeMap::new()); }

    if !constructor_args.is_empty() {
        candidates.sort_by_key(|(arity,_,_)| (if *arity == constructor_args.len() { 0usize } else { 1usize }, (*arity).abs_diff(constructor_args.len()), *arity));
    } else {
        candidates.sort_by_key(|(arity,_,_)| *arity);
    }
    let (_, body, names) = candidates.remove(0);
    let mut parameters = BTreeMap::new();
    for name in &names { parameters.insert(name.clone(), 0.0); }
    for (name, value) in names.iter().zip(constructor_args.iter()) {
        let number = value.as_f64().or_else(|| value.as_i64().map(|v| v as f64))
            .or_else(|| value.as_u64().map(|v| v as f64))
            .or_else(|| value.as_bool().map(|v| if v { 1.0 } else { 0.0 }));
        if let Some(number) = number { parameters.insert(name.clone(), number); }
    }
    (body, parameters)
}

/// Port of Beta 12's decompiled legacy ModelBase / ModelRenderer source parser.
/// Animation formulas are retained as source evidence; the procedural animation
/// interpreter is being ported independently so geometry is never discarded.
pub fn parse_legacy_java_model(
    text: &str,
    source_name: &str,
    constructor_args: &[Value],
) -> Result<ParsedModel> {
    let clean = strip_java_comments(text);
    let mut warnings = Vec::new();
    let class_name = source_class_name(&clean, source_name);
    let mut constants = find_numeric_constants(&clean);
    let (body, parameter_defaults) = constructor_body(&clean, &class_name, constructor_args);
    constants.extend(parameter_defaults);

    let mut texture_width = 64;
    let mut texture_height = 32;
    for (field_name, is_width) in [
        ("field_78090_t", true), ("textureWidth", true),
        ("field_78089_u", false), ("textureHeight", false),
    ] {
        if let Some(capture) = regex(&format!(r"(?:this\.)?{}\s*=\s*([^;]+);", regex::escape(field_name)))?.captures(&body) {
            let expression = capture.get(1).unwrap().as_str();
            if let Ok(value) = eval_int(expression, &constants) {
                if is_width { texture_width = value.max(1) as u32; } else { texture_height = value.max(1) as u32; }
            }
        }
    }

    let mut biped_inflate = 0.0;
    if let Some(capture) = regex(r"\bsuper\s*\(([^;]*)\)\s*;")?.captures(&body) {
        let args = split_top_level(capture.get(1).unwrap().as_str(), ',');
        if let Some(first) = args.first() {
            if let Ok(value) = eval_number(first, &constants) { biped_inflate = value; }
        }
        if args.len() >= 4 {
            if let (Ok(width), Ok(height)) = (
                eval_int(&args[args.len()-2], &constants),
                eval_int(&args[args.len()-1], &constants),
            ) {
                texture_width = width.max(1) as u32;
                texture_height = height.max(1) as u32;
            }
        }
    }

    let mut builder = ModelBuilder::new(biped_inflate);
    let declaration = regex(&format!(r"\bclass\s+{}(?:\s+extends\s+([A-Za-z_$][\w$.]*))?", regex::escape(&class_name)))?;
    let superclass = declaration.captures(&clean).and_then(|c| c.get(1))
        .map(|m| m.as_str().rsplit('.').next().unwrap_or(m.as_str()).to_owned()).unwrap_or_default();
    let biped_source = matches!(superclass.as_str(), "ModelBiped" | "ModelPlayer")
        || BIPED_PARTS.iter().any(|part| body.contains(part.variable));
    if biped_source {
        let mut seen = std::collections::HashSet::new();
        for base in BIPED_PARTS {
            if seen.insert(base.name) && (body.contains(base.variable) || matches!(superclass.as_str(), "ModelBiped" | "ModelPlayer") || superclass.starts_with("Model")) {
                builder.ensure_part(base.variable);
                for alias in BIPED_PARTS.iter().filter(|alias| alias.name == base.name) {
                    let index = builder.by_variable[base.variable];
                    builder.by_variable.insert(alias.variable.to_owned(), index);
                }
            }
        }
    }

    let mut events: Vec<Event> = Vec::new();
    let renderer_re = regex(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*=\s*new\s+ModelRenderer\s*\(")?;
    for capture in renderer_re.captures_iter(&body) {
        let full = capture.get(0).unwrap();
        if let Ok((args, _)) = extract_balanced(&body, full.end()-1, '(', ')') {
            events.push(Event { position:full.start(), kind:"create", variable:capture.get(1).unwrap().as_str().to_owned(), raw_args:args });
        }
    }

    let call_specs: &[(&str, &[&str])] = &[
        ("set_uv", &["func_78784_a", "setTextureOffset"]),
        ("add_box", &["func_78789_a", "func_78790_a", "addBox"]),
        ("offset", &["func_78793_a", "setRotationPoint"]),
        ("add_child", &["func_78792_a", "addChild"]),
    ];
    for (kind, methods) in call_specs {
        let method_alt = methods.iter().map(|name| regex::escape(name)).collect::<Vec<_>>().join("|");
        let pattern = regex(&format!(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*(?:{method_alt})\s*\("))?;
        for capture in pattern.captures_iter(&body) {
            let full = capture.get(0).unwrap();
            if let Ok((args, _)) = extract_balanced(&body, full.end()-1, '(', ')') {
                events.push(Event { position:full.start(), kind:*kind, variable:capture.get(1).unwrap().as_str().to_owned(), raw_args:args });
            }
        }
    }

    let rotation_helper = regex(r"(?:this\.)?[A-Za-z_$][\w$]*(?:Rotation|Rotate|rotation|rotate)[A-Za-z_$\w]*\s*\(")?;
    for capture in rotation_helper.captures_iter(&body) {
        let full = capture.get(0).unwrap();
        if let Ok((raw_args, _)) = extract_balanced(&body, full.end()-1, '(', ')') {
            let args = split_top_level(&raw_args, ',');
            if args.len() >= 4 {
                let variable = args[0].trim().strip_prefix("this.").unwrap_or(args[0].trim());
                if regex(r"^[A-Za-z_$][\w$]*$")?.is_match(variable) {
                    events.push(Event { position:full.start(), kind:"rotation", variable:variable.to_owned(), raw_args:args[1..4].join(",") });
                }
            }
        }
    }

    let assignment_specs: &[(&str, &[&str])] = &[
        ("rotation_x", &["field_78795_f", "rotateAngleX", "xRot"]),
        ("rotation_y", &["field_78796_g", "rotateAngleY", "yRot"]),
        ("rotation_z", &["field_78808_h", "rotateAngleZ", "zRot"]),
        ("mirror", &["field_78809_i", "mirror"]),
    ];
    for (kind, fields) in assignment_specs {
        let field_alt = fields.iter().map(|field| regex::escape(field)).collect::<Vec<_>>().join("|");
        let pattern = regex(&format!(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*(?:{field_alt})\s*=\s*([^;]+);"))?;
        for capture in pattern.captures_iter(&body) {
            events.push(Event {
                position:capture.get(0).unwrap().start(), kind,
                variable:capture.get(1).unwrap().as_str().to_owned(),
                raw_args:capture.get(2).unwrap().as_str().to_owned(),
            });
        }
    }

    let alias_pattern = regex(r"(?:\bModelRenderer\s+)?([A-Za-z_$][\w$]*)\s*=\s*(?:this\.)?([A-Za-z_$][\w$]*)\s*;")?;
    for capture in alias_pattern.captures_iter(&body) {
        let full = capture.get(0).unwrap();
        let lhs = capture.get(1).unwrap().as_str();
        if lhs.starts_with("var") || full.as_str().contains("ModelRenderer") {
            events.push(Event { position:full.start(), kind:"alias", variable:lhs.to_owned(), raw_args:capture.get(2).unwrap().as_str().to_owned() });
        }
    }

    let delta_specs: &[(&str, &[&str])] = &[
        ("rotation_delta_x", &["field_78795_f", "rotateAngleX", "xRot"]),
        ("rotation_delta_y", &["field_78796_g", "rotateAngleY", "yRot"]),
        ("rotation_delta_z", &["field_78808_h", "rotateAngleZ", "zRot"]),
        ("position_delta_x", &["field_78798_e", "rotationPointX", "x"]),
        ("position_delta_y", &["field_78797_d", "rotationPointY", "y"]),
        ("position_delta_z", &["field_78800_c", "rotationPointZ", "z"]),
    ];
    for (kind, fields) in delta_specs {
        let field_alt = fields.iter().map(|field| regex::escape(field)).collect::<Vec<_>>().join("|");
        let pattern = regex(&format!(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*(?:{field_alt})\s*([+-])=\s*([^;]+);"))?;
        for capture in pattern.captures_iter(&body) {
            events.push(Event {
                position:capture.get(0).unwrap().start(), kind,
                variable:capture.get(1).unwrap().as_str().to_owned(),
                raw_args:format!("{}{}", capture.get(2).unwrap().as_str(), capture.get(3).unwrap().as_str()),
            });
        }
    }

    let model_box_re = regex(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*(?:field_78804_l|cubeList)\s*\.\s*add\s*\(\s*new\s+ModelBox\s*\(")?;
    for capture in model_box_re.captures_iter(&body) {
        let full = capture.get(0).unwrap();
        if let Ok((args, _)) = extract_balanced(&body, full.end()-1, '(', ')') {
            events.push(Event { position:full.start(), kind:"model_box", variable:capture.get(1).unwrap().as_str().to_owned(), raw_args:args });
        }
    }

    events.sort_by(|a,b| a.position.cmp(&b.position).then_with(|| a.kind.cmp(b.kind)));

    for event in events {
        if event.kind == "alias" {
            let rhs = event.raw_args.strip_prefix("this.").unwrap_or(&event.raw_args).to_owned();
            let resolved = builder.aliases.get(&rhs).cloned().unwrap_or(rhs);
            builder.aliases.insert(event.variable, resolved);
            continue;
        }

        let variable = builder.resolve_alias(&event.variable);
        let index = builder.ensure_part(&variable);
        let args = split_top_level(&event.raw_args, ',');
        let parsed: Result<()> = (|| {
            match event.kind {
                "create" => {
                    if args.len() >= 3 {
                        builder.uv_by_variable.insert(variable.clone(), (
                            eval_int(&args[args.len()-2], &constants)?,
                            eval_int(&args[args.len()-1], &constants)?,
                        ));
                    }
                }
                "set_uv" if args.len() >= 2 => {
                    builder.uv_by_variable.insert(variable.clone(), (eval_int(&args[0], &constants)?, eval_int(&args[1], &constants)?));
                }
                "offset" if args.len() >= 3 => {
                    builder.parts[index].offset = [eval_number(&args[0], &constants)?, eval_number(&args[1], &constants)?, eval_number(&args[2], &constants)?];
                }
                "add_child" if !args.is_empty() => {
                    let child = args[0].trim().strip_prefix("this.").unwrap_or(args[0].trim());
                    builder.link_child(index, child);
                }
                "rotation" if args.len() >= 3 => {
                    builder.parts[index].rotation = [eval_number(&args[0], &constants)?, eval_number(&args[1], &constants)?, eval_number(&args[2], &constants)?];
                }
                "rotation_x" | "rotation_y" | "rotation_z" => {
                    let value = eval_number(&event.raw_args, &constants)?;
                    let axis = match event.kind { "rotation_x" => 0, "rotation_y" => 1, _ => 2 };
                    builder.parts[index].rotation[axis] = value;
                }
                "rotation_delta_x" | "rotation_delta_y" | "rotation_delta_z" => {
                    let raw = event.raw_args.trim();
                    let sign = if raw.starts_with('-') { -1.0 } else { 1.0 };
                    let expr = raw.strip_prefix('+').or_else(|| raw.strip_prefix('-')).unwrap_or(raw);
                    let axis = match event.kind { "rotation_delta_x" => 0, "rotation_delta_y" => 1, _ => 2 };
                    builder.parts[index].rotation[axis] += eval_number(expr, &constants)? * sign;
                }
                "position_delta_x" | "position_delta_y" | "position_delta_z" => {
                    let raw = event.raw_args.trim();
                    let sign = if raw.starts_with('-') { -1.0 } else { 1.0 };
                    let expr = raw.strip_prefix(['+', '-']).unwrap_or(raw);
                    let axis = match event.kind { "position_delta_x" => 0, "position_delta_y" => 1, _ => 2 };
                    builder.parts[index].offset[axis] += eval_number(expr, &constants)? * sign;
                }
                "mirror" => {
                    builder.mirror_by_variable.insert(variable.clone(), event.raw_args.trim().eq_ignore_ascii_case("true"));
                }
                "add_box" => {
                    let mut values = args.clone();
                    let mut cube_name = format!("{}_cube{}", builder.parts[index].name, builder.parts[index].cubes.len()+1);
                    if values.first().map(|v| v.trim().starts_with('"')).unwrap_or(false) {
                        cube_name = unquote_java_string(values.remove(0));
                    }
                    if values.len() >= 6 {
                        let x = eval_number(&values[0], &constants)?;
                        let y = eval_number(&values[1], &constants)?;
                        let z = eval_number(&values[2], &constants)?;
                        let dx = eval_number(&values[3], &constants)?;
                        let dy = eval_number(&values[4], &constants)?;
                        let dz = eval_number(&values[5], &constants)?;
                        let inflate = if values.len() >= 7 { eval_number(&values[6], &constants)? } else { 0.0 };
                        let (uv_x, uv_y) = builder.uv_by_variable.get(&variable).copied().unwrap_or((0,0));
                        let mirror = builder.mirror_by_variable.get(&variable).copied().unwrap_or(false);
                        builder.parts[index].cubes.push(Cube { x,y,z,dx,dy,dz,uv_x,uv_y,mirror,inflate,name:cube_name });
                    }
                }
                "model_box" => {
                    if args.len() >= 9 {
                        let first = args[0].trim();
                        let has_owner_arg = eval_number(first, &constants).is_err();
                        let offset = if has_owner_arg { 1 } else { 0 };
                        let values = &args[offset..];
                        if values.len() >= 8 {
                            let uv_x = eval_int(&values[0], &constants)?;
                            let uv_y = eval_int(&values[1], &constants)?;
                            let x = eval_number(&values[2], &constants)?;
                            let y = eval_number(&values[3], &constants)?;
                            let z = eval_number(&values[4], &constants)?;
                            let dx = eval_number(&values[5], &constants)?;
                            let dy = eval_number(&values[6], &constants)?;
                            let dz = eval_number(&values[7], &constants)?;
                            let inflate = if values.len() >= 9 { eval_number(&values[8], &constants)? } else { 0.0 };
                            let mirror = if values.len() >= 10 { values[9].trim().eq_ignore_ascii_case("true") } else { builder.mirror_by_variable.get(&variable).copied().unwrap_or(false) };
                            let name = format!("{}_cube{}", builder.parts[index].name, builder.parts[index].cubes.len()+1);
                            builder.parts[index].cubes.push(Cube { x,y,z,dx,dy,dz,uv_x,uv_y,mirror,inflate,name });
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = parsed {
            warnings.push(format!("Skipped {} operation for {}: {}", event.kind, variable, error));
        }
    }

    let mut keep = vec![true; builder.parts.len()];
    for (index, part) in builder.parts.iter().enumerate() {
        if part.cubes.is_empty() && part.children.is_empty() && part.parent.is_none() {
            keep[index] = false;
        }
    }
    let mut remapped = HashMap::<String, usize>::new();
    let mut parts = Vec::new();
    for (index, mut part) in builder.parts.into_iter().enumerate() {
        if keep[index] {
            part.source_index = parts.len();
            remapped.insert(part.name.clone(), parts.len());
            parts.push(part);
        }
    }
    if parts.is_empty() || parts.iter().map(|p| p.cubes.len()).sum::<usize>() == 0 {
        bail!("No legacy ModelRenderer/ModelBox geometry was recovered from Java source.");
    }

    for part in &mut parts {
        part.children.retain(|name| remapped.contains_key(name));
        if part.parent.as_ref().map(|name| !remapped.contains_key(name)).unwrap_or(false) {
            part.parent = None;
            part.parent_variable = "root".to_owned();
        }
    }

    let mut model = ParsedModel::new(class_name, texture_width, texture_height, parts, warnings);
    model.source_format = "legacy_java_source".to_owned();
    model.constructor_args = constructor_args.to_vec();
    model.animation_analysis = serde_json::json!({
        "source_format":"legacy_java_source",
        "mode":"geometry_recovered",
        "animation_port_status":"pending",
        "note":"The source model geometry is reconstructed. Procedural animation interpretation remains a separate parity stage."
    });
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructs_legacy_model_renderer_geometry_and_hierarchy() {
        let source = r#"
            public class ModelTest extends ModelBase {
                public ModelRenderer body;
                public ModelRenderer head;
                public ModelTest() {
                    this.textureWidth = 64;
                    this.textureHeight = 32;
                    this.body = new ModelRenderer(this, 0, 0);
                    this.body.setRotationPoint(0, 4, 0);
                    this.body.addBox(-2, -3, -1, 4, 6, 2, 0.25F);
                    this.head = new ModelRenderer(this, 16, 0);
                    this.head.addBox(-1, -1, -1, 2, 2, 2);
                    this.body.addChild(this.head);
                }
            }
        "#;
        let model = parse_legacy_java_model(source, "ModelTest.java", &[]).unwrap();
        assert_eq!(model.class_name, "ModelTest");
        assert_eq!(model.texture_width, 64);
        assert_eq!(model.texture_height, 32);
        let body = model.parts.iter().find(|p| p.name == "body").unwrap();
        assert_eq!(body.cubes.len(), 1);
        assert_eq!(body.offset, [0.0,4.0,0.0]);
        assert!(body.children.contains(&"head".to_owned()));
        let head = model.parts.iter().find(|p| p.name == "head").unwrap();
        assert_eq!(head.parent.as_deref(), Some("body"));
        assert_eq!(head.cubes[0].uv_x, 16);
    }

    #[test]
    fn keeps_constructor_alias_position_adjustments() {
        let source = r#"
            public class ModelAlias extends ModelBase {
                public ModelRenderer head;
                public ModelAlias() {
                    this.head = new ModelRenderer(this, 0, 0);
                    this.head.addBox(-1, -1, -1, 2, 2, 2);
                    ModelRenderer var10000 = this.head;
                    var10000.rotationPointY += 6.0F;
                }
            }
        "#;
        let model = parse_legacy_java_model(source, "ModelAlias.java", &[]).unwrap();
        let head = model.parts.iter().find(|p| p.name == "head").unwrap();
        assert_eq!(head.offset[1], 6.0);
    }
}
