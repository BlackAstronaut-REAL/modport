# Beta 12 → Native Rust Engine Port Map

The immutable behavioral reference is oldVer/ (MojModelBridge 3.4.0 Beta 12).
The Rust implementation is independent under rust/ and must not invoke Python.

## Current stage map

| Legacy module / entry point | Native Rust module | State |
|---|---|---|
| texture_resolver.py::discover_mod_layout | discovery | Initial directory-classification port; parity review remains |
| mod_metadata.py::infer_mod_metadata | metadata | Initial metadata discovery and source-priority port |
| model_bridge.py Java lexical helpers | source/java | Balanced extraction, splitting, comments and numeric expressions |
| model_bridge.py::parse_legacy_java_model | source/legacy_model | Initial ModelRenderer/ModelBox geometry parser |
| model_bridge.py::parse_java_model | source/model | Modern Mojmap and legacy Java parser selection |
| model_bridge.py::model_to_bbmodel | blockbench | Initial serializer with Beta 12 UUID namespace and coordinate conversion |
| model_bridge.py::convert_file | pipeline | Java source → Blockbench; basic PNG name matching and sidecars |
| legacy_bridge.py class interpretation | bytecode | Class-file structure, descriptors and instruction boundaries; interpreter pending |
| modern_bytecode_bridge.py | bytecode/modern_model | Class-file foundation exists; modern builder interpreter pending |
| legacy_bridge.py::_parse_animation | animation/legacy | Initial direct JVM symbolic pose/formula extraction, runtime parameter mapping and same-class helper inlining; formula sampling/keyframe baking pending |
| texture_resolver.py::TextureIndex | texture | Not ported: resource references, candidate scoring, primary/variant/auxiliary selection |
| legacy_mod_graph.py::scan_legacy_mod | graph | Not ported: registrations, constructor arguments, entity metadata and aliases |
| mcreator_export.py::export_mcreator_workspace | export/mcreator | Not ported |
| mcreator_content_export.py | export/mcreator/content | Not ported |
| mcreator_source_bridge.py | export/mcreator/source_bridge | Not ported |
| workspace_validator.py | validate/workspace | Not ported |
| model_bridge.py::batch_convert | pipeline | Not ported: mixed roots, resume/cache, reports and MCreator handoff |

## Shared representations

The Rust model IR mirrors Cube, Part and ParsedModel. Parent/child relations use
stable part-name references instead of Python object pointers. The JVM class IR
retains constant-pool entries, method descriptors, Code attributes, exception
tables, line tables, bootstrap methods and instruction offsets.

Further stages must preserve source spans, bytecode offsets, source/constructor
selection evidence, inferred values, confidence and warnings. MCreator JSON
must retain unknown fields so round-tripping does not silently discard data.

## Regression gates

1. Geometry: selected constructor, class, part order, hierarchy, cube count,
   coordinates, pivots, rotations, UV, mirror/inflate and UUIDs.
2. Animations: methods, formulas, channels, loop endpoints, frame timing,
   position rebasing and preview poses.
3. Textures: companion references, candidate ranks, primary texture, variants,
   auxiliary overlays and evidence explanations.
4. Mod graph: same registrations, constructor args, registry aliases and entity attributes.
5. MCreator: structurally equal workspace descriptors/elements and generated source/resources.
6. Whole mod: OreSpawn, Lost Infinity, Twilight Forest and Chaos Persists corpora.

Classify every difference as intentional, parity bug or unsupported. Rust is not
considered faster solely because of its language; profile after parity and
preserve deterministic output while parallelizing.
