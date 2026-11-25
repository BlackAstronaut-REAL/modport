//! Version-aware method catalogue for Forge-era source APIs.
//!
//! Entries describe source-side behavior, not text substitutions. The three
//! legacy releases are kept explicit because signatures and extension points
//! changed between them. Target APIs name the nearest NeoForge 1.21.1 surface;
//! entries marked as adapters still require semantic reconstruction.

use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const V_ALL: &[&str] = &["1.7.10", "1.8.9", "1.12.2"];
const V_18: &[&str] = &["1.8.9", "1.12.2"];
const V_112: &[&str] = &["1.12.2"];
const V_17: &[&str] = &["1.7.10"];
const V_17_18: &[&str] = &["1.7.10", "1.8.9"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyApiMethodMapping {
    pub source_owner: String,
    pub source_method: String,
    pub source_versions: Vec<String>,
    pub normalized_behavior: String,
    pub target_api: String,
    pub category: String,
    pub conversion_status: String,
    pub notes: String,
    pub documentation_urls: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Definition {
    owner: &'static str,
    method: &'static str,
    versions: &'static [&'static str],
    behavior: &'static str,
    target: &'static str,
    category: &'static str,
    status: &'static str,
    notes: &'static str,
    docs: &'static str,
}

macro_rules! d {
    ($owner:literal, $method:literal, $versions:ident, $behavior:literal, $target:literal, $category:literal, $status:literal, $notes:literal, $docs:literal) => {
        Definition {
            owner: $owner, method: $method, versions: $versions, behavior: $behavior,
            target: $target, category: $category, status: $status, notes: $notes, docs: $docs,
        }
    };
}

const DEFINITIONS: &[Definition] = &[
    // Item lifecycle, interaction, combat and presentation.
    d!("Item", "onItemRightClick", V_ALL, "item_use", "Item#use", "item_interaction", "adapter_required", "Preserve hand, player, side, returned stack and action-result semantics; 1.7.10 uses a different signature and result type.", "net/minecraft/item/Item.html"),
    d!("Item", "onItemUse", V_ALL, "block_use", "Item#useOn / Block interaction events", "item_interaction", "adapter_required", "Translate integer coordinates and hit fractions into UseOnContext; preserve PASS/SUCCESS/FAIL and stack mutation.", "net/minecraft/item/Item.html"),
    d!("Item", "onItemUseFirst", V_18, "pre_block_use", "IItemExtension#onItemUseFirst / interaction events", "item_interaction", "adapter_required", "Runs before normal block activation; preserve cancellation and result ordering.", "net/minecraft/item/Item.html"),
    d!("Item", "onUpdate", V_ALL, "inventory_tick", "Item#inventoryTick", "item_lifecycle", "adapter_required", "Distinguish inventory ticking from dropped ItemEntity ticking; preserve slot and selected state.", "net/minecraft/item/Item.html"),
    d!("Item", "onEntityItemUpdate", V_18, "dropped_item_tick", "IItemExtension#onEntityItemUpdate", "item_lifecycle", "adapter_required", "Preserve the dropped ItemEntity context and true-means-skip behavior.", "net/minecraft/item/Item.html"),
    d!("Item", "onItemUseFinish", V_18, "finish_using", "Item#finishUsingItem", "item_lifecycle", "adapter_required", "Return the resulting stack and preserve consumption, effects and container conversion.", "net/minecraft/item/Item.html"),
    d!("Item", "onUsingTick", V_18, "active_use_tick", "Item#onUseTick", "item_lifecycle", "adapter_required", "Check countdown direction and remaining-use-duration semantics.", "net/minecraft/item/Item.html"),
    d!("Item", "onPlayerStoppedUsing", V_ALL, "stop_using", "Item#releaseUsing", "item_lifecycle", "adapter_required", "Translate the remaining-use duration and preserve bow/charge release timing.", "net/minecraft/item/Item.html"),
    d!("Item", "onCreated", V_ALL, "crafted_stack_init", "Item#onCraftedBy / crafting events", "item_lifecycle", "adapter_required", "Preserve crafting player context and stack initialization.", "net/minecraft/item/Item.html"),
    d!("Item", "onEaten", V_17, "food_consumed", "Item#finishUsingItem", "food", "adapter_required", "Legacy ItemFood callback; preserve player effects, stack decrement and returned container.", "net/minecraft/item/ItemFood.html"),
    d!("ItemFood", "onEaten", V_18, "food_consumed", "Item#finishUsingItem", "food", "adapter_required", "Preserve the modern living-entity consumer and resulting stack.", "net/minecraft/item/ItemFood.html"),
    d!("Item", "hitEntity", V_ALL, "melee_hit", "Item#hurtEnemy", "combat", "adapter_required", "Translate target and attacker types; preserve durability damage and return semantics.", "net/minecraft/item/Item.html"),
    d!("Item", "onBlockDestroyed", V_ALL, "block_mined", "Item#mineBlock", "tool", "adapter_required", "Convert World/Block/coordinates to Level/BlockState/BlockPos and preserve tool wear.", "net/minecraft/item/Item.html"),
    d!("Item", "itemInteractionForEntity", V_ALL, "entity_interaction", "Item#interactLivingEntity", "item_interaction", "adapter_required", "Preserve the clicked living entity, player, hand and interaction result.", "net/minecraft/item/Item.html"),
    d!("Item", "onLeftClickEntity", V_18, "entity_attack_pre", "IItemExtension#onLeftClickEntity / attack events", "combat", "adapter_required", "Translate the old boolean cancellation convention into modern interaction results/events.", "net/minecraft/item/Item.html"),
    d!("Item", "onEntitySwing", V_18, "entity_swing", "IItemExtension#onEntitySwing / living attack animation events", "combat", "adapter_required", "The compatibility hook is deprecated in the target line; prefer a hand-aware event when possible.", "net/minecraft/item/Item.html"),
    d!("Item", "onDroppedByPlayer", V_18, "player_drop", "ItemTossEvent", "events", "event_adapter_required", "Move to item-toss event handling and preserve cancellation and stack transfer.", "net/minecraft/item/Item.html"),
    d!("Item", "onItemPickup", V_18, "item_pickup", "ItemPickupEvent / player item-pickup event", "events", "event_adapter_required", "Preserve pickup timing, cancellation, and stack merge behavior.", "net/minecraft/item/Item.html"),
    d!("Item", "getItemUseAction", V_ALL, "use_animation", "Item#getUseAnimation", "item_presentation", "enum_adapter_required", "Map EnumAction values to UseAnim and retain stack-sensitive behavior.", "net/minecraft/item/Item.html"),
    d!("Item", "getMaxItemUseDuration", V_ALL, "use_duration", "Item#getUseDuration", "item_presentation", "signature_adapter_required", "The target query is stack/entity-sensitive; preserve charge and food timings.", "net/minecraft/item/Item.html"),
    d!("Item", "addInformation", V_ALL, "tooltip_lines", "Item#appendHoverText", "tooltip", "adapter_required", "Translate String tooltip lines and formatting into Component; retain client/server safety and conditional logic.", "net/minecraft/item/Item.html"),
    d!("Item", "getItemStackDisplayName", V_ALL, "stack_display_name", "Item#getName", "localization", "signature_adapter_required", "Translate String to Component and preserve stack-sensitive naming.", "net/minecraft/item/Item.html"),
    d!("Item", "getHighlightTip", V_18, "highlight_name", "Item#getName / tooltip Component composition", "localization", "adapter_required", "Preserve appended mode/state text while avoiding duplicated display names.", "net/minecraft/item/Item.html"),
    d!("Item", "getUnlocalizedName", V_ALL, "translation_key", "Item#getDescriptionId", "localization", "property_recovery", "Recover the translation key, then migrate language entries to modern item translation keys.", "net/minecraft/item/Item.html"),
    d!("Item", "getUnlocalizedNameInefficiently", V_ALL, "stack_translation_key", "ItemStack#getHoverName / Item#getName", "localization", "adapter_required", "Preserve metadata-sensitive key selection; this is not a safe direct rename.", "net/minecraft/item/Item.html"),
    d!("Item", "hasEffect", V_ALL, "enchantment_glint", "Item#isFoil / DataComponents#ENCHANTMENT_GLINT_OVERRIDE", "rendering", "adapter_required", "Preserve stack-dependent glint; constant values can use the glint override component.", "net/minecraft/item/Item.html"),
    d!("Item", "getColorFromItemStack", V_ALL, "item_tint", "ItemColor registration / ItemColor", "rendering", "adapter_required", "Move tint logic to client item-color registration; preserve render-pass behavior where present.", "net/minecraft/item/Item.html"),
    d!("Item", "getRarity", V_ALL, "item_rarity", "ItemStack#getRarity / Rarity", "item_presentation", "adapter_required", "Map EnumRarity or IRarity to modern Rarity and retain stack-sensitive rules.", "net/minecraft/item/Item.html"),
    d!("Item", "getSubItems", V_ALL, "creative_tab_variants", "CreativeModeTab display items / Item#fillItemCategory", "item_registration", "adapter_required", "Translate creative inventory population to tab display-item providers and preserve metadata variants.", "net/minecraft/item/Item.html"),
    d!("Item", "getCreativeTab", V_ALL, "creative_tab", "CreativeModeTab / Item.Properties#tab", "item_registration", "property_recovery", "Recover the selected creative tab; modern tabs are registered separately.", "net/minecraft/item/Item.html"),
    d!("Item", "getCreativeTabs", V_18, "multiple_creative_tabs", "CreativeModeTab display item providers", "item_registration", "adapter_required", "Register each target tab separately and avoid duplicate stack insertion.", "net/minecraft/item/Item.html"),
    d!("Item", "getItemEnchantability", V_ALL, "enchantability", "IItemExtension#getEnchantmentValue / Item# getEnchantmentValue", "enchanting", "adapter_required", "Preserve stack-sensitive enchantability and target overload differences.", "net/minecraft/item/Item.html"),
    d!("Item", "getIsRepairable", V_ALL, "repair_predicate", "Item#isValidRepairItem", "repair", "adapter_required", "Translate candidate repair stacks, tags and material checks.", "net/minecraft/item/Item.html"),
    d!("Item", "getItemAttributeModifiers", V_ALL, "item_attributes", "ItemAttributeModifiers / IItemExtension#getDefaultAttributeModifiers", "attributes", "adapter_required", "Translate UUIDs, operation types, slots and stack sensitivity.", "net/minecraft/item/Item.html"),
    d!("Item", "getAttributeModifiers", V_18, "stack_item_attributes", "IItemExtension#getDefaultAttributeModifiers", "attributes", "adapter_required", "1.12.2 adds an equipment-slot parameter; preserve slot and stack-dependent modifiers.", "net/minecraft/item/Item.html"),
    d!("Item", "getContainerItem", V_ALL, "crafting_remainder", "Item.Properties#craftRemainder / IItemExtension#getCraftingRemainingItem", "crafting", "adapter_required", "Static remainders use a property; stack-dependent remainders need an extension adapter.", "net/minecraft/item/Item.html"),
    d!("Item", "hasContainerItem", V_ALL, "has_crafting_remainder", "Item#hasCraftingRemainingItem", "crafting", "adapter_required", "Reconstruct both the presence predicate and resulting stack.", "net/minecraft/item/Item.html"),
    d!("Item", "doesContainerItemLeaveCraftingGrid", V_17, "crafting_remainder_grid", "CraftingContainer remainder handling", "crafting", "adapter_required", "Preserve whether the remainder stays in the crafting grid.", "net/minecraft/item/Item.html"),
    d!("Item", "getEntityLifespan", V_18, "dropped_item_lifespan", "IItemExtension#getEntityLifespan", "item_entity", "adapter_required", "Preserve per-stack and per-level lifespan calculations.", "net/minecraft/item/Item.html"),
    d!("Item", "createEntity", V_18, "replace_dropped_item_entity", "IItemExtension#hasCustomEntity / Item#createEntity", "item_entity", "adapter_required", "Port both custom-entity detection and replacement entity construction.", "net/minecraft/item/Item.html"),
    d!("Item", "doesSneakBypassUse", V_18, "sneak_use_bypass", "IItemExtension#doesSneakBypassUse", "item_interaction", "adapter_required", "Translate IBlockAccess/World and position arguments into modern read-only level context.", "net/minecraft/item/Item.html"),
    d!("Item", "canApplyAtEnchantingTable", V_112, "enchantability_predicate", "IItemExtension#isEnchantable / enchantment events", "enchanting", "adapter_required", "Preserve item/enchantment-specific restrictions.", "net/minecraft/item/Item.html"),
    d!("Item", "getHarvestLevel", V_18, "tool_harvest_level", "Tool data component rules / Item#isCorrectToolForDrops", "tool", "adapter_required", "Translate tool class and block context into tags and Tool rules.", "net/minecraft/item/Item.html"),
    d!("Item", "getDurabilityForDisplay", V_18, "durability_bar_fraction", "Item#getBarWidth", "item_presentation", "adapter_required", "Convert normalized durability fraction to the target 0-13 width.", "net/minecraft/item/Item.html"),
    d!("Item", "getFontRenderer", V_18, "tooltip_font", "Tooltip rendering hooks", "rendering", "manual_review", "Client rendering implementation; no safe automatic server-side replacement.", "net/minecraft/item/Item.html"),
    d!("Item", "registerIcons", V_17, "legacy_item_icons", "Item model JSON / client model registration", "rendering", "resource_adapter_required", "1.7.10 IIcon registration must be converted into model and texture resources.", "net/minecraft/item/Item.html"),
    d!("Item", "getIconFromDamage", V_17, "metadata_item_icon", "Item model JSON / ItemOverrides", "rendering", "resource_adapter_required", "Translate metadata-based icon selection into model variants or overrides.", "net/minecraft/item/Item.html"),
    d!("Item", "getIconFromDamageForRenderPass", V_17, "multi_pass_item_icon", "Item model JSON / layered render pipeline", "rendering", "resource_adapter_required", "Convert per-pass texture selection into layered models or custom rendering.", "net/minecraft/item/Item.html"),
    d!("Item", "getIconIndex", V_17, "stack_item_icon", "Item model JSON / ItemOverrides", "rendering", "resource_adapter_required", "Preserve stack-sensitive icon selection through item overrides or custom renderers.", "net/minecraft/item/Item.html"),
    d!("Item", "getSpriteNumber", V_17, "texture_atlas_selection", "TextureAtlas / model resources", "rendering", "resource_adapter_required", "The old block/item atlas selection model no longer maps directly.", "net/minecraft/item/Item.html"),
    d!("Item", "getTextureName", V_17, "legacy_item_texture", "textures/item resource path / model JSON", "rendering", "resource_adapter_required", "Recover texture path and namespace; legacy texture-name strings are not modern resource locations.", "net/minecraft/item/Item.html"),
    d!("Item", "setTextureName", V_17, "legacy_item_texture", "textures/item resource path / model JSON", "rendering", "resource_adapter_required", "Translate the texture name into a namespaced resource location and model reference.", "net/minecraft/item/Item.html"),
    d!("Item", "requiresMultipleRenderPasses", V_17, "multi_pass_item_render", "Item model JSON / custom item renderer", "rendering", "resource_adapter_required", "Represent separate passes as layers or a custom renderer.", "net/minecraft/item/Item.html"),
    d!("Item", "getMovingObjectPositionFromPlayer", V_17, "player_ray_trace", "Level#clip / ClipContext", "item_interaction", "adapter_required", "Translate player eye/look vectors and liquid/solid hit flags into ClipContext.", "net/minecraft/item/Item.html"),
    d!("Item", "getPotionEffect", V_17, "legacy_potion_effect", "FoodProperties.Builder#effect / MobEffectInstance", "food", "adapter_required", "Parse legacy potion-effect strings only when their grammar is known; otherwise preserve as a manual conversion.", "net/minecraft/item/Item.html"),
    d!("Item", "getItemBurnTime", V_18, "item_fuel_time", "IItemExtension#getBurnTime / fuel recipe integration", "fuel", "adapter_required", "Preserve item-stack and recipe-type-specific burn time.", "net/minecraft/item/Item.html"),
    d!("Item", "getShareTag", V_ALL, "stack_share_tag_enabled", "DataComponentType persistent/network codec", "networking", "component_adapter_required", "Replace ad-hoc share-tag transport with persistent and network-synchronized components.", "net/minecraft/item/Item.html"),
    d!("Item", "getHasSubtypes", V_ALL, "has_item_variants", "DataComponents / ItemStack components / item model overrides", "item_data", "adapter_required", "Recover metadata/subtype use and split distinct registry objects where modern components are insufficient.", "net/minecraft/item/Item.html"),
    d!("Item", "getMetadata", V_ALL, "item_metadata", "ItemStack components / item model overrides", "item_data", "adapter_required", "Metadata-based item identity should become explicit components, tags or separate items.", "net/minecraft/item/Item.html"),
    d!("Item", "isDamageable", V_ALL, "item_damageable", "DataComponents#MAX_DAMAGE", "item_data", "property_recovery", "Recover whether durability is enabled and emit an appropriate max-damage component.", "net/minecraft/item/Item.html"),
    d!("Item", "isFull3D", V_ALL, "legacy_full_3d_flag", "Item model JSON / display transforms", "rendering", "resource_adapter_required", "Convert full-3D held rendering into model geometry and display transforms.", "net/minecraft/item/Item.html"),
    d!("Item", "setFull3D", V_ALL, "legacy_full_3d_flag", "Item model JSON / display transforms", "rendering", "resource_adapter_required", "This is a presentation flag, not a gameplay property.", "net/minecraft/item/Item.html"),
    d!("Item", "isItemTool", V_ALL, "tool_class_predicate", "Tool data component rules", "tool", "adapter_required", "Reconstruct tool tags and block rules; avoid assuming every old ItemTool maps one-to-one.", "net/minecraft/item/Item.html"),
    d!("Item", "canItemEditBlocks", V_ALL, "can_edit_world", "Item#canPerformAction / interaction events", "item_interaction", "adapter_required", "Preserve adventure-mode and interaction restrictions.", "net/minecraft/item/Item.html"),
    d!("Item", "setContainerItem", V_ALL, "crafting_remainder_property", "Item.Properties#craftRemainder", "crafting", "property_recovery", "Static container items map to a target remainder property; dynamic values need an extension.", "net/minecraft/item/Item.html"),
    d!("Item", "setCreativeTab", V_ALL, "creative_tab_property", "CreativeModeTab registration / Item.Properties#tab", "item_registration", "property_recovery", "Move tab membership into modern creative tab display registration.", "net/minecraft/item/Item.html"),
    d!("Item", "setHasSubtypes", V_ALL, "subtype_flag", "ItemStack components / ItemOverrides", "item_data", "adapter_required", "Recover all metadata variants before selecting a modern item representation.", "net/minecraft/item/Item.html"),
    d!("Item", "setMaxDamage", V_ALL, "max_durability", "Item.Properties#durability / DataComponents#MAX_DAMAGE", "item_data", "property_recovery", "Static values can use Item.Properties#durability; dynamic stack-specific values need components.", "net/minecraft/item/Item.html"),
    d!("Item", "setMaxStackSize", V_ALL, "max_stack_size", "Item.Properties#stacksTo / DataComponents#MAX_STACK_SIZE", "item_data", "property_recovery", "Static stack sizes map directly to stacksTo.", "net/minecraft/item/Item.html"),
    d!("Item", "setPotionEffect", V_ALL, "food_potion_effect", "FoodProperties.Builder#effect", "food", "property_recovery", "Recover the effect, duration, amplifier and probability.", "net/minecraft/item/Item.html"),
    d!("Item", "setUnlocalizedName", V_ALL, "translation_key", "Item.Properties#descriptionId / language resources", "localization", "property_recovery", "Move the old unlocalized name into a stable registry key and modern language file.", "net/minecraft/item/Item.html"),

    // Block lifecycle, state, shape, drops, and interaction.
    d!("Block", "breakBlock", V_ALL, "block_removed", "Block#onRemove", "block_lifecycle", "adapter_required", "Preserve old/new state comparison and block-entity teardown ordering.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockAdded", V_ALL, "block_placed_in_world", "Block#onPlace", "block_lifecycle", "adapter_required", "Translate metadata/world coordinates into BlockState and BlockPos.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockPlacedBy", V_ALL, "block_placed_by_entity", "Block#setPlacedBy", "block_lifecycle", "adapter_required", "Preserve placer, ItemStack, facing and block-entity initialization.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockPlaced", V_ALL, "block_placement_state", "Block#getStateForPlacement", "block_state", "adapter_required", "Return the modern BlockState rather than mutating metadata after placement.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockActivated", V_ALL, "block_use", "Block#use", "block_interaction", "adapter_required", "Translate clicked coordinates and side into UseOnContext and preserve client prediction.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockClicked", V_ALL, "block_left_click", "PlayerInteractEvent.LeftClickBlock / block attack hooks", "block_interaction", "event_adapter_required", "Preserve timing relative to break progress and cancellation.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockHarvested", V_ALL, "block_harvested", "Block#playerWillDestroy / block-break events", "block_drops", "adapter_required", "Preserve player, harvest tool, silk touch and drop timing.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockDestroyedByPlayer", V_ALL, "block_destroyed_by_player", "Block#onRemove / block-break events", "block_lifecycle", "event_adapter_required", "Reconstruct player context via events; onRemove alone does not imply player destruction.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockDestroyedByExplosion", V_ALL, "block_destroyed_by_explosion", "Block#wasExploded", "block_lifecycle", "adapter_required", "Preserve explosion context and whether the block was actually removed.", "net/minecraft/block/Block.html"),
    d!("Block", "onEntityCollidedWithBlock", V_ALL, "entity_inside_block", "Block#entityInside", "block_interaction", "adapter_required", "Translate entity collision callback and preserve per-tick behavior.", "net/minecraft/block/Block.html"),
    d!("Block", "onEntityWalking", V_ALL, "entity_walked_on_block", "Block#stepOn", "block_interaction", "adapter_required", "Preserve foot contact timing, entity type and velocity changes.", "net/minecraft/block/Block.html"),
    d!("Block", "onFallenUpon", V_ALL, "entity_fell_on_block", "Block#fallOn", "block_interaction", "adapter_required", "Translate fall distance and landing damage semantics.", "net/minecraft/block/Block.html"),
    d!("Block", "onNeighborBlockChange", V_ALL, "neighbor_changed", "Block#neighborChanged", "block_updates", "signature_adapter_required", "The old neighbor block argument and coordinate signature differ from modern state/position callbacks.", "net/minecraft/block/Block.html"),
    d!("Block", "updateTick", V_ALL, "scheduled_block_tick", "Block#tick", "block_updates", "adapter_required", "Preserve scheduled delay, random source, state and server-side execution.", "net/minecraft/block/Block.html"),
    d!("Block", "randomTick", V_18, "random_block_tick", "Block#randomTick", "block_updates", "adapter_required", "Preserve random-tick speed assumptions and server-only mutation.", "net/minecraft/block/Block.html"),
    d!("Block", "randomDisplayTick", V_ALL, "client_random_display_tick", "Block#animateTick", "block_rendering", "adapter_required", "Client-only particles/sounds should move to animateTick or a client effect adapter.", "net/minecraft/block/Block.html"),
    d!("Block", "getDrops", V_18, "block_drop_list", "Block#getDrops / loot table", "block_drops", "adapter_required", "Prefer loot tables for static behavior; custom conditional drops need a LootItemCondition or block adapter.", "net/minecraft/block/Block.html"),
    d!("Block", "quantityDropped", V_ALL, "drop_quantity", "Loot table functions", "block_drops", "property_recovery", "Move deterministic quantities to loot tables; random quantities need explicit providers/functions.", "net/minecraft/block/Block.html"),
    d!("Block", "quantityDroppedWithBonus", V_ALL, "fortune_drop_quantity", "Loot table bonus count functions", "block_drops", "property_recovery", "Translate fortune scaling to loot table bonus-count functions.", "net/minecraft/block/Block.html"),
    d!("Block", "damageDropped", V_ALL, "drop_metadata", "Loot table / block-state properties", "block_drops", "adapter_required", "Old metadata drops should become explicit block states or loot table conditions.", "net/minecraft/block/Block.html"),
    d!("Block", "getItemDropped", V_ALL, "drop_item", "Block loot table", "block_drops", "property_recovery", "Static item drops map directly to loot tables.", "net/minecraft/block/Block.html"),
    d!("Block", "getExpDrop", V_18, "experience_drop", "Block#getExpDrop / loot tables", "block_drops", "adapter_required", "Preserve tool enchantments, block state and random XP ranges.", "net/minecraft/block/Block.html"),
    d!("Block", "getPickBlock", V_18, "pick_block_stack", "Block#getCloneItemStack", "block_drops", "adapter_required", "Translate hit result and block state to the modern clone-stack query.", "net/minecraft/block/Block.html"),
    d!("Block", "getBlockHardness", V_ALL, "block_hardness", "BlockBehaviour.Properties#destroyTime", "block_properties", "property_recovery", "Recover constant hardness when possible; world/state-dependent hardness needs custom behavior.", "net/minecraft/block/Block.html"),
    d!("Block", "getExplosionResistance", V_ALL, "explosion_resistance", "BlockBehaviour.Properties#explosionResistance", "block_properties", "property_recovery", "Recover constant resistance; entity/world-dependent behavior needs an adapter.", "net/minecraft/block/Block.html"),
    d!("Block", "getLightValue", V_ALL, "emitted_light", "BlockBehaviour.Properties#lightLevel", "block_properties", "property_recovery", "Translate legacy 0-15 light values to the modern state-aware light function.", "net/minecraft/block/Block.html"),
    d!("Block", "getLightOpacity", V_ALL, "light_blocking", "BlockBehaviour.Properties#lightBlock", "block_properties", "property_recovery", "Preserve state-dependent opacity and neighbor-brightness behavior.", "net/minecraft/block/Block.html"),
    d!("Block", "isOpaqueCube", V_ALL, "opaque_cube", "BlockState#isSolidRender / render shape", "block_shape", "adapter_required", "Modern solidity and render shape are separate properties; do not conflate them.", "net/minecraft/block/Block.html"),
    d!("Block", "isFullCube", V_ALL, "full_cube", "BlockState#isCollisionShapeFullBlock", "block_shape", "adapter_required", "Use collision and visual shapes independently.", "net/minecraft/block/Block.html"),
    d!("Block", "isNormalCube", V_ALL, "normal_cube", "BlockState#isRedstoneConductor / shape queries", "block_shape", "adapter_required", "Recover the actual property being queried; legacy normal-cube semantics bundled several concepts.", "net/minecraft/block/Block.html"),
    d!("Block", "getCollisionBoundingBox", V_ALL, "collision_shape", "Block#getCollisionShape", "block_shape", "adapter_required", "Convert AxisAlignedBB to VoxelShape and preserve position/state dependencies.", "net/minecraft/block/Block.html"),
    d!("Block", "addCollisionBoxesToList", V_ALL, "collision_shape_list", "Block#getCollisionShape", "block_shape", "adapter_required", "Combine per-box collision output into a VoxelShape; retain entity-dependent collision only with custom logic.", "net/minecraft/block/Block.html"),
    d!("Block", "getSelectedBoundingBox", V_ALL, "outline_shape", "Block#getShape", "block_shape", "adapter_required", "Convert selection AABB to VoxelShape and preserve state/position dependence.", "net/minecraft/block/Block.html"),
    d!("Block", "setBlockBounds", V_ALL, "static_block_bounds", "Block#getShape / Block#getCollisionShape", "block_shape", "resource_adapter_required", "Static bounds can become a box VoxelShape; dynamic bounds need state-aware shape methods.", "net/minecraft/block/Block.html"),
    d!("Block", "getRenderType", V_ALL, "legacy_render_type", "BlockState#getRenderShape / model JSON", "block_rendering", "resource_adapter_required", "Map MODEL, INVISIBLE, ENTITYBLOCK_ANIMATED and crossed geometry to modern render shapes/models.", "net/minecraft/block/Block.html"),
    d!("Block", "getBlockLayer", V_18, "render_layer", "RenderType / client render-layer registration", "block_rendering", "client_adapter_required", "Map translucent/cutout layers to modern render types and client registration.", "net/minecraft/block/Block.html"),
    d!("Block", "getStateFromMeta", V_18, "metadata_to_state", "Block#getStateForPlacement / StateDefinition", "block_state", "adapter_required", "Create explicit block-state properties and translate each metadata combination.", "net/minecraft/block/Block.html"),
    d!("Block", "getMetaFromState", V_18, "state_to_metadata", "BlockState properties / serialization", "block_state", "adapter_required", "Metadata is no longer the state identity; preserve serialization only where compatibility requires it.", "net/minecraft/block/Block.html"),
    d!("Block", "getDefaultState", V_18, "default_block_state", "Block#registerDefaultState", "block_state", "property_recovery", "Recover the default property assignments and register them after state definition.", "net/minecraft/block/Block.html"),
    d!("Block", "isReplaceable", V_ALL, "replaceable_block", "Block#canBeReplaced", "block_properties", "adapter_required", "Preserve placement context and fluid/plant replacement rules.", "net/minecraft/block/Block.html"),
    d!("Block", "isLeaves", V_ALL, "leaves_classification", "BlockTags#LEAVES / LeavesBlock behavior", "block_properties", "adapter_required", "Prefer tags and native leaves behavior over an isolated boolean.", "net/minecraft/block/Block.html"),
    d!("Block", "isWood", V_ALL, "wood_classification", "BlockTags#LOGS / BlockTags#PLANKS", "block_properties", "adapter_required", "Translate classification checks into appropriate block tags.", "net/minecraft/block/Block.html"),
    d!("Block", "canBlockStay", V_ALL, "placement_survival", "Block#canSurvive", "block_state", "adapter_required", "Preserve support-block and neighbor conditions.", "net/minecraft/block/Block.html"),
    d!("Block", "canPlaceBlockAt", V_ALL, "placement_validity", "Block#canSurvive / Block#mayPlaceOn", "block_state", "adapter_required", "Translate coordinate checks and placement context into state survival rules.", "net/minecraft/block/Block.html"),
    d!("Block", "registerBlockIcons", V_17, "legacy_block_icons", "Block model JSON / blockstate resources", "block_rendering", "resource_adapter_required", "Convert IIcon atlas registration to block model and texture assets.", "net/minecraft/block/Block.html"),
    d!("Block", "getIcon", V_ALL, "block_face_texture", "Block model JSON / model variants", "block_rendering", "resource_adapter_required", "Map face/metadata texture selection to model JSON variants or multipart definitions.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockEventReceived", V_ALL, "block_event", "Block#triggerEvent / BlockEntity event handling", "block_updates", "adapter_required", "Preserve event IDs and client synchronization through block-entity packets where necessary.", "net/minecraft/block/Block.html"),
    d!("Block", "onBlockPreDestroy", V_ALL, "block_pre_destroy", "Block#playerWillDestroy", "block_lifecycle", "adapter_required", "Preserve pre-drop ordering and player context.", "net/minecraft/block/Block.html"),
    d!("Block", "getMapColor", V_18, "map_color", "BlockBehaviour.Properties#mapColor", "block_properties", "property_recovery", "Recover fixed map color or a state-dependent color function.", "net/minecraft/block/Block.html"),

    // Entity base class and synchronized entity state.
    d!("Entity", "entityInit", V_ALL, "define_synced_data", "Entity#defineSynchedData", "entity_data", "adapter_required", "Translate DataWatcher entries and serializers to EntityDataAccessor types.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "onUpdate", V_ALL, "entity_tick", "Entity#tick", "entity_lifecycle", "adapter_required", "Preserve tick phase and avoid duplicating superclass ticking.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "onEntityUpdate", V_ALL, "entity_base_tick", "Entity#baseTick / Entity#tick", "entity_lifecycle", "adapter_required", "Separate base environmental updates from subclass tick logic.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "setDead", V_ALL, "discard_entity", "Entity#discard", "entity_lifecycle", "automatic_or_adapter", "Direct discard is safe only when removal reason and side effects are equivalent.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "isEntityAlive", V_ALL, "entity_alive", "Entity#isAlive / LivingEntity#isAlive", "entity_state", "automatic_rename_candidate", "Check whether callers require death-state, removal-state or health-state semantics.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "setPosition", V_ALL, "entity_position", "Entity#setPos", "entity_movement", "signature_adapter_required", "Preserve bounding-box refresh and server tracking behavior.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "setPositionAndRotation", V_ALL, "entity_position_rotation", "Entity#moveTo", "entity_movement", "signature_adapter_required", "Translate yaw/pitch and preserve interpolation/network update expectations.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "setVelocity", V_ALL, "entity_velocity", "Entity#setDeltaMovement", "entity_movement", "signature_adapter_required", "Translate velocity vector and preserve impulse synchronization.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "moveEntity", V_ALL, "entity_collision_move", "Entity#move", "entity_movement", "signature_adapter_required", "Collision movement semantics changed; preserve step height, clipping and movement flags.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "attackEntityFrom", V_ALL, "entity_damage", "Entity#hurt", "entity_combat", "adapter_required", "Map DamageSource construction and boolean damage acceptance.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "setFire", V_ALL, "entity_ignite", "Entity#igniteForSeconds / Entity#setSecondsOnFire", "entity_combat", "signature_adapter_required", "Legacy duration is in ticks; target convenience APIs may use seconds.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "extinguish", V_ALL, "entity_extinguish", "Entity#clearFire", "entity_combat", "automatic_rename_candidate", "Preserve fire immunity and wet-state interactions.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "canBeCollidedWith", V_ALL, "entity_pickable", "Entity#isPickable", "entity_collision", "automatic_rename_candidate", "Do not confuse selection/picking with physical collision.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "canBePushed", V_ALL, "entity_pushable", "Entity#isPushable", "entity_collision", "automatic_rename_candidate", "Preserve pushability separately from pickability.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getCollisionBoundingBox", V_ALL, "entity_collision_box", "Entity#getBoundingBox", "entity_collision", "adapter_required", "Modern bounding boxes are position-based; preserve null/no-clip semantics.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "applyEntityCollision", V_ALL, "entity_collision_response", "Entity#push", "entity_collision", "adapter_required", "Translate reciprocal collision response and avoid double-applying pushes.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "onCollideWithPlayer", V_ALL, "player_entity_collision", "Entity#playerTouch", "entity_collision", "adapter_required", "Preserve pickup/interaction timing and server-only state changes.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "writeEntityToNBT", V_ALL, "entity_save_data", "Entity#addAdditionalSaveData", "persistence", "adapter_required", "Translate NBTTagCompound operations and preserve inherited save fields.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "readEntityFromNBT", V_ALL, "entity_load_data", "Entity#readAdditionalSaveData", "persistence", "adapter_required", "Translate NBT types and reconstruct defaults for absent keys.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getDistanceToEntity", V_ALL, "entity_distance", "Entity#distanceTo", "entity_math", "automatic_rename_candidate", "Verify squared versus non-squared distance at every call site.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getDistanceSqToEntity", V_ALL, "entity_distance_squared", "Entity#distanceToSqr", "entity_math", "automatic_rename_candidate", "Retain squared-distance semantics; never replace with linear distance.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "isBurning", V_ALL, "entity_burning_state", "Entity#isOnFire", "entity_state", "automatic_rename_candidate", "Keep fire state distinct from fire immunity.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "isInWater", V_ALL, "entity_in_water", "Entity#isInWater", "entity_state", "automatic_rename_candidate", "Check whether the old query includes flowing water or only material checks.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "isWet", V_ALL, "entity_wet_state", "Entity#isInWaterRainOrBubble", "entity_state", "adapter_required", "Preserve rain, water and bubble-column wetness semantics.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "isSneaking", V_ALL, "entity_crouching", "Entity#isCrouching", "entity_state", "automatic_rename_candidate", "Preserve synchronized crouching state.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "mountEntity", V_ALL, "entity_mount", "Entity#startRiding / Entity#stopRiding", "entity_movement", "adapter_required", "Translate null mount to stopRiding and check force/passenger rules.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "updateRidden", V_ALL, "riding_tick", "Entity#rideTick", "entity_movement", "adapter_required", "Preserve passenger positioning and vehicle-side tick order.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getCommandSenderName", V_17_18, "entity_legacy_name", "Entity#getName", "localization", "automatic_rename_candidate", "Use Component-based name output; preserve custom name and localization behavior.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "setCustomNameTag", V_ALL, "entity_custom_name", "Entity#setCustomName", "localization", "signature_adapter_required", "Translate String to Component and preserve visibility flags.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getUniqueID", V_ALL, "entity_uuid", "Entity#getUUID", "entity_state", "automatic_rename_candidate", "Preserve UUID identity through save/load and network synchronization.", "net/minecraft/entity/Entity.html"),

    // Living entities, combat, attributes, equipment and AI.
    d!("EntityLivingBase", "onLivingUpdate", V_ALL, "living_entity_tick", "LivingEntity#aiStep", "living_lifecycle", "adapter_required", "Separate AI, animation, potion and living-tick work; avoid moving client-only effects to server.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "damageEntity", V_ALL, "living_damage_applied", "LivingEntity#actuallyHurt", "living_combat", "adapter_required", "Preserve armor, resistance, absorption and damage-source ordering.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "heal", V_ALL, "living_heal", "LivingEntity#heal", "living_combat", "automatic_rename_candidate", "Check modern heal source and event hooks where side effects matter.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getHealth", V_ALL, "living_health_read", "LivingEntity#getHealth", "living_attributes", "automatic_rename_candidate", "Preserve float precision and max-health attribute semantics.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setHealth", V_ALL, "living_health_write", "LivingEntity#setHealth", "living_attributes", "automatic_rename_candidate", "Preserve clamping, death transition and synchronization.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getMaxHealth", V_ALL, "max_health", "Attributes#MAX_HEALTH / LivingEntity#getMaxHealth", "living_attributes", "adapter_required", "Translate attribute lookups and default attribute registration.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getEntityAttribute", V_ALL, "attribute_instance", "LivingEntity#getAttribute", "living_attributes", "adapter_required", "Map old attribute instances and namespaced attribute keys to Attribute suppliers.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getAttributeMap", V_ALL, "attribute_map", "AttributeMap / AttributeSupplier", "living_attributes", "adapter_required", "Reconstruct the attribute set at entity-type creation time.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getAIMoveSpeed", V_ALL, "ai_move_speed", "Mob#getSpeed", "living_ai", "adapter_required", "Determine whether source uses base movement speed or current navigation speed.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setAIMoveSpeed", V_ALL, "ai_move_speed_set", "Mob#setSpeed", "living_ai", "adapter_required", "Preserve movement controller and attribute interactions.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getEyeHeight", V_ALL, "eye_height", "Entity#getEyeHeight", "living_attributes", "signature_adapter_required", "Modern eye height may be pose/state-dependent.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "jump", V_ALL, "living_jump", "LivingEntity#jumpFromGround", "living_movement", "adapter_required", "Preserve jump boost, fluid state and movement input.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "knockBack", V_ALL, "living_knockback", "LivingEntity#knockback", "living_combat", "signature_adapter_required", "Translate source/strength/vector semantics and preserve knockback resistance.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "swingItem", V_ALL, "swing_hand", "LivingEntity#swing", "living_animation", "signature_adapter_required", "Map main/off hand and preserve animation packet behavior.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setRevengeTarget", V_ALL, "revenge_target", "Mob#setLastHurtByMob", "living_ai", "adapter_required", "Preserve target goal notifications and last-hurt-by state.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getAITarget", V_ALL, "revenge_target_read", "Mob#getLastHurtByMob", "living_ai", "automatic_rename_candidate", "Check whether the source expects the revenge target or current attack target.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setLastAttacker", V_ALL, "last_attacker", "LivingEntity#setLastHurtByMob", "living_ai", "adapter_required", "Preserve attacker attribution and AI target events.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getLastAttacker", V_ALL, "last_attacker_read", "LivingEntity#getLastHurtByMob", "living_ai", "automatic_rename_candidate", "Distinguish last attacker from current target.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getHeldItem", V_ALL, "held_item", "LivingEntity#getItemInHand", "equipment", "adapter_required", "Resolve which hand is meant; legacy main/offhand overloads differ by version.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getEquipmentInSlot", V_ALL, "equipment_slot_read", "LivingEntity#getItemBySlot", "equipment", "adapter_required", "Translate legacy numeric armor slots into EquipmentSlot values.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setCurrentItemOrArmor", V_ALL, "equipment_slot_write", "LivingEntity#setItemSlot", "equipment", "adapter_required", "Translate numeric slots and preserve equipment-change events.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getTotalArmorValue", V_ALL, "armor_value", "Attributes#ARMOR / armor attribute", "living_attributes", "adapter_required", "Reconstruct armor attributes and item modifiers instead of copying the aggregate integer.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "onDeath", V_ALL, "living_death", "LivingEntity#die", "living_lifecycle", "adapter_required", "Preserve drops, XP, death messages and server-side ordering.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "dropFewItems", V_ALL, "living_death_drops", "LivingEntity#dropAllDeathLoot / loot tables", "living_drops", "adapter_required", "Prefer loot tables; reconstruct looting level and random drop counts.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "dropEquipment", V_ALL, "living_equipment_drops", "LivingEntity#dropAllDeathLoot / equipment drop chances", "living_drops", "adapter_required", "Translate guaranteed, chance-based and looting-scaled equipment drops.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getExperiencePoints", V_ALL, "living_experience_drop", "Experience orb / living death loot handling", "living_drops", "adapter_required", "Preserve player-kill conditions and XP randomization.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isAIEnabled", V_ALL, "living_ai_enabled", "Mob#aiStep / GoalSelector registration", "living_ai", "adapter_required", "Modern entity AI is configured through goals and entity type setup rather than this flag.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getHurtSound", V_ALL, "living_hurt_sound", "LivingEntity#getHurtSound", "audio", "automatic_rename_candidate", "Return a modern SoundEvent and preserve damage-source-dependent selection.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getDeathSound", V_ALL, "living_death_sound", "LivingEntity#getDeathSound", "audio", "automatic_rename_candidate", "Return the registered target SoundEvent.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getLivingSound", V_ALL, "living_idle_sound", "Mob#getAmbientSound", "audio", "automatic_rename_candidate", "Map ambient sound timing and silence conditions.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "playStepSound", V_ALL, "living_step_sound", "Entity#playStepSound", "audio", "signature_adapter_required", "Map BlockPos/BlockState and source volume/pitch.", "net/minecraft/entity/EntityLivingBase.html"),

    // World and level operations; these are source method names commonly called by mods.
    d!("World", "setBlock", V_ALL, "set_block_state", "Level#setBlock", "world", "signature_adapter_required", "Translate integer coordinates and metadata/flags to BlockPos, BlockState and update flags.", "net/minecraft/world/World.html"),
    d!("World", "setBlockToAir", V_ALL, "remove_block", "Level#removeBlock", "world", "signature_adapter_required", "Preserve block-entity removal and neighbor update flags.", "net/minecraft/world/World.html"),
    d!("World", "getBlock", V_ALL, "get_block", "Level#getBlockState", "world", "signature_adapter_required", "The target returns BlockState rather than the old Block singleton.", "net/minecraft/world/World.html"),
    d!("World", "getBlockMetadata", V_ALL, "get_block_metadata", "BlockState property access", "world", "adapter_required", "Translate metadata to explicit state properties; do not preserve numeric metadata as an identity.", "net/minecraft/world/World.html"),
    d!("World", "getBlockState", V_18, "get_block_state", "Level#getBlockState", "world", "automatic_rename_candidate", "Check position type and unloaded-chunk behavior.", "net/minecraft/world/World.html"),
    d!("World", "isAirBlock", V_ALL, "air_state_query", "Level#isEmptyBlock / BlockState#isAir", "world", "automatic_rename_candidate", "Preserve unloaded-position and fluid behavior.", "net/minecraft/world/World.html"),
    d!("World", "getTileEntity", V_ALL, "block_entity_lookup", "Level#getBlockEntity", "block_entity", "automatic_rename_candidate", "Translate coordinates into BlockPos and handle absent/unloaded block entities.", "net/minecraft/world/World.html"),
    d!("World", "setTileEntity", V_ALL, "block_entity_attach", "Level#setBlockEntity", "block_entity", "adapter_required", "Preserve ownership, position, level attachment and chunk lifecycle.", "net/minecraft/world/World.html"),
    d!("World", "removeTileEntity", V_ALL, "block_entity_remove", "Level#removeBlockEntity", "block_entity", "automatic_rename_candidate", "Ensure the block state and block entity are removed in the correct order.", "net/minecraft/world/World.html"),
    d!("World", "spawnEntityInWorld", V_ALL, "spawn_entity", "Level#addFreshEntity", "world", "automatic_rename_candidate", "Respect logical side and spawn rejection result.", "net/minecraft/world/World.html"),
    d!("World", "playSoundAtEntity", V_ALL, "entity_sound", "Level#playSound", "audio", "signature_adapter_required", "Translate entity source, sound category, volume, pitch and recipient semantics.", "net/minecraft/world/World.html"),
    d!("World", "playSoundEffect", V_ALL, "position_sound", "Level#playSound", "audio", "signature_adapter_required", "Translate coordinates and old sound-name strings to SoundEvent and SoundSource.", "net/minecraft/world/World.html"),
    d!("World", "createExplosion", V_ALL, "world_explosion", "Level#explode", "world", "signature_adapter_required", "Preserve source entity, fire flag, block interaction and explosion radius semantics.", "net/minecraft/world/World.html"),
    d!("World", "getEntitiesWithinAABB", V_ALL, "entity_aabb_query", "Level#getEntitiesOfClass / EntityGetter#getEntities", "world", "signature_adapter_required", "Translate AxisAlignedBB and class predicates to AABB and target predicates.", "net/minecraft/world/World.html"),
    d!("World", "getClosestPlayer", V_ALL, "nearest_player_query", "Level#getNearestPlayer", "world", "signature_adapter_required", "Preserve range, spectator and predicate filtering.", "net/minecraft/world/World.html"),
    d!("World", "getTotalWorldTime", V_ALL, "total_world_time", "Level#getGameTime", "world", "automatic_rename_candidate", "Do not confuse total game time with day/night time.", "net/minecraft/world/World.html"),
    d!("World", "getWorldTime", V_ALL, "day_time", "Level#getDayTime", "world", "automatic_rename_candidate", "Preserve daylight-cycle and dimension-specific time behavior.", "net/minecraft/world/World.html"),
    d!("World", "isBlockPowered", V_ALL, "redstone_power_query", "Level#hasNeighborSignal", "redstone", "automatic_rename_candidate", "Map direct/indirect signal checks exactly as used by the source.", "net/minecraft/world/World.html"),
    d!("World", "getStrongPower", V_ALL, "strong_redstone_power", "Level#getSignal", "redstone", "signature_adapter_required", "Translate direction and position; distinguish direct signal from neighbor signal.", "net/minecraft/world/World.html"),
    d!("World", "scheduleBlockUpdate", V_ALL, "schedule_block_tick", "Level#scheduleTick", "block_updates", "signature_adapter_required", "Translate delay and priority; preserve duplicate scheduling rules.", "net/minecraft/world/World.html"),
    d!("World", "markBlockForUpdate", V_ALL, "block_client_sync", "Level#sendBlockUpdated", "networking", "signature_adapter_required", "Translate update flags and ensure server-side packet propagation.", "net/minecraft/world/World.html"),
    d!("World", "notifyBlocksOfNeighborChange", V_ALL, "neighbor_notification", "Level#updateNeighborsAt", "block_updates", "signature_adapter_required", "Preserve notification direction and recursion behavior.", "net/minecraft/world/World.html"),
    d!("World", "getChunkFromBlockCoords", V_ALL, "chunk_lookup", "Level#getChunkAt", "world", "adapter_required", "Avoid force-loading chunks when the source only queried loaded chunks.", "net/minecraft/world/World.html"),
    d!("World", "getBiomeGenForCoords", V_ALL, "biome_query", "Level#getBiome", "worldgen", "signature_adapter_required", "Translate coordinates to BlockPos and preserve biome holder/key resolution.", "net/minecraft/world/World.html"),
    d!("World", "canBlockSeeTheSky", V_ALL, "sky_visibility", "Level#canSeeSky", "world", "automatic_rename_candidate", "Translate coordinates to BlockPos.", "net/minecraft/world/World.html"),
    d!("World", "isDaytime", V_ALL, "daylight_query", "Level#isDay", "world", "automatic_rename_candidate", "Preserve dimension time and weather considerations.", "net/minecraft/world/World.html"),

    // Tile entities / block entities.
    d!("TileEntity", "readFromNBT", V_ALL, "block_entity_load", "BlockEntity#loadAdditional", "block_entity", "adapter_required", "Translate NBTTagCompound and preserve base metadata and custom data versioning.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "writeToNBT", V_ALL, "block_entity_save", "BlockEntity#saveAdditional", "block_entity", "adapter_required", "Only write custom fields in saveAdditional; preserve base serialization.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "updateEntity", V_ALL, "block_entity_tick", "BlockEntityTicker / BlockEntity# tick", "block_entity", "adapter_required", "Register a ticker from the block entity type and separate client/server tick logic.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "getDescriptionPacket", V_ALL, "block_entity_update_packet", "BlockEntity#getUpdatePacket", "networking", "adapter_required", "Use ClientboundBlockEntityDataPacket or an explicit custom payload.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "onDataPacket", V_ALL, "block_entity_packet_receive", "BlockEntity#onDataPacket", "networking", "adapter_required", "Translate packet NBT and update client render state safely.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "markDirty", V_ALL, "block_entity_mark_changed", "BlockEntity#setChanged", "block_entity", "automatic_rename_candidate", "Preserve chunk dirty marking and client update packet requirements.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "invalidate", V_ALL, "block_entity_invalidate", "BlockEntity#setRemoved", "block_entity", "automatic_rename_candidate", "Preserve capability invalidation and removal lifecycle.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "validate", V_ALL, "block_entity_validate", "BlockEntity#onLoad / lifecycle attachment", "block_entity", "adapter_required", "Modern lifecycle timing is tied to chunk load and level attachment.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "getBlockType", V_ALL, "block_entity_block", "BlockEntity#getBlockState", "block_entity", "automatic_rename_candidate", "The modern state is more useful than a global Block singleton.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "getBlockMetadata", V_ALL, "block_entity_metadata", "BlockEntity#getBlockState properties", "block_entity", "adapter_required", "Translate metadata checks to state-property comparisons.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "getWorldObj", V_ALL, "block_entity_level", "BlockEntity#getLevel", "block_entity", "automatic_rename_candidate", "Handle a null level before attachment or after removal.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "getPos", V_18, "block_entity_position", "BlockEntity#getBlockPos", "block_entity", "automatic_rename_candidate", "The 1.7.10 coordinate fields need to be reconstructed into BlockPos.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "shouldRefresh", V_18, "block_entity_refresh_predicate", "BlockEntity# setRemoved / state replacement checks", "block_entity", "adapter_required", "Preserve old/new block-state comparison to avoid unnecessary block-entity recreation.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "getMaxRenderDistanceSquared", V_ALL, "block_entity_render_distance", "BlockEntityRenderer#getViewDistance", "rendering", "client_adapter_required", "Move the value into the target block-entity renderer.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "hasFastRenderer", V_18, "block_entity_fast_renderer", "BlockEntityRenderer / model data", "rendering", "client_adapter_required", "Replace the legacy fast-renderer flag with a modern renderer implementation.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "getUpdateTag", V_112, "block_entity_update_tag", "BlockEntity#getUpdateTag", "networking", "adapter_required", "Separate update tags from disk save data and preserve only client-required fields.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "handleUpdateTag", V_112, "block_entity_update_tag_apply", "BlockEntity#handleUpdateTag", "networking", "adapter_required", "Translate network tag handling and avoid overwriting server-only data.", "net/minecraft/tileentity/TileEntity.html"),

    // Stack and tag APIs: ubiquitous call-site methods in item/entity logic.
    d!("ItemStack", "getItem", V_ALL, "stack_item_identity", "ItemStack#getItem", "item_data", "automatic_rename_candidate", "Preserve empty-stack behavior and null handling.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getStackSize", V_17_18, "stack_count", "ItemStack#getCount", "item_data", "automatic_rename_candidate", "Translate the mutable stackSize field to the modern count API.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getCount", V_112, "stack_count", "ItemStack#getCount", "item_data", "automatic_rename_candidate", "Preserve empty-stack semantics and count clamping.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "setCount", V_112, "stack_count_write", "ItemStack#setCount", "item_data", "automatic_rename_candidate", "Preserve zero-count normalization to an empty stack.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getItemDamage", V_ALL, "stack_damage_read", "ItemStack#getDamageValue", "item_data", "automatic_rename_candidate", "Translate damage value separately from maximum durability.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "setItemDamage", V_ALL, "stack_damage_write", "ItemStack#setDamageValue", "item_data", "automatic_rename_candidate", "Preserve clamping and breakage behavior.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getMaxDamage", V_ALL, "stack_max_damage", "ItemStack#getMaxDamage", "item_data", "automatic_rename_candidate", "Stack components can override the item default; preserve per-stack values.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "isItemStackDamageable", V_ALL, "stack_damageable_predicate", "ItemStack#isDamageableItem", "item_data", "automatic_rename_candidate", "Do not infer damageability from positive max damage alone when item components override it.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "isStackable", V_ALL, "stackable_predicate", "ItemStack#isStackable", "item_data", "automatic_rename_candidate", "Preserve per-stack max-count components and damageability checks.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "isItemEnchantable", V_ALL, "stack_enchantable_predicate", "ItemStack#isEnchantable", "enchanting", "adapter_required", "Translate item enchantability and existing enchantment state.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "isItemEnchanted", V_ALL, "stack_has_enchantments", "ItemStack#isEnchanted", "enchanting", "automatic_rename_candidate", "Preserve stored and active enchantment distinctions.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getEnchantmentTagList", V_ALL, "stack_enchantments_read", "EnchantmentHelper / DataComponents#ENCHANTMENTS", "enchanting", "component_adapter_required", "Translate legacy NBT enchantment lists to modern enchantment components.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "addEnchantment", V_ALL, "stack_add_enchantment", "ItemEnchantments component mutation", "enchanting", "component_adapter_required", "Validate enchantment compatibility and update the stack component.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "copy", V_ALL, "stack_copy", "ItemStack#copy", "item_data", "automatic_rename_candidate", "Preserve component/tag deep-copy behavior.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "splitStack", V_ALL, "stack_split", "ItemStack#split", "item_data", "automatic_rename_candidate", "Translate count splitting and preserve empty-stack invariants.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "writeToNBT", V_ALL, "stack_serialize", "ItemStack#save", "persistence", "adapter_required", "Translate item registry lookup and preserve component/legacy tag migration.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "readFromNBT", V_ALL, "stack_deserialize", "ItemStack#parse / ItemStack#of", "persistence", "adapter_required", "Use registry-aware parsing and handle invalid item IDs safely.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getTagCompound", V_ALL, "stack_custom_data_read", "DataComponents#CUSTOM_DATA / CustomData", "item_data", "component_adapter_required", "Replace mutable root NBT access with explicit custom-data component operations.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "setTagCompound", V_ALL, "stack_custom_data_write", "DataComponents#CUSTOM_DATA / CustomData", "item_data", "component_adapter_required", "Migrate custom data without overwriting unrelated item components.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "hasTagCompound", V_ALL, "stack_has_custom_data", "DataComponents#CUSTOM_DATA", "item_data", "component_adapter_required", "Check the custom-data component while retaining empty-compound behavior.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getDisplayName", V_ALL, "stack_hover_name", "ItemStack#getHoverName", "localization", "adapter_required", "Convert legacy String names to Component and preserve custom-name precedence.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "setStackDisplayName", V_ALL, "stack_custom_name_write", "DataComponents#CUSTOM_NAME / ItemStack#setHoverName", "localization", "component_adapter_required", "Represent custom stack names as Components and preserve localization.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "hasDisplayName", V_ALL, "stack_has_custom_name", "ItemStack#hasCustomHoverName", "localization", "automatic_rename_candidate", "Preserve distinction between custom and default translated names.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "areItemsEqual", V_ALL, "stack_item_equality", "ItemStack#isSameItem", "item_data", "automatic_rename_candidate", "Compare item identity without accidentally including count or components.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "areItemStackTagsEqual", V_ALL, "stack_tag_equality", "ItemStack#matches / component comparison", "item_data", "adapter_required", "Decide which modern components participate; legacy tag equality may be narrower than full stack equality.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "areItemStacksEqual", V_ALL, "stack_full_equality", "ItemStack#matches", "item_data", "automatic_rename_candidate", "Preserve identity, count, damage and component equality semantics.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getMetadata", V_ALL, "stack_metadata", "ItemStack components / ItemOverrides", "item_data", "adapter_required", "Convert metadata variants into state/components or distinct registry entries.", "net/minecraft/item/ItemStack.html"),
    d!("ItemStack", "getUnlocalizedName", V_ALL, "stack_translation_key", "ItemStack#getDescriptionId / Item#getDescriptionId", "localization", "adapter_required", "Preserve metadata-specific translation key selection.", "net/minecraft/item/ItemStack.html"),

    // NBT APIs used in custom item/entity/block-entity state.
    d!("NBTTagCompound", "setString", V_ALL, "nbt_string_write", "CompoundTag#putString", "persistence", "automatic_rename_candidate", "Translate key and String values while preserving default behavior.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getString", V_ALL, "nbt_string_read", "CompoundTag#getString", "persistence", "automatic_rename_candidate", "Preserve missing-key default values.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setInteger", V_ALL, "nbt_int_write", "CompoundTag#putInt", "persistence", "automatic_rename_candidate", "Translate numeric type and key semantics.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getInteger", V_ALL, "nbt_int_read", "CompoundTag#getInt", "persistence", "automatic_rename_candidate", "Preserve absent-key zero semantics.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setBoolean", V_ALL, "nbt_boolean_write", "CompoundTag#putBoolean", "persistence", "automatic_rename_candidate", "Preserve boolean encoding and key names.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getBoolean", V_ALL, "nbt_boolean_read", "CompoundTag#getBoolean", "persistence", "automatic_rename_candidate", "Preserve absent-key false semantics.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setByte", V_ALL, "nbt_byte_write", "CompoundTag#putByte", "persistence", "automatic_rename_candidate", "Preserve signed byte values.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getByte", V_ALL, "nbt_byte_read", "CompoundTag#getByte", "persistence", "automatic_rename_candidate", "Preserve numeric coercion semantics.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setShort", V_ALL, "nbt_short_write", "CompoundTag#putShort", "persistence", "automatic_rename_candidate", "Preserve signed short values.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getShort", V_ALL, "nbt_short_read", "CompoundTag#getShort", "persistence", "automatic_rename_candidate", "Preserve numeric coercion semantics.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setLong", V_ALL, "nbt_long_write", "CompoundTag#putLong", "persistence", "automatic_rename_candidate", "Preserve 64-bit values.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getLong", V_ALL, "nbt_long_read", "CompoundTag#getLong", "persistence", "automatic_rename_candidate", "Preserve missing-key default and 64-bit range.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setFloat", V_ALL, "nbt_float_write", "CompoundTag#putFloat", "persistence", "automatic_rename_candidate", "Preserve float precision.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getFloat", V_ALL, "nbt_float_read", "CompoundTag#getFloat", "persistence", "automatic_rename_candidate", "Preserve float precision and missing-key default.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setDouble", V_ALL, "nbt_double_write", "CompoundTag#putDouble", "persistence", "automatic_rename_candidate", "Preserve double precision.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getDouble", V_ALL, "nbt_double_read", "CompoundTag#getDouble", "persistence", "automatic_rename_candidate", "Preserve double precision and missing-key default.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setTag", V_ALL, "nbt_nested_tag_write", "CompoundTag#put", "persistence", "signature_adapter_required", "Use the correct modern Tag subtype and ownership/copy behavior.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getCompoundTag", V_ALL, "nbt_nested_compound_read", "CompoundTag#getCompound", "persistence", "automatic_rename_candidate", "Preserve missing or wrong-type tag behavior.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getTagList", V_ALL, "nbt_list_read", "CompoundTag#getList", "persistence", "signature_adapter_required", "Translate the expected element type ID and guard malformed legacy data.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "hasKey", V_ALL, "nbt_key_exists", "CompoundTag#contains", "persistence", "signature_adapter_required", "Translate the optional NBT type constraint to the typed contains overload.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "removeTag", V_ALL, "nbt_key_remove", "CompoundTag#remove", "persistence", "automatic_rename_candidate", "Preserve key removal semantics.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "hasNoTags", V_ALL, "nbt_empty_predicate", "CompoundTag#isEmpty", "persistence", "automatic_rename_candidate", "Preserve empty-compound behavior.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getKeySet", V_ALL, "nbt_key_set", "CompoundTag#getAllKeys", "persistence", "automatic_rename_candidate", "Translate Set<String> and preserve iteration assumptions where relevant.", "net/minecraft/nbt/NBTTagCompound.html"),

    // AI goals and navigation.
    d!("EntityAIBase", "shouldExecute", V_ALL, "ai_goal_can_start", "Goal#canUse", "entity_ai", "automatic_rename_candidate", "Preserve random gating, target state and mutex conflict behavior.", "net/minecraft/entity/ai/EntityAIBase.html"),
    d!("EntityAIBase", "shouldContinueExecuting", V_ALL, "ai_goal_should_continue", "Goal#canContinueToUse", "entity_ai", "automatic_rename_candidate", "Preserve continuation checks separately from initial eligibility.", "net/minecraft/entity/ai/EntityAIBase.html"),
    d!("EntityAIBase", "startExecuting", V_ALL, "ai_goal_start", "Goal#start", "entity_ai", "automatic_rename_candidate", "Preserve one-time start side effects.", "net/minecraft/entity/ai/EntityAIBase.html"),
    d!("EntityAIBase", "resetTask", V_ALL, "ai_goal_stop", "Goal#stop", "entity_ai", "automatic_rename_candidate", "Preserve cleanup and navigation cancellation.", "net/minecraft/entity/ai/EntityAIBase.html"),
    d!("EntityAIBase", "updateTask", V_ALL, "ai_goal_tick", "Goal#tick", "entity_ai", "automatic_rename_candidate", "Preserve tick cadence and selector ordering.", "net/minecraft/entity/ai/EntityAIBase.html"),
    d!("EntityAIBase", "setMutexBits", V_ALL, "ai_goal_control_flags", "Goal#setFlags", "entity_ai", "adapter_required", "Translate mutex bit masks into modern Goal.Flag sets.", "net/minecraft/entity/ai/EntityAIBase.html"),
    d!("EntityAIBase", "getMutexBits", V_ALL, "ai_goal_control_flags_read", "Goal#getFlags", "entity_ai", "adapter_required", "Translate Goal.Flag sets back to a bitmask only if source logic consumes it.", "net/minecraft/entity/ai/EntityAIBase.html"),
    d!("PathNavigate", "tryMoveToXYZ", V_ALL, "navigate_to_position", "PathNavigation#moveTo", "entity_ai", "signature_adapter_required", "Create a target BlockPos/vector and preserve speed and path failure handling.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "tryMoveToEntityLiving", V_ALL, "navigate_to_entity", "PathNavigation#moveTo", "entity_ai", "signature_adapter_required", "Preserve live target tracking rather than freezing the initial coordinates.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "noPath", V_ALL, "navigation_has_no_path", "PathNavigation#isDone", "entity_ai", "automatic_rename_candidate", "Preserve meaning while path recalculation is pending.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "clearPathEntity", V_ALL, "navigation_stop", "PathNavigation#stop", "entity_ai", "automatic_rename_candidate", "Preserve navigation cancellation and move-control state.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "onUpdateNavigation", V_ALL, "navigation_tick", "PathNavigation#tick", "entity_ai", "adapter_required", "Prefer registered navigation tick lifecycle over manually ticking twice.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "setPath", V_ALL, "navigation_set_path", "PathNavigation#moveTo(Path, double)", "entity_ai", "signature_adapter_required", "Translate Path representation and speed units.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "getPath", V_ALL, "navigation_path_read", "PathNavigation#getPath", "entity_ai", "automatic_rename_candidate", "Check null/done handling and modern path ownership.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "setCanSwim", V_ALL, "navigation_swim_flag", "PathNavigation#setCanFloat", "entity_ai", "adapter_required", "Modern float navigation is not identical to every legacy swimming behavior.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "setEnterDoors", V_ALL, "navigation_door_flag", "GroundPathNavigation#setCanOpenDoors", "entity_ai", "adapter_required", "Preserve door opening and breaking distinctions.", "net/minecraft/pathfinding/PathNavigate.html"),
    d!("PathNavigate", "setBreakDoors", V_ALL, "navigation_break_doors_flag", "GroundPathNavigation#setCanBreakDoors", "entity_ai", "adapter_required", "Preserve door-breaking capability separately from opening.", "net/minecraft/pathfinding/PathNavigate.html"),

    // Player food, potion state, inventory and menus.
    d!("FoodStats", "addStats", V_ALL, "player_food_restore", "FoodData#eat", "player", "signature_adapter_required", "Translate nutrition and saturation modifier semantics.", "net/minecraft/util/FoodStats.html"),
    d!("FoodStats", "getFoodLevel", V_ALL, "player_food_level", "FoodData#getFoodLevel", "player", "automatic_rename_candidate", "Preserve integer hunger level.", "net/minecraft/util/FoodStats.html"),
    d!("FoodStats", "getSaturationLevel", V_ALL, "player_food_saturation", "FoodData#getSaturationLevel", "player", "automatic_rename_candidate", "Preserve float saturation independently from exhaustion.", "net/minecraft/util/FoodStats.html"),
    d!("FoodStats", "needFood", V_ALL, "player_needs_food", "FoodData#needsFood", "player", "automatic_rename_candidate", "Preserve saturation/food-level full-state semantics.", "net/minecraft/util/FoodStats.html"),
    d!("PotionEffect", "getPotionID", V_ALL, "potion_effect_type", "MobEffectInstance#getEffect", "effects", "adapter_required", "Resolve numeric potion IDs to registry-backed MobEffect holders.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "getDuration", V_ALL, "potion_effect_duration", "MobEffectInstance#getDuration", "effects", "automatic_rename_candidate", "Preserve tick units and infinite-duration handling.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "getAmplifier", V_ALL, "potion_effect_amplifier", "MobEffectInstance#getAmplifier", "effects", "automatic_rename_candidate", "Preserve zero-based amplifier semantics.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "onUpdate", V_ALL, "potion_effect_tick", "MobEffectInstance#tick", "effects", "adapter_required", "Modern effect ticking is owned by LivingEntity; do not manually double-tick.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "combine", V_ALL, "potion_effect_merge", "MobEffectInstance#update", "effects", "adapter_required", "Preserve duration/amplifier precedence and ambient/particle flags.", "net/minecraft/potion/PotionEffect.html"),
    d!("EntityPlayer", "addExperience", V_ALL, "player_experience_add", "Player#giveExperiencePoints", "player", "automatic_rename_candidate", "Preserve experience-orb versus raw point behavior.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "addExperienceLevel", V_ALL, "player_experience_levels_add", "Player#giveExperienceLevels", "player", "automatic_rename_candidate", "Preserve negative-level and advancement side effects.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "canEat", V_ALL, "player_can_eat", "Player#canEat", "player", "signature_adapter_required", "Translate the ignore-hunger flag and modern food state.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getFoodStats", V_ALL, "player_food_data", "Player#getFoodData", "player", "automatic_rename_candidate", "Return the target FoodData object and adapt any direct field access.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "openGui", V_ALL, "open_mod_gui", "NetworkHooks#openScreen / MenuProvider", "menus", "manual_adapter_required", "Replace integer GUI IDs with registered MenuTypes and server-authoritative screen opening.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("InventoryPlayer", "getCurrentItem", V_ALL, "player_selected_stack", "Player#getMainHandItem", "inventory", "automatic_rename_candidate", "Preserve selected hotbar slot and main-hand semantics.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("InventoryPlayer", "getCurrentItemStack", V_17, "player_selected_stack", "Player#getMainHandItem", "inventory", "automatic_rename_candidate", "Translate the 1.7.10 selected hotbar stack field/accessor.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("InventoryPlayer", "addItemStackToInventory", V_ALL, "player_inventory_insert", "Inventory#add / Player inventory insertion", "inventory", "adapter_required", "Preserve partial insertion, overflow drops and return semantics.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("InventoryPlayer", "consumeInventoryItem", V_ALL, "player_inventory_consume", "Inventory#removeItem", "inventory", "adapter_required", "Preserve first-match selection and stack splitting.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("Container", "canInteractWith", V_ALL, "menu_still_valid", "AbstractContainerMenu#stillValid", "menus", "adapter_required", "Preserve player distance, block identity and ownership checks.", "net/minecraft/inventory/Container.html"),
    d!("Container", "transferStackInSlot", V_ALL, "menu_shift_click", "AbstractContainerMenu#quickMoveStack", "menus", "manual_adapter_required", "Reconstruct slot ranges and merge directions; never blindly copy indices.", "net/minecraft/inventory/Container.html"),
    d!("Container", "slotClick", V_ALL, "menu_slot_click", "AbstractContainerMenu#clicked", "menus", "manual_adapter_required", "Modern menu click handling is centralized; preserve click type and carried-stack state.", "net/minecraft/inventory/Container.html"),
    d!("Container", "detectAndSendChanges", V_ALL, "menu_sync_changes", "AbstractContainerMenu#broadcastChanges", "menus", "adapter_required", "Translate listener synchronization to menu data slots and carried stack updates.", "net/minecraft/inventory/Container.html"),
    d!("Container", "onContainerClosed", V_ALL, "menu_close", "AbstractContainerMenu#removed", "menus", "adapter_required", "Preserve item return/drop behavior when the player closes the menu.", "net/minecraft/inventory/Container.html"),

    // Forge registries, ore dictionary and legacy networking call sites.
    d!("GameRegistry", "registerItem", V_ALL, "register_item", "DeferredRegister<Item> / RegisterEvent", "registration", "registration_adapter_required", "Translate registry names and constructor timing into deferred target registration.", ""),
    d!("GameRegistry", "registerBlock", V_ALL, "register_block", "DeferredRegister<Block> / RegisterEvent", "registration", "registration_adapter_required", "Recover block item registration separately and preserve registry names.", ""),
    d!("GameRegistry", "registerTileEntity", V_ALL, "register_block_entity_type", "DeferredRegister<BlockEntityType<?>>", "registration", "registration_adapter_required", "Resolve associated blocks and block-entity factory before registration.", ""),
    d!("GameRegistry", "registerEntity", V_ALL, "register_entity_type", "DeferredRegister<EntityType<?>>", "registration", "registration_adapter_required", "Translate legacy numeric IDs, tracking range, update frequency and factory.", ""),
    d!("GameRegistry", "registerWorldGenerator", V_ALL, "register_worldgen", "BiomeModifier / Feature registration", "worldgen", "registration_adapter_required", "Reconstruct generation stage, biome filters and configured feature data.", ""),
    d!("GameRegistry", "registerFuelHandler", V_ALL, "register_fuel", "Fuel item extension / recipe integration", "fuel", "registration_adapter_required", "Move the fuel predicate and burn-time logic into a target hook or data-driven fuel definition.", ""),
    d!("GameRegistry", "register", V_112, "registry_object_register", "DeferredRegister / RegisterEvent", "registration", "registration_adapter_required", "Map the registry instance and supplier lifecycle, not just the registry name.", ""),
    d!("OreDictionary", "registerOre", V_ALL, "ore_tag_membership", "TagKey / data pack tags", "tags", "registration_adapter_required", "Convert OreDictionary names to namespaced tags and preserve cross-mod interoperability.", ""),
    d!("OreDictionary", "getOre", V_ALL, "ore_tag_query", "HolderLookup / tag membership query", "tags", "adapter_required", "Resolve tag contents at runtime rather than storing a frozen ItemStack list.", ""),
    d!("OreDictionary", "getOreID", V_ALL, "ore_name_id", "ResourceLocation / TagKey", "tags", "adapter_required", "Replace process-local integer IDs with stable namespaced tag identifiers.", ""),
    d!("OreDictionary", "getOreName", V_ALL, "ore_id_name", "ResourceLocation / TagKey", "tags", "adapter_required", "Recover the original tag string and namespace conversion rules.", ""),
    d!("OreDictionary", "getOreNames", V_ALL, "ore_registered_names", "Data pack tag enumeration", "tags", "adapter_required", "Dynamic data-pack tags cannot be enumerated at static initialization time.", ""),
    d!("SimpleNetworkWrapper", "registerMessage", V_18, "register_packet_handler", "RegisterPayloadHandler / CustomPacketPayload", "networking", "manual_adapter_required", "Define typed payload codecs, direction, handler context and version-safe registration.", ""),
    d!("SimpleNetworkWrapper", "sendTo", V_18, "send_packet_to_player", "PacketDistributor.PLAYER", "networking", "adapter_required", "Translate IMessage to a typed payload and keep server-thread scheduling.", ""),
    d!("SimpleNetworkWrapper", "sendToAll", V_18, "broadcast_packet", "PacketDistributor.ALL", "networking", "adapter_required", "Preserve dimension/server filtering and payload side.", ""),
    d!("SimpleNetworkWrapper", "sendToAllAround", V_18, "send_packet_near_position", "PacketDistributor.NEAR", "networking", "adapter_required", "Translate TargetPoint dimension, position and radius into the modern distributor target.", ""),
    d!("SimpleNetworkWrapper", "sendToServer", V_18, "send_packet_to_server", "PacketDistributor.SERVER", "networking", "adapter_required", "Keep client-thread safety and server-side validation.", ""),

    // Forge extension interfaces often found in mod source.
    d!("IInventory", "getSizeInventory", V_ALL, "inventory_size", "Container / ItemHandler# getSlots", "inventory", "adapter_required", "Translate inventory indices and choose between menu Container and capability ItemHandler.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "getStackInSlot", V_ALL, "inventory_read_slot", "Container#getItem / IItemHandler#getStackInSlot", "inventory", "adapter_required", "Preserve empty-stack semantics and slot bounds.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "decrStackSize", V_ALL, "inventory_split_stack", "Container#removeItem / IItemHandler#extractItem", "inventory", "adapter_required", "Preserve amount limits, simulation and remainder semantics.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "getStackInSlotOnClosing", V_ALL, "inventory_remove_on_close", "Container#removeItemNoUpdate", "inventory", "adapter_required", "Preserve whether closing removes the full stack or only returns it.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "setInventorySlotContents", V_ALL, "inventory_write_slot", "Container#setItem / IItemHandler#setStackInSlot", "inventory", "adapter_required", "Preserve slot validation and stack-size clamping.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "getInventoryStackLimit", V_ALL, "inventory_stack_limit", "Container#getMaxStackSize", "inventory", "automatic_rename_candidate", "Translate per-inventory stack limits; per-slot limits may need Slot#getMaxStackSize.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "markDirty", V_ALL, "inventory_changed", "Container#setChanged / BlockEntity#setChanged", "inventory", "adapter_required", "Connect menu changes to backing block-entity dirty marking.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "isUseableByPlayer", V_ALL, "inventory_usable_by_player", "AbstractContainerMenu#stillValid", "inventory", "adapter_required", "Preserve distance, ownership, block identity and spectator checks.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "openInventory", V_ALL, "inventory_open", "AbstractContainerMenu#startOpen", "inventory", "adapter_required", "Move viewer-count changes into menu lifecycle.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "closeInventory", V_ALL, "inventory_close", "AbstractContainerMenu#stopOpen", "inventory", "adapter_required", "Move viewer-count and lock cleanup into menu lifecycle.", "net/minecraft/inventory/IInventory.html"),
    d!("IInventory", "isItemValidForSlot", V_ALL, "inventory_slot_validation", "Slot#mayPlace / IItemHandler#isItemValid", "inventory", "adapter_required", "Preserve slot-specific filters and sided inventory behavior.", "net/minecraft/inventory/IInventory.html"),
    d!("ISidedInventory", "getSlotsForFace", V_ALL, "sided_inventory_slots", "IItemHandler capability / sided slot rules", "inventory", "adapter_required", "Translate facing-specific exposed slot indices.", "net/minecraft/inventory/ISidedInventory.html"),
    d!("ISidedInventory", "canInsertItem", V_ALL, "sided_inventory_insert", "IItemHandler#isItemValid / sided capability", "inventory", "adapter_required", "Preserve face- and slot-specific insertion restrictions.", "net/minecraft/inventory/ISidedInventory.html"),
    d!("ISidedInventory", "canExtractItem", V_ALL, "sided_inventory_extract", "IItemHandler#extractItem / sided capability", "inventory", "adapter_required", "Preserve face-, slot- and item-dependent extraction restrictions.", "net/minecraft/inventory/ISidedInventory.html"),
    d!("IRecipe", "matches", V_ALL, "recipe_match", "Recipe#matches", "recipes", "adapter_required", "Move matching logic into modern Recipe with a Container/RecipeInput and Level context.", "net/minecraft/item/crafting/IRecipe.html"),
    d!("IRecipe", "getCraftingResult", V_ALL, "recipe_result", "Recipe#assemble", "recipes", "adapter_required", "Translate inventory input and return a copied output stack.", "net/minecraft/item/crafting/IRecipe.html"),
    d!("IRecipe", "getRecipeSize", V_ALL, "recipe_input_size", "Recipe#getIngredients / Recipe#canCraftInDimensions", "recipes", "adapter_required", "Preserve shaped/shapeless dimension and ingredient constraints.", "net/minecraft/item/crafting/IRecipe.html"),
    d!("IRecipe", "getRecipeOutput", V_ALL, "recipe_preview_output", "Recipe#getResultItem", "recipes", "adapter_required", "Provide a registry-aware result stack and preserve dynamic output only when supported.", "net/minecraft/item/crafting/IRecipe.html"),
    d!("IWorldGenerator", "generate", V_ALL, "world_generation", "BiomeModifier / Feature / ConfiguredFeature", "worldgen", "manual_adapter_required", "Reconstruct generation stage, biome filters, random source, chunk coordinates and dimension restrictions.", ""),
    d!("IFuelHandler", "getBurnTime", V_ALL, "fuel_burn_time", "Fuel recipe / item fuel extension", "fuel", "adapter_required", "Translate stack-sensitive burn time into target fuel data or an extension hook.", ""),
    d!("IEntityAdditionalSpawnData", "writeSpawnData", V_ALL, "entity_spawn_payload_write", "Entity# addAdditionalSaveData / custom spawn payload", "networking", "adapter_required", "Distinguish persistent NBT from initial spawn synchronization.", ""),
    d!("IEntityAdditionalSpawnData", "readSpawnData", V_ALL, "entity_spawn_payload_read", "Entity# readAdditionalSaveData / custom spawn payload", "networking", "adapter_required", "Read exactly the payload written during spawn and preserve ordering.", ""),
    d!("IExtendedEntityProperties", "init", V_ALL, "entity_extension_init", "Entity attachment / capability registration", "capabilities", "adapter_required", "Convert per-entity properties into a registered attachment/capability with explicit lifecycle.", ""),
    d!("IExtendedEntityProperties", "saveNBTData", V_ALL, "entity_extension_save", "Entity attachment serialization / codec", "persistence", "adapter_required", "Persist attachment data through the modern serialization lifecycle.", ""),
    d!("IExtendedEntityProperties", "loadNBTData", V_ALL, "entity_extension_load", "Entity attachment serialization / codec", "persistence", "adapter_required", "Restore attachment state and handle absent legacy fields.", ""),
    d!("IMessage", "toBytes", V_ALL, "network_message_encode", "CustomPacketPayload / StreamCodec", "networking", "adapter_required", "Translate ByteBuf serialization into typed payload StreamCodec; preserve field order and bounds.", ""),
    d!("IMessage", "fromBytes", V_ALL, "network_message_decode", "CustomPacketPayload / StreamCodec", "networking", "adapter_required", "Replace mutable decode callbacks with a typed StreamCodec and validation.", ""),
    d!("IMessageHandler", "onMessage", V_ALL, "network_message_handle", "Payload handler registration / NetworkEvent.Context", "networking", "adapter_required", "Preserve thread enqueuing, direction checks, sender validation and side effects.", ""),
    d!("ITickHandler", "tickStart", V_17_18, "legacy_tick_start", "NeoForge tick events", "events", "event_adapter_required", "Translate tick phase and side into the correct server/client tick event.", ""),
    d!("ITickHandler", "tickEnd", V_17_18, "legacy_tick_end", "NeoForge tick events", "events", "event_adapter_required", "Preserve END phase ordering and avoid duplicate per-level/entity ticks.", ""),
    d!("IPlayerTracker", "onPlayerLogin", V_ALL, "player_login", "PlayerLoggedInEvent", "events", "event_adapter_required", "Move to the login event and preserve server-only behavior.", ""),
    d!("IPlayerTracker", "onPlayerLogout", V_ALL, "player_logout", "PlayerLoggedOutEvent", "events", "event_adapter_required", "Move cleanup to logout and handle disconnects during world unload.", ""),
    d!("IPlayerTracker", "onPlayerChangedDimension", V_ALL, "player_dimension_change", "EntityTravelToDimensionEvent / player dimension-change events", "events", "event_adapter_required", "Preserve source and destination dimensions and avoid duplicate transition handling.", ""),
    d!("IPlayerTracker", "onPlayerRespawn", V_ALL, "player_respawn", "PlayerRespawnEvent", "events", "event_adapter_required", "Copy persistent player data at the correct respawn phase.", ""),
];

fn canonical_version(version: &str) -> Option<&'static str> {
    let value = version.trim();
    if value.starts_with("1.7.10") { Some("1.7.10") }
    else if value.starts_with("1.8.9") { Some("1.8.9") }
    else if value.starts_with("1.12.2") { Some("1.12.2") }
    else { None }
}

fn documentation_url(version: &str, path: &str) -> Option<String> {
    if path.is_empty() { return None; }
    let base = match version {
        "1.7.10" => "https://skmedix.github.io/ForgeJavaDocs/javadoc/forge/1.7.10-10.13.4.1614/",
        "1.8.9" => "https://skmedix.github.io/ForgeJavaDocs/javadoc/forge/1.8.9-11.15.1.2318/",
        "1.12.2" => "https://nekoyue.github.io/ForgeJavaDocs-NG/javadoc/1.12.2/",
        _ => return None,
    };
    Some(format!("{base}{path}"))
}

fn to_mapping(definition: &Definition) -> LegacyApiMethodMapping {
    let source_versions = definition.versions.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
    let documentation_urls = definition.versions.iter()
        .filter_map(|version| documentation_url(version, definition.docs))
        .collect();
    LegacyApiMethodMapping {
        source_owner: definition.owner.to_owned(),
        source_method: definition.method.to_owned(),
        source_versions,
        normalized_behavior: definition.behavior.to_owned(),
        target_api: definition.target.to_owned(),
        category: definition.category.to_owned(),
        conversion_status: definition.status.to_owned(),
        notes: definition.notes.to_owned(),
        documentation_urls,
    }
}

fn invocation_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"\b(?P<receiver>[A-Za-z_$][A-Za-z0-9_$]*)\s*\.\s*(?P<name>[A-Za-z_$][A-Za-z0-9_$]*)\s*\("
    ).expect("static legacy API invocation regex"))
}

fn typed_variable_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"\b(?P<owner>[A-Za-z_$][A-Za-z0-9_$.]*)\s+(?P<variable>[A-Za-z_$][A-Za-z0-9_$]*)\s*(?:[;=,)\[])"
    ).expect("static legacy API typed-variable regex"))
}

fn inheritance_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"\b(?:extends|implements)\s+(?P<owner>[A-Za-z_$][A-Za-z0-9_$.]*)"
    ).expect("static legacy API inheritance regex"))
}

