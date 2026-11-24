use anyhow::{Context, Result};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct ProjectileSource {
    pub class_name: String,
    pub superclass: String,
    pub registry_name: String,
    pub source_path: String,
    #[serde(skip)]
    pub source_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectileExportRecord {
    pub class_name: String,
    pub element_name: String,
    pub registry_name: String,
    pub bound_item: Option<String>,
    pub ranged_items_updated: usize,
    pub source_path: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectileWorkspaceExportReport {
    pub workspace_file: PathBuf,
    pub discovered: usize,
    pub exported: usize,
    pub skipped: usize,
    pub ranged_items_updated: usize,
    pub records: Vec<ProjectileExportRecord>,
    pub warnings: Vec<String>,
}

fn snake(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower) {
                output.push('_');
            }
        }
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else if !output.is_empty() && !output.ends_with('_') {
            output.push('_');
        }
    }
    let output = output.trim_matches('_').to_owned();
    if output.is_empty() { "converted_projectile".to_owned() } else { output }
}

fn pascal(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::new();
    let mut upper = true;
    for ch in chars {
        if !ch.is_ascii_alphanumeric() {
            upper = true;
        } else if upper {
            output.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            output.push(ch);
        }
    }
    if output.is_empty() { output = "ConvertedProjectile".to_owned(); }
    if output.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        output.insert_str(0, "Projectile");
    }
    output
}

