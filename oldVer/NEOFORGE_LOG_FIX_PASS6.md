# NeoForge 1.21.1 Log Fix Pass 6

## Fixed from user runtime log
- Removed raw legacy `dimension_type`, `dimension`, `worldgen/biome`, configured/placed feature, and raw recipe resource copying that was bypassing MCreator/NeoForge 1.21.1 codecs.
- Added resource-only biome/dimension discovery as native MCreator elements.
- Added native block model detection for cross, crop, grass, flower-pot, cube/cube_all, and multi-face block models.
- Prevented copied source block/item model JSON from overriding MCreator-generated structural models.
- Preserved safe source data and namespace remapping for remaining resources.

## Log-specific root cause
NeoForge 1.21.1 rejected legacy dimension type random-provider JSON of the form:
`{"type":"minecraft:uniform","value":{"min_inclusive":0,"max_inclusive":7}}`
The failing load happened when opening Singleplayer while dynamic registries were loaded.

## Validation
- pytest: 65 passed
- compileall: passed