fn owner_candidates(owner: &str) -> BTreeSet<String> {
    let short = owner.rsplit('.').next().unwrap_or(owner);
    let mut candidates = BTreeSet::new();
    candidates.insert(short.to_owned());
    match short {
        "ItemFood" | "ItemArmor" | "ItemTool" | "ItemSword" | "ItemBow" | "ItemHoe"
        | "ItemBlock" | "ItemShears" | "ItemPickaxe" | "ItemSpade" | "ItemAxe" => {
            candidates.insert("Item".to_owned());
        }
        "BlockContainer" | "BlockFalling" | "BlockBush" | "BlockDoor" | "BlockLiquid"
        | "BlockDynamicLiquid" | "BlockStaticLiquid" | "BlockCrops" | "BlockFlower"
        | "BlockOre" | "BlockRedstone" => {
            candidates.insert("Block".to_owned());
        }
        "EntityLivingBase" => {
            candidates.insert("Entity".to_owned());
        }
        "EntityLiving" | "EntityCreature" | "EntityAgeable" | "EntityAnimal"
        | "EntityMob" | "EntityPlayer" | "EntityPlayerMP" | "EntityVillager"
        | "EntityWaterMob" | "EntityFlying" => {
            candidates.insert("Entity".to_owned());
            candidates.insert("EntityLivingBase".to_owned());
        }
        "WorldServer" | "WorldClient" => {
            candidates.insert("World".to_owned());
        }
        "TileEntityLockable" | "TileEntityChest" | "TileEntityFurnace"
        | "TileEntityMobSpawner" | "TileEntitySign" => {
            candidates.insert("TileEntity".to_owned());
        }
        _ => {}
    }
    candidates
}

