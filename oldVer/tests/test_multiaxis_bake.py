from __future__ import annotations

import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from legacy_bridge import _LegacyPart, _build_baked_preview


def main() -> None:
    parts = {
        "bone": _LegacyPart(
            name="bone",
            rotation=[math.radians(10), math.radians(20), math.radians(30)],
            offset=[0.0, 0.0, 0.0],
        )
    }
    analysis = {
        "formulas": [
            {
                "bone": "bone",
                "channel": "rotation",
                "axis": "x",
                "expression": "sin(limbSwing) * limbSwingAmount",
                "animation_class": "walk_cycle",
            },
            {
                "bone": "bone",
                "channel": "rotation",
                "axis": "y",
                "expression": "cos(ageInTicks * 0.1) * 0.2",
                "animation_class": "idle",
            },
        ]
    }
    baked = _build_baked_preview(analysis, parts)
    walk = baked["animations"]["animation.model.walk"]["bones"]["bone"]["rotation"]
    idle = baked["animations"]["animation.model.idle"]["bones"]["bone"]["rotation"]

    walk0 = walk["0.000000"]
    idle0 = idle["0.000000"]

    assert math.isclose(walk0[1], 0.0, abs_tol=1e-6) and math.isclose(walk0[2], 0.0, abs_tol=1e-6), walk0
    assert math.isclose(idle0[0], 0.0, abs_tol=1e-6) and math.isclose(idle0[2], 0.0, abs_tol=1e-6), idle0
    assert any(abs(v[0]) > 1e-6 for v in walk.values()), "walk X axis did not animate"
    assert any(abs(v[1]) > 1e-6 for v in idle.values()), "idle Y axis did not animate"
    print("PASS: multi-axis/multi-animation bake regression")


if __name__ == "__main__":
    main()