fn compact(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn workspace_descriptor(root: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::<(i64, usize, PathBuf)>::new();
    for entry in fs::read_dir(root).with_context(|| format!("Reading workspace {}", root.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("mcreator") { continue; }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
        if !value.get("workspaceSettings").is_some_and(Value::is_object)
            || !value.get("mod_elements").is_some_and(Value::is_array) { continue; }
        candidates.push((
            value.get("mcreatorVersion").and_then(Value::as_i64).unwrap_or(0),
            value.get("mod_elements").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            path,
        ));
    }
    candidates.sort_by_key(|(version,count,path)|(*version,*count,path.to_string_lossy().to_ascii_lowercase()));
    candidates.pop().map(|(_,_,path)|path)
        .ok_or_else(||anyhow::anyhow!("No valid .mcreator workspace descriptor found in {}",root.display()))
}

fn template_for(root: &Path) -> Result<Value> {
    let elements = root.join("elements");
    if elements.is_dir() {
        let mut paths: Vec<PathBuf> = fs::read_dir(elements)?.filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
            if value.get("_type").and_then(Value::as_str) == Some("projectile")
                && value.get("definition").is_some_and(Value::is_object) {
                return Ok(value);
            }
        }
    }
    serde_json::from_str(include_str!("../../oldVer/mcreator_templates/ProjectileSchema.mod.json"))
        .context("Parsing embedded MCreator projectile schema")
}

fn java_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in roots {
        if !root.is_dir() { continue; }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("java")) {
                files.push(entry.into_path());
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn strip_comments(source: &str) -> String {
    crate::source::java::strip_java_comments(source)
}

fn simple_name(value: &str) -> String {
    value.rsplit('.').next().unwrap_or(value).rsplit('$').next().unwrap_or(value).to_owned()
}

fn class_headers(source: &str) -> Vec<(String, String, bool)> {
    let regex = Regex::new(
        r"\b((?:(?:public|protected|private|static|final|abstract)\s+)*)class\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>{}]*>)?\s+extends\s+([A-Za-z_$][A-Za-z0-9_$.]*)"
    ).unwrap();
    regex.captures_iter(source).filter_map(|capture| {
        let modifiers = capture.get(1).map(|value| value.as_str()).unwrap_or("");
        Some((
            capture.get(2)?.as_str().to_owned(),
            simple_name(capture.get(3)?.as_str()),
            modifiers.split_whitespace().any(|part| part == "abstract"),
        ))
    }).collect()
}

fn is_projectile_super(superclass: &str, parents: &BTreeMap<String,String>, depth: usize) -> bool {
    if depth > 12 { return false; }
    let lower = simple_name(superclass).to_ascii_lowercase();
    if [
        "entityarrow","entitythrowable","entityprojectile","abstractarrow",
        "throwableitemprojectile","abstracthurtingprojectile","projectile",
        "arrow","throwableprojectile",
    ].iter().any(|token| lower == *token || lower.contains(token)) {
        return true;
    }
    parents.get(&simple_name(superclass))
        .is_some_and(|next|is_projectile_super(next,parents,depth+1))
}

fn source_instantiations(text: &str) -> BTreeSet<String> {
    let regex = Regex::new(r"\bnew\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\(").unwrap();
    regex.captures_iter(text)
        .filter_map(|capture| capture.get(1).map(|value|simple_name(value.as_str())))
        .collect()
}

/// Discover concrete Java projectile subclasses. Registry metadata wins; otherwise
/// the source class name is used only when another source class directly instantiates it.
pub fn discover_projectile_sources(source_roots: &[PathBuf]) -> Result<Vec<ProjectileSource>> {
    let files = java_files(source_roots);
    let mut class_texts = BTreeMap::<String,String>::new();
    let mut class_paths = BTreeMap::<String,PathBuf>::new();
    let mut parents = BTreeMap::<String,String>::new();
    let mut abstract_classes = HashSet::<String>::new();
    let mut combined = String::new();

    for path in &files {
        let Ok(bytes) = fs::read(path) else { continue };
        let text = strip_comments(&String::from_utf8_lossy(&bytes));
        combined.push_str(&text);
        combined.push('\n');
        for (class,parent,is_abstract) in class_headers(&text) {
            class_texts.entry(class.clone()).or_insert_with(||text.clone());
            class_paths.entry(class.clone()).or_insert_with(||path.clone());
            parents.insert(class.clone(),parent);
            if is_abstract { abstract_classes.insert(class); }
        }
    }

    let instantiated = source_instantiations(&combined);
    let mut registry_names = BTreeMap::<String,String>::new();
    if let Some(root) = source_roots.first() {
        if let Ok(graph) = crate::mod_graph::scan_mod_graph(root,source_roots) {
            registry_names = graph.registry_names;
        }
    }

    let mut output = Vec::new();
    for (class,superclass) in &parents {
        if abstract_classes.contains(class) || !is_projectile_super(superclass,&parents,0) {
            continue;
        }
        let registered = registry_names.get(class).cloned().filter(|value|!value.trim().is_empty());
        let is_instantiated = instantiated.contains(class);
        let Some(source_text) = class_texts.get(class) else {continue};
        let Some(source_path) = class_paths.get(class) else {continue};
        if registered.is_none() && !is_instantiated {
            continue;
        }
        let registry_name = registered.unwrap_or_else(||snake(class));
        output.push(ProjectileSource {
            class_name:class.clone(),
            superclass:superclass.clone(),
            registry_name,
            source_path:source_path.to_string_lossy().replace('\\',"/"),
            source_text:source_text.clone(),
        });
    }
    output.sort_by(|a,b|a.registry_name.cmp(&b.registry_name).then(a.class_name.cmp(&b.class_name)));
    Ok(output)
}

fn projectile_sound(source: &str, template: &Value) -> String {
    let mappings = [
        ("SNOWBALL_THROW","entity.snowball.throw"),
        ("ENDER_PEARL_THROW","entity.ender_pearl.throw"),
        ("EGG_THROW","entity.egg.throw"),
        ("TRIDENT_THROW","item.trident.throw"),
        ("FIREWORK_ROCKET_LAUNCH","entity.firework_rocket.launch"),
        ("ARROW_SHOOT","entity.arrow.shoot"),
        ("BOW_SHOOT","entity.arrow.shoot"),
    ];
    for (constant,sound) in mappings {
        if source.contains(constant) {
            return sound.to_owned();
        }
    }
    template.get("definition").and_then(|value|value.get("actionSound"))
        .and_then(Value::as_str).unwrap_or("entity.arrow.shoot").to_owned()
}

fn projectile_display(registry: &str) -> String {
    registry.split(|ch:char|!ch.is_ascii_alphanumeric()).filter(|part|!part.is_empty())
        .map(|part|{
            let mut chars=part.chars();
            match chars.next(){
                Some(first)=>{
                    let mut value=first.to_ascii_uppercase().to_string();
                    value.extend(chars.map(|ch|ch.to_ascii_lowercase()));
                    value
                }
                None=>String::new()
            }
        }).collect::<Vec<_>>().join(" ")
}

fn unique_element_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut name=base.to_owned();
    let mut suffix=2usize;
    while !used.insert(name.to_ascii_lowercase()){
        name=format!("{base}{suffix}");
        suffix+=1;
    }
    name
}

fn source_class_text<'a>(java: &'a [(String,String)], class_name: &str) -> Option<&'a str> {
    let simple = simple_name(class_name);
    let declaration = Regex::new(&format!(r"\bclass\s+{}\b",Regex::escape(&simple))).ok()?;
    java.iter()
        .find(|(file_name,text)|file_name.eq_ignore_ascii_case(&simple)||declaration.is_match(text))
        .map(|(_,text)|text.as_str())
}

