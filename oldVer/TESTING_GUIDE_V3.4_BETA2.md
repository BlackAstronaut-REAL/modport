# MojModelBridge 3.4.0 Beta 2 Desktop Testing Guide

Do not fix every cosmetic issue immediately. Record results first so failures can be grouped by subsystem.

## 1. Input modes

Run the same representative mod in these modes where possible:

- Compiled only
- Decompiled only
- Combined compiled + decompiled via `full_port_combined.bat`

Confirm all three generate non-entity content and that combined mode does not duplicate elements.

## 2. MCreator import/build

- Open the generated `.mcreator` workspace.
- Confirm elements appear in their categories.
- Regenerate code.
- Build the Forge 1.20.1 mod.
- Save the complete MCreator console output for any failure.

## 3. Language and descriptions

Check:

- Entity capitalization and spacing
- Items, tools, weapons, blocks, plants, and armor names
- Custom creative-tab names
- Lost Infinity item descriptions/tooltips
- Registry IDs remain original even when display names are translated

## 4. Equipment values

For several 1.7.10 tools and armor sets compare against the original:

- Durability
- Attack damage
- Mining speed
- Harvest level
- Enchantability
- Armor durability
- Helmet/chest/legs/boots protection

Specifically confirm no ordinary tool has 1,000–9,000 attack damage merely because that is its durability.

## 5. Armor textures

With a complete resource corpus:

- Confirm helmet/chest/leggings/boots inventory icons are distinct and correct.
- Equip the full set and verify layer 1 and layer 2 render correctly.
- Check legacy names such as `ultimate_1.png` / `ultimate_2.png`.
- Record any material whose icons or layers cross-match.

## 6. Models

In Blockbench:

- Open a selection of small, segmented, flying, quadruped, and very large models.
- Verify cubes, UVs, pivots, hierarchy, mirrors, and inflation.
- Compare compiled-only and decompiled-only output for the same model.
- Prioritize King, Queen, Mobzilla/Godzilla, Kraken, Sea Viper, Stink Bug, Basilisk, and Java-only 1.12 armor/entity models.

## 7. Animations

Check clips repeatedly, not only once:

- Does the clip loop without snapping?
- Are all limbs involved rather than only one tail/leg cube?
- Does the initial static pose match the original?
- Does the entity use the correct clip for idle/walk?
- Are attack/flight/swim/other source states present or missing?

Record model name, clip name, affected/missing bones, and whether the problem is in Blockbench, MCreator preview, or Minecraft.

## 8. Tool 3D Models

Test first- and third-person holding plus dropped/GUI views:

- Big Bertha
- Slice
- Royal Guardian Sword
- Blades of Duality
- Shields
- Two-ended or asymmetrical weapons

Check that:

- Inventory uses the intended icon.
- Held view uses the full model.
- The hand grips the actual handle.
- First- and third-person transforms are both sensible.
- Left/right-hand behavior is not mirrored incorrectly.

## 9. Texture identity

Check multiple items/entities whose source classes start with the same mod prefix. Confirm each receives its own referenced texture rather than one shared `OreSpawn*` texture.

## 10. Report format

For each issue provide:

```text
Mod/version:
Input mode: compiled / decompiled / combined
Element/model:
Expected:
Actual:
MCreator error or Minecraft log:
Relevant generated .mod.json/.bbmodel if small:
Screenshot if visual:
```