/// Identify API calls using the declared receiver type or a static API owner.
/// This supplements method-override detection so ItemStack/NBT/World/registry
/// calls inside ordinary methods can be normalized too.
pub fn map_legacy_api_usages(source: &str, version: Option<&str>) -> Vec<LegacyApiMethodMapping> {
    let Some(version) = version.map(canonical_version).transpose_option() else {
        return Vec::new();
    };

    let known_owners: BTreeSet<&str> = DEFINITIONS.iter().map(|entry| entry.owner).collect();
    let mut receivers = BTreeMap::<String, BTreeSet<String>>::new();
    for capture in typed_variable_regex().captures_iter(source) {
        let (Some(owner), Some(variable)) = (capture.name("owner"), capture.name("variable")) else { continue; };
        let candidates = owner_candidates(owner.as_str());
        if candidates.iter().any(|candidate| known_owners.contains(candidate.as_str())) {
            receivers.entry(variable.as_str().to_owned()).or_default().extend(candidates);
        }
    }

    let mut implicit_owners = BTreeSet::new();
    for capture in inheritance_regex().captures_iter(source) {
        if let Some(owner) = capture.name("owner") {
            implicit_owners.extend(owner_candidates(owner.as_str()));
        }
    }

    let mut matched = BTreeMap::<(String, String), LegacyApiMethodMapping>::new();
    for capture in invocation_regex().captures_iter(source) {
        let (Some(receiver), Some(method)) = (capture.name("receiver"), capture.name("name")) else { continue; };
        let receiver = receiver.as_str();
        let method = method.as_str();
        let owners = if known_owners.contains(receiver) {
            owner_candidates(receiver)
        } else if receiver == "this" || receiver == "super" {
            implicit_owners.clone()
        } else {
            receivers.get(receiver).cloned().unwrap_or_default()
        };
        if owners.is_empty() { continue; }

        for definition in DEFINITIONS.iter().filter(|entry| {
            entry.method == method
                && owners.contains(entry.owner)
                && version.map(|v| entry.versions.contains(&v)).unwrap_or(true)
        }) {
            matched.entry((definition.owner.to_owned(), definition.method.to_owned()))
                .or_insert_with(|| to_mapping(definition));
        }
    }
    matched.into_values().collect()
}

