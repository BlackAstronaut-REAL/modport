# MojModelBridge — Native Rust Rewrite

This directory is the independent Rust implementation of MojModelBridge 4.x.
The Python 3.4.0 Beta 12 engine under ../oldVer is a permanent, read-only
behavioral reference. The Rust application does not spawn Python, import Python
modules, or require a Python installation. The Windows release workflow compiles
the engine into the native executable.

## Native stages implemented

- Project layout discovery, metadata inference and merged compiled/decompiled inventory.
- Bounded, path-safe JAR/ZIP extraction.
- Direct JVM class-file parsing foundations: constant pool, member and Code
  attributes, line numbers, bootstrap method metadata and instruction boundaries.
- Native legacy compiled animation extraction: pose-field symbolic expressions,
  runtime parameter mapping, same-class helper inlining, and walk/idle/head
  channel classification.
- JVM field/method descriptor parsing, including arrays and wide local slots.
- Shared Java lexical helpers, balanced-delimiter extraction, top-level splitting
  and numeric constant/expression evaluation.
- Legacy ModelBase/ModelRenderer and ModelBox Java-source geometry reconstruction.
- Modern Mojmap PartDefinition/ModelPartData and CubeListBuilder Java-source
  geometry reconstruction.
- Blockbench 5.x serialization with the Beta 12 UUID namespace, coordinate
  conversion, hierarchy, model bounds and embedded PNG textures.
- Companion-aware texture indexing across Java and class sources, exact resource
  reference scoring, body-variant discovery and auxiliary eye/glow exclusion.
- Initial source-driven mod graph connecting entity registrations, renderer classes,
  model classes and constructor arguments, with source-derived entity metadata.
- First-pass legacy JVM animation formula evaluation and additive walk/idle
  Blockbench preview clips, rebased to the converted bind pose.
- Repeated-render source detection for modern Mojmap models, including rotation
  loops and helper-call position variants, expanded into actual Blockbench parts.
- Native MCreator workspace validation, including duplicate registry/name checks, element schema checks, dimension/biome integrity, copied-resource references, automatic BlockItem collisions and machine-readable/text reports.
- Native source resource bridge for namespace-aware assets/data import, sound path canonicalization, generated-worldgen exclusions and Forge-to-NeoForge biome modifier migration.
- Native model/animation Java source copying, texture animation metadata copying and element metadata manifest updates.
- MCreator Mojmap model Java generation and sampled AnimationDefinition export, with helper splitting for large clips and repeated-render instance support.
- Resource-only biome/dimension discovery and conversion of dimension-type flags, biome effects and spawn tables into MCreator definition fields.
- Native block model/blockstate inference for MCreator render types, plus referenced texture materialization with animation sidecars and collision-safe names.
- Native Minecraft recipe discovery, MCreator element emission, workspace descriptor updates and data-pack copying for shaped/shapeless crafting, cooking, stonecutting and smithing; item/block/tag references are mapped to MCreator tokens; legacy addShapedRecipe/addShapelessRecipe Java calls are recovered when JSON recipes are absent.
- Native item/tool MCreator element builders and Java source discovery, including legacy registerItem/registerBlock and modern ITEMS/BLOCKS.register calls; workspace element, texture, metadata and language-map updates.
- Native MCreator armor set discovery and grouped four-piece export, including independent item icons, layer-1/layer-2 texture copying, metadata manifests and localization.
- Native registered block export using block/blockstate JSON inference, render-type classification, source model/blockstate copying, texture materialization, and hardness/resistance/light/tool-property recovery.
- Native registered plant export for flower/bush/crop/sapling subclasses, with PlantSchema fields, cross-model texture resolution, source resource copying, metadata and localization.
- Native resource-authored biome/dimension export using biome JSON, dimension JSON and dimension-type JSON to build MCreator elements and custom-biome references.
- Native projectile element export for arrow/throwable/projectile subclasses, pickup-item references and back-linking to ranged item/tool definitions.
- The convert-model command writes .bbmodel files and preserves recognized
  procedural animation methods into Java/JSON sidecars.

