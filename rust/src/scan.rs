use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::discovery::{discover_mod_layout, ModLayout};
use crate::metadata::{infer_mod_metadata, ModMetadata};

#[derive(Debug, Clone, Serialize)]
pub struct RootScan {
    pub root: PathBuf,
    pub layout: ModLayout,
    pub file_counts: BTreeMap<String, usize>,
    pub model_candidates: Vec<PathBuf>,
    pub java_sources: Vec<PathBuf>,
    pub compiled_classes: Vec<PathBuf>,
    pub assets: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectScan {
    pub roots: Vec<PathBuf>,
    pub metadata: ModMetadata,
    pub root_scans: Vec<RootScan>,
    pub merged_model_roots: Vec<PathBuf>,
    pub merged_renderer_roots: Vec<PathBuf>,
    pub merged_texture_roots: Vec<PathBuf>,
    pub merged_animation_roots: Vec<PathBuf>,
    pub merged_resource_roots: Vec<PathBuf>,
    pub notes: Vec<String>,
}

fn add_unique(target: &mut Vec<PathBuf>, values: &[PathBuf]) {
    for value in values {
        if !target.contains(value) { target.push(value.clone()); }
    }
}

fn scan_root(root: &Path) -> Result<RootScan> {
    let layout = discover_mod_layout(root)?;
    let mut counts = BTreeMap::<String, usize>::new();
    let mut models = Vec::new();
    let mut java = Vec::new();
    let mut classes = Vec::new();
    let mut assets = Vec::new();

    for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() { continue; }
        let extension = entry.path().extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        *counts.entry(if extension.is_empty() { "[no extension]".into() } else { extension.clone() }).or_insert(0) += 1;
        match extension.as_str() {
            "java" => java.push(entry.path().to_path_buf()),
            "class" => classes.push(entry.path().to_path_buf()),
            "png" | "json" | "lang" | "properties" | "toml" => assets.push(entry.path().to_path_buf()),
            _ => {}
        }
    }
    models.extend(java.iter().chain(classes.iter())
        .filter(|path| path.file_stem().and_then(|x| x.to_str()).map(|s| s.to_ascii_lowercase().starts_with("model")).unwrap_or(false))
        .cloned());
    models.sort();
    java.sort(); classes.sort(); assets.sort();

    Ok(RootScan {
        root: root.to_path_buf(), layout, file_counts: counts, model_candidates: models,
        java_sources: java, compiled_classes: classes, assets,
    })
}

/// Scan and merge compiled/decompiled/resource roots into one logical mod.
/// This is the first native pipeline stage; conversion stages will be added
/// behind this project object as their differential tests reach parity.
pub fn scan_project(roots: &[PathBuf]) -> Result<ProjectScan> {
    let mut scans = Vec::new();
    for root in roots { scans.push(scan_root(root)?); }

    let primary = roots.first().map(|root| infer_mod_metadata(root)).unwrap_or_default();
    let metadata = if primary.modid.is_empty() {
        roots.iter().skip(1).map(|root| infer_mod_metadata(root))
            .find(|meta| !meta.modid.is_empty()).unwrap_or(primary)
    } else { primary };

    let mut merged_model_roots = Vec::new();
    let mut merged_renderer_roots = Vec::new();
    let mut merged_texture_roots = Vec::new();
    let mut merged_animation_roots = Vec::new();
    let mut merged_resource_roots = Vec::new();
    for scan in &scans {
        add_unique(&mut merged_model_roots, &scan.layout.model_roots);
        add_unique(&mut merged_renderer_roots, &scan.layout.render_roots);
        add_unique(&mut merged_texture_roots, &scan.layout.texture_roots);
        add_unique(&mut merged_animation_roots, &scan.layout.animation_roots);
        add_unique(&mut merged_resource_roots, &scan.layout.resource_roots);
    }

    let mut notes = vec!["Inspection only: model reconstruction and exporters are not yet wired into the Rust pipeline.".into()];
    if metadata.modid.is_empty() {
        notes.push("No mod ID was found in metadata, source annotations, or assets namespaces.".into());
    }

    Ok(ProjectScan {
        roots: roots.to_vec(), metadata, root_scans: scans,
        merged_model_roots, merged_renderer_roots, merged_texture_roots,
        merged_animation_roots, merged_resource_roots, notes,
    })
}
