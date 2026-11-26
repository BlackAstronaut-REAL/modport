from __future__ import annotations

import json
import shutil
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
import sys
sys.path.insert(0, str(ROOT))

from legacy_mod_graph import LegacyModGraph, scan_legacy_mod
from mcreator_content_export import _source_registered_names, _write_placeholder_png, export_mcreator_content
from mcreator_export import _default_living_definition, _ensure_import_folder_tree


def test_modern_deferred_register_recovers_real_names() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        source = root / "Main.java"
        source.write_text(
            '''
class Main {
  Object ITEMS = null;
  Object BLOCKS = null;
  Object ENTITY_TYPES = null;
  void init() {
    ITEMS.register("alpha_item", () -> new AlphaItem());
    BLOCKS.register("alpha_block", AlphaBlock::new);
    ENTITY_TYPES.register("alpha_mob", () -> EntityType.Builder.<AlphaMob>of(AlphaMob::new, MobCategory.CREATURE).build("alpha_mob"));
  }
}
class AlphaItem extends Item {}
class AlphaBlock extends Block {}
class AlphaMob extends Mob {}
''',
            encoding="utf-8",
        )
        result = _source_registered_names(root)
        assert result["AlphaItem"] == {"alpha_item"}
        assert result["AlphaBlock"] == {"alpha_block"}
        assert result["AlphaMob"] == {"alpha_mob"}


def test_entity_renderer_registryobject_source_pattern_recovers_inline_model() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        main = root / "Main.java"
        main.write_text(
            '''
class Main {
  static final RegistryObject<EntityType<AlphaMob>> ENTITY_TYPE_ALPHA = ENTITY_TYPES.register("alpha_mob", () -> EntityType.Builder.<AlphaMob>of(AlphaMob::new, MobCategory.CREATURE).build("alpha_mob"));
  void renderers(EntityRenderersEvent.RegisterRenderers event) {
    event.registerEntityRenderer(ENTITY_TYPE_ALPHA.get(), ctx -> new RenderAlpha(ctx, new ModelAlpha(0.5f), 0.4f, 1.0f));
  }
}
class AlphaMob extends Mob {}
class RenderAlpha {}
class ModelAlpha {}
''',
            encoding="utf-8",
        )
        graph = scan_legacy_mod(root)
        matches = [r for r in graph.registrations if r.entity_class == "AlphaMob"]
        assert matches
        assert matches[0].model_class == "ModelAlpha"
        assert matches[0].model_args == [0.5]
        assert graph.registry_names.get("AlphaMob") == "alpha_mob"


def test_import_folders_are_namespaced_by_source_mod() -> None:
    workspace = {"foldersRoot": {"name": "~", "children": []}}
    _ensure_import_folder_tree(workspace, "chaospersists", {"Mobs", "Blocks", "Items", "Foods", "Tools", "Weapons"})
    folder = workspace["foldersRoot"]["children"][0]
    assert folder["name"] == "chaospersists"
    assert {child["name"] for child in folder["children"]} == {"Mobs", "Blocks", "Items", "Foods", "Tools", "Weapons"}


def test_import_content_does_not_invent_unregistered_items_and_namespaces_template_collisions() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "elements").mkdir()
        source = root / "source"
        source.mkdir()
        (source / "Main.java").write_text(
            'class Main { static Object ITEMS; static { ITEMS.register("real_item", () -> new RealItem()); } }',
            encoding="utf-8",
        )
        (source / "RealItem.java").write_text('class RealItem extends Item {}', encoding="utf-8")
        (source / "HelperItem.java").write_text('class HelperItem extends Item {}', encoding="utf-8")
        schema = json.loads((ROOT / "mcreator_templates" / "ItemSchema.mod.json").read_text(encoding="utf-8"))
        (root / "elements" / "ItemSchema.mod.json").write_text(json.dumps(schema, indent=2), encoding="utf-8")
        workspace = {
            "mod_elements": [{
                "name": "TemplateRealItem", "type": "item", "registry_name": "real_item",
                "compiles": True, "locked_code": False, "metadata": {"files": []}, "path": "~/Template"
            }],
            "language_map": {"en_us": {}},
            "workspaceSettings": {"modid": "template"},
            "foldersRoot": {"name": "~", "children": []},
        }
        graph = LegacyModGraph(root=source, source_roots=[source], java_files={p.stem: p for p in source.rglob("*.java")})
        # Add the source texture separately so template fallback cannot satisfy the import.
        source_tex = source / "real_item.png"
        _write_placeholder_png(source_tex)
        result = export_mcreator_content(
            graph, workspace, root, root / "elements", root / "assets/textures",
            texture_source_roots=[source], modid="template", import_modid="demo", log=None
        )
        created = [e for e in result.exports if e.status == "created"]
        assert any(e.element_type == "item" and e.registry_name == "demo_real_item" for e in created)
        assert not any(e.registry_name == "helper_item" for e in created)
        template = next(x for x in workspace["mod_elements"] if x["registry_name"] == "real_item")
        assert template["name"] == "TemplateRealItem"
        imported = next(x for x in workspace["mod_elements"] if x["registry_name"] == "demo_real_item")
        assert imported["path"] == "~/demo/Items"


