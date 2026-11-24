# MojModelBridge v3.4 Beta 2 — Animation Accuracy Pass 5

- Joint continuity is **wing-only**. Tails, necks, heads, arms, legs, jaws, and other body parts retain source-authored animation positions.
- Custom MCreator imports are organized under a mod-level folder: `~/orespawn/Items`, `~/orespawn/Tools`, `~/orespawn/Blocks`, `~/orespawn/Mobs`, etc. `lostinfinity` uses the readable `~/LostInfinity/...` root.
- Custom 3D item-model textures are mirrored into `assets/<modid>/textures/block/` because MCreator item model JSON texture slots resolve through block textures.
- Godzilla/Mobzilla legacy animation regression remains covered by the existing branch-recovery test.
- PitchBlack wing/tail formulas are recovered from clean source with scalar getter handling intact.

Validation: 50/50 pytest regression tests passed; 108/109 model Java sources parsed from the clean corpus. `LegacySkinModel.java` is intentionally a legacy/non-modern model fixture and is handled by the legacy path rather than `parse_java_model()`.