## Current conversion command

    cargo run --manifest-path rust/Cargo.toml -- convert-model ModelExample.java output/ModelExample.bbmodel
    cargo run --manifest-path rust/Cargo.toml -- convert-model ModelExample.java output/ModelExample.bbmodel --texture-root path/to/assets
    cargo run --manifest-path rust/Cargo.toml -- mod-graph path/to/mod --source-root path/to/decompiled-source --output output/mod-graph.json
    cargo run --manifest-path rust/Cargo.toml -- validate-workspace path/to/MCreatorWorkspace
    cargo run --manifest-path rust/Cargo.toml -- copy-resources path/to/source path/to/workspace/src/main/resources --source-modid oldmod --target-modid newmod
    cargo run --manifest-path rust/Cargo.toml -- copy-model-sources path/to/workspace com.example.mod --model path/to/Model.java --animation path/to/ModelAnimation.java
    cargo run --manifest-path rust/Cargo.toml -- export-model-sources path/to/Model.java path/to/MCreatorWorkspace --modid demo
    cargo run --manifest-path rust/Cargo.toml -- export-recipes path/to/MCreatorWorkspace path/to/source --source-modid oldmod --registry-map path/to/registry-map.json
    cargo run --manifest-path rust/Cargo.toml -- export-equipment path/to/MCreatorWorkspace path/to/source --source-modid oldmod
    cargo run --manifest-path rust/Cargo.toml -- export-armor path/to/MCreatorWorkspace path/to/source --source-modid oldmod
    cargo run --manifest-path rust/Cargo.toml -- export-blocks path/to/MCreatorWorkspace path/to/source --source-modid oldmod
    cargo run --manifest-path rust/Cargo.toml -- export-plants path/to/MCreatorWorkspace path/to/source --source-modid oldmod
    cargo run --manifest-path rust/Cargo.toml -- export-worldgen path/to/MCreatorWorkspace path/to/source --source-modid oldmod
    cargo run --manifest-path rust/Cargo.toml -- export-projectiles path/to/MCreatorWorkspace path/to/source --source-modid oldmod
    cargo run --manifest-path rust/Cargo.toml -- export-all path/to/MCreatorWorkspace path/to/source --source-modid oldmod

It currently converts decompiled .java and supported legacy/modern Mojmap .class
models. The legacy compiled animation preview sampler handles a constrained math
subset and the common walk/idle channels; exact loop-period parity, complex control
flow, modern compiled animation extraction, render-loop instance expansion and
the full texture-selection edge-case corpus remain in progress.

## Remaining major engine ports

The bytecode interpreters and initial legacy animation/texture stages now exist,
but still need wider parity coverage. Remaining major ports are compiled JVM renderer-registration and full entity-stat
reconstruction, modern compiled animation extraction, advanced animation control-
flow and loop baking, Remaining MCreator block/plant/biome/dimension/projectile exports, compiled-only equipment registry recovery, full armor defense/material/repair inference, full compile bridge and complete batch/resume/report pipeline. The app UI is not considered complete until these use the native engine.

## Parity rules

Preserve source/constructor selection, part order, hierarchy, geometry coordinates,
pivots, rotations, UV offsets, inflate/mirror behavior, UUIDs, texture selection,
animation formulas/timing/loop endpoints, warnings, reports and MCreator output.
Differences must be documented and covered by regression tests against oldVer/.
Never silently simplify unsupported input; retain source evidence and warnings.


### MCreator model source generation

Generate the Mojmap model and sampled AnimationDefinition Java sources from either decompiled Java or a compiled model class. The files are written to the same staging directories used by the Beta 12 exporter: `models/mojmap-1.21.x` and `models/animations` for NeoForge 1.21.1.

```powershell
mojmodelbridge export-model-sources .\ModelDragon.class .\MCreatorWorkspace --modid examplemod
mojmodelbridge export-model-sources .\ModelDragon.java .\MCreatorWorkspace --modid examplemod --overwrite
```


### Registered MCreator equipment export

