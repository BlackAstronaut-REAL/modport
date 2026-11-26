use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tempfile::Builder;
use walkdir::WalkDir;
use rayon::prelude::*;
use zip::ZipArchive;

static REGISTRY_CACHE: OnceLock<Mutex<HashMap<String, (u64, CompiledRegistryIndex)>>> = OnceLock::new();

use crate::bytecode::{
    decode_instructions, parse_method_descriptor, ClassFile, ConstantPoolEntry, Instruction,
};

#[derive(Debug, Clone)]
pub struct CompiledClassInfo {
    pub class_name: String,
    pub superclass: String,
    pub access_flags: u16,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledRegistryIndex {
    /// Simple class name -> class header and path.
    pub classes: BTreeMap<String, CompiledClassInfo>,
    /// Concrete class name -> every registry path recovered from bytecode.
    pub registries: BTreeMap<String, BTreeSet<String>>,
    /// JVM owner#field -> concrete class assigned to that static field.
    pub field_classes: BTreeMap<String, String>,
    /// Decoded class files retained for lambda resolution and downstream exporters.
    pub parsed_classes: HashMap<String, Arc<ClassFile>>,
    /// Original path -> decoded class, matching the winning simple-name class index.
    pub parsed_class_paths: HashMap<PathBuf, Arc<ClassFile>>,
    pub warnings: Vec<String>,
    /// Keeps archive-extracted class files alive while downstream exporters
    /// consume the paths recorded in `classes`.
    pub archive_extractions: Vec<Arc<tempfile::TempDir>>,
}

#[derive(Debug, Clone)]
enum Value {
    Text(String),
    Class(String),
    New(String),
    Object(String),
    Field(String),
    Lambda(String),
    Unknown,
}

fn simple(value: &str) -> String {
    value.rsplit('/').next().unwrap_or(value).rsplit('$').next().unwrap_or(value).to_owned()
}

fn field_key(owner: &str, name: &str) -> String {
    format!("{owner}#{name}")
}

fn normalize_registry(value: &str) -> Option<String> {
    let mut value = value.trim().trim_matches('"').trim_matches('\'').to_owned();
    if let Some((_, path)) = value.split_once(':') {
        value = path.to_owned();
    }
    value = value.trim_matches('/').replace('-', "_").to_ascii_lowercase();
    if value.ends_with(".png") || value.ends_with(".jpg") || value.ends_with(".json")
        || value.starts_with("assets/") || value.starts_with("textures/") {
        return None;
    }
    if value.is_empty() || value.len() > 128
        || !value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-' | '/'))
        || value.replace('_', "").chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(value)
}

fn u16_at(bytes: &[u8], index: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(index)?, *bytes.get(index + 1)?]))
}

fn load_constant(class: &ClassFile, instruction: &Instruction) -> Value {
    let op = instruction.opcode;
    let index = match op {
        0x12 => instruction.operands.first().copied().map(u16::from),
        0x13 | 0x14 => u16_at(&instruction.operands, 0),
        _ => None,
    };
    if let Some(index) = index {
        return match class.constant(index) {
            Some(crate::bytecode::classfile::ConstantValue::String(value)) => Value::Text(value),
            Some(crate::bytecode::classfile::ConstantValue::Class(value)) => Value::Class(value),
            _ => Value::Unknown,
        };
    }
    match op {
        0x01 | 0x02..=0x0f | 0x10 | 0x11 => Value::Unknown,
        _ => Value::Unknown,
    }
}

fn pop(stack: &mut Vec<Value>) -> Value {
    stack.pop().unwrap_or(Value::Unknown)
}

fn pop_n(stack: &mut Vec<Value>, count: usize) -> Vec<Value> {
    let split = stack.len().saturating_sub(count);
    let mut values = vec![Value::Unknown; count.saturating_sub(stack.len())];
    values.extend(stack.drain(split..));
    values
}

