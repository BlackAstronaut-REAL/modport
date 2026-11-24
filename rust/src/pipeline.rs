use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::blockbench::model_to_bbmodel;
use crate::bytecode::parse_any_model_class;
use crate::discovery::discover_mod_layout;
use crate::source::parse_java_model;

fn normalize_stem(value: &str) -> String {
    value.to_ascii_lowercase().chars().filter(|ch| ch.is_ascii_alphanumeric()).collect()
}

fn discover_textures(source: &Path, class_name: &str, supplied_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = supplied_roots.to_vec();
    if roots.is_empty() {
        if let Some(parent) = source.parent() {
            roots.push(parent.to_path_buf());
            // Search nearby project roots, not the entire filesystem. A source
            // file commonly lives several packages below src/main/java.
            for ancestor in parent.ancestors().take(7) {
                if ancestor.is_dir() {
                    if let Ok(layout) = discover_mod_layout(ancestor) {
                        roots.extend(layout.texture_roots);
                        roots.extend(layout.resource_roots);
                        if !roots.is_empty() { break; }
                    }
                }
            }
        }
    }

    let class_key = normalize_stem(class_name);
    let shortened_key = normalize_stem(class_name.strip_prefix("Model").unwrap_or(class_name));
    let mut candidates = Vec::<(u8, String, PathBuf)>::new();
    for root in roots {
        if !root.exists() { continue; }
        for entry in WalkDir::new(&root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if !entry.file_type().is_file() { continue; }
            if !entry.path().extension().and_then(|v| v.to_str()).map(|v| v.eq_ignore_ascii_case("png")).unwrap_or(false) { continue; }

            let filename = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if ["glow", "overlay", "eyes", "eye_", "emissive", "armor"].iter().any(|word| filename.contains(word)) { continue; }

            let stem = normalize_stem(entry.path().file_stem().and_then(|v| v.to_str()).unwrap_or(""));
            let score = if stem == class_key { 0 }
                else if !shortened_key.is_empty() && stem == shortened_key { 1 }
                else if stem.starts_with(&class_key) && !class_key.is_empty() { 2 }
                else { continue };
            let display = entry.path().to_string_lossy().to_ascii_lowercase().to_string();
            candidates.push((score, display, entry.path().to_path_buf()));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut result = Vec::new();
    for (_, _, path) in candidates {
        if !result.contains(&path) { result.push(path); }
    }
    result
}

#[derive(Debug, Serialize)]
pub struct ModelConversion {
    pub source: PathBuf,
    pub output: PathBuf,
    pub source_format: String,
    pub part_count: usize,
    pub cube_count: usize,
    pub texture_count: usize,
    pub warnings: Vec<String>,
}

/// Native Java source or legacy JVM class → Blockbench conversion. Uses the
/// modern/legacy source parser or direct class-file interpreter; no Python, javap, or JDK.
pub fn convert_model_source(
    source: &Path,
    output: &Path,
    texture_roots: &[PathBuf],
    overwrite: bool,
) -> Result<ModelConversion> {
    if output.exists() && !overwrite {
        bail!("Output already exists: {}", output.display());
    }
    if !source.is_file() {
        bail!("Model source does not exist or is not a file: {}", source.display());
    }
    let extension = source.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut model = match extension.as_str() {
        "java" => {
            let source_text = fs::read_to_string(source)
                .with_context(|| format!("Reading model source {}", source.display()))?;
            parse_java_model(&source_text, &source.to_string_lossy(), &[])?
        }
        "class" => parse_any_model_class(source, &[])?,
        _ => bail!("This stage accepts decompiled .java or compiled legacy .class model files."),
    };
    let resolved_textures = discover_textures(source, &model.class_name, texture_roots);
    if let Some(primary) = resolved_textures.first() {
        model.primary_texture = primary.to_string_lossy().to_string();
        model.texture_evidence.push(format!("Native filename match for {}", model.class_name));
    } else {
        model.warnings.push("No matching body PNG was found for automatic embedding.".to_owned());
    }
    model.texture_variants = resolved_textures.iter().map(|path| path.to_string_lossy().to_string()).collect();

    let part_count = model.parts.len();
    let cube_count = model.cube_count();
    if cube_count == 0 {
        bail!("No model cubes were recovered from {}. Parser warnings: {}", source.display(), model.warnings.join(" | "));
    }

    let project = model_to_bbmodel(&mut model, &source.to_string_lossy(), &resolved_textures)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() { fs::create_dir_all(parent)?; }
    }
    fs::write(output, format!("{}\n", serde_json::to_string_pretty(&project)?))
        .with_context(|| format!("Writing Blockbench model {}", output.display()))?;

    let sidecar_dir = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output.file_stem().and_then(|v| v.to_str()).unwrap_or("model");
    if !model.animation_logic.is_empty() {
        fs::write(sidecar_dir.join(format!("{stem}_animation.java")), &model.animation_logic)?;
        fs::write(
            sidecar_dir.join(format!("{stem}_animation_analysis.json")),
            format!("{}\n", serde_json::to_string_pretty(&model.animation_analysis)?),
        )?;
    }
    if !model.warnings.is_empty() {
        fs::write(sidecar_dir.join(format!("{stem}_conversion_warnings.txt")), model.warnings.join("\n") + "\n")?;
    }

    Ok(ModelConversion {
        source: source.to_path_buf(),
        output: output.to_path_buf(),
        source_format: model.source_format,
        part_count,
        cube_count,
        texture_count: resolved_textures.len(),
        warnings: model.warnings,
    })
}
