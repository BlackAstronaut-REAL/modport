use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::bytecode::classfile::{ClassFile, ConstantPoolEntry, ConstantValue, MemberInfo};
use crate::bytecode::{decode_instructions, parse_method_descriptor, Instruction, JvmType};
use crate::model::{Cube, ParsedModel, Part};

#[derive(Debug, Clone)]
enum Value {
    Number(f64),
    Boolean(bool),
    String(String),
    Handle(usize),
    Opaque(String),
    Null,
}

impl Value {
    fn number(&self) -> Option<f64> {
        match self { Self::Number(v)=>Some(*v), Self::Boolean(v)=>Some(if *v {1.0}else{0.0}), _=>None }
    }
    fn boolean(&self) -> Option<bool> {
        match self { Self::Boolean(v)=>Some(*v), Self::Number(v)=>Some(*v!=0.0), _=>None }
    }
    fn string(&self) -> Option<&str> { if let Self::String(v)=self {Some(v)} else {None} }
    fn display(&self) -> String {
        match self {
            Self::Number(v)=>format!("{v}"), Self::Boolean(v)=>v.to_string(),
            Self::String(v)=>v.clone(), Self::Opaque(v)=>v.clone(),
            Self::Handle(v)=>format!("object_{v}"), Self::Null=>"null".into()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectKind { Mesh, Builder, Deformation, Pose, Part, Layer, Other }

#[derive(Debug, Clone)]
struct Object {
    kind: ObjectKind,
    name: String,
    parent: Option<usize>,
    part_index: Option<usize>,
    cubes: Vec<Cube>,
    uv_x: i32,
    uv_y: i32,
    mirror: bool,
    inflate: f64,
    pose: [f64;6],
}

impl Object {
    fn new(kind:ObjectKind)->Self {
        Self {kind,name:String::new(),parent:None,part_index:None,cubes:Vec::new(),uv_x:0,uv_y:0,mirror:false,inflate:0.0,pose:[0.0;6]}
    }
}

#[derive(Debug)]
struct Interpreter<'a> {
    class: &'a ClassFile,
    class_name: String,
    methods: HashMap<String, Vec<usize>>,
    objects: Vec<Object>,
    parts: Vec<Part>,
    root: Option<usize>,
    texture_width: u32,
    texture_height: u32,
    warnings: Vec<String>,
    call_stack: Vec<usize>,
}

fn local_index(instruction:&Instruction)->Option<usize>{
    let op=instruction.opcode;
    let implicit=match op {
        0x1a..=0x1d=>Some((op-0x1a) as usize),0x1e..=0x21=>Some((op-0x1e) as usize),
        0x22..=0x25=>Some((op-0x22) as usize),0x26..=0x29=>Some((op-0x26) as usize),
        0x2a..=0x2d=>Some((op-0x2a) as usize),0x3b..=0x3e=>Some((op-0x3b) as usize),
        0x3f..=0x42=>Some((op-0x3f) as usize),0x43..=0x46=>Some((op-0x43) as usize),
        0x47..=0x4a=>Some((op-0x47) as usize),0x4b..=0x4e=>Some((op-0x4b) as usize),_=>None
    };
    if implicit.is_some(){return implicit;}
    match op {
        0x15..=0x19|0x36..=0x3a|0xa9=>instruction.operands.first().map(|v|*v as usize),
        0xc4 if instruction.operands.len()>=3=>Some(u16::from_be_bytes([instruction.operands[1],instruction.operands[2]]) as usize),
        _=>None
    }
}

fn cp_class_name(class:&ClassFile,index:u16)->Option<String>{
    match class.constant_pool.get(index as usize)?.as_ref()? {
        ConstantPoolEntry::Class{name_index}=>class.utf8(*name_index).map(str::to_owned),_=>None
    }
}

fn constant_value(class:&ClassFile,op:u8,operands:&[u8])->Option<Value>{
    let numeric=match op {
        0x02=>Some(-1.0),0x03..=0x08=>Some((op-0x03) as f64),
        0x09..=0x0a=>Some((op-0x09) as f64),
        0x0b..=0x0d=>Some((op-0x0b) as f64),0x0e..=0x0f=>Some((op-0x0e) as f64),
        0x10=>Some(*operands.first()? as i8 as f64),
        0x11=>Some(i16::from_be_bytes(operands.get(0..2)?.try_into().ok()?) as f64),_=>None
    };
    if let Some(value)=numeric{return Some(Value::Number(value));}
    let index=match op {
        0x12=>*operands.first()? as u16,
        0x13|0x14=>u16::from_be_bytes(operands.get(0..2)?.try_into().ok()?),_=>return None
    };
    match class.constant(index)? {
        ConstantValue::Integer(v)=>Some(Value::Number(v as f64)),
        ConstantValue::Float(v)=>Some(Value::Number(v as f64)),
        ConstantValue::Long(v)=>Some(Value::Number(v as f64)),
        ConstantValue::Double(v)=>Some(Value::Number(v)),
        ConstantValue::String(v)=>Some(Value::String(v)),
        ConstantValue::Class(v)=>Some(Value::Opaque(v)),
    }
}

fn pop(stack:&mut Vec<Value>)->Value{stack.pop().unwrap_or(Value::Null)}
fn pop_n(stack:&mut Vec<Value>,count:usize)->Vec<Value>{
    let missing=count.saturating_sub(stack.len());
    let mut values=vec![Value::Null;missing];
    let split=stack.len().saturating_sub(count);
    values.extend(stack.drain(split..));
    values
}
fn class_method_static(method:&MemberInfo)->bool{method.access_flags & 0x0008 != 0}
fn return_is_void(desc:&str)->bool{desc.split(')').nth(1).map(|v|v=="V").unwrap_or(true)}
fn simple_name(name:&str)->&str{name.rsplit('/').next().unwrap_or(name).rsplit('$').next().unwrap_or(name)}

impl<'a> Interpreter<'a>{
    fn new(class:&'a ClassFile)->Self{
        let mut methods:HashMap<String,Vec<usize>>=HashMap::new();
        for (i,method) in class.methods.iter().enumerate(){
            methods.entry(method.name.clone()).or_default().push(i);
        }
        Self{
            class,class_name:class.this_name().map(|n|simple_name(n).to_owned()).unwrap_or_else(||"Model".into()),
            methods,objects:Vec::new(),parts:Vec::new(),root:None,texture_width:64,texture_height:32,
            warnings:Vec::new(),call_stack:Vec::new()
        }
    }
    fn new_object(&mut self,kind:ObjectKind)->Value{
        let id=self.objects.len();self.objects.push(Object::new(kind));Value::Handle(id)
    }
    fn object(&self,value:&Value)->Option<&Object>{
        if let Value::Handle(id)=value{self.objects.get(*id)}else{None}
    }
    fn object_mut(&mut self,value:&Value)->Option<&mut Object>{
        if let Value::Handle(id)=value{self.objects.get_mut(*id)}else{None}
    }
    fn invoke_helper(&mut self,method_index:usize,args:&[Value])->Result<Option<Value>>{
        if self.call_stack.contains(&method_index){return Ok(None);}
        self.call_stack.push(method_index);
        let method=self.class.methods[method_index].clone();
        let result=self.execute_method(&method,args);
        self.call_stack.pop();
        result
    }
    fn default_args(&mut self,method:&MemberInfo)->Result<Vec<Value>>{
        let descriptor=parse_method_descriptor(&method.descriptor)?;
        let mut args=Vec::new();
        for ty in descriptor.parameters {
            let value=match ty{
                JvmType::Object(ref name) if simple_name(name)=="CubeDeformation"=>self.new_object(ObjectKind::Deformation),
                JvmType::Object(ref name) if simple_name(name)=="MeshDefinition"=>self.new_object(ObjectKind::Mesh),
                JvmType::Object(ref name) if simple_name(name)=="CubeListBuilder"=>self.new_object(ObjectKind::Builder),
                JvmType::Object(ref name) if simple_name(name)=="PartPose"=>self.new_object(ObjectKind::Pose),
                JvmType::Object(ref name) if simple_name(name)=="PartDefinition"=>{
                    let root=self.ensure_root();Value::Handle(root)
                }
                JvmType::Float|JvmType::Double|JvmType::Byte|JvmType::Char|JvmType::Int|JvmType::Long|JvmType::Short=>Value::Number(0.0),
                JvmType::Boolean=>Value::Boolean(false),
                JvmType::Array(_)|JvmType::Object(_)=>Value::Null,
                JvmType::Void=>Value::Null,
            };
            args.push(value);
        }
        Ok(args)
    }
    fn ensure_root(&mut self)->usize{
        if let Some(root)=self.root{return root;}
        let id=self.objects.len();
        let mut root=Object::new(ObjectKind::Part);root.name="root".into();
        self.objects.push(root);self.root=Some(id);id
    }
    fn push_child(&mut self,parent_id:usize,name:String,builder:Option<usize>,pose:Option<usize>)->usize{
        let parent_name=self.objects.get(parent_id).map(|v|v.name.clone()).unwrap_or_else(||"root".into());
        let parent_part=self.objects.get(parent_id).and_then(|v|v.part_index);
        let pose_values=pose.and_then(|v|self.objects.get(v).map(|o|o.pose)).unwrap_or([0.0;6]);
        let cubes=builder.and_then(|v|self.objects.get(v).map(|o|o.cubes.clone())).unwrap_or_default();
        let part=Part{
            name:name.clone(),variable:name.clone(),parent_variable:if parent_name.is_empty(){"root".into()}else{parent_name.clone()},
            offset:[pose_values[0],pose_values[1],pose_values[2]],rotation:[pose_values[3],pose_values[4],pose_values[5]],
            cubes,parent:parent_part.map(|index|self.parts[index].name.clone()),children:Vec::new(),
            source_index:self.parts.len(),source_name:name.clone(),render_instance_index:0,
            render_instance_spec:serde_json::Value::Null,origin:[0.0,24.0,0.0],group_uuid:String::new()
        };
        let part_index=self.parts.len();self.parts.push(part);
        if let Some(parent_part)=parent_part {
            if !self.parts[parent_part].children.contains(&name){self.parts[parent_part].children.push(name.clone());}
        }
        let mut object=Object::new(ObjectKind::Part);object.name=name;object.parent=Some(parent_id);object.part_index=Some(part_index);
        let id=self.objects.len();self.objects.push(object);id
    }
    fn concat_dynamic(&self,cp_index:u16,args:&[Value])->String{
        let Some((bootstrap_index,_,_))=self.class.invoke_dynamic(cp_index) else {
            return format!("part_{}",args.first().map(Value::display).unwrap_or_else(||"0".into()));
        };
        let Some(bootstrap)=self.class.bootstrap_methods.get(bootstrap_index as usize) else {
            return format!("part_{}",args.first().map(Value::display).unwrap_or_else(||"0".into()));
        };
        let handle_ref=match self.class.constant_pool.get(bootstrap.method_ref as usize).and_then(|x|x.as_ref()){
            Some(ConstantPoolEntry::MethodHandle{reference_index,..})=>*reference_index,_=>0
        };
        let is_concat=self.class.member_reference(handle_ref)
            .map(|(_,name,_)|name=="makeConcatWithConstants"||name=="makeConcat").unwrap_or(false);
        if !is_concat{return format!("part_{}",args.first().map(Value::display).unwrap_or_else(||"0".into()));}
        let recipe=bootstrap.arguments.first().and_then(|i|self.class.string_constant(*i)).unwrap_or("");
        let mut dynamic=0usize;
        let mut static_arg=1usize;
        let mut output=String::new();
        for ch in recipe.chars(){
            match ch{
                '\u{0001}'=>{if let Some(arg)=args.get(dynamic){output.push_str(&arg.display());}dynamic+=1;}
                '\u{0002}'=>{if let Some(index)=bootstrap.arguments.get(static_arg){output.push_str(self.class.string_constant(*index).unwrap_or(""));}static_arg+=1;}
                _=>output.push(ch)
            }
        }
        if output.is_empty(){
            output=args.iter().map(Value::display).collect::<String>();
        }
        output
    }
    fn execute_method(&mut self,method:&MemberInfo,incoming:&[Value])->Result<Option<Value>>{
        let Some(code)=method.code.as_ref() else{return Ok(None);};
        let instructions=decode_instructions(&code.code)?;
        let descriptor=parse_method_descriptor(&method.descriptor)?;
        let is_static=class_method_static(method);
        let mut locals=HashMap::<usize,Value>::new();
        if !is_static{locals.insert(0,Value::Opaque("this".into()));}
        let mut slot=if is_static{0}else{1};
        for (index,ty) in descriptor.parameters.iter().enumerate(){
            locals.insert(slot,incoming.get(index).cloned().unwrap_or(Value::Null));
            slot+=ty.slots();
        }
        let mut stack=Vec::<Value>::new();
        for ins in &instructions{
            let op=ins.opcode;
            if let Some(index)=local_index(ins){
                let effective=if op==0xc4{ins.operands.first().copied().unwrap_or(0)}else{op};
                if matches!(effective,0x15..=0x19|0x1a..=0x2d){
                    stack.push(locals.get(&index).cloned().unwrap_or(Value::Null));continue;
                }
                if matches!(effective,0x36..=0x3a|0x3b..=0x4e){
                    locals.insert(index,pop(&mut stack));continue;
                }
            }
            if matches!(op,0x02..=0x14){
                if let Some(v)=constant_value(self.class,op,&ins.operands){stack.push(v);}continue;
            }
            match op{
                0x01=>stack.push(Value::Null),
                0xbb=>{
                    let index=u16::from_be_bytes([ins.operands[0],ins.operands[1]]);
                    let name=cp_class_name(self.class,index).unwrap_or_default();
                    let simple=simple_name(&name);
                    let kind=match simple{
                        "MeshDefinition"|"Mesh" =>ObjectKind::Mesh,
                        "CubeListBuilder"|"ModelPartBuilder"=>ObjectKind::Builder,
                        "CubeDeformation"|"Dilation"=>ObjectKind::Deformation,
                        "PartPose"|"ModelTransform"=>ObjectKind::Pose,
                        "PartDefinition"|"ModelPartData"=>ObjectKind::Part,
                        "LayerDefinition"|"TexturedModelData"=>ObjectKind::Layer,
                        _=>ObjectKind::Other
                    };
                    stack.push(self.new_object(kind));
                }
                0x59=>if let Some(v)=stack.last().cloned(){stack.push(v);},
                0x5a=>if stack.len()>=2{let a=pop(&mut stack);let b=pop(&mut stack);stack.push(a.clone());stack.push(b);stack.push(a);},
                0x5f=>if stack.len()>=2{let n=stack.len();stack.swap(n-1,n-2);},
                0x57=>{let _=pop(&mut stack);},0x58=>{let _=pop(&mut stack);let _=pop(&mut stack);},
                0x60..=0x73=>{
                    let b=pop(&mut stack).number();let a=pop(&mut stack).number();
                    let result=match(a,b){(Some(a),Some(b))=>match op{
                        0x60..=0x63=>Some(a+b),0x64..=0x67=>Some(a-b),0x68..=0x6b=>Some(a*b),
                        0x6c..=0x6f=>Some(if b.abs()>f64::EPSILON{a/b}else{0.0}),
                        0x70..=0x73=>Some(if b.abs()>f64::EPSILON{a%b}else{0.0}),_=>None},_=>None};
                    stack.push(result.map(Value::Number).unwrap_or(Value::Null));
                }
                0x74..=0x77=>{let v=pop(&mut stack).number().unwrap_or(0.0);stack.push(Value::Number(-v));}
                0x85..=0x93=>{let v=pop(&mut stack);stack.push(v);}
                0x99..=0xa8|0xaa..=0xab|0xc6..=0xc9=>{
                    self.warnings.push(format!("Control-flow opcode 0x{op:02x} in {} was not executed; static model-builder evaluation may be incomplete.",method.name));
                }
                0xb2=>{
                    let index=u16::from_be_bytes([ins.operands[0],ins.operands[1]]);
                    if let Some((owner,name,_))=self.class.member_reference(index){
                        let kind=simple_name(&owner);
                        if kind=="PartPose"&&(name=="ZERO"||name=="f_171404_"){
                            stack.push(self.new_object(ObjectKind::Pose));
                        }else if kind=="CubeDeformation"&&(name=="ZERO"||name=="f_171458_"){
                            stack.push(self.new_object(ObjectKind::Deformation));
                        }else{stack.push(Value::Opaque(format!("{owner}.{name}")));}
                    }else{stack.push(Value::Null);}
                }
                0xba=>{
                    let index=u16::from_be_bytes([ins.operands[0],ins.operands[1]]);
                    let descriptor=self.class.invoke_dynamic(index).map(|v|v.2).unwrap_or_else(||"()Ljava/lang/String;".into());
                    let count=parse_method_descriptor(&descriptor)?.parameters.len();
                    let args=pop_n(&mut stack,count);
                    stack.push(Value::String(self.concat_dynamic(index,&args)));
                }
                0xb6..=0xb9=>{
                    let index=u16::from_be_bytes([ins.operands[0],ins.operands[1]]);
                    let Some((owner,name,desc))=self.class.member_reference(index) else{continue;};
                    let descriptor=parse_method_descriptor(&desc)?;
                    let args=pop_n(&mut stack,descriptor.parameters.len());
                    let receiver=if op==0xb8{Value::Null}else{pop(&mut stack)};
                    let simple=simple_name(&owner);
                    if name=="<init>"{
                        if let Value::Handle(id)=receiver{
                            let object=&mut self.objects[id];
                            match object.kind{
                                ObjectKind::Deformation=>{
                                    let values:Vec<f64>=args.iter().filter_map(Value::number).collect();
                                    object.inflate=if values.len()>=3 {(values[values.len()-3]+values[values.len()-2]+values[values.len()-1])/3.0}
                                        else {values.last().copied().unwrap_or(0.0)};
                                }
                                ObjectKind::Pose=>{
                                    let values:Vec<f64>=args.iter().filter_map(Value::number).collect();
                                    if values.len()>=6{object.pose.copy_from_slice(&values[values.len()-6..]);}
                                    else if values.len()>=3{object.pose[0..3].copy_from_slice(&values[values.len()-3..]);}
                                }
                                ObjectKind::Other=>{
                                    if simple=="CubeDeformation"||simple=="Dilation"{
                                        let values:Vec<f64>=args.iter().filter_map(Value::number).collect();
                                        object.kind=ObjectKind::Deformation;
                                        object.inflate=if values.len()>=3 {(values[values.len()-3]+values[values.len()-2]+values[values.len()-1])/3.0}
                                            else {values.last().copied().unwrap_or(0.0)};
                                    }
                                }
                                _=>{}
                            }
                        }
                        continue;
                    }
                    if simple=="CubeListBuilder"&&(name=="create"||name=="m_171558_"){
                        stack.push(self.new_object(ObjectKind::Builder));continue;
                    }
                    if (simple=="PartPose"||simple=="ModelTransform")&&matches!(name.as_str(),"offset"|"m_171419_"|"pivot"){
                        let values:Vec<f64>=args.iter().filter_map(Value::number).collect();
                        let id=match self.new_object(ObjectKind::Pose){Value::Handle(v)=>v,_=>0};
                        if values.len()>=3{self.objects[id].pose[0..3].copy_from_slice(&values[values.len()-3..]);}
                        stack.push(Value::Handle(id));continue;
                    }
                    if (simple=="PartPose"||simple=="ModelTransform")&&matches!(name.as_str(),"offsetAndRotation"|"m_171423_"|"of"){
                        let values:Vec<f64>=args.iter().filter_map(Value::number).collect();
                        let id=match self.new_object(ObjectKind::Pose){Value::Handle(v)=>v,_=>0};
                        if values.len()>=6{self.objects[id].pose.copy_from_slice(&values[values.len()-6..]);}
                        stack.push(Value::Handle(id));continue;
                    }
                    if (simple=="PartPose"||simple=="ModelTransform")&&matches!(name.as_str(),"rotation"|"m_171422_"|"m_171420_"){
                        let values:Vec<f64>=args.iter().filter_map(Value::number).collect();
                        let id=match self.new_object(ObjectKind::Pose){Value::Handle(v)=>v,_=>0};
                        if values.len()>=3{self.objects[id].pose[3..6].copy_from_slice(&values[values.len()-3..]);}
                        stack.push(Value::Handle(id));continue;
                    }
                    if (simple=="MeshDefinition"||simple=="Mesh")&&matches!(name.as_str(),"getRoot"|"m_171576_"){
                        let id=self.ensure_root();stack.push(Value::Handle(id));continue;
                    }
                    if (simple=="CubeDeformation"||simple=="Dilation")&&matches!(name.as_str(),"extend"|"extendUniform"){
                        let base=self.object(&receiver).map(|v|v.inflate).unwrap_or(0.0);
                        let amount=args.last().and_then(Value::number).unwrap_or(0.0);
                        let id=match self.new_object(ObjectKind::Deformation){Value::Handle(v)=>v,_=>0};
                        self.objects[id].inflate=base+amount;stack.push(Value::Handle(id));continue;
                    }
                    if (simple=="CubeListBuilder"||simple=="ModelPartBuilder")&&matches!(name.as_str(),"texOffs"|"m_171514_"){
                        if let Value::Handle(id)=receiver{
                            let u=args.first().and_then(Value::number).unwrap_or(0.0) as i32;
                            let v=args.get(1).and_then(Value::number).unwrap_or(0.0) as i32;
                            self.objects[id].uv_x=u;self.objects[id].uv_y=v;stack.push(Value::Handle(id));
                        }continue;
                    }
                    if (simple=="CubeListBuilder"||simple=="ModelPartBuilder")&&matches!(name.as_str(),"mirror"|"m_171480_"){
                        if let Value::Handle(id)=receiver{
                            self.objects[id].mirror=args.first().and_then(Value::boolean).unwrap_or(true);
                            stack.push(Value::Handle(id));
                        }continue;
                    }
                    if (simple=="CubeListBuilder"||simple=="ModelPartBuilder")&&matches!(name.as_str(),"addBox"|"m_171481_"|"m_171482_"|"m_171483_"|"m_171484_"|"m_171488_"|"m_171489_"){
                        if let Value::Handle(id)=receiver{
                            let mut params=args.clone();
                            if params.first().and_then(Value::string).is_some(){params.remove(0);}
                            let numbers:Vec<f64>=params.iter().filter_map(Value::number).collect();
                            let inflate=params.iter().find_map(|v|if let Value::Handle(h)=v{self.objects.get(*h).filter(|o|o.kind==ObjectKind::Deformation).map(|o|o.inflate)}else{None}).unwrap_or(0.0);
                            if numbers.len()>=6{
                                let builder=&mut self.objects[id];
                                let cube_index=builder.cubes.len()+1;
                                builder.cubes.push(Cube{
                                    x:numbers[0],y:numbers[1],z:numbers[2],dx:numbers[3],dy:numbers[4],dz:numbers[5],
                                    uv_x:builder.uv_x,uv_y:builder.uv_y,mirror:builder.mirror,inflate,
                                    name:format!("cube_{cube_index}")
                                });
                            }else{
                                self.warnings.push(format!("Unsupported CubeListBuilder.addBox signature in {} at bytecode offset {}.",self.class_name,ins.offset));
                            }
                            stack.push(Value::Handle(id));
                        }continue;
                    }
                    if (simple=="PartDefinition"||simple=="ModelPartData")&&matches!(name.as_str(),"addOrReplaceChild"|"addChild"|"m_171599_"|"m_171591_"){
                        let parent=if let Value::Handle(id)=receiver{id}else{self.ensure_root()};
                        let name=args.first().map(Value::display).unwrap_or_else(||format!("part_{}",self.parts.len()));
                        let builder=args.get(1).and_then(|v|if let Value::Handle(id)=v{Some(*id)}else{None});
                        let pose=args.get(2).and_then(|v|if let Value::Handle(id)=v{Some(*id)}else{None});
                        let child=self.push_child(parent,name,builder,pose);
                        stack.push(Value::Handle(child));continue;
                    }
                    if (simple=="LayerDefinition"||simple=="TexturedModelData")&&matches!(name.as_str(),"create"|"m_171565_"|"of"){
                        if args.len()>=2{
                            if let Some(width)=args.get(args.len()-2).and_then(Value::number){self.texture_width=(width as i64).max(1) as u32;}
                            if let Some(height)=args.last().and_then(Value::number){self.texture_height=(height as i64).max(1) as u32;}
                        }
                        stack.push(self.new_object(ObjectKind::Layer));continue;
                    }
                    if simple=="Math"&&(name=="sin"||name=="cos"){
                        let value=args.first().and_then(Value::number).unwrap_or(0.0);
                        stack.push(Value::Number(if name=="sin"{value.sin()}else{value.cos()}));continue;
                    }
                    if owner.is_empty()||simple==self.class_name{
                        let chosen=self.methods.get(&name).and_then(|candidates|{
                            candidates.iter().copied().find(|idx|self.class.methods[*idx].descriptor==desc)
                                .or_else(||candidates.first().copied())
                        });
                        if let Some(chosen)=chosen{
                            let helper_args=args.clone();
                            if let Some(value)=self.invoke_helper(chosen,&helper_args)?{stack.push(value);continue;}
                        }
                    }
                    if !return_is_void(&desc){stack.push(Value::Opaque(format!("{owner}.{name}")));}
                }
                0xc0=>{},
                0xb0=>return Ok(Some(pop(&mut stack))),
                0xb1=>return Ok(None),
                0x00=>{},
                _=>{}
            }
        }
        Ok(stack.pop())
    }
    fn execute(&mut self)->Result<()>{
        let priority=["createBodyLayer","create","createMesh","createLayerDefinition","getTexturedModelData","getModelData","createLayer"];
        let mut candidates=Vec::new();
        for name in priority{
            if let Some(items)=self.methods.get(name){candidates.extend(items.iter().copied());}
        }
        if candidates.is_empty(){
            for (index,method) in self.class.methods.iter().enumerate(){
                let text=method.code.as_ref().map(|c|c.code.clone()).unwrap_or_default();
                let has_modern_calls=text.windows(2).any(|pair|pair==[0xb6,0x00])||method.descriptor.contains("LayerDefinition");
                if has_modern_calls && method.name!="<init>"{candidates.push(index);}
            }
        }
        for method_index in candidates{
            self.parts.clear();self.objects.clear();self.root=None;self.warnings.clear();self.call_stack.clear();
            let method=self.class.methods[method_index].clone();
            let args=self.default_args(&method)?;
            match self.invoke_helper(method_index,&args){
                Ok(_)=>{},
                Err(error)=>self.warnings.push(format!("Bytecode interpretation failed in {}: {error}",method.name))
            }
            if self.parts.iter().any(|p|!p.cubes.is_empty()){return Ok(());}
        }
        bail!("No modern PartDefinition/CubeListBuilder geometry was recovered from this class file.")
    }
}

pub fn parse_modern_bytecode_class_file(path:&Path,constructor_args:&[serde_json::Value])->Result<ParsedModel>{
    let bytes=fs::read(path).with_context(||format!("Reading modern compiled model {}",path.display()))?;
    let class=ClassFile::parse(&bytes)?;
    let mut interpreter=Interpreter::new(&class);
    interpreter.execute()?;
    let mut model=ParsedModel::new(interpreter.class_name.clone(),interpreter.texture_width,interpreter.texture_height,interpreter.parts,interpreter.warnings);
    model.source_format="modern_mojmap_bytecode".into();
    model.constructor_args=constructor_args.to_vec();

    let animation_methods:Vec<_>=class.methods.iter().filter(|method|matches!(method.name.as_str(),"setupAnim"|"setAngles"|"renderToBuffer"|"render")).map(|method|{
        let bytecode=method.code.as_ref().map(|code|code.code.iter().map(|b|format!("{b:02x}")).collect::<String>()).unwrap_or_default();
        serde_json::json!({"method":method.name,"descriptor":method.descriptor,"bytecode_hex":bytecode})
    }).collect();
    let mut preserved_analysis = serde_json::json!({
        "source_format":"modern_mojmap_bytecode",
        "mode":"preserve_bytecode",
        "animation_methods":animation_methods,
        "note":"Geometry was interpreted directly from JVM bytecode. Animation methods are retained as exact bytecode evidence."
    });
    match crate::animation::legacy::extract_legacy_animation(
        &class,
        &model.parts,
        &std::collections::BTreeMap::new()
    ) {
        Ok(Some(animation)) => {
            model.animation_analysis = animation.analysis;
            model.animation_analysis["source_format"] = serde_json::json!("modern_mojmap_bytecode");
            model.animation_analysis["mode"] = serde_json::json!("sampled_bytecode_formulas");
            model.animation_analysis["preserved_bytecode_methods"] = preserved_analysis["animation_methods"].clone();
            model.animation_logic = animation.logic;
            model.animation_source = animation.source;
            model.blockbench_animations = animation.blockbench_animations;
            model.warnings.extend(
                model.animation_analysis["warnings"].as_array()
                    .into_iter().flatten()
                    .filter_map(|warning| warning.as_str().map(str::to_owned))
            );
            model.baked_animation_json = model.animation_analysis.clone();
        }
        Ok(None) => {
            if !animation_methods.is_empty() {
                model.warnings.push("Compiled animation methods were found and their raw bytecode was retained; no supported ModelPart pose assignments were recovered.".into());
            }
            model.animation_analysis = preserved_analysis;
        }
        Err(error) => {
            model.warnings.push(format!("Compiled modern animation analysis was incomplete: {error}"));
            model.animation_analysis = preserved_analysis;
        }
    }
    Ok(model)
}

pub fn is_modern_model_class(class:&ClassFile)->bool{
    class.constant_pool.iter().flatten().any(|entry|match entry{
        ConstantPoolEntry::Utf8(text)=>["PartDefinition","CubeListBuilder","ModelPartData","ModelPartBuilder","LayerDefinition","TexturedModelData","MeshDefinition"]
            .iter().any(|needle|text.contains(needle)),
        _=>false
    })
}

pub fn parse_any_model_class(path:&Path,constructor_args:&[serde_json::Value])->Result<ParsedModel>{
    let bytes=fs::read(path).with_context(||format!("Reading compiled model {}",path.display()))?;
    let class=ClassFile::parse(&bytes)?;
    if is_modern_model_class(&class){
        parse_modern_bytecode_class_file(path,constructor_args)
    }else{
        super::legacy_model::parse_legacy_class_file(path,constructor_args)
    }
}
