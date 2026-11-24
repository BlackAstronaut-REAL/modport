from __future__ import annotations

import json
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
    _copy_resource_item_model_as_custom,
    _extract_tooltip_strings,
    _armor_layer_texture_stems,
    _bytecode_equipment_stats,
    _armor_elements_from_registry,
    _armor_piece_texture_stems,
    _exact_texture_candidates,
    _find_item_model_json,
    _load_language_names,
    _legacy_renderer_display,
    _language_descriptions,
    _recover_tool_stats,
    _source_armor_stats,
    _source_tool_materials,
    _texture_candidates,
    _translated_name,
    _held_renderer_specs,
)
from mcreator_export import _parse_mcreator_model
from model_bridge import Cube, Part, ParsedModel, model_to_bbmodel, parse_legacy_java_model


def test_tool_damage_does_not_use_durability_or_use_duration() -> None:
    source = """
    class DemoSword extends ItemSword {
      private int weaponDamage = 15;
      DemoSword(){ this.setMaxDamage(9000); }
      public int getMaxItemUseDuration(ItemStack stack){ return 72000; }
      public int getDamageVsEntity(){ return this.weaponDamage; }
    }
    """
    durability, damage, efficiency, enchantability, warnings = _recover_tool_stats(source, "Sword")
    assert durability == 9000
    assert damage == 15
    assert damage not in {9000, 72000}
    assert efficiency == 4.0
    assert enchantability == 10
    assert not any("durability/use duration" in warning for warning in warnings)


def test_javap_named_damage_method_is_read_without_other_constants() -> None:
    javap = """
  public int getMaxItemUseDuration();
    Code:
       0: sipush        7200
       3: ireturn
  public int getDamageVsEntity();
    Code:
       0: bipush        15
       2: ireturn
    """
    _durability, damage, _efficiency, _enchantability, _warnings = _recover_tool_stats(javap, "Sword")
    assert damage == 15



def test_compiled_registry_recovers_exact_equipment_stats_and_registry_keys() -> None:
    registry = ROOT / "tests" / "fixtures" / "orespawn_content" / "OreSpawnMain.class"
    from legacy_bridge import _run_javap
    tools, armor, names = _bytecode_equipment_stats({registry: _run_javap(registry)})
    assert "OreSpawn_AmethystAxe" in names["AmethystAxe"]
    assert tools["orespawnamethystaxe"] == {
        "harvest": 4.0,
        "uses": 2000.0,
        "efficiency": 11.0,
        "damage": 11.0,
        "enchantability": 70.0,
    }
    assert armor["amethyst"] == {
        "durability": 100.0,
        "helmet": 4.0,
        "body": 8.0,
        "leggings": 7.0,
        "boots": 3.0,
        "enchantability": 40.0,
    }

def test_source_material_and_armor_stat_tables_are_recovered() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "Registry.java").write_text(
            """
            class Registry {
              static WeaponStats bertha_stats;
              static Item.ToolMaterial toolBERTHA;
              static Item Bertha;
              static ArmorStats Ruby_armorstats;
              static {
                bertha_stats = this.get_weaponstats(config, weapons, "Bertha", 3, 9000, 15, 496, 100);
                toolBERTHA = EnumHelper.addToolMaterial("BERTHA", bertha_stats.harvestlevel, bertha_stats.maxuses, (float)bertha_stats.efficiency, (float)bertha_stats.damage, bertha_stats.enchantability);
                Bertha = new BigBertha(1, toolBERTHA);
                GameRegistry.registerItem(Bertha, "big_bertha");
                Ruby_armorstats = this.get_armorstats(config, "Ruby", 90, 4, 9, 8, 4, 40, 0, 0, 0, 0, 0, 0, 0, 0);
              }
            }
            """,
            encoding="utf-8",
        )
        material = _source_tool_materials([root])["bigbertha"]
        assert material["uses"] == 9000
        assert material["damage"] == 496
        stats = _source_armor_stats([root])["ruby"]
        assert stats == {
            "durability": 90.0,
            "helmet": 4.0,
            "body": 9.0,
            "leggings": 8.0,
            "boots": 4.0,
            "enchantability": 40.0,
        }


