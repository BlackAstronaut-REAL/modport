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
- The convert-model command writes .bbmodel files and preserves recognized
  procedural animation methods into Java/JSON sidecars.

## Current conversion command

    cargo run --manifest-path rust/Cargo.toml -- convert-model ModelExample.java output/ModelExample.bbmodel
    cargo run --manifest-path rust/Cargo.toml -- convert-model ModelExample.java output/ModelExample.bbmodel --texture-root path/to/assets

It currently converts decompiled .java model source. Compiled .class input,
bytecode interpretation, texture-reference scoring/variant classification, numeric formula sampling,
loop-period inference and Blockbench keyframe baking remain in progress.

## Remaining major engine ports

The legacy compiled-class ModelRenderer interpreter, modern compiled-model
interpreter, symbolic animation interpreter and preview baker, full texture
resolver, entity/renderer mod graph, MCreator workspace/content/source exporters,
workspace validator and complete batch/resume/report pipeline remain to be
ported. The app UI is not considered complete until these use the native engine.

## Parity rules

Preserve source/constructor selection, part order, hierarchy, geometry coordinates,
pivots, rotations, UV offsets, inflate/mirror behavior, UUIDs, texture selection,
animation formulas/timing/loop endpoints, warnings, reports and MCreator output.
Differences must be documented and covered by regression tests against oldVer/.
Never silently simplify unsupported input; retain source evidence and warnings.