def test_new_import_definition_clears_template_visual_bindings_but_existing_is_untouched() -> None:
    from mcreator_content_export import _sanitize_new_import_definition
    template = {
        "texture": "TemplateTexture",
        "customModelName": "TemplateModel:default",
        "itemTexture": "TemplateItemTexture",
        "particleTexture": "TemplateParticle",
        "hardness": 3.0,
    }
    cleaned = _sanitize_new_import_definition(template, "block")
    assert cleaned["texture"] == ""
    assert cleaned["customModelName"] == "Normal"
    assert cleaned["itemTexture"] == ""
    assert cleaned["particleTexture"] == ""
    assert cleaned["hardness"] == 3.0
    assert template["texture"] == "TemplateTexture"


def test_worldgen_data_namespace_is_remapped_for_template_imports() -> None:
    from mcreator_source_bridge import _rewrite_data_namespace
    source = {
        "targets": [
            "chaospersists:deepslate_oresalt",
            "minecraft:stone",
        ],
        "state": {"Name": "chaospersists:oresalt"},
    }
    rewritten = _rewrite_data_namespace(source, "chaospersists", "targetmod")
    assert rewritten["targets"][0] == "targetmod:deepslate_oresalt"
    assert rewritten["targets"][1] == "minecraft:stone"
    assert rewritten["state"]["Name"] == "targetmod:oresalt"


def test_new_native_items_keep_valid_normal_model_sentinel() -> None:
    from mcreator_content_export import _item_element
    template = json.loads((ROOT / "mcreator_templates" / "ItemSchema.mod.json").read_text(encoding="utf-8"))
    element = _item_element(Path("DemoItem.java"), "DemoItem", "Item", "class DemoItem extends Item {}", "demo_item", 89, template["definition"], "demo_item")
    assert element["definition"]["renderType"] == 0
    assert element["definition"]["customModelName"] == "Normal"


def test_mcreator_recipe_element_is_emitted_from_crafting_json() -> None:
    from mcreator_content_export import _emit_mcreator_recipe_elements
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        recipe = root / "data" / "demo" / "recipes" / "sword.json"
        recipe.parent.mkdir(parents=True)
        recipe.write_text(json.dumps({
            "type": "minecraft:crafting_shaped",
            "pattern": [" I ", " I ", " S "],
            "key": {"I": {"item": "demo:iron"}, "S": {"item": "minecraft:stick"}},
            "result": {"item": "demo:sword", "count": 1},
        }), encoding="utf-8")
        workspace = {"mod_elements": [], "foldersRoot": {"name": "~", "children": []}}
        exports, warnings = _emit_mcreator_recipe_elements([root], root / "elements", workspace, "demo", {"iron": "Iron", "sword": "Sword"})
        assert not warnings
        assert exports and exports[0].element_type == "recipe"
        path = root / "elements" / "SwordRecipe.mod.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        assert data["_type"] == "recipe"
        assert data["definition"]["recipeReturnStack"] == "CUSTOM:Sword"
        assert data["definition"]["recipeSlots"][1] == "CUSTOM:Iron"
        assert data["definition"]["recipeSlots"][7] == "Items.STICK"


