use anyhow::{Context, Result};
use serde_json::{json, Map, Value as JsonValue};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::bytecode::classfile::{ClassFile, ConstantValue, MemberInfo};
use crate::bytecode::{decode_instructions, parse_method_descriptor, Instruction, JvmType};
use crate::model::Part;

#[derive(Clone, Debug, PartialEq)]
enum Symbol {
    This,
    Part(usize),
    Expr(String),
}

impl Symbol {
    fn text(&self, parts: &[Part]) -> String {
        match self {
            Self::This => "this".to_owned(),
            Self::Part(index) => parts.get(*index).map(|part| part.name.clone()).unwrap_or_else(|| format!("part_{index}")),
            Self::Expr(value) => value.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct Formula {
    bone: String,
    channel: String,
    axis: String,
    expression: String,
    offset: usize,
    animation_class: String,
}

struct AnimationContext<'a> {
    class: &'a ClassFile,
    parts: &'a [Part],
    scalar_fields: &'a BTreeMap<String, f64>,
    part_fields: HashMap<String, usize>,
    part_state: HashMap<(String, String, String), String>,
    formulas: Vec<Formula>,
    warnings: Vec<String>,
    active_methods: Vec<usize>,
    method_indexes: HashMap<(String, String), usize>,
    current_class: String,
}

#[derive(Clone)]
pub struct LegacyAnimation {
    pub analysis: JsonValue,
    pub logic: String,
    pub source: String,
    pub blockbench_animations: Vec<JsonValue>,
}

fn clean_number(value: f64) -> String {
    if value.abs() < 1e-10 { return "0".to_owned(); }
    if (value - value.round()).abs() < 1e-9 { return format!("{}", value.round() as i64); }
    format!("{value:.8}")
}

fn class_simple(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name).rsplit('$').next().unwrap_or(name)
}

fn read_u16(bytes: &[u8], index: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(index)?, *bytes.get(index + 1)?]))
}

fn load_index(instruction: &Instruction) -> Option<usize> {
    let op = instruction.opcode;
    let implicit = match op {
        0x1a..=0x1d => Some((op - 0x1a) as usize),
        0x1e..=0x21 => Some((op - 0x1e) as usize),
        0x22..=0x25 => Some((op - 0x22) as usize),
        0x26..=0x29 => Some((op - 0x26) as usize),
        0x2a..=0x2d => Some((op - 0x2a) as usize),
        0x3b..=0x3e => Some((op - 0x3b) as usize),
        0x3f..=0x42 => Some((op - 0x3f) as usize),
        0x43..=0x46 => Some((op - 0x43) as usize),
        0x47..=0x4a => Some((op - 0x47) as usize),
        0x4b..=0x4e => Some((op - 0x4b) as usize),
        _ => None,
    };
    if implicit.is_some() { return implicit; }
    match op {
        0x15..=0x19 | 0x36..=0x3a | 0xa9 => instruction.operands.first().map(|v| *v as usize),
        0xc4 if instruction.operands.len() >= 3 => read_u16(&instruction.operands, 1).map(|v| v as usize),
        _ => None,
    }
}

fn parameter_names(method: &MemberInfo) -> Result<BTreeMap<usize, String>> {
    let descriptor = parse_method_descriptor(&method.descriptor)?;
    let name = method.name.as_str();
    let names: Vec<&str> = if matches!(name, "func_78087_a" | "setRotationAngles") {
        vec!["limbSwing", "limbSwingAmount", "ageInTicks", "netHeadYaw", "headPitch", "scaleFactor", "entity"]
    } else if name == "setupAnim" {
        vec!["entity", "limbSwing", "limbSwingAmount", "ageInTicks", "netHeadYaw", "headPitch", "scaleFactor"]
    } else if matches!(name, "func_78088_a" | "render") {
        vec!["entity", "limbSwing", "limbSwingAmount", "ageInTicks", "netHeadYaw", "headPitch", "scaleFactor"]
    } else {
        Vec::new()
    };
    let mut result = BTreeMap::new();
    let mut slot = if method.access_flags & 0x0008 != 0 { 0 } else { 1 };
    for (index, ty) in descriptor.parameters.iter().enumerate() {
        let value = names.get(index).copied().map(str::to_owned)
            .unwrap_or_else(|| format!("arg{}", index + 1));
        result.insert(slot, value);
        slot += ty.slots();
    }
    Ok(result)
}

