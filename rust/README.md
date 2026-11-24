# MojModelBridge — Native Rust Rewrite

This directory is the independent Rust implementation of MojModelBridge 4.x.

The Python 3.4.0 Beta 12 engine under [../oldVer](../oldVer) is a permanent,
read-only behavioral reference. The Rust application does not spawn Python,
import Python modules, or require a Python installation. The final application
will compile the engine into its native executable.

## Current implementation status

The initial native foundation includes:

- A serializable model representation carrying the legacy model, animation,
  texture, render-instance, and source-evidence fields.
- Mod layout discovery for model, renderer, texture, animation, and resource roots.
- Metadata inference across mcmod.info, Fabric metadata, Forge/NeoForge
  mods.toml, Java @Mod annotations, and asset namespaces.
- Merged compiled/decompiled root inventory.
- Bounded, path-safe JAR/ZIP extraction.

This is not yet a feature-complete converter. The source/bytecode interpreters,
Blockbench serializer, MCreator exporters, and workspace validator are being
ported in subsequent parity-tested stages.

## Build

Requires Rust stable and Cargo:

    cargo test --manifest-path rust/Cargo.toml
    cargo run --manifest-path rust/Cargo.toml -- inspect /path/to/mod
    cargo run --manifest-path rust/Cargo.toml -- extract mod.jar /path/to/extracted

No Python runtime or JDK is used by the Rust inspection/extraction foundation.
The final JVM class-file interpreter will parse .class files directly in Rust.

## Porting rule

Port behavior and data flow, not Python syntax. Keep the old stage ordering,
selection rules, scoring, defaults, deterministic identifiers, warnings,
serialization shape, and fallback behavior unless a change is explicitly
documented and covered by a regression test. Do not silently simplify an
unsupported source construct; retain evidence and emit a warning.
