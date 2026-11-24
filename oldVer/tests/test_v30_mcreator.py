from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from mcreator_export import animation_definition_java, _write_placeholder_png, _write_standalone_scaffold
from model_bridge import ParsedModel


def test_animation_axis_conversion_and_chunking() -> None:
    bones = {}
    for index in range(40):
        frames = {
            f"{frame / 8:.6f}": [float(frame + 1), float(frame + 2), float(frame + 3)]
            for frame in range(17)
        }
        bones[f"bone{index}"] = {"rotation": frames, "position": frames}
    model = ParsedModel(
        class_name="ModelStress",
        texture_width=64,
        texture_height=64,
        parts=[],
        baked_animation_json={
            "animations": {
                "animation.model.idle": {"loop": True, "bones": bones},
            }
        },
    )
    java, records = animation_definition_java(model, "ModelStress")
    assert records and records[0]["channel_count"] == 80
    assert records[0]["helper_count"] > 1
    # Baked values are already additive ModelPart/Blockbench deltas.
    assert "degreeVec(1.0F, 2.0F, 3.0F)" in java
    assert "posVec(1.0F, 2.0F, 3.0F)" in java
    assert "private static AnimationDefinition.Builder addModelStressIdlePart" in java

    javac = shutil.which("javac")
    if javac:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "ModelStressAnimation.java").write_text(java, encoding="utf-8")
            (root / "Stubs.java").write_text(
                """class AnimationDefinition { static class Builder { static Builder withLength(float f){return new Builder();} Builder looping(){return this;} Builder addAnimation(String s, AnimationChannel c){return this;} AnimationDefinition build(){return new AnimationDefinition();} } }
class AnimationChannel { static class Targets { static final Object ROTATION=new Object(), POSITION=new Object(); } static class Interpolations { static final Object LINEAR=new Object(); } AnimationChannel(Object t, Keyframe... f){} }
class Keyframe { Keyframe(float t,Object v,Object i){} }
class KeyframeAnimations { static Object degreeVec(float x,float y,float z){return new Object();} static Object posVec(float x,float y,float z){return new Object();} }
""",
                encoding="utf-8",
            )
            subprocess.run([javac, "ModelStressAnimation.java", "Stubs.java"], cwd=root, check=True)


def test_standalone_scaffold_and_placeholder() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        _write_standalone_scaffold(root, "converted_mod")
        build = (root / "build.gradle").read_text(encoding="utf-8")
        assert 'base.archivesName = "converted_mod"' in build
        assert not any(line.startswith('\"') for line in build.splitlines())
        assert (root / "settings.gradle").exists()
        assert (root / "src/main/resources/pack.mcmeta").exists()

        png = root / "texture.png"
        _write_placeholder_png(png)
        data = png.read_bytes()
        assert data.startswith(b"\x89PNG\r\n\x1a\n")
        assert len(data) > 64


def test_king_fixture() -> None:
    source = ROOT / "tests" / "fixtures" / "ModelTheKing.class"
    assert source.exists()
    from legacy_bridge import parse_legacy_class
    from model_bridge import Cube, Part

    model = parse_legacy_class(source, Cube, Part, ParsedModel, constructor_args=[0.65])
    assert model.texture_width == 2048 and model.texture_height == 2048
    assert len(model.parts) >= 119
    assert model.cube_count >= 119
    assert model.baked_animation_json.get("animations", {}).get("animation.model.idle")
    channels = model.baked_animation_json["animations"]["animation.model.idle"]["bones"]
    assert any("position" in value for value in channels.values())


if __name__ == "__main__":
    test_animation_axis_conversion_and_chunking()
    test_standalone_scaffold_and_placeholder()
    test_king_fixture()
    print("v3.1 MCreator/King regression tests passed")
