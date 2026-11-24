from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from legacy_bridge import parse_legacy_class
from legacy_mod_graph import LegacyModGraph, scan_legacy_mod
from mcreator_content_export import (
    _craft_enchantment_calls,
    _enchantment_procedure_xml,
    MCreatorContentExport,
    _copy_texture_with_metadata,
    _infer_projectile_item_bindings,
    _source_creative_tabs,
    _source_field_registered_names,
    _source_worldgen_by_registry,
)
from mcreator_source_bridge import _texture_resource_path
from model_bridge import Cube, Part, ParsedModel
from modern_bytecode_bridge import parse_modern_bytecode_class


def test_mobzilla_preserves_dynamic_if_else_animation_branches() -> None:
    model = parse_legacy_class(
        ROOT / "tests" / "fixtures" / "ModelGodzilla.class",
        Cube,
        Part,
        ParsedModel,
        constructor_args=[0.2],
    )
    animations = model.baked_animation_json["animations"]
    walk = animations["animation.model.walk"]["bones"]
    idle = animations["animation.model.idle"]["bones"]

    # The old linear bytecode pass overwrote these dynamic branches with the
    # constant fallback from the other side of an if/else.
    assert len(walk) >= 20
    assert len(idle) >= 50
    for bone in ("LUpperLeg", "RLegUpper", "LToe1", "RToe1"):
        assert bone in walk
    for bone in ("LUpperArm", "RUpperArm", "TailBase", "LowerJaw"):
        assert bone in idle


def test_modern_compiled_static_helper_recovers_cubes_and_uvs() -> None:
    model = parse_modern_bytecode_class(
        ROOT / "tests" / "fixtures" / "KnightmetalArmorModel.class",
        Cube,
        Part,
        ParsedModel,
    )
    assert len(model.parts) >= 10
    assert model.cube_count >= 10
    assert any((cube.uv_x, cube.uv_y) != (0, 0) for part in model.parts for cube in part.cubes)


def test_source_registry_worldgen_creative_tabs_and_animated_texture_copy() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "Registry.java").write_text(
            """
class Registry {
  static Object pizzaBlock = (BlockPizza)(new BlockPizza("wrong_internal_name"));
  static Object pizzaItem = new ItemPizza("also_wrong");
  static Object titanium = new OreTitanium("titanium_internal");
  static Object titanium_stats;
  static {
    GameRegistry.registerBlock(pizzaBlock, "pizza_block");
    GameRegistry.registerItem(pizzaItem, "pizza");
    GameRegistry.registerBlock(titanium, "titanium_ore");
    titanium_stats = this.get_orestats("Titanium", 99, 3, 4, 0, 20);
    new WorldGenMinable(titanium, titanium_stats.clumpsize);
  }
}
""",
            encoding="utf-8",
        )
        (root / "ItemCombat.java").write_text(
            "class ItemCombat { ItemCombat(){ this.func_77637_a(CreativeTabs.field_78037_j); } }",
            encoding="utf-8",
        )

        fields = _source_field_registered_names(root)
        assert fields["pizzaBlock"] == {"pizza_block"}
        assert fields["pizzaItem"] == {"pizza"}

        worldgen = _source_worldgen_by_registry(root)
        assert "titaniumore" in worldgen
        assert worldgen["titaniumore"]["frequencyPerChunks"] == 3
        assert worldgen["titaniumore"]["frequencyOnChunk"] == 4
        assert worldgen["titaniumore"]["minGenerateHeight"] == 0
        assert worldgen["titaniumore"]["maxGenerateHeight"] == 20
        assert "pizzablock" not in worldgen

        assert _source_creative_tabs(root)["ItemCombat"] == "COMBAT"

        source = root / "animated.png"
        source.write_bytes(b"png-placeholder")
        source.with_name("animated.png.mcmeta").write_text(
            '{"animation":{"frametime":2}}', encoding="utf-8"
        )
        target = root / "out" / "animated.png"
        _copy_texture_with_metadata(source, target)
        assert target.read_bytes() == source.read_bytes()
        assert target.with_name("animated.png.mcmeta").read_text(encoding="utf-8") == '{"animation":{"frametime":2}}'


def test_source_worldgen_prefers_exact_placed_features_and_enchants_follow_helpers() -> None:
    source = ROOT.parent / "srcmod" / "Chaos-Persists-Forge-1.20.1"
    if not source.exists():
        import pytest
        pytest.skip("Historical source fixture is not bundled in this release archive")
    worldgen = _source_worldgen_by_registry(source)
    assert worldgen["chaospersistsoreruby"]["sourceResourceExact"] is True
    assert worldgen["chaospersistsoreruby"]["frequencyPerChunks"] == 10
    assert worldgen["chaospersistsoreruby"]["minGenerateHeight"] == 1
    assert worldgen["chaospersistsoreruby"]["maxGenerateHeight"] == 50
    assert worldgen["chaospersistsoreruby"]["veinSize"] == 5

    ultimate_pickaxe = next(source.rglob("UltimatePickaxe.java"))
    enchants = _craft_enchantment_calls(ultimate_pickaxe.read_text(encoding="utf-8"))
    assert enchants == [("EFFICIENCY", 5), ("FORTUNE", 5)]
    xml = _enchantment_procedure_xml(enchants)
    assert "item_add_enhancement" in xml
    assert "EFFICIENCY" in xml and "FORTUNE" in xml