fn declaration_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"(?m)^[ \t]*(?:(?:@[A-Za-z_$][A-Za-z0-9_$.]*(?:\([^)]*\))?)[ \t]*\r?\n[ \t]*)*(?:(?:public|protected|private)\s+)(?:(?:static|final|synchronized|native|abstract)\s+)*(?:<[^>\r\n]+>\s*)?[A-Za-z_$][A-Za-z0-9_$.<>\[\], ?]*[ \t]+(?P<name>[A-Za-z_$][A-Za-z0-9_$]*)[ \t]*\("
    ).expect("static legacy API declaration regex"))
}

/// Return the complete version-aware source API dictionary.
pub fn legacy_api_dictionary() -> Vec<LegacyApiMethodMapping> {
    DEFINITIONS.iter().map(to_mapping).collect()
}

/// Return mappings that are documented for one source version.
pub fn legacy_api_dictionary_for_version(version: &str) -> Vec<LegacyApiMethodMapping> {
    let Some(version) = canonical_version(version) else { return Vec::new(); };
    DEFINITIONS.iter()
        .filter(|entry| entry.versions.contains(&version))
        .map(to_mapping)
        .collect()
}

/// Identify declared legacy API overrides. If the source version is unknown,
/// all supported-version candidates are retained instead of guessing a version.
pub fn map_legacy_api_methods(source: &str, version: Option<&str>) -> Vec<LegacyApiMethodMapping> {
    let Some(version) = version.map(canonical_version).transpose_option() else {
        return Vec::new();
    };
    let declared: BTreeSet<String> = declaration_regex()
        .captures_iter(source)
        .filter_map(|capture| capture.name("name").map(|value| value.as_str().to_owned()))
        .collect();
    let known_owners: BTreeSet<&str> = DEFINITIONS.iter().map(|entry| entry.owner).collect();
    let owners: BTreeSet<String> = inheritance_regex()
        .captures_iter(source)
        .filter_map(|capture| capture.name("owner").map(|value| value.as_str().to_owned()))
        .flat_map(|owner| owner_candidates(&owner))
        .filter(|owner| known_owners.contains(owner.as_str()))
        .collect();

    DEFINITIONS.iter()
        .filter(|entry| declared.contains(entry.method) && owners.contains(entry.owner))
        .filter(|entry| version.map(|v| entry.versions.contains(&v)).unwrap_or(true))
        .map(to_mapping)
        .collect()
}

