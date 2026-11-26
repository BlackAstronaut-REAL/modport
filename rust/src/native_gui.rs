//! Legacy GUI discovery for block-backed Forge containers.
//!
//! The scanner follows the old block -> openGui(id) -> IGuiHandler -> client
//! screen/server container chain. It records only explicit links; registry-name
//! similarity is never used to attach a GUI to a block.

use anyhow::Result;
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

use crate::source::java::extract_balanced;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeGuiSlot {
    pub index: usize,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeGuiPlan {
    pub key: String,
    pub source_gui_id: String,
    pub source_handler_class: String,
    pub source_screen_class: String,
    pub source_menu_class: String,
    pub source_path: String,
    pub texture_path: Option<String>,
    pub image_width: u32,
    pub image_height: u32,
    pub slots: Vec<NativeGuiSlot>,
    pub associated_blocks: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct JavaSource {
    class_name: String,
    path: PathBuf,
    text: String,
}

#[derive(Debug, Clone, Default)]
struct HandlerGui {
    screen: Option<String>,
    menu: Option<String>,
}

fn java_sources(roots: &[PathBuf]) -> Vec<JavaSource> {
    let mut paths = Vec::new();
    for root in roots {
        if !root.is_dir() { continue; }
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("java")) {
                paths.push(entry.into_path());
            }
        }
    }
    paths.sort();
    paths.dedup();
    let class_pattern = Regex::new(r"\b(?:class|record)\s+([A-Za-z_$][A-Za-z0-9_$]*)")
        .expect("static class-name regex");
    let mut output = Vec::new();
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        for capture in class_pattern.captures_iter(&text) {
            if let Some(name) = capture.get(1).map(|value| value.as_str().to_owned()) {
                output.push(JavaSource { class_name: name, path: path.clone(), text: text.clone() });
            }
        }
    }
    output.sort_by(|a, b| a.class_name.cmp(&b.class_name).then(a.path.cmp(&b.path)));
    output
}

fn simple_class(value: &str) -> String {
    value.rsplit('.').next().unwrap_or(value).rsplit('$').next().unwrap_or(value).to_owned()
}

fn snake(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut result = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower) {
                result.push('_');
            }
        }
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else if !result.is_empty() && !result.ends_with('_') {
            result.push('_');
        }
    }
    let value = result.trim_matches('_').to_owned();
    if value.is_empty() { "converted_gui".to_owned() } else { value }
}

fn method_body(source: &str, method_name: &str) -> Option<(String, String)> {
    let pattern = Regex::new(&format!(
        r"(?s)\b{}\s*\(\s*int\s+([A-Za-z_$][A-Za-z0-9_$]*)[^)]*\)\s*\{{",
        regex::escape(method_name)
    )).ok()?;
    for capture in pattern.captures_iter(source) {
        let full = capture.get(0)?;
        let parameter = capture.get(1)?.as_str().to_owned();
        let open = full.end().checked_sub(1)?;
        if let Ok((body, _)) = extract_balanced(source, open, '{', '}') {
            return Some((parameter, body));
        }
    }
    None
}

fn gui_ids_from_method(source: &str, method_name: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let Some((parameter, body)) = method_body(source, method_name) else { return found };
    let returns = Regex::new(r"\breturn\s+new\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\(")
        .expect("static GUI return regex");
    let equal_left = Regex::new(&format!(r"\b{}\s*==\s*(-?\d+)", regex::escape(&parameter)))
        .expect("static GUI id regex");
    let equal_right = Regex::new(&format!(r"(-?\d+)\s*==\s*\b{}", regex::escape(&parameter)))
        .expect("static GUI id regex");
    let switch_case = Regex::new(r"\bcase\s+(-?\d+)\s*:")
        .expect("static GUI case regex");
    for capture in returns.captures_iter(&body) {
        let Some(class_name) = capture.get(1).map(|value| simple_class(value.as_str())) else { continue };
        let end = capture.get(0).map(|value| value.start()).unwrap_or(0);
        let prefix = &body[..end];
        let id = equal_left.captures_iter(prefix).last().and_then(|value| value.get(1).map(|m| m.as_str().to_owned()))
            .or_else(|| equal_right.captures_iter(prefix).last().and_then(|value| value.get(1).map(|m| m.as_str().to_owned())))
            .or_else(|| switch_case.captures_iter(prefix).last().and_then(|value| value.get(1).map(|m| m.as_str().to_owned())));
        if let Some(id) = id { found.entry(id).or_insert(class_name); }
    }
    found
}

fn numeric_gui_id(expression: &str, all_sources: &[JavaSource]) -> Option<String> {
    let expression = expression.trim();
    if expression.parse::<i64>().is_ok() { return Some(expression.to_owned()); }
    let identifier = Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$.]*$")
        .expect("static GUI identifier regex");
    if !identifier.is_match(expression) { return None; }
    let leaf = expression.rsplit('.').next().unwrap_or(expression);
    let assignment = Regex::new(&format!(r"(?s)\b{}\s*=\s*(-?\d+)\b", regex::escape(leaf)))
        .expect("static GUI constant regex");
    all_sources.iter().find_map(|source| assignment.captures(&source.text)
        .and_then(|capture| capture.get(1).map(|value| value.as_str().to_owned())))
}

