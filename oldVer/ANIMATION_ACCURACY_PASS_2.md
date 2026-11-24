# MojModelBridge v3.4 Beta 2 - Animation Accuracy Pass 2

This pass targets animation timing, source-state propagation, and procedural render-chain fidelity.

## Changes

- Animated bind-state lookup now accepts authored names and parser variables.
- Renderer/entity state arrays such as `thighdisplayangle[i]` and `ydisplayangle[i]` are normalized into scalar source-state channels when their assignments exist in the model source.
- Helper methods that receive `ModelPart` parameters now substitute the actual source part names before recursively extracting their transforms.
- `getAttacking()==0` style branches use the non-attacking state for normal idle/walk previews instead of accidentally forcing attack branches.
- Animation sampling uses a robust source-period estimate instead of the absolute fastest/slowest channel.
- Clips test progressively longer source-time windows and only mark themselves looping when the sampled endpoint actually returns to the start pose. This prevents false end-of-animation snaps.
- Sampling density increased to 64 keys per clip before deduplication.

## Validation

- Full regression suite is updated for source-accurate non-looping endpoints.
- Clean model corpus remains conversion-valid.
- Render-multiplicity geometry remains enabled; runtime state arrays are only baked when the source model actually provides their assignments.
