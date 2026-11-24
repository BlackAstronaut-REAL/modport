# MojModelBridge 3.4.0 Beta 2 Static Validation Report

## Automated tests

```text
45 passed
0 failed
```

## OreSpawn 1.7.10 geometry parity

A geometry-only audit disabled animation evaluation and parsed the same concrete model from both representations.

```text
Common concrete models: 109
Compiled parses: 109
Decompiled parses: 109
Total cubes: 3,584
Geometry mismatches: 0
Parse failures: 0
```

Compared fields:

- Part identity
- Parent hierarchy
- Rotation points/pivots
- Static rotations
- Cube positions and dimensions
- UV offsets
- Mirror flags
- Inflation

## Decompiled 1.12.2 model path

The Lost Infinity decompiled model audit produced 229 model outputs without the prior MCreator `model not found` path failure. Java-only legacy armor/entity models are accepted by both standalone conversion and MCreator entity generation.

## Dual-root full-port smoke

Inputs:

- Separate compiled root containing a real legacy compiled model fixture
- Separate decompiled root containing Java-only item registration and English language resource

Results:

```text
Compiled model converted: yes (71 cubes)
Java-only item generated: yes
Language display name applied: yes
Creative tab applied: yes
Workspace validation errors: 0
Workspace validation warnings: 0
```

## Equipment recovery

The real compiled OreSpawn registry fixture recovers Amethyst tool/armor material data, including 2,000 uses and material damage 11, without confusing durability for attack damage.

## Resource/held-model fixtures

Validated statically:

- Four independent armor inventory icons
- Both legacy armor layers copied and renamed
- Exact resource item-model `display` transforms retained
- Legacy `IItemRenderer` first-/third-person GL transforms retained
- Animated texture `.mcmeta` sidecars retained
- Language-file descriptions and literal tooltip strings retained

## Environment limitation

This report does not claim MCreator GUI import, generated Forge compilation, Minecraft rendering, or Blockbench visual correctness. Those require the user's desktop toolchain and are covered by `TESTING_GUIDE_V3.4_BETA2.md`.