def test_language_names_and_source_tooltips_are_preserved() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        lang = root / "assets" / "demo" / "lang"
        lang.mkdir(parents=True)
        (lang / "en_us.lang").write_text(
            "item.blades_of_duality.name=Blades of Duality\n"
            "entity.shadow_beast.name=Shadow Beast\n",
            encoding="utf-8",
        )
        names = _load_language_names([root])
        assert _translated_name(names, "blades_of_duality", "fallback") == "Blades of Duality"
        assert _translated_name(names, "shadow_beast", "fallback") == "Shadow Beast"
        tooltips = _extract_tooltip_strings(
            'void addInformation(List<String> tooltip){ tooltip.add(TextFmt.Gold + "Hold the center handle."); tooltip.add("Second line"); }'
        )
        assert tooltips == ["Hold the center handle.", "Second line"]


def test_texture_matching_does_not_collapse_common_mod_prefixes() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        tex = root / "assets" / "demo" / "textures" / "items"
        tex.mkdir(parents=True)
        for name in ("orespawn.png", "orespawn_slice.png", "orespawn_bertha.png"):
            (tex / name).write_bytes(name.encode())
        slice_candidates = _texture_candidates([root], ["orespawn_slice"], ("item", "items"))
        bertha_candidates = _texture_candidates([root], ["orespawn_bertha"], ("item", "items"))
        assert slice_candidates[0].stem == "orespawn_slice"
        assert bertha_candidates[0].stem == "orespawn_bertha"


def test_armor_texture_matching_keeps_items_and_layers_separate() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        items = root / "assets" / "demo" / "textures" / "items"
        layers = root / "assets" / "demo" / "textures" / "models" / "armor"
        items.mkdir(parents=True); layers.mkdir(parents=True)
        for path in (
            items / "ruby_helmet.png",
            items / "ruby_boots.png",
            items / "ruby.png",
            layers / "ruby_1.png",
            layers / "ruby_2.png",
        ):
            path.write_bytes(path.name.encode())
        helmet = _exact_texture_candidates(
            [root], _armor_piece_texture_stems("Ruby", "RubyHelmet", "helmet"), ("item", "items")
        )
        boots = _exact_texture_candidates(
            [root], _armor_piece_texture_stems("Ruby", "RubyBoots", "boots"), ("item", "items")
        )
        layer_one = _exact_texture_candidates(
            [root], _armor_layer_texture_stems("Ruby", 1), ("models/armor", "armor")
        )
        assert helmet[0].stem == "ruby_helmet"
        assert boots[0].stem == "ruby_boots"
        assert layer_one[0].stem == "ruby_1"


