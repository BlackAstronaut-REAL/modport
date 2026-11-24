use anyhow::{bail, Context, Result};
use mojmodelbridge::metadata::infer_mod_metadata;
use mojmodelbridge::mod_graph::scan_mod_graph;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const CATEGORIES: &[&str] = &[
    "Mobs", "Blocks", "Weapons", "Tools", "Armor", "Items", "Food", "Models",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    MobEgg,
    Texture,
    Model,
}

#[derive(Debug, Clone)]
pub struct BrowserEntry {
    pub category: String,
    pub name: String,
    pub detail: String,
    pub icon_kind: IconKind,
    pub icon_path: Option<PathBuf>,
    pub model_source: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserIndex {
    pub root: Option<PathBuf>,
    pub display_name: String,
    pub entries: Vec<BrowserEntry>,
    pub texture_roots: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl BrowserIndex {
    pub fn in_category(&self, category: &str) -> Vec<usize> {
        self.entries.iter().enumerate()
            .filter_map(|(index, entry)| (entry.category == category).then_some(index))
            .collect()
    }
}

pub fn scan_mod(root: &Path) -> Result<BrowserIndex> {
    if !root.is_dir() {
        bail!("Choose an extracted mod/project directory. JAR/ZIP archive support will be added to the browser scan next.");
    }
    let root = std::fs::canonicalize(root)
        .with_context(|| format!("Resolving mod directory {}", root.display()))?;
    let metadata = infer_mod_metadata(&root);
    let display_name = if !metadata.name.is_empty() {
        metadata.name
    } else if !metadata.modid.is_empty() {
        metadata.modid
    } else {
        root.file_name().and_then(|value| value.to_str()).unwrap_or("Mod project").to_owned()
    };

    // The shared graph already links entity registrations to renderer/model
    // classes. Reuse it for the mob browser instead of guessing from names.
    let graph = scan_mod_graph(&root, &[])?;
    let mut all_files = Vec::<PathBuf>::new();
    for entry in WalkDir::new(&root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file() { continue; }
        let path = entry.path();
        if path.components().any(|component| {
            matches!(component.as_os_str().to_string_lossy().to_ascii_lowercase().as_str(),
                ".git" | ".gradle" | "build" | "target" | "node_modules")
        }) {
            continue;
        }
        all_files.push(path.to_path_buf());
        if all_files.len() >= 250_000 { break; }
    }
    all_files.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());

    let mut entries = Vec::<BrowserEntry>::new();
    let mut seen = HashSet::<String>::new();
    let mut texture_roots = Vec::<PathBuf>::new();

    let mut add = |entry: BrowserEntry| {
        let key = format!("{}:{}", entry.category.to_ascii_lowercase(), entry.name.to_ascii_lowercase());
        if seen.insert(key) { entries.push(entry); }
    };

    // Registered entities get their display name and model linkage from the
    // existing registration graph. Their browser icon is a spawn-egg glyph.
    for (entity_name, entity) in &graph.entities {
        let registration = graph.registrations.iter()
            .find(|registration| registration.entity_class == *entity_name);
        let model_source = registration.and_then(|registration| {
            let model = registration.model_class.rsplit(['.', '/', '$']).next().unwrap_or("");
            graph.java_files.get(model).or_else(|| graph.class_files.get(model)).cloned()
        });
        let icon_path = registration.and_then(|registration| {
            find_texture_reference(&all_files, &registration.texture_refs)
        });
        add(BrowserEntry {
            category: "Mobs".to_owned(),
            name: if entity.display_name.is_empty() {
                humanize(entity_name)
            } else {
                entity.display_name.clone()
            },
            detail: if let Some(registration) = registration {
                if registration.model_class.is_empty() {
                    entity.registry_name.clone()
                } else {
                    format!("{} · {}", entity.registry_name, registration.model_class)
                }
            } else {
                entity.registry_name.clone()
            },
            icon_kind: IconKind::MobEgg,
            icon_path,
            model_source,
        });
    }

    // Resource textures supply the first concrete item/block catalog. Category
    // classification is intentionally conservative and is based on asset paths
    // plus obvious vanilla item-name families, not mod-specific hardcoding.
    for path in &all_files {
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else { continue; };
        if !extension.eq_ignore_ascii_case("png") { continue; }
        let components: Vec<String> = path.components()
            .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
            .collect();
        let Some(texture_index) = components.iter().position(|part| part == "textures") else { continue; };
        let Some(kind) = components.get(texture_index + 1).map(String::as_str) else { continue; };
        let category = match kind {
            "block" | "blocks" => "Blocks",
            "item" | "items" => categorize_item(path.file_stem().and_then(|value| value.to_str()).unwrap_or("")),
            _ => continue,
        };
        if let Some(parent) = path.parent() {
            if !texture_roots.contains(&parent.to_path_buf()) {
                texture_roots.push(parent.to_path_buf());
            }
        }
        add(BrowserEntry {
            category: category.to_owned(),
            name: humanize(path.file_stem().and_then(|value| value.to_str()).unwrap_or("texture")),
            detail: path.strip_prefix(&root).unwrap_or(path).to_string_lossy().to_string(),
            icon_kind: IconKind::Texture,
            icon_path: Some(path.clone()),
            model_source: None,
        });
    }

    // Model candidates without a known mob registration remain browsable.
    // Selecting one opens the exact native parser path used by conversion.
    for path in &all_files {
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else { continue; };
        if !extension.eq_ignore_ascii_case("java") && !extension.eq_ignore_ascii_case("class") { continue; }
        let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("");
        if !stem.to_ascii_lowercase().starts_with("model") { continue; }
        let already_linked = graph.registrations.iter().any(|registration| {
            registration.model_class.rsplit(['.', '/', '$']).next()
                .map(|name| name.eq_ignore_ascii_case(stem)).unwrap_or(false)
        });
        if already_linked { continue; }
        add(BrowserEntry {
            category: "Models".to_owned(),
            name: humanize(stem.strip_prefix("Model").unwrap_or(stem)),
            detail: path.strip_prefix(&root).unwrap_or(path).to_string_lossy().to_string(),
            icon_kind: IconKind::Model,
            icon_path: None,
            model_source: Some(path.clone()),
        });
    }

    drop(add);
    entries.sort_by(|a, b| {
        category_order(&a.category).cmp(&category_order(&b.category))
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    let mut warnings = graph.warnings;
    if all_files.len() >= 250_000 {
        warnings.push("The browser scan reached its 250,000-file safety limit; some assets may not be listed.".to_owned());
    }
    if entries.is_empty() {
        warnings.push("No supported entity registrations, item/block textures, or model source files were found. This is an asset browser index, not a complete registry exporter.".to_owned());
    }

    Ok(BrowserIndex {
        root: Some(root),
        display_name,
        entries,
        texture_roots,
        warnings,
    })
}

fn category_order(category: &str) -> usize {
    CATEGORIES.iter().position(|candidate| *candidate == category).unwrap_or(usize::MAX)
}

fn humanize(value: &str) -> String {
    let chars: Vec<char> = value.replace('_', " ").replace('-', " ").chars().collect();
    let mut output = String::with_capacity(chars.len() + 8);
    for (index, ch) in chars.iter().copied().enumerate() {
        if index > 0 && ch.is_ascii_uppercase() && chars[index - 1].is_ascii_lowercase() {
            output.push(' ');
        }
        output.push(ch);
    }
    output.split_whitespace().map(|word| {
        let mut chars = word.chars();
        chars.next().map(|first| first.to_uppercase().to_string() + chars.as_str()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}

fn categorize_item(name: &str) -> &'static str {
    let value = name.to_ascii_lowercase();
    if ["sword", "bow", "crossbow", "trident", "launcher", "dagger", "spear"]
        .iter().any(|term| value.contains(term))
    {
        "Weapons"
    } else if ["pickaxe", "axe", "shovel", "hoe", "shears", "fishing_rod", "brush", "flint_and_steel"]
        .iter().any(|term| value.contains(term))
    {
        "Tools"
    } else if ["helmet", "chestplate", "leggings", "boots"]
        .iter().any(|term| value.contains(term))
    {
        "Armor"
    } else if [
        "apple", "bread", "beef", "steak", "chicken", "porkchop", "mutton", "rabbit",
        "cod", "salmon", "carrot", "potato", "beetroot", "cookie", "cake", "pie",
        "stew", "soup", "melon", "berries", "chorus_fruit", "dried_kelp", "honey_bottle",
        "pumpkin_pie", "sweet_berries", "glow_berries",
    ].iter().any(|term| value.contains(term)) {
        "Food"
    } else {
        "Items"
    }
}

fn find_texture_reference(files: &[PathBuf], references: &[String]) -> Option<PathBuf> {
    for reference in references {
        let cleaned = reference.replace('\\', "/");
        let suffix = cleaned.rsplit(':').next().unwrap_or(&cleaned).trim_start_matches('/');
        let suffix = if suffix.to_ascii_lowercase().ends_with(".png") {
            suffix.to_owned()
        } else {
            format!("{suffix}.png")
        };
        if let Some(found) = files.iter().find(|path| {
            path.to_string_lossy().replace('\\', "/").to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase())
        }) {
            return Some(found.clone());
        }
        let filename = Path::new(&suffix).file_name().and_then(|value| value.to_str()).unwrap_or("");
        if let Some(found) = files.iter().find(|path| {
            path.file_name().and_then(|value| value.to_str())
                .map(|name| name.eq_ignore_ascii_case(filename)).unwrap_or(false)
                && path.to_string_lossy().to_ascii_lowercase().contains("textures")
        }) {
            return Some(found.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_categories_are_inferred_from_asset_name_families() {
        assert_eq!(categorize_item("iron_sword"), "Weapons");
        assert_eq!(categorize_item("diamond_pickaxe"), "Tools");
        assert_eq!(categorize_item("leather_chestplate"), "Armor");
        assert_eq!(categorize_item("enchanted_golden_apple"), "Food");
        assert_eq!(categorize_item("magic_orb"), "Items");
    }

    #[test]
    fn humanizes_registry_and_model_names() {
        assert_eq!(humanize("red_ant"), "Red Ant");
        assert_eq!(humanize("FireBeetle"), "Fire Beetle");
        assert_eq!(humanize("ModelTriffid"), "Model Triffid");
    }
}
