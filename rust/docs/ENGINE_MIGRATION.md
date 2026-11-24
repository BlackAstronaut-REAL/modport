# Beta 12 → Native Rust Engine Port Map

## Source of truth

The immutable reference is oldVer/ (MojModelBridge 3.4.0 Beta 12). The Rust
implementation lives only under rust/. No Python source is copied into the
runtime, and the Rust executable must not invoke Python.

## Legacy pipeline and Rust ownership

| Legacy source / entry point | Native Rust subsystem | Required parity |
|---|---|---|
| model_bridge.py::batch_convert | pipeline | Root merge order, resume behavior, paired model handling, per-file failure isolation, reports, MCreator handoff |
| texture_resolver.py::discover_mod_layout | discovery | Conventional roots, named folders, flat-package fallback, canonical de-duplication |
| texture_resolver.py::TextureIndex | texture | Java/class reference extraction, companion model links, canonicalization, candidate scores, primary/variant/auxiliary selection |
| model_bridge.py Java helpers and parse_java_model | source/java_* | Comment stripping, balanced expression parsing, nested calls, constructor arguments, helper expansion, geometry and modern animations |
| legacy_bridge.py::parse_legacy_class | bytecode/legacy_model | Class-file parsing, constructor selection, operand stack/local slots, ModelRenderer object graph, parent-child relationships |
| legacy_bridge.py::_parse_animation | animation/legacy | JVM expression recovery, helper-method traversal, axis signs, formula scoring, baked previews and additive keyframes |
| modern_bytecode_bridge.py | bytecode/modern_model | Constant-pool/invokedynamic handling, method prioritization, CubeListBuilder / PartDefinition / PartPose reconstruction |
| legacy_mod_graph.py::scan_legacy_mod | graph | Registration discovery, renderer constructor args, entity metadata, registry aliases, source + bytecode merge |
| model_bridge.py::model_to_bbmodel | export/blockbench | Stable UUIDs, hierarchy, cube faces, UVs, texture embedding, preview baking, animation format |
| mcreator_export.py::export_mcreator_workspace | export/mcreator | Template clone, workspace descriptor, entity/model Java, animations, textures, procedures, generator-specific resources |
| mcreator_content_export.py::export_mcreator_content | export/mcreator/content | Items, tools, armor, blocks, plants, recipes, projectiles, biomes, dimensions, worldgen, sounds, language and custom resources |
| mcreator_source_bridge.py::generate_compile_bridge | export/mcreator/source_bridge | Generated source/resources, registry patching, namespace rewrites and compile bridge |
| workspace_validator.py::validate_workspace | validate/workspace | Duplicate IDs, missing element files, malformed definitions, texture references, invalid model values, report shape |
| mod_metadata.py::infer_mod_metadata | metadata | Source priority and fallback merge semantics |

## Shared native intermediate representations

The Rust model IR mirrors the legacy Cube, Part, and ParsedModel structures,
but replaces Python object cycles with stable part-name references. It must
preserve all render-instance, constructor-argument, texture-evidence, animation,
and source-format fields used downstream.

Additional IRs will be introduced for:

- JVM classes: constant pool, class/member metadata, descriptors, code attributes,
  exception tables, bootstrap methods, line tables, and decoded instructions.
- Source evidence: file, bytecode offset, declaration/call span, inferred value,
  confidence, and warning.
- Mod graph: registrations, entities, renderers, constructor arguments,
  registry aliases, and source/bytecode provenance.
- MCreator workspace: raw JSON values plus typed accessors. Unknown fields must
  round-trip unchanged rather than being discarded by rigid structs.

## Compatibility gates

Every ported stage gets fixtures before it is considered complete.

1. Model geometry: same class selection, constructor selection, part order,
   hierarchy, cube count, cube coordinates, pivots, rotations, UV offsets,
   mirror/inflate values, and deterministic Blockbench UUIDs.
2. Animations: same recovered methods, formula count, expression set,
   channel classification, loop endpoints, keyframe timing, rebasing, and
   preview poses. Preserve unknown animations rather than dropping them.
3. Texture resolution: same companion links, candidate ranking, primary
   texture, variants, auxiliary textures, and evidence explanations.
4. Mod graph: same entity/renderer registrations and constructor arguments,
   registry names, and inferred entity attributes.
5. MCreator: compare generated descriptor and element JSON structurally,
   generated Java/resource trees byte-for-byte where stable, and validation
   diagnostics by code/context.
6. Whole pipeline: run OreSpawn, Lost Infinity, Twilight Forest, and Chaos
   Persists regression corpora. Differences must be classified as intentional,
   parity bugs, or unsupported behavior; never silently accepted.

## Performance policy

Rust is not considered faster merely because it is Rust. Profile the complete
pipeline and optimize expensive scans, bytecode interpretation, repeated
resource indexing, and JSON generation only after output parity is established.
Parallelize independent file-level work while preserving deterministic output
ordering and warning order.