def test_legacy_armor_layers_and_all_piece_icons_are_copied() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        source = root / "source"
        items = source / "assets" / "demo" / "textures" / "items"
        layers = source / "assets" / "demo" / "textures" / "models" / "armor"
        items.mkdir(parents=True); layers.mkdir(parents=True)
        for path in (
            items / "ruby_helmet.png", items / "ruby_body.png",
            items / "ruby_legs.png", items / "ruby_boots.png",
            layers / "ruby_1.png", layers / "ruby_2.png",
        ):
            path.write_bytes(path.name.encode())
        registry = source / "Registry.java"
        registry.write_text(
            "class Registry { DemoArmor RubyHelmet; DemoArmor RubyBody; "
            "DemoArmor RubyLegs; DemoArmor RubyBoots; }",
            encoding="utf-8",
        )
        schema = json.loads((ROOT / "mcreator_templates" / "ArmorSchema.mod.json").read_text())
        workspace = {"mod_elements": [], "language_map": {"en_us": {}}, "workspaceSettings": {"modid": "demo"}}
        elements = root / "workspace" / "elements"
        elements.mkdir(parents=True)
        texture_root = root / "workspace" / "src" / "main" / "resources" / "assets" / "demo" / "textures"
        exports = _armor_elements_from_registry(
            registry, schema["_fv"], schema["definition"], set(), set(),
            texture_root, [source], elements, workspace, "demo", None,
            {"ruby": {"durability": 80, "helmet": 3, "body": 8, "leggings": 6, "boots": 3, "enchantability": 20}},
        )
        definition = json.loads((elements / "RubyArmor.mod.json").read_text())["definition"]
        assert [definition[key] for key in ("textureHelmet", "textureBody", "textureLeggings", "textureBoots")] == [
            "ruby_helmet", "ruby_body", "ruby_legs", "ruby_boots"
        ]
        assert definition["armorTextureFile"] == "ruby"
        assert (texture_root / "models" / "armor" / "ruby_layer_1.png").exists()
        assert (texture_root / "models" / "armor" / "ruby_layer_2.png").exists()
        assert not exports[0].warnings




def test_regular_item_can_use_custom_resource_model_and_texture_mapping() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        source = root / "assets" / "demo" / "models" / "item" / "infinity_trident.json"
        texture = root / "assets" / "demo" / "textures" / "items" / "trident.png"
        source.parent.mkdir(parents=True)
        texture.parent.mkdir(parents=True)
        texture.write_bytes(b"png")
        payload = {
            "display": {"firstperson_righthand": {"rotation": [10, 20, 30]}},
            "textures": {"0": "demo:items/trident"},
            "elements": [{
                "from": [0, 0, 0], "to": [1, 1, 8],
                "faces": {"north": {"uv": [0, 0, 1, 1], "texture": "#0"}},
            }],
        }
        source.write_text(json.dumps(payload), encoding="utf-8")
        slug, outputs = _copy_resource_item_model_as_custom(source, root / "workspace", "demo", "infinity_trident")
        assert slug == "infinity_trident_held"
        sidecar = json.loads((root / "workspace" / "models" / f"{slug}.json.textures").read_text())
        assert sidecar["mappings"]["default"]["map"][""] == "trident"
        copied = root / "workspace" / "src" / "main" / "resources" / "assets" / "demo" / "textures" / "trident.png"
        assert copied.exists()
        assert json.loads(Path(outputs[0]).read_text())["display"] == payload["display"]

def test_resource_item_model_preserves_display_transforms_as_custom_model() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        source = root / "assets" / "demo" / "models" / "item" / "dual_blade.json"
        source.parent.mkdir(parents=True)
        payload = {
            "parent": "item/generated",
            "textures": {"layer0": "demo:items/dual_blade"},
            "display": {
                "firstperson_righthand": {
                    "rotation": [230, 90, 0],
                    "translation": [5, 4, 2],
                    "scale": [2, 2, 2],
                }
            },
        }
        source.write_text(json.dumps(payload), encoding="utf-8")
        assert _find_item_model_json([root], "dual_blade") == source
        slug, outputs = _copy_resource_item_model_as_custom(source, root / "workspace", "converted", "dual_blade")
        assert slug == "dual_blade_held"
        copied = json.loads(Path(outputs[0]).read_text())
        assert copied["display"] == payload["display"]
        assert copied["textures"][""] == "converted:items/dual_blade"