fn binding_items(
    workspace: &Value,
    source_roots: &[PathBuf],
    source_modid: &str,
    projectiles: &[ProjectileSource],
) -> BTreeMap<String,String> {
    let Some(elements)=workspace.get("mod_elements").and_then(Value::as_array) else {return BTreeMap::new()};
    let field_map=crate::recipe_bridge::source_field_registered_names(source_roots,source_modid);
    let mut item_by_registry=BTreeMap::<String,String>::new();
    let mut item_source_classes=Vec::<(String,String)>::new();
    for item in elements {
        let kind=item.get("type").and_then(Value::as_str).unwrap_or("");
        if kind!="item" && kind!="tool" {continue;}
        let registry=item.get("registry_name").and_then(Value::as_str).unwrap_or("");
        let name=item.get("name").and_then(Value::as_str).unwrap_or("");
        if !registry.is_empty()&&!name.is_empty(){
            item_by_registry.insert(compact(registry),name.to_owned());
        }
        let source_class=item.get("mojmodelbridge_source_class").and_then(Value::as_str).unwrap_or("");
        if !source_class.is_empty()&&!name.is_empty(){
            item_source_classes.push((simple_name(source_class),name.to_owned()));
        }
    }

    let all_java:Vec<(String,String)>=java_files(source_roots).into_iter().filter_map(|path|{
        let stem=path.file_stem()?.to_str()?.to_owned();
        let bytes=fs::read(path).ok()?;
        Some((stem,strip_comments(&String::from_utf8_lossy(&bytes))))
    }).collect();

    let mut result=BTreeMap::new();
    for projectile in projectiles {
        let mut candidates=Vec::<(i32,String)>::new();
        for (field,registries) in &field_map {
            if !Regex::new(&format!(r"\b{}\b",regex::escape(field))).unwrap().is_match(&projectile.source_text) {
                continue;
            }
            for registry in registries {
                if let Some(item)=item_by_registry.get(&compact(registry)){
                    candidates.push((200,item.clone()));
                }
            }
        }

        let constructor=Regex::new(&format!(r"\bnew\s+{}\s*\(",regex::escape(&projectile.class_name))).unwrap();
        for (source_class,item_name) in &item_source_classes {
            if let Some(text)=source_class_text(&all_java,source_class) {
                if constructor.is_match(text){
                    candidates.push((100,item_name.clone()));
                }
            }
        }
        candidates.sort_by(|a,b|b.0.cmp(&a.0).then(a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase())));
        if let Some((_,item))=candidates.into_iter().next(){
            result.insert(projectile.class_name.clone(),item);
        }
    }
    result
}