fn lambda_class(
    class: &ClassFile,
    instruction: &Instruction,
    parsed_classes: &HashMap<String, Arc<ClassFile>>,
) -> Option<String> {
    let cp_index = u16_at(&instruction.operands, 0)?;
    let (bootstrap_index, _, _) = class.invoke_dynamic(cp_index)?;
    let bootstrap = class.bootstrap_methods.get(bootstrap_index as usize)?;
    for argument in &bootstrap.arguments {
        let ConstantPoolEntry::MethodHandle { reference_kind, reference_index } =
            class.constant_pool.get(*argument as usize)?.as_ref()? else { continue };
        let (owner, method_name, _) = class.member_reference(*reference_index)?;
        // Constructor method references (MyItem::new) are represented directly
        // as REF_newInvokeSpecial handles. They have no synthetic lambda body or
        // NEW opcode to inspect; the owner is the instantiated content class.
        if *reference_kind == 8 || method_name == "<init>" {
            return Some(simple(&owner));
        }
        let target = parsed_classes.get(&owner)?;
        let method = target.methods.iter().find(|method| method.name == method_name)?;
        let instructions = decode_instructions(&method.code.as_ref()?.code).ok()?;
        // The synthetic lambda body normally constructs exactly one registered
        // object. Keep the first concrete allocation and let type classification
        // reject helper/internal objects when needed.
        for instruction in instructions {
            if instruction.opcode == 0xbb {
                let index = u16_at(&instruction.operands, 0)?;
                return target.class_name(index).map(simple);
            }
        }
    }
    None
}

fn load_reference_local(opcode: u8, operands: &[u8], locals: &[Value]) -> Option<Value> {
    let index = match opcode {
        0x19 => *operands.first()? as usize,
        0x2a..=0x2d => (opcode - 0x2a) as usize,
        _ => return None,
    };
    Some(locals.get(index).cloned().unwrap_or(Value::Unknown))
}

fn store_reference_local(opcode: u8, operands: &[u8], value: Value, locals: &mut [Value]) -> bool {
    let index = match opcode {
        0x3a => match operands.first() {
            Some(index) => *index as usize,
            None => return false,
        },
        0x4b..=0x4e => (opcode - 0x4b) as usize,
        _ => return false,
    };
    if let Some(slot) = locals.get_mut(index) {
        *slot = value;
    }
    true
}