def test_legacy_item_renderer_transforms_are_preserved_for_tool_model() -> None:
    source = """
    class RenderDemo implements IItemRenderer {
      public void renderItem(ItemRenderType type, ItemStack item, Object... data) {
        switch(type) {
          case EQUIPPED: this.third(-4F, 2F, -3F, .25F); break;
          case EQUIPPED_FIRST_PERSON: this.first(6F, 3F, -5F, .5F); break;
        }
      }
      private void third(float x,float y,float z,float scale) {
        GL11.glRotatef(90F,1F,0F,0F); GL11.glRotatef(-90F,0F,0F,1F);
        GL11.glScalef(scale,scale,scale); GL11.glTranslatef(x,y,z);
      }
      private void first(float x,float y,float z,float scale) {
        GL11.glRotatef(190F,1F,0F,0F); GL11.glRotatef(25F,0F,0F,1F);
        GL11.glScalef(scale,scale,scale); GL11.glTranslatef(x,y,z);
      }
    }
    """
    display = _legacy_renderer_display(source)
    assert display["thirdperson_righthand"] == {
        "rotation": [90.0, 0.0, -90.0],
        "translation": [-4.0, 2.0, -3.0],
        "scale": [0.25, 0.25, 0.25],
    }
    assert display["firstperson_righthand"]["translation"] == [6.0, 3.0, -5.0]
    assert display["firstperson_righthand"]["scale"] == [0.5, 0.5, 0.5]


def test_language_description_can_match_item_suffix() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        lang = root / "assets" / "demo" / "lang"
        lang.mkdir(parents=True)
        (lang / "en_us.lang").write_text(
            "description.demo.jei.ingots.kyvorium=Forged in the refinery.\n",
            encoding="utf-8",
        )
        _load_language_names([root])
        assert _language_descriptions([root], "kyvorium_ingot") == ["Forged in the refinery."]


def test_decompiled_java_model_is_accepted_by_mcreator_model_path() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        model_source = root / "ModelDemo.java"
        model_source.write_text(
            """
            class ModelDemo extends ModelBase {
              ModelRenderer body;
              ModelDemo(){
                this.textureWidth = 64; this.textureHeight = 32;
                this.body = new ModelRenderer(this, 0, 0);
                this.body.addBox(-2F, -2F, -2F, 4, 4, 4);
                this.body.setRotationPoint(0F, 20F, 0F);
              }
            }
            """,
            encoding="utf-8",
        )
        graph = LegacyModGraph(root=root, source_roots=[root], java_files={"ModelDemo": model_source})
        model, kind, _ = _parse_mcreator_model(model_source, graph, [], Cube, Part, ParsedModel)
        assert kind == "legacy_java_source"
        assert model.cube_count == 1


def test_looping_baked_animations_close_exactly() -> None:
    model = parse_legacy_class(
        ROOT / "tests" / "fixtures" / "ModelGodzilla.class",
        Cube,
        Part,
        ParsedModel,
        constructor_args=[0.2],
    )
    for animation in model.baked_animation_json["animations"].values():
        if not animation.get("loop", False):
            continue
        for channels in animation["bones"].values():
            for keyframes in channels.values():
                ordered = sorted(keyframes.items(), key=lambda item: float(item[0]))
                assert ordered[1][1] == ordered[-1][1]


def test_scan_graph_can_merge_separate_compiled_and_decompiled_roots() -> None:
    javac = shutil.which("javac")
    if not javac:
        return
    with tempfile.TemporaryDirectory() as td:
        base = Path(td)
        compiled = base / "Compiled"
        decompiled = base / "Decompiled"
        compiled.mkdir(); decompiled.mkdir()
        (compiled / "Demo.java").write_text("public class Demo {}", encoding="utf-8")
        subprocess.run([javac, "Demo.java"], cwd=compiled, check=True, capture_output=True)
        (compiled / "Demo.java").unlink()
        (decompiled / "ModelDemo.java").write_text("class ModelDemo {}", encoding="utf-8")
        graph = scan_legacy_mod(decompiled, source_roots=[compiled, decompiled])
        assert "Demo" in graph.class_files
        assert "ModelDemo" in graph.java_files
        assert {compiled.resolve(), decompiled.resolve()}.issubset(set(graph.roots))


