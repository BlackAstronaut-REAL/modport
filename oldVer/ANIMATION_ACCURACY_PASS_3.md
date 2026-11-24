# MojModelBridge v3.4 Beta 2 — Animation Accuracy Pass 3

This pass targets three recurring problems in procedural model animation:

1. Joint continuity. When source animation explicitly derives a part's position from a neighboring part, the bake now infers the relationship from the source and uses the actual cube geometry at the parent far edge and child near edge to keep the joint connected at every sampled keyframe.
2. Excessive/incorrect animation duration. Candidate loop windows are searched from source-clock recurrence instead of using a percentile of incidental frequencies. This keeps primary motion periods short while still searching for a real closing window.
3. Loop endpoint snapping. A looping clip no longer duplicates the first keyframe at the final timestamp. The final keyframe is the true sampled end state, allowing Blockbench's loop transition to continue naturally into frame 0.

Other existing behavior is preserved: bind-pose-relative additive animation, explicit render-loop multiplicity, source-ordered temporary evaluation, helper-method recovery, and native Blockbench animation export.

Validation:
- 50/50 repository regression tests passed.
- 108/108 model Java sources in the supplied `models.zip` converted successfully.
- 0 conversion failures in the corpus run.
