# MojModelBridge — Native Rust Rewrite

This directory is the independent Rust implementation of MojModelBridge 4.x.
The Python 3.4.0 Beta 12 engine under ../oldVer is a permanent, read-only
behavioral reference. The Rust application does not spawn Python, import Python
modules, or require a Python installation. The Windows release workflow compiles
the engine into the native executable.

## NeoForge 1.21.1 documentation index

The complete 1.21.1 NeoForge versioned documentation snapshot is bundled under `docs/neoforge/1.21.1` (63 Markdown pages and 15 category metadata files). Its source commit and per-page provenance are recorded in `manifest.json`; the original Markdown hierarchy and MIT license are retained.

The native Rust crate exposes offline, identifier-aware search commands:

```powershell
cargo run --manifest-path rust/Cargo.toml -- docs-status
cargo run --manifest-path rust/Cargo.toml -- docs-search "DeferredRegister registry"
cargo run --manifest-path rust/Cargo.toml -- docs-search "DataComponentType" --limit 10 --json
cargo run --manifest-path rust/Cargo.toml -- docs-guide blocks
cargo run --manifest-path rust/Cargo.toml -- docs-guide items "DataComponentType" --json
cargo run --manifest-path rust/Cargo.toml -- docs-knowledge --output build/neoforge-1.21.1-knowledge.json
```

Set `MODPORT_NEOFORGE_DOCS` or pass `--docs-root` to search another local corpus. Search ranks titles, headings, paths, and body text, and splits Java/PascalCase identifiers while preserving complete identifiers. The index also extracts structured API symbols, section headings, and conversion-domain tags. Use docs-guide to retrieve implementation references for a specific conversion domain, or docs-knowledge to emit a machine-readable catalogue. Document frequency is precomputed when the index loads, avoiding repeated full-corpus scans during search.

## Native stages implemented

- Project layout discovery, metadata inference and merged compiled/decompiled inventory.
- Bounded, path-safe JAR/ZIP extraction.
- Direct JVM class-file parsing foundations: constant pool, member and Code
  attributes, line numbers, bootstrap method metadata and instruction boundaries.
- Compiled-only content registration recovery from loose `.class` files and
  JAR/ZIP inputs, covering legacy GameRegistry calls, static registry-holder
  fields, modern DeferredRegister supplier lambdas and constructor references,
  plus entity registration APIs. The shared index is reused across exporters.
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
- Native item/tool MCreator element builders and mixed Java/bytecode discovery,
  including legacy registerItem/registerBlock and modern ITEMS/BLOCKS.register
  supplier calls; workspace element, texture, metadata and language-map updates.
- Native MCreator armor set discovery and grouped four-piece export from either
  Java declarations or compiled registered armor subclasses, including independent
  item icons, layer-1/layer-2 texture copying, metadata manifests and localization.
- Native registered block export from source or compiled registrations using
  block/blockstate JSON inference, render-type classification, source model/
  blockstate copying, texture materialization, and available block property recovery.
- Native registered plant export for flower/bush/crop/sapling subclasses, with PlantSchema fields, cross-model texture resolution, source resource copying, metadata and localization.
- Native resource-authored biome/dimension export using biome JSON, dimension JSON and dimension-type JSON to build MCreator elements and custom-biome references.
- Native projectile element export for source- or bytecode-registered arrow/
  throwable/projectile subclasses, pickup-item references and back-linking to
  ranged item/tool definitions.
- Native Gradle/MCreator build bridge: validate the workspace, prefer its Gradle
  wrapper, execute requested tasks, capture compiler diagnostics, and persist
  complete logs plus machine-readable reports. Windows batch wrappers are launched
  through cmd.exe.
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
    cargo run --manifest-path rust/Cargo.toml -- export-all path/to/MCreatorWorkspace path/to/source --source-modid oldmod --compile
    cargo run --manifest-path rust/Cargo.toml -- compile-workspace path/to/MCreatorWorkspace --task build

It currently converts decompiled .java and supported legacy/modern Mojmap .class
models. The legacy compiled animation preview sampler handles a constrained math
subset and the common walk/idle channels; exact loop-period parity, complex control
flow, modern compiled animation extraction, render-loop instance expansion and
the full texture-selection edge-case corpus remain in progress.

## Remaining major engine ports

The bytecode interpreters and initial legacy animation/texture stages now exist,
but still need wider parity coverage. The native `export-all` pipeline orchestrates
equipment, armor, blocks, plants, resource-authored worldgen, renderer-linked living
entities, projectiles and recipes in dependency order, and can optionally invoke Gradle
after export. Compiled-only registration recovery is now a shared first-pass facility;
it does not yet reproduce constructor-only stats, all anonymous/indirect registry
patterns, reflection-driven registration, or arbitrary registration control flow.
Archive class extraction is limited to class entries; resources and textures in raw
JAR files still need to be supplied through extracted resource roots for complete
asset fidelity.
Remaining major parity work includes complete tool material and repair-item inference,
custom armor-model adaptation, detailed block-state/property and loot/drop procedures,
custom plant growth and biome-placement procedures, configured/placed feature
conversion into MCreator generation procedures, custom projectile hit/collision
procedures, fuller compiled JVM renderer/entity-stat reconstruction, modern compiled
animation extraction, advanced animation control-flow and loop baking, and a deeper
MCreator regeneration/Gradle diagnostic bridge with durable batch/resume reporting.

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