def test_decompiled_legacy_animation_recovers_walk_and_preserves_endpoint() -> None:
    source = r"""
    class ModelWalk extends ModelBase {
      ModelRenderer leg;
      private float speed = 0.8F;
      ModelWalk(){
        this.textureWidth = 64; this.textureHeight = 32;
        this.leg = new ModelRenderer(this, 0, 0);
        this.leg.func_78789_a(-1F, 0F, -1F, 2, 6, 2);
      }
      public void func_78087_a(float f, float f1, float f2, float f3, float f4, float f5, Entity e) {
        float wave = MathHelper.func_76134_b(f * this.speed) * f1;
        this.leg.field_78795_f = wave;
      }
    }
    """
    model = parse_legacy_java_model(source, "ModelWalk.java")
    animations = model.baked_animation_json["animations"]
    assert "animation.model.walk" in animations
    rotation = animations["animation.model.walk"]["bones"]["leg"]["rotation"]
    ordered = sorted(rotation.items(), key=lambda item: float(item[0]))
    animation = animations["animation.model.walk"]
    if animation.get("loop", False):
        assert ordered[1][1] == ordered[-1][1]
    assert any(value != ordered[0][1] for _, value in ordered[1:-1])


def test_decompiled_animation_keeps_every_bone_animator_after_uuid_assignment() -> None:
    source = r"""
    class ModelTwoLegs extends ModelBase {
      ModelRenderer left; ModelRenderer right;
      ModelTwoLegs(){
        this.left = new ModelRenderer(this,0,0); this.left.func_78789_a(-1F,0F,-1F,2,6,2);
        this.right = new ModelRenderer(this,0,0); this.right.func_78789_a(-1F,0F,-1F,2,6,2);
      }
      public void func_78087_a(float f,float f1,float f2,float f3,float f4,float f5,Entity e){
        this.left.field_78795_f = MathHelper.func_76134_b(f) * f1;
        this.right.field_78795_f = MathHelper.func_76134_b(f + 3.1415927F) * f1;
      }
    }
    """
    model = parse_legacy_java_model(source, "ModelTwoLegs.java")
    payload = model_to_bbmodel(model)
    walk = next(animation for animation in payload["animations"] if animation["name"] == "animation.model.walk")
    assert {animator["name"] for animator in walk["animators"].values()} == {"left", "right"}


def test_decompiled_constructor_alias_position_adjustments_are_preserved() -> None:
    source = r"""
    class ModelShifted extends ModelBase {
      ModelRenderer body;
      ModelShifted(float speed){
        this.body = new ModelRenderer(this, 0, 0);
        this.body.func_78789_a(-1F, -1F, -1F, 2, 2, 2);
        this.body.func_78793_a(2F, 10F, 3F);
        ModelRenderer var10000 = this.body;
        var10000.field_78798_e += 8F;
        var10000 = this.body;
        var10000.field_78797_d -= 4F;
      }
    }
    """
    model = parse_legacy_java_model(source, "ModelShifted.java", [0.75])
    assert model.parts[0].offset == (10.0, 6.0, 3.0)
    assert model.constructor_args == [0.75]