fn update_ranged_items(
    workspace_root: &Path,
    workspace: &Value,
    source_roots: &[PathBuf],
    projectiles: &[ProjectileSource],
    projectile_elements: &BTreeMap<String,String>,
) -> Result<usize> {
    let java:Vec<(String,String)>=java_files(source_roots).into_iter().filter_map(|path|{
        let stem=path.file_stem()?.to_str()?.to_owned();
        let bytes=fs::read(path).ok()?;
        Some((stem,strip_comments(&String::from_utf8_lossy(&bytes))))
    }).collect();
    let Some(descriptors)=workspace.get("mod_elements").and_then(Value::as_array) else {return Ok(0)};
    let mut updated=0usize;

    for item in descriptors {
        let kind=item.get("type").and_then(Value::as_str).unwrap_or("");
        if kind!="item" && kind!="tool"{continue;}
        let element_name=item.get("name").and_then(Value::as_str).unwrap_or("");
        let source_class=item.get("mojmodelbridge_source_class").and_then(Value::as_str).unwrap_or("");
        if element_name.is_empty()||source_class.is_empty(){continue;}
        let path=workspace_root.join("elements").join(format!("{element_name}.mod.json"));
        let Ok(bytes)=fs::read(&path) else {continue};
        let Ok(mut element)=serde_json::from_slice::<Value>(&bytes) else {continue};
        let Some(definition)=element.get_mut("definition").and_then(Value::as_object_mut) else {continue};
        if definition.get("enableRanged").and_then(Value::as_bool)!=Some(true){continue;}
        let Some(source_text)=source_class_text(&java,source_class) else {continue};
        for projectile in projectiles {
            let regex=Regex::new(&format!(r"\bnew\s+{}\s*\(",regex::escape(&projectile.class_name))).unwrap();
            if regex.is_match(source_text) {
                if let Some(projectile_element)=projectile_elements.get(&projectile.class_name){
                    definition.insert("projectile".to_owned(),json!(format!("CUSTOM:{projectile_element}")));
                    fs::write(&path,format!("{}\n",serde_json::to_string_pretty(&element)?))
                        .with_context(||format!("Writing ranged item binding {}",path.display()))?;
                    updated+=1;
                    break;
                }
            }
        }
    }
    Ok(updated)
}

