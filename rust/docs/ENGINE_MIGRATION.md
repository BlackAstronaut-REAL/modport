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
| legacy_mod_graph.py::scan_legacy_mod | mod_graph + compiled_registry | Java and compiled entity→renderer→model discovery; shared compiled registration index covers loose class files and JAR/ZIP class entries, legacy registry calls, static content-holder fields, DeferredRegister suppliers and constructor references. Source IDs retain precedence; unusual control flow and complete compiled behavior/stat recovery remain in progress |
| mcreator_export.py model Java/animation generation | mcreator_model | Native Mojmap model geometry and sampled AnimationDefinition Java generation (including large-clip helper splitting and render-instance copies); element/workspace assembly remains unported |
| mcreator_entity.rs::build_living_entity_element / mcreator_workspace.rs::export_living_entities_to_workspace | mcreator_entity + mcreator_workspace | End-to-end renderer-linked living-entity export into an existing workspace: model/animation Java generation, body texture resolution/placeholder, element JSON, metadata manifest and English language map. Content exporters are now orchestrated by mcreator_pipeline; full compiled entity behavior and complete workspace regeneration parity remain in progress |
| mcreator_export.py::export_mcreator_workspace | mcreator_pipeline + native content exporters | Initial dependency-aware export-all orchestration now assembles equipment, armor, blocks, plants, worldgen, living entities, projectiles and recipes, then validates the workspace. Remaining exporters, source scaffolds, registration patching, behavior reconstruction and full batch/resume parity remain in progress |
| mcreator_content_export.py::_resource_biome_definitions / _resource_dimension_definitions / _resource_*_updates | worldgen_bridge | Native resource-only biome/dimension discovery and MCreator field mapping for biome effects, spawn entries, dimension settings and dimension-type flags; full content exporter integration remains pending |
| mcreator_content_export.py::_mcreator_block_model_info / _copy_model_ref_texture_to_workspace | block_model_bridge | Native block/cross/grass/cube model classification, blockstate variant/multipart resolution, source blockstate/model JSON copying with namespace remapping, namespaced texture handling, PNG + mcmeta materialization and collision-safe output; full content exporter integration pending |
| mcreator_content_export.py::_recipe_source_files / _recipe_ref / _recipe_output_ref / _recipe_ingredient_ref / _recipe_mcreator_definition | recipe_bridge | Native recipe discovery and definition mapping for shaped/shapeless crafting, cooking, stonecutting and smithing, source namespace-to-element and vanilla item/block references, recipe element JSON/descriptor emission, workspace updates and data-pack copying; Java addShapedRecipe/addShapelessRecipe recovery is now included; recipe registration edge cases and full content-export integration remain pending |
| mcreator_content_export.py::_item_element / _tool_element / _classify_tool / _recover_tool_stats / _extract_food_info / _ranged_behavior_updates / Java item registry scan | mcreator_equipment + compiled_registry | Native item/tool element builders with Java and compiled-only registration discovery (legacy registerItem/registerBlock and modern DeferredRegister patterns), texture and .mcmeta copying, workspace metadata and language-map updates. Constructor-only stats, exact tool material/repair inference and arbitrary factory/control-flow recovery remain pending |
| mcreator_content_export.py::_group_armor_fields / _armor_piece_registry_aliases / _armor_elements_from_registry | mcreator_armor + compiled_registry | Native four-piece armor grouping from Java declarations and conventional compiled registered armor subclasses; independent icon resolution, armor layer-1/layer-2 discovery/staging, schema JSON and workspace localization. Unusual compiled layouts, custom ModelBiped adaptation, and armor stat/repair inference remain pending |
| mcreator_content_export.py block registry/model/resource export | mcreator_block + compiled_registry | Native legacy and modern source registration plus compiled-only registered Block subclass discovery; BlockSchema emission, available hardness/resistance/light/gravity/waterlogging/sound/tool-tag recovery, block model/render-type inference, blockstate variants/multipart copying, texture + .mcmeta staging and workspace metadata. Compiled constructor properties, detailed state/property conversion, loot/drop procedures remain pending |
| mcreator_content_export.py plant registry export | mcreator_plant | Native registered flower/bush/crop/sapling discovery using the block registry scanner, PlantSchema emission, cross-model texture staging, exact blockstate/model resource copying, texture metadata, and workspace manifest/language updates; growth procedures, biome placement rules, and custom drops remain pending |
| mcreator_content_export.py resource-only biome/dimension export | mcreator_worldgen | Native discovery of data/<modid>/worldgen/biome, dimension and dimension_type JSON, MCreator biome/dimension schema emission, biome-source CUSTOM references, dimension-type flag mapping, biome effects/spawn-table mapping, workspace descriptor and language updates; full source worldgen feature/ore generation integration remains pending |
| mcreator_content_export.py projectile export / _infer_projectile_item_bindings | mcreator_projectile + compiled_registry | Native source-instantiated and bytecode-registered arrow/throwable/projectile discovery, ProjectileSchema writing, pickup-item references, ranged item/tool back-linking and workspace localization. Compiled-only custom hit/collision behavior and projectile renderer model adaptation remain pending |
| mcreator_content_export.py | export/mcreator/content | Remaining content/element exporters not ported |
| mcreator_source_bridge.py::_copy_source_resources | resource_bridge | Native asset/data resource-copy slice ported with namespace remapping, generated-data filtering, sound canonicalization and Forge biome-modifier migration; Java source generation/registry patching remain unported |
| mcreator_source_bridge.py::_copy_model_sources / _copy_animation_metadata / _ensure_workspace_metadata | source_bridge + gradle_bridge | Native Java model/animation copying, package-header synthesis, texture animation metadata and element file manifests. A separate compile-workspace CLI now validates and runs the workspace Gradle wrapper with persistent logs/reports; MCreator editor-side regeneration and complete registry/procedure patching remain pending |
| mcreator_source_bridge.py (remaining functions) | export/mcreator/source_bridge | Not ported: generated entity/item/armor/block Java scaffolds, model resource generation and registry/procedure patching |
| workspace_validator.py::validate_workspace | workspace_validation + gradle_bridge | Native workspace validation with JSON/text reports and duplicate/schema/resource checks, plus an initial Gradle wrapper/system-Gradle compile bridge with compiler diagnostics and persisted build logs/reports. Full generator matrix and MCreator regeneration parity remain to be verified |
| model_bridge.py::batch_convert | pipeline + mcreator_pipeline | Initial dependency-aware `export-all` and per-stage reports exist; general batch conversion, resumable job state, cancellation, and full legacy handoff parity remain outstanding |
| Native staged workspace orchestration | mcreator_pipeline | Dependency-aware `export-all` sequence for equipment, armor, blocks, plants, worldgen, living entities, projectiles and recipes, followed by workspace validation; broader corpus parity, durable resume/cancel, and regeneration-aware build validation remain open |
| Native compiled registration discovery | compiled_registry | Shared index for loose .class files and JAR/ZIP class entries; constant-pool/bytecode registration recovery, static-field class resolution, DeferredRegister supplier and constructor method references; archive class extraction is cached for the exporter run. Raw JAR resource/texture copying still needs integration with the resource bridge |
| Native workspace build | gradle_bridge | Initial `compile-workspace` CLI validates a workspace, invokes gradlew/gradlew.bat or system Gradle, parses compiler diagnostics, and writes durable log/JSON reports. MCreator editor regeneration and broader generator coverage remain open |

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
