# MojModelBridge 3.4.0 Beta 12

MojModelBridge converts legacy and modern Java Minecraft mod content into editable Blockbench models and a Forge 1.20.1 MCreator workspace.

> **Source-first rule:** infer behavior from the original mod whenever possible. OreSpawn, Lost Infinity, Twilight Forest, and all other examples are regression tests—not special cases, content baselines, or reasons to add hardcoded behavior.

This build focuses on fidelity: models, equipment values, language resources, armor textures, item descriptions, animations, and held 3D tools. Static conversion and workspace validation were run here; MCreator, Forge, Minecraft, and Blockbench remain the authoritative runtime tests.

## Recommended workflows

### Separate compiled and decompiled folders

Use this whenever both representations are available:

```bat
full_port_combined.bat "D:\Mods\MyMod\Compiled" "D:\Mods\MyMod\Decompiled" "D:\Converted\MyMod"
```

With an existing MCreator workspace used only as a container/schema reference:

```bat
full_port_combined.bat "D:\Mods\MyMod\Compiled" "D:\Mods\MyMod\Decompiled" "D:\MCreator\Template" "D:\Converted\MyMod"
```

The converter merges both roots into one mod graph:

- Compiled bytecode is preferred for exact executable constants and control flow.
- Decompiled Java supplies readable registry declarations, models, tooltips, language-linked metadata, and source structure.
- Original resources from either tree are indexed together.
- A class/model present in both trees is emitted once, not duplicated.

### One combined mod folder

```bat
full_port.bat "D:\Mods\MyLegacyMod" "D:\Converted\MyLegacyMod"
```

Or with a template/container:

```bat
full_port.bat "D:\Mods\MyLegacyMod" "D:\MCreator\Template" "D:\Converted\MyLegacyMod"
```

### GUI

```bat
run_gui.bat
```

The GUI has separate fields for:

- Combined/mod root
- Compiled root
- Decompiled root
- Blockbench output
- MCreator output
- Optional MCreator template
- Author override

### Direct CLI

```bat
python model_bridge.py --full-port ^
  --compiled-root "D:\Mods\MyMod\Compiled" ^
  --decompiled-root "D:\Mods\MyMod\Decompiled" ^
  --output "D:\Converted\MyMod\Blockbench" ^
  --mcreator-output "D:\Converted\MyMod\MCreatorWorkspace" ^
  --model-type auto
```

Use `--no-overwrite` to resume an interrupted large conversion.

## What 3.4 Beta 2 adds

### Legacy and decompiled model fidelity

- Decompiled 1.7.10–1.12.2 `ModelRenderer`, `ModelBox`, Techne, and Tabula-style geometry.
- MCP and SRG methods such as `addBox`, `func_78789_a`, `func_78790_a`, and direct `ModelBox` construction.
- Renderer-supplied constructor arguments.
- Constructor-time position and rotation arithmetic, including decompiler aliases such as `var10000.field_78797_d += 6`.
- Correct JVM stack consumption for superclass/helper calls in compiled model constructors.
- Hierarchy, pivots, rotations, cubes, UV offsets, mirror state, and inflation.
- Source-only model lookup during MCreator entity generation.

The OreSpawn geometry audit now reports:

```text
109 / 109 concrete models
3,584 cubes
0 compiled/decompiled geometry mismatches
0 parse failures
```

### Animations

- Decompiled Java procedural animation recovery.
- Multiple animated bones retained after Blockbench UUID assignment.
- Same-class helper expansion and recursive helper chains.
- Rotation and position channels.
- Walk/idle classification where identifiable.
- Looping clips end at the same pose where they begin, preventing wrap-to-start snapping.
- Dynamic branch expressions are preserved instead of being overwritten by static fallback branches.

Arbitrary gameplay state machines are not claimed as fully solved. Attack, flight, swim, roar, death, hurt, and custom clips still require visual/runtime verification, and unknown clips should ultimately be retained as `anim1`, `anim2`, and so on.

### Equipment values

The converter now treats these as separate properties:

- Tool durability/max uses
- Item use duration
- Tool material attack bonus
- Explicit damage-vs-entity values
- Harvest level
- Mining efficiency
- Enchantability
- Armor durability and per-slot protection

For 1.7.10, values are recovered from both decompiled `EnumHelper.addToolMaterial` / `addArmorMaterial` source and compiled registry bytecode. A value such as `setMaxDamage(9000)` is no longer used as 9,000 attack damage.

### Language files and descriptions

English `.lang` and `en_us.json` files are used for:

- Item/tool/weapon names
- Block and plant names
- Entity names
- Armor piece names
- Creative tabs
- Tooltip and JEI-style descriptions where a registry identity can be matched

Literal `tooltip.add(...)` strings in decompiled source are also recovered and written to MCreator `specialInformation` when available.

Registry IDs remain unchanged; language resources affect only user-facing text.