/// Export custom projectile classes and connect their pickup items and ranged-item users.
pub fn export_projectiles_to_workspace(
    source_roots:&[PathBuf],
    workspace_root:&Path,
    source_modid:&str,
    imported_fv:i64,
)->Result<ProjectileWorkspaceExportReport>{
    let workspace_root=fs::canonicalize(workspace_root).unwrap_or_else(|_|workspace_root.to_path_buf());
    let workspace_file=workspace_descriptor(&workspace_root)?;
    let bytes=fs::read(&workspace_file).with_context(||format!("Reading {}",workspace_file.display()))?;
    let mut workspace:Value=serde_json::from_slice(&bytes).context("Parsing MCreator workspace descriptor")?;
    let import_modid=if !source_modid.trim().is_empty() {
        source_modid.trim().to_owned()
    } else {
        source_roots.first().and_then(|root|root.file_name()).and_then(|value|value.to_str())
            .unwrap_or("converted_mod").to_owned()
    };
    let target_modid=workspace.get("workspaceSettings").and_then(|v|v.get("modid")).and_then(Value::as_str)
        .filter(|v|!v.trim().is_empty()).unwrap_or("converted_mod").to_owned();
    let projectiles=discover_projectile_sources(source_roots)?;
    let template=template_for(&workspace_root)?;
    let bindings=binding_items(&workspace,source_roots,&import_modid,&projectiles);

    let mut report=ProjectileWorkspaceExportReport{
        workspace_file:workspace_file.clone(),discovered:projectiles.len(),exported:0,skipped:0,
        ranged_items_updated:0,records:Vec::new(),warnings:Vec::new()
    };
    let mut used=HashSet::<String>::new();
    let mut existing=BTreeSet::<String>::new();
    let mut class_to_element=BTreeMap::<String,String>::new();
    for item in workspace.get("mod_elements").and_then(Value::as_array).into_iter().flatten(){
        if item.get("type").and_then(Value::as_str)==Some("projectile"){
            existing.insert(item.get("registry_name").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase());
            if let (Some(source_class),Some(name))=(
                item.get("mojmodelbridge_source_class").and_then(Value::as_str),
                item.get("name").and_then(Value::as_str)
            ) {
                class_to_element.insert(simple_name(source_class),name.to_owned());
            }
        }
        if let Some(name)=item.get("name").and_then(Value::as_str){used.insert(name.to_ascii_lowercase());}
    }

    let output_elements=workspace_root.join("elements");
    fs::create_dir_all(&output_elements)?;
    for source in &projectiles {
        let registry=source.registry_name.to_ascii_lowercase();
        if existing.contains(&registry){report.skipped+=1;continue;}
        let element_base=pascal(&source.class_name);
        let element_name=unique_element_name(&element_base,&mut used);
        let bound_item=bindings.get(&source.class_name).cloned();
        let mut definition=template.get("definition").and_then(Value::as_object).cloned().unwrap_or_default();
        definition.insert("projectileItem".to_owned(),json!(bound_item.as_ref().map(|name|format!("CUSTOM:{name}")).unwrap_or_default()));
        definition.insert("customModelTexture".to_owned(),json!(""));
        definition.insert("modelWidth".to_owned(),json!(0.5));
        definition.insert("modelHeight".to_owned(),json!(0.5));
        definition.insert("actionSound".to_owned(),json!(projectile_sound(&source.source_text,&template)));
        if source.source_text.contains("noGravity")||source.source_text.contains("setNoGravity(true)"){
            definition.insert("disableGravity".to_owned(),json!(true));
        }
        if source.source_text.contains("setNoDespawn")||source.source_text.contains("persistent"){
            definition.insert("disableDiscarding".to_owned(),json!(true));
        }
        let element=json!({"_fv":template.get("_fv").and_then(Value::as_i64).unwrap_or(imported_fv).clamp(0,89),"_type":"projectile","definition":definition});
        let path=output_elements.join(format!("{element_name}.mod.json"));
        fs::write(&path,format!("{}\n",serde_json::to_string_pretty(&element)?))
            .with_context(||format!("Writing projectile element {}",path.display()))?;
        workspace["mod_elements"].as_array_mut().expect("workspace has mod_elements").push(json!({
            "name":element_name,
            "type":"projectile",
            "compiles":true,
            "locked_code":false,
            "registry_name":registry,
            "metadata":{"files":[]},
            "path":format!("~/{}/Projectiles",import_modid.replace('-',"_")),
            "mojmodelbridge_generated":true,
            "mojmodelbridge_source_modid":import_modid,
            "mojmodelbridge_source_class":source.class_name,
            "mojmodelbridge_source_path":source.source_path
        }));
        class_to_element.insert(source.class_name.clone(),element_name.clone());
        let mut warnings=Vec::new();
        if let Some(item)=bound_item.as_ref(){
            warnings.push(format!("Projectile pickup item was bound to '{item}' from source references; verify pickup behavior."));
        } else {
            warnings.push("No source item/tool binding could be recovered for this projectile.".to_owned());
            report.warnings.push(format!("{}: no source item/tool binding could be recovered.",source.class_name));
        }
        report.exported+=1;
        report.records.push(ProjectileExportRecord{
            class_name:source.class_name.clone(),element_name,registry_name:registry,
            bound_item,ranged_items_updated:0,source_path:source.source_path.clone(),warnings
        });
    }

    report.ranged_items_updated=update_ranged_items(&workspace_root,&workspace,source_roots,&projectiles,&class_to_element)?;
    if report.ranged_items_updated>0{
        report.warnings.push(format!("Bound {} ranged item(s) to the recovered custom projectile classes.",report.ranged_items_updated));
    }
    if !workspace.get("language_map").is_some_and(Value::is_object){workspace["language_map"]=json!({});}
    if !workspace["language_map"].get("en_us").is_some_and(Value::is_object){workspace["language_map"]["en_us"]=json!({});}
    let language=workspace["language_map"]["en_us"].as_object_mut().unwrap();
    for record in &report.records{
        language.insert(format!("projectile.{target_modid}.{}",record.registry_name),json!(projectile_display(&record.registry_name)));
    }

    fs::write(&workspace_file,format!("{}\n",serde_json::to_string_pretty(&workspace)?))
        .with_context(||format!("Writing projectile workspace {}",workspace_file.display()))?;
    Ok(report)
}