fn static_field_assignments(
    class: &ClassFile,
    parsed_classes: &HashMap<String, Arc<ClassFile>>,
) -> BTreeMap<String, String> {
    let owner = class.this_name().unwrap_or("");
    let mut output = BTreeMap::new();
    let Some(method) = class.methods.iter().find(|method| method.name == "<clinit>") else {
        return output;
    };
    let Some(code) = method.code.as_ref() else { return output };
    let Ok(instructions) = decode_instructions(&code.code) else { return output };
    let mut stack = Vec::<Value>::new();
    let mut locals = vec![Value::Unknown; code.max_locals as usize];

    for instruction in &instructions {
        let op = instruction.opcode;
        if (0x12..=0x14).contains(&op) || (0x01..=0x11).contains(&op) {
            stack.push(load_constant(class, instruction));
            continue;
        }
        match op {
            0x00 => {}
            0x19 | 0x2a..=0x2d => stack.push(
                load_reference_local(op, &instruction.operands, &locals).unwrap_or(Value::Unknown)
            ),
            0x15..=0x18 | 0x1a..=0x29 => stack.push(Value::Unknown),
            0x3a | 0x4b..=0x4e => {
                let value = pop(&mut stack);
                store_reference_local(op, &instruction.operands, value, &mut locals);
            }
            0x36..=0x39 | 0x3b..=0x4a => { let _ = pop(&mut stack); }
            0xbb => {
                let index = u16_at(&instruction.operands, 0).unwrap_or(0);
                let name = class.class_name(index).unwrap_or("java/lang/Object").to_owned();
                stack.push(Value::New(name));
            }
            0x59 => {
                if let Some(value) = stack.last().cloned() { stack.push(value); }
            }
            0x5a => {
                if stack.len() >= 2 {
                    let first = pop(&mut stack);
                    let second = pop(&mut stack);
                    stack.push(first.clone());
                    stack.push(second);
                    stack.push(first);
                }
            }
            0x5f => {
                if stack.len() >= 2 {
                    let len = stack.len();
                    stack.swap(len - 1, len - 2);
                }
            }
            0xb2 => {
                let index = u16_at(&instruction.operands, 0).unwrap_or(0);
                let field = class.member_reference(index)
                    .map(|(field_owner, name, _)| Value::Field(field_key(&field_owner, &name)))
                    .unwrap_or(Value::Unknown);
                stack.push(field);
            }
            0xb3 => {
                let value = pop(&mut stack);
                let index = u16_at(&instruction.operands, 0).unwrap_or(0);
                if let Some((field_owner, name, _)) = class.member_reference(index) {
                    let class_name = match value {
                        Value::New(name) | Value::Object(name) => Some(simple(&name)),
                        _ => None,
                    };
                    if let Some(class_name) = class_name {
                        output.insert(field_key(&field_owner, &name), class_name);
                    }
                }
            }
            0xb4 => { let _ = pop(&mut stack); stack.push(Value::Unknown); }
            0xb5 => { let _ = pop(&mut stack); let _ = pop(&mut stack); }
            0xb6..=0xb9 => {
                let index = u16_at(&instruction.operands, 0).unwrap_or(0);
                let Some((_, name, descriptor)) = class.member_reference(index) else { continue };
                let Some(parsed) = parse_method_descriptor(&descriptor).ok() else { continue };
                let _args = pop_n(&mut stack, parsed.parameters.len());
                if op != 0xb8 { let _receiver = pop(&mut stack); }
                if name != "<init>" && !matches!(parsed.return_type, crate::bytecode::JvmType::Void) {
                    stack.push(Value::Unknown);
                }
            }
            0xba => {
                let lambda = lambda_class(class, instruction, parsed_classes);
                let (_, _, descriptor) = match u16_at(&instruction.operands, 0)
                    .and_then(|index| class.invoke_dynamic(index)) {
                    Some(value) => value,
                    None => continue,
                };
                let count = parse_method_descriptor(&descriptor).map(|desc| desc.parameters.len()).unwrap_or(0);
                let _ = pop_n(&mut stack, count);
                stack.push(lambda.map(Value::Lambda).unwrap_or(Value::Unknown));
            }
            0x57 => { let _ = pop(&mut stack); }
            0x58 => { let _ = pop(&mut stack); let _ = pop(&mut stack); }
            0xc0 | 0xc1 => {}
            0xac..=0xb1 | 0xbf => break,
            _ => {}
        }
    }
    let _ = owner;
    output
}

fn record_registration_call(
    class: &ClassFile,
    instruction: &Instruction,
    method_name: &str,
    args: &[Value],
    field_classes: &BTreeMap<String, String>,
    output: &mut BTreeMap<String, BTreeSet<String>>,
) {
    if !matches!(method_name, "registerItem" | "registerBlock" | "register" | "registerModEntity" | "registerGlobalEntityID") {
        return;
    }
    let registry = args.iter().find_map(|value| {
        if let Value::Text(text) = value { normalize_registry(text) } else { None }
    });
    let Some(registry) = registry else { return };

    let mut class_name = args.iter().find_map(|value| match value {
        Value::Class(name) | Value::New(name) | Value::Object(name) | Value::Lambda(name) => Some(simple(name)),
        Value::Field(field) => field_classes.get(field).cloned(),
        _ => None,
    });
    if class_name.is_none() {
        // A field can be referenced just before the invocation but may have
        // been popped into a local/temporary stack value unsupported by this
        // conservative interpreter. Check the nearby GETSTATIC instructions.
        let _ = class;
        let _ = instruction;
    }
    if let Some(class_name) = class_name.take() {
        output.entry(class_name).or_default().insert(registry);
    }
}