def test_compiled_constructor_stack_and_position_field_math_are_preserved() -> None:
    from legacy_bridge import _Method, _parse_constructor

    method = _Method(
        header="public demo.ModelShifted(float);",
        instructions=[
            (0, "aload_0", ""),
            (1, "invokespecial #1", 'Method net/minecraft/client/model/ModelBase."<init>":()V'),
            (4, "aload_0", ""),
            (5, "new #2", "class net/minecraft/client/model/ModelRenderer"),
            (8, "dup", ""),
            (9, "aload_0", ""),
            (10, "iconst_0", ""),
            (11, "iconst_0", ""),
            (12, "invokespecial #3", 'Method net/minecraft/client/model/ModelRenderer."<init>":(Lnet/minecraft/client/model/ModelBase;II)V'),
            (15, "putfield #4", "Field body:Lnet/minecraft/client/model/ModelRenderer;"),
            (18, "aload_0", ""),
            (19, "getfield #4", "Field body:Lnet/minecraft/client/model/ModelRenderer;"),
            (22, "fconst_1", ""),
            (23, "bipush 10", ""),
            (25, "iconst_3", ""),
            (26, "invokevirtual #5", "Method net/minecraft/client/model/ModelRenderer.func_78793_a:(FFF)V"),
            (29, "aload_0", ""),
            (30, "getfield #4", "Field body:Lnet/minecraft/client/model/ModelRenderer;"),
            (33, "dup", ""),
            (34, "getfield #6", "Field net/minecraft/client/model/ModelRenderer.field_78797_d:F"),
            (37, "bipush 6", ""),
            (39, "fadd", ""),
            (40, "putfield #6", "Field net/minecraft/client/model/ModelRenderer.field_78797_d:F"),
        ],
    )
    parts, _texture, _scalars, _warnings = _parse_constructor(method, constructor_args=[0.75])
    assert parts["body"].offset == [1.0, 16.0, 3.0]


def test_legacy_item_model_applies_modelrenderer_rotation_point_to_cubes() -> None:
    model = ParsedModel(
        "ModelBlade",
        64,
        texture_height=32,
        parts=[
            Part(
                name="blade",
                variable="blade",
                parent_variable="",
                offset=(2.0, 3.0, -1.0),
                rotation=(0.0, 0.25, 0.0),
                cubes=[Cube(x=0.0, y=-6.0, z=0.0, dx=1.0, dy=12.0, dz=1.0)],
            )
        ],
    )
    from mcreator_content_export import _parsed_model_to_tool_json
    payload = _parsed_model_to_tool_json(model, "blade_texture")
    element = payload["elements"][0]
    assert element["from"] == [9.0, -19.0, 7.0]
    assert element["to"] == [10.0, -7.0, 8.0]
    assert element["rotation"]["origin"] == [9.5, 5.0, 7.5]


def test_custom_item_model_parent_is_detected_without_forcing_generated_items_custom() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        models = root / "assets" / "demo" / "models" / "item"
        models.mkdir(parents=True)
        (models / "normal.json").write_text(
            json.dumps({"parent": "item/generated", "textures": {"layer0": "demo:items/normal"}}),
            encoding="utf-8",
        )
        (models / "dual_blade.json").write_text(
            json.dumps({"parent": "demo:item/custom_blade", "display": {"firstperson_righthand": {"scale": [2, 2, 2]}}}),
            encoding="utf-8",
        )
        assert _find_item_model_json([root], "normal") is None
        assert _find_item_model_json([root], "dual_blade") == models / "dual_blade.json"


def test_segmented_legacy_animation_propagates_walk_class_to_dependent_positions() -> None:
    source = r"""
    class ModelChain extends ModelBase {
      ModelRenderer body1; ModelRenderer body2;
      ModelChain(){
        this.body1 = new ModelRenderer(this,0,0);
        this.body2 = new ModelRenderer(this,0,0);
        this.body1.func_78789_a(-1F,-1F,-1F,2,2,2);
        this.body2.func_78789_a(-1F,-1F,-1F,2,2,2);
      }
      public void func_78087_a(float limb,float amount,float age,float yaw,float pitch,float scale,Entity e){
        this.body1.field_78796_g = MathHelper.func_76134_b(age * 1.3F) * 0.1F * amount;
        this.body2.field_78798_e = this.body1.field_78798_e + MathHelper.func_76134_b(this.body1.field_78796_g) * 12F;
        this.body2.field_78800_c = this.body1.field_78800_c + MathHelper.func_76126_a(this.body1.field_78796_g) * 12F;
      }
    }
    """
    model = parse_legacy_java_model(source, "ModelChain.java")
    walk = model.baked_animation_json["animations"]["animation.model.walk"]
    assert "body2" in walk["bones"]
    assert "position" in walk["bones"]["body2"]
    positions = walk["bones"]["body2"]["position"]
    ordered = [value for _, value in sorted(positions.items(), key=lambda item: float(item[0]))]
    assert any(value != ordered[0] for value in ordered[1:-1])