The `export-equipment` command discovers Java-registered Item/Tool subclasses, maps native tool stats and food/ranged fields where source evidence exists, copies item icons and `.mcmeta` sidecars, writes item/tool element JSON, and updates the workspace descriptors and English language map. The current discovery stage reads Java source; compiled-only registry reconstruction, armor set grouping, and the remaining block/plant/projectile types are not yet part of this command.

```powershell
mojmodelbridge export-equipment .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
mojmodelbridge export-equipment .\\MCreatorWorkspace .\\DecompiledMod .\\SourceOnly --source-modid examplemod
```

### MCreator armor set export

The `export-armor` command groups Java-declared helmet/body/leggings/boots fields into one native armor element. It resolves each icon independently, copies texture animation sidecars, discovers legacy armor layer-1/layer-2 images and stages them using the MCreator texture prefix convention. When source stats or repair ingredients are not directly recovered, the element uses explicit conservative defaults and records warnings; the custom armor model adapter and compiled-only armor field discovery remain outstanding.

```powershell
mojmodelbridge export-armor .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
```

### MCreator block export

The `export-blocks` command discovers registered Java Block subclasses, resolves blockstate variants/multipart models, classifies supported vanilla model parents into MCreator render types, copies blockstate/model JSON with namespace remapping, copies referenced textures and animation metadata, and writes block element/workspace metadata. Hardness, resistance, luminance, gravity/waterlogging, basic sound type and common tool tags are recovered where source evidence is present. Custom loot/drop procedures, complete state/property mapping, and plants are still separate work.

```powershell
mojmodelbridge export-blocks .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
```

### MCreator plant export

The `export-plants` command discovers registered flower/bush/crop/sapling-style Block subclasses and emits PlantSchema elements with cross-model defaults, copied source blockstate/model resources, texture and `.mcmeta` files, and workspace metadata/localization. It preserves referenced model resources but does not yet reconstruct custom growth procedures, biome placement conditions, or loot/drop behavior.

```powershell
mojmodelbridge export-plants .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
```

### MCreator biome and dimension export

The `export-worldgen` command discovers resource-authored `data/<modid>/worldgen/biome`, `dimension`, and `dimension_type` JSON. It generates native MCreator biome/dimension elements, maps biome source references to `CUSTOM:` entries, carries dimension-type lighting/respawn/coordinate/time/sky flags and biome effects/spawn tables, and updates the workspace descriptor and English language map. It does not yet reconstruct all configured/placed feature generation into MCreator biome/ore procedures.

```powershell
mojmodelbridge export-worldgen .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
```

### Complete native export pipeline

The `export-all` command invokes the registered native exporters in dependency order: equipment, armor, blocks, plants, worldgen, living entities, projectiles, then recipes. Recipe element references are built from the workspace after content exporters have run. Each stage reports exported/skipped counts and warnings; a hard stage error stops the pipeline so it does not silently report a complete conversion.

```powershell
mojmodelbridge export-all .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
```

### MCreator projectile export

The `export-projectiles` command discovers registered or source-instantiated arrow/throwable/projectile subclasses, emits ProjectileSchema elements, resolves pickup-item references from registered item fields, and links ranged item/tool elements back to their custom projectile elements. Custom collision/hit procedures, projectile renderer models, and compiled-only registrations remain to be ported.

```powershell
mojmodelbridge export-projectiles .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
```

### Renderer-linked MCreator entity export

Export renderer-linked entities from a mixed source/class mod directory into an existing MCreator workspace. The command creates model and AnimationDefinition Java sources, copies them into the workspace package, resolves body textures or writes placeholders, generates living-entity element JSON, and updates the workspace manifest and English language map. It does not yet port item/tool/armor/block/plant/biome/dimension/projectile exports or the full Gradle compile bridge.

```powershell
mojmodelbridge export-mcreator-entities .\ExtractedMod .\MCreatorWorkspace --source-root .\DecompiledSources
mojmodelbridge export-mcreator-entities .\ExtractedMod .\MCreatorWorkspace --overwrite
```
