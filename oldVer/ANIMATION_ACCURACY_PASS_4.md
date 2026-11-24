# MojModelBridge v3.4 Beta 2 - Animation Accuracy Pass 4

Changes in this pass:

- Joint continuity is now selective. Automatic edge-to-edge joint correction is limited to wing/feather/membrane/fin-like part chains. Tails, necks, heads, arms, and legs retain source-authored positions unless their own source equations explicitly control them.
- Exact source aliases such as `jaw1.xRot = jaw5.xRot` now share the same baked channel rather than being independently re-evaluated and acquiring a bind-pose offset.
- Mirrored left/right anatomy can recover a missing counterpart channel from the animated side when the counterpart is otherwise static. Position X is mirrored; rotation Y/Z are mirrored while X is retained.
- Java ternaries are emitted as lazy Python conditional expressions so unused branches cannot divide by zero or otherwise poison a valid animation.
- Scalar-valued getters such as scale/speed/size/factor/amount are treated as numeric inputs; activity/state getters remain separate boolean-like state inputs.
- Animation clips use a short practical source period rather than an LCM-like multi-second recurrence. Looping clips end at the first moving sample's state (keyframe 2) rather than duplicating the bind/start pose at the end.
- Bind-pose-relative animation and explicit render-loop multiplicity remain enabled.

Validation:

- 50/50 regression tests passed.
- 108/108 clean-source models in models.zip exported successfully.
- 147 native Blockbench animation clips emitted.