def test_source_registry_alias_is_preserved_on_import_descriptor() -> None:
    from mcreator_content_export import _upsert_workspace_element
    workspace = {"mod_elements": []}
    entry = _upsert_workspace_element(workspace, None, "ImportedOre", "block", "demo_deepslate_ore", "demo", "deepslate_ore", "Blocks")
    assert entry["mojmodelbridge_source_registry"] == "deepslate_ore"
    assert entry["registry_name"] == "demo_deepslate_ore"

def test_dimension_worldgen_uniform_is_not_copied_as_legacy_source_data(tmp_path):
    from mcreator_source_bridge import _copy_source_resources
    src = tmp_path / 'src'
    out = tmp_path / 'out'
    data = src / 'data' / 'demo' / 'dimension_type'
    data.mkdir(parents=True)
    (data / 'test.json').write_text(json.dumps({
        'monster_spawn_light_level': {'type': 'minecraft:uniform', 'value': {'min_inclusive': 0, 'max_inclusive': 7}}
    }), encoding='utf-8')
    _copy_source_resources(src, out, source_modid='demo', target_modid='demo2')
    assert not (out / 'data' / 'demo2' / 'dimension_type' / 'test.json').exists()


def test_resource_only_dimension_and_biome_are_importable(tmp_path):
    from mcreator_content_export import _resource_biome_definitions, _resource_dimension_definitions, _resource_biome_updates, _resource_dimension_updates
    src = tmp_path / 'src'
    biome_dir = src / 'data' / 'demo' / 'worldgen' / 'biome'
    dim_dir = src / 'data' / 'demo' / 'dimension'
    type_dir = src / 'data' / 'demo' / 'dimension_type'
    for d in (biome_dir, dim_dir, type_dir):
        d.mkdir(parents=True)
    (biome_dir / 'test_biome.json').write_text(json.dumps({'temperature': 0.8, 'downfall': 0.2, 'effects': {}, 'spawners': {}}), encoding='utf-8')
    (type_dir / 'test.json').write_text(json.dumps({'bed_works': True, 'has_skylight': True, 'ambient_light': 0.5}), encoding='utf-8')
    (dim_dir / 'test_dim.json').write_text(json.dumps({'generator': {'biome_source': {'type': 'minecraft:multi_noise', 'biomes': [{'biome': 'demo:test_biome'}]}, 'settings': {'sea_level': 63}}}), encoding='utf-8')
    bios = _resource_biome_definitions(src, 'demo')
    dims, types = _resource_dimension_definitions(src, 'demo')
    assert 'test_biome' in bios and 'test_dim' in dims and 'test' in types
    assert _resource_biome_updates(bios['test_biome'])['temperature'] == 0.8
    updates = _resource_dimension_updates(dims['test_dim'], types['test'], 'test_biome', 'demo')
    assert updates['biomesInDimension'] == ['CUSTOM:TestBiome']


def test_block_model_parent_maps_to_native_render_type(tmp_path):
    from mcreator_content_export import _mcreator_block_model_info
    src = tmp_path / 'src'
    model = src / 'assets' / 'demo' / 'models' / 'block'
    model.mkdir(parents=True)
    (model / 'grass.json').write_text(json.dumps({'parent': 'minecraft:block/grass_block', 'textures': {'side': 'demo:block/grass', 'top': 'demo:block/grass_top', 'bottom': 'demo:block/dirt', 'overlay': 'minecraft:block/grass_side_overlay'}}), encoding='utf-8')
    info = _mcreator_block_model_info([src], 'demo', 'grass')
    assert info['renderType'] == 14
    assert info['textureTop'] == 'demo:block/grass_top'


def test_block_model_texture_reference_is_materialized_to_workspace(tmp_path):
    from mcreator_content_export import _copy_model_ref_texture_to_workspace
    from PIL import Image
    src = tmp_path / 'src'
    model = src / 'assets' / 'demo' / 'models' / 'block'
    tex = src / 'assets' / 'demo' / 'textures' / 'block'
    model.mkdir(parents=True); tex.mkdir(parents=True)
    (model / 'foo.json').write_text('{}', encoding='utf-8')
    Image.new('RGBA', (2,2), (12,34,56,255)).save(tex / 'foo.png')
    out = tmp_path / 'out'
    stem = _copy_model_ref_texture_to_workspace(model / 'foo.json', 'demo:block/foo', out, 'block', 'foo')
    assert stem == 'foo'
    assert (out / 'block' / 'foo.png').exists()