fn constant_expression(class: &ClassFile, instruction: &Instruction) -> Option<String> {
    let op = instruction.opcode;
    let number = match op {
        0x02 => Some(-1.0),
        0x03..=0x08 => Some((op - 0x03) as f64),
        0x09..=0x0a => Some((op - 0x09) as f64),
        0x0b..=0x0d => Some((op - 0x0b) as f64),
        0x0e..=0x0f => Some((op - 0x0e) as f64),
        0x10 => instruction.operands.first().map(|v| *v as i8 as f64),
        0x11 => instruction.operands.get(0..2).map(|v| i16::from_be_bytes([v[0], v[1]]) as f64),
        _ => None,
    };
    if let Some(value) = number { return Some(clean_number(value)); }
    let index = match op {
        0x12 => *instruction.operands.first()? as u16,
        0x13 | 0x14 => read_u16(&instruction.operands, 0)?,
        _ => return None,
    };
    match class.constant(index)? {
        ConstantValue::Integer(v) => Some(v.to_string()),
        ConstantValue::Float(v) => Some(clean_number(v as f64)),
        ConstantValue::Long(v) => Some(v.to_string()),
        ConstantValue::Double(v) => Some(clean_number(v)),
        ConstantValue::String(v) => Some(format!("{v:?}")),
        ConstantValue::Class(v) => Some(v),
    }
}

fn pop(stack: &mut Vec<Symbol>) -> Symbol {
    stack.pop().unwrap_or_else(|| Symbol::Expr("<?>".to_owned()))
}

fn pop_n(stack: &mut Vec<Symbol>, count: usize) -> Vec<Symbol> {
    let missing = count.saturating_sub(stack.len());
    let mut values = vec![Symbol::Expr("<?>".to_owned()); missing];
    let split = stack.len().saturating_sub(count);
    values.extend(stack.drain(split..));
    values
}

fn field_axis(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "field_78795_f" | "rotateAngleX" | "xRot" => Some(("rotation", "x")),
        "field_78796_g" | "rotateAngleY" | "yRot" => Some(("rotation", "y")),
        "field_78808_h" | "rotateAngleZ" | "zRot" => Some(("rotation", "z")),
        "field_78800_c" | "rotationPointX" | "x" => Some(("position", "x")),
        "field_78797_d" | "rotationPointY" | "y" => Some(("position", "y")),
        "field_78798_e" | "rotationPointZ" | "z" => Some(("position", "z")),
        _ => None,
    }
}

fn is_dynamic(expression: &str) -> bool {
    ["limbSwing", "limbSwingAmount", "ageInTicks", "netHeadYaw", "headPitch"]
        .iter().any(|name| expression.contains(name))
        || ["sin(", "cos(", "abs(", "sqrt(", ".get", ".is", ".has"]
            .iter().any(|marker| expression.contains(marker))
}

impl<'a> AnimationContext<'a> {
    fn new(class: &'a ClassFile, parts: &'a [Part], scalar_fields: &'a BTreeMap<String, f64>) -> Self {
        let part_fields = parts.iter().enumerate().flat_map(|(index, part)| {
            [
                (part.name.clone(), index),
                (part.variable.clone(), index),
                (part.source_name.clone(), index),
            ]
        }).collect::<HashMap<_, _>>();
        let mut part_state = HashMap::new();
        for part in parts {
            for (axis_index, axis) in ["x", "y", "z"].iter().enumerate() {
                part_state.insert((part.name.clone(), "rotation".to_owned(), (*axis).to_owned()), clean_number(part.rotation[axis_index]));
                part_state.insert((part.name.clone(), "position".to_owned(), (*axis).to_owned()), clean_number(part.offset[axis_index]));
            }
        }
        let method_indexes = class.methods.iter().enumerate()
            .map(|(index, method)| ((method.name.clone(), method.descriptor.clone()), index))
            .collect();
        Self {
            class, parts, scalar_fields, part_fields, part_state,
            formulas: Vec::new(), warnings: Vec::new(), active_methods: Vec::new(),
            method_indexes, current_class: class.this_name().unwrap_or_default().to_owned(),
        }
    }