fn gui_call_ids(block_source: &str, all_sources: &[JavaSource]) -> BTreeSet<String> {
    let open_gui = Regex::new(r"(?s)\bopenGui\s*\(\s*[^,]+,\s*([^,\)]+)")
        .expect("static openGui regex");
    open_gui.captures_iter(block_source).filter_map(|capture| {
        numeric_gui_id(capture.get(1)?.as_str(), all_sources)
    }).collect()
}

fn texture_path(source: &str) -> Option<String> {
    let patterns = [
        Regex::new(r#"(?s)new\s+ResourceLocation\s*\(\s*"[^"]+"\s*,\s*"([^"]+)"\s*\)"#).unwrap(),
        Regex::new(r#"(?s)new\s+ResourceLocation\s*\(\s*"[^"]*:([^"]+)"\s*\)"#).unwrap(),
        Regex::new(r#"(?s)new\s+ResourceLocation\s*\(\s*"(textures/[^"]+)"\s*\)"#).unwrap(),
        Regex::new(r#"(?s)(?:ResourceLocation\.parse|ResourceLocation\.tryParse)\s*\(\s*"[^"]*:([^"]+)"\s*\)"#).unwrap(),
    ];
    for pattern in patterns {
        if let Some(value) = pattern.captures(source).and_then(|capture| capture.get(1)) {
            let mut path = value.as_str().replace('\\', "/");
            if let Some(index) = path.find("textures/") { path = path[index..].to_owned(); }
            if !path.starts_with("textures/") { path = format!("textures/gui/{path}"); }
            return Some(path);
        }
    }
    None
}

fn dimension(source: &str, names: &[&str], fallback: u32) -> u32 {
    for name in names {
        let pattern = Regex::new(&format!(r"\b{}\s*=\s*(\d+)", regex::escape(name)))
            .expect("static GUI dimension regex");
        if let Some(value) = pattern.captures(source).and_then(|capture| capture.get(1))
            .and_then(|value| value.as_str().parse::<u32>().ok()) {
            if (16..=1024).contains(&value) { return value; }
        }
    }
    fallback
}

fn slots(source: &str) -> Vec<NativeGuiSlot> {
    let pattern = Regex::new(
        r"new\s+Slot\s*\(\s*[^,]+,\s*(\d+)\s*,\s*(-?\d+)\s*,\s*(-?\d+)\s*\)"
    ).expect("static GUI slot regex");
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for capture in pattern.captures_iter(source) {
        let Some(index) = capture.get(1).and_then(|value| value.as_str().parse::<usize>().ok()) else { continue };
        let Some(x) = capture.get(2).and_then(|value| value.as_str().parse::<i32>().ok()) else { continue };
        let Some(y) = capture.get(3).and_then(|value| value.as_str().parse::<i32>().ok()) else { continue };
        if seen.insert(index) { result.push(NativeGuiSlot { index, x, y }); }
    }
    result.sort_by_key(|slot| slot.index);
    result
}

fn linked_block_key(block_id: &str, gui_id: &str) -> String {
    format!("{}_{}", snake(block_id), snake(gui_id))
}

pub fn discover_block_guis(
    roots: &[PathBuf],
    block_bindings: &[(String, String)],
) -> Result<Vec<NativeGuiPlan>> {
    let sources = java_sources(roots);
    let mut handlers = BTreeMap::<String, HandlerGui>::new();
    let mut handler_paths = BTreeMap::<String, String>::new();
    let mut handler_names = BTreeMap::<String, String>::new();

    for source in &sources {
        if !source.text.contains("IGuiHandler")
            && !source.text.contains("getClientGuiElement")
            && !source.text.contains("getServerGuiElement") {
            continue;
        }
        let client = gui_ids_from_method(&source.text, "getClientGuiElement");
        let server = gui_ids_from_method(&source.text, "getServerGuiElement");
        for gui_id in client.keys().chain(server.keys()) {
            let entry = handlers.entry(gui_id.clone()).or_default();
            if let Some(screen) = client.get(gui_id) { entry.screen = Some(screen.clone()); }
            if let Some(menu) = server.get(gui_id) { entry.menu = Some(menu.clone()); }
            handler_paths.entry(gui_id.clone()).or_insert_with(|| source.path.to_string_lossy().to_string());
            handler_names.entry(gui_id.clone()).or_insert_with(|| source.class_name.clone());
        }
    }

    let source_by_class: BTreeMap<String, &JavaSource> = sources.iter()
        .map(|source| (source.class_name.clone(), source)).collect();
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();

    for (block_id, block_class) in block_bindings {
        let Some(block_source) = source_by_class.get(block_class) else { continue };
        for gui_id in gui_call_ids(&block_source.text, &sources) {
            let Some(handler) = handlers.get(&gui_id) else { continue };
            let key = linked_block_key(block_id, &gui_id);
            if !seen.insert(key.clone()) { continue; }
            let screen_source = handler.screen.as_ref().and_then(|class| source_by_class.get(class).copied());
            let menu_source = handler.menu.as_ref().and_then(|class| source_by_class.get(class).copied());
            let source_screen_class = handler.screen.clone().unwrap_or_default();
            let source_menu_class = handler.menu.clone().unwrap_or_default();
            let mut warnings = Vec::new();
            if handler.screen.is_none() {
                warnings.push(format!("GUI id {gui_id} has no client screen return from its IGuiHandler."));
            }
            if handler.menu.is_none() {
                warnings.push(format!("GUI id {gui_id} has no server container return from its IGuiHandler."));
            }
            let screen_text = screen_source.map(|source| source.text.as_str()).unwrap_or("");
            let menu_text = menu_source.map(|source| source.text.as_str()).unwrap_or("");
            let texture = texture_path(screen_text);
            if texture.is_none() && handler.screen.is_some() {
                warnings.push(format!("Could not resolve a static texture for GUI screen {}.", source_screen_class));
            }
            let slots = slots(menu_text);
            if slots.is_empty() && handler.menu.is_some() {
                warnings.push(format!("No legacy Slot positions were recovered from container {}.", source_menu_class));
            }
            warnings.push(
                "Custom button actions, progress bars, dynamic widgets, container predicates, recipes and machine-specific transfer logic are not reconstructed by the generated compatibility screen.".to_owned()
            );
            warnings.push(
                "The generated GUI uses a generic block inventory; source TileEntity data fields and NBT semantics remain a separate behavior-porting task.".to_owned()
            );
            let width = dimension(screen_text, &["xSize", "imageWidth"], 176);
            let height = dimension(screen_text, &["ySize", "imageHeight"], 166);
            let source_path = screen_source.or(menu_source).map(|source| {
                source.path.to_string_lossy().to_string()
            }).or_else(|| handler_paths.get(&gui_id).cloned()).unwrap_or_default();

            output.push(NativeGuiPlan {
                key,
                source_gui_id: gui_id.clone(),
                source_handler_class: handler_names.get(&gui_id).cloned().unwrap_or_default(),
                source_screen_class,
                source_menu_class,
                source_path,
                texture_path: texture,
                image_width: width,
                image_height: height,
                slots,
                associated_blocks: vec![block_id.clone()],
                warnings,
            });
        }
    }

    output.sort_by(|a, b| a.associated_blocks.cmp(&b.associated_blocks)
        .then(a.source_gui_id.cmp(&b.source_gui_id))
        .then(a.key.cmp(&b.key)));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn links_gui_handler_screen_and_container_to_the_exact_open_gui_block() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/BlockRubyMachine.java"), r#"
            class BlockRubyMachine extends Block {
                public boolean onBlockActivated(EntityPlayer player) {
                    player.openGui(Main.instance, 7, player.world, 0, 0, 0);
                    return true;
                }
            }
        "#).unwrap();
        fs::write(root.join("src/GuiHandler.java"), r#"
            class GuiHandler implements IGuiHandler {
                public Object getClientGuiElement(int id, EntityPlayer player, World world, int x, int y, int z) {
                    if (id == 7) return new GuiRubyMachine(player.inventory, world);
                    return null;
                }
                public Object getServerGuiElement(int id, EntityPlayer player, World world, int x, int y, int z) {
                    if (id == 7) return new ContainerRubyMachine(player.inventory, world);
                    return null;
                }
            }
        "#).unwrap();
        fs::write(root.join("src/GuiRubyMachine.java"), r#"
            class GuiRubyMachine extends GuiContainer {
                private ResourceLocation texture = new ResourceLocation("demo", "textures/gui/ruby_machine.png");
                public GuiRubyMachine() { xSize = 194; ySize = 178; }
            }
        "#).unwrap();
        fs::write(root.join("src/ContainerRubyMachine.java"), r#"
            class ContainerRubyMachine extends Container {
                public ContainerRubyMachine(InventoryPlayer player, TileEntityRubyMachine tile) {
                    addSlotToContainer(new Slot(tile, 0, 56, 17));
                    addSlotToContainer(new Slot(tile, 1, 116, 35));
                }
            }
        "#).unwrap();

        let result = discover_block_guis(
            &[root.to_path_buf()],
            &[("ruby_machine".to_owned(), "BlockRubyMachine".to_owned())],
        ).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_gui_id, "7");
        assert_eq!(result[0].source_handler_class, "GuiHandler");
        assert_eq!(result[0].source_screen_class, "GuiRubyMachine");
        assert_eq!(result[0].source_menu_class, "ContainerRubyMachine");
        assert_eq!(result[0].associated_blocks, vec!["ruby_machine"]);
        assert_eq!(result[0].texture_path.as_deref(), Some("textures/gui/ruby_machine.png"));
        assert_eq!(result[0].image_width, 194);
        assert_eq!(result[0].image_height, 178);
        assert_eq!(result[0].slots, vec![
            NativeGuiSlot { index: 0, x: 56, y: 17 },
            NativeGuiSlot { index: 1, x: 116, y: 35 },
        ]);
    }
}
