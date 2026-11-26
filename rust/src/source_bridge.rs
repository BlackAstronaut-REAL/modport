use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

fn java_package_path(package: &str) -> Result<PathBuf> {
    let identifier = Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$]*$")?;
    let parts: Vec<&str> = package.split('.').collect();
    if parts.is_empty() || parts.iter().any(|part| !identifier.is_match(part)) {
        bail!("Invalid Java package name: {package}");
    }
    let mut path = PathBuf::new();
    for part in parts { path.push(part); }
    Ok(path)
}

fn package_imports(package: &str, animation: bool) -> String {
    if animation {
        format!("package {package}.client.model.animations;\n\nimport net.minecraft.client.animation.*;\n\n")
    } else {
        format!(
            "package {package}.client.model;\n\n\
             import com.mojang.blaze3d.vertex.*;\n\
             import net.minecraft.client.model.*;\n\
             import net.minecraft.client.model.geom.*;\n\
             import net.minecraft.client.model.geom.builders.*;\n\
             import net.minecraft.resources.*;\n\
             import net.minecraft.util.*;\n\
             import net.minecraft.world.entity.*;\n\n"
        )
    }
}

fn has_package_declaration(source: &str) -> bool {
    source.lines().any(|line| line.trim_start().starts_with("package "))
}

fn write_java_file(destination: &Path, source: &str) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| format!("Creating Java source directory {}", parent.display()))?;
    }
    fs::write(destination, format!("{}\n", source.trim_end()))
        .with_context(|| format!("Writing Java source {}", destination.display()))
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// Copy source model and animation Java files into the generated MCreator package.
/// Sources lacking package declarations receive the import headers used by oldVer.
pub fn copy_model_sources(
    output_root: &Path,
    package: &str,
    model_files: &[PathBuf],
    animation_files: &[PathBuf],
) -> Result<BTreeMap<String, String>> {
    let package_path = java_package_path(package)?;
    let model_destination = output_root.join("src/main/java").join(package_path).join("client/model");
    let animation_destination = model_destination.join("animations");
    let mut written = BTreeMap::new();

    for source_path in model_files {
        let bytes = fs::read(source_path).with_context(|| format!("Reading model Java source {}", source_path.display()))?;
        let mut source = String::from_utf8_lossy(&bytes).into_owned();
        if !has_package_declaration(&source) {
            source = format!("{}{}", package_imports(package, false), source);
        }
        let file_name = source_path.file_name().ok_or_else(|| anyhow::anyhow!("Model source path has no filename: {}", source_path.display()))?;
        let destination = model_destination.join(file_name);
        write_java_file(&destination, &source)?;
        let stem = source_path.file_stem().and_then(|value| value.to_str()).unwrap_or_default().to_owned();
        written.insert(stem, relative_display(&destination, output_root));
    }

    for source_path in animation_files {
        let bytes = fs::read(source_path).with_context(|| format!("Reading animation Java source {}", source_path.display()))?;
        let mut source = String::from_utf8_lossy(&bytes).into_owned();
        if !has_package_declaration(&source) {
            source = format!("{}{}", package_imports(package, true), source);
        }
        let file_name = source_path.file_name().ok_or_else(|| anyhow::anyhow!("Animation source path has no filename: {}", source_path.display()))?;
        let destination = animation_destination.join(file_name);
        write_java_file(&destination, &source)?;
        let stem = source_path.file_stem().and_then(|value| value.to_str()).unwrap_or_default().to_owned();
        written.insert(stem, relative_display(&destination, output_root));
    }

    Ok(written)
}

/// Copy a sibling PNG animation metadata sidecar next to its imported texture.
pub fn copy_animation_metadata(source: &Path, target: &Path) -> Result<bool> {
    let mut source_meta_name: OsString = source.as_os_str().to_os_string();
    source_meta_name.push(".mcmeta");
    let source_meta = PathBuf::from(source_meta_name);
    if !source_meta.is_file() { return Ok(false); }

    let mut target_meta_name: OsString = target.as_os_str().to_os_string();
    target_meta_name.push(".mcmeta");
    let target_meta = PathBuf::from(target_meta_name);
    if let Some(parent) = target_meta.parent() {
        fs::create_dir_all(parent).with_context(|| format!("Creating texture metadata directory {}", parent.display()))?;
    }
    fs::copy(&source_meta, &target_meta)
        .with_context(|| format!("Copying animation metadata {} to {}", source_meta.display(), target_meta.display()))?;
    Ok(true)
}

