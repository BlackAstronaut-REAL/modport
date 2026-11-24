use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use mojmodelbridge::pipeline::convert_model_source;
use mojmodelbridge::mcreator_model::export_model_sources;
use mojmodelbridge::mod_graph::scan_mod_graph;
use mojmodelbridge::scan::scan_project;
use mojmodelbridge::workspace_validation::validate_workspace;
use mojmodelbridge::resource_bridge::copy_source_resources;

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
    /// Validate an exported MCreator workspace and write parity diagnostics.
    ValidateWorkspace {
        /// Workspace directory containing a root-level .mcreator file.
        root: PathBuf,
        /// Skip writing the JSON and text validation report files.
        #[arg(long)]
        no_reports: bool,
    },
    /// Generate MCreator Mojmap model and animation Java files from a Java or class model.
    ExportModelSources {
        source: PathBuf,
        output_root: PathBuf,
        #[arg(long, default_value = "converted_mod")]
        modid: String,
        #[arg(long)]
        class_name: Option<String>,
        #[arg(long, default_value = "neoforge-1.21.1")]
        generator: String,
        #[arg(long)]
        overwrite: bool,
    },
    /// Copy imported mod assets/data while remapping namespaces for the target workspace.
    CopyResources {
        source_root: PathBuf,
        output_resources: PathBuf,
        #[arg(long)]
        source_modid: Option<String>,
        #[arg(long)]
        target_modid: Option<String>,
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
        Command::ValidateWorkspace { root, no_reports } => {
            let report = validate_workspace(&root, None, !no_reports)?;
            println!(
                "Workspace validation: {} ({} error(s), {} warning(s)).",
                if report.ok { "PASS" } else { "FAIL" },
                report.errors.len(),
                report.warnings.len()
            );
            if !no_reports {
                println!("Reports: {}/MOJMODELBRIDGE_WORKSPACE_VALIDATION.[json|txt]", root.display());
            }
            for issue in report.errors.iter().chain(report.warnings.iter()) {
                eprintln!("[{}:{}] {}", issue.severity.to_ascii_uppercase(), issue.code, issue.message);
            }
            if !report.ok {
                std::process::exit(2);
            }
        }
        Command::ExportModelSources { source, output_root, modid, class_name, generator, overwrite } => {
            let result = export_model_sources(
                &source, &output_root, &modid, class_name.as_deref(), &generator, overwrite
            )?;
            println!(
                "Generated MCreator model {}: {} part(s), {} cube(s).",
                result.class_name, result.part_count, result.cube_count
            );
            println!("Model Java: {}", result.model_path.display());
            if let Some(path) = result.animation_path {
                println!("Animation Java: {}", path.display());
            }
            for animation in result.animations {
                println!("Animation {} -> {} ({} channels, {:.3}s).",
                    animation.source_name, animation.field, animation.channel_count, animation.length);
            }
            for warning in result.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::CopyResources { source_root, output_resources, source_modid, target_modid } => {
            let copied = copy_source_resources(
                &source_root,
                &output_resources,
                source_modid.as_deref(),
                target_modid.as_deref(),
            )?;
            println!("Copied {copied} source resource/data file(s) into {}.", output_resources.display());
        }
        Command::Extract { archive, destination } => {
            let path = mojmodelbridge::archive::extract_archive(&archive, &destination)?;
            println!("Extracted {} to {}", archive.display(), path.display());
        }
    }
    Ok(())
}