fn scan_registrations(
    class: &ClassFile,
    field_classes: &BTreeMap<String, String>,
    parsed_classes: &HashMap<String, Arc<ClassFile>>,
    output: &mut BTreeMap<String, BTreeSet<String>>,
) {
    for method in &class.methods {
        let Some(code) = method.code.as_ref() else { continue };
        let Ok(instructions) = decode_instructions(&code.code) else { continue };
        let mut stack = Vec::<Value>::new();
        let mut locals = vec![Value::Unknown; code.max_locals as usize];
        for instruction in &instructions {
            let op = instruction.opcode;
            if (0x12..=0x14).contains(&op) || (0x01..=0x11).contains(&op) {
                stack.push(load_constant(class, instruction));
                continue;
            }
            match op {
                0x00 => {}
                0x19 | 0x2a..=0x2d => stack.push(
                    load_reference_local(op, &instruction.operands, &locals).unwrap_or(Value::Unknown)
                ),
                0x15..=0x18 | 0x1a..=0x29 => stack.push(Value::Unknown),
                0x3a | 0x4b..=0x4e => {
                    let value = pop(&mut stack);
                    store_reference_local(op, &instruction.operands, value, &mut locals);
                }
                0x36..=0x39 | 0x3b..=0x4a => { let _ = pop(&mut stack); }
                0xbb => {
                    let index = u16_at(&instruction.operands, 0).unwrap_or(0);
                    let name = class.class_name(index).unwrap_or("java/lang/Object").to_owned();
                    stack.push(Value::New(name));
                }
                0x59 => {
                    if let Some(value) = stack.last().cloned() { stack.push(value); }
                }
                0x5a => {
                    if stack.len() >= 2 {
                        let first = pop(&mut stack);
                        let second = pop(&mut stack);
                        stack.push(first.clone());
                        stack.push(second);
                        stack.push(first);
                    }
                }
                0x5f => {
                    if stack.len() >= 2 {
                        let len = stack.len();
                        stack.swap(len - 1, len - 2);
                    }
                }
                0xb2 => {
                    let index = u16_at(&instruction.operands, 0).unwrap_or(0);
                    let field = class.member_reference(index)
                        .map(|(owner, name, _)| Value::Field(field_key(&owner, &name)))
                        .unwrap_or(Value::Unknown);
                    stack.push(field);
                }
                0xb3 | 0x57 => { let _ = pop(&mut stack); }
                0xb4 => { let _ = pop(&mut stack); stack.push(Value::Unknown); }
                0xb5 => { let _ = pop(&mut stack); let _ = pop(&mut stack); }
                0xb6..=0xb9 => {
                    let index = u16_at(&instruction.operands, 0).unwrap_or(0);
                    let Some((_, name, descriptor)) = class.member_reference(index) else { continue };
                    let Ok(parsed) = parse_method_descriptor(&descriptor) else { continue };
                    let args = pop_n(&mut stack, parsed.parameters.len());
                    if name != "<init>" && op != 0xb8 { let _receiver = pop(&mut stack); }
                    if name == "<init>" {
                        // Constructor has consumed the uninitialized reference; a
                        // duplicate left beneath it retains the concrete class.
                    } else {
                        record_registration_call(class, instruction, &name, &args, field_classes, output);
                        if !matches!(parsed.return_type, crate::bytecode::JvmType::Void) {
                            stack.push(Value::Unknown);
                        }
                    }
                }
                0xba => {
                    let lambda = lambda_class(class, instruction, parsed_classes);
                    let descriptor = u16_at(&instruction.operands, 0)
                        .and_then(|index| class.invoke_dynamic(index))
                        .map(|(_, _, descriptor)| descriptor);
                    let count = descriptor.and_then(|value| parse_method_descriptor(&value).ok())
                        .map(|desc| desc.parameters.len()).unwrap_or(0);
                    let _ = pop_n(&mut stack, count);
                    stack.push(lambda.map(Value::Lambda).unwrap_or(Value::Unknown));
                }
                0xac..=0xb1 | 0xbf => break,
                _ => {}
            }
        }
    }
}