/// Attach workspace-relative files to an element descriptor; returns false if absent.
pub fn ensure_workspace_metadata(workspace: &mut Value, element_name: &str, files: &[String]) -> bool {
    let Some(elements) = workspace.get_mut("mod_elements").and_then(Value::as_array_mut) else { return false };
    for element in elements {
        let matches = element.get("name").and_then(Value::as_str)
            .is_some_and(|name| name.eq_ignore_ascii_case(element_name));
        if !matches { continue; }
        let Some(element) = element.as_object_mut() else { return false };
        let metadata = element.entry("metadata".to_owned()).or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !metadata.is_object() { *metadata = Value::Object(serde_json::Map::new()); }
        let metadata = metadata.as_object_mut().expect("metadata object was initialized");
        let existing = metadata.entry("files".to_owned()).or_insert_with(|| Value::Array(Vec::new()));
        if !existing.is_array() { *existing = Value::Array(Vec::new()); }
        let existing = existing.as_array_mut().expect("files array was initialized");
        for file in files {
            if !file.is_empty() && !existing.iter().any(|entry| entry.as_str() == Some(file)) {
                existing.push(Value::String(file.clone()));
            }
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn copies_model_and_animation_sources_with_expected_headers() {
        let temp = tempdir().unwrap();
        let sources = temp.path().join("sources");
        fs::create_dir_all(&sources).unwrap();
        let model = sources.join("ModelExample.java");
        let animation = sources.join("ModelExampleAnimation.java");
        fs::write(&model, "public class ModelExample { }\n").unwrap();
        fs::write(&animation, "public class ModelExampleAnimation { }\n").unwrap();
        let output = temp.path().join("workspace");

        let written = copy_model_sources(&output, "com.example.demo", &[model], &[animation]).unwrap();
        let model_path = output.join("src/main/java/com/example/demo/client/model/ModelExample.java");
        let animation_path = output.join("src/main/java/com/example/demo/client/model/animations/ModelExampleAnimation.java");
        let model_text = fs::read_to_string(model_path).unwrap();
        let animation_text = fs::read_to_string(animation_path).unwrap();
        assert!(model_text.starts_with("package com.example.demo.client.model;"));
        assert!(model_text.contains("import net.minecraft.client.model.geom.builders.*;"));
        assert!(animation_text.starts_with("package com.example.demo.client.model.animations;"));
        assert!(animation_text.contains("import net.minecraft.client.animation.*;"));
        assert!(written["ModelExample"].ends_with("client/model/ModelExample.java"));
        assert!(written["ModelExampleAnimation"].ends_with("client/model/animations/ModelExampleAnimation.java"));
    }

    #[test]
    fn preserves_existing_packages_and_rejects_unsafe_package_paths() {
        let temp = tempdir().unwrap();
        let model = temp.path().join("AlreadyPackaged.java");
        fs::write(&model, "package author.supplied;\npublic class AlreadyPackaged {}\n").unwrap();
        let output = temp.path().join("workspace");
        copy_model_sources(&output, "com.example", &[model], &[]).unwrap();
        let target = output.join("src/main/java/com/example/client/model/AlreadyPackaged.java");
        assert_eq!(fs::read_to_string(target).unwrap(), "package author.supplied;\npublic class AlreadyPackaged {}\n");
        assert!(copy_model_sources(&output, "../unsafe", &[], &[]).is_err());
    }

    #[test]
    fn copies_animation_metadata_and_updates_element_file_manifest() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source/icon.png");
        let target = temp.path().join("workspace/assets/icon.png");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"png").unwrap();
        fs::write(format!("{}.mcmeta", source.display()), b"{\"animation\":{}}").unwrap();
        assert!(copy_animation_metadata(&source, &target).unwrap());
        assert_eq!(fs::read(format!("{}.mcmeta", target.display())).unwrap(), b"{\"animation\":{}}");

        let mut workspace = json!({"mod_elements":[
            {"name":"ExampleMob","metadata":{"other":"preserved","files":["existing.java"]}}
        ]});
        assert!(ensure_workspace_metadata(&mut workspace, "examplemob", &[
            "existing.java".to_owned(), "models/Example.java".to_owned()
        ]));
        assert_eq!(workspace["mod_elements"][0]["metadata"]["other"], "preserved");
        assert_eq!(workspace["mod_elements"][0]["metadata"]["files"], json!(["existing.java","models/Example.java"]));
        assert!(!ensure_workspace_metadata(&mut workspace, "missing", &[]));
    }
}
