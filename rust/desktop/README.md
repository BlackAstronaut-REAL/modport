# MojModelBridge Native Desktop

This is the standalone Rust desktop application. It depends on the shared mojmodelbridge library and does not load or launch oldVer/.

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

- Persistent far-left navigation rail with a collapse control.
- Content browser with mobs, registered blocks, weapons, tools, armor, items, food, and standalone models.
- Registered equipment, blocks, armor sets, and entities are the preferred catalog source; orphaned PNGs remain in neutral asset categories instead of being guessed as tools/weapons/food by filename.
- Mob rows use spawn-egg glyphs; registered item/block rows show texture thumbnails.
- Item names/descriptions use discovered English .lang/en_us.json data and recovered Java localization calls.
- Legacy tooltip strings are recovered from Java addInformation-style list additions, retaining Minecraft § formatting.
- Static component JSON is read from item resource definitions when present.
- Registered legacy item renderers can link a tool/weapon to its Java model and renderer texture.
- Armor sets can link a getArmorModel Java model and their layer_1/layer_2 armor textures.
- Custom Java Edition item/block model JSON can preview cuboid elements, parent inheritance, and per-face UVs.
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

The viewport currently uses a CPU-side triangle rasterizer to produce the preview image, displayed inside the eframe window. It is a working 3D preview foundation, not yet the final GPU mesh renderer. Alpha cutout/translucent layer handling, multiple simultaneous face textures, unusual geometry, pivots/rotations, and animation edge cases still need broader differential validation against Blockbench and oldVer/.

The basic editor fields are draft-only in this prototype. They update the tooltip preview but are not yet written back to a Minecraft resource pack or MCreator workspace. Component JSON is loaded from item resource definitions where available; Java-constructed or runtime-generated component values are not yet comprehensively reconstructed. Translation keys are shown as their raw key because the desktop app does not yet load Minecraft language tables.

## Optional Minecraftia font

The app does not redistribute Minecraftia. To enable it, place a properly licensed copy at:

rust/desktop/assets/fonts/Minecraftia-Regular.ttf

A root-level Minecraftia-Regular.ttf beside the running executable is also recognized. Without the font file, the preview uses the system UI font.

Font page: https://www.dafont.com/Minecraftia.font