### Armor textures

- Inventory icons are resolved independently for helmet, chestplate, leggings, and boots.
- Armor layers are searched only in armor-layer resource locations.
- Legacy `<material>_1.png` and `<material>_2.png` are copied as MCreator-compatible `<material>_layer_1.png` and `_layer_2.png`.
- Exact semantic matches are preferred; a checker placeholder is safer than silently assigning an unrelated item/entity texture.

### Tool 3D Models and held transforms

For 1.8–1.12 resource models, MojModelBridge preserves the original item JSON and exact `display` transforms.

For 1.7 legacy renderers, it follows:

```text
registered item
  -> IItemRenderer
  -> model class and texture
  -> glRotatef / glTranslatef / glScalef
  -> MCreator Tool 3D Model
```

Compiled-only legacy renderers are also supported. When decompiled Java is unavailable, the bytecode fallback follows `registerItemRenderer` registrations, resolves the registered item field, renderer class, model class, and texture literals, and emits the same custom-model path. Decompiled source remains preferred when both representations are supplied because it provides richer transform evidence.

This allows the inventory icon to remain a small 2D texture while the held item uses its original full model and grip/orientation. It is intended for cases such as Big Bertha, Slice, Royal Guardian Sword, Blades of Duality, shields, and other unusually shaped tools.

### Texture identity

Texture matching no longer treats a shared mod prefix as a strong identity. Classes or registries beginning with `OreSpawn`, for example, must still match their own semantic suffix/resource reference instead of all receiving one common texture.

## Source precedence

When evidence conflicts, prefer:

1. Explicit original registration and behavior in source.
2. Compiled bytecode executed by the original mod.
3. Original assets/resources and metadata.
4. English language files.
5. A user-supplied MCreator workspace for schema/reference only.
6. Conservative inference.
7. Safe defaults, reported clearly.

Do not:

- Hardcode OreSpawn, Lost Infinity, Twilight Forest, or another example.
- Hardcode registry names.
- Depend on example workspaces.
- Replace source values with modern assumptions.
- Change registry IDs merely to hide a collision.
- Invent data when it can be recovered.
- Let defaults silently override explicit evidence.

## Regression and smoke results

Automated suite:

```text
45 passed
0 failed
```

Focused audits:

- OreSpawn compiled/decompiled model parity: 109/109 models, 3,584 cubes.
- Decompiled Lost Infinity model conversion: 229 model outputs, no model-not-found failures in the audit corpus.
- Separate compiled + decompiled full-port smoke: compiled model plus Java-only localized item merged into one workspace.
- Dual-root smoke workspace validation: 0 errors, 0 warnings.
- Armor texture fixture: four distinct inventory icons plus both legacy armor layers.
- Held-model fixtures: original resource `display` transforms and legacy GL transforms retained.

The prior Beta 3 Lost Infinity preview was reported by desktop testing to build successfully with working creative tabs, entities, and animations. Beta 3.1 then fixed source-only non-entity discovery in the packaged `full_port` path.

## Known incomplete areas

- Perfect inference of every custom animation state and semantic clip name.
- Custom AI, procedures, GUIs, packets, capabilities, and gameplay state machines.
- Exact custom dimension terrain generators and every biome feature.
- Full projectile movement/collision/effect behavior.
- Arbitrary recipes and loot tables.
- Runtime verification of every held-item grip and armor layer.
- Compiled-only legacy renderer transforms may have less precise GL transform recovery than the decompiled-source path; model/texture identity is recovered, while exact first/third-person transform fidelity still requires runtime validation.
- The planned collapsible spreadsheet-style MCreator review/editor tab.

## Reports

Full ports generate HTML/JSON reports including:

- `conversion_report.html` / `.json`
- `legacy_mod_graph.json`
- `mcreator_export_report.html` / `.json`
- `MOJMODELBRIDGE_WORKSPACE_VALIDATION.html` / `.json`
- `MOJMODELBRIDGE_SOURCE_BRIDGE_REPORT.html` / `.json`

A structural pass is not proof of runtime parity. Review reports, then test in MCreator, Forge, Minecraft, and Blockbench.

## Requirements

- Python 3.10+
- A JDK with `javap` for compiled-class analysis
- MCreator v2026.2 with the Forge 1.20.1 generator for the current target
- Blockbench and Minecraft for visual/runtime testing

Normal conversion uses only the Python standard library. `pytest` is optional and only needed to run the regression suite.


## NeoForge 1.21.1 MCreator target

Standalone MCreator exports default to `neoforge-1.21.1`, matching MCreator 2026.2's native 1.21.1 generator. The generator target uses the `mojmap-1.21.x` model source set and Java 21. Use `--mcreator-generator forge-1.20.1` only when intentionally targeting the legacy Forge workspace path.

See `V3.4_BETA5_CHANGELOG.md` and `RELEASE.txt` for the full source-fidelity fixes.