const MAX_ARCHIVE_CLASS_BYTES: u64 = 4 * 1024 * 1024 * 1024;

fn extract_archive_classes(
    archive_path: &Path,
    destination: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<()> {
    let file = File::open(archive_path)
        .with_context(|| format!("Opening compiled mod archive {}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("Reading compiled mod archive {}", archive_path.display()))?;
    if archive.len() > crate::archive::MAX_ARCHIVE_ENTRIES {
        anyhow::bail!(
            "Archive has {} entries; limit is {}",
            archive.len(),
            crate::archive::MAX_ARCHIVE_ENTRIES
        );
    }

    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().replace('\\', "/");
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".class") || lower.starts_with("meta-inf/versions/") {
            continue;
        }
        let relative = entry.enclosed_name().context("Unsafe class path in archive")?.to_path_buf();
        let declared = entry.size();
        if declared > MAX_ARCHIVE_CLASS_BYTES.saturating_sub(expanded) {
            anyhow::bail!("Compiled class content exceeds the {} byte limit", MAX_ARCHIVE_CLASS_BYTES);
        }
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&target)?;
        let written = std::io::copy(&mut entry, &mut file)?;
        if written != declared {
            anyhow::bail!(
                "Truncated class entry in {} (expected {}, wrote {})",
                archive_path.display(),
                declared,
                written
            );
        }
        expanded = expanded.saturating_add(written);
        output.push(target);
    }
    Ok(())
}

