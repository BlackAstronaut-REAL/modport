import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from legacy_bridge import _source_rotation_to_blockbench_degrees, parse_legacy_class
from model_bridge import Cube, Part, ParsedModel


def assert_close(actual, expected, eps=1e-5):
    assert len(actual) == len(expected)
    for a, e in zip(actual, expected):
        assert abs(a - e) <= eps, (actual, expected)


def main():
    # Axis convention used by static groups and baked keyframes must match.
    assert_close(
        _source_rotation_to_blockbench_degrees(10.0, 20.0, 30.0),
        (-10.0, -20.0, 30.0),
    )

    spider = parse_legacy_class(ROOT / 'tests' / 'fixtures' / 'ModelDeviantSpider.class', Cube, Part, ParsedModel)
    assert len(spider.parts) == 21
    assert spider.cube_count == 21
    assert not spider.warnings
    walk = spider.baked_animation_json['animations']['animation.model.walk']['bones']

    # Blockbench keyframes are additive deltas from the static bind pose.
    # At t=0, leg6 animates X only, so Y/Z remain zero deltas.
    expected_leg6 = [
        -math.degrees(0.9),
        0.0,
        0.0,
    ]
    assert_close(walk['leg6']['rotation']['0.000000'], expected_leg6, 1e-4)

    expected_leg1 = [
        math.degrees(0.9),
        0.0,
        11.7,
    ]
    assert_close(walk['leg1']['rotation']['0.000000'], expected_leg1, 1e-4)

    ravager = parse_legacy_class(ROOT / 'tests' / 'fixtures' / 'ModelRavager.class', Cube, Part, ParsedModel)
    assert len(ravager.parts) == 16
    assert ravager.cube_count == 59
    assert ravager.baked_animation_json.get('animations') is not None

    # Generated runtime Java must apply the same sign transform.
    assert 'this.leg6.xRot = -(' in spider.animation_logic
    assert 'this.leg6.zRot = ' in spider.animation_logic

    print('rotation consistency tests passed')


if __name__ == '__main__':
    main()
