use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use mojmodelbridge::pipeline::convert_model_source;
use mojmodelbridge::mcreator_model::export_model_sources;
use mojmodelbridge::mcreator_workspace::scan_and_export_living_entities;
use mojmodelbridge::mod_graph::scan_mod_graph;
use mojmodelbridge::scan::scan_project;
use mojmodelbridge::workspace_validation::validate_workspace;
use mojmodelbridge::resource_bridge::copy_source_resources;
use mojmodelbridge::source_bridge::copy_model_sources;
use mojmodelbridge::recipe_bridge::export_recipes_to_workspace;
use mojmodelbridge::mcreator_equipment::export_equipment_to_workspace;
use mojmodelbridge::mcreator_armor::export_armor_to_workspace;
use mojmodelbridge::mcreator_block::export_blocks_to_workspace;
use mojmodelbridge::mcreator_plant::export_plants_to_workspace;
use mojmodelbridge::mcreator_worldgen::export_worldgen_to_workspace;
use mojmodelbridge::mcreator_projectile::export_projectiles_to_workspace;
use mojmodelbridge::mcreator_pipeline::export_all_to_workspace;
use mojmodelbridge::gradle_bridge::compile_workspace;
use mojmodelbridge::neoforge_docs::{default_docs_root, NeoForgeDocsIndex};
use mojmodelbridge::neoforge_project::{create_neoforge_project, DEFAULT_NEOFORGE_VERSION};

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
    /// Search the bundled NeoForge 1.21.1 documentation.
    DocsSearch {
        query: String,
        #[arg(long, default_value_t = 8)]
        limit: usize,
        #[arg(long)]
        docs_root: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Validate and report the bundled NeoForge documentation corpus.
    DocsStatus {
        #[arg(long)]
        docs_root: Option<PathBuf>,
    },
    /// Export structured API knowledge extracted from the complete documentation set.
    DocsKnowledge {
        #[arg(long)]
        docs_root: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Retrieve NeoForge documentation and API symbols for a conversion domain.
    DocsGuide {
        domain: String,
        query: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        docs_root: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Create a native NeoForge 1.21.1 Gradle/Java project scaffold.
    CreateNeoForgeProject {
        output: PathBuf,
        #[arg(long)]
        mod_id: String,
        #[arg(long)]
        mod_name: String,
        #[arg(long)]
        package: String,
        #[arg(long, default_value = DEFAULT_NEOFORGE_VERSION)]
        neo_version: String,
        #[arg(long)]
        overwrite: bool,
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
    /// Import recipe JSON files as MCreator recipe elements and copy their data resources.
    ExportRecipes {
        workspace_root: PathBuf,
        #[arg(required = true)]
        source_roots: Vec<PathBuf>,
        #[arg(long)]
        source_modid: String,
        #[arg(long)]
        registry_map: Option<PathBuf>,
        #[arg(long, default_value_t = 89)]
        imported_fv: i64,
    },
    /// Export Java-registered item and tool subclasses into an existing MCreator workspace.
    ExportEquipment {
        workspace_root: PathBuf,
        #[arg(required = true)]
        source_roots: Vec<PathBuf>,
        #[arg(long, default_value = "")]
        source_modid: String,
        #[arg(long, default_value_t = 89)]
        imported_fv: i64,
    },
    /// Export registered four-piece armor sets into native MCreator armor elements.
    ExportArmor {
        workspace_root: PathBuf,
        #[arg(required = true)]
        source_roots: Vec<PathBuf>,
        #[arg(long, default_value = "")]
        source_modid: String,
        #[arg(long, default_value_t = 89)]
        imported_fv: i64,
    },
    /// Export registered Java Block subclasses with blockstate/model/texture resources.
    ExportBlocks {
        workspace_root: PathBuf,
        #[arg(required = true)]
        source_roots: Vec<PathBuf>,
        #[arg(long, default_value = "")]
        source_modid: String,
        #[arg(long, default_value_t = 89)]
        imported_fv: i64,
    },
    /// Export registered flower, bush, crop and sapling subclasses as MCreator plant elements.
    ExportPlants {
        workspace_root: PathBuf,
        #[arg(required = true)]
        source_roots: Vec<PathBuf>,
        #[arg(long, default_value = "")]
        source_modid: String,
        #[arg(long, default_value_t = 89)]
        imported_fv: i64,
    },
    /// Export resource-authored biome and dimension JSON as MCreator elements.
    ExportWorldgen {
        workspace_root: PathBuf,
        #[arg(required = true)]
        source_roots: Vec<PathBuf>,
        #[arg(long, default_value = "")]
        source_modid: String,
        #[arg(long, default_value_t = 89)]
        imported_fv: i64,
    },
    /// Export Java arrow/throwable/projectile subclasses and bind pickup/ranged items.
    ExportProjectiles {
        workspace_root: PathBuf,
        #[arg(required = true)]
        source_roots: Vec<PathBuf>,
        #[arg(long, default_value = "")]
        source_modid: String,
        #[arg(long, default_value_t = 89)]
        imported_fv: i64,
    },
    /// Run the complete native content-export pipeline against an existing workspace.
    ExportAll {
        workspace_root: PathBuf,
        #[arg(required = true)]
        source_roots: Vec<PathBuf>,
        #[arg(long, default_value = "")]
        source_modid: String,
        #[arg(long, default_value_t = 89)]
        imported_fv: i64,
        /// Run the workspace Gradle build after export and native validation.
        #[arg(long)]
        compile: bool,
        /// Run Gradle without resolving remote dependencies.
        #[arg(long)]
        offline: bool,
        /// Gradle task to execute when --compile is selected (repeatable; defaults to build).
        #[arg(long = "compile-task")]
        compile_tasks: Vec<String>,
    },
    /// Compile an existing MCreator workspace through its Gradle wrapper.
    CompileWorkspace {
        root: PathBuf,
        /// Gradle tasks to execute (defaults to build).
        #[arg(long = "task")]
        tasks: Vec<String>,
        /// Do not resolve dependencies from remote repositories.
        #[arg(long)]
        offline: bool,
        /// Skip native workspace validation before compilation.
        #[arg(long)]
        skip_validation: bool,
        /// Return failure when native workspace validation fails.
        #[arg(long)]
        fail_on_validation: bool,
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
    /// Export renderer-linked entities and their MCreator model/animation sources into an existing workspace.
    ExportMcreatorEntities {
        mod_root: PathBuf,
        workspace_root: PathBuf,
        #[arg(long = "source-root")]
        source_roots: Vec<PathBuf>,
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
    /// Copy model and animation Java sources into the target MCreator package.
    CopyModelSources {
        output_root: PathBuf,
        package: String,
        #[arg(long = "model")]
        model_files: Vec<PathBuf>,
        #[arg(long = "animation")]
        animation_files: Vec<PathBuf>,
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
        Command::ExportRecipes { workspace_root, source_roots, source_modid, registry_map, imported_fv } => {
            let registry_to_element = if let Some(path) = registry_map {
                serde_json::from_slice::<std::collections::BTreeMap<String, String>>(&std::fs::read(&path)?)?
            } else {
                std::collections::BTreeMap::new()
            };
            let report = export_recipes_to_workspace(
                &source_roots,
                &workspace_root,
                &source_modid,
                &registry_to_element,
                imported_fv,
            )?;
            println!(
                "Exported {} MCreator recipe element(s) and copied {} recipe resource(s).",
                report.exported.len(),
                report.copied_recipe_resources.len()
            );
            for warning in report.warnings.iter().chain(report.exported.iter().flat_map(|item| item.warnings.iter())) {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::ExportEquipment { workspace_root, source_roots, source_modid, imported_fv } => {
            let report = export_equipment_to_workspace(
                &source_roots,
                &workspace_root,
                &source_modid,
                imported_fv,
            )?;
            println!(
                "MCreator equipment export: {} discovered, {} exported, {} skipped.",
                report.discovered,
                report.exported,
                report.skipped
            );
            println!("Updated workspace: {}", report.workspace_file.display());
            for item in report.records {
                println!(
                    "{} [{}] -> {} ({}, texture={})",
                    item.class_name,
                    item.registry_name,
                    item.element_name,
                    item.element_type,
                    item.texture
                );
                for warning in item.warnings {
                    eprintln!("[WARN] {}: {warning}", item.class_name);
                }
            }
            for warning in report.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::ExportArmor { workspace_root, source_roots, source_modid, imported_fv } => {
            let report = export_armor_to_workspace(
                &source_roots,
                &workspace_root,
                &source_modid,
                imported_fv,
            )?;
            println!(
                "MCreator armor export: {} set(s) discovered, {} exported, {} skipped.",
                report.discovered,
                report.exported,
                report.skipped
            );
            println!("Updated workspace: {}", report.workspace_file.display());
            for item in report.records {
                println!(
                    "{} [{}] -> {} (layer prefix={}, pieces={})",
                    item.prefix,
                    item.registry_name,
                    item.element_name,
                    item.armor_texture_prefix,
                    item.piece_textures.len()
                );
                for warning in item.warnings {
                    eprintln!("[WARN] {}: {warning}", item.prefix);
                }
            }
            for warning in report.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::ExportBlocks { workspace_root, source_roots, source_modid, imported_fv } => {
            let report = export_blocks_to_workspace(
                &source_roots,
                &workspace_root,
                &source_modid,
                imported_fv,
            )?;
            println!(
                "MCreator block export: {} discovered, {} exported, {} skipped.",
                report.discovered,
                report.exported,
                report.skipped
            );
            println!("Updated workspace: {}", report.workspace_file.display());
            for item in report.records {
                println!(
                    "{} [{}] -> {} (render type={}, textures={})",
                    item.class_name,
                    item.registry_name,
                    item.element_name,
                    item.render_type,
                    item.textures.len()
                );
                for warning in item.warnings {
                    eprintln!("[WARN] {}: {warning}", item.class_name);
                }
            }
            for warning in report.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::ExportPlants { workspace_root, source_roots, source_modid, imported_fv } => {
            let report = export_plants_to_workspace(
                &source_roots,
                &workspace_root,
                &source_modid,
                imported_fv,
            )?;
            println!(
                "MCreator plant export: {} discovered, {} exported, {} skipped.",
                report.discovered,
                report.exported,
                report.skipped
            );
            println!("Updated workspace: {}", report.workspace_file.display());
            for item in report.records {
                println!(
                    "{} [{}] -> {} (texture={})",
                    item.class_name,
                    item.registry_name,
                    item.element_name,
                    item.texture
                );
                for warning in item.warnings {
                    eprintln!("[WARN] {}: {warning}", item.class_name);
                }
            }
            for warning in report.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::ExportWorldgen { workspace_root, source_roots, source_modid, imported_fv } => {
            let report = export_worldgen_to_workspace(
                &source_roots,
                &workspace_root,
                &source_modid,
                imported_fv,
            )?;
            println!(
                "MCreator worldgen export: {} biome(s) discovered / {} exported, {} dimension(s) discovered / {} exported, {} skipped.",
                report.biomes_discovered,
                report.biomes_exported,
                report.dimensions_discovered,
                report.dimensions_exported,
                report.skipped
            );
            println!("Updated workspace: {}", report.workspace_file.display());
            for item in report.records {
                println!(
                    "{} [{}] -> {} ({})",
                    item.registry_name,
                    item.element_type,
                    item.element_name,
                    item.source_path
                );
                for warning in item.warnings {
                    eprintln!("[WARN] {}: {warning}", item.registry_name);
                }
            }
            for warning in report.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::ExportProjectiles { workspace_root, source_roots, source_modid, imported_fv } => {
            let report = export_projectiles_to_workspace(
                &source_roots,
                &workspace_root,
                &source_modid,
                imported_fv,
            )?;
            println!(
                "MCreator projectile export: {} discovered, {} exported, {} skipped; {} ranged item(s) linked.",
                report.discovered,
                report.exported,
                report.skipped,
                report.ranged_items_updated
            );
            println!("Updated workspace: {}", report.workspace_file.display());
            for item in report.records {
                println!(
                    "{} [{}] -> {} (pickup item={})",
                    item.class_name,
                    item.registry_name,
                    item.element_name,
                    item.bound_item.as_deref().unwrap_or("unresolved")
                );
                for warning in item.warnings {
                    eprintln!("[WARN] {}: {warning}", item.class_name);
                }
            }
            for warning in report.warnings {
                eprintln!("[WARN] {warning}");
            }
        }
        Command::ExportAll { workspace_root, source_roots, source_modid, imported_fv, compile, offline, compile_tasks } => {
            let report = export_all_to_workspace(
                &source_roots,
                &workspace_root,
                &source_modid,
                imported_fv,
            )?;
            let compile_report = if compile {
                Some(compile_workspace(&workspace_root, &compile_tasks, offline, false, false)?)
            } else {
                None
            };
            let total: usize = report.stages.iter().map(|stage| stage.exported).sum();
            let skipped: usize = report.stages.iter().map(|stage| stage.skipped).sum();
            println!(
                "Native export pipeline finished: {} element(s) exported, {} skipped across {} stages in {:.2}s ({} ms).",
                total,
                skipped,
                report.stages.len(),
                report.total_duration_ms as f64 / 1000.0,
                report.total_duration_ms
            );
            println!(
                "Workspace validation: {}.",
                if report.validation_ok { "PASS" } else { "FAIL" }
            );
            println!("Updated workspace: {}", report.workspace_file.display());
            for stage in report.stages {
                println!(
                    "{}: {} exported, {} skipped ({:.2}s / {} ms)",
                    stage.name,
                    stage.exported,
                    stage.skipped,
                    stage.duration_ms as f64 / 1000.0,
                    stage.duration_ms
                );
            }
            for warning in report.warnings {
                eprintln!("[WARN] {warning}");
            }
            if let Some(build) = compile_report {
                println!(
                    "Gradle compilation: {} (exit={:?}, {:.1}s).",
                    if build.success { "PASS" } else { "FAIL" },
                    build.exit_code, build.duration_seconds
                );
                println!("Build log: {}", build.log_file.display());
                println!("Build report: {}", build.report_file.display());
                for diagnostic in build.diagnostics {
                    eprintln!("[{}] {}", diagnostic.severity.to_ascii_uppercase(), diagnostic.message);
                }
                if !build.success {
                    std::process::exit(3);
                }
            }
        }
        Command::CompileWorkspace { root, tasks, offline, skip_validation, fail_on_validation } => {
            let report = compile_workspace(&root, &tasks, offline, skip_validation, fail_on_validation)?;
            println!(
                "Gradle compilation: {} (exit={:?}, {:.1}s).",
                if report.success { "PASS" } else { "FAIL" },
                report.exit_code, report.duration_seconds
            );
            println!("Build log: {}", report.log_file.display());
            println!("Build report: {}", report.report_file.display());
            if let Some(ok) = report.validation_ok {
                println!(
                    "Pre-build workspace validation: {} ({} error(s), {} warning(s)).",
                    if ok { "PASS" } else { "FAIL" },
                    report.validation_errors, report.validation_warnings
                );
            }
            for diagnostic in report.diagnostics {
                eprintln!("[{}] {}", diagnostic.severity.to_ascii_uppercase(), diagnostic.message);
            }
            if !report.success {
                std::process::exit(3);
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
        Command::ExportMcreatorEntities { mod_root, workspace_root, source_roots, generator, overwrite } => {
            let report = scan_and_export_living_entities(
                &mod_root, &source_roots, &workspace_root, &generator, overwrite
            )?;
            println!(
                "MCreator entity export: {} entity element(s) written, {} skipped.",
                report.exported_entities, report.skipped_entities
            );
            println!("Updated workspace: {}", report.workspace_file.display());
            for entity in report.records {
                println!(
                    "{} [{}] -> {} ({} texture, {} animation(s))",
                    entity.entity_class, entity.registry_name, entity.model_class,
                    entity.texture, entity.animations.len()
                );
                for warning in entity.warnings {
                    eprintln!("[WARN] {}: {warning}", entity.entity_class);
                }
            }
            for warning in report.warnings {
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
        Command::CopyModelSources { output_root, package, model_files, animation_files } => {
            let written = copy_model_sources(&output_root, &package, &model_files, &animation_files)?;
            println!("Copied {} model/animation Java source file(s) into {}.", written.len(), output_root.display());
        }
        Command::DocsSearch { query, limit, docs_root, json } => {
            let root = docs_root.unwrap_or_else(default_docs_root);
            let index = NeoForgeDocsIndex::load(&root)?;
            let results = index.search(&query, limit);
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else if results.is_empty() {
                println!("No NeoForge 1.21.1 documentation matches for: {query}");
            } else {
                println!("NeoForge {} docs — {} result(s) for '{}':", index.manifest().minecraft_version, results.len(), query);
                for (position, result) in results.iter().enumerate() {
                    println!("\n{}. {}  [{:.2}]", position + 1, result.title, result.score);
                    println!("   {} · {}", result.section, result.path);
                    println!("   {}", result.snippet);
                    println!("   {}", result.source_url);
                }
            }
        }
        Command::DocsStatus { docs_root } => {
            let root = docs_root.unwrap_or_else(default_docs_root);
            let index = NeoForgeDocsIndex::load(&root)?;
            let manifest = index.manifest();
            println!("{} documentation — Minecraft {}", manifest.platform, manifest.minecraft_version);
            println!("{} Markdown pages · {} category metadata files · {} bytes", manifest.page_count, manifest.category_count, manifest.total_source_bytes);
            println!("Source: {} @ {}", manifest.source_repository, manifest.source_commit);
            println!("Local corpus: {}", index.root().display());
        }
        Command::DocsKnowledge { docs_root, output } => {
            let root = docs_root.unwrap_or_else(default_docs_root);
            let index = NeoForgeDocsIndex::load(&root)?;
            let knowledge = index.knowledge_entries();
            let data = serde_json::to_string_pretty(&knowledge)?;
            if let Some(path) = output {
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() { std::fs::create_dir_all(parent)?; }
                }
                std::fs::write(&path, format!("{data}\\n"))?;
                println!("Wrote {} structured NeoForge knowledge page(s) to {}.", knowledge.len(), path.display());
            } else {
                println!("{data}");
            }
        }
        Command::DocsGuide { domain, query, limit, docs_root, json } => {
            let root = docs_root.unwrap_or_else(default_docs_root);
            let index = NeoForgeDocsIndex::load(&root)?;
            let results = index.guide(&domain, query.as_deref(), limit);
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else if results.is_empty() {
                println!("No NeoForge {} documentation found for domain '{}'.", index.manifest().minecraft_version, domain);
                println!("Available domains: {}", index.domains().join(", "));
            } else {
                println!("NeoForge {} guidance for '{}' — {} page(s):", index.manifest().minecraft_version, domain, results.len());
                for (position, result) in results.iter().enumerate() {
                    println!("\n{}. {}", position + 1, result.title);
                    println!("   {} · {}", result.section, result.path);
                    if query.is_some() { println!("   Match score: {:.2}", result.score); }
                    println!("   {}", result.snippet);
                    if !result.api_symbols.is_empty() {
                        println!("   API: {}", result.api_symbols.iter().take(16).cloned().collect::<Vec<_>>().join(", "));
                    }
                    println!("   {}", result.source_url);
                }
            }
        }
        Command::CreateNeoForgeProject { output, mod_id, mod_name, package, neo_version, overwrite } => {
            let report = create_neoforge_project(
                &output, &mod_id, &mod_name, &package, &neo_version, overwrite
            )?;
            println!(
                "Created NeoForge {} scaffold for '{}' at {} ({} files).",
                report.minecraft_version, report.mod_id, report.output_root.display(),
                report.generated_files.len()
            );
            for warning in report.warnings {
                eprintln!("[NOTE] {warning}");
            }
        }
        Command::Extract { archive, destination } => {
            let path = mojmodelbridge::archive::extract_archive(&archive, &destination)?;
            println!("Extracted {} to {}", archive.display(), path.display());
        }
    }
    Ok(())
}
