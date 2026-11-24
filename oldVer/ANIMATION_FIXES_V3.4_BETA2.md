# MojModelBridge v3.4 Beta 2 - Animation Accuracy Pass

This pass fixes two systemic animation-bake errors found in the clean-source regression corpus:

- Animation channels are now rebased against the authored model bind pose, not the first sampled runtime frame. This preserves source expressions such as `bind + cos(t) * amplitude` as the expected `-amplitude..+amplitude` animation around the authored pose.
- Reused local temporaries are resolved in source order. Decompiled `if/else` fallbacks no longer erase a recoverable dynamic branch with a later static assignment.
- Bind-pose lookup accepts both generated parser variables and authored model-part names.
- Existing render-loop multiplicity support remains enabled and tested.

Validation:
- 50/50 regression tests passed.
- 109 clean-source Java model inputs discovered.
- 108 model outputs generated successfully (one source remains intentionally unsupported/non-model input).
- 95 models emitted native Blockbench animation clips; 147 native clips total.
- Rotator: 24 rendered instances.
- AntRobot: 77 rendered instances.
- SpiderRobot: 172 rendered instances.
- PurplePower: 18 rendered instances.
