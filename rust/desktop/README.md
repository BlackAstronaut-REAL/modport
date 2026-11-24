# MojModelBridge Native Desktop

This is the standalone Rust desktop application. It depends on the shared mojmodelbridge library and does not load or launch oldVer/.

## Build and run

From the repository root:

\`\`\`powershell
cargo run --manifest-path rust/desktop/Cargo.toml
\`\`\`

Build a release executable:

\`\`\`powershell
cargo build --release --manifest-path rust/desktop/Cargo.toml
\`\`\`

The desktop executable is written to rust/desktop/target/release/mojmodelbridge-desktop.exe on Windows.

## Current desktop prototype

- Persistent far-left navigation rail with a collapse control.
- Content browser with category counts for mobs, blocks, weapons, tools, armor, items, food, and standalone models.
- Mob entries reuse the engine's entity → renderer → model registration graph.
- PNG item/block icons and mob spawn-egg glyphs in the asset list.
- Textured hierarchical model viewport using ParsedModel → ViewerScene; no second model parser.
- Orbit, pan, zoom, animation selection, play/pause, and a timeline scrubber.
- Texture-variant selector in the basic editor.
- Name, lore, and component JSON draft fields with a live Minecraft-style tooltip preview.
- Legacy § formatting, named and hex JSON text colors, bold, italic, underlined, strikethrough, and an obfuscation approximation.
- Optional Minecraftia font loading from a user-provided local file.

## Prototype boundaries

The first browser scan accepts an extracted project/mod directory. It indexes registered entities plus item/block texture assets and model source files; item and block rows are currently asset-derived rather than a complete registry-backed catalog. Raw JAR/ZIP browsing, registry-backed item detail, source provenance panels, and full MCreator integration are still in progress.

The viewport currently uses a CPU-side triangle rasterizer to produce the preview image, displayed inside the eframe window. It is a working 3D preview foundation, not yet the final GPU mesh renderer. UV orientation, alpha cutout, uncommon geometry and animation edge cases still need differential validation against Blockbench.

The basic editor fields are draft-only in this prototype. They update the tooltip preview but are not yet written back to a Minecraft resource pack or MCreator workspace. The preview displays Java Edition text component JSON; translation keys are shown as their raw key because the desktop app does not yet load Minecraft language tables.

## Optional Minecraftia font

The app does not redistribute Minecraftia. To enable it, place a properly licensed copy at:

rust/desktop/assets/fonts/Minecraftia-Regular.ttf

A root-level Minecraftia-Regular.ttf beside the running executable is also recognized. Without the font file, the preview uses the system UI font.

Font page: https://www.dafont.com/Minecraftia.font
