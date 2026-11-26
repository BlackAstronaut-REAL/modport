use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::blockbench::model_to_bbmodel;
use crate::bytecode::parse_any_model_class;
use crate::discovery::discover_mod_layout;
use crate::source::parse_java_model;
use crate::model::{expand_render_instances, ParsedModel};
use crate::texture::TextureIndex;

fn resolve_texture_set(source: &Path, model_name: &str, supplied_roots: &[PathBuf]) -> Result<crate::texture::TextureSetResolution> {
    let mut roots = crate::texture::infer_search_roots(source, supplied_roots);
    let mut shallow_roots = Vec::new();
    if let Some(parent) = source.parent() {
        roots.push(parent.to_path_buf());
        shallow_roots.push(parent.to_path_buf());

        // Decompiled mods often split model and renderer packages beside each
        // other. Inspect the nearest ancestor that actually contains a resource
        // root, and avoid repeatedly walking the same source tree.
        let resource_root = parent.ancestors().take(8).find(|ancestor| {
            ["assets", "resources", "textures", "src/main/resources"]
                .iter().any(|name| ancestor.join(name).is_dir())
        });
        if let Some(search_root) = resource_root {
            if let Ok(layout) = discover_mod_layout(search_root) {
                roots.extend(layout.model_roots);
                roots.extend(layout.render_roots);
                roots.extend(layout.texture_roots);
                roots.extend(layout.resource_roots);
            }
        }
    }
    let index = TextureIndex::new(&roots, &shallow_roots)?;
    index.resolve_set(source, model_name, None, 32)
}

/// Parsed model and the body texture files resolved by the same pipeline used
/// by the Blockbench exporter. The desktop viewer consumes this structure and
/// does not run a second Java/bytecode parser.
#[derive(Debug, Clone)]
pub struct ModelPreviewAsset {
    pub model: ParsedModel,
    pub texture_paths: Vec<PathBuf>,
}

pub fn load_model_preview(source: &Path, texture_roots: &[PathBuf]) -> Result<ModelPreviewAsset> {
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
        _ => bail!("The viewer accepts decompiled .java or compiled legacy .class model files."),
    };
    let texture_resolution = resolve_texture_set(source, &model.class_name, texture_roots)?;
    let texture_paths = texture_resolution.variants.clone();
    model.texture_evidence = texture_resolution.evidence.clone();
    model.texture_evidence.extend(texture_resolution.candidate_details.iter().map(|(path, score, reason)| {
        format!("{} [{score:.3}]: {reason}", path.display())
    }));
    model.texture_variants = texture_paths.iter().map(|path| path.to_string_lossy().to_string()).collect();
    model.auxiliary_textures = texture_resolution.auxiliaries.iter().map(|path| path.to_string_lossy().to_string()).collect();
    if let Some(primary) = texture_resolution.primary.as_ref() {
        model.primary_texture = primary.to_string_lossy().to_string();
        model.warnings.push(format!(
            "Primary texture resolved to '{}' ({})",
            primary.display(),
            texture_resolution.evidence.join("; ")
        ));
    } else if !texture_resolution.auxiliaries.is_empty() {
        model.warnings.push(format!(
            "Only auxiliary textures matched; no eye/glow/overlay texture was selected as the body skin. Skipped: {}",
            texture_resolution.auxiliaries.iter().map(|path| path.file_name().and_then(|value| value.to_str()).unwrap_or("").to_owned()).collect::<Vec<_>>().join(", ")
        ));
    } else {
        model.warnings.push("No body PNG could be resolved from model/renderer references or conservative texture-family matching.".to_owned());
    }
    expand_render_instances(&mut model);
    if model.cube_count() == 0 {
        bail!("No model cubes were recovered from {}. Parser warnings: {}", source.display(), model.warnings.join(" | "));
    }
    Ok(ModelPreviewAsset { model, texture_paths })
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
    let texture_resolution = resolve_texture_set(source, &model.class_name, texture_roots)?;
    let resolved_textures = texture_resolution.variants.clone();
    model.texture_evidence = texture_resolution.evidence.clone();
    model.texture_evidence.extend(texture_resolution.candidate_details.iter().map(|(path, score, reason)| {
        format!("{} [{score:.3}]: {reason}", path.display())
    }));
    model.texture_variants = resolved_textures.iter().map(|path| path.to_string_lossy().to_string()).collect();
    model.auxiliary_textures = texture_resolution.auxiliaries.iter().map(|path| path.to_string_lossy().to_string()).collect();
    if let Some(primary) = texture_resolution.primary.as_ref() {
        model.primary_texture = primary.to_string_lossy().to_string();
        model.warnings.push(format!(
            "Primary texture resolved to '{}' ({})",
            primary.display(),
            texture_resolution.evidence.join("; ")
        ));
        if resolved_textures.len() > 1 {
            model.warnings.push(format!(
                "Embedded {} compatible body/color textures; the alternate texture candidates are retained in the project metadata.",
                resolved_textures.len()
            ));
        }
    } else if !texture_resolution.auxiliaries.is_empty() {
        model.warnings.push(format!(
            "Only auxiliary textures matched; no eye/glow/overlay texture was selected as the body skin. Skipped: {}",
            texture_resolution.auxiliaries.iter().map(|path| path.file_name().and_then(|value| value.to_str()).unwrap_or("").to_owned()).collect::<Vec<_>>().join(", ")
        ));
    } else {
        model.warnings.push("No body PNG could be resolved from model/renderer references or conservative texture-family matching.".to_owned());
    }

    expand_render_instances(&mut model);
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
