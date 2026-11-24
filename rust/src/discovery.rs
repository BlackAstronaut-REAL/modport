use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModLayout {
    pub root: PathBuf,
    pub model_roots: Vec<PathBuf>,
    pub render_roots: Vec<PathBuf>,
    pub texture_roots: Vec<PathBuf>,
    pub animation_roots: Vec<PathBuf>,
    pub resource_roots: Vec<PathBuf>,
}

impl ModLayout {
    pub fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "root": self.root,
            "model_roots": self.model_roots,
            "render_roots": self.render_roots,
            "texture_roots": self.texture_roots,
            "animation_roots": self.animation_roots,
            "resource_roots": self.resource_roots
        })
    }
}

fn normalized_key(path: &Path) -> String {
    let value = path.to_string_lossy().to_string();
    if cfg!(windows) { value.to_lowercase() } else { value }
}

fn unique_existing(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for raw in paths {
        let path = fs::canonicalize(&raw).unwrap_or(raw);
        let key = normalized_key(&path);
        if seen.insert(key) && path.exists() {
            result.push(path);
        }
    }
    result
}

/// Discover common decompiled-mod and Gradle project layouts. The traversal
/// happens once, then directories are classified from the collected inventory.
pub fn discover_mod_layout(mod_root: &Path) -> Result<ModLayout> {
    let root = fs::canonicalize(mod_root)?;
    if !root.is_dir() {
        bail!("Mod root is not a directory: {}", root.display());
    }

    let mut directories = Vec::<PathBuf>::new();
    let mut java_class_files = Vec::<PathBuf>::new();
    let mut has_code_below = HashSet::<PathBuf>::new();
    let mut has_json_below = HashSet::<PathBuf>::new();

    for entry in WalkDir::new(&root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if entry.file_type().is_dir() {
            directories.push(path.to_path_buf());
        } else if entry.file_type().is_file() {
            let extension = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if extension == "java" || extension == "class" {
                java_class_files.push(path.to_path_buf());
                let mut parent = path.parent();
                while let Some(dir) = parent {
                    has_code_below.insert(dir.to_path_buf());
                    if dir == root.as_path() { break; }
                    parent = dir.parent();
                }
            } else if extension == "json" {
                let mut parent = path.parent();
                while let Some(dir) = parent {
                    has_json_below.insert(dir.to_path_buf());
                    if dir == root { break; }
                    parent = dir.parent();
                }
            }
        }
    }

    let mut model_roots = Vec::new();
    let mut render_roots = Vec::new();
    let mut texture_roots = Vec::new();
    let mut animation_roots = Vec::new();
    let mut resource_roots = Vec::new();

    for candidate in [
        root.join("src/main/java"),
        root.join("src/main/resources"),
        root.join("assets"),
        root.join("resources"),
    ] {
        if candidate.exists() { resource_roots.push(candidate); }
    }

    for path in &directories {
        let name = path.file_name().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        match name.as_str() {
            "model" | "models" if has_code_below.contains(path) => model_roots.push(path.clone()),
            "render" | "renderer" | "renderers" if has_code_below.contains(path) => render_roots.push(path.clone()),
            "textures" => {
                let under_assets_or_resources = path.ancestors().any(|ancestor| {
                    ancestor.file_name().and_then(|x| x.to_str())
                        .map(|name| matches!(name.to_ascii_lowercase().as_str(), "assets" | "resources"))
                        .unwrap_or(false)
                });
                if under_assets_or_resources { texture_roots.push(path.clone()); }
            }
            "animation" | "animations" if has_json_below.contains(path) => animation_roots.push(path.clone()),
            _ => {}
        }
    }

    // Legacy flat-package fallback for Model*.java / Model*.class sources.
    if model_roots.is_empty() {
        let mut parent_counts = BTreeMap::<PathBuf, usize>::new();
        for file in &java_class_files {
            let stem = file.file_stem().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            let parent_name = file.parent().and_then(Path::file_name).and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !stem.starts_with("model")
                || stem.ends_with("animation")
                || stem.ends_with("animations")
                || parent_name == "animation"
                || parent_name == "animations"
            {
                continue;
            }
            if let Some(parent) = file.parent() {
                *parent_counts.entry(parent.to_path_buf()).or_insert(0) += 1;
            }
        }
        let mut candidates: Vec<_> = parent_counts.into_iter().collect();
        candidates.sort_by(|(path_a, count_a), (path_b, count_b)| {
            count_b.cmp(count_a).then_with(|| normalized_key(path_a).cmp(&normalized_key(path_b)))
        });
        model_roots.extend(candidates.into_iter().map(|(path, _)| path));
    }

    resource_roots.extend(texture_roots.iter().cloned());
    resource_roots.extend(animation_roots.iter().cloned());

    Ok(ModLayout {
        root,
        model_roots: unique_existing(model_roots),
        render_roots: unique_existing(render_roots),
        texture_roots: unique_existing(texture_roots),
        animation_roots: unique_existing(animation_roots),
        resource_roots: unique_existing(resource_roots),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_named_model_renderer_and_resource_roots() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/main/java/example/model")).unwrap();
        fs::create_dir_all(root.join("src/main/java/example/render")).unwrap();
        fs::create_dir_all(root.join("src/main/resources/assets/demo/textures/entity")).unwrap();
        fs::create_dir_all(root.join("src/main/resources/assets/demo/animations")).unwrap();
        fs::write(root.join("src/main/java/example/model/ModelThing.java"), "class ModelThing {}").unwrap();
        fs::write(root.join("src/main/java/example/render/RenderThing.java"), "class RenderThing {}").unwrap();
        fs::write(root.join("src/main/resources/assets/demo/animations/thing.json"), "{}").unwrap();
        fs::write(root.join("src/main/resources/assets/demo/textures/entity/thing.png"), b"png").unwrap();

        let layout = discover_mod_layout(root).unwrap();
        assert_eq!(layout.model_roots.len(), 1);
        assert_eq!(layout.render_roots.len(), 1);
        assert_eq!(layout.texture_roots.len(), 1);
        assert_eq!(layout.animation_roots.len(), 1);
    }
}