The `export-equipment` command discovers Java-registered and compiled-only Item/Tool subclasses, maps native tool stats and food/ranged fields where source evidence exists, copies item icons and `.mcmeta` sidecars, writes item/tool element JSON, and updates the workspace descriptors and English language map. Compiled-only classes use conservative defaults where constructor details cannot yet be reconstructed.

```powershell
mojmodelbridge export-equipment .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
mojmodelbridge export-equipment .\\MCreatorWorkspace .\\DecompiledMod .\\SourceOnly --source-modid examplemod
```

### MCreator armor set export

The `export-armor` command groups Java-declared helmet/body/leggings/boots fields or compiled registered armor subclasses into one native armor element. Compiled-only grouping uses conventional class-name piece suffixes and recovered registry IDs. It resolves each icon independently, copies texture animation sidecars, discovers legacy armor layer-1/layer-2 images and stages them using the MCreator texture prefix convention. When source stats or repair ingredients are not directly recovered, the element uses explicit conservative defaults and records warnings; the custom armor model adapter and less conventional compiled armor layouts remain outstanding.

```powershell
mojmodelbridge export-armor .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
```

### MCreator block export

The `export-blocks` command discovers registered Java or compiled Block subclasses, resolves blockstate variants/multipart models, classifies supported vanilla model parents into MCreator render types, copies blockstate/model JSON with namespace remapping, copies referenced textures and animation metadata, and writes block element/workspace metadata. Hardness, resistance, luminance, gravity/waterlogging, basic sound type and common tool tags are recovered where source evidence is present. Compiled-only blocks currently receive conservative property defaults; custom loot/drop procedures and complete state/property mapping remain outstanding.

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

The `export-all` command invokes the registered native exporters in dependency order: equipment, armor, blocks, plants, worldgen, living entities, projectiles, then recipes. The shared compiled-registration index is reused across stages. The living-entity stage builds a merged ModGraph from the supplied source roots and honors `--source-modid` for generated source/resource paths. Recipe element references are built from the workspace after content exporters have run. Each stage reports exported/skipped counts and warnings; a hard stage error stops the pipeline, and a final workspace validation writes JSON/text reports and surfaces remaining errors. Add `--compile` to run the workspace Gradle build after export, or use `compile-workspace` separately to run a selected Gradle task.

```powershell
mojmodelbridge export-all .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
mojmodelbridge export-all .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod --compile
```

### Gradle / MCreator compilation bridge

`compile-workspace` resolves the workspace directory, runs the native workspace
validator, prefers the workspace's `gradlew` / `gradlew.bat`, and falls back to a
system Gradle installation. It defaults to the `build` task. Compiler output is
saved to `MOJMODELBRIDGE_GRADLE_BUILD.log`, with structured diagnostics and exit
status in `MOJMODELBRIDGE_GRADLE_BUILD.json`.

```powershell
mojmodelbridge compile-workspace .\MCreatorWorkspace
mojmodelbridge compile-workspace .\MCreatorWorkspace --task compileJava
mojmodelbridge compile-workspace .\MCreatorWorkspace --task build --offline
```

Use `export-all --compile` to run the same bridge after the export pipeline. This
stage compiles the Gradle project currently present in the workspace; it does not
silently pretend that MCreator's editor-side regeneration has occurred. When an
MCreator version requires editor regeneration before Gradle sources are current,
open/reload the workspace in MCreator first, then run the bridge to validate the
resulting project.

### MCreator projectile export

The `export-projectiles` command discovers source-instantiated or bytecode-registered arrow/throwable/projectile subclasses, emits ProjectileSchema elements, resolves pickup-item references from registered item fields, and links ranged item/tool elements back to their custom projectile elements. Compiled-only behavior defaults conservatively; custom collision/hit procedures and projectile renderer models remain outstanding.

```powershell
mojmodelbridge export-projectiles .\\MCreatorWorkspace .\\DecompiledMod --source-modid examplemod
```

### Renderer-linked MCreator entity export

Export renderer-linked entities from a mixed source/class mod directory into an existing MCreator workspace. The command creates model and AnimationDefinition Java sources, copies them into the workspace package, resolves body textures or writes placeholders, generates living-entity element JSON, and updates the workspace manifest and English language map. It does not yet port item/tool/armor/block/plant/biome/dimension/projectile exports or the full Gradle compile bridge.

```powershell
mojmodelbridge export-mcreator-entities .\ExtractedMod .\MCreatorWorkspace --source-root .\DecompiledSources
mojmodelbridge export-mcreator-entities .\ExtractedMod .\MCreatorWorkspace --overwrite
```
