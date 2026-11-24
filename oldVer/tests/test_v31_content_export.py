from __future__ import annotations

import json
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from legacy_mod_graph import LegacyModGraph
from mcreator_content_export import export_mcreator_content
from mcreator_export import _canonicalize_workspace_definition, _find_workspace_file


def _copy_templates(source: Path, target: Path) -> None:
    del source  # Regression workspaces are examples only; schemas are generic and bundled.
    target.mkdir(parents=True, exist_ok=True)
    (target / "elements").mkdir(exist_ok=True)
    for name in [
        "ItemSchema.mod.json", "ToolSchema.mod.json", "ArmorSchema.mod.json",
        "BlockSchema.mod.json", "DimensionSchema.mod.json", "BiomeSchema.mod.json",
        "PlantSchema.mod.json", "ProjectileSchema.mod.json",
    ]:
        shutil.copy2(ROOT / "mcreator_templates" / name, target / "elements" / name)


def test_template_selection_prefers_newest_and_canonicalizes() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        old = {"mcreatorVersion": 202500228610, "workspaceSettings": {}, "mod_elements": [{}]}
        new = {"mcreatorVersion": 202600230420, "workspaceSettings": {}, "mod_elements": [{}, {}]}
        (root / "old.mcreator").write_text(json.dumps(old), encoding="utf-8")
        (root / "new.mcreator").write_text(json.dumps(new), encoding="utf-8")
        chosen = _find_workspace_file(root)
        assert chosen is not None and chosen.name == "new.mcreator"
        canonical = _canonicalize_workspace_definition(root, chosen, "converted_mod")
        assert canonical.name == "converted_mod.mcreator"
        assert [p.name for p in root.glob("*.mcreator")] == ["converted_mod.mcreator"]


def test_content_export_recovers_core_legacy_types() -> None:
    sample = ROOT / "mcreator_templates"
    danger = ROOT / "tests" / "fixtures" / "orespawn_content"
    needed = ["OreSpawnMain", "AmethystAxe", "ItemMagicApple", "BlockPizza", "WorldProviderOreSpawn2", "BiomeGenUtopianPlains"]
    files = {name: danger / f"{name}.class" for name in needed}
    assert all(path.exists() for path in files.values())
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        _copy_templates(sample, root)
        workspace = {
            "mod_elements": [],
            "language_map": {"en_us": {}},
            "workspaceSettings": {"modid": "converted"},
        }
        graph = LegacyModGraph(danger, class_files=files)
        result = export_mcreator_content(
            graph, workspace, root, root / "elements", root / "assets/textures", "converted"
        )
        created_types = {item.element_type for item in result.exports if item.status == "created"}
        assert {"armor", "tool", "item", "block"}.issubset(created_types)
        assert "dimension" not in created_types and "biome" not in created_types
        assert (root / "elements" / "AmethystArmor.mod.json").exists()
        assert (root / "elements" / "AmethystAxe.mod.json").exists()
        assert (root / "elements" / "MagicApple.mod.json").exists()
        assert (root / "elements" / "Pizza.mod.json").exists()
        assert not (root / "elements" / "UtopianPlains.mod.json").exists()
        assert not any("ModelBattleAxe" in item.source_class for item in result.exports)
        assert not any("RenderBattleAxe" in item.source_class for item in result.exports)


def test_content_export_supports_decompiled_java_without_class_files() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        _copy_templates(ROOT / "mcreator_templates", root)
        (root / "Registry.java").write_text(
            """
class Registry {
  static Object demoItem = new ItemDemo("wrong_name");
  static Object demoBlock = new BlockDemo("wrong_block");
  static {
    GameRegistry.registerItem(demoItem, "demo_item");
    GameRegistry.registerBlock(demoBlock, "demo_block");
  }
}
""",
            encoding="utf-8",
        )
        (root / "ItemDemo.java").write_text(
            "class ItemDemo extends Item {}", encoding="utf-8"
        )
        (root / "BlockDemo.java").write_text(
            "class BlockDemo extends Block {}", encoding="utf-8"
        )
        graph = LegacyModGraph(root=root, class_files={})
        result = export_mcreator_content(
            graph,
            {
                "mod_elements": [],
                "language_map": {"en_us": {}},
                "workspaceSettings": {"modid": "converted"},
            },
            root,
            root / "elements",
            root / "assets/textures",
            "converted",
        )
        created = {(item.element_type, item.registry_name) for item in result.exports if item.status == "created"}
        assert ("item", "demo_item") in created
        assert ("block", "demo_block") in created
        assert (root / "elements" / "DemoItem.mod.json").exists()
        assert (root / "elements" / "DemoBlock.mod.json").exists()


if __name__ == "__main__":
    test_template_selection_prefers_newest_and_canonicalizes()
    test_content_export_recovers_core_legacy_types()
    print("v3.1 content/template tests passed")
