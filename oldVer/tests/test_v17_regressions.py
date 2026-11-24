from __future__ import annotations

from pathlib import Path
import tempfile
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from legacy_bridge import parse_legacy_class
from model_bridge import Cube, Part, ParsedModel


def test_piglin_absolute_animation_semantics() -> None:
    piglin = parse_legacy_class(ROOT / 'tests' / 'fixtures' / 'ModelDeviantPiglin.class', Cube, Part, ParsedModel)
    walk = piglin.baked_animation_json['animations']['animation.model.walk']['bones']
    idle = piglin.baked_animation_json['animations']['animation.model.idle']['bones']

    # The back pieces are only written by the idle branch in source. The walk clip
    # therefore omits them entirely, while the idle clip preserves the authored
    # bind-pose-relative values and swings opposite wing pieces in opposite directions.
    assert 'back1' not in walk
    assert 'back2' not in walk
    assert 'back3' not in walk
    for bone in ('back1', 'back2', 'back3'):
        assert abs(float(idle[bone]['rotation']['0.000000'][0])) > 1e-6
    final_time = max(idle['back1']['rotation'], key=float)
    assert idle['back1']['rotation'][final_time][0] < 0
    assert idle['back2']['rotation'][final_time][0] > 0
    assert idle['back3']['rotation'][final_time][0] > 0


def test_custom_animated_model_frameworks_parse() -> None:
    cthulhu = parse_legacy_class(ROOT / 'tests' / 'fixtures' / 'ModelCthulhu.class', Cube, Part, ParsedModel)
    tentacle = parse_legacy_class(ROOT / 'tests' / 'fixtures' / 'ModelCthulhuTentacle.class', Cube, Part, ParsedModel)

    assert len(cthulhu.parts) == 31
    assert cthulhu.cube_count == 44
    assert cthulhu.animation_resource == 'lostinfinity:animation/cthulhu/cthulhu.json'

    assert len(tentacle.parts) == 5
    assert tentacle.cube_count == 5
    assert tentacle.animation_resource == 'lostinfinity:animation/cthulhu/tentacle.json'


def test_external_animation_resource_resolution() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        resource = root / 'assets' / 'lostinfinity' / 'animation' / 'cthulhu' / 'cthulhu.json'
        resource.parent.mkdir(parents=True)
        resource.write_text('{}', encoding='utf-8')

        cthulhu = parse_legacy_class(
            ROOT / 'tests' / 'fixtures' / 'ModelCthulhu.class', Cube, Part, ParsedModel,
            resource_roots=[root],
        )
        assert cthulhu.animation_resource_path == str(resource.resolve())
        assert any('External animation JSON detected' in w for w in cthulhu.warnings)


if __name__ == '__main__':
    test_piglin_absolute_animation_semantics()
    test_custom_animated_model_frameworks_parse()
    test_external_animation_resource_resolution()
    print('v1.7 regression tests passed')