    fn run_method(&mut self, method_index: usize, incoming: &[Symbol], depth: usize) -> Result<()> {
        if depth > 3 {
            self.warnings.push("Animation helper recursion exceeded three levels.".to_owned());
            return Ok(());
        }
        if self.active_methods.contains(&method_index) { return Ok(()); }
        let method = self.class.methods.get(method_index).context("Invalid animation method index")?.clone();
        let Some(code) = method.code.as_ref() else { return Ok(()); };
        let descriptor = parse_method_descriptor(&method.descriptor)?;
        let is_static = method.access_flags & 0x0008 != 0;
        let mut locals = HashMap::<usize, Symbol>::new();
        if !is_static { locals.insert(0, Symbol::This); }
        let mut slot = if is_static { 0 } else { 1 };
        let names = parameter_names(&method)?;
        for (index, ty) in descriptor.parameters.iter().enumerate() {
            let value = incoming.get(index).cloned()
                .unwrap_or_else(|| Symbol::Expr(names.get(&slot).cloned().unwrap_or_else(|| format!("arg{}", index + 1))));
            locals.insert(slot, value);
            slot += ty.slots();
        }

        let instructions = decode_instructions(&code.code)?;
        self.active_methods.push(method_index);
        let result = self.interpret_instructions(&method, &instructions, &mut locals, depth);
        self.active_methods.pop();
        result
    }

