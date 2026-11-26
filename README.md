# Modport — Native Rust Mod Converter

Modport is a work-in-progress Minecraft mod migration tool. The active engine and desktop application are written in Rust. The current output target is **Minecraft 1.21.1 with NeoForge**.

## Desktop application

Build and launch the GUI:

```powershell
cargo run --release --manifest-path rust/desktop/Cargo.toml
```

The **Porter** page accepts multiple mod folders, JAR/ZIP archives, decompiled Java roots, and loose Java/class files. It merges the selected source roots, detects source metadata, lets you override the target mod ID/name/package/NeoForge version, generates a NeoForge project, and can automatically run its Gradle build. Conversion and Gradle work run in the background so the UI remains responsive.

The **Build & Test** page can build an already generated Gradle project. It shows the exit code, diagnostics, output tail, and the report/log paths written into the generated project.

## Command-line conversion

The same native library is available from the CLI:

```powershell
cargo build --release --manifest-path rust/Cargo.toml
.\rust\target\release\mojmodelbridge.exe port-neoforge output/converted path/to/legacy-mod.jar
```

Or run it through Cargo:

```powershell
cargo run --release --manifest-path rust/Cargo.toml -- port-neoforge output/converted path/to/legacy-mod.jar
```

The generated project contains Gradle files and launchers (`gradlew.bat` on Windows, `gradlew` on Linux/macOS), a NeoForge `@Mod` entrypoint, Java content registries, resources, creative-tab assignments, and reports at `modport/port-analysis.json` and `modport/port-status.json`. Build it with Java 21:

```powershell
cd output/converted
.\gradlew.bat build
```

The launcher downloads Gradle 9.2.1 into the user cache on its first run; no separate Gradle installation is required.

## Current conversion limits

Modport emits native registered items, tools, armor, blocks, plants, basic living-entity placeholders, supported throwable projectiles, recipes, and imported resources. It is a partial converter: custom AI, attacks, item abilities, event handlers, networking, block-entity logic, and specialized rendering/animation behaviors still require dedicated adapters. The analysis report records warnings and pending behavior rather than claiming a complete 1:1 conversion.

Please credit the original mod authors and obtain permission before redistributing a port. A generated project does not grant rights to the original mod's code or assets.
