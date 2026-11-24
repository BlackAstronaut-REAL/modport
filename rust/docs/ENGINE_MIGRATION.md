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
| model_bridge.py::parse_java_model | source/model | Modern Mojmap and legacy Java parser selection; first-pass render-loop and repeated-helper instance detection |
| model_bridge.py::model_to_bbmodel | blockbench | Initial serializer with Beta 12 UUID namespace, coordinate conversion and explicit repeated-render part expansion |
| model_bridge.py::convert_file | pipeline | Java and supported legacy/modern class input → Blockbench; shared texture index and sidecars |
| legacy_bridge.py::parse_legacy_class | bytecode/legacy_model | Initial direct ModelRenderer constructor interpreter with constructor selection and geometry |
| modern_bytecode_bridge.py::parse_modern_bytecode_class | bytecode/modern_model | Initial MeshDefinition/PartDefinition/CubeListBuilder factory interpreter |
| legacy_bridge.py::_parse_animation | animation/legacy | Symbolic pose/formula extraction, runtime parameter mapping and helper inlining; first-pass evaluator and additive walk/idle preview baking |
| texture_resolver.py::TextureIndex | texture | Initial Java/class reference indexing, model/renderer companion association, resource path/basename scoring, variant ranking and auxiliary exclusion; full edge-case parity pending |
| legacy_mod_graph.py::scan_legacy_mod | mod_graph | Initial Java-source registrations, entity→renderer→model links, constructor args, registry names and source-based entity metadata; compiled registration bytecode and richer stat inference pending |
| mcreator_export.py model Java/animation generation | mcreator_model | Native Mojmap model geometry and sampled AnimationDefinition Java generation (including large-clip helper splitting and render-instance copies); element/workspace assembly remains unported |
| mcreator_entity.rs::build_living_entity_element / mcreator_workspace.rs::export_living_entities_to_workspace | mcreator_entity + mcreator_workspace | Initial end-to-end renderer-linked living-entity export into an existing workspace: model/animation Java generation, body texture resolution/placeholder, element JSON, metadata manifest and English language map; items/blocks/biomes/projectiles and the full compile bridge remain pending |
| mcreator_export.py::export_mcreator_workspace | export/mcreator | Not ported: workspace assembly, entity/content exports, registrations and reports |
| mcreator_content_export.py::_resource_biome_definitions / _resource_dimension_definitions / _resource_*_updates | worldgen_bridge | Native resource-only biome/dimension discovery and MCreator field mapping for biome effects, spawn entries, dimension settings and dimension-type flags; full content exporter integration remains pending |
| mcreator_content_export.py::_mcreator_block_model_info / _copy_model_ref_texture_to_workspace | block_model_bridge | Native block/cross/grass/cube model classification, blockstate variant/multipart resolution, source blockstate/model JSON copying with namespace remapping, namespaced texture handling, PNG + mcmeta materialization and collision-safe output; full content exporter integration pending |
| mcreator_content_export.py::_recipe_source_files / _recipe_ref / _recipe_output_ref / _recipe_ingredient_ref / _recipe_mcreator_definition | recipe_bridge | Native recipe discovery and definition mapping for shaped/shapeless crafting, cooking, stonecutting and smithing, source namespace-to-element and vanilla item/block references, recipe element JSON/descriptor emission, workspace updates and data-pack copying; Java addShapedRecipe/addShapelessRecipe recovery is now included; recipe registration edge cases and full content-export integration remain pending |
| mcreator_content_export.py::_item_element / _tool_element / _classify_tool / _recover_tool_stats / _extract_food_info / _ranged_behavior_updates / Java item registry scan | mcreator_equipment | Native item/tool element schema builders and Java registry discovery (legacy registerItem/registerBlock and modern ITEMS/BLOCKS.register), texture and .mcmeta copying, item/tool element writing, workspace metadata and language-map updates; compiled-only registration recovery and exact material/repair reconstruction remain pending |
| mcreator_content_export.py::_group_armor_fields / _armor_piece_registry_aliases / _armor_elements_from_registry | mcreator_armor | Native Java four-piece armor grouping, registered piece-id association, independent icon resolution, armor layer-1/layer-2 discovery and staging, armor schema JSON, workspace manifest/language updates; compiled-only armor discovery, custom ModelBiped adapter and armor stat/repair inference remain pending |
| mcreator_content_export.py block registry/model/resource export | mcreator_block | Native legacy registerBlock and modern BLOCKS.register source discovery, BlockSchema emission, hardness/resistance/light/gravity/waterlogging/sound/tool-tag extraction, block model render-type inference, blockstate variant/multipart copying, texture + .mcmeta staging and workspace language/metadata updates; compiled-only registration, detailed state/property conversion, loot/drop procedures, and plants remain pending |
| mcreator_content_export.py | export/mcreator/content | Remaining content/element exporters not ported |
| mcreator_source_bridge.py::_copy_source_resources | resource_bridge | Native asset/data resource-copy slice ported with namespace remapping, generated-data filtering, sound canonicalization and Forge biome-modifier migration; Java source generation/registry patching remain unported |
| mcreator_source_bridge.py::_copy_model_sources / _copy_animation_metadata / _ensure_workspace_metadata | source_bridge | Native Java model/animation copy, package-header synthesis, texture animation metadata copy and element file-manifest attachment; compile-bridge generation and registry patching remain unported |
| mcreator_source_bridge.py (remaining functions) | export/mcreator/source_bridge | Not ported: generated entity/item/armor/block Java scaffolds, model resource generation and registry/procedure patching |
| workspace_validator.py::validate_workspace | workspace_validation | Initial native parity port with report output and focused regression tests; corpus parity review remains |
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
