use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn collect_markdown(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(root, &path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("md") {
            files.push(path.strip_prefix(root).expect("docs file must be under root").to_path_buf());
        }
    }
    Ok(())
}

fn main() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo")
    );
    let docs_root = manifest_dir.join("docs/neoforge/1.21.1");
    let manifest = docs_root.join("manifest.json");
    if !manifest.is_file() {
        panic!("Bundled NeoForge documentation manifest is missing: {}", manifest.display());
    }

    let mut files = vec![(PathBuf::from("manifest.json"), manifest)];
    let mut pages = Vec::new();
    collect_markdown(&docs_root, &docs_root, &mut pages)
        .expect("enumerate bundled NeoForge Markdown pages");
    pages.sort();
    files.extend(pages.into_iter().map(|relative| {
        let absolute = docs_root.join(&relative);
        (relative, absolute)
    }));

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let generated = out_dir.join("bundled_neoforge_docs.rs");
    let mut source = String::from("const BUNDLED_DOC_FILES: &[(&str, &str)] = &[\n");

    for (relative, absolute) in &files {
        let content = fs::read_to_string(absolute)
            .unwrap_or_else(|error| panic!("Read {}: {error}", absolute.display()));
        let relative = relative.to_string_lossy().replace('\\', "/");
        source.push_str(&format!("    ({relative:?}, {content:?}),\n"));
        println!("cargo:rerun-if-changed={}", absolute.display());
    }

    source.push_str("];\n");
    fs::write(&generated, source)
        .unwrap_or_else(|error| panic!("Write {}: {error}", generated.display()));
    println!("cargo:rerun-if-changed=build.rs");
}
