# MojModelBridge v3.4 Beta 2 — Animation / Content Fix Pass 6

## Animation fixes
- Native procedural animation baking now samples 128 points per clip instead of 64 for smoother jaw/teeth and segmented-body motion.
- Removed the jaw-specific amplitude/centering heuristic that could transform source-authored jaw rotations into exaggerated motion.
- Activity/state-gated wing formulas select the animated wing branch during native baking while non-wing state-gated body/jaw formulas retain neutral state behavior. This is pattern-based and not tied to a specific mod.
- Existing selective joint continuity remains **wing-only**.

## MCreator content fixes
- Block and plant model JSON texture references are now resolved to the actual source PNG and materialized into `textures/block/` before MCreator definitions reference them.
- Source registry names remain authoritative even when the target workspace must namespace an imported element to avoid a template collision.
- Legacy armor piece classes are filtered out of generic item/tool import when they already belong to a native MCreator armor element, preventing duplicate armor item variants.
- Armor icon matching has a conservative shared-prefix fallback for legacy names such as material-prefix variations while still requiring the correct armor-piece suffix.
- Mob language keys now emit both registry and `.name` forms, plus element-name forms; class-name aliases and fuzzy localization lookup improve recovery of names such as display-oriented entity names.
- Custom spawn-egg textures are analyzed for their two dominant visible colors and mapped to MCreator's native spawnEggBaseColor/spawnEggDotColor fields when a custom egg texture is present.

## Validation
- 68/68 Python tests pass.
- 108/109 model sources parse through the modern parser (the single LegacySkinModel fixture is intentionally a legacy-format path).
- 95 models emit native animation clips; 148 clips in the clean model corpus.