def test_armor_fuzzy_piece_texture_matches_shared_legacy_prefix(tmp_path):
    from mcreator_content_export import _armor_related_texture_candidates, _armor_piece_texture_stems
    root = tmp_path / 'src'
    tex = root / 'assets' / 'demo' / 'textures' / 'items'
    tex.mkdir(parents=True)
    (tex / 'peacock_boots.png').write_bytes(b'boots')
    matches = _armor_related_texture_candidates([root], _armor_piece_texture_stems('PeacockFeather', 'PeacockFeatherBoots', 'boots'), 'boots')
    assert matches and matches[0].stem == 'peacock_boots'


def test_spawn_egg_colors_are_recovered_from_custom_texture(tmp_path):
    from mcreator_export import _extract_spawn_egg_colors
    from PIL import Image
    root = tmp_path / 'src'
    tex = root / 'assets' / 'demo' / 'textures' / 'entity'
    tex.mkdir(parents=True)
    img = Image.new('RGBA', (8,8), (220,20,20,255))
    for x in range(3):
        for y in range(8):
            img.putpixel((x,y), (20,30,210,255))
    img.save(tex / 'mobzilla_spawn_egg.png')
    colors = _extract_spawn_egg_colors([root], 'mobzilla', 'Godzilla')
    assert colors and colors[0] != colors[1]


def test_cube_all_block_model_uses_single_texture_render_type(tmp_path):
    from mcreator_content_export import _mcreator_block_model_info
    src = tmp_path / "src"
    model = src / "assets" / "demo" / "models" / "block"
    model.mkdir(parents=True)
    (model / "ore.json").write_text(json.dumps({
        "parent": "minecraft:block/cube_all",
        "textures": {"all": "demo:block/ore", "particle": "demo:block/ore"},
    }), encoding="utf-8")
    info = _mcreator_block_model_info([src], "demo", "ore")
    assert info["renderType"] == 11
    assert info["customModelName"] == "Single texture"


def test_plant_models_use_mcreator_cross_model_type(tmp_path):
    from mcreator_content_export import _mcreator_block_model_info
    src = tmp_path / "src"
    model = src / "assets" / "demo" / "models" / "block"
    model.mkdir(parents=True)
    (model / "flower.json").write_text(json.dumps({
        "parent": "minecraft:block/cross",
        "textures": {"cross": "demo:block/flower"},
    }), encoding="utf-8")
    info = _mcreator_block_model_info([src], "demo", "flower")
    assert info["renderType"] == 12
    assert info["customModelName"] == "Cross model"



def test_source_only_entity_metadata_and_modern_goals_are_recovered() -> None:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "Main.java").write_text(
            """
class Main {
  static Object ENTITY_TYPES;
  static final RegistryObject<EntityType<AlphaMob>> ENTITY_TYPE_ALPHA =
      ENTITY_TYPES.register("alpha_mob", () -> EntityType.Builder.<AlphaMob>of(AlphaMob::new, MobCategory.CREATURE).build("alpha_mob"));
  void renderers(EntityRenderersEvent.RegisterRenderers event) {
    event.registerEntityRenderer(ENTITY_TYPE_ALPHA.get(), ctx -> new RenderAlpha(ctx, new ModelAlpha(), 0.4f, 1.0f));
  }
}
class RenderAlpha {}
class ModelAlpha {}
""",
            encoding="utf-8",
        )
        (root / "AlphaMob.java").write_text(
            """
class AlphaMob extends PathfinderMob {
  public static AttributeSupplier.Builder createAttributes() {
    return Mob.createMobAttributes()
        .add(Attributes.MAX_HEALTH, 48.0D)
        .add(Attributes.MOVEMENT_SPEED, 0.28D)
        .add(Attributes.ATTACK_DAMAGE, 9.0D)
        .add(Attributes.FOLLOW_RANGE, 32.0D)
        .add(Attributes.ARMOR, 4.0D);
  }
  protected void registerGoals() {
    this.goalSelector.addGoal(0, new FloatGoal(this));
    this.goalSelector.addGoal(2, new MeleeAttackGoal(this, 1.2D, true));
    this.goalSelector.addGoal(7, new RandomLookAroundGoal(this));
    this.targetSelector.addGoal(1, new NearestAttackableTargetGoal<>(this, Player.class, true));
  }
  protected void setDimensions() { this.setSize(1.2F, 2.4F); }
  public boolean isFireImmune() { return true; }
  public int getExperienceReward() { return 30; }
}
""",
            encoding="utf-8",
        )
        graph = scan_legacy_mod(root)
        meta = graph.entities["AlphaMob"]
        assert meta.registry_name == "alpha_mob"
        assert (meta.width, meta.height) == (1.2, 2.4)
        assert meta.health == 48.0
        assert meta.movement_speed == 0.28
        assert meta.attack_damage == 9.0
        assert meta.follow_range == 32.0
        assert meta.armor == 4.0
        assert meta.xp == 30
        assert meta.fire_immune is True
        assert set(meta.ai_classes) == {
            "FloatGoal", "MeleeAttackGoal", "RandomLookAroundGoal", "NearestAttackableTargetGoal"
        }
        assert meta.ai_priorities["MeleeAttackGoal"] == 2