def test_projectile_binding_prefers_source_registry_field() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "Registry.java").write_text(
            """
class Registry {
  static Object thunderStaff = new ItemThunderStaff("bad_name");
  static { GameRegistry.registerItem(thunderStaff, "thunder_staff"); }
}
""",
            encoding="utf-8",
        )
        (root / "ThunderBolt.java").write_text(
            "class ThunderBolt { Object pickup(){ return Registry.thunderStaff; } }",
            encoding="utf-8",
        )
        (root / "ItemThunderStaff.java").write_text(
            "class ItemThunderStaff {}", encoding="utf-8"
        )
        graph = LegacyModGraph(root=root)
        tool = MCreatorContentExport(
            "ThunderStaff", "thunder_staff", "tool", str(root / "ItemThunderStaff.java"), "created"
        )
        projectile = MCreatorContentExport(
            "ThunderBolt", "thunder_bolt", "projectile", str(root / "ThunderBolt.java"), "created"
        )
        bindings = _infer_projectile_item_bindings(graph, [tool, projectile])
        assert bindings["ThunderBolt"] is tool


def test_model_resource_locations_strip_png_extensions() -> None:
    assert _texture_resource_path("slice.png") == "slice"
    assert _texture_resource_path("weapons/big_bertha.PNG") == "weapons/big_bertha"


def test_generic_112_registry_event_and_lambda_renderer_scanner() -> None:
    javac = shutil.which("javac")
    if not javac:
        return
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        for name in ("EntityExample", "RenderExample", "ModelExample"):
            (root / f"{name}.java").write_text(f"public class {name} {{}}", encoding="utf-8")
        subprocess.run(
            [javac, "EntityExample.java", "RenderExample.java", "ModelExample.java"],
            cwd=root,
            check=True,
            capture_output=True,
        )
        # Source scanners intentionally do not require these snippets to compile;
        # this mirrors a mixed compiled/decompiled mod input.
        (root / "ClientRegistry.java").write_text(
            """
class ClientRegistry {
  void register() {
    RenderingRegistry.registerEntityRenderingHandler(EntityExample.class, manager -> new RenderExample(manager));
  }
}
""",
            encoding="utf-8",
        )
        (root / "RenderExample.java").write_text(
            """
class RenderExample {
  RenderExample(Object manager) { super(manager, new ModelExample(), 0.75F); }
  String texture = "example:textures/entity/example.png";
}
""",
            encoding="utf-8",
        )
        (root / "EntityRegistry.java").write_text(
            """
class EntityRegistry {
  void register() {
    registerModEntity(EntityExample.class, "example_mob", 1, this, 64, 1, true);
  }
}
""",
            encoding="utf-8",
        )
        graph = scan_legacy_mod(root)
        registration = next(reg for reg in graph.registrations if reg.entity_class == "EntityExample")
        assert registration.renderer_class == "RenderExample"
        assert registration.model_class == "ModelExample"
        assert registration.shadow_size == 0.75
        assert graph.registry_names["EntityExample"] == "example_mob"


def test_content_classification_does_not_treat_renderer_layers_as_items():
    from mcreator_content_export import _classify_header

    assert _classify_header("LayerHeldItem", "LayerHeldItemOffset")[0] == "unknown"
    assert _classify_header("Object", "ItemInit")[0] == "unknown"
    assert _classify_header("Object", "BlockRegistry")[0] == "unknown"
    assert _classify_header("Item", "ItemActualContent")[0] == "item"


def test_generic_armor_fields_and_transient_projectile_detection() -> None:
    from mcreator_content_export import _group_armor_fields, _source_instantiated_classes

    text = """
      public static example.ItemCustomArmor RubyHelmet;
      public static example.ItemCustomArmor RubyBody;
      public static example.ItemCustomArmor RubyLegs;
      public static example.ItemCustomArmor RubyBoots;
    """
    assert set(_group_armor_fields(text)["Ruby"]) == {"helmet", "body", "leggings", "boots"}

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "ProjectileBase.java").write_text(
            "class ProjectileBase {}", encoding="utf-8"
        )
        (root / "ThunderBolt.java").write_text(
            "class ThunderBolt extends ProjectileBase {}", encoding="utf-8"
        )
        (root / "ThunderStaff.java").write_text(
            "class ThunderStaff { Object fire(){ return new ThunderBolt(); } }", encoding="utf-8"
        )
        instantiated = _source_instantiated_classes(root)
        assert "ThunderBolt" in instantiated
        assert "ProjectileBase" not in instantiated


def test_model_discovery_rejects_registry_layer_and_abstract_base_classes() -> None:
    from model_bridge import collect_model_files

    javac = shutil.which("javac")
    if not javac:
        return
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "ModelBase.java").write_text("public class ModelBase {}", encoding="utf-8")
        (root / "ModelActual.java").write_text(
            "public class ModelActual extends ModelBase {}", encoding="utf-8"
        )
        (root / "ModelRegistry.java").write_text(
            "public class ModelRegistry { ModelActual model = new ModelActual(); }", encoding="utf-8"
        )
        (root / "AnimatedModelBase.java").write_text(
            "public abstract class AnimatedModelBase extends ModelBase {}", encoding="utf-8"
        )
        subprocess.run(
            [javac, "ModelBase.java", "ModelActual.java", "ModelRegistry.java", "AnimatedModelBase.java"],
            cwd=root,
            check=True,
            capture_output=True,
        )
        names = {path.name for path in collect_model_files([root], recursive=True, model_type="class")}
        assert "ModelActual.class" in names
        assert "ModelRegistry.class" not in names
        assert "AnimatedModelBase.class" not in names
