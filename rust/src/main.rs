use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use mojmodelbridge::pipeline::convert_model_source;
use mojmodelbridge::mod_graph::scan_mod_graph;
use mojmodelbridge::scan::scan_project;

#[derive(Debug, Parser)]
#[command(name = "MojModelBridge", version, about = "Native Rust Minecraft mod migration engine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect a decompiled, compiled, or combined mod directory.
    Inspect {
        /// One or more roots to merge into the same logical mod project.
        #[arg(required = true)]
        roots: Vec<PathBuf>,
        /// Write the scan report to this JSON path instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Convert a decompiled legacy or modern Java model into a Blockbench project.
    ConvertModel {
        source: PathBuf,
        output: PathBuf,
        /// Directories searched recursively for matching PNG textures.
        #[arg(long = "texture-root")]
        texture_roots: Vec<PathBuf>,
        /// Replace an existing output.
        #[arg(long)]
        overwrite: bool,
    },
    /// Build the native entity → renderer → model relationship graph from mixed source roots.
    ModGraph {
        root: PathBuf,
        /// Additional decompiled source/resource roots to merge with the primary root.
        #[arg(long = "source-root")]
        source_roots: Vec<PathBuf>,
        /// Write the graph as JSON instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Safely extract a JAR or ZIP archive.
    Extract {
        archive: PathBuf,
        destination: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Inspect { roots, output } => {
            let report = scan_project(&roots)?;
            let data = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(&path, format!("{data}\n"))?;
                println!("Wrote project scan: {}", path.display());
            } else {
                println!("{data}");
            }
        }
        Command::ConvertModel { source, output, texture_roots, overwrite } => {
            let result = convert_model_source(&source, &output, &texture_roots, overwrite)?;
            println!(
                "Converted {}: {} parts, {} cubes, {} texture(s), parser={}.",
                result.source.display(), result.part_count, result.cube_count,
                result.texture_count, result.source_format
            );
            println!("Blockbench project: {}", result.output.display());
            for warning in result.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::ModGraph { root, source_roots, output } => {
            let graph = scan_mod_graph(&root, &source_roots)?;
            let data = serde_json::to_string_pretty(&graph)?;
            if let Some(path) = output {
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(&path, format!("{data}\n"))?;
                println!(
                    "Wrote mod graph: {} renderer links, {} model classes, {} entities → {}",
                    graph.registrations.len(),
                    graph.registrations_by_model().len(),
                    graph.entities.len(),
                    path.display()
                );
            } else {
                println!("{data}");
            }
            for warning in graph.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::Extract { archive, destination } => {
            let path = mojmodelbridge::archive::extract_archive(&archive, &destination)?;
            println!("Extracted {} to {}", archive.display(), path.display());
        }
    }
    Ok(())
}