    fn interpret_instructions(
        &mut self,
        method: &MemberInfo,
        instructions: &[Instruction],
        locals: &mut HashMap<usize, Symbol>,
        depth: usize,
    ) -> Result<()> {
        let mut stack = Vec::<Symbol>::new();
        for instruction in instructions {
            let op = instruction.opcode;
            if op == 0x00 { continue; }

            if let Some(index) = load_index(instruction) {
                let effective = if op == 0xc4 { instruction.operands.first().copied().unwrap_or(0) } else { op };
                if matches!(effective, 0x19 | 0x2a..=0x2d) {
                    stack.push(locals.get(&index).cloned().unwrap_or_else(|| {
                        Symbol::Expr(parameter_names(method).ok().and_then(|v| v.get(&index).cloned()).unwrap_or_else(|| format!("local{index}")))
                    }));
                    continue;
                }
                if matches!(effective, 0x15..=0x18 | 0x1a..=0x29) {
                    stack.push(locals.get(&index).cloned().unwrap_or_else(|| {
                        Symbol::Expr(parameter_names(method).ok().and_then(|v| v.get(&index).cloned()).unwrap_or_else(|| format!("local{index}")))
                    }));
                    continue;
                }
                if matches!(effective, 0x36..=0x3a | 0x3b..=0x4e) {
                    let value = pop(&mut stack);
                    let previous = locals.get(&index).map(|v| v.text(self.parts));
                    let current = value.text(self.parts);
                    if !(previous.as_deref().map(is_dynamic).unwrap_or(false) && !is_dynamic(&current)) {
                        locals.insert(index, value);
                    } else if !self.warnings.iter().any(|warning| warning.contains(&format!("local {index}")) && warning.contains("dynamic branch")) {
                        self.warnings.push(format!("Local {index} had a dynamic branch followed by a static fallback; the dynamic expression was preserved."));
                    }
                    continue;
                }
            }

            if matches!(op, 0x02..=0x14) {
                if let Some(value) = constant_expression(self.class, instruction) {
                    stack.push(Symbol::Expr(value));
                }
                continue;
            }

            match op {
                0x01 => stack.push(Symbol::Expr("null".into())),
                0x57 => { let _ = pop(&mut stack); }
                0x58 => { let _ = pop(&mut stack); let _ = pop(&mut stack); }
                0x59 => if let Some(value) = stack.last().cloned() { stack.push(value); },
                0x5a => if stack.len() >= 2 {
                    let a = pop(&mut stack); let b = pop(&mut stack);
                    stack.push(a.clone()); stack.push(b); stack.push(a);
                },
                0x5c => if stack.len() >= 2 {
                    let a = stack[stack.len() - 2].clone();
                    let b = stack[stack.len() - 1].clone();
                    stack.push(a); stack.push(b);
                } else if let Some(value) = stack.last().cloned() { stack.push(value); },
                0x5f => if stack.len() >= 2 {
                    let length = stack.len();
                    stack.swap(length - 1, length - 2);
                },
                0x99..=0x9e | 0xc6..=0xc7 => { let _ = pop(&mut stack); }
                0x9f..=0xa6 => { let _ = pop(&mut stack); let _ = pop(&mut stack); }
                0xa7 | 0xc8 | 0xaa | 0xab => {}
                0x60..=0x63 => binary(&mut stack, "+", self.parts),
                0x64..=0x67 => binary(&mut stack, "-", self.parts),
                0x68..=0x6b => binary(&mut stack, "*", self.parts),
                0x6c..=0x6f => binary(&mut stack, "/", self.parts),
                0x70..=0x73 => binary(&mut stack, "%", self.parts),
                0x74..=0x77 => { let a = pop(&mut stack); stack.push(Symbol::Expr(format!("-({})", a.text(self.parts)))); }
                0x84 => if instruction.operands.len() >= 2 {
                    let index = instruction.operands[0] as usize;
                    let delta = instruction.operands[1] as i8;
                    let current = locals.get(&index).cloned().unwrap_or_else(|| Symbol::Expr(format!("local{index}")));
                    locals.insert(index, Symbol::Expr(format!("({} + {delta})", current.text(self.parts))));
                },
                0x85..=0x93 | 0x94..=0x98 => {},
                0xb2 => {
                    let cp = read_u16(&instruction.operands, 0).unwrap_or(0);
                    let field = self.class.member_reference(cp);
                    let value = field.map(|(_, name, _)| Symbol::Expr(name)).unwrap_or_else(|| Symbol::Expr("<?>".into()));
                    stack.push(value);
                }
                0xb4 => {
                    let cp = read_u16(&instruction.operands, 0).unwrap_or(0);
                    let owner = pop(&mut stack);
                    let (_, name, _) = self.class.member_reference(cp).unwrap_or_default();
                    match owner {
                        Symbol::This if self.part_fields.contains_key(&name) => {
                            stack.push(Symbol::Part(self.part_fields[&name]));
                        }
                        Symbol::This if self.scalar_fields.contains_key(&name) => {
                            stack.push(Symbol::Expr(clean_number(self.scalar_fields[&name])));
                        }
                        Symbol::This => stack.push(Symbol::Expr(format!("this.{name}"))),
                        Symbol::Part(index) => {
                            if let Some((channel, axis)) = field_axis(&name) {
                                let part_name = self.parts.get(index).map(|part| part.name.clone()).unwrap_or_default();
                                let value = self.part_state.get(&(part_name, channel.to_owned(), axis.to_owned()))
                                    .cloned().unwrap_or_else(|| "0".to_owned());
                                stack.push(Symbol::Expr(value));
                            } else {
                                let part_name = self.parts.get(index).map(|part| part.name.clone()).unwrap_or_default();
                                stack.push(Symbol::Expr(format!("{part_name}.{name}")));
                            }
                        }
                        Symbol::Expr(value) => stack.push(Symbol::Expr(format!("{value}.{name}"))),
                    }
                }
                0xb5 => {
                    let cp = read_u16(&instruction.operands, 0).unwrap_or(0);
                    let (_, name, _) = self.class.member_reference(cp).unwrap_or_default();
                    let value = pop(&mut stack);
                    let owner = pop(&mut stack);
                    if let (Symbol::Part(index), Some((channel, axis))) = (owner, field_axis(&name)) {
                        if let Some(part) = self.parts.get(index) {
                            let expression = value.text(self.parts);
                            let key = (part.name.clone(), channel.to_owned(), axis.to_owned());
                            let previous = self.part_state.get(&key).cloned().unwrap_or_default();
                            if is_dynamic(&previous) && !is_dynamic(&expression) {
                                if !self.warnings.iter().any(|warning| warning.contains("dynamic branch fallback")) {
                                    self.warnings.push("A static branch fallback was ignored because a dynamic pose expression was already recovered.".to_owned());
                                }
                            } else {
                                self.part_state.insert(key, expression.clone());
                                self.formulas.push(Formula {
                                    bone: part.name.clone(), channel: channel.to_owned(), axis: axis.to_owned(),
                                    expression, offset: instruction.offset, animation_class: String::new(),
                                });
                            }
                        }
                    }
                }
                0xb6..=0xb9 => self.invoke(instruction, &mut stack, depth)?,
                0xba => { // invokedynamic: preserve concatenation as symbolic string
                    let cp = read_u16(&instruction.operands, 0).unwrap_or(0);
                    let desc = self.class.invoke_dynamic(cp).map(|v| v.2).unwrap_or_else(|| "()Ljava/lang/String;".into());
                    let count = parse_method_descriptor(&desc)?.parameters.len();
                    let args = pop_n(&mut stack, count);
                    stack.push(Symbol::Expr(args.iter().map(|v| v.text(self.parts)).collect::<Vec<_>>().join(" + ")));
                }
                0xc0 => {}, // checkcast
                0xac..=0xb1 | 0xbf => break,
                _ => {}
            }
        }
        Ok(())
    }

