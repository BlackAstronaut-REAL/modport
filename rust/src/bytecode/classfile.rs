use anyhow::{bail, Context, Result};
use std::convert::TryInto;

#[derive(Debug, Clone)]
pub enum ConstantPoolEntry {
    Utf8(String), Integer(i32), Float(f32), Long(i64), Double(f64),
    Class { name_index: u16 }, String { string_index: u16 },
    Fieldref { class_index: u16, name_and_type_index: u16 },
    Methodref { class_index: u16, name_and_type_index: u16 },
    InterfaceMethodref { class_index: u16, name_and_type_index: u16 },
    NameAndType { name_index: u16, descriptor_index: u16 },
    MethodHandle { reference_kind: u8, reference_index: u16 },
    MethodType { descriptor_index: u16 },
    Dynamic { bootstrap_method_attr_index: u16, name_and_type_index: u16 },
    InvokeDynamic { bootstrap_method_attr_index: u16, name_and_type_index: u16 },
    Module { name_index: u16 }, Package { name_index: u16 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstantValue {
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    String(String),
    Class(String),
}

#[derive(Debug, Clone, Default)]
pub struct AttributeInfo { pub name: String, pub info: Vec<u8> }

#[derive(Debug, Clone, Default)]
pub struct ExceptionHandler {
    pub start_pc: u16, pub end_pc: u16, pub handler_pc: u16, pub catch_type: u16,
}

#[derive(Debug, Clone, Default)]
pub struct LineNumber { pub start_pc: u16, pub line_number: u16 }

#[derive(Debug, Clone, Default)]
pub struct CodeAttribute {
    pub max_stack: u16,
    pub max_locals: u16,
    pub code: Vec<u8>,
    pub exception_table: Vec<ExceptionHandler>,
    pub attributes: Vec<AttributeInfo>,
    pub line_numbers: Vec<LineNumber>,
}

#[derive(Debug, Clone, Default)]
pub struct MemberInfo {
    pub access_flags: u16,
    pub name: String,
    pub descriptor: String,
    pub attributes: Vec<AttributeInfo>,
    pub code: Option<CodeAttribute>,
}

#[derive(Debug, Clone, Default)]
pub struct BootstrapMethod { pub method_ref: u16, pub arguments: Vec<u16> }

#[derive(Debug, Clone)]
pub struct ClassFile {
    pub minor_version: u16,
    pub major_version: u16,
    pub access_flags: u16,
    pub this_class: u16,
    pub super_class: u16,
    pub interfaces: Vec<u16>,
    pub constant_pool: Vec<Option<ConstantPoolEntry>>,
    pub fields: Vec<MemberInfo>,
    pub methods: Vec<MemberInfo>,
    pub attributes: Vec<AttributeInfo>,
    pub bootstrap_methods: Vec<BootstrapMethod>,
}

struct Reader<'a> { bytes: &'a [u8], position: usize }

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, position: 0 } }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.position.checked_add(n).context("Class-file offset overflow")?;
        if end > self.bytes.len() { bail!("Unexpected end of class file at byte {}", self.position); }
        let part = &self.bytes[self.position..end];
        self.position = end;
        Ok(part)
    }
    fn u1(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    fn u2(&mut self) -> Result<u16> { Ok(u16::from_be_bytes(self.take(2)?.try_into()?)) }
    fn u4(&mut self) -> Result<u32> { Ok(u32::from_be_bytes(self.take(4)?.try_into()?)) }
    fn u8(&mut self) -> Result<u64> { Ok(u64::from_be_bytes(self.take(8)?.try_into()?)) }
    fn done(&self) -> bool { self.position == self.bytes.len() }
}

fn modified_utf8(bytes: &[u8]) -> String {
    let mut units = Vec::<u16>::new();
    let mut i = 0;
    while i < bytes.len() {
        let a = bytes[i];
        if a & 0x80 == 0 {
            units.push(a as u16); i += 1;
        } else if a & 0xe0 == 0xc0 && i + 1 < bytes.len() {
            let b = bytes[i + 1];
            if b & 0xc0 != 0x80 { units.push(0xfffd); i += 1; continue; }
            units.push((((a & 0x1f) as u16) << 6) | (b & 0x3f) as u16); i += 2;
        } else if a & 0xf0 == 0xe0 && i + 2 < bytes.len() {
            let b = bytes[i + 1]; let c = bytes[i + 2];
            if b & 0xc0 != 0x80 || c & 0xc0 != 0x80 { units.push(0xfffd); i += 1; continue; }
            units.push((((a & 0x0f) as u16) << 12) | (((b & 0x3f) as u16) << 6) | (c & 0x3f) as u16);
            i += 3;
        } else { units.push(0xfffd); i += 1; }
    }
    String::from_utf16_lossy(&units)
}