def test_modern_vanilla_goal_classes_produce_native_mcreator_ai_blocks() -> None:
    from mcreator_export import _ai_xml
    xml = _ai_xml([
        "FloatGoal", "MeleeAttackGoal", "WaterAvoidingRandomStrollGoal",
        "RandomLookAroundGoal", "NearestAttackableTargetGoal",
    ])
    for block in ("swim_in_water", "attack_on_collide", "wander", "look_around", "attack_action"):
        assert f'<block type="{block}">' in xml


def test_compiled_only_custom_armor_model_override_is_linked(tmp_path, monkeypatch) -> None:
    import mcreator_content_export

    root = tmp_path / "compiled"
    root.mkdir()
    compiled = root / "ItemCrystalArmor.class"
    compiled.write_bytes(b"compiled fixture")
    javap = '''public class ItemCrystalArmor extends ItemArmor {
  public ModelBiped getArmorModel(EntityLivingBase, ItemStack, int, EntityPlayer);
    Code:
       0: new           #7                  // class net/example/ModelCrystalArmor
       3: dup
       4: invokespecial #9                  // Method net/example/ModelCrystalArmor."<init>":()V
       7: areturn
}
'''
    monkeypatch.setattr(mcreator_content_export, "_run_javap", lambda _path: javap)
    links = mcreator_content_export._custom_armor_model_links([root])
    assert links["crystal"] == "ModelCrystalArmor"



def test_generic_food_constructor_values_are_bound_to_each_registry_id(tmp_path) -> None:
    from mcreator_content_export import _extract_food_info, _source_registered_constructor_records

    root = tmp_path / "food"
    root.mkdir()
    source = root / "Foods.java"
    source.write_text(
        """
class Foods {
  static Object ITEMS;
  static void register() {
    ITEMS.register("sweet_berry_food", () -> new GenericFood(6, 0.5F, true));
    ITEMS.register("cooked_fish_food", () -> new GenericFood(8, 0.8F, false));
  }
}
class GenericFood extends Item {
  public GenericFood(int nutrition, float saturation, boolean alwaysEdible) {
    super(new Item.Properties().food(new FoodProperties.Builder()
        .nutrition(nutrition).saturationMod(saturation)));
    if (alwaysEdible) { this.builder.alwaysEat(); }
  }
}
""",
        encoding="utf-8",
    )
    instances = _source_registered_constructor_records(root)["GenericFood"]
    source_text = source.read_text(encoding="utf-8")
    berry = _extract_food_info(source_text, "Item", [instances["sweet_berry_food"]])
    fish = _extract_food_info(source_text, "Item", [instances["cooked_fish_food"]])
    assert berry["nutritionalValue"] == 6
    assert berry["saturation"] == 0.5
    assert berry["isAlwaysEdible"] is True
    assert fish["nutritionalValue"] == 8
    assert fish["saturation"] == 0.8
    assert fish["isAlwaysEdible"] is False
