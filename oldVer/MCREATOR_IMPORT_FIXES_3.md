# MCreator NeoForge 1.21.1 Import Fixes 3

## Fixed

- New item/tool definitions now use the canonical native model sentinel `customModelName: "Normal"` instead of an empty custom model name when `renderType=0`.
- New imported element schemas use the bundled canonical MCreator FV89 schemas first, rather than inheriting arbitrary template element state from a complex existing element.
- Workspace validation now catches missing/empty definitions, element type mismatches, future format versions, and invalid native model sentinel combinations before MCreator loads the workspace.
- Legacy/modern recipe JSONs are now converted into actual MCreator `recipe` elements for crafting, shapeless crafting, smelting, blasting, smoking, campfire cooking, stone cutting, and smithing patterns where representable.
- Imported recipe references map source-mod registries to the actual imported MCreator element names.
- Source registry aliases are retained on imported descriptors so template collision renaming can be propagated into copied worldgen/data references.
- Copied template-import data resources are rewritten after import to target the actual registered imported block/item IDs, preventing stale source registry keys from crashing NeoForge worldgen.

## Validation

- Python regression suite: 62/62 passing.
- Workspace preflight: matching `.mod.json` for every descriptor; no empty definitions detected in tested output.
