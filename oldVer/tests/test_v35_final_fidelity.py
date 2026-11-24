from pathlib import Path
import json
import sys
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

def test_rotator_runtime_rotation_java_is_valid_float_syntax():
    from mcreator_export import model_to_mojmap_java
    from model_bridge import parse_java_model
    text = (ROOT / 'tests' / 'fixtures' / 'ModelRotator.java').read_text(encoding='utf-8')
    model = parse_java_model(text, 'ModelRotator.java')
    java = model_to_mojmap_java(model, 'ModelRotator', 'cndcc')
    assert '2.0Ff' not in java
    assert '2.0f' in java or '2F' in java

def test_source_tool_materials_and_custom_use_stats():
    from mcreator_content_export import _source_tool_materials, _recover_tool_stats
    root = ROOT / 'tests' / 'fixtures' / 'clean_source_tool_stats'
    root.mkdir(parents=True, exist_ok=True)
    (root/'Tools.java').write_text('''\n        class Tools {\n          static WeaponStats emerald_stats = get_weaponstats(config, weapons, "Emerald", 3, 1300, 10, 6, 75);\n          static Tier toolEMERALD = EnumHelper.addToolMaterial("REALEMERALD", emerald_stats.harvestlevel, emerald_stats.maxuses, emerald_stats.efficiency, emerald_stats.damage, emerald_stats.enchantability);\n          static void r() { ITEMS.register("experiencesword", () -> new ExperienceSword(toolEMERALD)); ITEMS.register("fairysword", () -> new FairySword(toolEMERALD)); }\n        }\n    ''', encoding='utf-8')
    mats=_source_tool_materials([root])
    assert mats['experiencesword']['uses']==1300
    assert mats['fairysword']['enchantability']==75
    source='''class UltimateBow extends Item { UltimateBow(){super(new Item.Properties().stacksTo(1).durability(1000));} public int getEnchantmentValue(){return 50;} }'''
    usage, damage, eff, ench, _ = _recover_tool_stats(source, 'Bow', None)
    assert usage == 1000 and ench == 50

def test_block_sound_and_drop_recovery():
    from mcreator_content_export import _extract_block_sound_type, _block_custom_drop_updates
    source='''class BlockPizza extends Block { BlockPizza(){super(Block.Properties.of().sound(SoundType.WOOL));} List<ItemStack> getDrops(){ return Collections.emptyList(); }}'''
    assert _extract_block_sound_type(source)=='WOOL'
    updates=_block_custom_drop_updates(source, {})
    assert updates['customDrop']=='' and updates['dropAmount']==0

def test_ore_generation_replacement_semantics_from_example():
    from mcreator_content_export import _source_worldgen_by_registry
    root=ROOT / 'tests' / 'fixtures' / 'ore_feature_example'
    (root/'src/main/resources/data/demo/worldgen/configured_feature').mkdir(parents=True, exist_ok=True)
    (root/'src/main/resources/data/demo/worldgen/placed_feature').mkdir(parents=True, exist_ok=True)
    (root/'src/main/resources/data/demo/worldgen/configured_feature/deep.json').write_text(json.dumps({'type':'minecraft:ore','config':{'size':4,'targets':[{'target':{'predicate':{'block':'minecraft:deepslate'},'state':{'Name':'minecraft:deepslate_diamond_ore'}}}]}}), encoding='utf-8')
    (root/'src/main/resources/data/demo/worldgen/placed_feature/deep.json').write_text(json.dumps({'feature':'demo:deep','placement':[{'type':'minecraft:count','count':2},{'type':'minecraft:height_range','height':{'type':'uniform','min_inclusive':{'absolute':-64},'max_inclusive':{'absolute':0}}}]}), encoding='utf-8')
    data=_source_worldgen_by_registry(root)['deepslatediamondore']
    assert data['frequencyPerChunks']==2 and data['frequencyOnChunk']==4
    assert data['blocksToReplace']==['Blocks.DEEPSLATE']


def test_clean_source_bow_uses_native_ranged_item_fields():
    from mcreator_content_export import _item_element
    import json
    template=json.load(open('/mnt/data/beta12_examples/workspace/elements/ItemExample.mod.json', encoding='utf-8'))
    text='class UltimateBow extends Item { public UseAnim getUseAnimation(ItemStack s){ return UseAnim.BOW; } public int getUseDuration(ItemStack s){ return 9000; } }'
    element=_item_element(Path('UltimateBow.java'),'UltimateBow','Item',text,'ultimatebow',89,template['definition'],'minecraft:bow',[])
    d=element['definition']
    assert d['enableRanged'] is True
    assert d['animation']=='bow'
    assert d['useDuration']==9000


def test_exact_stateful_block_models_are_mojmap_copied():
    from mcreator_content_export import _copy_source_block_model_resources
    import tempfile, json
    with tempfile.TemporaryDirectory() as td:
        out=Path(td)
        _copy_source_block_model_resources([Path('/mnt/data/beta12_source/chaos/Chaos-Persists-Forge-1.20.1')],'chaospersists','cndcc','ducttape',out)
        state=json.loads((out/'src/main/resources/assets/cndcc/blockstates/ducttape.json').read_text())
        assert state['variants']['slices=0']['model']=='cndcc:block/ducttape_0'
        assert (out/'src/main/resources/assets/cndcc/models/block/ducttape_5.json').exists()


def test_real_source_ore_generation_is_enabled_and_uses_single_replace_block():
    from mcreator_content_export import _source_worldgen_by_registry
    data=_source_worldgen_by_registry(Path('/mnt/data/beta12_source/chaos/Chaos-Persists-Forge-1.20.1'))
    assert data['oreruby']['generateFeature'] is True
    assert data['oreruby']['blocksToReplace']==['Blocks.STONE']
    assert data['deepslateoreruby']['blocksToReplace']==['Blocks.DEEPSLATE']
    assert data['oreruby']['frequencyOnChunk']==5

def test_recipe_normalizes_lowercase_nested_holders(tmp_path):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from mcreator_content_export import _normalize_mcreator_item_ref, _java_recipe_item_ref
    assert _normalize_mcreator_item_ref('items.ITEMS.stick') == 'Items.STICK'
    assert _normalize_mcreator_item_ref('items.ITEMS.diamond') == 'Items.DIAMOND'
    assert _normalize_mcreator_item_ref('Blocks.BLOCKS.lapis_block') == 'Blocks.LAPIS_BLOCK'
    assert _java_recipe_item_ref('new ItemStack(items.ITEMS.stick)', {}, {}, '') == 'Items.STICK'

def test_ranged_items_use_compile_safe_native_arrow():
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from mcreator_content_export import _ranged_behavior_updates
    d = _ranged_behavior_updates('class SkateBowItem extends Item { public void x(){ UseAnim.BOW; new IrukandjiArrow(); } }', 'skate_bow', {'useDuration': 0})
    assert d['enableRanged'] is True
    assert d['projectile'] == 'Arrow'
    assert d['mojmodelbridge_source_projectile_class'] == 'IrukandjiArrow'