#[cfg(test)]
mod tests{
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn discovers_instantiated_legacy_arrow_subclasses_without_entity_registry(){
        let temp=tempdir().unwrap();
        let root=temp.path();
        fs::write(root.join("RubyArrow.java"),r#"
            public class RubyArrow extends EntityArrow { }
        "#).unwrap();
        fs::write(root.join("ExampleBow.java"),r#"
            public class ExampleBow extends ItemBow {
                public void releaseUsing(){ new RubyArrow(); }
            }
        "#).unwrap();
        let found=discover_projectile_sources(&[root.to_path_buf()]).unwrap();
        assert_eq!(found.len(),1);
        assert_eq!(found[0].class_name,"RubyArrow");
        assert_eq!(found[0].registry_name,"ruby_arrow");
    }

    #[test]
    fn binds_projectile_pickup_item_and_updates_ranged_item_projectile_field(){
        let temp=tempdir().unwrap();
        let source=temp.path().join("source");
        let workspace=temp.path().join("workspace");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(source.join("ModItems.java"),r#"
            public class ModItems {
                public static Item RUBY_ARROW = new RubyArrowItem();
                public static void register(){ GameRegistry.registerItem(ModItems.RUBY_ARROW,"ruby_arrow"); }
            }
            class RubyArrow extends EntityArrow {
                public ItemStack getPickupItem(){ return new ItemStack(ModItems.RUBY_ARROW); }
            }
            class RubyArrowItem extends Item { }
            class ExampleBow extends ItemBow {
                public void releaseUsing(){ new RubyArrow(); }
            }
        "#).unwrap();
        let descriptor=json!({
            "mcreatorVersion":202600,
            "workspaceSettings":{"modid":"converted"},
            "mod_elements":[
                {"name":"RubyArrowItem","type":"item","registry_name":"ruby_arrow","mojmodelbridge_source_class":"RubyArrowItem","mojmodelbridge_source_modid":"demo"},
                {"name":"ExampleBow","type":"tool","registry_name":"example_bow","mojmodelbridge_source_class":"ExampleBow","mojmodelbridge_source_modid":"demo"}
            ]
        });
        fs::write(workspace.join("test.mcreator"),serde_json::to_vec(&descriptor).unwrap()).unwrap();
        fs::create_dir_all(workspace.join("elements")).unwrap();
        fs::write(workspace.join("elements/ExampleBow.mod.json"),serde_json::to_vec(&json!({
            "_fv":89,"_type":"tool","definition":{"enableRanged":true,"projectile":"Arrow"}
        })).unwrap()).unwrap();

        let report=export_projectiles_to_workspace(&[source],&workspace,"demo",89).unwrap();
        assert_eq!(report.exported,1);
        assert_eq!(report.records[0].bound_item.as_deref(),Some("RubyArrowItem"));
        assert_eq!(report.ranged_items_updated,1);
        let projectile:Value=serde_json::from_slice(&fs::read(workspace.join("elements/RubyArrow.mod.json")).unwrap()).unwrap();
        assert_eq!(projectile["definition"]["projectileItem"],"CUSTOM:RubyArrowItem");
        let bow:Value=serde_json::from_slice(&fs::read(workspace.join("elements/ExampleBow.mod.json")).unwrap()).unwrap();
        assert_eq!(bow["definition"]["projectile"],"CUSTOM:RubyArrow");
    }
}
