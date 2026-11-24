from pathlib import Path
from tempfile import TemporaryDirectory
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from model_bridge import collect_model_files
from texture_resolver import discover_mod_layout


with TemporaryDirectory() as tmp:
    root = Path(tmp)
    flat = root / "danger" / "example"
    flat.mkdir(parents=True)

    # Legacy duplicate: compiled bytecode should win over decompiled source.
    (flat / "ModelLegacy.java").write_text(
        "public class ModelLegacy extends ModelBase { ModelRenderer body; }",
        encoding="utf-8",
    )
    (flat / "ModelLegacy.class").write_bytes(
        b"\xca\xfe\xba\xbe\x00\x00\x00\x34net/minecraft/client/model/ModelRenderer"
    )

    # Modern duplicate: the readable CubeListBuilder Java source should win.
    (flat / "CreatureModel.java").write_text(
        "class CreatureModel { MeshDefinition m; LayerDefinition l; CubeListBuilder c; }",
        encoding="utf-8",
    )
    (flat / "CreatureModel.class").write_bytes(
        b"\xca\xfe\xba\xbe\x00\x00\x00\x3dnet/minecraft/client/model/geom/builders/CubeListBuilder"
    )

    # Renderer classes remain available to the resource index but are not model inputs.
    (flat / "RenderLegacy.class").write_bytes(
        b"\xca\xfe\xba\xbe\x00\x00\x00\x32net/minecraft/client/model/ModelRenderer"
    )

    layout = discover_mod_layout(root)
    assert flat.resolve() in layout.model_roots

    selected = collect_model_files(layout.model_roots, recursive=True, model_type="auto")
    names = {(path.name, path.suffix) for path in selected}
    assert ("ModelLegacy.class", ".class") in names
    assert ("ModelLegacy.java", ".java") not in names
    assert ("CreatureModel.java", ".java") in names
    assert ("CreatureModel.class", ".class") not in names
    assert not any(path.name.startswith("Render") for path in selected)

print("v2.1 source selection and flat-package discovery tests passed")


def test_parallel_compiled_decompiled_pairing() -> None:
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        compiled = root / "Compiled" / "com" / "example"
        decompiled = root / "Decompiled" / "com" / "example"
        compiled.mkdir(parents=True)
        decompiled.mkdir(parents=True)
        (compiled / "ModelLegacy.class").write_bytes(
            b"\xca\xfe\xba\xbenet/minecraft/client/model/ModelRenderer"
        )
        (decompiled / "ModelLegacy.java").write_text(
            "public class ModelLegacy extends ModelBase { ModelRenderer body; }",
            encoding="utf-8",
        )
        auto = collect_model_files([root], recursive=True, model_type="auto")
        both = collect_model_files([root], recursive=True, model_type="both")
        assert [path.suffix for path in auto] == [".class"], auto
        assert {path.suffix for path in both} == {".java", ".class"}, both
