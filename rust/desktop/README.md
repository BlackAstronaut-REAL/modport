# MojModelBridge Native Desktop

This is the standalone Rust desktop application. It depends on the shared mojmodelbridge library and runs without a Python engine.

## Build and run

From the repository root:

~~~powershell
cargo run --manifest-path rust/desktop/Cargo.toml
~~~

Build a release executable:

~~~powershell
cargo build --release --manifest-path rust/desktop/Cargo.toml
~~~

The desktop executable is written to rust/desktop/target/release/mojmodelbridge-desktop.exe on Windows.

## Current desktop prototype

The desktop app is currently a project browser and model/animation preview. The navigation entries for Porter and Build & Test are placeholders; actual NeoForge conversion and generated-mod compilation are currently exposed through the Rust CLI documented in the repository root README.

- Persistent far-left navigation rail with a collapse control.
- Content browser with mobs, registered blocks, weapons, tools, armor, items, food, and standalone models.
- Registered equipment, blocks, armor sets, and entities are the preferred catalog source; orphaned PNGs remain in neutral asset categories instead of being guessed as tools/weapons/food by filename.
- Mob rows use spawn-egg glyphs; registered item/block rows show texture thumbnails.
- Item names/descriptions use discovered English .lang/en_us.json data and recovered Java localization calls.
- Legacy tooltip strings are recovered from Java addInformation-style list additions, retaining Minecraft § formatting.
- Static component JSON is read from item resource definitions when present.
- Registered legacy item renderers can link a tool/weapon to its Java model and renderer texture.
- Armor sets can link a getArmorModel Java model and their layer_1/layer_2 armor textures; leggings select the second armor layer.
- Blockstates are followed to their referenced block model; legacy render IDs can link blocks to custom Java renderer/model classes.
- Block previews infer built-in cube, cross/plant, floor, torch, and wall-torch shapes, and custom cuboid block models use their declared per-face texture references.
- Item models support parent inheritance, independent per-face UVs, and simultaneous face-specific texture images.
- Item source properties include legacy getters/material stats and modern food properties; mob details expose recovered health, attack damage, armor, speed, follow range, dimensions, XP, AI goals, and a `Special abilities: true/false` label.
- Textured hierarchical model viewport using ParsedModel -> ViewerScene; the viewer reuses the existing model/animation parser.
- Orbit, pan, zoom, animation selection, play/pause, and timeline scrubbing.
- Source animation previews close loop clips with an eased return-to-start section; 1.7.10 float-first animation parameters and SRG ModelRenderer pivots have dedicated regression coverage.
- Animated PNG .mcmeta preview supports frame dimensions, order, per-frame durations, and vertical/grid sheets.
- Texture-variant selector in the basic editor.
- Name, lore, and component JSON draft fields with a live Minecraft-style tooltip preview.
- Legacy § formatting, named and hex JSON text colors, bold, italic, underlined, strikethrough, and an obfuscation approximation.
- Optional Minecraftia font loading from a user-provided local file.

## Prototype boundaries

The first browser scan accepts an extracted project/mod directory. Raw JAR/ZIP browser indexing is still in progress. Registered resource entries are preferred, but unrecognized/orphan assets remain incomplete; the browser is not yet a universal registry browser for every loader and legacy registration idiom.

The viewport currently uses a CPU-side triangle rasterizer to produce the preview image, displayed inside the eframe window. It is a working 3D preview foundation, not yet the final GPU mesh renderer. Alpha cutout/translucent layer handling, multipart blockstate assembly, unusual geometry, pivots/rotations, and animation edge cases still need broader differential validation against Blockbench and the committed Rust regression fixtures.

The basic editor fields are draft-only in this prototype. They update the tooltip preview but are not yet written back to a Minecraft resource pack or MCreator workspace. Component JSON is loaded from item resource definitions where available; Java-constructed or runtime-generated component values are not yet comprehensively reconstructed. Translation keys are shown as their raw key because the desktop app does not yet load Minecraft language tables.

## Optional Minecraftia font

The app does not redistribute Minecraftia. To enable it, place a properly licensed copy at:

rust/desktop/assets/fonts/Minecraftia-Regular.ttf

A root-level Minecraftia-Regular.ttf beside the running executable is also recognized. Without the font file, the preview uses the system UI font.

Font page: https://www.dafont.com/Minecraftia.font
