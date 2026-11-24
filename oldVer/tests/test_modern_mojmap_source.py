from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from model_bridge import model_to_bbmodel, parse_java_model


def test_modern_mojmap_source_preserves_texture_and_animation_logic() -> None:
    source = ROOT / "tests" / "fixtures" / "ModelTheKing.java"
    text = source.read_text(encoding="utf-8")
    model = parse_java_model(text, source.name)

    assert model.source_format == "modern_mojmap_java"
    assert model.texture_width == 2048
    assert model.texture_height == 2048
    assert model.animation_analysis.get("methods") == ["setupAnim", "renderToBuffer"]
    assert "setupAnim" in model.animation_logic
    assert "renderToBuffer" in model.animation_logic

    bbmodel = model_to_bbmodel(model)
    assert bbmodel["resolution"] == {"width": 2048, "height": 2048}
    assert bbmodel["elements"]
    assert bbmodel["elements"][0]["uv_offset"] == [0, 1450]


def test_modern_mojmap_source_bakes_position_animation_without_disjointing() -> None:
    source = ROOT / "tests" / "fixtures" / "ModelTheKing.java"
    text = source.read_text(encoding="utf-8")
    model = parse_java_model(text, source.name)
    bbmodel = model_to_bbmodel(model)

    animations = bbmodel.get("animations", [])
    assert animations
    anim = animations[0]

    # King should keep procedural motion, and the baked position channels should
    # not collapse to a flat zero pose.
    all_position_keyframes = []
    for animator in anim.get("animators", {}).values():
        all_position_keyframes.extend(kf for kf in animator.get("keyframes", []) if kf.get("channel") == "position")

    assert all_position_keyframes
    assert any(
        any(abs(float(point.get(axis, 0.0))) > 1e-6 for axis in ("x", "y", "z"))
        for keyframe in all_position_keyframes
        for point in keyframe.get("data_points", [])
    )
    # The source model may intentionally start from a non-bind runtime pose.
    # Position animation is now rebased against the authored bind pose, so the
    # first keyframe is allowed to be non-zero when the source says so.
    assert any(abs(float(kf.get("time", 0.0))) < 1e-9 for kf in all_position_keyframes)


def test_explicit_render_loop_multiplicity_becomes_real_instances() -> None:
    source = ROOT / "tests" / "fixtures" / "ModelRotator.java"
    text = source.read_text(encoding="utf-8")
    model = parse_java_model(text, source.name)

    assert model.render_instances
    assert {name: len(specs) for name, specs in model.render_instances.items()} == {
        "shape1": 8,
        "shape2": 8,
        "shape3": 8,
    }

    bbmodel = model_to_bbmodel(model)
    assert len(bbmodel["elements"]) == 24
    assert len(bbmodel["groups"]) == 24