fn cp_utf8(cp: &[Option<ConstantPoolEntry>], index: u16) -> Option<&str> {
    match cp.get(index as usize)?.as_ref()? {
        ConstantPoolEntry::Utf8(value) => Some(value), _ => None,
    }
}

fn read_attributes(reader: &mut Reader<'_>, cp: &[Option<ConstantPoolEntry>]) -> Result<Vec<AttributeInfo>> {
    let count = reader.u2()? as usize;
    let mut attrs = Vec::with_capacity(count);
    for _ in 0..count {
        let name_index = reader.u2()?;
        let size = reader.u4()? as usize;
        let name = cp_utf8(cp, name_index).context("Invalid JVM attribute name index")?.to_owned();
        attrs.push(AttributeInfo { name, info: reader.take(size)?.to_vec() });
    }
    Ok(attrs)
}

fn parse_line_numbers(info: &[u8]) -> Result<Vec<LineNumber>> {
    let mut r = Reader::new(info);
    let count = r.u2()? as usize;
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count { lines.push(LineNumber { start_pc: r.u2()?, line_number: r.u2()? }); }
    if !r.done() { bail!("Malformed LineNumberTable attribute"); }
    Ok(lines)
}

fn parse_code_attribute(info: &[u8], cp: &[Option<ConstantPoolEntry>]) -> Result<CodeAttribute> {
    let mut r = Reader::new(info);
    let max_stack = r.u2()?;
    let max_locals = r.u2()?;
    let code_len = r.u4()? as usize;
    let code = r.take(code_len)?.to_vec();
    let exception_count = r.u2()? as usize;
    let mut exception_table = Vec::with_capacity(exception_count);
    for _ in 0..exception_count {
        exception_table.push(ExceptionHandler {
            start_pc: r.u2()?, end_pc: r.u2()?, handler_pc: r.u2()?, catch_type: r.u2()?,
        });
    }
    let attributes = read_attributes(&mut r, cp)?;
    let mut line_numbers = Vec::new();
    for attr in &attributes {
        if attr.name == "LineNumberTable" { line_numbers.extend(parse_line_numbers(&attr.info)?); }
    }
    if !r.done() { bail!("Malformed Code attribute"); }
    Ok(CodeAttribute { max_stack, max_locals, code, exception_table, attributes, line_numbers })
}

fn read_members(reader: &mut Reader<'_>, cp: &[Option<ConstantPoolEntry>], with_code: bool) -> Result<Vec<MemberInfo>> {
    let count = reader.u2()? as usize;
    let mut members = Vec::with_capacity(count);
    for _ in 0..count {
        let access_flags = reader.u2()?;
        let name_index = reader.u2()?;
        let descriptor_index = reader.u2()?;
        let attributes = read_attributes(reader, cp)?;
        let name = cp_utf8(cp, name_index).context("Invalid member name index")?.to_owned();
        let descriptor = cp_utf8(cp, descriptor_index).context("Invalid member descriptor index")?.to_owned();
        let code = if with_code {
            attributes.iter().find(|a| a.name == "Code").map(|a| parse_code_attribute(&a.info, cp)).transpose()?
        } else { None };
        members.push(MemberInfo { access_flags, name, descriptor, attributes, code });
    }
    Ok(members)
}

