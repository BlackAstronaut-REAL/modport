from pathlib import Path
import json

from mcreator_export import _finalize_worldgen_namespace


def test_worldgen_finalizer_scrubs_unknown_non_target_namespace(tmp_path: Path):
    data = tmp_path / "src" / "main" / "resources" / "data"
    rogue = data / "chaospersists" / "worldgen" / "placed_feature"
    target_cf = data / "cndcc" / "worldgen" / "configured_feature"
    target = data / "cndcc" / "worldgen" / "placed_feature"
    rogue.mkdir(parents=True)
    target_cf.mkdir(parents=True)
    target.mkdir(parents=True)
    (rogue / "ore_ruby_placed.json").write_text(json.dumps({"feature":"chaospersists:ore_ruby"}))
    (target_cf / "ore_ruby.json").write_text(json.dumps({"type":"minecraft:ore","config":{"size":5,"discard_chance_on_air_exposure":0.0,"targets":[]}}))
    (target / "ore_ruby_placed.json").write_text(json.dumps({"feature":"cndcc:ore_ruby"}))
    _finalize_worldgen_namespace(tmp_path, "wrong_inferred_source", "cndcc")
    assert not rogue.exists()
    assert (target / "ore_ruby_placed.json").exists()
