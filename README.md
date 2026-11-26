# Modport — Native Rust Mod Converter

Modport is a work-in-progress Minecraft mod migration tool. The active engine and desktop application are written in Rust. The repository no longer contains the former Python conversion engine.

The current output target is **Minecraft 1.21.1 with NeoForge**.

## Port a mod from the command line

The native `port-neoforge` command accepts a mod JAR/ZIP, an extracted mod directory, decompiled Java source, compiled class files, or multiple roots.

Build and run the converter:

```powershell
cargo build --release --manifest-path rust/Cargo.toml
.\rust\target\release\mojmodelbridge.exe port-neoforge output/converted path/to/legacy-mod.jar
```

Or run it directly through Cargo:

```powershell
cargo run --release --manifest-path rust/Cargo.toml -- port-neoforge output/converted path/to/legacy-mod.jar
```

The generated project contains Gradle files, a NeoForge `@Mod` entrypoint, Java content registries, resources, and machine-readable reports at `modport/port-analysis.json` and `modport/port-status.json`.

Build the generated mod:

```powershell
cd output/converted
gradle build
```

The JAR is written to `build/libs/`. The generated project targets Java 21, NeoForge 21.1.235, and Minecraft 1.21.1 by default. An installed JDK 21 and Gradle are needed to build the generated Minecraft mod.

## Build the native desktop app

```powershell
cargo build --release --manifest-path rust/desktop/Cargo.toml
```

The executable is written to `rust/desktop/target/release/mojmodelbridge-desktop.exe` on Windows. The desktop app currently provides the Rust-powered project browser and model/animation preview. Its Porter and Build & Test pages are not yet connected to the direct NeoForge conversion workflow; use the CLI commands above to generate and build a mod.

## Current conversion limits

Modport currently emits native registered items, tools, armor, blocks, plants, basic living-entity placeholders, supported throwable projectiles, recipes, and imported resources. This is a partial converter: custom AI, attacks, item abilities, event handlers, networking, block-entity logic, and many specialized rendering/animation behaviors still require dedicated adapters. The analysis report records warnings and pending behavior instead of claiming a complete 1:1 conversion.

Please credit the original mod authors and obtain permission before redistributing a port. A generated project does not grant rights to the original mod's code or assets.
