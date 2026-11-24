from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tempfile import TemporaryDirectory

from mcreator_source_bridge import _item_java, _patch_crafted_procedure_calls, generate_compile_bridge

STALE = '''import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.entity.player.Player;

public class ExampleItem {
  public void onCraftedBy(ItemStack itemstack, Level world, Player entity) {
    ExampleItemIsCraftedProcedure.execute();
  }
}
'''


def test_item_java_uses_context_aware_crafted_procedure_call():
    text = _item_java("net.example", "ExampleItem", "ExampleItemIsCrafted")
    assert "onCraftedBy(ItemStack itemstack, Level world, Player entity)" in text
    assert "ExampleItemIsCraftedProcedure.execute(world, itemstack);" in text
    assert "ExampleItemIsCraftedProcedure.execute();" not in text


def test_existing_stale_call_is_repaired():
    with TemporaryDirectory() as d:
        root = Path(d)
        java = root / "src/main/java/net/example/item/ExampleItem.java"
        java.parent.mkdir(parents=True)
        java.write_text(STALE, encoding="utf-8")
        assert _patch_crafted_procedure_calls(root) == 1
        result = java.read_text(encoding="utf-8")
        assert "ExampleItemIsCraftedProcedure.execute(world, itemstack);" in result
        assert "ExampleItemIsCraftedProcedure.execute();" not in result


def test_neoforge_bridge_repairs_stale_existing_item_source():
    with TemporaryDirectory() as d:
        root = Path(d)
        java = root / "src/main/java/net/example/item/ExampleItem.java"
        java.parent.mkdir(parents=True)
        java.write_text(STALE, encoding="utf-8")
        result = generate_compile_bridge(
            output_root=root,
            package="net.example",
            modid="example",
            workspace={"mod_elements": []},
            entity_exports=[],
            content_exports=[],
            entity_meta={},
            target_generator="neoforge-1.21.1",
        )
        assert result["patched_crafted_procedure_calls"] == 1
        assert "ExampleItemIsCraftedProcedure.execute(world, itemstack);" in java.read_text(encoding="utf-8")


def test_generated_procedure_metadata_declares_itemstack_and_world_dependencies():
    from mcreator_content_export import _add_crafted_enchantment_procedure
    with TemporaryDirectory() as d:
        root = Path(d)
        workspace = {"mod_elements": []}
        element = {"_fv": 89, "_type": "tool", "definition": {}}
        out = root / "elements"
        out.mkdir()
        used = set()
        name = _add_crafted_enchantment_procedure(
            element,
            '''class X {
void onCraftedBy(ItemStack stack, Level level, Player player) {
    apply(stack);
}
void apply(ItemStack stack) {
    stack.enchant(Enchantments.BLOCK_EFFICIENCY, 5);
}
}''',
            'PickaxeExample',
            out,
            workspace,
            used,
            'chaospersists',
        )
        assert name
        entry = next(x for x in workspace['mod_elements'] if x['name'] == name)
        assert entry['metadata']['dependencies'] == [
            {'name': 'itemstack', 'type': 'itemstack'},
            {'name': 'world', 'type': 'world'},
        ]
