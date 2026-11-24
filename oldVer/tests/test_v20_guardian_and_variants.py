from __future__ import annotations

import base64
import tempfile
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from legacy_bridge import _Method, _parse_constructor
from texture_resolver import TextureIndex, discover_mod_layout


def test_set_rotation_angle_helper() -> None:
    method = _Method(
        "public Example();",
        [
            (0, "aload_0", ""),
            (1, "new", "class net/minecraft/client/model/ModelRenderer"),
            (2, "dup", ""),
            (3, "aload_0", ""),
            (4, "invokespecial #1", 'Method net/minecraft/client/model/ModelRenderer."<init>":(Lnet/minecraft/client/model/ModelBase;)V'),
            (7, "putfield #2", "Field wing:Lnet/minecraft/client/model/ModelRenderer;"),
            (10, "aload_0", ""),
            (11, "aload_0", ""),
            (12, "getfield #2", "Field wing:Lnet/minecraft/client/model/ModelRenderer;"),
            (15, "ldc #3", "float 0.25f"),
            (17, "ldc #4", "float -0.5f"),
            (19, "ldc #5", "float 1.0f"),
            (21, "invokevirtual #6", "Method setRotationAngle:(Lnet/minecraft/client/model/ModelRenderer;FFF)V"),
        ],
    )
    parts, _texture, _scalars, warnings = _parse_constructor(method)
    assert all("no cubes" in w.lower() for w in warnings), warnings
    assert parts["wing"].rotation == [0.25, -0.5, 1.0], parts["wing"].rotation


def test_mod_root_variant_graph() -> None:
    png = base64.b64decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z9+QAAAAASUVORK5CYII="
    )
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        model_dir = root / "xol/lostinfinity/mob/model/deviant"
        render_dir = root / "xol/lostinfinity/mob/render/deviant"
        tex_dir = root / "assets/lostinfinity/textures/entity/deviant"
        model_dir.mkdir(parents=True)
        render_dir.mkdir(parents=True)
        (tex_dir / "super").mkdir(parents=True)
        source = model_dir / "ModelDeviantPiglin.class"
        source.write_bytes(b"ModelDeviantPiglin")
        (render_dir / "RenderDeviantPiglin.java").write_text(
            'class RenderDeviantPiglin { ModelDeviantPiglin m; String a="lostinfinity:textures/entity/deviant/deviantpiglin.png"; String b="lostinfinity:textures/entity/deviant/deviantpiglineye.png"; }',
            encoding="utf-8",
        )
        (render_dir / "RenderSuperDeviantPiglin.java").write_text(
            'class RenderSuperDeviantPiglin { ModelDeviantPiglin m; String a="lostinfinity:textures/entity/deviant/super/superdeviantpiglin.png"; String b="lostinfinity:textures/entity/deviant/super/superdeviantpiglin_red.png"; }',
            encoding="utf-8",
        )
        for rel in (
            "deviantpiglin.png",
            "deviantpiglineye.png",
            "super/superdeviantpiglin.png",
            "super/superdeviantpiglin_red.png",
        ):
            path = tex_dir / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(png)

        layout = discover_mod_layout(root)
        assert layout.model_roots and layout.render_roots and layout.texture_roots
        index = TextureIndex([*layout.model_roots, *layout.render_roots, *layout.texture_roots])
        result = index.resolve_set(source, "ModelDeviantPiglin")
        assert result.primary and result.primary.name == "deviantpiglin.png", result
        assert {p.name for p in result.variants} == {
            "deviantpiglin.png", "superdeviantpiglin.png", "superdeviantpiglin_red.png"
        }, result.variants
        assert {p.name for p in result.auxiliaries} == {"deviantpiglineye.png"}, result.auxiliaries


if __name__ == "__main__":
    test_set_rotation_angle_helper()
    test_mod_root_variant_graph()
    print("v2.0 Guardian/helper and texture-variant tests passed")
