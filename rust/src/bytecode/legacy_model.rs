use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::bytecode::classfile::{ClassFile, ConstantPoolEntry};
use crate::bytecode::{decode_instructions, parse_method_descriptor, ConstantValue, Instruction, JvmType};
use crate::model::{Cube, ParsedModel, Part};

#[derive(Debug, Clone)]
struct LegacyPart {
    name: String,
    texture_u: i32,
    texture_v: i32,
    offset: [f64; 3],
    rotation: [f64; 3],
    mirror: bool,
    texture_prefix: String,
    cubes: Vec<Cube>,
    parent: Option<String>,
    source_index: usize,
}

impl LegacyPart {
    fn new(name: String, source_index: usize) -> Self {
        Self {
            name, texture_u: 0, texture_v: 0, offset: [0.0; 3],
            rotation: [0.0; 3], mirror: false, texture_prefix: String::new(),
            cubes: Vec::new(), parent: None, source_index,
        }
    }
}

#[derive(Debug, Clone)]
struct CubeData {
    x: f64, y: f64, z: f64,
    dx: f64, dy: f64, dz: f64,
    texture_u: i32, texture_v: i32,
    inflate: f64, mirror: bool,
    owner: Option<usize>,
}

impl Default for CubeData {
    fn default() -> Self {
        Self { x:0.0, y:0.0, z:0.0, dx:0.0, dy:0.0, dz:0.0, texture_u:0, texture_v:0, inflate:0.0, mirror:false, owner:None }
    }
}

#[derive(Debug, Clone)]
enum Value {
    This,
    Number(f64),
    Boolean(bool),
    String(String),
    Object(usize),
    CubeList(usize),
    Cube(usize),
    Opaque(String),
    Null,
}

impl Value {
    fn number(&self) -> Option<f64> {
        match self { Self::Number(v) => Some(*v), Self::Boolean(v) => Some(if *v {1.0} else {0.0}), _ => None }
    }
    fn boolean(&self) -> Option<bool> {
        match self { Self::Boolean(v) => Some(*v), Self::Number(v) => Some(*v != 0.0), _ => None }
    }
}

#[derive(Debug, Clone)]
struct ConstructorResult {
    parts: Vec<LegacyPart>,
    texture_width: u32,
    texture_height: u32,
    scalar_fields: HashMap<String, f64>,
    warnings: Vec<String>,
    cube_count: usize,
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(offset)?, *bytes.get(offset+1)?]))
}

fn local_index(instruction: &Instruction) -> Option<usize> {
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
        0xc4 if instruction.operands.len() >= 3 => u16_at(&instruction.operands, 1).map(|v| v as usize),
        _ => None,
    }
}

fn push_constant(class: &ClassFile, instruction: &Instruction) -> Option<Value> {
    let op = instruction.opcode;
    let value = match op {
        0x02 => -1.0,
        0x03..=0x08 => (op - 0x03) as f64,
        0x09..=0x0a => (op - 0x09) as f64,
        0x0b..=0x0d => (op - 0x0b) as f64,
        0x0e..=0x0f => (op - 0x0e) as f64,
        0x10 => *instruction.operands.first()? as i8 as f64,
        0x11 => i16::from_be_bytes(instruction.operands.get(0..2)?.try_into().ok()?) as f64,
        _ => {
            let index = match op {
                0x12 => *instruction.operands.first()? as u16,
                0x13 | 0x14 => u16_at(&instruction.operands, 0)?,
                _ => return None,
            };
            return match class.constant(index)? {
                ConstantValue::Integer(v) => Some(Value::Number(v as f64)),
                ConstantValue::Float(v) => Some(Value::Number(v as f64)),
                ConstantValue::Long(v) => Some(Value::Number(v as f64)),
                ConstantValue::Double(v) => Some(Value::Number(v)),
                ConstantValue::String(v) => Some(Value::String(v)),
                ConstantValue::Class(v) => Some(Value::Opaque(v)),
            };
        }
    };
    Some(Value::Number(value))
}

fn pop(stack: &mut Vec<Value>) -> Value { stack.pop().unwrap_or(Value::Null) }

fn pop_n(stack: &mut Vec<Value>, count: usize) -> Vec<Value> {
    let missing = count.saturating_sub(stack.len());
    let mut values = vec![Value::Null; missing];
    let split = stack.len().saturating_sub(count);
    values.extend(stack.drain(split..));
    values
}