/// Build a native class/registry index from .class files alone. It handles
/// legacy GameRegistry calls, static content-holder fields, modern DeferredRegister
/// Supplier lambdas, and old entity registration APIs. Decompiled sources are
/// merged by each exporter and retain precedence over inferred names.
pub fn scan_compiled_registrations(roots: &[PathBuf]) -> Result<CompiledRegistryIndex> {
    let mut paths = Vec::<PathBuf>::new();
    let mut archives = Vec::<PathBuf>::new();
    for root in roots {
        if root.is_dir() {
            for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|value| value.ok()) {
                if !entry.file_type().is_file() { continue; }
                match entry.path().extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
                    "class" => paths.push(entry.into_path()),
                    "jar" | "zip" => archives.push(entry.into_path()),
                    _ => {}
                }
            }
        } else if root.is_file() {
            match root.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
                "class" => paths.push(root.clone()),
                "jar" | "zip" => archives.push(root.clone()),
                _ => {}
            }
        }
    }
    paths.sort();
    paths.dedup();
    archives.sort();
    archives.dedup();

    // Several MCreator exporters consume the same compiled index in one
    // export-all run. Cache by canonical roots plus each class/archive file's
    // size and modification timestamp so scans are reused without going stale.
    let mut hasher = DefaultHasher::new();
    let cache_key = roots.iter().map(|root| {
        fs::canonicalize(root).unwrap_or_else(|_| root.clone()).to_string_lossy().into_owned()
    }).collect::<Vec<_>>().join("|");
    cache_key.hash(&mut hasher);
    for path in paths.iter().chain(archives.iter()) {
        path.hash(&mut hasher);
        if let Ok(metadata) = fs::metadata(path) {
            metadata.len().hash(&mut hasher);
            if let Ok(modified) = metadata.modified() {
                if let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH) {
                    elapsed.as_nanos().hash(&mut hasher);
                }
            }
        }
    }
    let fingerprint = hasher.finish();
    let cache = REGISTRY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some((cached_fingerprint, cached_index)) = guard.get(&cache_key) {
            if *cached_fingerprint == fingerprint {
                return Ok(cached_index.clone());
            }
        }
    }

    let mut index = CompiledRegistryIndex::default();
    // Archive extraction is independent by input JAR. Extract concurrently into
    // isolated temp directories, then merge in sorted archive order so warning
    // order and duplicate-class precedence remain deterministic.
    let extracted_archives: Vec<Result<(PathBuf, Arc<tempfile::TempDir>, Vec<PathBuf>), (PathBuf, String)>> =
        archives.par_iter().map(|archive_path| {
            let extraction = match Builder::new().prefix("mojmodelbridge-classes-").tempdir() {
                Ok(value) => Arc::new(value),
                Err(error) => {
                    return Err((
                        archive_path.clone(),
                        format!("Could not create temporary class extraction: {error}"),
                    ));
                }
            };
            let mut extracted_paths = Vec::new();
            match extract_archive_classes(archive_path, extraction.path(), &mut extracted_paths) {
                Ok(()) => Ok((archive_path.clone(), extraction, extracted_paths)),
                Err(error) => Err((
                    archive_path.clone(),
                    format!("Could not index compiled classes: {error}"),
                )),
            }
        }).collect();

    for result in extracted_archives {
        match result {
            Ok((_archive_path, extraction, mut extracted_paths)) => {
                paths.append(&mut extracted_paths);
                index.archive_extractions.push(extraction);
            }
            Err((archive_path, error)) => {
                index.warnings.push(format!("{} for {}", error, archive_path.display()));
            }
        }
    }
    paths.sort();
    paths.dedup();

    // Class-file decoding is independent per file and is the CPU-heavy portion
    // of large legacy mod scans. Parse in parallel, then merge in sorted path
    // order so duplicate-class precedence and diagnostics remain deterministic.
    let parsed_results: Vec<(PathBuf, Result<ClassFile>)> = paths.par_iter()
        .map(|path| {
            let parsed = fs::read(path)
                .with_context(|| format!("Reading compiled class {}", path.display()))
                .and_then(|bytes| ClassFile::parse(&bytes));
            (path.clone(), parsed)
        })
        .collect();

    let mut parsed = HashMap::<String, Arc<ClassFile>>::new();
    let mut parsed_class_paths = HashMap::<PathBuf, Arc<ClassFile>>::new();
    for (path, result) in parsed_results {
        let class = match result {
            Ok(value) => value,
            Err(error) => {
                index.warnings.push(format!("Skipped unreadable class {}: {error}", path.display()));
                continue;
            }
        };
        let internal = class.this_name().unwrap_or("").to_owned();
        if internal.is_empty() { continue; }
        let name = simple(&internal);
        let superclass = class.super_name().map(|value| simple(value)).unwrap_or_default();
        let info = CompiledClassInfo {
            class_name: name.clone(),
            superclass,
            access_flags: class.access_flags,
            path: path.clone(),
        };
        let replace = index.classes.get(&name).map(|old| {
            old.path.file_stem().is_some_and(|stem| stem.to_string_lossy().contains('$'))
                && !path.file_stem().is_some_and(|stem| stem.to_string_lossy().contains('$'))
        }).unwrap_or(true);
        if replace { index.classes.insert(name, info); }
        let parsed_class = Arc::new(class);
        parsed.insert(internal, Arc::clone(&parsed_class));
        parsed_class_paths.insert(path, parsed_class);
    }

    // The class-local initializer and registration passes are independent once
    // the complete parsed-class map is available. Use sorted keys with Rayon,
    // then merge in that same order for deterministic duplicate resolution.
    let mut parsed_entries: Vec<_> = parsed.iter().collect();
    parsed_entries.sort_by(|a, b| a.0.cmp(b.0));

    let field_results: Vec<BTreeMap<String, String>> = parsed_entries.par_iter()
        .map(|(_, class)| {
            let class = class.as_ref();
            static_field_assignments(class, &parsed)
        })
        .collect();
    let mut fields = BTreeMap::<String, String>::new();
    for assignments in field_results {
        fields.extend(assignments);
    }
    index.field_classes = fields.clone();

    let registration_results: Vec<BTreeMap<String, BTreeSet<String>>> = parsed_entries.par_iter()
        .map(|(_, class)| {
            let class = class.as_ref();
            let mut registrations = BTreeMap::new();
            scan_registrations(class, &fields, &parsed, &mut registrations);
            registrations
        })
        .collect();
    for registrations in registration_results {
        for (class_name, registry_names) in registrations {
            index.registries.entry(class_name).or_default().extend(registry_names);
        }
    }
    index.parsed_classes = parsed.clone();
    index.parsed_class_paths = parsed_class_paths;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(cache_key, (fingerprint, index.clone()));
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::FileOptions;

    #[test]
    fn registry_normalization_rejects_noise_and_preserves_paths() {
        assert_eq!(normalize_registry("ExampleMod:Ruby-Sword"), Some("ruby_sword".to_owned()));
        assert_eq!(normalize_registry("assets/example/ruby.png"), None);
        assert_eq!(normalize_registry(""), None);
    }

    #[test]
    fn resolves_deferred_register_constructor_method_references() {
        let class = ClassFile {
            minor_version: 0,
            major_version: 61,
            access_flags: 0,
            this_class: 2,
            super_class: 0,
            interfaces: Vec::new(),
            constant_pool: vec![
                None,
                Some(ConstantPoolEntry::Utf8("RubyItem".to_owned())),
                Some(ConstantPoolEntry::Class { name_index: 1 }),
                Some(ConstantPoolEntry::Utf8("<init>".to_owned())),
                Some(ConstantPoolEntry::Utf8("()V".to_owned())),
                Some(ConstantPoolEntry::NameAndType { name_index: 3, descriptor_index: 4 }),
                Some(ConstantPoolEntry::Methodref { class_index: 2, name_and_type_index: 5 }),
                Some(ConstantPoolEntry::MethodHandle { reference_kind: 8, reference_index: 6 }),
                Some(ConstantPoolEntry::Utf8("get".to_owned())),
                Some(ConstantPoolEntry::Utf8("()Ljava/util/function/Supplier;".to_owned())),
                Some(ConstantPoolEntry::NameAndType { name_index: 8, descriptor_index: 9 }),
                Some(ConstantPoolEntry::InvokeDynamic { bootstrap_method_attr_index: 0, name_and_type_index: 10 }),
            ],
            fields: Vec::new(),
            methods: Vec::new(),
            attributes: Vec::new(),
            bootstrap_methods: vec![crate::bytecode::classfile::BootstrapMethod {
                method_ref: 0,
                arguments: vec![7],
            }],
        };
        let instruction = Instruction {
            offset: 0,
            opcode: 0xba,
            mnemonic: "invokedynamic",
            operands: vec![0, 11, 0, 0],
        };
        assert_eq!(
            lambda_class(&class, &instruction, &HashMap::new()).as_deref(),
            Some("RubyItem"),
        );
    }

    #[test]
    fn recovers_block_registrations_when_registry_values_are_stored_in_locals() {
        let class = ClassFile {
            minor_version: 0,
            major_version: 52,
            access_flags: 0,
            this_class: 2,
            super_class: 0,
            interfaces: Vec::new(),
            constant_pool: vec![
                None,
                Some(ConstantPoolEntry::Utf8("com/example/Registry".to_owned())),
                Some(ConstantPoolEntry::Class { name_index: 1 }),
                Some(ConstantPoolEntry::Utf8("com/example/BlockRegistry".to_owned())),
                Some(ConstantPoolEntry::Class { name_index: 3 }),
                Some(ConstantPoolEntry::Utf8("RUBY".to_owned())),
                Some(ConstantPoolEntry::Utf8("Lnet/minecraft/block/Block;".to_owned())),
                Some(ConstantPoolEntry::NameAndType { name_index: 5, descriptor_index: 6 }),
                Some(ConstantPoolEntry::Fieldref { class_index: 4, name_and_type_index: 7 }),
                Some(ConstantPoolEntry::Utf8("registerBlock".to_owned())),
                Some(ConstantPoolEntry::Utf8("(Lnet/minecraft/block/Block;Ljava/lang/String;)V".to_owned())),
                Some(ConstantPoolEntry::NameAndType { name_index: 9, descriptor_index: 10 }),
                Some(ConstantPoolEntry::Utf8("net/minecraftforge/common/registry/GameRegistry".to_owned())),
                Some(ConstantPoolEntry::Class { name_index: 12 }),
                Some(ConstantPoolEntry::Methodref { class_index: 13, name_and_type_index: 11 }),
                Some(ConstantPoolEntry::Utf8("ruby_ore".to_owned())),
                Some(ConstantPoolEntry::String { string_index: 15 }),
            ],
            fields: Vec::new(),
            methods: vec![crate::bytecode::classfile::MemberInfo {
                access_flags: 0x0008,
                name: "register".to_owned(),
                descriptor: "()V".to_owned(),
                attributes: Vec::new(),
                code: Some(crate::bytecode::classfile::CodeAttribute {
                    max_stack: 2,
                    max_locals: 3,
                    code: vec![
                        0xb2, 0x00, 0x08, // getstatic BlockRegistry.RUBY
                        0x4c,             // astore_1
                        0x12, 0x10,       // ldc "ruby_ore"
                        0x4d,             // astore_2
                        0x2b,             // aload_1
                        0x2c,             // aload_2
                        0xb8, 0x00, 0x0e, // invokestatic GameRegistry.registerBlock
                        0xb1,             // return
                    ],
                    ..Default::default()
                }),
            }],
            attributes: Vec::new(),
            bootstrap_methods: Vec::new(),
        };
        let fields = BTreeMap::from([(
            "com/example/BlockRegistry#RUBY".to_owned(),
            "RubyBlock".to_owned(),
        )]);
        let mut output = BTreeMap::new();

        scan_registrations(&class, &fields, &HashMap::new(), &mut output);

        assert_eq!(
            output.get("RubyBlock").and_then(|ids| ids.iter().next()).map(String::as_str),
            Some("ruby_ore"),
        );
    }

    #[test]
    fn indexes_compiled_registrations_directly_from_a_mod_jar() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("compiled-only-mod.jar");
        {
            let file = File::create(&archive_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer.start_file(
                "com/example/LegacyEntityRegistry.class",
                FileOptions::default(),
            ).unwrap();
            writer.write_all(include_bytes!("../tests/fixtures/LegacyEntityRegistry.class")).unwrap();
            writer.finish().unwrap();
        }

        let index = scan_compiled_registrations(&[archive_path]).unwrap();
        assert_eq!(
            index.registries.get("EntityRedAnt").and_then(|values| values.iter().next()).map(String::as_str),
            Some("red_ant"),
        );
        assert!(
            index.parsed_classes.values().any(|class| {
                class.this_name().is_some_and(|name| {
                    let simple = name.rsplit('/').next().unwrap_or(name);
                    simple.rsplit('$').next().unwrap_or(simple) == "LegacyEntityRegistry"
                })
            }),
            "the shared index retains the parsed class for downstream stat exporters"
        );
        let class = index.classes.get("LegacyEntityRegistry").unwrap();
        assert!(class.path.is_file());
        assert_eq!(index.archive_extractions.len(), 1);
    }

    #[test]
    fn reads_class_headers_and_class_literal_registrations_without_java_sources() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("LegacyEntityRegistry.class");
        fs::write(
            &fixture,
            include_bytes!("../tests/fixtures/LegacyEntityRegistry.class"),
        ).unwrap();
        let index = scan_compiled_registrations(&[temp.path().to_path_buf()]).unwrap();
        assert!(index.classes.contains_key("LegacyEntityRegistry"));
        assert_eq!(
            index.registries.get("EntityRedAnt").and_then(|values| values.iter().next()).map(String::as_str),
            Some("red_ant"),
        );
    }

    #[test]
    fn field_keys_include_owner_to_avoid_name_collisions() {
        assert_ne!(field_key("example/Items", "SWORD"), field_key("example/Blocks", "SWORD"));
    }
}