// Helper keeps an invalid explicit version distinct from the "unknown" case.
trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}

impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            Some(Some(value)) => Some(Some(value)),
            Some(None) => None,
            None => Some(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_has_broad_versioned_coverage() {
        let all = legacy_api_dictionary();
        assert!(all.len() >= 150, "expected broad API coverage, got {}", all.len());
        for version in ["1.7.10", "1.8.9", "1.12.2"] {
            assert!(legacy_api_dictionary_for_version(version).len() >= 70, "coverage too small for {version}");
        }
        assert!(legacy_api_dictionary_for_version("1.20.1").is_empty());
    }

    #[test]
    fn covers_gameplay_and_forge_extension_domains() {
        let all = legacy_api_dictionary();
        for (owner, method) in [
            ("Item", "onItemRightClick"),
            ("Block", "onBlockActivated"),
            ("Entity", "writeEntityToNBT"),
            ("EntityLivingBase", "onLivingUpdate"),
            ("World", "spawnEntityInWorld"),
            ("TileEntity", "updateEntity"),
            ("IInventory", "decrStackSize"),
            ("IRecipe", "getCraftingResult"),
            ("IMessage", "toBytes"),
            ("IWorldGenerator", "generate"),
        ] {
            assert!(all.iter().any(|entry| entry.source_owner == owner && entry.source_method == method),
                "missing {owner}#{method}");
        }
    }

    #[test]
    fn version_filter_preserves_version_specific_apis() {
        let old = legacy_api_dictionary_for_version("1.7.10");
        let mid = legacy_api_dictionary_for_version("1.8.9");
        let late = legacy_api_dictionary_for_version("1.12.2");
        assert!(old.iter().any(|entry| entry.source_method == "registerIcons"));
        assert!(!mid.iter().any(|entry| entry.source_method == "registerIcons"));
        assert!(late.iter().any(|entry| entry.source_method == "getUpdateTag"));
        assert!(!old.iter().any(|entry| entry.source_method == "getUpdateTag"));
    }

    #[test]
    fn detects_declared_methods_without_matching_call_sites() {
        let source = r#"
            class ExampleBlock extends Block {
                @Override
                public void updateTick(World world, int x, int y, int z, Random random) { }
                public void helper() { spawnEntityInWorld(entity); }
            }
            class ExampleEntity extends Entity {
                @Override
                protected void writeEntityToNBT(NBTTagCompound tag) { }
            }
        "#;
        let detected = map_legacy_api_methods(source, Some("1.7.10"));
        assert!(detected.iter().any(|entry| entry.source_owner == "Block" && entry.source_method == "updateTick"));
        assert!(detected.iter().any(|entry| entry.source_owner == "Entity" && entry.source_method == "writeEntityToNBT"));
        assert!(!detected.iter().any(|entry| entry.source_method == "spawnEntityInWorld"));
    }

    #[test]
        #[test]
    fn detects_typed_call_sites_without_confusing_same_named_methods() {
        let source = r#"
            class Example {
                void use(ItemStack stack, NBTTagCompound tag, World world) {
                    stack.getItem();
                    tag.getInteger("charge");
                    world.setBlock(1, 2, 3, block);
                    GameRegistry.registerItem(item, "example");
                    helper.updateTask();
                }
            }
        "#;
        let found = map_legacy_api_usages(source, Some("1.12.2"));
        assert!(found.iter().any(|entry| entry.source_owner == "ItemStack" && entry.source_method == "getItem"));
        assert!(found.iter().any(|entry| entry.source_owner == "NBTTagCompound" && entry.source_method == "getInteger"));
        assert!(found.iter().any(|entry| entry.source_owner == "World" && entry.source_method == "setBlock"));
        assert!(found.iter().any(|entry| entry.source_owner == "GameRegistry" && entry.source_method == "registerItem"));
        assert!(!found.iter().any(|entry| entry.source_method == "updateTask"));
    }

fn mappings_include_versioned_javadoc_sources() {
        let item = legacy_api_dictionary_for_version("1.7.10").into_iter()
            .find(|entry| entry.source_owner == "Item" && entry.source_method == "onItemRightClick")
            .unwrap();
        assert!(item.documentation_urls.iter().any(|url| url.contains("1.7.10-10.13.4.1614")));
        let block = legacy_api_dictionary_for_version("1.12.2").into_iter()
            .find(|entry| entry.source_owner == "Block" && entry.source_method == "onBlockActivated")
            .unwrap();
        assert!(block.documentation_urls.iter().any(|url| url.contains("javadoc/1.12.2/")));
    }
}
