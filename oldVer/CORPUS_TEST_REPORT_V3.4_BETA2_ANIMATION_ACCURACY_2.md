# MojModelBridge v3.4 Beta 2 — Animation Accuracy Pass 2 Corpus Report

## Validation corpus

Clean-source corpus: `models.zip`

- Model Java inputs discovered: 109
- Successful model exports: 108
- Conversion failures: 0
- Native-animation models: 95
- Native animation clips: 148

## Main fixes in this pass

- Animation bind-pose lookup accepts both authored model-part names and generated parser variables.
- Renderer/entity state arrays such as `thighdisplayangle[i]`, `shindisplayangle[i]`, and related indexed state are normalized into scalar channels when the assignments exist in the model source.
- Helper methods receiving ModelPart parameters substitute actual part identities before recursive animation extraction.
- Normal idle/walk previews use the non-attacking branch for `getAttacking()==0` style ternaries instead of forcing the attack branch.
- Animation sampling uses a robust period estimate so tiny incidental frequencies do not make an entire clip crawl.
- The baker searches progressively longer source-time windows for a genuinely closed loop. It no longer forces the final keyframe to frame-zero when the source motion has not actually returned there.
- Sample density increased to 64 samples per clip.

## Regression highlights

- ModelBee: idle animation exported.
- ModelGodzilla: walk + idle animations exported.
- ModelPitchBlack: idle animation exported with non-forced endpoint handling.
- ModelEmperorScorpion: walk + idle animations exported; helper-generated leg channels are recovered.
- ModelGiantRobot: walk + idle animations exported, including renderer-state-derived channels where source assignments are available.
- ModelTheKing: idle animation exported with bind-pose-relative position channels.
- ModelAntRobot: 77 geometry instances from explicit six-pass render multiplicity.
- ModelSpiderRobot: 172 geometry instances from explicit eight-pass render multiplicity.
- ModelRotator: 24 geometry instances from explicit eight-pass render multiplicity.
- ModelPurplePower: 18 geometry instances from explicit repeated renders.

## Important source-data boundary

Some multi-render models read per-instance state from entity-owned/renderer-owned objects such as `getRenderSpiderRobotInfo()` without defining those numeric arrays in the model class itself. The model parser now preserves and normalizes such references, but exact numeric baking requires the corresponding entity/renderer source to be available to the conversion run. No arbitrary values are invented for those channels.