impl ClassFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let magic = r.u4()?;
        if magic != 0xcafebabe { bail!("Invalid class-file magic: 0x{magic:08x}"); }
        let minor_version = r.u2()?;
        let major_version = r.u2()?;
        let cp_count = r.u2()? as usize;
        if cp_count == 0 { bail!("Empty JVM constant pool"); }

        let mut constant_pool = vec![None; cp_count];
        let mut index = 1usize;
        while index < cp_count {
            let tag = r.u1()?;
            let entry = match tag {
                1 => { let len = r.u2()? as usize; ConstantPoolEntry::Utf8(modified_utf8(r.take(len)?)) }
                3 => ConstantPoolEntry::Integer(r.u4()? as i32),
                4 => ConstantPoolEntry::Float(f32::from_bits(r.u4()?)),
                5 => ConstantPoolEntry::Long(r.u8()? as i64),
                6 => ConstantPoolEntry::Double(f64::from_bits(r.u8()?)),
                7 => ConstantPoolEntry::Class { name_index: r.u2()? },
                8 => ConstantPoolEntry::String { string_index: r.u2()? },
                9 => ConstantPoolEntry::Fieldref { class_index: r.u2()?, name_and_type_index: r.u2()? },
                10 => ConstantPoolEntry::Methodref { class_index: r.u2()?, name_and_type_index: r.u2()? },
                11 => ConstantPoolEntry::InterfaceMethodref { class_index: r.u2()?, name_and_type_index: r.u2()? },
                12 => ConstantPoolEntry::NameAndType { name_index: r.u2()?, descriptor_index: r.u2()? },
                15 => ConstantPoolEntry::MethodHandle { reference_kind: r.u1()?, reference_index: r.u2()? },
                16 => ConstantPoolEntry::MethodType { descriptor_index: r.u2()? },
                17 => ConstantPoolEntry::Dynamic { bootstrap_method_attr_index: r.u2()?, name_and_type_index: r.u2()? },
                18 => ConstantPoolEntry::InvokeDynamic { bootstrap_method_attr_index: r.u2()?, name_and_type_index: r.u2()? },
                19 => ConstantPoolEntry::Module { name_index: r.u2()? },
                20 => ConstantPoolEntry::Package { name_index: r.u2()? },
                other => bail!("Unsupported constant-pool tag {other} at index {index}"),
            };
            let wide = matches!(&entry, ConstantPoolEntry::Long(_) | ConstantPoolEntry::Double(_));
            constant_pool[index] = Some(entry);
            index += if wide { 2 } else { 1 };
        }

        let access_flags = r.u2()?;
        let this_class = r.u2()?;
        let super_class = r.u2()?;
        let interface_count = r.u2()? as usize;
        let mut interfaces = Vec::with_capacity(interface_count);
        for _ in 0..interface_count { interfaces.push(r.u2()?); }
        let fields = read_members(&mut r, &constant_pool, false)?;
        let methods = read_members(&mut r, &constant_pool, true)?;
        let attributes = read_attributes(&mut r, &constant_pool)?;
        if !r.done() { bail!("Trailing bytes after JVM class attributes"); }

        let mut result = Self {
            minor_version, major_version, access_flags, this_class, super_class,
            interfaces, constant_pool, fields, methods, attributes, bootstrap_methods: Vec::new(),
        };
        result.bootstrap_methods = result.read_bootstrap_methods()?;
        Ok(result)
    }

    pub fn utf8(&self, index: u16) -> Option<&str> { cp_utf8(&self.constant_pool, index) }
    pub fn class_name(&self, index: u16) -> Option<&str> {
        match self.constant_pool.get(index as usize)?.as_ref()? {
            ConstantPoolEntry::Class { name_index } => self.utf8(*name_index), _ => None,
        }
    }
    pub fn this_name(&self) -> Option<&str> { self.class_name(self.this_class) }
    pub fn super_name(&self) -> Option<&str> { self.class_name(self.super_class) }

    pub fn member_reference(&self, index: u16) -> Option<(String, String, String)> {
        let (class_index, nt_index) = match self.constant_pool.get(index as usize)?.as_ref()? {
            ConstantPoolEntry::Fieldref { class_index, name_and_type_index }
            | ConstantPoolEntry::Methodref { class_index, name_and_type_index }
            | ConstantPoolEntry::InterfaceMethodref { class_index, name_and_type_index } => (*class_index, *name_and_type_index),
            _ => return None,
        };
        let owner = self.class_name(class_index)?.to_owned();
        let (name_index, desc_index) = match self.constant_pool.get(nt_index as usize)?.as_ref()? {
            ConstantPoolEntry::NameAndType { name_index, descriptor_index } => (*name_index, *descriptor_index),
            _ => return None,
        };
        Some((owner, self.utf8(name_index)?.to_owned(), self.utf8(desc_index)?.to_owned()))
    }

    pub fn string_constant(&self, index: u16) -> Option<&str> {
        match self.constant_pool.get(index as usize)?.as_ref()? {
            ConstantPoolEntry::String { string_index } => self.utf8(*string_index),
            ConstantPoolEntry::Utf8(value) => Some(value), _ => None,
        }
    }

    /// Resolve an ldc-loadable constant into a typed value.
    pub fn constant(&self, index: u16) -> Option<ConstantValue> {
        match self.constant_pool.get(index as usize)?.as_ref()? {
            ConstantPoolEntry::Integer(value) => Some(ConstantValue::Integer(*value)),
            ConstantPoolEntry::Float(value) => Some(ConstantValue::Float(*value)),
            ConstantPoolEntry::Long(value) => Some(ConstantValue::Long(*value)),
            ConstantPoolEntry::Double(value) => Some(ConstantValue::Double(*value)),
            ConstantPoolEntry::String { string_index } => self.utf8(*string_index).map(|v| ConstantValue::String(v.to_owned())),
            ConstantPoolEntry::Class { name_index } => self.utf8(*name_index).map(|v| ConstantValue::Class(v.to_owned())),
            _ => None,
        }
    }

    /// Resolve an invokedynamic constant to its bootstrap method index, name,
    /// and descriptor. Bootstrap arguments remain available on the class file.
    pub fn invoke_dynamic(&self, index: u16) -> Option<(u16, String, String)> {
        let (bootstrap_index, name_and_type_index) = match self.constant_pool.get(index as usize)?.as_ref()? {
            ConstantPoolEntry::InvokeDynamic { bootstrap_method_attr_index, name_and_type_index } =>
                (*bootstrap_method_attr_index, *name_and_type_index),
            _ => return None,
        };
        let (name_index, descriptor_index) = match self.constant_pool.get(name_and_type_index as usize)?.as_ref()? {
            ConstantPoolEntry::NameAndType { name_index, descriptor_index } => (*name_index, *descriptor_index),
            _ => return None,
        };
        Some((bootstrap_index, self.utf8(name_index)?.to_owned(), self.utf8(descriptor_index)?.to_owned()))
    }

    fn read_bootstrap_methods(&self) -> Result<Vec<BootstrapMethod>> {
        let Some(attr) = self.attributes.iter().find(|a| a.name == "BootstrapMethods") else { return Ok(Vec::new()); };
        let mut r = Reader::new(&attr.info);
        let count = r.u2()? as usize;
        let mut methods = Vec::with_capacity(count);
        for _ in 0..count {
            let method_ref = r.u2()?;
            let n = r.u2()? as usize;
            let mut arguments = Vec::with_capacity(n);
            for _ in 0..n { arguments.push(r.u2()?); }
            methods.push(BootstrapMethod { method_ref, arguments });
        }
        if !r.done() { bail!("Malformed BootstrapMethods attribute"); }
        Ok(methods)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u1(out: &mut Vec<u8>, v: u8) { out.push(v); }
    fn u2(out: &mut Vec<u8>, v: u16) { out.extend(v.to_be_bytes()); }
    fn u4(out: &mut Vec<u8>, v: u32) { out.extend(v.to_be_bytes()); }
    fn utf8(out: &mut Vec<u8>, value: &str) { u1(out, 1); u2(out, value.len() as u16); out.extend(value.as_bytes()); }

    fn minimal_class() -> Vec<u8> {
        let mut out = Vec::new();
        u4(&mut out, 0xcafebabe); u2(&mut out, 0); u2(&mut out, 52); u2(&mut out, 10);
        utf8(&mut out, "Test");
        u1(&mut out, 7); u2(&mut out, 1);
        utf8(&mut out, "java/lang/Object");
        u1(&mut out, 7); u2(&mut out, 3);
        utf8(&mut out, "<init>"); utf8(&mut out, "()V");
        u1(&mut out, 12); u2(&mut out, 5); u2(&mut out, 6);
        u1(&mut out, 10); u2(&mut out, 4); u2(&mut out, 7);
        utf8(&mut out, "Code");
        u2(&mut out, 0x0021); u2(&mut out, 2); u2(&mut out, 4);
        u2(&mut out, 0); u2(&mut out, 0);
        u2(&mut out, 1); u2(&mut out, 1); u2(&mut out, 5); u2(&mut out, 6);
        u2(&mut out, 1); u2(&mut out, 9); u4(&mut out, 17);
        u2(&mut out, 1); u2(&mut out, 1); u4(&mut out, 5);
        out.extend([0x2a, 0xb7, 0, 8, 0xb1]);
        u2(&mut out, 0); u2(&mut out, 0); u2(&mut out, 0);
        out
    }

    #[test]
    fn parses_class_header_pool_and_constructor() {
        let class = ClassFile::parse(&minimal_class()).unwrap();
        assert_eq!(class.major_version, 52);
        assert_eq!(class.this_name(), Some("Test"));
        assert_eq!(class.super_name(), Some("java/lang/Object"));
        assert_eq!(class.methods[0].name, "<init>");
        assert_eq!(class.methods[0].code.as_ref().unwrap().code, [0x2a, 0xb7, 0, 8, 0xb1]);
        assert_eq!(class.member_reference(8).unwrap(), ("java/lang/Object".into(), "<init>".into(), "()V".into()));
    }

    #[test]
    fn rejects_non_class_input() { assert!(ClassFile::parse(b"not a class").is_err()); }
}
