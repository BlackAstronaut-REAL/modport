use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    /// Safely extract a JAR/ZIP archive into a new directory.
    Extract {
        archive: PathBuf,
        destination: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect { roots, output } => {
            let report = scan_project(&roots)?;
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, format!("{json}\n"))?;
                println!("Wrote project scan: {}", path.display());
            } else {
                println!("{json}");
            }
        }
        Command::Extract { archive, destination } => {
            let extracted = mojmodelbridge::archive::extract_archive(&archive, &destination)?;
            println!("Extracted {} to {}", archive.display(), extracted.display());
        }
    }
    Ok(())
}
