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
- Native Minecraft recipe discovery, MCreator element emission, workspace descriptor updates and data-pack copying for shaped/shapeless crafting, cooking, stonecutting and smithing; item/block/tag references are mapped to MCreator tokens.
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

It currently converts decompiled .java and supported legacy/modern Mojmap .class
models. The legacy compiled animation preview sampler handles a constrained math
subset and the common walk/idle channels; exact loop-period parity, complex control
flow, modern compiled animation extraction, render-loop instance expansion and
the full texture-selection edge-case corpus remain in progress.

## Remaining major engine ports

The bytecode interpreters and initial legacy animation/texture stages now exist,
but still need wider parity coverage. Remaining major ports are compiled JVM renderer-registration and full entity-stat
reconstruction, modern compiled animation extraction, advanced animation control-
flow and loop baking, Remaining MCreator item/tool/armor/block/plant/biome/dimension/projectile exports, full compile bridge and complete batch/resume/report pipeline. The app UI is not considered complete until these use the native engine.

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


### Renderer-linked MCreator entity export

Export renderer-linked entities from a mixed source/class mod directory into an existing MCreator workspace. The command creates model and AnimationDefinition Java sources, copies them into the workspace package, resolves body textures or writes placeholders, generates living-entity element JSON, and updates the workspace manifest and English language map. It does not yet port item/tool/armor/block/plant/biome/dimension/projectile exports or the full Gradle compile bridge.

```powershell
mojmodelbridge export-mcreator-entities .\ExtractedMod .\MCreatorWorkspace --source-root .\DecompiledSources
mojmodelbridge export-mcreator-entities .\ExtractedMod .\MCreatorWorkspace --overwrite
```