    fn invoke(&mut self, instruction: &Instruction, stack: &mut Vec<Symbol>, depth: usize) -> Result<()> {
        let cp = read_u16(&instruction.operands, 0).unwrap_or(0);
        let Some((owner, name, descriptor)) = self.class.member_reference(cp) else { return Ok(()); };
        let signature = parse_method_descriptor(&descriptor)?;
        let args = pop_n(stack, signature.parameters.len());
        let receiver = if instruction.opcode == 0xb8 { Symbol::Expr(String::new()) } else { pop(stack) };
        let simple = class_simple(&owner);

        if instruction.opcode == 0xb8 {
            if matches!(name.as_str(), "func_76126_a" | "sin") && (simple.contains("MathHelper") || simple == "Math" || simple == "Mth") {
                let arg = args.first().map(|v| v.text(self.parts)).unwrap_or_else(|| "0".into());
                stack.push(Symbol::Expr(format!("sin({arg})")));
                return Ok(());
            }
            if matches!(name.as_str(), "func_76134_b" | "cos") && (simple.contains("MathHelper") || simple == "Math" || simple == "Mth") {
                let arg = args.first().map(|v| v.text(self.parts)).unwrap_or_else(|| "0".into());
                stack.push(Symbol::Expr(format!("cos({arg})")));
                return Ok(());
            }
            if matches!(name.as_str(), "abs" | "func_76135_e")
                && (simple.contains("MathHelper") || simple == "Math")
            {
                let arg = args.first().map(|v| v.text(self.parts)).unwrap_or_else(|| "0".into());
                stack.push(Symbol::Expr(format!("abs({arg})")));
                return Ok(());
            }
            if matches!(name.as_str(), "sqrt" | "func_76133_a")
                && (simple.contains("MathHelper") || simple == "Math")
            {
                let arg = args.first().map(|v| v.text(self.parts)).unwrap_or_else(|| "0".into());
                stack.push(Symbol::Expr(format!("sqrt({arg})")));
                return Ok(());
            }
        }

        // Generated legacy models often route poses through a tiny
        // setRotateAngle(ModelRenderer, x, y, z) helper. Recover that call
        // directly instead of relying on helper bytecode being available.
        if matches!(name.as_str(), "setRotateAngle" | "setRotationAngle" | "setRotation" | "setAngles")
            && args.len() >= 4
        {
            if let Symbol::Part(index) = &args[0] {
                let index = *index;
                if let Some(part) = self.parts.get(index) {
                    let bone = part.name.clone();
                    for (axis, value) in ["x", "y", "z"].iter().zip(args.iter().skip(1).take(3)) {
                        let expression = value.text(self.parts);
                        let key = (bone.clone(), "rotation".to_owned(), (*axis).to_owned());
                        self.part_state.insert(key, expression.clone());
                        self.formulas.push(Formula {
                            bone: bone.clone(),
                            channel: "rotation".to_owned(),
                            axis: (*axis).to_owned(),
                            expression,
                            offset: instruction.offset,
                            animation_class: String::new(),
                        });
                    }
                }
                return Ok(());
            }
        }

        if name == "<init>" { return Ok(()); }

        let harmless = (simple == "ModelBase" && matches!(name.as_str(), "func_78088_a" | "render" | "func_78087_a" | "setRotationAngles"))
            || (simple == "ModelRenderer" && matches!(name.as_str(), "func_78785_a" | "render" | "func_78791_b" | "postRender"));
        if harmless { return Ok(()); }

        let owner_matches = owner == self.current_class || class_simple(&owner) == class_simple(&self.current_class);
        let returns_void = matches!(signature.return_type, JvmType::Void);
        if owner_matches && !returns_void && instruction.opcode != 0xb8 {
            // Same-class non-void helpers can return a symbolic expression, but
            // the initial animation pass only needs methods that mutate pose.
        }
        if owner_matches && receiver == Symbol::This && depth < 3 && !returns_void {
            // Helper return expressions are uncommon in model pose code. Preserve
            // exact expression and continue with the normal descriptor behavior.
        }
        let static_call = instruction.opcode == 0xb8;
        if owner_matches && depth < 3 && (static_call || receiver == Symbol::This) {
            if let Some(index) = self.method_indexes.get(&(name.clone(), descriptor.clone())).copied() {
                let method_is_static = self.class.methods.get(index)
                    .map(|method| method.access_flags & 0x0008 != 0)
                    .unwrap_or(false);
                if method_is_static == static_call {
                    self.run_method(index, &args, depth + 1)?;
                    if !returns_void { stack.push(Symbol::Expr(format!("{name}()"))); }
                    return Ok(());
                }
            }
        }

        if !returns_void {
            let argument_text = args.iter().map(|value| value.text(self.parts)).collect::<Vec<_>>().join(", ");
            let receiver_text = receiver.text(self.parts);
            stack.push(Symbol::Expr(format!("{receiver_text}.{name}({argument_text})")));
        }
        Ok(())
    }
}

