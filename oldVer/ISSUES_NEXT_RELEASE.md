# MojModelBridge 3.4.0 Beta 2 Fidelity Issue Tracker

All fixes in this release are generic source/bytecode/resource recovery. OreSpawn, Lost Infinity, Twilight Forest, and other examples remain regression tests only.

## Implemented

- [x] Copy and link legacy armor layer textures, including `<material>_1.png` / `<material>_2.png` → MCreator `<material>_layer_1.png` / `_layer_2.png`.
- [x] Keep armor inventory icons separate from armor layers and from unrelated item/entity textures.
- [x] Recover 1.7.10 tool/armor material statistics from decompiled source and compiled registry bytecode.
- [x] Separate durability, use duration, material attack bonus, and explicit entity damage so 1,000–9,000 durability values are not emitted as attack damage.
- [x] Use English `.lang` and `en_us.json` resources for items, tools, blocks, plants, entities, armor pieces, creative tabs, and descriptions.
- [x] Port source tooltip/description strings into MCreator `specialInformation` when the schema supports it.
- [x] Close looping animation endpoints to the initial pose to prevent a visible wrap snap.
- [x] Preserve all Java animation bones after Blockbench UUID assignment instead of collapsing them onto the final animator.
- [x] Accept decompiled 1.7.10–1.12.2 model sources in both standalone model conversion and MCreator entity export.
- [x] Preserve constructor-time position/rotation adjustments in both bytecode and decompiled Java.
- [x] Prevent common prefixes such as `OreSpawn*` from collapsing unrelated content onto one texture.
- [x] Add separate compiled and decompiled roots in the CLI, GUI, and `full_port_combined.bat`.
- [x] Preserve resource item-model `display` transforms for 1.8–1.12 tools/weapons/shields.
- [x] Recover legacy 1.7 `IItemRenderer` model, texture, and GL transforms and emit MCreator Tool 3D Models.
- [x] Preserve inventory icon and held 3D model as separate representations.
- [x] Reach compiled/decompiled geometry parity for all 109 concrete OreSpawn models (3,584 cubes).
- [x] Recover compiled-only legacy `IItemRenderer` registrations and link registered fields to renderer/model/texture evidence.

## Runtime verification still required

- [ ] Confirm every recovered armor layer renders on the player in Minecraft.
- [ ] Confirm first-/third-person Tool 3D Model transforms are correct for Big Bertha, Slice, Royal Guardian Sword, Blades of Duality, shields, and unusual two-ended weapons.
- [ ] Visually inspect animation timing and state selection for King, Queen, Mobzilla, Basilisk, and other complex entities.
- [ ] Confirm custom item descriptions render as intended in the target MCreator/Forge build.
- [ ] Test full-resource conversions where all original textures are present.

## Release rule

Do not hardcode example mods, registry names, assets, or behavior. Prefer explicit original source, bytecode, resources, and language data over inference or defaults.