def test_sounds_json_recovers_events_categories_files_and_localized_subtitles() -> None:
    from mcreator_export import _recover_sound_elements
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        sound_dir = root / "assets" / "demo" / "sounds" / "entity"
        lang_dir = root / "assets" / "demo" / "lang"
        sound_dir.mkdir(parents=True)
        lang_dir.mkdir(parents=True)
        (sound_dir / "roar.ogg").write_bytes(b"OggS")
        (lang_dir / "en_us.lang").write_text(
            "subtitle.demo.entity.roar=Beast Roar\n",
            encoding="utf-8",
        )
        (root / "assets" / "demo" / "sounds.json").write_text(
            json.dumps({
                "demo.entity.roar": {
                    "category": "hostile",
                    "subtitle": "subtitle.demo.entity.roar",
                    "sounds": ["demo:entity/roar"],
                }
            }),
            encoding="utf-8",
        )
        workspace = {"sound_elements": []}
        from mcreator_content_export import _load_language_names
        count, warnings = _recover_sound_elements([root], workspace, _load_language_names([root]))
        assert count == 1
        assert not warnings
        assert workspace["sound_elements"][0] == {
            "name": "demo.entity.roar",
            "files": ["entity/roar"],
            "category": "hostile",
            "subtitle": "Beast Roar",
        }


def test_compiled_only_legacy_item_renderers_are_recovered() -> None:
    fixture = ROOT / "tests" / "fixtures" / "compiled_item_renderer"
    specs = _held_renderer_specs([fixture])
    assert specs["orespawnbertha"]["renderer_class"] == "RenderBertha"
    assert specs["orespawnbertha"]["model_class"] == "ModelBertha"
    assert specs["orespawnbertha"]["texture_ref"].lower() == "berthatexture"
    assert specs["orespawnslice"]["renderer_class"] == "RenderSlice"
    assert specs["orespawnslice"]["model_class"] == "ModelSlice"
    assert specs["orespawnroyal"]["renderer_class"] == "RenderRoyal"

def test_tooltip_extraction_preserves_formatting_codes() -> None:
    from mcreator_content_export import _extract_tooltip_strings

    source = r'''
    class DemoItem {
      void addTooltip(java.util.List list) {
        list.add(TextFormatting.GOLD + "Shiny" + TextFormatting.RESET + " Sword");
        list.add(EnumChatFormatting.RED + "Danger");
      }
    }
    '''
    values = _extract_tooltip_strings(source)
    assert "§6Shiny§r Sword" in values
    assert "§cDanger" in values


def test_recover_recipe_json_resources_into_workspace_data_tree() -> None:
    from mcreator_content_export import _copy_recipe_resources

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        recipe = root / "assets" / "demo" / "recipes" / "tools" / "sword.json"
        recipe.parent.mkdir(parents=True)
        recipe.write_text(json.dumps({
            "type": "minecraft:crafting_shaped",
            "pattern": [" X ", " X ", " / "],
            "key": {"X": {"item": "minecraft:iron_ingot"}, "/": {"item": "minecraft:stick"}},
            "result": {"item": "demo:sword"}
        }), encoding="utf-8")
        workspace = Path(td) / "workspace"
        outputs = _copy_recipe_resources([root], workspace, "demo")
        assert outputs
        copied = workspace / "src" / "main" / "resources" / "data" / "demo" / "recipes" / "tools" / "sword.json"
        assert copied.exists()
        payload = json.loads(copied.read_text(encoding="utf-8"))
        assert payload["result"]["item"] == "demo:sword"

