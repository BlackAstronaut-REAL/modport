# MojModelBridge v3.4 Beta 2 — Models Corpus Validation

Corpus: `models.zip`

- Model source files found: 109
- Converter outputs produced: 108 (`LegacySkinModel.java` is a non-entity/support model and is excluded from model export)
- Conversion failures: 0
- Native Blockbench animation-bearing models: 94
- Native Blockbench animation clips: 114
- Existing Python regression suite: 50 passed

## Explicit render multiplicity checks

- `ModelRotator`: 3 source parts expanded to 24 model instances (3 × 8 render passes).
- `ModelSpiderRobot`: explicit 8-pass leg render loop materialized into repeated model instances.
- Runtime per-instance transforms are preserved as source metadata when their values come from entity state and cannot be numerically sampled from the model source alone.

## Remaining native-animation exclusions

The 14 models without native clips fall into two groups:

### Static/content models
`ModelBattleAxe`, `ModelBertha`, `ModelElevator`, `ModelHammy`, `ModelPurplePower`, `ModelQueenBattleAxe`, `ModelRockBase`, `ModelSlice`, `ModelSquidZooka`, `ModelVortex`

These do not contain recoverable model animation clocks in the attached model source.

### Runtime/entity-state-driven render animation
`ModelAntRobot`, `ModelRotator`, `ModelSeaMonster`, `ModelSpiderRobot`

These drive visible motion from runtime renderer/entity state rather than self-contained animation clocks. The converter preserves the Java render logic and materializes explicit repeated-render multiplicity, but does not invent numeric keyframes for unavailable entity state.

## Regression result

`pytest -q tests`: **50 passed**.