fn binary(stack: &mut Vec<Symbol>, operator: &str, parts: &[Part]) {
    let right = pop(stack);
    let left = pop(stack);
    stack.push(Symbol::Expr(format!("({} {operator} {})", left.text(parts), right.text(parts))));
}

fn classify(formula: &Formula) -> String {
    let expression = formula.expression.as_str();
    let head = expression.contains("netHeadYaw") || expression.contains("headPitch");
    let walk = expression.contains("limbSwing") || expression.contains("limbSwingAmount");
    let idle = expression.contains("ageInTicks");
    if head && (walk || idle) { "combined".into() }
    else if head { "head_tracking".into() }
    else if walk && idle { "combined".into() }
    else if walk { "walk_cycle".into() }
    else if idle { "idle".into() }
    else { "static".into() }
}

fn java_expression(expression: &str) -> String {
    expression.replace("sin(", "Mth.sin(")
        .replace("cos(", "Mth.cos(")
        .replace("abs(", "Math.abs(")
        .replace("sqrt(", "Mth.sqrt(")
}

/// Reconstruct legacy ModelBase animation assignments directly from JVM Code
/// attributes. The interpreter keeps the animation as symbolic expressions,
/// tracks same-class helper calls, and does not execute game code.
pub fn extract_legacy_animation(
    class: &ClassFile,
    parts: &[Part],
    scalar_fields: &BTreeMap<String, f64>,
) -> Result<Option<LegacyAnimation>> {
    let preferred = ["func_78087_a", "setRotationAngles", "setupAnim", "func_78088_a", "render"];
    let mut candidates = Vec::<(usize, String, AnimationContext<'_>)>::new();
    for method_name in preferred {
        for (index, method) in class.methods.iter().enumerate() {
            if method.name != method_name || method.code.is_none() { continue; }
            let mut context = AnimationContext::new(class, parts, scalar_fields);
            context.run_method(index, &[], 0)?;
            candidates.push((context.formulas.len(), method.name.clone(), context));
        }
    }
    let Some((_, method_name, mut context)) = candidates.into_iter().max_by_key(|(count, _, _)| *count) else {
        return Ok(None);
    };
    if context.formulas.is_empty() { return Ok(None); }

    for formula in &mut context.formulas {
        formula.animation_class = classify(formula);
    }
    let mut by_bone = Map::new();
    let mut classification: HashMap<String, HashSet<String>> = HashMap::new();
    for formula in &context.formulas {
        let bone = formula.bone.clone();
        let mut channels = by_bone.get(&bone).and_then(JsonValue::as_object).cloned().unwrap_or_default();
        let mut expressions = channels.get(&formula.channel).and_then(JsonValue::as_array).cloned().unwrap_or_default();
        expressions.push(json!(formula.expression));
        channels.insert(formula.channel.clone(), JsonValue::Array(expressions));
        by_bone.insert(bone.clone(), JsonValue::Object(channels));
        if formula.animation_class != "static" {
            classification.entry(bone).or_default().insert(formula.animation_class.clone());
        }
    }
    let classification_json: Map<String, JsonValue> = classification.into_iter().map(|(bone, classes)| {
        let mut values: Vec<String> = classes.into_iter().collect();
        values.sort();
        (bone, if values.len() == 1 { json!(values[0]) } else { json!(values) })
    }).collect();
    let formulas_json: Vec<JsonValue> = context.formulas.iter().map(|formula| json!({
        "bone":formula.bone,"channel":formula.channel,"axis":formula.axis,
        "expression":formula.expression,"bytecode_offset":formula.offset,
        "animation_class":formula.animation_class
    })).collect();
    let parameter_mapping = if let Some(method) = class.methods.iter().find(|method| method.name == method_name) {
        let names = parameter_names(method)?;
        names.into_iter().map(|(slot, name)| (name, json!({"slot":slot}))).collect::<Map<String, JsonValue>>()
    } else { Map::new() };
    let warnings = context.warnings.clone();
    let mut analysis = json!({
        "source_method":method_name,
        "source_format":"legacy_modelbase_bytecode",
        "parameter_mapping":parameter_mapping,
        "formula_count":context.formulas.len(),
        "bones":by_bone,
        "classification":classification_json,
        "formulas":formulas_json,
        "warnings":warnings
    });
    let mut logic = String::from(
        "// Generated by MojModelBridge native Rust from recovered legacy ModelBase animation bytecode.\n\
         // Bind-pose values remain in the converted model; formulas apply at runtime.\n\n\
         public void setupAnim(Entity entity, float limbSwing, float limbSwingAmount, float ageInTicks,\n\
                               float netHeadYaw, float headPitch) {\n"
    );
    if context.formulas.iter().any(|formula| formula.expression.contains("scaleFactor")) {
        logic.push_str("    final float scaleFactor = 1.0F;\n");
    }
    let mut emitted = HashSet::<String>::new();
    for formula in &context.formulas {
        let mut expression = java_expression(&formula.expression);
        if matches!(formula.axis.as_str(), "x" | "y") {
            expression = format!("-({expression})");
        }
        let line = if formula.channel == "rotation" {
            format!("    this.{}.{}Rot = {};", formula.bone, formula.axis, expression)
        } else {
            format!("    this.{}.{} = {};", formula.bone, formula.axis, expression)
        };
        if emitted.insert(line.clone()) { logic.push_str(&line); logic.push('\n'); }
    }
    logic.push_str("}\n");

    let class_name = class.this_name()
        .map(|name| name.rsplit('/').next().unwrap_or(name).to_owned())
        .unwrap_or_default();
    let blockbench_animations =
        crate::animation::preview::bake_blockbench_animations(&class_name, parts, &analysis);
    analysis["baked_blockbench_clips"] = json!(
        blockbench_animations.iter()
            .filter_map(|clip| clip["name"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    );

    Ok(Some(LegacyAnimation { analysis, logic, source: method_name, blockbench_animations }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_legacy_modelrenderer_position_fields_to_correct_axes() {
        assert_eq!(field_axis("field_78800_c"), Some(("position", "x")));
        assert_eq!(field_axis("field_78797_d"), Some(("position", "y")));
        assert_eq!(field_axis("field_78798_e"), Some(("position", "z")));
    }

    #[test]
    fn classifies_animation_formula_channels() {
        let f = Formula { bone:"wing".into(), channel:"rotation".into(), axis:"x".into(),
            expression:"Mth.sin(ageInTicks * 0.5F) + limbSwing".into(), offset:23, animation_class:String::new() };
        assert_eq!(classify(&f), "combined");
    }

    #[test]
    fn java_formula_names_are_translated_for_target_logic() {
        assert_eq!(java_expression("sin(ageInTicks) + cos(limbSwing)"), "Mth.sin(ageInTicks) + Mth.cos(limbSwing)");
    }
}
