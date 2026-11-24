from __future__ import annotations

import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from mcreator_export import _finalize_worldgen_namespace


def test_worldgen_finalizer_keeps_ore_and_removes_biomes_dimensions_villages(tmp_path):
    res = tmp_path / "src" / "main" / "resources"
    target = res / "data" / "cndcc"
    source = res / "data" / "chaospersists"
    for p in [
        target / "worldgen" / "configured_feature",
        target / "worldgen" / "placed_feature",
        target / "worldgen" / "biome",
        target / "worldgen" / "structure",
        target / "worldgen" / "structure_set",
        target / "worldgen" / "template_pool",
        target / "dimension",
        target / "dimension_type",
        target / "neoforge" / "biome_modifier",
    ]: p.mkdir(parents=True, exist_ok=True)
    (target/"worldgen/configured_feature/ruby.json").write_text(json.dumps({"type":"minecraft:ore","config":{"size":5,"targets":[]}}))
    (target/"worldgen/placed_feature/ruby.json").write_text(json.dumps({"feature":"cndcc:ruby","placement":[]}))
    (target/"worldgen/biome/bad.json").write_text('{}')
    (target/"worldgen/structure/boyfriend_village.json").write_text('{}')
    (target/"worldgen/structure_set/bad.json").write_text('{}')
    (target/"worldgen/template_pool/bad.json").write_text('{}')
    (target/"dimension/bad.json").write_text('{}')
    (target/"dimension_type/bad.json").write_text('{}')
    (target/"neoforge/biome_modifier/bad.json").write_text(json.dumps({"type":"neoforge:add_features","features":"cndcc:bad"}))
    (target/"neoforge/biome_modifier/ruby_source_worldgen.json").write_text(json.dumps({"type":"neoforge:add_features","features":"cndcc:ruby"}))
    (source/"worldgen/placed_feature").mkdir(parents=True, exist_ok=True)
    (source/"worldgen/placed_feature/stale.json").write_text('{}')

    _finalize_worldgen_namespace(tmp_path, "chaospersists", "cndcc")

    assert (target/"worldgen/configured_feature/ruby.json").exists()
    assert (target/"worldgen/placed_feature/ruby.json").exists()
    assert not (target/"worldgen/biome").exists()
    assert not (target/"worldgen/structure").exists()
    assert not (target/"worldgen/structure_set").exists()
    assert not (target/"worldgen/template_pool").exists()
    assert not (target/"dimension").exists()
    assert not (target/"dimension_type").exists()
    assert not (target/"neoforge/biome_modifier/bad.json").exists()
    assert (target/"neoforge/biome_modifier/ruby_source_worldgen.json").exists()
    assert not (source/"worldgen/placed_feature").exists()