fn descriptor_slots(descriptor: &str) -> Result<Vec<JvmType>> {
    Ok(parse_method_descriptor(descriptor)?.parameters)
}

fn apply_binary(op: u8, stack: &mut Vec<Value>) {
    let right = pop(stack);
    let left = pop(stack);
    let result = match (left.number(), right.number()) {
        (Some(a), Some(b)) => {
            let value = match op {
                0x60..=0x63 => a + b,
                0x64..=0x67 => a - b,
                0x68..=0x6b => a * b,
                0x6c..=0x6f => if b.abs() > f64::EPSILON {a / b} else {0.0},
                0x70..=0x73 => if b.abs() > f64::EPSILON {a % b} else {0.0},
                _ => 0.0,
            };
            Value::Number(value)
        }
        _ => Value::Number(0.0),
    };
    stack.push(result);
}

fn constant_class_name(class: &ClassFile, index: u16) -> Option<String> {
    match class.constant_pool.get(index as usize)?.as_ref()? {
        ConstantPoolEntry::Class { name_index } => class.utf8(*name_index).map(str::to_owned),
        _ => None,
    }
}

fn run_constructor(
    class: &ClassFile,
    method: &crate::bytecode::MemberInfo,
    supplied_args: &[serde_json::Value],
) -> Result<ConstructorResult> {
    let code = method.code.as_ref().context("Constructor has no Code attribute")?;
    let instructions = decode_instructions(&code.code)?;
    let descriptor = parse_method_descriptor(&method.descriptor)?;
    let mut locals = HashMap::<usize, Value>::new();
    locals.insert(0, Value::This);

    let mut slot = 1usize;
    for (index, parameter) in descriptor.parameters.iter().enumerate() {
        let supplied = supplied_args.get(index);
        let value = if let Some(value) = supplied {
            if let Some(number) = value.as_f64().or_else(|| value.as_i64().map(|v| v as f64)) {
                Value::Number(number)
            } else if let Some(boolean) = value.as_bool() {
                Value::Boolean(boolean)
            } else if let Some(string) = value.as_str() {
                Value::String(string.to_owned())
            } else {
                Value::Opaque(format!("constructor_arg_{index}"))
            }
        } else {
            match parameter {
                JvmType::Boolean => Value::Boolean(false),
                JvmType::Byte | JvmType::Char | JvmType::Int | JvmType::Long | JvmType::Float | JvmType::Double | JvmType::Short => Value::Number(0.0),
                JvmType::Object(name) => Value::Opaque(name.clone()),
                JvmType::Array(_) => Value::Opaque(format!("array_arg_{index}")),
                JvmType::Void => Value::Null,
            }
        };
        locals.insert(slot, value);
        slot += parameter.slots();
    }

    let mut stack = Vec::<Value>::new();
    let mut parts = Vec::<LegacyPart>::new();
    let mut field_objects = HashMap::<String, usize>::new();
    let mut cube_objects = Vec::<CubeData>::new();
    let mut texture_offsets = HashMap::<String, (i32, i32)>::new();
    let mut texture_width = 64u32;
    let mut texture_height = 32u32;
    let mut explicit_width = false;
    let mut explicit_height = false;
    let mut scalar_fields = HashMap::<String, f64>::new();
    let mut warnings = Vec::<String>::new();

    for instruction in &instructions {
        let op = instruction.opcode;

        // Local loads.
        if let Some(index) = local_index(instruction) {
            if matches!(op, 0x15..=0x19 | 0x1a..=0x2d) {
                stack.push(locals.get(&index).cloned().unwrap_or(Value::Opaque(format!("local{index}"))));
                continue;
            }
            if matches!(op, 0x36..=0x3a | 0x3b..=0x4e) {
                locals.insert(index, pop(&mut stack));
                continue;
            }
        }

        if let Some(value) = push_constant(class, instruction) {
            if matches!(op, 0x02..=0x14) {
                stack.push(value);
                continue;
            }
        }

        match op {
            0xbb => {
                let index = u16_at(&instruction.operands, 0).unwrap_or(0);
                let name = constant_class_name(class, index).unwrap_or_default();
                if name.ends_with("ModelRenderer") || name.ends_with("ModelBox") {
                    if name.ends_with("ModelRenderer") {
                        let part_index = parts.len();
                        parts.push(LegacyPart::new(format!("__tmp_{part_index}"), part_index));
                        stack.push(Value::Object(part_index));
                    } else {
                        let cube_index = cube_objects.len();
                        cube_objects.push(CubeData::default());
                        stack.push(Value::Cube(cube_index));
                    }
                } else {
                    stack.push(Value::Opaque(name));
                }
            }
            0x59 => {
                if let Some(value) = stack.last().cloned() { stack.push(value); }
            }
            0x5a => { // dup_x1
                if stack.len() >= 2 {
                    let a = pop(&mut stack); let b = pop(&mut stack);
                    stack.push(a.clone()); stack.push(b); stack.push(a);
                }
            }
            0x5c => { // dup2 (common for category-2 numeric or two refs)
                if stack.len() >= 2 {
                    let a = stack[stack.len()-2].clone();
                    let b = stack[stack.len()-1].clone();
                    stack.push(a); stack.push(b);
                } else if let Some(value) = stack.last().cloned() { stack.push(value); }
            }
            0x57 => { let _ = pop(&mut stack); }
            0x58 => { let _ = pop(&mut stack); let _ = pop(&mut stack); }
            0x60..=0x73 => apply_binary(op, &mut stack),
            0x74..=0x77 => {
                let value = pop(&mut stack);
                stack.push(Value::Number(-value.number().unwrap_or(0.0)));
            }
            0x84 => {
                if instruction.operands.len() >= 2 {
                    let index = instruction.operands[0] as usize;
                    let delta = instruction.operands[1] as i8 as f64;
                    let current = locals.get(&index).and_then(Value::number).unwrap_or(0.0);
                    locals.insert(index, Value::Number(current + delta));
                }
            }
            0x85..=0x93 | 0x94..=0x98 => { /* numeric conversions/comparisons retain no model data */ }
            0xb2 | 0xb4 => {
                let cp_index = u16_at(&instruction.operands, 0).unwrap_or(0);
                let reference = class.member_reference(cp_index);
                let owner = pop(&mut stack);
                if let Some((_, name, _)) = reference {
                    if let Value::Object(part_index) = owner {
                        if name == "field_78804_l" || name == "cubeList" {
                            stack.push(Value::CubeList(part_index));
                        } else if let Some(part) = parts.get(part_index) {
                            let value = match name.as_str() {
                                "field_78795_f" | "rotateAngleX" => Value::Number(part.rotation[0]),
                                "field_78796_g" | "rotateAngleY" => Value::Number(part.rotation[1]),
                                "field_78808_h" | "rotateAngleZ" => Value::Number(part.rotation[2]),
                                "field_78798_e" | "rotationPointX" => Value::Number(part.offset[0]),
                                "field_78797_d" | "rotationPointY" => Value::Number(part.offset[1]),
                                "field_78800_c" | "rotationPointZ" => Value::Number(part.offset[2]),
                                _ => Value::Opaque(name),
                            };
                            stack.push(value);
                        } else {
                            stack.push(Value::Opaque(name));
                        }
                    } else if matches!(owner, Value::This) && name == "field_78090_t" {
                        stack.push(Value::Opaque("__texture_width".into()));
                    } else if matches!(owner, Value::This) && name == "field_78089_u" {
                        stack.push(Value::Opaque("__texture_height".into()));
                    } else {
                        stack.push(Value::Opaque(name));
                    }
                }
            }
            0xb3 => { let _ = pop(&mut stack); }
            0xb5 => {
                let cp_index = u16_at(&instruction.operands, 0).unwrap_or(0);
                let reference = class.member_reference(cp_index);
                let value = pop(&mut stack);
                let owner = pop(&mut stack);
                if let Some((_, name, _)) = reference {
                    match value {
                        Value::Object(index) => {
                            if let Some(part) = parts.get_mut(index) {
                                part.name = name.clone();
                            }
                            field_objects.insert(name, index);
                        }
                        Value::Number(number) if matches!(owner, Value::This) => {
                            if name == "field_78090_t" || name == "textureWidth" {
                                texture_width = (number as i64).max(1) as u32;
                                explicit_width = true;
                            } else if name == "field_78089_u" || name == "textureHeight" {
                                texture_height = (number as i64).max(1) as u32;
                                explicit_height = true;
                            } else {
                                scalar_fields.insert(name, number);
                            }
                        }
                        Value::Boolean(value) => {
                            if let Value::Object(index) = owner {
                                if let Some(part) = parts.get_mut(index) {
                                    if name == "field_78809_i" || name == "mirror" { part.mirror = value; }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            0xb6 | 0xb7 | 0xb8 | 0xb9 => {
                let cp_index = u16_at(&instruction.operands, 0).unwrap_or(0);
                let reference = class.member_reference(cp_index);
                let Some((owner_name, name, desc)) = reference else {
                    continue;
                };
                let parameters = descriptor_slots(&desc)?;
                let args = pop_n(&mut stack, parameters.len());
                let receiver = if op == 0xb8 { Value::Null } else { pop(&mut stack) };

                if name == "<init>" && owner_name.ends_with("ModelRenderer") {
                    if let Value::Object(index) = receiver {
                        let numeric: Vec<f64> = args.iter().filter_map(Value::number).collect();
                        if numeric.len() >= 2 {
                            if let Some(part) = parts.get_mut(index) {
                                part.texture_u = numeric[numeric.len()-2] as i32;
                                part.texture_v = numeric[numeric.len()-1] as i32;
                            }
                        }
                        // A ModelBase;String constructor carries a texture prefix.
                        if let Some(Value::String(prefix)) = args.last() {
                            if let Some(part) = parts.get_mut(index) { part.texture_prefix = prefix.clone(); }
                        }
                    }
                    continue;
                }

                if name == "<init>" && owner_name.ends_with("ModelBox") {
                    if let Value::Cube(cube_index) = receiver {
                        let numbers: Vec<f64> = args.iter().filter_map(Value::number).collect();
                        // Arguments: renderer, u, v, x, y, z, dx, dy, dz, delta, mirror.
                        if numbers.len() >= 8 {
                            let mut cube = CubeData::default();
                            let start = if args.first().map(|v| matches!(v, Value::Object(_))).unwrap_or(false) { 1 } else { 0 };
                            let tail: Vec<Value> = args.iter().skip(start).cloned().collect();
                            if tail.len() >= 8 {
                                let values: Vec<f64> = tail.iter().map(|v| v.number().unwrap_or(0.0)).collect();
                                cube.texture_u = values[0] as i32;
                                cube.texture_v = values[1] as i32;
                                cube.x=values[2]; cube.y=values[3]; cube.z=values[4];
                                cube.dx=values[5]; cube.dy=values[6]; cube.dz=values[7];
                                if values.len() > 8 { cube.inflate=values[8]; }
                                cube.mirror=tail.get(9).and_then(Value::boolean).unwrap_or(false);
                                cube.owner = args.first().and_then(|v| if let Value::Object(i)=v {Some(*i)} else {None});
                                cube_objects[cube_index] = cube;
                            }
                        }
                    }
                    continue;
                }

                if name == "setTextureSize" || name == "func_78787_b" {
                    if let (Some(width),Some(height))=(args.first().and_then(Value::number),args.get(1).and_then(Value::number)) {
                        if !explicit_width { texture_width=texture_width.max(width as u32); }
                        if !explicit_height { texture_height=texture_height.max(height as u32); }
                    }
                    continue;
                }

                if name == "setTextureOffset" || name == "func_78784_a" {
                    if let (Value::Object(index),Some(u),Some(v))=(receiver.clone(),args.first().and_then(Value::number),args.get(1).and_then(Value::number)) {
                        if let Some(part)=parts.get_mut(index) {part.texture_u=u as i32;part.texture_v=v as i32;}
                        stack.push(receiver);
                    } else if let (Some(Value::String(key)),Some(u),Some(v))=(args.first(),args.get(1).and_then(Value::number),args.get(2).and_then(Value::number)) {
                        texture_offsets.insert(key.clone(),(u as i32,v as i32));
                    }
                    continue;
                }

                if name == "setRotationPoint" || name == "func_78793_a" {
                    if let Value::Object(index)=receiver {
                        let numbers:Vec<f64>=args.iter().filter_map(Value::number).collect();
                        if numbers.len()>=3 { if let Some(part)=parts.get_mut(index) {part.offset=[numbers[0],numbers[1],numbers[2]];} }
                    }
                    continue;
                }

                if name == "setRotateAngle" || name == "setRotationAngle" || name == "setRotation" || name == "setAngles" {
                    if let Value::Object(index)=args.first().cloned().unwrap_or(Value::Null) {
                        let numbers:Vec<f64>=args.iter().skip(1).filter_map(Value::number).collect();
                        if numbers.len()>=3 { if let Some(part)=parts.get_mut(index) {part.rotation=[numbers[0],numbers[1],numbers[2]];} }
                    }
                    continue;
                }

                if name == "addChild" || name == "func_78792_a" {
                    if let (Value::Object(parent),Some(Value::Object(child)))=(receiver,args.first()) {
                        if let Some(child_part)=parts.get_mut(*child) {
                            child_part.parent=parts.get(parent).map(|p|p.name.clone());
                        }
                    }
                    continue;
                }

                if name == "addBox" || name == "func_78789_a" || name == "func_78790_a" || name == "func_78786_a" {
                    if let Value::Object(index)=receiver {
                        let has_name=parameters.first().map(|p| matches!(p,JvmType::Object(name) if name=="java/lang/String")).unwrap_or(false);
                        let raw_name=if has_name { args.first().and_then(|v| if let Value::String(s)=v {Some(s.clone())} else {None}) } else {None};
                        let numbers:Vec<f64>=args.iter().skip(usize::from(has_name)).filter_map(Value::number).collect();
                        if numbers.len()>=6 {
                            if let Some(part)=parts.get_mut(index) {
                                part.cubes.push(Cube {
                                    x:numbers[0],y:numbers[1],z:numbers[2],dx:numbers[3],dy:numbers[4],dz:numbers[5],
                                    uv_x:part.texture_u,uv_y:part.texture_v,mirror:part.mirror,
                                    inflate:numbers.get(6).copied().unwrap_or(0.0),
                                    name:raw_name.unwrap_or_else(|| format!("{}_cube_{}",part.name,part.cubes.len()+1)),
                                });
                            }
                        }
                    }
                    continue;
                }

                if name == "add" && (owner_name.contains("List") || owner_name.contains("Collection")) {
                    if let (Some(Value::CubeList(owner)),Some(Value::Cube(cube)))=(Some(receiver),args.first()) {
                        if let (Some(part),Some(cube))=(parts.get_mut(owner),cube_objects.get(*cube)) {
                            let name=format!("{}_cube_{}",part.name,part.cubes.len()+1);
                            part.cubes.push(Cube {
                                x:cube.x,y:cube.y,z:cube.z,dx:cube.dx,dy:cube.dy,dz:cube.dz,
                                uv_x:cube.texture_u,uv_y:cube.texture_v,mirror:cube.mirror,inflate:cube.inflate,name,
                            });
                        }
                    }
                    continue;
                }

                // Consume unsupported method calls using descriptor arity, just
                // as Beta 12 did, so stale stack values cannot skew later ops.
                if !parameters.is_empty() || op != 0xb8 {
                    let _ = args;
                }
                if desc.ends_with(")V") {
                    // void result: nothing to push
                } else if owner_name.starts_with("java/lang/Math") && (name == "sin" || name == "cos") {
                    let arg = args.first().and_then(Value::number).unwrap_or(0.0);
                    stack.push(Value::Number(if name=="sin" {arg.sin()} else {arg.cos()}));
                } else {
                    stack.push(Value::Opaque(format!("{owner_name}.{name}")));
                }
            }
            0xc0 => { /* checkcast leaves the reference on the stack */ }
            0xbe => { let _ = pop(&mut stack); stack.push(Value::Number(0.0)); }
            0xb0..=0xb1 | 0xbf => break,
            _ => {}
        }
    }

    // Apply CubeList.add(ModelBox) objects after bytecode interpreted their constructors.
    // Any direct list-add above attached them at that exact call position.
    let mut ordered = parts;
    for (index, part) in ordered.iter_mut().enumerate() { part.source_index=index; }
    let mut seen = HashSet::new();
    ordered.retain(|part| {
        if !seen.insert(part.name.clone()) { return false; }
        !part.cubes.is_empty() || part.parent.is_some()
    });
    let cube_count = ordered.iter().map(|part|part.cubes.len()).sum();
    Ok(ConstructorResult {
        parts: ordered, texture_width, texture_height, scalar_fields, warnings, cube_count,
    })
}

/// Reconstruct the legacy ModelBase / ModelRenderer geometry directly from JVM
/// bytes. This replaces the old javap subprocess for 1.7.10–1.12.2 class files.
pub fn parse_legacy_class_file(path: &Path, constructor_args: &[serde_json::Value]) -> Result<ParsedModel> {
    let bytes=fs::read(path).with_context(||format!("Reading compiled model {}",path.display()))?;
    let class=ClassFile::parse(&bytes)?;
    let class_name=class.this_name().map(|v|v.rsplit('/').next().unwrap_or(v).to_owned())
        .unwrap_or_else(||path.file_stem().and_then(|v|v.to_str()).unwrap_or("LegacyModel").to_owned());
    let super_name=class.super_name().unwrap_or_default();
    let has_renderer_fields=class.fields.iter().any(|field| field.descriptor.ends_with("ModelRenderer;") || field.descriptor.contains("ModelRenderer"));
    if !(super_name.ends_with("ModelBase") || super_name.ends_with("ModelBiped") || has_renderer_fields) {
        bail!("{} does not look like a legacy ModelBase/ModelRenderer model.",path.display());
    }

    let constructors:Vec<_>=class.methods.iter().filter(|method|method.name=="<init>").collect();
    if constructors.is_empty() { bail!("No JVM constructor was found in {}.",path.display()); }
    let mut candidates=Vec::new();
    for method in constructors {
        match run_constructor(&class,method,constructor_args) {
            Ok(result)=>candidates.push((result,method.descriptor.clone())),
            Err(error)=>candidates.push((ConstructorResult {
                parts:Vec::new(),texture_width:64,texture_height:32,scalar_fields:HashMap::new(),
                warnings:vec![format!("Constructor {} failed: {error}",method.descriptor)],cube_count:0
            },method.descriptor.clone()))
        }
    }
    candidates.sort_by(|(a,desc_a),(b,desc_b)| b.cube_count.cmp(&a.cube_count)
        .then_with(||b.parts.len().cmp(&a.parts.len()))
        .then_with(||desc_a.len().cmp(&desc_b.len())));
    let (best,descriptor)=candidates.into_iter().next().unwrap();
    if best.parts.is_empty() || best.cube_count==0 {
        bail!("No ModelRenderer cubes were recovered from {}. Constructor tested: {descriptor}.",path.display());
    }
    let mut parts=Vec::<Part>::new();
    let names:HashMap<String,String>=best.parts.iter().map(|p|(p.name.clone(),p.parent.clone().unwrap_or_default())).collect();
    for (index,legacy) in best.parts.iter().enumerate() {
        let mut part=Part {
            name:legacy.name.clone(),variable:legacy.name.clone(),
            parent_variable:legacy.parent.clone().unwrap_or_else(||"root".into()),
            offset:legacy.offset,rotation:legacy.rotation,cubes:legacy.cubes.clone(),
            parent:legacy.parent.clone(),children:Vec::new(),source_index:index,source_name:legacy.name.clone(),
            render_instance_index:0,render_instance_spec:serde_json::Value::Null,
            origin:[0.0,24.0,0.0],group_uuid:String::new(),
        };
        part.children=names.iter().filter(|(_,parent)|*parent==&legacy.name).map(|(name,_)|name.clone()).collect();
        parts.push(part);
    }

    let mut model=ParsedModel::new(class_name,best.texture_width,best.texture_height,parts,best.warnings);
    model.source_format=if class.major_version<=50 {
        "minecraft_1.7_1.8_modelbase_class".into()
    } else if class.major_version<=52 {
        "minecraft_1.9_1.12_modelbase_class".into()
    } else {
        "legacy_modelbase_class".into()
    };
    model.constructor_args=constructor_args.to_vec();
    model.scalar_fields=best.scalar_fields.into_iter().collect();
    model.animation_analysis=serde_json::json!({
        "source_format":model.source_format,
        "mode":"geometry_recovered",
        "animation_port_status":"pending",
        "note":"Model geometry was reconstructed by the native class-file interpreter. Procedural JVM animation interpretation and keyframe baking remain separate stages."
    });
    Ok(model)
}
