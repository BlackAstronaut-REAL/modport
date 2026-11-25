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
use crate::target_api_dictionary::{find_target_api_methods, find_target_api_methods_by_legacy_alias, TargetApiMethod};

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
    pub target_documentation_urls: Vec<String>,
    /// Resolved target method contracts, including exact signature and semantics.
    pub target_api_methods: Vec<TargetApiMethod>,
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
    d!("PotionEffect", "getPotionID", V_17_18, "potion_effect_type", "MobEffectInstance#getEffect", "effects", "adapter_required", "Resolve numeric potion IDs to registry-backed MobEffect holders.", "net/minecraft/potion/PotionEffect.html"),
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

    // Expanded world, entity, capability, rendering and lifecycle APIs.
    d!("IBlockState", "getBlock", V_18, "state_block_identity", "BlockState#getBlock", "block_state", "automatic_rename_candidate", "Translate the state object to a modern BlockState and preserve registry identity.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "getValue", V_18, "state_property_read", "BlockState#getValue", "block_state", "signature_adapter_required", "Translate IProperty to Property and keep the correct generic value type.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "withProperty", V_18, "state_property_write", "BlockState#setValue", "block_state", "signature_adapter_required", "Translate immutable state updates and property value types.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "cycleProperty", V_18, "state_property_cycle", "BlockState#cycle", "block_state", "automatic_rename_candidate", "Preserve property cycling order and default values.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "getProperties", V_18, "state_property_enumeration", "BlockState#getValues", "block_state", "adapter_required", "Translate property-keyed maps and avoid assumptions about property iteration order.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "getMaterial", V_18, "state_material", "BlockState#liquid / BlockState#isAir / material tags", "block_state", "adapter_required", "Material was a bundled classification; recover the exact liquid, replaceable, solid or movement property.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "isFullBlock", V_18, "state_full_block", "BlockState#isCollisionShapeFullBlock", "block_state", "adapter_required", "Do not confuse full collision, visual solidity and occlusion.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "isSideSolid", V_18, "state_side_solid", "BlockState#isFaceSturdy", "block_state", "signature_adapter_required", "Map EnumFacing and EnumBlockFaceShape to Direction and SupportType.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "getCollisionBoundingBox", V_18, "state_collision_box", "BlockState#getCollisionShape", "block_shape", "adapter_required", "Convert AxisAlignedBB to VoxelShape and preserve world/position context.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "getBoundingBox", V_18, "state_outline_box", "BlockState#getShape", "block_shape", "adapter_required", "Preserve state-dependent outline geometry and convert AABB to VoxelShape.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "getBlockHardness", V_18, "state_hardness", "BlockState#getDestroySpeed", "block_properties", "adapter_required", "Preserve world, position and player-specific hardness calculations.", "net/minecraft/block/state/IBlockState.html"),
    d!("IBlockState", "getLightValue", V_18, "state_light_value", "BlockState#getLightEmission", "block_properties", "automatic_rename_candidate", "Preserve state-dependent emitted light.", "net/minecraft/block/state/IBlockState.html"),
    d!("BlockPos", "add", V_18, "block_position_offset", "BlockPos#offset / BlockPos#relative", "world_math", "signature_adapter_required", "Translate immutable/mutable position types and axis/direction overloads.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "offset", V_18, "block_position_direction_offset", "BlockPos#relative", "world_math", "signature_adapter_required", "Preserve direction and distance, including negative offsets.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "up", V_18, "block_position_up", "BlockPos#above", "world_math", "automatic_rename_candidate", "Keep distance argument and immutable position semantics.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "down", V_18, "block_position_down", "BlockPos#below", "world_math", "automatic_rename_candidate", "Keep distance argument and immutable position semantics.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "north", V_18, "block_position_north", "BlockPos#north", "world_math", "automatic_rename_candidate", "Preserve distance and coordinate orientation.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "south", V_18, "block_position_south", "BlockPos#south", "world_math", "automatic_rename_candidate", "Preserve distance and coordinate orientation.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "east", V_18, "block_position_east", "BlockPos#east", "world_math", "automatic_rename_candidate", "Preserve distance and coordinate orientation.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "west", V_18, "block_position_west", "BlockPos#west", "world_math", "automatic_rename_candidate", "Preserve distance and coordinate orientation.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "toLong", V_18, "block_position_pack", "BlockPos#asLong", "world_math", "automatic_rename_candidate", "Preserve packed coordinate bit layout only for compatible serialization.", "net/minecraft/util/math/BlockPos.html"),
    d!("BlockPos", "fromLong", V_18, "block_position_unpack", "BlockPos#of", "world_math", "adapter_required", "Only use packed decoding for values created with the same coordinate encoding.", "net/minecraft/util/math/BlockPos.html"),
    d!("AxisAlignedBB", "offset", V_ALL, "aabb_translate", "AABB#move", "geometry", "signature_adapter_required", "Translate vector and preserve min/max coordinate order.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("AxisAlignedBB", "expand", V_ALL, "aabb_expand", "AABB#inflate", "geometry", "signature_adapter_required", "Legacy expansion may be directional; preserve negative and axis-specific values.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("AxisAlignedBB", "grow", V_ALL, "aabb_grow", "AABB#inflate", "geometry", "signature_adapter_required", "Preserve symmetric versus directional expansion behavior.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("AxisAlignedBB", "union", V_ALL, "aabb_union", "AABB#minmax", "geometry", "automatic_rename_candidate", "Preserve inclusive min/max geometry semantics.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("AxisAlignedBB", "intersects", V_ALL, "aabb_intersection", "AABB#intersects", "geometry", "automatic_rename_candidate", "Preserve strict versus touching-edge intersection semantics.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("AxisAlignedBB", "contains", V_ALL, "aabb_contains", "AABB#contains", "geometry", "automatic_rename_candidate", "Translate Vec3 to modern Vec3 and preserve edge inclusion.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("AxisAlignedBB", "calculateXOffset", V_ALL, "aabb_clip_x", "Entity collision movement / Shapes#collide", "geometry", "adapter_required", "Collision clipping must be migrated as a complete axis-aware movement operation.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("AxisAlignedBB", "calculateYOffset", V_ALL, "aabb_clip_y", "Entity collision movement / Shapes#collide", "geometry", "adapter_required", "Preserve stepping, gravity and collision-axis ordering.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("AxisAlignedBB", "calculateZOffset", V_ALL, "aabb_clip_z", "Entity collision movement / Shapes#collide", "geometry", "adapter_required", "Preserve stepping, gravity and collision-axis ordering.", "net/minecraft/util/AxisAlignedBB.html"),
    d!("RayTraceResult", "getBlockPos", V_18, "raytrace_block_position", "BlockHitResult#getBlockPos", "raytrace", "automatic_rename_candidate", "Preserve hit type and block face.", "net/minecraft/util/math/RayTraceResult.html"),
    d!("RayTraceResult", "getEntity", V_18, "raytrace_entity", "EntityHitResult#getEntity", "raytrace", "adapter_required", "Modern block and entity hit results are distinct subclasses.", "net/minecraft/util/math/RayTraceResult.html"),
    d!("RayTraceResult", "getSideHit", V_18, "raytrace_hit_face", "BlockHitResult#getDirection", "raytrace", "automatic_rename_candidate", "Map EnumFacing to Direction.", "net/minecraft/util/math/RayTraceResult.html"),
    d!("Vec3d", "addVector", V_18, "vector_add", "Vec3#add", "world_math", "automatic_rename_candidate", "Preserve double precision and immutable vector result.", "net/minecraft/util/math/Vec3d.html"),
    d!("Vec3d", "subtract", V_18, "vector_subtract", "Vec3#subtract", "world_math", "automatic_rename_candidate", "Preserve operand order.", "net/minecraft/util/math/Vec3d.html"),
    d!("Vec3d", "scale", V_18, "vector_scale", "Vec3#scale", "world_math", "automatic_rename_candidate", "Preserve scalar multiplication and precision.", "net/minecraft/util/math/Vec3d.html"),
    d!("Vec3d", "normalize", V_18, "vector_normalize", "Vec3#normalize", "world_math", "automatic_rename_candidate", "Preserve zero-length vector behavior.", "net/minecraft/util/math/Vec3d.html"),
    d!("Vec3d", "distanceTo", V_18, "vector_distance", "Vec3#distanceTo", "world_math", "automatic_rename_candidate", "Preserve non-squared distance.", "net/minecraft/util/math/Vec3d.html"),
    d!("Vec3d", "squareDistanceTo", V_18, "vector_distance_squared", "Vec3#distanceToSqr", "world_math", "automatic_rename_candidate", "Preserve squared-distance semantics.", "net/minecraft/util/math/Vec3d.html"),
    d!("World", "setBlockState", V_18, "set_block_state", "Level#setBlock", "world", "signature_adapter_required", "Translate flags and preserve block-entity update behavior.", "net/minecraft/world/World.html"),
    d!("World", "destroyBlock", V_18, "destroy_block", "Level#destroyBlock", "world", "signature_adapter_required", "Preserve drops, experience, player context and update flags.", "net/minecraft/world/World.html"),
    d!("World", "isBlockLoaded", V_18, "block_loaded_query", "Level#hasChunkAt", "world", "automatic_rename_candidate", "Avoid force-loading chunks; retain boundary behavior.", "net/minecraft/world/World.html"),
    d!("World", "isAreaLoaded", V_18, "area_loaded_query", "Level#hasChunksAt", "world", "signature_adapter_required", "Preserve inclusive/exclusive bounds and chunk-loading policy.", "net/minecraft/world/World.html"),
    d!("World", "getHeight", V_ALL, "world_height_query", "Level#getHeight / LevelReader#getHeight", "world", "signature_adapter_required", "Translate legacy height-map type and coordinate arguments.", "net/minecraft/world/World.html"),
    d!("World", "getHeightValue", V_17, "world_surface_height", "Level#getHeightmapPos", "world", "signature_adapter_required", "Translate coordinates and preserve top-solid/top-liquid semantics.", "net/minecraft/world/World.html"),
    d!("World", "getTopSolidOrLiquidBlock", V_18, "world_top_block", "Level#getHeightmapPos", "world", "signature_adapter_required", "Choose the correct modern Heightmap.Types value.", "net/minecraft/world/World.html"),
    d!("World", "getPrecipitationHeight", V_18, "world_precipitation_height", "Level#getHeightmapPos", "world", "adapter_required", "Preserve precipitation and motion-blocking heightmap distinctions.", "net/minecraft/world/World.html"),
    d!("World", "getRainStrength", V_ALL, "world_rain_strength", "Level#getRainLevel", "world_weather", "automatic_rename_candidate", "Preserve partial-tick interpolation.", "net/minecraft/world/World.html"),
    d!("World", "getThunderStrength", V_ALL, "world_thunder_strength", "Level#getThunderLevel", "world_weather", "automatic_rename_candidate", "Preserve partial-tick interpolation.", "net/minecraft/world/World.html"),
    d!("World", "isRaining", V_ALL, "world_raining", "Level#isRaining", "world_weather", "automatic_rename_candidate", "Preserve local biome precipitation checks if present.", "net/minecraft/world/World.html"),
    d!("World", "isThundering", V_ALL, "world_thundering", "Level#isThundering", "world_weather", "automatic_rename_candidate", "Preserve dimension weather behavior.", "net/minecraft/world/World.html"),
    d!("World", "getDifficulty", V_ALL, "world_difficulty", "Level#getDifficulty", "world", "automatic_rename_candidate", "Translate EnumDifficulty to Difficulty.", "net/minecraft/world/World.html"),
    d!("World", "getWorldInfo", V_ALL, "world_level_data", "Level#getLevelData", "world", "adapter_required", "Only map fields that still exist; WorldInfo responsibilities are split in the target.", "net/minecraft/world/World.html"),
    d!("World", "getGameRules", V_ALL, "world_game_rules", "Level#getGameRules", "world", "automatic_rename_candidate", "Preserve typed rule keys instead of string-only lookups.", "net/minecraft/world/World.html"),
    d!("World", "getWorldBorder", V_ALL, "world_border", "Level#getWorldBorder", "world", "automatic_rename_candidate", "Translate world-border warning and collision APIs.", "net/minecraft/world/World.html"),
    d!("World", "getDifficultyForLocation", V_18, "local_difficulty", "Level#getCurrentDifficultyAt", "world", "signature_adapter_required", "Convert BlockPos and preserve inhabited-time scaling.", "net/minecraft/world/World.html"),
    d!("World", "getChunk", V_ALL, "chunk_lookup", "Level#getChunk", "world", "signature_adapter_required", "Use a non-loading lookup when the legacy call only requested an already-loaded chunk.", "net/minecraft/world/World.html"),
    d!("World", "getChunkFromChunkCoords", V_ALL, "chunk_coordinate_lookup", "Level#getChunk(int,int)", "world", "signature_adapter_required", "Preserve chunk-load behavior and coordinate order.", "net/minecraft/world/World.html"),
    d!("World", "getLoadedEntityList", V_ALL, "loaded_entities", "Level#getEntities / EntityGetter", "world", "adapter_required", "Avoid exposing mutable internal entity lists; use filtered query APIs.", "net/minecraft/world/World.html"),
    d!("World", "getEntityByID", V_ALL, "entity_id_lookup", "Level#getEntity", "world", "automatic_rename_candidate", "Preserve runtime entity ID rather than UUID lookup.", "net/minecraft/world/World.html"),
    d!("World", "getEntityByUuid", V_18, "entity_uuid_lookup", "ServerLevel#getEntity / Entity#getUUID", "world", "adapter_required", "Use the appropriate server-level lookup and unloaded-entity behavior.", "net/minecraft/world/World.html"),
    d!("World", "spawnParticle", V_ALL, "spawn_particle", "Level#addParticle / sendParticles", "particles", "signature_adapter_required", "Choose client-only or server-broadcast particle dispatch and preserve particle arguments.", "net/minecraft/world/World.html"),
    d!("World", "isRemote", V_ALL, "logical_client_side", "Level#isClientSide", "sides", "automatic_rename_candidate", "Preserve logical-side checks; do not substitute physical Dist checks.", "net/minecraft/world/World.html"),
    d!("WorldProvider", "getDimensionName", V_ALL, "dimension_name", "DimensionType / Level#dimension", "dimensions", "adapter_required", "Translate provider names to registered dimension identifiers and effects.", "net/minecraft/world/WorldProvider.html"),
    d!("WorldProvider", "getDimensionId", V_ALL, "dimension_numeric_id", "ResourceKey<Level> / Level#dimension", "dimensions", "adapter_required", "Numeric dimension IDs are no longer the stable dimension identity.", "net/minecraft/world/WorldProvider.html"),
    d!("WorldProvider", "getCloudHeight", V_ALL, "dimension_cloud_height", "DimensionSpecialEffects#getCloudLevel", "dimensions", "client_adapter_required", "Move visual cloud height into dimension client effects.", "net/minecraft/world/WorldProvider.html"),
    d!("WorldProvider", "isSurfaceWorld", V_ALL, "surface_dimension_flag", "DimensionType attributes / Level#dimension", "dimensions", "adapter_required", "Recover sky/light/respawn properties instead of a single surface boolean.", "net/minecraft/world/WorldProvider.html"),
    d!("WorldProvider", "canRespawnHere", V_ALL, "dimension_respawn_allowed", "ServerLevel#isRespawnable", "dimensions", "adapter_required", "Preserve bed/anchor and dimension-specific respawn policy.", "net/minecraft/world/WorldProvider.html"),
    d!("WorldProvider", "getRespawnDimension", V_ALL, "respawn_dimension", "ServerPlayer respawn dimension / ServerLevel", "dimensions", "manual_adapter_required", "Reconstruct player respawn flow using modern respawn data and events.", "net/minecraft/world/WorldProvider.html"),
    d!("WorldProvider", "calculateCelestialAngle", V_ALL, "celestial_angle", "DimensionSpecialEffects#getSunriseColor / Level#getTimeOfDay", "dimensions", "adapter_required", "Preserve custom day length and sky rendering interpolation.", "net/minecraft/world/WorldProvider.html"),
    d!("WorldProvider", "getDimensionType", V_18, "dimension_type", "Level#dimensionType / DimensionType", "dimensions", "adapter_required", "Translate the provider's dimension type into a registered dimension type.", "net/minecraft/world/WorldProvider.html"),
    d!("Chunk", "getBlockState", V_18, "chunk_block_state_read", "Level#getBlockState", "chunk", "adapter_required", "Prefer level access to preserve palette and unloaded-chunk semantics.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "setBlockState", V_18, "chunk_block_state_write", "Level#setBlock / LevelChunk#setBlockState", "chunk", "adapter_required", "Preserve heightmap, lighting, block-entity and neighbor update effects.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "getTileEntity", V_ALL, "chunk_block_entity_lookup", "LevelChunk#getBlockEntity", "chunk", "adapter_required", "Translate coordinates to BlockPos and check creation/load flags.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "addTileEntity", V_ALL, "chunk_block_entity_add", "LevelChunk#setBlockEntity", "chunk", "adapter_required", "Ensure the entity is attached to the correct level and position.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "removeTileEntity", V_ALL, "chunk_block_entity_remove", "LevelChunk#removeBlockEntity", "chunk", "automatic_rename_candidate", "Preserve removal reason and deferred block-entity lifecycle.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "getHeightValue", V_ALL, "chunk_height_value", "Level#getHeight / Heightmap", "chunk", "adapter_required", "Translate legacy heightmap type into Heightmap.Types.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "getEntitiesWithinAABBForEntity", V_ALL, "chunk_entity_aabb_query", "Level#getEntitiesOfClass", "chunk", "adapter_required", "Use a type-filtered level query and preserve entity exclusion predicates.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "addEntity", V_ALL, "chunk_add_entity", "ServerLevel#addFreshEntity / Level#addFreshEntity", "chunk", "adapter_required", "Entity insertion is controlled by the level/entity manager, not direct chunk lists.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "onLoad", V_ALL, "chunk_load_lifecycle", "ChunkEvent.Load / LevelChunk lifecycle", "chunk", "event_adapter_required", "Move work to chunk load events/ticking lifecycle and preserve once-only execution.", "net/minecraft/world/chunk/Chunk.html"),
    d!("Chunk", "onUnload", V_ALL, "chunk_unload_lifecycle", "ChunkEvent.Unload / LevelChunk lifecycle", "chunk", "event_adapter_required", "Move cleanup to unload events and invalidate attached capabilities.", "net/minecraft/world/chunk/Chunk.html"),
    d!("BiomeGenBase", "getTemperature", V_ALL, "biome_temperature", "Biome#getModifiedClimateSettings", "biomes", "adapter_required", "Temperature is climate settings and may be position-adjusted.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("BiomeGenBase", "getRainfall", V_ALL, "biome_downfall", "Biome#getModifiedClimateSettings", "biomes", "adapter_required", "Map rainfall to downfall and preserve precipitation type.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("BiomeGenBase", "getBiomeName", V_ALL, "biome_display_name", "Biome registry key / Component localization", "biomes", "adapter_required", "Use registry identity for logic and Component localization for display.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("BiomeGenBase", "getSpawnableList", V_ALL, "biome_spawn_list", "Biome spawner settings / biome modifiers", "biomes", "manual_adapter_required", "Rebuild weighted spawn entries as biome spawner data or biome modifier additions.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("BiomeGenBase", "getGrassColorAtPos", V_ALL, "biome_grass_color", "Biome#getGrassColor", "biomes", "client_adapter_required", "Preserve position tinting through biome color resolvers.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("BiomeGenBase", "getFoliageColorAtPos", V_ALL, "biome_foliage_color", "Biome#getFoliageColor", "biomes", "client_adapter_required", "Preserve position tinting through biome color resolvers.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("Block", "getBlockFaceShape", V_112, "block_face_shape", "BlockState#isFaceSturdy / SupportType", "block_shape", "adapter_required", "Map EnumBlockFaceShape to modern face-sturdiness support queries.", "net/minecraft/block/Block.html"),
    d!("Block", "isSideSolid", V_ALL, "block_side_solid", "BlockState#isFaceSturdy", "block_shape", "adapter_required", "Translate direction and support type; old boolean solidity combined multiple meanings.", "net/minecraft/block/Block.html"),
    d!("Block", "isSideInvisible", V_18, "block_face_culling", "Block#skipRendering", "block_rendering", "adapter_required", "Preserve neighbor-state culling and face-specific render behavior.", "net/minecraft/block/Block.html"),
    d!("Block", "isPassable", V_ALL, "block_passable", "BlockState#getCollisionShape / Block#isPathfindable", "block_shape", "adapter_required", "Determine whether the source is collision passability or mob pathfinding.", "net/minecraft/block/Block.html"),
    d!("Block", "getWeakPower", V_ALL, "weak_redstone_power", "Block#getSignal", "redstone", "signature_adapter_required", "Translate block state, level, position and direction.", "net/minecraft/block/Block.html"),
    d!("Block", "getStrongPower", V_ALL, "strong_redstone_power", "Block#getDirectSignal", "redstone", "signature_adapter_required", "Preserve direct versus weak signal semantics.", "net/minecraft/block/Block.html"),
    d!("Block", "isProvidingWeakPower", V_17, "weak_redstone_power", "Block#getSignal", "redstone", "signature_adapter_required", "1.7 callback uses World coordinates; map to modern state/level/position/direction.", "net/minecraft/block/Block.html"),
    d!("Block", "isProvidingStrongPower", V_17, "strong_redstone_power", "Block#getDirectSignal", "redstone", "signature_adapter_required", "Preserve direct signal strength and direction.", "net/minecraft/block/Block.html"),
    d!("Block", "getComparatorInputOverride", V_ALL, "comparator_output", "Block#getAnalogOutputSignal", "redstone", "signature_adapter_required", "Map comparator override and has-analog-output predicate.", "net/minecraft/block/Block.html"),
    d!("Block", "hasComparatorInputOverride", V_18, "comparator_output_enabled", "Block#hasAnalogOutputSignal", "redstone", "automatic_rename_candidate", "Preserve state-dependent analog output availability.", "net/minecraft/block/Block.html"),
    d!("Block", "canProvidePower", V_ALL, "redstone_provider", "Block#isSignalSource", "redstone", "automatic_rename_candidate", "Preserve distinction between signal source and current signal strength.", "net/minecraft/block/Block.html"),
    d!("Block", "hasTileEntity", V_18, "block_entity_presence", "Block#hasBlockEntity / BlockEntityType association", "block_entity", "adapter_required", "Translate state-dependent presence and registration into modern block-entity type association.", "net/minecraft/block/Block.html"),
    d!("Block", "createTileEntity", V_18, "block_entity_factory", "EntityBlock#newBlockEntity / BlockEntityType", "block_entity", "adapter_required", "Move construction into the block entity type and EntityBlock implementation.", "net/minecraft/block/Block.html"),
    d!("Block", "onLoad", V_112, "block_chunk_load", "Block#onPlace / ChunkEvent.Load", "block_lifecycle", "event_adapter_required", "Identify whether source work is block placement or chunk-load restoration.", "net/minecraft/block/Block.html"),
    d!("Block", "getStateId", V_18, "block_state_numeric_id", "Block.getId / BlockStateIdMap", "block_state", "adapter_required", "Numeric runtime state IDs are not stable across registry reloads.", "net/minecraft/block/Block.html"),
    d!("Block", "getStateById", V_18, "block_state_from_numeric_id", "Block.stateById / BlockStateIdMap", "block_state", "adapter_required", "Only decode legacy saved IDs with a compatibility table; never treat them as target IDs.", "net/minecraft/block/Block.html"),
    d!("Block", "getActualState", V_18, "block_actual_state", "Block#getStateForPlacement / BlockState properties", "block_state", "adapter_required", "Move neighbor-dependent state resolution into explicit modern state/property logic.", "net/minecraft/block/Block.html"),
    d!("Block", "getExtendedState", V_18, "block_extended_model_state", "IForgeBakedModel#getModelData / ModelData", "block_rendering", "client_adapter_required", "Translate IBlockState/IUnlistedProperty model data into modern ModelData.", "net/minecraft/block/Block.html"),
    d!("Block", "getOffset", V_18, "block_model_offset", "BlockStateBase#getOffset / OffsetType", "block_rendering", "adapter_required", "Preserve deterministic/random offset behavior and state context.", "net/minecraft/block/Block.html"),
    d!("EntityLivingBase", "getCreatureAttribute", V_ALL, "creature_type", "MobType", "living_attributes", "enum_adapter_required", "Map UNDEFINED/UNDEAD/ARTHROPOD to MobType and preserve combat enchantment interactions.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isPotionActive", V_ALL, "has_active_effect", "LivingEntity#hasEffect", "effects", "automatic_rename_candidate", "Translate Potion to MobEffect and preserve duration/amplifier checks.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getActivePotionEffect", V_ALL, "active_effect_lookup", "LivingEntity#getEffect", "effects", "adapter_required", "Translate Potion to MobEffect and effect instance representation.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "addPotionEffect", V_ALL, "apply_status_effect", "LivingEntity#addEffect", "effects", "signature_adapter_required", "Preserve effect source, ambient flag, particles and update semantics.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "removePotionEffect", V_ALL, "remove_status_effect", "LivingEntity#removeEffect", "effects", "adapter_required", "Translate Potion to MobEffect and preserve removal callbacks.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "clearActivePotions", V_ALL, "clear_status_effects", "LivingEntity#removeAllEffects", "effects", "automatic_rename_candidate", "Preserve effect removal callbacks and hidden-effect chains.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isEntityInvulnerable", V_ALL, "entity_damage_immune", "Entity#isInvulnerableTo", "living_combat", "signature_adapter_required", "Modern check is damage-source-aware; preserve source-specific immunity.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isOnLadder", V_ALL, "entity_on_climbable", "LivingEntity#onClimbable", "living_movement", "automatic_rename_candidate", "Preserve custom climbable block tags and collision behavior.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isElytraFlying", V_18, "elytra_flight", "LivingEntity#isFallFlying", "living_movement", "automatic_rename_candidate", "Preserve fall-flying pose and equipment checks.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isChild", V_ALL, "living_child_state", "AgeableMob#isBaby", "living_state", "adapter_required", "Map child/adult scaling and ageable mob type.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getSoundVolume", V_ALL, "living_sound_volume", "LivingEntity#getSoundVolume", "audio", "automatic_rename_candidate", "Preserve per-entity volume scaling.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getSoundPitch", V_ALL, "living_sound_pitch", "LivingEntity#getVoicePitch", "audio", "automatic_rename_candidate", "Preserve randomized voice pitch and child scaling.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getMaxFallHeight", V_ALL, "living_max_fall_height", "Mob#getMaxFallDistance", "living_movement", "adapter_required", "Preserve target block and effect-dependent fall-height calculations.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLiving", "initEntityAI", V_ALL, "living_ai_goal_registration", "Mob#registerGoals", "entity_ai", "manual_adapter_required", "Reconstruct GoalSelector priorities and target selector ordering.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "applyEntityAttributes", V_ALL, "living_attribute_defaults", "Mob#createAttributes / AttributeSupplier.Builder", "living_attributes", "adapter_required", "Move base attribute values into entity-type attribute supplier registration.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getCanSpawnHere", V_ALL, "mob_spawn_predicate", "SpawnPlacements / Mob#checkSpawnRules", "spawning", "adapter_required", "Move spawn rules into SpawnPlacements and preserve difficulty/light/biome checks.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "onInitialSpawn", V_ALL, "mob_finalize_spawn", "Mob#finalizeSpawn", "spawning", "signature_adapter_required", "Translate difficulty, group data and spawn reason.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setAttackTarget", V_ALL, "mob_attack_target_write", "Mob#setTarget", "entity_ai", "automatic_rename_candidate", "Preserve target goals and last-hurt-by state.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getAttackTarget", V_ALL, "mob_attack_target_read", "Mob#getTarget", "entity_ai", "automatic_rename_candidate", "Distinguish attack target from revenge/last-attacker target.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getNavigator", V_ALL, "mob_navigation", "Mob#getNavigation", "entity_ai", "automatic_rename_candidate", "Translate PathNavigate to typed PathNavigation implementations.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getEntitySenses", V_ALL, "mob_senses", "Mob#getSensing", "entity_ai", "automatic_rename_candidate", "Preserve cached visibility queries and target visibility timing.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getMoveHelper", V_ALL, "mob_move_control", "Mob#getMoveControl", "entity_ai", "automatic_rename_candidate", "Port custom move helper behavior to MoveControl.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getLookHelper", V_ALL, "mob_look_control", "Mob#getLookControl", "entity_ai", "automatic_rename_candidate", "Port look targets and rotation limits to LookControl.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "isWithinHomeDistanceFromPosition", V_ALL, "mob_home_distance", "Mob#isWithinRestriction", "entity_ai", "signature_adapter_required", "Translate BlockPos and preserve home restriction radius.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setHomePosAndDistance", V_ALL, "mob_set_home", "Mob#restrictTo", "entity_ai", "signature_adapter_required", "Translate BlockPos and radius; preserve home removal semantics.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "detachHome", V_ALL, "mob_clear_home", "Mob#clearRestriction", "entity_ai", "automatic_rename_candidate", "Preserve navigation and home-seeking goal interactions.", "net/minecraft/entity/EntityLiving.html"),
    d!("Entity", "isInLava", V_ALL, "entity_in_lava", "Entity#isInLava", "entity_state", "automatic_rename_candidate", "Preserve fluid-tag behavior and eye-submersion distinctions.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "isInsideOfMaterial", V_ALL, "entity_inside_material", "Entity#isInWater / fluid-state queries", "entity_state", "adapter_required", "Replace Material tests with FluidState and tag checks.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getEntityId", V_ALL, "entity_runtime_id", "Entity#getId", "entity_state", "automatic_rename_candidate", "Runtime IDs are session-local and differ from UUID identity.", "net/minecraft/entity/Entity.html"),
    d!("InventoryPlayer", "getStackInSlot", V_ALL, "player_inventory_slot_read", "Inventory#getItem", "inventory", "signature_adapter_required", "Translate hotbar/armor/offhand index layout to modern inventory accessors.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("InventoryPlayer", "setInventorySlotContents", V_ALL, "player_inventory_slot_write", "Inventory#setItem", "inventory", "signature_adapter_required", "Preserve selected hotbar slot and equipment synchronization.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("InventoryPlayer", "clearMatchingItems", V_ALL, "player_inventory_consume_matching", "Inventory#clearOrCountMatchingItems / player inventory search", "inventory", "manual_adapter_required", "Reconstruct item, metadata and tag matching with stack components.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("InventoryPlayer", "getFirstEmptyStack", V_ALL, "player_inventory_empty_slot", "Inventory#getFreeSlot", "inventory", "automatic_rename_candidate", "Preserve hotbar-only versus whole-inventory search.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("InventoryPlayer", "getSlotFor", V_ALL, "player_inventory_find_item", "Inventory#findSlotMatchingItem", "inventory", "adapter_required", "Preserve item identity, component matching and search order.", "net/minecraft/entity/player/InventoryPlayer.html"),
    d!("EntityPlayer", "getDisplayName", V_ALL, "player_display_name", "Player#getDisplayName", "localization", "automatic_rename_candidate", "Preserve scoreboard/team display name formatting.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getGameProfile", V_ALL, "player_profile", "Player#getGameProfile", "player", "automatic_rename_candidate", "Preserve UUID/name identity and offline-profile handling.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "canPlayerEdit", V_ALL, "player_can_edit_block", "Player#mayBuild / adventure mode checks", "player", "signature_adapter_required", "Translate position, face and ItemStack restrictions into adventure-mode predicates.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "addStat", V_ALL, "player_award_stat", "Player#awardStat", "player", "signature_adapter_required", "Map Achievement/StatBase to ResourceLocation-backed Stats and criteria.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "addExhaustion", V_ALL, "player_add_exhaustion", "Player#causeFoodExhaustion", "player", "automatic_rename_candidate", "Preserve exhaustion source and saturation/food interactions.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "setSpawnPoint", V_ALL, "player_set_respawn", "ServerPlayer#setRespawnPosition", "player", "signature_adapter_required", "Translate BlockPos, forced flag and dimension key.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getBedLocation", V_ALL, "player_respawn_position", "ServerPlayer#getRespawnPosition", "player", "adapter_required", "Modern respawn data includes dimension and forced-respawn policy.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isSpectator", V_ALL, "player_spectator", "Player#isSpectator", "player", "automatic_rename_candidate", "Preserve spectator state and interaction restrictions.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isCreative", V_ALL, "player_creative", "Player#isCreative", "player", "automatic_rename_candidate", "Preserve instant-break and item-use behavior.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("PlayerCapabilities", "isCreativeMode", V_ALL, "player_creative_flag", "Abilities#instabuild", "player", "automatic_rename_candidate", "Map creative build flag, not flight or invulnerability.", "net/minecraft/entity/player/PlayerCapabilities.html"),
    d!("PlayerCapabilities", "allowFlying", V_ALL, "player_flight_permission", "Abilities#mayfly", "player", "automatic_rename_candidate", "Preserve flight permission separately from current flying state.", "net/minecraft/entity/player/PlayerCapabilities.html"),
    d!("PlayerCapabilities", "isFlying", V_ALL, "player_flying_state", "Abilities#flying", "player", "automatic_rename_candidate", "Preserve current flight state and server ability synchronization.", "net/minecraft/entity/player/PlayerCapabilities.html"),
    d!("PlayerCapabilities", "setFlySpeed", V_ALL, "player_fly_speed", "Abilities#setFlyingSpeed", "player", "automatic_rename_candidate", "Preserve speed units and client/server packet updates.", "net/minecraft/entity/player/PlayerCapabilities.html"),
    d!("NBTTagList", "appendTag", V_ALL, "nbt_list_append", "ListTag#add", "persistence", "automatic_rename_candidate", "Preserve typed list element invariants and compound ownership.", "net/minecraft/nbt/NBTTagList.html"),
    d!("NBTTagList", "get", V_18, "nbt_list_read", "ListTag#get", "persistence", "signature_adapter_required", "Modern list access returns Tag; convert to the expected typed subtype.", "net/minecraft/nbt/NBTTagList.html"),
    d!("NBTTagList", "getCompoundTagAt", V_ALL, "nbt_list_compound_read", "ListTag#getCompound", "persistence", "automatic_rename_candidate", "Preserve default empty-compound behavior for invalid entries.", "net/minecraft/nbt/NBTTagList.html"),
    d!("NBTTagList", "getStringTagAt", V_ALL, "nbt_list_string_read", "ListTag#getString", "persistence", "automatic_rename_candidate", "Preserve index bounds and non-string element behavior.", "net/minecraft/nbt/NBTTagList.html"),
    d!("NBTTagList", "tagCount", V_ALL, "nbt_list_size", "ListTag#size", "persistence", "automatic_rename_candidate", "Preserve list length including malformed legacy entries.", "net/minecraft/nbt/NBTTagList.html"),
    d!("NBTTagList", "removeTag", V_ALL, "nbt_list_remove", "ListTag#remove", "persistence", "automatic_rename_candidate", "Preserve index validation and shifted element ordering.", "net/minecraft/nbt/NBTTagList.html"),
    d!("NBTTagCompound", "getTag", V_ALL, "nbt_compound_get", "CompoundTag#get", "persistence", "automatic_rename_candidate", "Preserve absent-key null handling and tag ownership.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setIntArray", V_ALL, "nbt_int_array_write", "CompoundTag#putIntArray", "persistence", "automatic_rename_candidate", "Preserve array copying and integer range.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getIntArray", V_ALL, "nbt_int_array_read", "CompoundTag#getIntArray", "persistence", "automatic_rename_candidate", "Preserve absent-key empty-array behavior.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setByteArray", V_ALL, "nbt_byte_array_write", "CompoundTag#putByteArray", "persistence", "automatic_rename_candidate", "Preserve byte array copy/ownership behavior.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getByteArray", V_ALL, "nbt_byte_array_read", "CompoundTag#getByteArray", "persistence", "automatic_rename_candidate", "Preserve absent-key empty-array behavior.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "setUniqueId", V_18, "nbt_uuid_write", "CompoundTag#putUUID", "persistence", "automatic_rename_candidate", "Translate split-most/least legacy UUID encoding to modern UUID NBT.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTTagCompound", "getUniqueId", V_18, "nbt_uuid_read", "CompoundTag#getUUID", "persistence", "adapter_required", "Support legacy split-long UUID fields when migrating old saved data.", "net/minecraft/nbt/NBTTagCompound.html"),
    d!("NBTBase", "getId", V_ALL, "nbt_tag_type_id", "Tag#getId / TagTypes", "persistence", "automatic_rename_candidate", "Preserve NBT type id semantics.", "net/minecraft/nbt/NBTBase.html"),
    d!("NBTBase", "copy", V_ALL, "nbt_tag_copy", "Tag#copy", "persistence", "automatic_rename_candidate", "Preserve deep-copy semantics for compound/list descendants.", "net/minecraft/nbt/NBTBase.html"),
    d!("ICapabilityProvider", "hasCapability", V_18, "capability_presence", "ICapabilityProvider#getCapability / RegisterCapabilitiesEvent", "capabilities", "signature_adapter_required", "Translate Capability and EnumFacing to registered NeoForge capability and context.", "net/minecraftforge/common/capabilities/ICapabilityProvider.html"),
    d!("ICapabilityProvider", "getCapability", V_18, "capability_lookup", "ICapabilityProvider#getCapability", "capabilities", "signature_adapter_required", "Modern capability lookups use typed capabilities and optional results.", "net/minecraftforge/common/capabilities/ICapabilityProvider.html"),
    d!("CapabilityManager", "register", V_18, "capability_type_registration", "RegisterCapabilitiesEvent#register", "capabilities", "registration_adapter_required", "Register capability types during mod lifecycle and provide serializers where required.", "net/minecraftforge/common/capabilities/CapabilityManager.html"),
    d!("Capability", "cast", V_18, "capability_type_cast", "Capability#isRegistered / Optional capability values", "capabilities", "adapter_required", "Avoid unchecked legacy capability casts; preserve invalidation and provider ownership.", "net/minecraftforge/common/capabilities/Capability.html"),
    d!("Fluid", "getName", V_18, "fluid_registry_name", "Fluid#isSame / FluidStack registry holder", "fluids", "adapter_required", "Modern fluid identity is registry-backed; do not rely on display name.", "net/minecraftforge/fluids/Fluid.html"),
    d!("Fluid", "getLocalizedName", V_18, "fluid_display_name", "FluidStack#getDisplayName / Component", "fluids", "signature_adapter_required", "Translate localized String to Component and keep client localization.", "net/minecraftforge/fluids/Fluid.html"),
    d!("Fluid", "getStill", V_18, "fluid_still_texture", "FluidType#initializeClient / fluid render resources", "fluids", "client_adapter_required", "Move still/flow textures to the FluidType client extension.", "net/minecraftforge/fluids/Fluid.html"),
    d!("Fluid", "getFlowing", V_18, "fluid_flow_texture", "FluidType#initializeClient / fluid render resources", "fluids", "client_adapter_required", "Move flowing texture to FluidType client extension.", "net/minecraftforge/fluids/Fluid.html"),
    d!("FluidStack", "getFluid", V_18, "fluid_stack_identity", "FluidStack#getFluid", "fluids", "automatic_rename_candidate", "Preserve registry identity and empty stack semantics.", "net/minecraftforge/fluids/FluidStack.html"),
    d!("FluidStack", "getAmount", V_18, "fluid_stack_amount", "FluidStack#getAmount", "fluids", "automatic_rename_candidate", "Preserve amount units (millibuckets) and capacity bounds.", "net/minecraftforge/fluids/FluidStack.html"),
    d!("FluidStack", "grow", V_18, "fluid_stack_grow", "FluidStack#grow", "fluids", "automatic_rename_candidate", "Preserve amount clamping and empty-stack normalization.", "net/minecraftforge/fluids/FluidStack.html"),
    d!("FluidStack", "shrink", V_18, "fluid_stack_shrink", "FluidStack#shrink", "fluids", "automatic_rename_candidate", "Preserve amount clamping and empty-stack normalization.", "net/minecraftforge/fluids/FluidStack.html"),
    d!("IFluidHandler", "fill", V_18, "fluid_handler_fill", "IFluidHandler#fill", "fluids", "signature_adapter_required", "Translate FluidAction simulation/execute semantics and fluid contents.", "net/minecraftforge/fluids/capability/IFluidHandler.html"),
    d!("IFluidHandler", "drain", V_18, "fluid_handler_drain", "IFluidHandler#drain", "fluids", "signature_adapter_required", "Preserve simulation and amount-versus-fluid-stack overload distinctions.", "net/minecraftforge/fluids/capability/IFluidHandler.html"),
    d!("IFluidHandler", "getTankProperties", V_18, "fluid_handler_tanks", "IFluidHandler#getTanks / tank capacity queries", "fluids", "adapter_required", "Modern fluid handlers may expose tank count and per-tank state through different extension methods.", "net/minecraftforge/fluids/capability/IFluidHandler.html"),
    d!("IEnergyStorage", "receiveEnergy", V_112, "energy_receive", "IEnergyStorage#receiveEnergy", "energy", "automatic_rename_candidate", "Preserve simulate flag and energy unit (Forge Energy).", "net/minecraftforge/energy/IEnergyStorage.html"),
    d!("IEnergyStorage", "extractEnergy", V_112, "energy_extract", "IEnergyStorage#extractEnergy", "energy", "automatic_rename_candidate", "Preserve simulate flag and extraction limits.", "net/minecraftforge/energy/IEnergyStorage.html"),
    d!("IEnergyStorage", "getEnergyStored", V_112, "energy_stored", "IEnergyStorage#getEnergyStored", "energy", "automatic_rename_candidate", "Preserve stored energy query and unit.", "net/minecraftforge/energy/IEnergyStorage.html"),
    d!("IEnergyStorage", "getMaxEnergyStored", V_112, "energy_capacity", "IEnergyStorage#getMaxEnergyStored", "energy", "automatic_rename_candidate", "Preserve capacity and transfer-rate distinction.", "net/minecraftforge/energy/IEnergyStorage.html"),
    d!("IEnergyStorage", "canExtract", V_112, "energy_can_extract", "IEnergyStorage#canExtract", "energy", "automatic_rename_candidate", "Preserve directional extraction rules.", "net/minecraftforge/energy/IEnergyStorage.html"),
    d!("IEnergyStorage", "canReceive", V_112, "energy_can_receive", "IEnergyStorage#canReceive", "energy", "automatic_rename_candidate", "Preserve directional input rules.", "net/minecraftforge/energy/IEnergyStorage.html"),
    d!("RenderingRegistry", "registerEntityRenderingHandler", V_ALL, "entity_renderer_registration", "EntityRenderersEvent.RegisterRenderers / EntityRendererProvider", "rendering", "registration_adapter_required", "Register an EntityRendererProvider on the mod event bus and preserve renderer/model dependencies.", "net/minecraftforge/fml/client/registry/RenderingRegistry.html"),
    d!("ClientRegistry", "bindTileEntitySpecialRenderer", V_18, "block_entity_renderer_registration", "EntityRenderersEvent.RegisterRenderers", "rendering", "registration_adapter_required", "Convert TileEntitySpecialRenderer to BlockEntityRenderer and register by BlockEntityType.", "net/minecraftforge/fml/client/registry/ClientRegistry.html"),
    d!("ModelRegistryEvent", "getRegistry", V_18, "model_registry_event", "ModelEvent.RegisterAdditional / RegisterGeometryLoaders", "rendering", "event_adapter_required", "Separate additional model registration from baked model modification and geometry-loader registration.", "net/minecraftforge/client/event/ModelRegistryEvent.html"),
    d!("ModelBakeEvent", "getModelRegistry", V_18, "model_bake_registry", "ModelEvent.ModifyBakingResult", "rendering", "event_adapter_required", "Preserve model-location key lookup and bake ordering.", "net/minecraftforge/client/event/ModelBakeEvent.html"),
    d!("TextureStitchEvent", "getMap", V_18, "texture_stitch_atlas", "TextureAtlasStitchedEvent / RegisterSpriteEvent", "rendering", "event_adapter_required", "Move sprite registration to atlas-specific registration events.", "net/minecraftforge/client/event/TextureStitchEvent.html"),
    d!("MinecraftForge", "EVENT_BUS", V_ALL, "forge_global_event_bus", "NeoForge.EVENT_BUS / mod event bus split", "events", "registration_adapter_required", "Choose NeoForge global game bus versus mod lifecycle bus; event timing differs.", "net/minecraftforge/common/MinecraftForge.html"),
    d!("EventBus", "register", V_18, "event_listener_registration", "IEventBus#addListener / register", "events", "adapter_required", "Move static subscriber registration into the correct mod/game event bus and event package.", "net/minecraftforge/eventbus/api/IEventBus.html"),
    d!("EventBus", "post", V_18, "event_post", "NeoForge.EVENT_BUS#post", "events", "adapter_required", "Preserve cancellation, result and listener ordering.", "net/minecraftforge/eventbus/api/IEventBus.html"),
    d!("FMLCommonHandler", "instance", V_ALL, "fml_common_handler_singleton", "FMLEnvironment / FMLLoader / event bus", "loader", "adapter_required", "Replace broad loader singleton access with the specific environment/service API.", "net/minecraftforge/fml/common/FMLCommonHandler.html"),
    d!("Loader", "isModLoaded", V_ALL, "mod_presence_query", "ModList#isLoaded", "loader", "automatic_rename_candidate", "Preserve optional-mod dependency checks and case-sensitive mod IDs.", "net/minecraftforge/fml/common/Loader.html"),
    d!("Loader", "getModList", V_ALL, "loaded_mod_containers", "ModList#getMods", "loader", "adapter_required", "Translate ModContainer metadata and registry state.", "net/minecraftforge/fml/common/Loader.html"),
    d!("DimensionManager", "registerProviderType", V_ALL, "dimension_provider_registration", "LevelStem / DimensionType / dimension datapack", "dimensions", "registration_adapter_required", "Port dimension type and LevelStem resources; provider classes are not directly registered.", "net/minecraftforge/common/DimensionManager.html"),
    d!("DimensionManager", "registerDimension", V_ALL, "dimension_registration", "LevelStem / dimension datapack", "dimensions", "registration_adapter_required", "Translate numeric dimension IDs to stable namespaced dimension keys and datapack definitions.", "net/minecraftforge/common/DimensionManager.html"),
    d!("DimensionManager", "unregisterDimension", V_ALL, "dimension_unregistration", "Dimension lifecycle / datapack registry", "dimensions", "manual_adapter_required", "Dynamic dimension removal is a server lifecycle operation and not a direct registry deletion.", "net/minecraftforge/common/DimensionManager.html"),

    // Event payloads, commands, packet buffers and client rendering APIs.
    d!("BlockEvent", "getState", V_ALL, "event_block_state", "BlockState on BlockEvent", "events", "adapter_required", "Preserve state at event time and cancellation/result semantics.", "net/minecraftforge/event/world/BlockEvent.html"),
    d!("BlockEvent", "getPos", V_ALL, "event_block_position", "BlockPos on BlockEvent", "events", "automatic_rename_candidate", "Use event position rather than mutable world coordinates.", "net/minecraftforge/event/world/BlockEvent.html"),
    d!("BlockEvent", "getWorld", V_ALL, "event_block_level", "LevelAccessor on BlockEvent", "events", "adapter_required", "Translate World to Level/LevelAccessor and preserve logical side.", "net/minecraftforge/event/world/BlockEvent.html"),
    d!("BlockEvent.BreakEvent", "getPlayer", V_ALL, "block_break_player", "Player on BlockEvent.BreakEvent", "events", "automatic_rename_candidate", "Preserve break actor, cancellation and experience-drop changes.", "net/minecraftforge/event/world/BlockEvent.BreakEvent.html"),
    d!("BlockEvent.BreakEvent", "getExpToDrop", V_ALL, "block_break_xp_read", "Experience amount on block break event", "events", "adapter_required", "Preserve modified XP amount and listener ordering.", "net/minecraftforge/event/world/BlockEvent.BreakEvent.html"),
    d!("BlockEvent.BreakEvent", "setExpToDrop", V_ALL, "block_break_xp_write", "Set block-break experience amount", "events", "adapter_required", "Preserve other listeners' changes and cancellation.", "net/minecraftforge/event/world/BlockEvent.BreakEvent.html"),
    d!("BlockEvent.PlaceEvent", "getPlayer", V_ALL, "block_place_player", "Player on block placement event", "events", "automatic_rename_candidate", "Preserve placer identity and multi-place semantics.", "net/minecraftforge/event/world/BlockEvent.PlaceEvent.html"),
    d!("BlockEvent.NeighborNotifyEvent", "getNotifiedSides", V_18, "neighbor_notify_sides", "Set<Direction> neighbor notification directions", "events", "signature_adapter_required", "Translate EnumFacing to Direction and retain mutable notification-side behavior.", "net/minecraftforge/event/world/BlockEvent.NeighborNotifyEvent.html"),
    d!("PlayerInteractEvent", "getEntityPlayer", V_ALL, "interaction_event_player", "Player on PlayerInteractEvent", "events", "automatic_rename_candidate", "Translate EntityPlayer to Player and preserve hand/side.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.html"),
    d!("PlayerInteractEvent", "getPos", V_ALL, "interaction_event_position", "BlockPos on PlayerInteractEvent", "events", "automatic_rename_candidate", "Preserve clicked position and distinguish entity from block interactions.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.html"),
    d!("PlayerInteractEvent", "getFace", V_ALL, "interaction_event_face", "Direction on PlayerInteractEvent", "events", "signature_adapter_required", "Map EnumFacing to Direction and preserve null face for non-block interactions.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.html"),
    d!("PlayerInteractEvent", "getHand", V_18, "interaction_event_hand", "InteractionHand on PlayerInteractEvent", "events", "automatic_rename_candidate", "Preserve offhand interactions and hand-specific cancellation.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.html"),
    d!("PlayerInteractEvent", "getItemStack", V_ALL, "interaction_event_stack", "ItemStack on PlayerInteractEvent", "events", "automatic_rename_candidate", "Use event-time stack snapshot and preserve count/components.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.html"),
    d!("PlayerInteractEvent.RightClickBlock", "getHitVec", V_18, "right_click_hit_location", "BlockHitResult#getLocation", "events", "signature_adapter_required", "Translate Vec3d hit position into BlockHitResult location.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.RightClickBlock.html"),
    d!("PlayerInteractEvent.RightClickBlock", "getUseBlock", V_112, "right_click_block_result", "InteractionResult event result", "events", "adapter_required", "Preserve DEFAULT/ALLOW/DENY behavior and client prediction.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.RightClickBlock.html"),
    d!("PlayerInteractEvent.RightClickBlock", "setUseBlock", V_112, "right_click_block_result_write", "Set tri-state block-use result", "events", "adapter_required", "Translate tri-state event results rather than a simple boolean.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.RightClickBlock.html"),
    d!("PlayerInteractEvent.RightClickItem", "getUseItem", V_112, "right_click_item_result", "InteractionResult event result", "events", "adapter_required", "Preserve item-use denial independently from block-use result.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.RightClickItem.html"),
    d!("PlayerInteractEvent.LeftClickBlock", "getUseBlock", V_112, "left_click_block_result", "InteractionResult event result", "events", "adapter_required", "Preserve block attack denial and client/server behavior.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.LeftClickBlock.html"),
    d!("PlayerInteractEvent.EntityInteract", "getTarget", V_18, "entity_interaction_target", "Entity on entity-interact event", "events", "automatic_rename_candidate", "Preserve clicked entity identity and target validity.", "net/minecraftforge/event/entity/player/PlayerInteractEvent.EntityInteract.html"),
    d!("WorldEvent", "getWorld", V_ALL, "world_event_level", "LevelAccessor on WorldEvent", "events", "automatic_rename_candidate", "Translate world load/unload lifecycle and ensure correct logical side.", "net/minecraftforge/event/world/WorldEvent.html"),
    d!("ChunkEvent", "getChunk", V_18, "chunk_event_chunk", "ChunkAccess on ChunkEvent", "events", "adapter_required", "Use chunk lifecycle events rather than direct chunk list manipulation.", "net/minecraftforge/event/world/ChunkEvent.html"),
    d!("EntityJoinWorldEvent", "getEntity", V_18, "entity_join_level_entity", "Entity on entity join level event", "events", "automatic_rename_candidate", "Preserve cancellation and server-side entity insertion timing.", "net/minecraftforge/event/entity/EntityJoinWorldEvent.html"),
    d!("EntityJoinWorldEvent", "getWorld", V_18, "entity_join_level", "LevelAccessor on entity join level event", "events", "adapter_required", "Translate World to LevelAccessor and retain spawn side.", "net/minecraftforge/event/entity/EntityJoinWorldEvent.html"),
    d!("EntityJoinWorldEvent", "isCanceled", V_18, "entity_join_canceled", "Cancelable event state", "events", "automatic_rename_candidate", "Preserve cancellation before the entity is inserted.", "net/minecraftforge/event/entity/EntityJoinWorldEvent.html"),
    d!("EntityItemPickupEvent", "getItem", V_ALL, "item_pickup_entity", "ItemEntity on pickup event", "events", "automatic_rename_candidate", "Preserve picked-up stack and pickup delay.", "net/minecraftforge/event/entity/item/EntityItemPickupEvent.html"),
    d!("EntityItemPickupEvent", "getPlayer", V_ALL, "item_pickup_player", "Player on pickup event", "events", "automatic_rename_candidate", "Preserve inventory insertion and canceled pickup behavior.", "net/minecraftforge/event/entity/item/EntityItemPickupEvent.html"),
    d!("ItemTossEvent", "getEntityItem", V_18, "item_toss_entity", "ItemEntity on ItemTossEvent", "events", "automatic_rename_candidate", "The event wraps the spawned item entity; preserve toss cancellation.", "net/minecraftforge/event/entity/item/ItemTossEvent.html"),
    d!("LivingEvent", "getEntityLiving", V_ALL, "living_event_entity", "LivingEntity on LivingEvent", "events", "automatic_rename_candidate", "Translate EntityLivingBase to LivingEntity.", "net/minecraftforge/event/entity/living/LivingEvent.html"),
    d!("LivingAttackEvent", "getSource", V_ALL, "living_attack_damage_source", "DamageSource on LivingAttackEvent", "events", "adapter_required", "Translate source type and preserve attack-stage cancellation.", "net/minecraftforge/event/entity/living/LivingAttackEvent.html"),
    d!("LivingAttackEvent", "getAmount", V_ALL, "living_attack_amount", "Pre-mitigation attack amount", "events", "automatic_rename_candidate", "Preserve pre-armor amount and cancellation.", "net/minecraftforge/event/entity/living/LivingAttackEvent.html"),
    d!("LivingHurtEvent", "getSource", V_ALL, "living_hurt_damage_source", "DamageSource on LivingHurtEvent", "events", "automatic_rename_candidate", "This stage is before later mitigation; preserve event ordering.", "net/minecraftforge/event/entity/living/LivingHurtEvent.html"),
    d!("LivingHurtEvent", "getAmount", V_ALL, "living_hurt_amount", "Current hurt amount", "events", "automatic_rename_candidate", "Preserve amount modifications before armor/resistance processing.", "net/minecraftforge/event/entity/living/LivingHurtEvent.html"),
    d!("LivingHurtEvent", "setAmount", V_ALL, "living_hurt_amount_write", "Set hurt amount", "events", "automatic_rename_candidate", "Preserve event amount modification and cancellation.", "net/minecraftforge/event/entity/living/LivingHurtEvent.html"),
    d!("LivingDamageEvent", "getAmount", V_18, "living_damage_amount", "Post-mitigation damage amount", "events", "automatic_rename_candidate", "This event stage is later than LivingHurtEvent; do not merge them.", "net/minecraftforge/event/entity/living/LivingDamageEvent.html"),
    d!("LivingDeathEvent", "getSource", V_ALL, "living_death_source", "DamageSource on LivingDeathEvent", "events", "automatic_rename_candidate", "Preserve final death source and death cancellation semantics.", "net/minecraftforge/event/entity/living/LivingDeathEvent.html"),
    d!("LivingDropsEvent", "getDrops", V_ALL, "living_death_drops_list", "List<ItemEntity> loot drops", "events", "adapter_required", "Replace mutable EntityItem list editing with modern loot/drop event hooks.", "net/minecraftforge/event/entity/living/LivingDropsEvent.html"),
    d!("LivingDropsEvent", "getLootingLevel", V_ALL, "living_death_looting", "Looting level on LivingDropsEvent", "events", "automatic_rename_candidate", "Preserve looting context in loot table conditions/functions.", "net/minecraftforge/event/entity/living/LivingDropsEvent.html"),
    d!("LivingExperienceDropEvent", "getDroppedExperience", V_18, "living_xp_drop_read", "Experience drop amount", "events", "automatic_rename_candidate", "Preserve player killer and XP amount modifications.", "net/minecraftforge/event/entity/living/LivingExperienceDropEvent.html"),
    d!("LivingExperienceDropEvent", "setDroppedExperience", V_18, "living_xp_drop_write", "Set experience drop amount", "events", "automatic_rename_candidate", "Preserve modified XP and event cancellation.", "net/minecraftforge/event/entity/living/LivingExperienceDropEvent.html"),
    d!("LivingEquipmentChangeEvent", "getSlot", V_112, "equipment_change_slot", "EquipmentSlot", "events", "automatic_rename_candidate", "Translate legacy armor indices to EquipmentSlot.", "net/minecraftforge/event/entity/living/LivingEquipmentChangeEvent.html"),
    d!("LivingEquipmentChangeEvent", "getFrom", V_112, "equipment_change_old_stack", "Previously equipped ItemStack", "events", "automatic_rename_candidate", "Preserve old stack identity and components.", "net/minecraftforge/event/entity/living/LivingEquipmentChangeEvent.html"),
    d!("LivingEquipmentChangeEvent", "getTo", V_112, "equipment_change_new_stack", "Newly equipped ItemStack", "events", "automatic_rename_candidate", "Preserve new stack identity and component changes.", "net/minecraftforge/event/entity/living/LivingEquipmentChangeEvent.html"),
    d!("PlayerEvent", "getPlayer", V_ALL, "player_event_player", "Player on PlayerEvent", "events", "automatic_rename_candidate", "Translate EntityPlayer and preserve server-side identity.", "net/minecraftforge/event/entity/player/PlayerEvent.html"),
    d!("PlayerEvent.BreakSpeed", "getOriginalSpeed", V_18, "player_break_original_speed", "Original block break speed", "events", "automatic_rename_candidate", "Preserve pre-modifier speed and haste/mining-fatigue context.", "net/minecraftforge/event/entity/player/PlayerEvent.BreakSpeed.html"),
    d!("PlayerEvent.BreakSpeed", "getNewSpeed", V_18, "player_break_new_speed", "Current modified break speed", "events", "automatic_rename_candidate", "Preserve event-modified speed and cancellation.", "net/minecraftforge/event/entity/player/PlayerEvent.BreakSpeed.html"),
    d!("PlayerEvent.BreakSpeed", "setNewSpeed", V_18, "player_break_speed_write", "Set modified break speed", "events", "automatic_rename_candidate", "Port speed modifiers without dropping other event listeners' contributions.", "net/minecraftforge/event/entity/player/PlayerEvent.BreakSpeed.html"),
    d!("RegistryEvent.Register", "getRegistry", V_112, "registry_event_registry", "IForgeRegistry<T>", "registration", "adapter_required", "Translate registry-specific events to RegisterEvent/DeferredRegister.", "net/minecraftforge/event/RegistryEvent.Register.html"),
    d!("RegistryEvent.MissingMappings", "getAllMappings", V_112, "missing_registry_mappings", "Missing registry mappings collection", "registration", "manual_adapter_required", "Use modern missing-mapping/remapping event and preserve remap/ignore decisions.", "net/minecraftforge/event/RegistryEvent.MissingMappings.html"),
    d!("IForgeRegistry", "register", V_112, "forge_registry_register", "DeferredRegister / RegisterEvent", "registration", "registration_adapter_required", "Use typed registry keys and deferred registration rather than runtime mutation.", "net/minecraftforge/registries/IForgeRegistry.html"),
    d!("IForgeRegistry", "getValue", V_112, "forge_registry_lookup", "Registry#get(ResourceLocation)", "registration", "automatic_rename_candidate", "Translate registry names to ResourceLocation and handle missing entries.", "net/minecraftforge/registries/IForgeRegistry.html"),
    d!("IForgeRegistry", "getValuesCollection", V_112, "forge_registry_values", "Registry holders and registry iteration", "registration", "adapter_required", "Prefer registry iteration APIs and account for datapack registries.", "net/minecraftforge/registries/IForgeRegistry.html"),
    d!("ICommand", "getCommandName", V_ALL, "command_name", "Brigadier literal command name", "commands", "adapter_required", "Convert ICommand to Brigadier registration and preserve aliases.", "net/minecraft/command/ICommand.html"),
    d!("ICommand", "getCommandUsage", V_ALL, "command_usage", "Component / Brigadier usage metadata", "commands", "adapter_required", "Translate usage keys/strings to localized Components and argument syntax.", "net/minecraft/command/ICommand.html"),
    d!("ICommand", "processCommand", V_ALL, "command_execute", "Brigadier Command#run", "commands", "manual_adapter_required", "Rebuild execution using CommandSourceStack and typed arguments.", "net/minecraft/command/ICommand.html"),
    d!("ICommand", "canCommandSenderUseCommand", V_ALL, "command_permission", "requires predicate on CommandSourceStack", "commands", "adapter_required", "Translate permission levels and sender types into Brigadier requires logic.", "net/minecraft/command/ICommand.html"),
    d!("ICommand", "addTabCompletionOptions", V_ALL, "command_suggestions", "SuggestionProvider<CommandSourceStack>", "commands", "adapter_required", "Rebuild suggestions using Brigadier builders and parsed context.", "net/minecraft/command/ICommand.html"),
    d!("ICommandSender", "getName", V_ALL, "command_sender_name", "CommandSourceStack#getTextName", "commands", "automatic_rename_candidate", "Use Component display name where available.", "net/minecraft/command/ICommandSender.html"),
    d!("ICommandSender", "getEntityWorld", V_ALL, "command_sender_level", "CommandSourceStack#getLevel", "commands", "automatic_rename_candidate", "Resolve the level through command source rather than legacy sender world.", "net/minecraft/command/ICommandSender.html"),
    d!("MinecraftServer", "getPlayerList", V_ALL, "server_player_list", "PlayerList", "server", "automatic_rename_candidate", "Preserve online player filtering and server-thread access.", "net/minecraft/server/MinecraftServer.html"),
    d!("MinecraftServer", "addScheduledTask", V_ALL, "server_thread_enqueue", "MinecraftServer#execute", "threading", "automatic_rename_candidate", "Schedule world mutations on the server thread.", "net/minecraft/server/MinecraftServer.html"),
    d!("PacketBuffer", "writeItemStack", V_ALL, "packet_stack_write", "FriendlyByteBuf#writeItem", "networking", "signature_adapter_required", "Serialize component-bearing ItemStacks using registry-aware buffer methods.", "net/minecraft/network/PacketBuffer.html"),
    d!("PacketBuffer", "readItemStack", V_ALL, "packet_stack_read", "FriendlyByteBuf#readItem", "networking", "signature_adapter_required", "Decode item registry IDs and data components with target protocol semantics.", "net/minecraft/network/PacketBuffer.html"),
    d!("PacketBuffer", "writeBlockPos", V_ALL, "packet_block_pos_write", "FriendlyByteBuf#writeBlockPos", "networking", "automatic_rename_candidate", "Preserve packed coordinate encoding and packet field order.", "net/minecraft/network/PacketBuffer.html"),
    d!("PacketBuffer", "readBlockPos", V_ALL, "packet_block_pos_read", "FriendlyByteBuf#readBlockPos", "networking", "automatic_rename_candidate", "Preserve packed coordinate decoding and validation.", "net/minecraft/network/PacketBuffer.html"),
    d!("PacketBuffer", "writeCompoundTag", V_ALL, "packet_nbt_write", "FriendlyByteBuf#writeNbt", "networking", "automatic_rename_candidate", "Preserve size limits and packet serialization order.", "net/minecraft/network/PacketBuffer.html"),
    d!("PacketBuffer", "readCompoundTag", V_ALL, "packet_nbt_read", "FriendlyByteBuf#readNbt", "networking", "automatic_rename_candidate", "Validate tag sizes and registry-aware nested values.", "net/minecraft/network/PacketBuffer.html"),
    d!("Minecraft", "displayGuiScreen", V_ALL, "client_open_screen", "Minecraft#setScreen", "client_gui", "adapter_required", "Move to client-only screen opening and preserve pending server menu state.", "net/minecraft/client/Minecraft.html"),
    d!("GuiScreen", "initGui", V_ALL, "screen_initialize", "Screen#init", "client_gui", "adapter_required", "Rebuild widgets and layout using modern screen dimensions.", "net/minecraft/client/gui/GuiScreen.html"),
    d!("GuiScreen", "drawScreen", V_ALL, "screen_render", "Screen#render", "client_gui", "manual_adapter_required", "Translate matrix stack, mouse coordinates and render ordering.", "net/minecraft/client/gui/GuiScreen.html"),
    d!("GuiScreen", "mouseClicked", V_ALL, "screen_mouse_click", "Screen#mouseClicked", "client_gui", "signature_adapter_required", "Preserve button, click count, widget dispatch and coordinate scaling.", "net/minecraft/client/gui/GuiScreen.html"),
    d!("GuiScreen", "keyTyped", V_ALL, "screen_key_input", "Screen#keyPressed / Screen#charTyped", "client_gui", "signature_adapter_required", "Separate key codes from typed characters and preserve modifier state.", "net/minecraft/client/gui/GuiScreen.html"),
    d!("KeyBinding", "isPressed", V_ALL, "keybinding_consume_press", "KeyMapping#consumeClick", "client_input", "automatic_rename_candidate", "Modern method consumes queued click presses.", "net/minecraft/client/settings/KeyBinding.html"),
    d!("KeyBinding", "isKeyDown", V_ALL, "keybinding_held_state", "KeyMapping#isDown", "client_input", "automatic_rename_candidate", "Use held-state query rather than consuming press events.", "net/minecraft/client/settings/KeyBinding.html"),
    d!("ModelRenderer", "addBox", V_ALL, "model_cube_add", "ModelPart#addOrReplaceChild / CubeListBuilder#addBox", "models", "signature_adapter_required", "Translate texture offsets, mirror flags and box inflate into modern model geometry.", "net/minecraft/client/model/ModelRenderer.html"),
    d!("ModelRenderer", "setRotationPoint", V_ALL, "model_pivot_set", "ModelPart#setPos", "models", "automatic_rename_candidate", "Translate pivot coordinates and preserve model hierarchy.", "net/minecraft/client/model/ModelRenderer.html"),
    d!("ModelRenderer", "addChild", V_18, "model_child_add", "ModelPart child hierarchy", "models", "adapter_required", "Rebuild parent/child model-part hierarchy and pivot transforms.", "net/minecraft/client/model/ModelRenderer.html"),
    d!("ModelRenderer", "render", V_ALL, "model_part_render", "ModelPart#render", "models", "manual_adapter_required", "Port geometry/animation into model parts and modern renderer layers.", "net/minecraft/client/model/ModelRenderer.html"),
    d!("ModelBase", "render", V_ALL, "entity_model_render", "EntityModel#renderToBuffer", "models", "manual_adapter_required", "Port geometry and animation into model parts and render state.", "net/minecraft/client/model/ModelBase.html"),
    d!("Render", "doRender", V_ALL, "entity_renderer_render", "EntityRenderer#render", "rendering", "manual_adapter_required", "Rebuild renderer state, pose stack, buffers and render layer selection.", "net/minecraft/client/renderer/entity/Render.html"),
    d!("RenderLivingBase", "doRender", V_ALL, "living_renderer_render", "LivingEntityRenderer#render", "rendering", "manual_adapter_required", "Translate model layers, render state extraction and animation setup.", "net/minecraft/client/renderer/entity/RenderLivingBase.html"),
    d!("TileEntitySpecialRenderer", "render", V_ALL, "block_entity_renderer", "BlockEntityRenderer#render", "rendering", "manual_adapter_required", "Move to BlockEntityRenderer with PoseStack, MultiBufferSource and partial tick.", "net/minecraft/client/renderer/tileentity/TileEntitySpecialRenderer.html"),
    d!("ItemMeshDefinition", "getModelLocation", V_18, "item_model_selection", "ItemOverrides / model JSON overrides", "rendering", "resource_adapter_required", "Represent dynamic model choice with item overrides, components or custom renderers.", "net/minecraft/client/renderer/ItemMeshDefinition.html"),
    d!("ModelLoader", "setCustomModelResourceLocation", V_18, "item_model_registration", "ModelEvent.RegisterAdditional / item model resources", "rendering", "resource_adapter_required", "Convert inventory variants to model JSON and modern model registration.", "net/minecraftforge/client/model/ModelLoader.html"),

    // Expanded configuration, AI, GUI, registry, container and server APIs.
    d!("Configuration", "get", V_ALL, "config_property_lookup", "ModConfigSpec.ConfigValue#get", "config", "adapter_required", "Move typed property declarations into ForgeConfigSpec and preserve config type/reload semantics.", "net/minecraftforge/common/config/Configuration.html"),
    d!("Configuration", "getCategory", V_ALL, "config_category_lookup", "ForgeConfigSpec category builder", "config", "adapter_required", "Rebuild category paths and comments in a declarative config spec.", "net/minecraftforge/common/config/Configuration.html"),
    d!("Configuration", "hasChanged", V_ALL, "config_dirty_check", "ModConfigEvent / config spec persistence", "config", "adapter_required", "Modern configs are registered and saved through config lifecycle APIs.", "net/minecraftforge/common/config/Configuration.html"),
    d!("Configuration", "load", V_ALL, "config_load", "ModConfig registration and config loading lifecycle", "config", "manual_adapter_required", "Replace manual load timing with registered ModConfigSpec and lifecycle events.", "net/minecraftforge/common/config/Configuration.html"),
    d!("Configuration", "save", V_ALL, "config_save", "ModConfigSpec save / ModConfigEvent", "config", "manual_adapter_required", "Use config spec persistence and preserve server/client config ownership.", "net/minecraftforge/common/config/Configuration.html"),
    d!("Property", "getBoolean", V_ALL, "config_boolean_read", "BooleanValue#get", "config", "automatic_rename_candidate", "Preserve default values and config validation.", "net/minecraftforge/common/config/Property.html"),
    d!("Property", "getInt", V_ALL, "config_integer_read", "IntValue#get", "config", "automatic_rename_candidate", "Translate min/max bounds and integer validation.", "net/minecraftforge/common/config/Property.html"),
    d!("Property", "getDouble", V_ALL, "config_double_read", "DoubleValue#get", "config", "automatic_rename_candidate", "Preserve floating-point bounds and validation.", "net/minecraftforge/common/config/Property.html"),
    d!("Property", "getString", V_ALL, "config_string_read", "ConfigValue<String>#get", "config", "automatic_rename_candidate", "Preserve allowed values and localization/comment behavior.", "net/minecraftforge/common/config/Property.html"),
    d!("Property", "getStringList", V_ALL, "config_string_list_read", "ConfigValue<List<String>>#get", "config", "adapter_required", "Preserve list ordering and element validation.", "net/minecraftforge/common/config/Property.html"),
    d!("EntityRegistry", "registerModEntity", V_ALL, "entity_type_registration", "DeferredRegister<EntityType<?>> / EntityType.Builder", "registration", "registration_adapter_required", "Translate tracking range, update frequency, velocity updates and entity factory.", "net/minecraftforge/fml/common/registry/EntityRegistry.html"),
    d!("EntityRegistry", "registerGlobalEntityID", V_ALL, "global_entity_id_registration", "EntityType registry registration", "registration", "manual_adapter_required", "Remove numeric global IDs and register a stable ResourceLocation entity type.", "net/minecraftforge/fml/common/registry/EntityRegistry.html"),
    d!("EntityRegistry", "addSpawn", V_ALL, "entity_biome_spawn_registration", "BiomeModifier / biome spawn settings", "worldgen", "registration_adapter_required", "Translate weighted spawn entry, creature category, min/max group size and biome filters.", "net/minecraftforge/fml/common/registry/EntityRegistry.html"),
    d!("EntityRegistry", "removeSpawn", V_ALL, "entity_biome_spawn_removal", "BiomeModifier spawn removal", "worldgen", "registration_adapter_required", "Remove matching biome spawn entries through biome modifier data.", "net/minecraftforge/fml/common/registry/EntityRegistry.html"),
    d!("NetworkRegistry", "newSimpleChannel", V_18, "network_channel_create", "RegisterPayloadHandlersEvent / PayloadRegistrar", "networking", "registration_adapter_required", "Replace channel protocol strings and discriminators with typed payload IDs/codecs.", "net/minecraftforge/fml/common/network/NetworkRegistry.html"),
    d!("NetworkRegistry", "registerGuiHandler", V_ALL, "gui_handler_registration", "MenuType registration / MenuProvider", "menus", "registration_adapter_required", "Replace integer GUI IDs with registered MenuTypes and explicit server/client screen factories.", "net/minecraftforge/fml/common/network/NetworkRegistry.html"),
    d!("IGuiHandler", "getServerGuiElement", V_ALL, "gui_server_menu_factory", "MenuProvider#createMenu / NetworkHooks.openScreen", "menus", "manual_adapter_required", "Build a server-authoritative AbstractContainerMenu and validate the opening context.", "net/minecraftforge/fml/common/network/IGuiHandler.html"),
    d!("IGuiHandler", "getClientGuiElement", V_ALL, "gui_client_screen_factory", "Screen factory / MenuScreens.register", "client_gui", "manual_adapter_required", "Create a client screen bound to the server menu type and synchronized container data.", "net/minecraftforge/fml/common/network/IGuiHandler.html"),
    d!("IForgeRegistryEntry", "setRegistryName", V_112, "registry_name_assignment", "ResourceLocation / DeferredRegister object name", "registration", "property_recovery", "Move names into deferred registry suppliers and preserve namespace/path.", "net/minecraftforge/registries/IForgeRegistryEntry.html"),
    d!("IForgeRegistryEntry", "getRegistryName", V_112, "registry_name_read", "BuiltInRegistries / ResourceKey / ResourceLocation", "registration", "adapter_required", "Use stable ResourceLocation identity rather than runtime registry object state.", "net/minecraftforge/registries/IForgeRegistryEntry.html"),
    d!("ItemBlock", "getBlock", V_ALL, "block_item_block_lookup", "BlockItem#getBlock", "item_data", "automatic_rename_candidate", "Preserve block-item identity and registry pairing.", "net/minecraft/item/ItemBlock.html"),
    d!("ItemBlock", "placeBlockAt", V_ALL, "block_item_placement", "BlockItem#place / BlockPlaceContext", "item_interaction", "adapter_required", "Port state selection, placement flags, block entity initialization and interaction result.", "net/minecraft/item/ItemBlock.html"),
    d!("Enchantment", "canApply", V_ALL, "enchantment_item_predicate", "Enchantment#canEnchant / Enchantment#isPrimaryItem", "enchanting", "adapter_required", "Preserve item type and primary-item semantics; target APIs may split applicability checks.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "canApplyTogether", V_ALL, "enchantment_compatibility", "Enchantment#checkCompatibility", "enchanting", "adapter_required", "Preserve pairwise compatibility and registry holder identity.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "getMinEnchantability", V_ALL, "enchantment_min_power", "Enchantment# getMinCost", "enchanting", "automatic_rename_candidate", "Preserve level-dependent power calculation.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "getMaxEnchantability", V_ALL, "enchantment_max_power", "Enchantment# getMaxCost", "enchanting", "automatic_rename_candidate", "Preserve level-dependent power range and table selection.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "getMaxLevel", V_ALL, "enchantment_max_level", "Enchantment#getMaxLevel", "enchanting", "automatic_rename_candidate", "Preserve maximum level and localized level display.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "getName", V_ALL, "enchantment_translation_key", "Enchantment#getDescriptionId", "localization", "property_recovery", "Migrate the translation key to modern enchantment language resources.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "getTranslatedName", V_ALL, "enchantment_display_name", "Enchantment#getFullname / Component", "localization", "signature_adapter_required", "Return a modern Component and preserve level formatting.", "net/minecraft/enchantment/Enchantment.html"),
    d!("EntityAITasks", "addTask", V_ALL, "ai_goal_add", "GoalSelector#addGoal", "entity_ai", "adapter_required", "Preserve integer priority and mutex/control flags.", "net/minecraft/entity/ai/EntityAITasks.html"),
    d!("EntityAITasks", "removeTask", V_ALL, "ai_goal_remove", "GoalSelector#removeGoal", "entity_ai", "adapter_required", "Map the old goal instance to the registered modern Goal.", "net/minecraft/entity/ai/EntityAITasks.html"),
    d!("EntityAITasks", "onUpdateTasks", V_ALL, "ai_goal_selector_tick", "GoalSelector#tick", "entity_ai", "adapter_required", "Do not manually tick the selector if the Mob lifecycle already does so.", "net/minecraft/entity/ai/EntityAITasks.html"),
    d!("EntitySenses", "canSee", V_ALL, "mob_can_see", "Sensing#hasLineOfSight", "entity_ai", "automatic_rename_candidate", "Preserve cached visibility semantics and target entity.", "net/minecraft/entity/ai/EntitySenses.html"),
    d!("EntityMoveHelper", "setMoveTo", V_ALL, "mob_move_to", "MoveControl#setWantedPosition", "entity_ai", "signature_adapter_required", "Translate speed modifier and wanted position.", "net/minecraft/entity/ai/EntityMoveHelper.html"),
    d!("EntityMoveHelper", "isUpdating", V_ALL, "mob_move_active", "MoveControl#hasWanted", "entity_ai", "automatic_rename_candidate", "Preserve state of an active movement request.", "net/minecraft/entity/ai/EntityMoveHelper.html"),
    d!("EntityMoveHelper", "onUpdateMoveHelper", V_ALL, "mob_move_control_tick", "MoveControl#tick", "entity_ai", "adapter_required", "Let Mob lifecycle tick movement controls once per game tick.", "net/minecraft/entity/ai/EntityMoveHelper.html"),
    d!("EntityLookHelper", "setLookPosition", V_ALL, "mob_look_position", "LookControl#setLookAt", "entity_ai", "signature_adapter_required", "Translate coordinates and yaw/pitch limits.", "net/minecraft/entity/EntityLookHelper.html"),
    d!("EntityLookHelper", "setLookPositionWithEntity", V_ALL, "mob_look_at_entity", "LookControl#setLookAt", "entity_ai", "signature_adapter_required", "Preserve entity eye-height and look-at target context.", "net/minecraft/entity/EntityLookHelper.html"),
    d!("EntityLookHelper", "onUpdateLook", V_ALL, "mob_look_control_tick", "LookControl#tick", "entity_ai", "adapter_required", "Do not tick look control twice from both Mob and a custom AI task.", "net/minecraft/entity/EntityLookHelper.html"),
    d!("PathNavigateGround", "setBreakDoors", V_ALL, "navigation_break_doors", "GroundPathNavigation#setCanBreakDoors", "entity_ai", "automatic_rename_candidate", "Preserve door breaking separately from opening.", "net/minecraft/pathfinding/PathNavigateGround.html"),
    d!("PathNavigateGround", "setEnterDoors", V_ALL, "navigation_open_doors", "GroundPathNavigation#setCanOpenDoors", "entity_ai", "automatic_rename_candidate", "Preserve ability to pass through open doors and open closed doors.", "net/minecraft/pathfinding/PathNavigateGround.html"),
    d!("PathNavigateGround", "setCanSwim", V_ALL, "navigation_swim", "PathNavigation#setCanFloat", "entity_ai", "adapter_required", "Modern float navigation is not identical to all legacy swimming rules.", "net/minecraft/pathfinding/PathNavigateGround.html"),
    d!("Container", "addSlotToContainer", V_ALL, "menu_add_slot", "AbstractContainerMenu#addSlot", "menus", "signature_adapter_required", "Rebuild slot order and container indices consistently with quick-move logic.", "net/minecraft/inventory/Container.html"),
    d!("Container", "getSlot", V_ALL, "menu_slot_lookup", "AbstractContainerMenu#slots", "menus", "automatic_rename_candidate", "Translate source slot indices after rebuilding the target menu.", "net/minecraft/inventory/Container.html"),
    d!("Container", "getSlotFromInventory", V_ALL, "menu_slot_lookup_by_inventory", "AbstractContainerMenu slot mapping", "menus", "adapter_required", "Preserve backing inventory identity, slot order and index mapping.", "net/minecraft/inventory/Container.html"),
    d!("Container", "mergeItemStack", V_ALL, "menu_stack_merge", "AbstractContainerMenu#moveItemStackTo", "menus", "manual_adapter_required", "Translate slot index ranges and reverse direction; preserve merge/extract rules.", "net/minecraft/inventory/Container.html"),
    d!("Slot", "onSlotChanged", V_ALL, "menu_slot_changed", "Slot#setChanged / AbstractContainerMenu#slotsChanged", "menus", "automatic_rename_candidate", "Notify the backing container and recompute derived crafting state.", "net/minecraft/inventory/Slot.html"),
    d!("Slot", "decrStackSize", V_ALL, "menu_slot_extract", "Slot#remove", "menus", "signature_adapter_required", "Preserve amount, slot callbacks and returned remainder.", "net/minecraft/inventory/Slot.html"),
    d!("Slot", "getStack", V_ALL, "menu_slot_stack", "Slot#getItem", "menus", "automatic_rename_candidate", "Translate empty stack and backing inventory access.", "net/minecraft/inventory/Slot.html"),
    d!("WorldServer", "getEntityFromUuid", V_18, "server_entity_uuid_lookup", "ServerLevel#getEntity", "world", "automatic_rename_candidate", "Preserve unloaded entity and dimension behavior.", "net/minecraft/world/WorldServer.html"),
    d!("WorldServer", "spawnEntity", V_18, "server_spawn_entity", "ServerLevel#addFreshEntity", "world", "automatic_rename_candidate", "Respect logical server, spawn reason and entity validity.", "net/minecraft/world/WorldServer.html"),
    d!("ChunkProviderServer", "getLoadedChunk", V_ALL, "loaded_chunk_lookup", "ServerChunkCache#getChunkNow", "chunk", "signature_adapter_required", "Use non-loading lookup rather than forcing a chunk load.", "net/minecraft/world/gen/ChunkProviderServer.html"),
    d!("ChunkProviderServer", "loadChunk", V_ALL, "chunk_load_request", "ServerChunkCache#getChunk", "chunk", "signature_adapter_required", "Preserve force-load semantics and chunk status requirements.", "net/minecraft/world/gen/ChunkProviderServer.html"),
    d!("TileEntityFurnace", "isItemFuel", V_ALL, "furnace_fuel_predicate", "IItemExtension#getBurnTime / fuel data", "fuel", "adapter_required", "Use a stack-aware fuel hook and preserve fuel item tags.", "net/minecraft/tileentity/TileEntityFurnace.html"),
    d!("TileEntityFurnace", "getItemBurnTime", V_ALL, "furnace_fuel_time", "IItemExtension#getBurnTime", "fuel", "adapter_required", "Preserve custom per-stack burn duration and recipe-type context.", "net/minecraft/tileentity/TileEntityFurnace.html"),

    // Concrete item handler, fluid tank and energy storage implementations.
    d!("ItemStackHandler", "getSlots", V_18, "item_handler_slot_count", "IItemHandler#getSlots", "inventory", "automatic_rename_candidate", "Preserve fixed/dynamic slot count and handler resizing behavior.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemStackHandler", "getStackInSlot", V_18, "item_handler_stack_read", "IItemHandler#getStackInSlot", "inventory", "automatic_rename_candidate", "Preserve empty-stack behavior and avoid exposing mutable internal stacks.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemStackHandler", "setStackInSlot", V_18, "item_handler_stack_write", "IItemHandlerModifiable#setStackInSlot", "inventory", "adapter_required", "Preserve validation and slot callback behavior.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemStackHandler", "insertItem", V_18, "item_handler_insert", "IItemHandler#insertItem", "inventory", "automatic_rename_candidate", "Preserve simulation flag, returned remainder and stack limits.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemStackHandler", "extractItem", V_18, "item_handler_extract", "IItemHandler#extractItem", "inventory", "automatic_rename_candidate", "Preserve simulation flag, max amount and slot extraction behavior.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemStackHandler", "getSlotLimit", V_18, "item_handler_slot_limit", "IItemHandler#getSlotLimit", "inventory", "automatic_rename_candidate", "Preserve per-slot stack-size limits.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemStackHandler", "isItemValid", V_18, "item_handler_validity", "IItemHandler#isItemValid", "inventory", "automatic_rename_candidate", "Preserve slot-specific item filters.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemStackHandler", "serializeNBT", V_18, "item_handler_serialize", "ItemStackHandler#serializeNBT / custom component codec", "persistence", "component_adapter_required", "Translate inventory NBT to persistent block-entity data or a typed data component.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemStackHandler", "deserializeNBT", V_18, "item_handler_deserialize", "ItemStackHandler#deserializeNBT / custom component codec", "persistence", "component_adapter_required", "Restore stacks using registry-aware item parsing and validate slot count.", "net/minecraftforge/items/ItemStackHandler.html"),
    d!("ItemHandlerHelper", "insertItem", V_18, "item_handler_helper_insert", "IItemHandler insertion helper", "inventory", "adapter_required", "Preserve stack merging, simulation and returned remainder.", "net/minecraftforge/items/ItemHandlerHelper.html"),
    d!("ItemHandlerHelper", "canItemStacksStack", V_18, "item_stack_merge_predicate", "ItemStack#isSameItemSameComponents", "inventory", "adapter_required", "Modern component equality can differ from legacy item/tag equality.", "net/minecraftforge/items/ItemHandlerHelper.html"),
    d!("ItemHandlerHelper", "copyStackWithSize", V_18, "item_stack_copy_size", "ItemStack#copyWithCount", "inventory", "automatic_rename_candidate", "Preserve item components and count clamping.", "net/minecraftforge/items/ItemHandlerHelper.html"),
    d!("FluidRegistry", "registerFluid", V_18, "fluid_registration", "DeferredRegister<FluidType> / FluidType registration", "fluids", "registration_adapter_required", "Translate fluid and fluid-type registration, textures, attributes and bucket item.", "net/minecraftforge/fluids/FluidRegistry.html"),
    d!("FluidRegistry", "getFluid", V_18, "fluid_lookup", "BuiltInRegistries.FLUID / ResourceLocation holder", "fluids", "automatic_rename_candidate", "Resolve fluid identity from a namespaced registry key.", "net/minecraftforge/fluids/FluidRegistry.html"),
    d!("FluidRegistry", "getFluidName", V_18, "fluid_registry_name", "BuiltInRegistries.FLUID#getKey", "fluids", "adapter_required", "Preserve registry identity rather than display/localized names.", "net/minecraftforge/fluids/FluidRegistry.html"),
    d!("FluidRegistry", "getFluidID", V_18, "fluid_numeric_id", "ResourceLocation / Holder<Fluid>", "fluids", "adapter_required", "Replace numeric runtime fluid IDs with stable namespaced registry keys.", "net/minecraftforge/fluids/FluidRegistry.html"),
    d!("FluidTank", "getFluid", V_18, "fluid_tank_contents", "IFluidHandler#getFluidInTank", "fluids", "automatic_rename_candidate", "Return a safe fluid-stack view/copy and preserve empty tank semantics.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidTank", "getFluidAmount", V_18, "fluid_tank_amount", "FluidStack#getAmount / tank content query", "fluids", "automatic_rename_candidate", "Preserve fluid amount units and empty tank behavior.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidTank", "getCapacity", V_18, "fluid_tank_capacity", "IFluidHandler#getTankCapacity", "fluids", "automatic_rename_candidate", "Preserve tank capacity separately from current fluid amount.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidTank", "isEmpty", V_18, "fluid_tank_empty", "FluidStack#isEmpty / tank contents", "fluids", "automatic_rename_candidate", "Preserve empty fluid stack normalization.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidTank", "setFluid", V_18, "fluid_tank_set_contents", "IFluidHandler implementation / tank state component", "fluids", "adapter_required", "Preserve capacity clamping and change notifications.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidTank", "fill", V_18, "fluid_tank_fill", "IFluidHandler#fill", "fluids", "automatic_rename_candidate", "Preserve FluidAction simulation/execute behavior and capacity checks.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidTank", "drain", V_18, "fluid_tank_drain", "IFluidHandler#drain", "fluids", "automatic_rename_candidate", "Preserve amount-vs-stack matching and simulation semantics.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidTank", "readFromNBT", V_18, "fluid_tank_load", "FluidTank#readFromNBT / registry-aware component codec", "persistence", "adapter_required", "Translate fluid registry IDs and amount data to modern fluid serialization.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidTank", "writeToNBT", V_18, "fluid_tank_save", "FluidTank#writeToNBT / registry-aware component codec", "persistence", "adapter_required", "Serialize fluid identity and amount without relying on unstable numeric IDs.", "net/minecraftforge/fluids/FluidTank.html"),
    d!("FluidContainerRegistry", "registerFluidContainer", V_18, "fluid_container_registration", "Item bucket / item fluid capability / data-driven recipe", "fluids", "registration_adapter_required", "Translate filled/empty containers into modern bucket or custom item fluid behavior.", "net/minecraftforge/fluids/FluidContainerRegistry.html"),
    d!("FluidContainerRegistry", "getFluidForFilledItem", V_18, "fluid_from_container", "IFluidHandlerItem#getFluid", "fluids", "adapter_required", "Preserve item-container extraction and stack remainder semantics.", "net/minecraftforge/fluids/FluidContainerRegistry.html"),
    d!("FluidUtil", "interactWithFluidHandler", V_18, "fluid_item_interaction", "FluidUtil / IFluidHandlerItem transfer operations", "fluids", "adapter_required", "Preserve hand, player, world, sided handler lookup and simulation.", "net/minecraftforge/fluids/FluidUtil.html"),
    d!("FluidUtil", "tryEmptyContainer", V_18, "fluid_container_empty", "FluidUtil.tryEmptyContainer / IFluidHandlerItem", "fluids", "adapter_required", "Preserve simulated transfer, fluid amount, container remainder and sound.", "net/minecraftforge/fluids/FluidUtil.html"),
    d!("FluidUtil", "tryFillContainer", V_18, "fluid_container_fill", "FluidUtil.tryFillContainer / IFluidHandlerItem", "fluids", "adapter_required", "Preserve source tank mutation and filled container output.", "net/minecraftforge/fluids/FluidUtil.html"),
    d!("IFluidTank", "getFluid", V_18, "fluid_tank_interface_contents", "IFluidHandler#getFluidInTank", "fluids", "automatic_rename_candidate", "Expose tank contents through the modern indexed handler API.", "net/minecraftforge/fluids/IFluidTank.html"),
    d!("IFluidTank", "getCapacity", V_18, "fluid_tank_interface_capacity", "IFluidHandler#getTankCapacity", "fluids", "automatic_rename_candidate", "Expose per-tank capacity through the handler contract.", "net/minecraftforge/fluids/IFluidTank.html"),
    d!("EnergyStorage", "receiveEnergy", V_112, "energy_storage_receive", "IEnergyStorage#receiveEnergy", "energy", "automatic_rename_candidate", "Preserve simulation, receive limits and stored/capacity bounds.", "net/minecraftforge/energy/EnergyStorage.html"),
    d!("EnergyStorage", "extractEnergy", V_112, "energy_storage_extract", "IEnergyStorage#extractEnergy", "energy", "automatic_rename_candidate", "Preserve simulation and extraction limits.", "net/minecraftforge/energy/EnergyStorage.html"),
    d!("EnergyStorage", "getEnergyStored", V_112, "energy_storage_read", "IEnergyStorage#getEnergyStored", "energy", "automatic_rename_candidate", "Return current energy rather than maximum capacity.", "net/minecraftforge/energy/EnergyStorage.html"),
    d!("EnergyStorage", "getMaxEnergyStored", V_112, "energy_storage_capacity", "IEnergyStorage#getMaxEnergyStored", "energy", "automatic_rename_candidate", "Return capacity rather than current energy.", "net/minecraftforge/energy/EnergyStorage.html"),
    d!("EnergyStorage", "canExtract", V_112, "energy_storage_can_extract", "IEnergyStorage#canExtract", "energy", "automatic_rename_candidate", "Preserve extraction permission separately from amount.", "net/minecraftforge/energy/EnergyStorage.html"),
    d!("EnergyStorage", "canReceive", V_112, "energy_storage_can_receive", "IEnergyStorage#canReceive", "energy", "automatic_rename_candidate", "Preserve receive permission separately from capacity.", "net/minecraftforge/energy/EnergyStorage.html"),

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
    // Status effects and attribute-driven special abilities.
    d!("Potion", "performEffect", V_ALL, "potion_tick_effect", "MobEffect#applyEffectTick", "abilities", "manual_adapter_required", "Reconstruct per-tick behavior, target predicates, amplifier scaling and server-side execution; custom effects must not be replaced by a similarly named vanilla effect.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "isReady", V_ALL, "potion_tick_schedule", "MobEffect#shouldApplyEffectTickThisTick", "abilities", "adapter_required", "Preserve duration/amplifier tick cadence and the final effect tick.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "affectEntity", V_ALL, "potion_indirect_effect", "MobEffect#applyInstantenousEffect", "abilities", "manual_adapter_required", "Preserve direct source, indirect source, target, amplifier and instant-versus-duration semantics.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "applyAttributesModifiersToEntity", V_ALL, "potion_attribute_apply", "MobEffect#addAttributeModifiers", "abilities", "adapter_required", "Translate attribute UUIDs, operations, amounts and stacking; prevent duplicate modifiers on effect refresh.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "removeAttributesModifiersFromEntity", V_ALL, "potion_attribute_remove", "MobEffect#removeAttributeModifiers", "abilities", "adapter_required", "Remove only modifiers owned by the effect and preserve equipment/other-effect modifiers.", "net/minecraft/potion/Potion.html"),
    d!("PotionEffect", "getIsAmbient", V_ALL, "potion_ambient_flag", "MobEffectInstance#isAmbient", "abilities", "automatic_rename_candidate", "Preserve ambient status separately from particles and icon visibility.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "getIsShowParticles", V_17_18, "potion_particle_visibility", "MobEffectInstance#isVisible", "abilities", "automatic_rename_candidate", "Preserve hidden-particle effects while retaining gameplay behavior.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "getCurativeItems", V_112, "potion_cure_items", "MobEffectInstance#isCurativeItem", "abilities", "adapter_required", "Translate custom curative-item lists and preserve whether milk or other items remove the effect.", "net/minecraft/potion/PotionEffect.html"),
    d!("EntityLivingBase", "getAbsorptionAmount", V_ALL, "absorption_health_read", "LivingEntity#getAbsorptionAmount", "combat", "automatic_rename_candidate", "Keep absorption hearts distinct from health, armor and toughness.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setAbsorptionAmount", V_ALL, "absorption_health_write", "LivingEntity#setAbsorptionAmount", "combat", "automatic_rename_candidate", "Preserve clamping and shield/absorption update behavior.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getActivePotionEffects", V_ALL, "active_effects_enumeration", "LivingEntity#getActiveEffects", "abilities", "automatic_rename_candidate", "Enumerate duration, amplifier, ambient and visibility metadata for every active effect.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isPotionApplicable", V_ALL, "effect_application_predicate", "LivingEntity#canBeAffected", "abilities", "adapter_required", "Preserve immunity, undead/creature-type rules and modded effect rejection.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "curePotionEffects", V_18, "effect_cure", "LivingEntity#removeAllEffects / MobEffectInstance cure checks", "abilities", "adapter_required", "Translate the supplied curative stack and preserve per-effect cure rules.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getRevengeTarget", V_112, "revenge_target_read", "Mob#getLastHurtByMob", "combat_ai", "automatic_rename_candidate", "Preserve revenge-target lifetime separately from the current attack target.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getLastAttackedEntity", V_112, "last_attacked_target_read", "Mob#getLastHurtMob", "combat_ai", "automatic_rename_candidate", "Preserve attacker-side last-hit target separately from the last attacker.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setLastAttackedEntity", V_112, "last_attacked_target_write", "Mob#setLastHurtMob", "combat_ai", "adapter_required", "Maintain combat bookkeeping used by retaliation, achievements and custom AI.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getCombatTracker", V_112, "combat_history_access", "LivingEntity#getCombatTracker", "combat", "adapter_required", "Preserve ordered damage history, fall attribution and death-message context.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setArrowCountInEntity", V_ALL, "embedded_projectile_count", "LivingEntity#setArrowCount", "combat", "adapter_required", "Preserve embedded-arrow count and client synchronization.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getArrowCountInEntity", V_ALL, "embedded_projectile_count_read", "LivingEntity#getArrowCount", "combat", "automatic_rename_candidate", "Expose synchronized embedded-projectile count used by custom bow/projectile effects.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isEntityUndead", V_ALL, "undead_classification", "LivingEntity#getMobType / MobType.UNDEAD", "abilities", "adapter_required", "Preserve undead-specific healing, damage and potion susceptibility.", "net/minecraft/entity/EntityLivingBase.html"),

    // Projectile behavior and custom ability delivery.
    d!("EntityThrowable", "onImpact", V_ALL, "throwable_impact", "ThrowableProjectile#onHit", "projectiles", "manual_adapter_required", "Rebuild block/entity hit dispatch, owner immunity, impact effects, pickup/drop behavior and discard timing.", "net/minecraft/entity/projectile/EntityThrowable.html"),
    d!("EntityThrowable", "getGravityVelocity", V_ALL, "projectile_gravity", "ThrowableProjectile#getGravity", "projectiles", "adapter_required", "Preserve per-tick gravity, drag, water behavior and launch trajectory.", "net/minecraft/entity/projectile/EntityThrowable.html"),
    d!("EntityThrowable", "getVelocity", V_ALL, "projectile_velocity", "Projectile#shoot", "projectiles", "adapter_required", "Translate launch speed and inaccuracy without changing normalized direction or inherited motion.", "net/minecraft/entity/projectile/EntityThrowable.html"),
    d!("EntityArrow", "onHit", V_112, "arrow_hit", "AbstractArrow#onHitEntity / onHitBlock", "projectiles", "manual_adapter_required", "Preserve hit type, damage, critical state, knockback, potion payload, pickup mode and event order.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityArrow", "onCollideWithPlayer", V_ALL, "arrow_pickup", "AbstractArrow#playerTouch", "projectiles", "adapter_required", "Preserve pickup eligibility, creative restrictions, inventory insertion and despawn behavior.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityArrow", "getDamage", V_ALL, "arrow_damage_read", "AbstractArrow#getBaseDamage", "projectiles", "automatic_rename_candidate", "Keep base damage separate from critical-hit bonus and enchantment modifiers.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityArrow", "setDamage", V_ALL, "arrow_damage_write", "AbstractArrow#setBaseDamage", "projectiles", "automatic_rename_candidate", "Preserve floating-point base damage and later critical/enchantment processing.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityArrow", "getKnockbackStrength", V_ALL, "arrow_knockback_read", "AbstractArrow#getKnockback", "projectiles", "automatic_rename_candidate", "Preserve projectile knockback independently from direct melee knockback.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityArrow", "setKnockbackStrength", V_ALL, "arrow_knockback_write", "AbstractArrow#setKnockback", "projectiles", "automatic_rename_candidate", "Preserve knockback strength and target velocity behavior.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityArrow", "setIsCritical", V_ALL, "arrow_critical_write", "AbstractArrow#setCritArrow", "projectiles", "automatic_rename_candidate", "Preserve critical flag and randomized bonus-damage calculation.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityArrow", "getIsCritical", V_ALL, "arrow_critical_read", "AbstractArrow#isCritArrow", "projectiles", "automatic_rename_candidate", "Expose critical projectile state used by custom bows and abilities.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityFireball", "onImpact", V_ALL, "fireball_impact", "Fireball#onHit", "projectiles", "manual_adapter_required", "Preserve explosion/fire effects, direct and indirect source, block collision and impact cancellation.", "net/minecraft/entity/projectile/EntityFireball.html"),
    d!("EntityFishHook", "onUpdate", V_ALL, "fishing_hook_tick", "FishingHook#tick", "projectiles", "manual_adapter_required", "Reconstruct owner state, water physics, bite timing, catch rolls, hooked-entity motion and loot outcomes.", "net/minecraft/entity/projectile/EntityFishHook.html"),

    // Damage-source classification and enchantment-triggered abilities.
    d!("DamageSource", "getDamageType", V_ALL, "damage_type_id", "DamageSource#getMsgId", "combat", "automatic_rename_candidate", "Preserve stable damage identity used by death messages, immunities and custom rules.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "isUnblockable", V_ALL, "damage_bypasses_armor", "DamageSource#isBypassArmor", "combat", "automatic_rename_candidate", "Translate armor bypass independently from magic, resistance and invulnerability bypass.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "isMagicDamage", V_ALL, "damage_magic_flag", "DamageSource#isMagic", "combat", "automatic_rename_candidate", "Preserve magic classification for resistance, enchantment protection and event filters.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "isFireDamage", V_ALL, "damage_fire_flag", "DamageSource#isFire", "combat", "automatic_rename_candidate", "Preserve fire classification for fire resistance and fire-protection calculations.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "isProjectile", V_ALL, "damage_projectile_flag", "DamageSource#isProjectile", "combat", "automatic_rename_candidate", "Preserve projectile classification for protection enchantments and event filters.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "isExplosion", V_ALL, "damage_explosion_flag", "DamageSource#isExplosion", "combat", "automatic_rename_candidate", "Preserve explosion classification for blast protection and knockback.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "getEntity", V_ALL, "damage_indirect_entity", "DamageSource#getEntity", "combat", "adapter_required", "Preserve the responsible living owner separately from the direct projectile or damaging entity.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "getSourceOfDamage", V_ALL, "damage_direct_entity", "DamageSource#getDirectEntity", "combat", "adapter_required", "Preserve the immediate damaging entity separately from its projectile owner or indirect source.", "net/minecraft/util/DamageSource.html"),
    d!("Enchantment", "calcModifierDamage", V_ALL, "enchantment_damage_protection", "EnchantmentHelper#getDamageProtection", "enchantments", "adapter_required", "Translate per-piece protection calculations, damage-source predicates and enchantment stacking caps.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "calcDamageByCreature", V_112, "enchantment_bonus_damage", "EnchantmentHelper#getDamageBonus", "enchantments", "adapter_required", "Preserve creature-type multipliers, level scaling and target classification.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "onEntityDamaged", V_112, "enchantment_post_attack", "EnchantmentHelper#doPostHurtEffects", "enchantments", "manual_adapter_required", "Reconstruct enchantment-triggered effects such as fire, knockback, thorns and modded procs with correct attacker/target roles.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "onUserHurt", V_112, "enchantment_defense_proc", "EnchantmentHelper#doPostDamageEffects", "enchantments", "manual_adapter_required", "Preserve defensive proc triggers, attacker attribution, chance and damage-source filtering.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "isTreasureEnchantment", V_112, "enchantment_treasure_flag", "Enchantment#isTreasureOnly", "enchantments", "automatic_rename_candidate", "Preserve treasure-only acquisition restrictions and loot/enchanting availability.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "isCurse", V_112, "enchantment_curse_flag", "Enchantment#isCurse", "enchantments", "automatic_rename_candidate", "Preserve curse classification, binding/removal restrictions and tooltip presentation.", "net/minecraft/enchantment/Enchantment.html"),

    // AI goal lifecycle and movement controllers are common special-ability hosts.
    d!("EntityAIBase", "continueExecuting", V_ALL, "ai_goal_should_continue", "Goal#canContinueToUse", "ai", "adapter_required", "Preserve continuation conditions separately from initial eligibility.", "net/minecraft/entity/ai/EntityAIBase.html"),

    // Dimension/world generation, model animation and renderer extension points.
    d!("BiomeGenBase", "getSpawningChance", V_ALL, "biome_spawn_chance", "Biome spawn settings / MobSpawnSettings", "worldgen", "adapter_required", "Preserve biome-specific spawn probability and category rules.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("BiomeGenBase", "getRandomWorldGenForTrees", V_ALL, "biome_tree_feature", "BiomeGenerationSettings / placed tree feature", "worldgen", "manual_adapter_required", "Recover selected tree feature, random selection and biome placement constraints.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("BiomeGenBase", "decorate", V_ALL, "biome_decoration", "BiomeGenerationSettings / placed features", "worldgen", "manual_adapter_required", "Recover decoration stage, feature ordering, count/chance and biome/dimension predicates.", "net/minecraft/world/biome/BiomeGenBase.html"),
    d!("WorldGenerator", "generate", V_ALL, "world_generator_feature", "Feature#place / ConfiguredFeature", "worldgen", "manual_adapter_required", "Translate legacy WorldGenerator placement into a configured placed feature with exact block/height/random rules.", "net/minecraft/world/gen/feature/WorldGenerator.html"),
    d!("ChunkProviderGenerate", "populate", V_ALL, "chunk_population", "ChunkGenerator / placed feature generation", "worldgen", "manual_adapter_required", "Reconstruct population phase ordering, chunk seed, neighboring-chunk access and event timing.", "net/minecraft/world/gen/ChunkProviderGenerate.html"),
    d!("ModelBase", "setLivingAnimations", V_ALL, "model_living_animation", "EntityModel#setupAnim", "rendering", "manual_adapter_required", "Recover pose inputs and animation formulas; preserve partial ticks, limb swing, head look and entity state.", "net/minecraft/client/model/ModelBase.html"),
    d!("ModelBase", "setRotationAngles", V_ALL, "model_rotation_angles", "EntityModel#setupAnim", "rendering", "manual_adapter_required", "Translate legacy limb/head parameters into modern setupAnim inputs and preserve trigonometric pose formulas.", "net/minecraft/client/model/ModelBase.html"),
    d!("RenderLivingBase", "applyRotations", V_ALL, "living_render_rotations", "LivingEntityRenderer#setupRotations", "rendering", "adapter_required", "Preserve death, sleeping, riding, upside-down and custom rotation branches.", "net/minecraft/client/renderer/entity/RenderLivingBase.html"),
    d!("RenderLivingBase", "preRenderCallback", V_ALL, "living_render_scale", "LivingEntityRenderer#scale", "rendering", "automatic_rename_candidate", "Preserve per-entity scale and model-space transform order.", "net/minecraft/client/renderer/entity/RenderLivingBase.html"),
    d!("RenderLivingBase", "getEntityTexture", V_ALL, "living_render_texture", "EntityRenderer#getTextureLocation", "rendering", "automatic_rename_candidate", "Preserve entity/variant-dependent texture selection and resource namespace.", "net/minecraft/client/renderer/entity/RenderLivingBase.html"),
    d!("TileEntity", "onChunkUnload", V_ALL, "block_entity_chunk_unload", "BlockEntity#onChunkUnloaded", "lifecycle", "adapter_required", "Release chunk-bound resources without treating chunk unload as permanent removal.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "getRenderBoundingBox", V_18, "block_entity_render_bounds", "BlockEntityRenderer render bounds", "rendering", "adapter_required", "Preserve oversized render bounds and distance-based culling.", "net/minecraft/tileentity/TileEntity.html"),
    d!("TileEntity", "shouldRenderInPass", V_18, "block_entity_render_pass", "BlockEntityRenderer / RenderType selection", "rendering", "adapter_required", "Translate legacy render-pass selection into modern render types and layer routing.", "net/minecraft/tileentity/TileEntity.html"),
    d!("Entity", "isImmuneToFire", V_ALL, "entity_fire_immunity", "Entity#fireImmune", "abilities", "automatic_rename_candidate", "Preserve permanent fire immunity separately from burning state and fire-resistance effects.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "isInvulnerable", V_ALL, "entity_invulnerability", "Entity#isInvulnerableTo", "combat", "adapter_required", "Preserve source-sensitive invulnerability rather than a global boolean.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getEyeHeight", V_ALL, "entity_eye_height", "Entity#getEyeHeight", "entities", "automatic_rename_candidate", "Preserve pose- and entity-dependent eye height for ray tracing and rendering.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getLookVec", V_ALL, "entity_look_vector", "Entity#getLookAngle", "entities", "automatic_rename_candidate", "Preserve partial-tick interpolation and normalized direction.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getPositionEyes", V_ALL, "entity_eye_position", "Entity#getEyePosition", "entities", "automatic_rename_candidate", "Translate partial-tick eye position for ray tracing and ability targeting.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getEntityBoundingBox", V_ALL, "entity_bounds", "Entity#getBoundingBox", "entities", "automatic_rename_candidate", "Preserve current pose dimensions and collision bounds.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "setEntityBoundingBox", V_ALL, "entity_bounds_write", "Entity#setBoundingBox", "entities", "adapter_required", "Preserve dimension recalculation and collision synchronization.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "isRiding", V_ALL, "entity_riding_state", "Entity#getVehicle", "entities", "automatic_rename_candidate", "Preserve passenger/vehicle directionality.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "dismountRidingEntity", V_ALL, "entity_stop_riding", "Entity#stopRiding", "entities", "automatic_rename_candidate", "Preserve vehicle cleanup and passenger position updates.", "net/minecraft/entity/Entity.html"),
    d!("Entity", "getBrightness", V_ALL, "entity_brightness", "Entity#getLightLevelDependentMagicValue", "rendering", "automatic_rename_candidate", "Preserve world-sampled brightness and partial-tick behavior.", "net/minecraft/entity/Entity.html"),
    d!("EntityJoinWorldEvent", "setCanceled", V_ALL, "entity_join_canceled", "EntityJoinLevelEvent#setCanceled", "events", "event_adapter_required", "Preserve cancellation before entity insertion and side effects that occur before posting.", "net/minecraftforge/event/entity/EntityJoinWorldEvent.html"),
    d!("Slot", "isItemValid", V_ALL, "menu_slot_validity", "Slot#mayPlace", "inventory", "automatic_rename_candidate", "Preserve slot-specific item predicates and crafting-result restrictions.", "net/minecraft/inventory/Slot.html"),
    d!("Slot", "canTakeStack", V_ALL, "menu_slot_extract", "Slot#mayPickup", "inventory", "automatic_rename_candidate", "Preserve ownership, lock, game-mode and custom extraction restrictions.", "net/minecraft/inventory/Slot.html"),

    // Exact 1.7.10 EntityLivingBase hooks verified against its method summary.
    d!("EntityLivingBase", "canBreatheUnderwater", V_ALL, "living_underwater_breathing", "LivingEntity#canBreatheUnderwater / air supply", "abilities", "adapter_required", "Preserve entity-specific breathing and air-depletion behavior.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "canEntityBeSeen", V_ALL, "living_line_of_sight", "Sensing#hasLineOfSight", "combat_ai", "adapter_required", "Preserve entity-specific visibility and ray obstruction checks used by ranged attacks and abilities.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getLastAttackerTime", V_17_18, "last_attacker_tick", "LivingEntity#getLastHurtByMobTimestamp", "combat_ai", "automatic_rename_candidate", "Preserve tick timestamp semantics for retaliation windows and target memory.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getLook", V_ALL, "living_look_vector", "Entity#getLookAngle", "entities", "adapter_required", "Translate partial-tick interpolation and preserve normalized direction for aiming/ray tracing.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getRNG", V_ALL, "living_random_source", "Entity#getRandom", "entities", "adapter_required", "Preserve entity-seeded random behavior where deterministic per-entity outcomes matter.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getRotationYawHead", V_ALL, "living_head_yaw_read", "LivingEntity#getYHeadRot", "rendering", "automatic_rename_candidate", "Preserve head yaw separately from body yaw for look and animation logic.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getSwingProgress", V_ALL, "living_swing_progress", "LivingEntity#getAttackAnim", "combat", "adapter_required", "Preserve partial-tick interpolation for attack animation and timing-sensitive effects.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getTeam", V_ALL, "living_team_read", "Entity#getTeam", "combat_ai", "automatic_rename_candidate", "Preserve scoreboard team membership used by target filtering and friendly-fire rules.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "handleHealthUpdate", V_17_18, "living_health_status_sync", "LivingEntity#handleEntityEvent", "networking", "adapter_required", "Translate legacy status bytes into entity event IDs and preserve client-side animation/sound effects.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isOnSameTeam", V_ALL, "living_team_predicate", "Entity#isAlliedTo", "combat_ai", "adapter_required", "Preserve team alliance semantics rather than comparing display names.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isOnTeam", V_17_18, "living_team_membership", "Entity#isAlliedTo / Team membership", "combat_ai", "adapter_required", "Preserve team membership and scoreboard collision rules.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isPlayerSleeping", V_ALL, "living_sleep_state", "Player#isSleeping", "player_abilities", "automatic_rename_candidate", "Preserve sleeping state for bed, time-skip and dimension transition behavior.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isMovementBlocked", V_ALL, "living_movement_blocked", "LivingEntity#isImmobile", "abilities", "adapter_required", "Preserve movement locks from death, sleep, stun or other state gates.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "onChangedPotionEffect", V_ALL, "potion_effect_changed_hook", "LivingEntity#onEffectUpdated", "abilities", "adapter_required", "Preserve attribute modifier refresh and synchronized potion metadata after an effect changes.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "onDeathUpdate", V_ALL, "living_death_tick", "LivingEntity#tickDeath", "lifecycle", "adapter_required", "Preserve death animation duration, removal timing and loot/drop ordering.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "onFinishedPotionEffect", V_ALL, "potion_effect_removed_hook", "LivingEntity#onEffectRemoved", "abilities", "adapter_required", "Remove effect-owned attributes and preserve expiry/cure/removal distinction.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "onNewPotionEffect", V_ALL, "potion_effect_added_hook", "LivingEntity#onEffectAdded", "abilities", "adapter_required", "Apply effect-owned attributes and synchronize initial effect metadata exactly once.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "performHurtAnimation", V_ALL, "living_hurt_animation", "LivingEntity#animateHurt", "combat", "adapter_required", "Preserve hurt animation state independently from damage acceptance and health changes.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "rayTrace", V_17_18, "living_ray_trace", "Entity#pick / Level#clip", "combat_ai", "adapter_required", "Translate reach, partial tick and block/entity hit selection for ability targeting.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "removePotionEffectClient", V_17_18, "client_effect_remove", "LivingEntity#removeEffectNoUpdate / effect synchronization", "abilities", "adapter_required", "Preserve client-only visual removal without changing authoritative server effect state.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "renderBrokenItemStack", V_ALL, "equipment_break_feedback", "LivingEntity#broadcastBreakEvent", "combat", "adapter_required", "Preserve break animation, status packet and hand/armor slot identity.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setJumping", V_ALL, "living_jump_state", "LivingEntity#setJumping", "movement", "automatic_rename_candidate", "Preserve jump input state used by AI, movement and animation.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setRotationYawHead", V_ALL, "living_head_yaw_write", "LivingEntity#setYHeadRot", "rendering", "automatic_rename_candidate", "Preserve head rotation separately from body rotation and interpolation.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "updateAITasks", V_17_18, "living_ai_task_update", "Mob#serverAiStep / custom AI goal lifecycle", "ai", "manual_adapter_required", "Rebuild server AI scheduling, target selection, navigation and custom task ordering.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "updateAITick", V_17_18, "living_ai_tick", "Mob#aiStep", "ai", "manual_adapter_required", "Preserve per-tick AI timing, movement controls, attack cadence and ability cooldowns.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "updateArmSwingProgress", V_ALL, "living_arm_swing_tick", "LivingEntity#updateSwingTime", "combat", "adapter_required", "Preserve swing progression and animation state used by attack effects.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "updateEntityActionState", V_ALL, "living_action_state_tick", "Mob#aiStep / travel", "ai", "manual_adapter_required", "Translate legacy action-state updates into AI, navigation and movement phases without changing order.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "updatePotionEffects", V_ALL, "potion_effect_tick", "LivingEntity#tickEffects", "abilities", "adapter_required", "Preserve effect duration decrement, periodic ticks, attribute updates, expiration and synchronization.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getRevengeTimer", V_112, "revenge_target_timestamp", "Mob#getLastHurtByMobTimestamp", "combat_ai", "automatic_rename_candidate", "Preserve timestamp changes that drive retaliation goals.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getLastAttackedEntityTime", V_112, "last_attack_timestamp", "Mob#getLastHurtMobTimestamp", "combat_ai", "automatic_rename_candidate", "Preserve target attack timestamp for AI memory and repeated-hit conditions.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getActivePotionMap", V_112, "active_effect_map", "LivingEntity#getActiveEffects", "abilities", "adapter_required", "Preserve effect-to-instance mapping and avoid flattening distinct effects with different amplifiers.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "removeActivePotionEffect", V_112, "active_effect_remove", "LivingEntity#removeEffect", "abilities", "adapter_required", "Preserve the removed effect instance and effect-removal callbacks.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "areAllPotionsAmbient", V_112, "active_effect_ambient_summary", "LivingEntity#areAllEffectsAmbient", "abilities", "automatic_rename_candidate", "Preserve aggregate ambient state for potion HUD and entity metadata.", "net/minecraft/entity/EntityLivingBase.html"),

    // Exact projectile and damage-source controls from Forge JavaDocs.
    d!("EntityThrowable", "getThrower", V_ALL, "projectile_owner_read", "Projectile#getOwner", "projectiles", "automatic_rename_candidate", "Preserve owner attribution for self-hit immunity, damage credit and projectile effects.", "net/minecraft/entity/projectile/EntityThrowable.html"),
    d!("EntityThrowable", "setThrowableHeading", V_ALL, "projectile_heading", "Projectile#shoot", "projectiles", "adapter_required", "Preserve normalized direction, velocity, inaccuracy and random spread.", "net/minecraft/entity/projectile/EntityThrowable.html"),
    d!("EntityThrowable", "setVelocity", V_ALL, "projectile_velocity_write", "Entity#setDeltaMovement", "projectiles", "adapter_required", "Preserve inherited motion and server/client velocity synchronization.", "net/minecraft/entity/projectile/EntityThrowable.html"),
    d!("EntityArrow", "setEnchantmentEffectsFromEntity", V_112, "arrow_enchantment_payload", "AbstractArrow#setEnchantmentEffectsFromEntity", "projectiles", "adapter_required", "Preserve bow enchantment effects such as power, punch and flame when constructing custom arrows.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("EntityArrow", "shoot", V_112, "arrow_launch", "Projectile#shoot", "projectiles", "adapter_required", "Preserve shooter orientation, pitch offsets, velocity, inaccuracy and random spread.", "net/minecraft/entity/projectile/EntityArrow.html"),
    d!("DamageSource", "canHarmInCreative", V_ALL, "damage_creative_allowed", "DamageSource#isCreativePlayer / player damage policy", "combat", "adapter_required", "Preserve creative-mode damage eligibility and source-specific exceptions.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "isDamageAbsolute", V_ALL, "damage_absolute_flag", "DamageSource#isBypassArmor / damage type tags", "combat", "adapter_required", "Preserve absolute damage separately from armor bypass and difficulty scaling.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "isDifficultyScaled", V_ALL, "damage_difficulty_scaled", "DamageSource#scalesWithDifficulty", "combat", "automatic_rename_candidate", "Preserve difficulty scaling for mob/player damage sources.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setDamageAllowedInCreativeMode", V_ALL, "damage_creative_flag_write", "DamageSource#isCreativePlayer / damage source policy", "combat", "adapter_required", "Preserve explicit creative damage permission.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setDamageBypassesArmor", V_ALL, "damage_armor_bypass_write", "DamageSource#bypassArmor", "combat", "automatic_rename_candidate", "Preserve armor bypass separately from magic and resistance bypass.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setDamageIsAbsolute", V_ALL, "damage_absolute_flag_write", "DamageSource#bypassInvul", "combat", "adapter_required", "Preserve absolute damage behavior without collapsing it into generic unblockable damage.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setDifficultyScaled", V_ALL, "damage_difficulty_flag_write", "DamageSource#scalesWithDifficulty", "combat", "automatic_rename_candidate", "Preserve difficulty-dependent damage scaling.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setExplosion", V_ALL, "damage_explosion_flag_write", "DamageSource#isExplosion", "combat", "automatic_rename_candidate", "Preserve explosion classification for blast protection and knockback.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setExplosionSource", V_ALL, "damage_explosion_source_write", "DamageSource#explosion", "combat", "adapter_required", "Preserve explosion instance, source entity and attribution for damage/knockback.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setFireDamage", V_ALL, "damage_fire_flag_write", "DamageSource#isFire", "combat", "automatic_rename_candidate", "Preserve fire classification for fire resistance and fire-protection calculations.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setMagicDamage", V_ALL, "damage_magic_flag_write", "DamageSource#isMagic", "combat", "automatic_rename_candidate", "Preserve magic classification for resistance and enchantment protection.", "net/minecraft/util/DamageSource.html"),
    d!("DamageSource", "setProjectile", V_ALL, "damage_projectile_flag_write", "DamageSource#isProjectile", "combat", "automatic_rename_candidate", "Preserve projectile classification for protection enchantments and event filters.", "net/minecraft/util/DamageSource.html"),
    d!("Enchantment", "isAllowedOnBooks", V_112, "enchantment_book_eligibility", "Enchantment#isAllowedOnBooks", "enchantments", "automatic_rename_candidate", "Preserve whether the enchantment can be obtained through books/table paths.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "getMinLevel", V_112, "enchantment_min_level", "Enchantment#getMinLevel", "enchantments", "automatic_rename_candidate", "Preserve minimum legal level independently from maximum level.", "net/minecraft/enchantment/Enchantment.html"),
    d!("Enchantment", "setName", V_ALL, "enchantment_name_write", "Enchantment#descriptionId / registry name", "enchantments", "adapter_required", "Preserve localization identity separately from registry identity.", "net/minecraft/enchantment/Enchantment.html"),

    // PotionEffect instance lifecycle and persistence (1.7.10 and 1.12.2 APIs).
    d!("PotionEffect", "performEffect", V_ALL, "potion_instance_tick", "MobEffectInstance tick / MobEffect#applyEffectTick", "abilities", "adapter_required", "Preserve instance duration/amplifier and ensure the effect executes only on its scheduled ticks.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "getEffectName", V_ALL, "potion_effect_translation_key", "MobEffect#getDescriptionId", "abilities", "automatic_rename_candidate", "Preserve effect localization key rather than hardcoding translated display text.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "getIsPotionDurationMax", V_ALL, "potion_effect_infinite_duration", "MobEffectInstance#isInfiniteDuration", "abilities", "adapter_required", "Preserve infinite-duration state separately from large finite durations.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "setPotionDurationMax", V_ALL, "potion_effect_duration_max_write", "MobEffectInstance infinite-duration state", "abilities", "adapter_required", "Preserve infinite-duration toggle independently from remaining duration.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "setSplashPotion", V_17_18, "potion_effect_splash_flag", "ThrownPotion payload / effect application context", "abilities", "adapter_required", "Preserve splash-origin context for custom effects and their impact behavior.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "readCustomPotionEffectFromNBT", V_ALL, "potion_effect_nbt_read", "MobEffectInstance#load", "persistence", "adapter_required", "Translate effect registry identity, duration, amplifier, ambient and visibility fields from NBT.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "writeCustomPotionEffectToNBT", V_ALL, "potion_effect_nbt_write", "MobEffectInstance#save", "persistence", "adapter_required", "Preserve all effect state and custom effect metadata in serialized NBT.", "net/minecraft/potion/PotionEffect.html"),

    // Potion registry, attribute and display hooks (especially relevant to custom effects).
    d!("Potion", "getEffectiveness", V_17_18, "potion_effectiveness", "MobEffect attribute/effect scaling", "abilities", "adapter_required", "Preserve custom effect strength scaling; do not treat it as duration or amplifier.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getId", V_17_18, "potion_legacy_numeric_id", "MobEffect registry key", "abilities", "adapter_required", "Convert numeric IDs to stable registry keys and preserve legacy lookup compatibility.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getLiquidColor", V_ALL, "potion_liquid_color", "MobEffect#getColor", "rendering", "automatic_rename_candidate", "Preserve potion particle, bottle and HUD tint.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getStatusIconIndex", V_ALL, "potion_status_icon_index", "MobEffect icon / MobEffectInstance display", "rendering", "adapter_required", "Preserve custom status icon selection and map it to modern effect icon resources.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "hasStatusIcon", V_ALL, "potion_has_status_icon", "MobEffect display/icon metadata", "rendering", "adapter_required", "Preserve effects that intentionally have no icon.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "isBadEffect", V_ALL, "potion_harmful_classification", "MobEffect beneficial/harmful classification", "abilities", "adapter_required", "Preserve harmful classification separately from actual gameplay effect and beneficial flag.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "isInstant", V_ALL, "potion_instant_classification", "MobEffect#isInstantenous", "abilities", "automatic_rename_candidate", "Preserve instant application semantics versus duration-based ticking.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "isUsable", V_17_18, "potion_usable_flag", "MobEffect availability/registry state", "abilities", "adapter_required", "Preserve whether an effect is selectable/applicable through potion brewing and item paths.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getDurationString", V_17_18, "potion_duration_format", "MobEffectInstance duration display", "rendering", "adapter_required", "Preserve duration formatting and duration-factor behavior for custom HUD/tooltips.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getPotionDurationString", V_112, "potion_duration_format", "MobEffectUtil formatDuration", "rendering", "adapter_required", "Preserve duration formatting and duration-factor behavior for custom HUD/tooltips.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "setIconIndex", V_ALL, "potion_icon_registration", "MobEffect icon resource metadata", "rendering", "adapter_required", "Map legacy atlas coordinates to modern resource locations without changing custom icon selection.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "setPotionName", V_ALL, "potion_translation_name", "MobEffect description ID", "abilities", "adapter_required", "Preserve localization key and namespaced effect identity.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getCurativeItems", V_112, "potion_curative_items", "MobEffectInstance curative item predicates", "abilities", "adapter_required", "Preserve custom cure-item defaults for every newly created effect instance.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getAttributeModifierMap", V_112, "potion_attribute_modifier_map", "MobEffect attribute modifiers", "attributes", "adapter_required", "Translate attribute keys, modifier UUIDs, amounts and operations while retaining effect-owned modifiers.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getAttributeModifierAmount", V_112, "potion_attribute_modifier_amount", "MobEffect attribute modifier amount", "attributes", "adapter_required", "Preserve amplifier-dependent modifier amount calculation.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "registerPotionAttributeModifier", V_112, "potion_attribute_modifier_register", "MobEffect#addAttributeModifier", "attributes", "adapter_required", "Preserve modifier UUID, operation and amplifier scaling.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "shouldRender", V_112, "potion_inventory_visibility", "MobEffectInstance showIcon / HUD display policy", "rendering", "adapter_required", "Preserve inventory visibility independently from HUD visibility and particle visibility.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "shouldRenderHUD", V_112, "potion_hud_visibility", "MobEffectInstance showIcon / HUD display policy", "rendering", "adapter_required", "Preserve HUD visibility independently from inventory visibility.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "shouldRenderInvText", V_112, "potion_inventory_text_visibility", "MobEffect tooltip/display policy", "rendering", "adapter_required", "Preserve whether standard effect name/duration text is shown.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "renderHUDEffect", V_112, "potion_custom_hud_render", "MobEffect HUD overlay rendering", "rendering", "manual_adapter_required", "Rebuild custom effect HUD rendering using modern GUI graphics and preserve icon/alpha/blink behavior.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "renderInventoryEffect", V_112, "potion_custom_inventory_render", "MobEffect inventory overlay rendering", "rendering", "manual_adapter_required", "Rebuild custom inventory effect rendering and preserve icon/text layout.", "net/minecraft/potion/Potion.html"),

    // 1.12 living-entity hand/item state: shields, bows, charged attacks and ability triggers.
    d!("EntityLivingBase", "getActiveHand", V_112, "living_active_hand_read", "LivingEntity#getUsedItemHand", "player_abilities", "automatic_rename_candidate", "Preserve main/offhand selection for active item use and custom ability dispatch.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getActiveItemStack", V_112, "living_active_stack_read", "LivingEntity#getUseItem", "player_abilities", "automatic_rename_candidate", "Preserve active stack snapshot while an item is being used.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getItemInUseCount", V_112, "living_item_use_ticks", "LivingEntity#getUseItemRemainingTicks", "player_abilities", "automatic_rename_candidate", "Preserve remaining use ticks for bows, food, shields and channelled abilities.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getItemInUseMaxCount", V_112, "living_item_use_max_ticks", "Item#getUseDuration", "player_abilities", "adapter_required", "Preserve item-defined maximum use duration separately from remaining ticks.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getHeldItemMainhand", V_112, "living_mainhand_stack", "LivingEntity#getMainHandItem", "inventory", "automatic_rename_candidate", "Preserve main-hand equipment and stack tags for item-triggered abilities.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getHeldItemOffhand", V_112, "living_offhand_stack", "LivingEntity#getOffhandItem", "inventory", "automatic_rename_candidate", "Preserve off-hand equipment and stack tags for shield/dual-wield abilities.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getItemStackFromSlot", V_112, "living_equipment_slot_read", "LivingEntity#getItemBySlot", "inventory", "automatic_rename_candidate", "Resolve equipment by armor/hand slot while preserving slot identity.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setItemStackToSlot", V_112, "living_equipment_slot_write", "LivingEntity#setItemSlot", "inventory", "adapter_required", "Preserve equipment replacement callbacks, attribute updates and break events.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setHeldItem", V_112, "living_hand_stack_write", "LivingEntity#setItemInHand", "inventory", "adapter_required", "Preserve active-use interruption and equipment-change side effects.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "setActiveHand", V_112, "living_start_item_use", "LivingEntity#startUsingItem", "player_abilities", "adapter_required", "Preserve active hand, use animation, item-use duration and server/client synchronization.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "stopActiveHand", V_112, "living_stop_item_use", "LivingEntity#releaseUsingItem", "player_abilities", "adapter_required", "Preserve release callbacks, charged projectile behavior and use cancellation.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "resetActiveHand", V_112, "living_reset_item_use", "LivingEntity#stopUsingItem", "player_abilities", "adapter_required", "Clear use state without accidentally invoking release behavior twice.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isHandActive", V_112, "living_hand_active", "LivingEntity#isUsingItem", "player_abilities", "automatic_rename_candidate", "Preserve whether an item-use action is currently active.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "isActiveItemStackBlocking", V_112, "living_blocking_state", "LivingEntity#isBlocking", "combat", "automatic_rename_candidate", "Preserve shield/blocking state separately from merely holding a shield.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "updateActiveHand", V_112, "living_active_hand_tick", "LivingEntity#updatingUsingItem", "player_abilities", "adapter_required", "Preserve item-use tick progression and interruption conditions.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "updateItemUse", V_112, "living_item_use_tick", "LivingEntity#tickUsingItem", "player_abilities", "adapter_required", "Preserve item-use tick cadence, particles and server/client behavior.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "onItemUseFinish", V_112, "living_item_use_finish", "LivingEntity#completeUsingItem", "player_abilities", "manual_adapter_required", "Preserve consume completion, remainder stacks, item callbacks and triggered ability effects.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "canBeHitWithPotion", V_112, "living_potion_hit_eligibility", "LivingEntity#canBeAffected", "abilities", "adapter_required", "Preserve entity-specific eligibility for splash/lingering potion effects.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "attemptTeleport", V_112, "living_teleport_attempt", "Entity#randomTeleport / teleportTo", "abilities", "adapter_required", "Preserve collision checks, world border, passenger handling and teleport event semantics.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getTicksElytraFlying", V_112, "living_gliding_ticks", "LivingEntity#fallFlyingTicks", "movement", "automatic_rename_candidate", "Preserve elapsed gliding ticks for durability, speed and flight ability formulas.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "sendEnterCombat", V_112, "living_combat_enter", "CombatTracker#recordDamage / combat event packet", "combat", "adapter_required", "Preserve combat tracker transition and client combat event synchronization.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "sendEndCombat", V_112, "living_combat_exit", "CombatTracker#recheckStatus / combat event packet", "combat", "adapter_required", "Preserve combat exit transition and damage history finalization.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getLastDamageSource", V_112, "living_last_damage_source", "LivingEntity#getLastDamageSource", "combat", "automatic_rename_candidate", "Preserve last accepted damage source for retaliation, kill credit and conditional abilities.", "net/minecraft/entity/EntityLivingBase.html"),
    d!("EntityLivingBase", "getPrimaryHand", V_112, "living_primary_hand", "LivingEntity#getMainArm", "player_abilities", "automatic_rename_candidate", "Preserve dominant hand for animations, equipment and offhand logic.", "net/minecraft/entity/EntityLivingBase.html"),

    // Remaining documented PotionEffect helpers and curative-item customization.
    d!("PotionEffect", "getPotion", V_112, "potion_effect_registry_object", "MobEffectInstance#getEffect", "abilities", "automatic_rename_candidate", "Preserve the actual registered Potion object instead of relying on legacy numeric IDs.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "doesShowParticles", V_112, "potion_effect_particle_visibility", "MobEffectInstance#isVisible", "rendering", "automatic_rename_candidate", "Preserve particle visibility independently from HUD icon visibility.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "isCurativeItem", V_112, "potion_effect_cure_predicate", "MobEffectInstance#isCurativeItem", "abilities", "automatic_rename_candidate", "Preserve exact stack/tag matching against this effect's configured curative-item list.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "setCurativeItems", V_112, "potion_effect_cure_items_write", "MobEffectInstance#setCurativeItems", "abilities", "adapter_required", "Replace the complete cure list without losing custom item tags or per-effect rules.", "net/minecraft/potion/PotionEffect.html"),
    d!("PotionEffect", "addCurativeItem", V_112, "potion_effect_cure_item_add", "MobEffectInstance#addCurativeItem", "abilities", "automatic_rename_candidate", "Append a curative item while retaining the existing cure list.", "net/minecraft/potion/PotionEffect.html"),
    d!("Potion", "getPotionById", V_112, "potion_registry_lookup_numeric", "ForgeRegistries.POTIONS / Registry lookup", "registration", "adapter_required", "Resolve legacy numeric IDs through a compatibility table and then query the registered effect.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getIdFromPotion", V_112, "potion_registry_numeric_id", "Potion registry key / compatibility ID table", "registration", "adapter_required", "Avoid using numeric potion IDs as modern persistent identity.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getPotionFromResourceLocation", V_112, "potion_registry_lookup_key", "ForgeRegistries.POTIONS#getValue", "registration", "automatic_rename_candidate", "Resolve namespaced effect keys through the Forge registry.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "getGuiSortColor", V_112, "potion_gui_sort_color", "MobEffectInstance display sorting", "rendering", "adapter_required", "Preserve custom potion effect ordering in inventory and status effect displays.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "isBeneficial", V_112, "potion_beneficial_flag", "MobEffect beneficial classification", "abilities", "automatic_rename_candidate", "Preserve beneficial classification independently from harmful status and effect behavior.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "setBeneficial", V_112, "potion_beneficial_flag_write", "MobEffect beneficial classification", "abilities", "adapter_required", "Preserve beneficial metadata used by effect sorting and UI.", "net/minecraft/potion/Potion.html"),
    d!("Potion", "registerPotions", V_112, "potion_registry_bootstrap", "DeferredRegister<MobEffect> / registry lifecycle", "registration", "registration_adapter_required", "Move effect registration to the registry event lifecycle and preserve registration order/dependencies.", "net/minecraft/potion/Potion.html"),

    // EntityLiving: mob targeting, spawning, leash, equipment and AI control.
    d!("EntityLiving", "canAttackClass", V_ALL, "mob_attack_class_filter", "Mob#canAttack", "combat_ai", "Preserve class-based target filtering for custom mob abilities and friendly/hostile entity rules.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "canBeSteered", V_ALL, "mob_steering_eligibility", "Mob#canBeControlledByRider", "entities", "adapter_required", "Preserve rider-control eligibility and entity-specific steering conditions.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "canDespawn", V_ALL, "mob_despawn_predicate", "Mob#removeWhenFarAway", "lifecycle", "adapter_required", "Preserve distance, persistence, name-tag and mod-specific despawn conditions.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "canPickUpLoot", V_ALL, "mob_loot_pickup_enabled", "Mob#canPickUpLoot", "inventory", "automatic_rename_candidate", "Preserve whether a mob can collect equipment and item entities.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "clearLeashed", V_ALL, "mob_unleash", "Mob#dropLeash / leash removal", "entities", "adapter_required", "Preserve lead-drop choice, detach notification and leash-holder cleanup.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "despawnEntity", V_ALL, "mob_despawn_tick", "Mob#checkDespawn", "lifecycle", "adapter_required", "Preserve despawn distance checks, persistence and player proximity conditions.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "eatGrassBonus", V_ALL, "mob_grass_eating_effect", "Sheep/Animal grass eating behavior", "abilities", "manual_adapter_required", "Reconstruct block replacement, growth/age effects, sound and particles; this is a gameplay hook, not just animation.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "enablePersistence", V_112, "mob_persistence_enable", "Mob#removeWhenFarAway / persistenceRequired", "lifecycle", "automatic_rename_candidate", "Prevent natural despawn while preserving normal death and chunk-unload behavior.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "faceEntity", V_ALL, "mob_face_entity", "LookControl#setLookAt", "ai", "adapter_required", "Preserve maximum yaw/pitch change and per-tick look-controller interpolation.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getJumpHelper", V_ALL, "mob_jump_controller", "JumpControl", "ai", "adapter_required", "Preserve queued jump requests and the controller's per-tick execution.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getLeashed", V_ALL, "mob_leashed_state", "Mob#isLeashed", "entities", "automatic_rename_candidate", "Preserve leash state separately from the leash-holder entity reference.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getLeashedToEntity", V_17_18, "mob_leash_holder", "Mob#getLeashHolder", "entities", "automatic_rename_candidate", "Translate the legacy holder entity reference and preserve leash synchronization.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getLeashHolder", V_112, "mob_leash_holder", "Mob#getLeashHolder", "entities", "automatic_rename_candidate", "Preserve holder identity and leash attachment state.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getLootTable", V_112, "mob_loot_table", "Mob#getLootTable", "loot", "automatic_rename_candidate", "Preserve registry-backed loot table identity and seeded loot behavior.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getMaxSpawnedInChunk", V_ALL, "mob_chunk_spawn_cap", "Mob#getMaxSpawnClusterSize", "worldgen", "adapter_required", "Preserve per-mob local density cap and chunk spawn accounting.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getPathPriority", V_112, "mob_path_node_priority_read", "Mob#getPathfindingMalus", "ai", "automatic_rename_candidate", "Preserve per-PathNodeType navigation cost values.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getRenderSizeModifier", V_ALL, "mob_render_scale", "LivingEntityRenderer#scale", "rendering", "adapter_required", "Preserve per-entity model scaling independently from hitbox dimensions.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getTalkInterval", V_ALL, "mob_ambient_sound_interval", "Mob#getAmbientSoundInterval", "audio", "automatic_rename_candidate", "Preserve random ambient sound cadence and tick scheduling.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "getVerticalFaceSpeed", V_ALL, "mob_vertical_look_speed", "LookControl pitch limit", "ai", "adapter_required", "Preserve vertical look rotation limits for flying and unusual mobs.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "isAIDisabled", V_112, "mob_ai_disabled_state", "Mob#isNoAi", "ai", "automatic_rename_candidate", "Preserve persistent no-AI state and its interaction with goals/navigation.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "isLeftHanded", V_112, "mob_dominant_hand", "LivingEntity#isLeftHanded", "entities", "automatic_rename_candidate", "Preserve dominant hand for equipment, animation and attack behavior.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "isNotColliding", V_112, "mob_spawn_collision_check", "Entity#isFree", "worldgen", "adapter_required", "Preserve collision and entity-overlap checks used during spawn/teleport placement.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "onSpawnWithEgg", V_17_18, "mob_initial_spawn", "Mob#finalizeSpawn", "worldgen", "manual_adapter_required", "Translate spawn group data, difficulty context, equipment rolls and inherited variant data.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "processInitialInteract", V_112, "mob_player_interaction", "Mob#mobInteract", "item_interaction", "adapter_required", "Preserve hand, player, stack mutation and typed interaction result.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setCanPickUpLoot", V_ALL, "mob_loot_pickup_write", "Mob#setCanPickUpLoot", "inventory", "automatic_rename_candidate", "Preserve runtime changes to mob equipment pickup behavior.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setDropChance", V_112, "mob_equipment_drop_chance", "Mob#setDropChance", "loot", "adapter_required", "Preserve per-slot drop chance and equipment slot identity.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setLeashHolder", V_112, "mob_leash_holder_write", "Mob#setLeashedTo", "entities", "adapter_required", "Preserve attach notification, holder tracking and lead behavior.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setLeftHanded", V_112, "mob_dominant_hand_write", "LivingEntity#setLeftHanded", "entities", "automatic_rename_candidate", "Preserve dominant-hand state and synchronized metadata.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setMoveForward", V_112, "mob_forward_input", "LivingEntity travel input", "movement", "adapter_required", "Preserve forward movement input used by custom AI and movement logic.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setMoveStrafing", V_112, "mob_strafe_input", "LivingEntity travel input", "movement", "adapter_required", "Preserve lateral movement input independently from forward movement.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setMoveVertical", V_112, "mob_vertical_input", "LivingEntity travel input", "movement", "adapter_required", "Preserve vertical movement input for flying/swimming AI.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setNoAI", V_112, "mob_ai_disabled_write", "Mob#setNoAi", "ai", "automatic_rename_candidate", "Preserve no-AI state while retaining entity physics and non-AI interactions.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "setPathPriority", V_112, "mob_path_node_priority_write", "Mob#setPathfindingMalus", "ai", "adapter_required", "Preserve per-node-type pathfinding malus values.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "updateEquipmentIfNeeded", V_112, "mob_equipment_pickup_update", "Mob#pickUpItem / equipment replacement", "inventory", "adapter_required", "Preserve pickup priority, equipment comparison, drop chances and item-entity removal.", "net/minecraft/entity/EntityLiving.html"),
    d!("EntityLiving", "updateLeashedState", V_ALL, "mob_leash_tick", "Mob#tickLeash", "entities", "adapter_required", "Preserve leash distance checks, holder movement and break/detach behavior.", "net/minecraft/entity/EntityLiving.html"),

    // Player interaction, cooldown, game mode and ability state.
    d!("EntityPlayer", "canAttackPlayer", V_ALL, "player_pvp_permission", "Player#canHarmPlayer", "combat", "adapter_required", "Preserve team, game-mode and server PVP restrictions.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "canHarvestBlock", V_ALL, "player_harvest_permission", "Player#hasCorrectToolForDrops / BlockState#requiresCorrectToolForDrops", "tool", "adapter_required", "Preserve tool tier, block harvest level and adventure-mode restrictions.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "clearItemInUse", V_17_18, "player_clear_item_use", "LivingEntity#stopUsingItem", "player_abilities", "adapter_required", "Clear active use state without invoking release callbacks twice.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "clonePlayer", V_ALL, "player_clone_state", "PlayerEvent.Clone / Player#restoreFrom", "lifecycle", "manual_adapter_required", "Preserve death-vs-dimension-change flag, inventory, capabilities, persistent data and modded progression.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "closeScreen", V_ALL, "player_close_menu", "Player#closeContainer", "menus", "automatic_rename_candidate", "Preserve menu close packets, container removal and server/client synchronization.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "destroyCurrentEquippedItem", V_17_18, "player_destroy_held_item", "ItemStack#shrink / equipment slot update", "inventory", "adapter_required", "Preserve selected hand, break event and inventory slot mutation.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "dropOneItem", V_17_18, "player_drop_single_item", "Player#drop / ItemTossEvent", "inventory", "adapter_required", "Preserve single-item versus full-stack drop, thrower attribution and cancellation.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "dropPlayerItemWithRandomChoice", V_17_18, "player_drop_stack", "Player#drop / ItemTossEvent", "inventory", "adapter_required", "Preserve random throw offset, full-stack behavior and item toss event.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getCurrentArmor", V_17_18, "player_armor_slot_read", "LivingEntity#getItemBySlot", "inventory", "adapter_required", "Translate legacy armor slot indices to EntityEquipmentSlot and preserve empty-stack behavior.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getCurrentEquippedItem", V_17_18, "player_held_stack_read", "Player#getMainHandItem", "inventory", "automatic_rename_candidate", "Preserve selected hotbar stack and active-hand context.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getCurrentPlayerStrVsBlock", V_17_18, "player_block_mining_speed", "Player#getDestroySpeed", "tool", "adapter_required", "Preserve block state, tool efficiency, haste, fatigue, underwater and on-ground modifiers.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getInventoryEnderChest", V_ALL, "player_ender_chest", "Player#getEnderChestInventory", "inventory", "automatic_rename_candidate", "Preserve player-owned ender chest inventory and its persistence.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getItemInUse", V_17_18, "player_active_item_stack", "LivingEntity#getUseItem", "player_abilities", "automatic_rename_candidate", "Preserve active-use stack while charging, blocking or consuming.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getItemInUseDuration", V_17_18, "player_item_use_elapsed", "LivingEntity#getTicksUsingItem", "player_abilities", "adapter_required", "Preserve elapsed use duration separately from remaining use ticks.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getScore", V_ALL, "player_score_read", "Player#getScore", "progression", "automatic_rename_candidate", "Preserve scoreboard score independently from XP and statistics.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getSleepTimer", V_ALL, "player_sleep_progress", "Player#getSleepTimer", "player_abilities", "automatic_rename_candidate", "Preserve sleep animation/time-skip progress.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isBlocking", V_17_18, "player_blocking_state", "LivingEntity#isBlocking", "combat", "adapter_required", "Preserve active shield/blocking state and source-specific block checks.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isCurrentToolAdventureModeExempt", V_17_18, "player_adventure_tool_exemption", "Player#hasCorrectToolForDrops / adventure mode checks", "tool", "adapter_required", "Preserve adventure-mode block/tool exemption rules.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isPlayerFullyAsleep", V_112, "player_fully_asleep", "Player#isSleeping / sleep timer", "player_abilities", "adapter_required", "Preserve sleep completion threshold separately from merely being in bed.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isSpawnForced", V_112, "player_forced_respawn", "ServerPlayer respawn data / forced spawn", "player_abilities", "adapter_required", "Preserve forced-spawn semantics per dimension.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isUsingItem", V_17_18, "player_item_use_active", "LivingEntity#isUsingItem", "player_abilities", "automatic_rename_candidate", "Preserve active use state for bow charge, food, shields and custom channelled abilities.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "onCriticalHit", V_ALL, "player_critical_hit_feedback", "ServerPlayer#crit / entity event", "combat", "adapter_required", "Preserve critical-hit visual/effect event separately from the damage calculation.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "onEnchantmentCritical", V_ALL, "player_enchantment_critical_feedback", "ServerPlayer#magicCrit / entity event", "combat", "adapter_required", "Preserve enchantment critical-hit feedback and proc timing.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "onKillEntity", V_ALL, "player_kill_entity_hook", "Player#killedEntity / LivingDeathEvent", "combat", "event_adapter_required", "Preserve kill credit, statistics, loot context and modded kill-triggered abilities.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "respawnPlayer", V_ALL, "player_respawn", "PlayerList#respawn / PlayerRespawnEvent", "lifecycle", "manual_adapter_required", "Preserve bed/anchor position, dimension, inventory, abilities and clone-event ordering.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "sendPlayerAbilities", V_ALL, "player_abilities_sync", "ServerPlayer#onUpdateAbilities / ClientboundPlayerAbilitiesPacket", "networking", "adapter_required", "Synchronize may-fly, flying, creative/invulnerable and movement speed flags.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "setCurrentItemOrArmor", V_17_18, "player_equipment_slot_write", "LivingEntity#setItemSlot", "inventory", "adapter_required", "Translate legacy slot indices and preserve equipment attribute/callback updates.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "setGameType", V_ALL, "player_game_mode_write", "PlayerList#setGameMode / ServerPlayer game mode", "player_abilities", "adapter_required", "Preserve permission checks, ability synchronization and game-mode change events.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "setItemInUse", V_17_18, "player_start_item_use", "LivingEntity#startUsingItem", "player_abilities", "adapter_required", "Preserve selected hand, item use duration, active stack and start-use synchronization.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "setScore", V_ALL, "player_score_write", "Player#setScore", "progression", "automatic_rename_candidate", "Preserve scoreboard score updates and team/scoreboard ownership.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "setSpawnChunk", V_112, "player_respawn_location_write", "ServerPlayer#setRespawnPosition", "player_abilities", "adapter_required", "Preserve dimension-aware respawn position and forced flag.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "setSpawnDimension", V_112, "player_respawn_dimension_write", "ServerPlayer respawn dimension data", "player_abilities", "adapter_required", "Preserve dimension-specific respawn routing.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "shouldHeal", V_ALL, "player_auto_heal_predicate", "Player#isHurt / food healing logic", "player_abilities", "adapter_required", "Preserve health and food conditions controlling natural regeneration.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "sleepInBedAt", V_17_18, "player_sleep_attempt", "Player#startSleepInBed", "player_abilities", "adapter_required", "Preserve bed position, dimension, nearby hostile checks and sleep result states.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "stopUsingItem", V_17_18, "player_stop_item_use", "LivingEntity#releaseUsingItem / stopUsingItem", "player_abilities", "adapter_required", "Preserve item release callback, charge duration and use-state cleanup.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "triggerAchievement", V_17_18, "player_achievement_trigger", "Player#awardStat / CriteriaTriggers", "progression", "adapter_required", "Translate achievement/stat identity to modern statistics and advancement criteria.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "wakeUpPlayer", V_ALL, "player_wake_up", "Player#stopSleepInBed", "player_abilities", "adapter_required", "Preserve immediate flag, time-skip behavior and spawn-point update semantics.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "xpBarCap", V_ALL, "player_xp_bar_cap", "Player#getXpNeededForNextLevel", "progression", "automatic_rename_candidate", "Preserve XP curve and level-dependent progress denominator.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getCooldownPeriod", V_112, "player_attack_cooldown_period", "Player#getCurrentItemAttackStrengthDelay", "combat", "adapter_required", "Preserve attack-speed attribute and cooldown period calculation.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getCooldownTracker", V_112, "player_item_cooldown_tracker", "Player#getCooldowns", "player_abilities", "automatic_rename_candidate", "Preserve per-item cooldown state and client synchronization.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getCooledAttackStrength", V_112, "player_cooled_attack_strength", "Player#getAttackStrengthScale", "combat", "adapter_required", "Preserve partial-tick cooldown interpolation for attack damage and sweeping behavior.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getDigSpeed", V_112, "player_block_mining_speed", "Player#getDestroySpeed", "tool", "adapter_required", "Preserve block state and position dependent mining speed.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getLuck", V_112, "player_luck_attribute", "Player#getLuck", "attributes", "automatic_rename_candidate", "Preserve luck attribute used by loot tables and fishing outcomes.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "getSpawnDimension", V_112, "player_respawn_dimension_read", "ServerPlayer respawn dimension data", "player_abilities", "automatic_rename_candidate", "Preserve dimension associated with the player's stored respawn position.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "hasSpawnDimension", V_112, "player_respawn_dimension_present", "ServerPlayer respawn data", "player_abilities", "automatic_rename_candidate", "Distinguish absent respawn dimension from overworld dimension ID.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "interactOn", V_112, "player_entity_interaction", "Player#interactOn", "item_interaction", "adapter_required", "Preserve hand, item stack, target and typed interaction result.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isAllowEdit", V_112, "player_world_edit_permission", "Player#mayBuild / adventure restrictions", "player_abilities", "adapter_required", "Preserve build permission independently from command permission and game mode.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isUser", V_112, "player_local_user_flag", "LocalPlayer / ServerPlayer distinction", "player_abilities", "adapter_required", "Preserve client-local user checks and avoid treating all server players as local.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "isWearing", V_112, "player_model_part_visibility", "PlayerModelPart / player skin customization", "rendering", "automatic_rename_candidate", "Preserve skin customization flags for cape, sleeves and model layers.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "resetCooldown", V_112, "player_attack_cooldown_reset", "Player#resetAttackStrengthTicker", "combat", "automatic_rename_candidate", "Reset attack cooldown at the same interaction point as the source behavior.", "net/minecraft/entity/player/EntityPlayer.html"),
    d!("EntityPlayer", "trySleep", V_112, "player_sleep_attempt", "Player#startSleepInBed", "player_abilities", "adapter_required", "Preserve sleep result enum, bed position, dimension and hostile-nearby checks.", "net/minecraft/entity/player/EntityPlayer.html"),

    // PlayerCapabilities: movement permissions and persistent ability state.
    d!("PlayerCapabilities", "getFlySpeed", V_ALL, "player_flight_speed_read", "Abilities#getFlyingSpeed", "player_abilities", "automatic_rename_candidate", "Preserve configured flight speed separately from the current flying flag.", "net/minecraft/entity/player/PlayerCapabilities.html"),
    d!("PlayerCapabilities", "getWalkSpeed", V_112, "player_walk_speed_read", "Abilities#getWalkingSpeed", "player_abilities", "automatic_rename_candidate", "Preserve configured walking speed and server-side ability synchronization.", "net/minecraft/entity/player/PlayerCapabilities.html"),
    d!("PlayerCapabilities", "setPlayerWalkSpeed", V_112, "player_walk_speed_write", "Abilities#setWalkingSpeed", "player_abilities", "adapter_required", "Preserve walking speed changes and ability packet updates.", "net/minecraft/entity/player/PlayerCapabilities.html"),
    d!("PlayerCapabilities", "readCapabilitiesFromNBT", V_ALL, "player_abilities_nbt_read", "Abilities / Player persistent data", "persistence", "adapter_required", "Preserve creative, invulnerability, flight permission, flying state, edit permission and movement speeds.", "net/minecraft/entity/player/PlayerCapabilities.html"),
    d!("PlayerCapabilities", "writeCapabilitiesToNBT", V_ALL, "player_abilities_nbt_write", "Abilities / Player persistent data", "persistence", "adapter_required", "Serialize all capability flags and movement speeds without conflating permission and active state.", "net/minecraft/entity/player/PlayerCapabilities.html"),

    // FoodStats: exhaustion, saturation and food-state persistence.
    d!("FoodStats", "addExhaustion", V_ALL, "food_exhaustion_add", "FoodData#addExhaustion", "food", "automatic_rename_candidate", "Preserve exhaustion accumulation, saturation depletion and server tick order.", "net/minecraft/util/FoodStats.html"),
    d!("FoodStats", "setFoodLevel", V_ALL, "food_level_write", "FoodData#setFoodLevel", "food", "automatic_rename_candidate", "Preserve clamping and food-level update behavior.", "net/minecraft/util/FoodStats.html"),
    d!("FoodStats", "setFoodSaturationLevel", V_ALL, "food_saturation_write", "FoodData#setSaturation", "food", "automatic_rename_candidate", "Preserve saturation clamping and interaction with exhaustion.", "net/minecraft/util/FoodStats.html"),
    d!("FoodStats", "onUpdate", V_ALL, "food_tick", "FoodData#tick", "food", "adapter_required", "Preserve regeneration, starvation, difficulty and exhaustion/saturation thresholds.", "net/minecraft/util/FoodStats.html"),
    d!("FoodStats", "readNBT", V_ALL, "food_state_nbt_read", "FoodData#readAdditionalSaveData", "persistence", "adapter_required", "Preserve food level, saturation, exhaustion and timer state.", "net/minecraft/util/FoodStats.html"),
    d!("FoodStats", "writeNBT", V_ALL, "food_state_nbt_write", "FoodData#addAdditionalSaveData", "persistence", "adapter_required", "Serialize food level, saturation, exhaustion and timer state.", "net/minecraft/util/FoodStats.html"),

    // Item cooldowns: per-item ability lockouts and client feedback.
    d!("CooldownTracker", "hasCooldown", V_112, "item_cooldown_active", "ItemCooldowns#isOnCooldown", "player_abilities", "automatic_rename_candidate", "Preserve per-item cooldown lookup and item identity.", "net/minecraft/util/CooldownTracker.html"),
    d!("CooldownTracker", "getCooldown", V_112, "item_cooldown_fraction", "ItemCooldowns#getCooldownPercent", "player_abilities", "adapter_required", "Preserve partial-tick interpolation for cooldown overlays and ability timing.", "net/minecraft/util/CooldownTracker.html"),
    d!("CooldownTracker", "tick", V_112, "item_cooldown_tick", "ItemCooldowns#tick", "player_abilities", "adapter_required", "Preserve per-tick expiration, callback timing and synchronization.", "net/minecraft/util/CooldownTracker.html"),
    d!("CooldownTracker", "setCooldown", V_112, "item_cooldown_set", "ItemCooldowns#addCooldown", "player_abilities", "adapter_required", "Preserve duration in ticks and replacement behavior for existing cooldowns.", "net/minecraft/util/CooldownTracker.html"),
    d!("CooldownTracker", "removeCooldown", V_112, "item_cooldown_remove", "ItemCooldowns#removeCooldown", "player_abilities", "automatic_rename_candidate", "Remove the matching item's cooldown and preserve removal notification behavior.", "net/minecraft/util/CooldownTracker.html"),
    d!("CooldownTracker", "notifyOnSet", V_112, "item_cooldown_started_hook", "ItemCooldowns#onCooldownStarted", "player_abilities", "adapter_required", "Preserve client HUD feedback and packet/event timing when a cooldown begins.", "net/minecraft/util/CooldownTracker.html"),
    d!("CooldownTracker", "notifyOnRemove", V_112, "item_cooldown_ended_hook", "ItemCooldowns#onCooldownEnded", "player_abilities", "adapter_required", "Preserve client HUD feedback and packet/event timing when a cooldown ends.", "net/minecraft/util/CooldownTracker.html"),

    // Common Forge Event contract. Cancellation, results and phase are semantic
    // behavior; they cannot be reduced to merely "listener called".
    d!("Event", "getPhase", V_ALL, "event_phase_read", "NeoForge event bus phase / event dispatch phase", "events", "adapter_required", "Preserve event dispatch phase and ordering; do not conflate it with tick START/END phase.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("Event", "setPhase", V_ALL, "event_phase_write", "NeoForge event dispatch phase", "events", "manual_adapter_required", "Only migrate when the target event lifecycle exposes equivalent phase control; preserve listener ordering and bus invariants.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("Event", "hasResult", V_ALL, "event_has_result", "Event result/cancellation contract", "events", "automatic_rename_candidate", "Distinguish events with DEFAULT/ALLOW/DENY results from cancelable and notification-only events.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("Event", "getResult", V_ALL, "event_result_read", "Event result / interaction decision", "events", "adapter_required", "Translate DEFAULT/ALLOW/DENY explicitly; these values are not interchangeable with canceled state.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("Event", "setResult", V_ALL, "event_result_write", "Event result / interaction decision", "events", "adapter_required", "Preserve result precedence and whether the event is result-bearing; do not substitute generic cancellation.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("Event", "isCancelable", V_ALL, "event_cancelable_predicate", "Event cancellation contract", "events", "automatic_rename_candidate", "Preserve which event types can be canceled before attempting to map cancellation logic.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("Event", "isCanceled", V_ALL, "event_canceled_read", "Cancelable event state / cancellation result", "events", "automatic_rename_candidate", "Read cancellation independently from ALLOW/DENY result state.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("Event", "setCanceled", V_ALL, "event_cancel_write", "Cancelable event cancellation", "events", "event_adapter_required", "Preserve cancellation timing and downstream listener behavior; reject calls on noncancelable events.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("Event", "getListenerList", V_ALL, "event_listener_list", "EventBus subscription dispatch", "events", "manual_review", "Low-level listener-list access is coupled to Forge event internals and should be retained as an explicit manual conversion.", "net/minecraftforge/fml/common/eventhandler/Event.html"),
    d!("EventBus", "unregister", V_18, "event_listener_unregister", "IEventBus#unregister / listener registration lifecycle", "events", "adapter_required", "Preserve object identity and unregister timing; remove only the requested listener instance.", "net/minecraftforge/fml/common/eventhandler/EventBus.html"),

    // Living entity events: damage, healing, falling, target selection and use lifecycle.
    d!("LivingFallEvent", "getDistance", V_18, "living_fall_distance", "LivingFallEvent fall distance / fall-damage calculation", "events_combat", "automatic_rename_candidate", "Preserve fall distance before damage calculation; custom reductions must not be converted into generic damage cancellation.", "net/minecraftforge/event/entity/living/LivingFallEvent.html"),
    d!("LivingFallEvent", "setDistance", V_18, "living_fall_distance_write", "LivingFallEvent fall-distance adjustment", "events_combat", "adapter_required", "Translate modified fall distance before the damage multiplier is applied.", "net/minecraftforge/event/entity/living/LivingFallEvent.html"),
    d!("LivingFallEvent", "getDamageMultiplier", V_18, "living_fall_damage_multiplier", "LivingFallEvent damage multiplier / fall damage calculation", "events_combat", "automatic_rename_candidate", "Preserve multiplicative fall-damage scaling independently from fall distance.", "net/minecraftforge/event/entity/living/LivingFallEvent.html"),
    d!("LivingFallEvent", "setDamageMultiplier", V_18, "living_fall_damage_multiplier_write", "LivingFallEvent damage multiplier / fall damage calculation", "events_combat", "adapter_required", "Preserve zero, fractional and amplified multipliers, including feather-fall-like custom behavior.", "net/minecraftforge/event/entity/living/LivingFallEvent.html"),
    d!("LivingHealEvent", "getAmount", V_18, "living_heal_amount", "LivingHealEvent heal amount / LivingEntity healing", "events_combat", "automatic_rename_candidate", "This is requested healing before it is applied; do not merge with health-after or damage events.", "net/minecraftforge/event/entity/living/LivingHealEvent.html"),
    d!("LivingHealEvent", "setAmount", V_18, "living_heal_amount_write", "LivingHealEvent heal amount mutation", "events_combat", "adapter_required", "Preserve event cancellation and amount mutation order for regeneration, lifesteal and custom healing.", "net/minecraftforge/event/entity/living/LivingHealEvent.html"),
    d!("LivingSetAttackTargetEvent", "getTarget", V_18, "living_attack_target_event", "Mob#setTarget / target-selection event", "events_ai", "automatic_rename_candidate", "Preserve the new target value at target assignment time; null target clears must remain distinguishable.", "net/minecraftforge/event/entity/living/LivingSetAttackTargetEvent.html"),
    d!("LivingPackSizeEvent", "getMaxPackSize", V_18, "living_spawn_pack_size", "MobSpawnSettings / spawn group size", "worldgen", "automatic_rename_candidate", "Preserve the maximum group size used by the current spawn attempt.", "net/minecraftforge/event/entity/living/LivingPackSizeEvent.html"),
    d!("LivingPackSizeEvent", "setMaxPackSize", V_18, "living_spawn_pack_size_write", "MobSpawnSettings / spawn group size", "worldgen", "adapter_required", "Translate the event result and mutable group limit; preserve spawn-attempt-local scope.", "net/minecraftforge/event/entity/living/LivingPackSizeEvent.html"),
    d!("LivingSpawnEvent", "getWorld", V_18, "living_spawn_world", "SpawnPlacementCheck / ServerLevel spawn context", "worldgen", "automatic_rename_candidate", "Preserve the exact world and logical side associated with a spawn attempt.", "net/minecraftforge/event/entity/living/LivingSpawnEvent.html"),
    d!("LivingSpawnEvent", "getX", V_18, "living_spawn_x", "SpawnPlacementCheck block position", "worldgen", "automatic_rename_candidate", "Preserve fractional spawn coordinates and their block-position conversion rules.", "net/minecraftforge/event/entity/living/LivingSpawnEvent.html"),
    d!("LivingSpawnEvent", "getY", V_18, "living_spawn_y", "SpawnPlacementCheck block position", "worldgen", "automatic_rename_candidate", "Preserve spawn height and dimension build-height checks.", "net/minecraftforge/event/entity/living/LivingSpawnEvent.html"),
    d!("LivingSpawnEvent", "getZ", V_18, "living_spawn_z", "SpawnPlacementCheck block position", "worldgen", "automatic_rename_candidate", "Preserve fractional spawn coordinates and their block-position conversion rules.", "net/minecraftforge/event/entity/living/LivingSpawnEvent.html"),
    d!("LivingSpawnEvent.CheckSpawn", "isSpawner", V_18, "living_spawn_from_spawner", "MobSpawnType.SPAWNER / spawn reason", "worldgen", "automatic_rename_candidate", "Preserve spawner-origin context because natural-spawn restrictions may differ.", "net/minecraftforge/event/entity/living/LivingSpawnEvent.CheckSpawn.html"),
    d!("LivingSpawnEvent.CheckSpawn", "getSpawner", V_112, "living_spawn_spawner_context", "MobSpawnType.SPAWNER / spawner context", "worldgen", "adapter_required", "Retain the actual spawner controller when present; natural spawn attempts have no spawner instance.", "net/minecraftforge/event/entity/living/LivingSpawnEvent.CheckSpawn.html"),
    d!("LivingEntityUseItemEvent", "getItem", V_112, "living_use_event_item", "LivingEntity item-use context / ItemStack", "events_item_use", "automatic_rename_candidate", "Preserve the pre-use item copy and distinguish it from the post-consumption result stack.", "net/minecraftforge/event/entity/living/LivingEntityUseItemEvent.html"),
    d!("LivingEntityUseItemEvent", "getDuration", V_112, "living_use_event_duration", "LivingEntity item-use remaining duration", "events_item_use", "automatic_rename_candidate", "Preserve duration units and whether the hook observes initial, remaining or elapsed use time.", "net/minecraftforge/event/entity/living/LivingEntityUseItemEvent.html"),
    d!("LivingEntityUseItemEvent", "setDuration", V_112, "living_use_event_duration_write", "LivingEntity item-use duration adjustment", "events_item_use", "adapter_required", "Preserve event-specific duration changes; changing remaining use time can alter completion and tick events.", "net/minecraftforge/event/entity/living/LivingEntityUseItemEvent.html"),

    // Ender teleport and mob spawn events.
    d!("EnderTeleportEvent", "getTargetX", V_18, "teleport_target_x", "Entity#randomTeleport / teleport event", "events_entity", "automatic_rename_candidate", "Preserve the proposed target coordinates before world-border/collision checks.", "net/minecraftforge/event/entity/living/EnderTeleportEvent.html"),
    d!("EnderTeleportEvent", "setTargetX", V_18, "teleport_target_x_write", "Entity#randomTeleport / teleport event", "events_entity", "adapter_required", "Apply X mutation before teleport validation; preserve event cancellation.", "net/minecraftforge/event/entity/living/EnderTeleportEvent.html"),
    d!("EnderTeleportEvent", "getTargetY", V_18, "teleport_target_y", "Entity#randomTeleport / teleport event", "events_entity", "automatic_rename_candidate", "Preserve the proposed vertical destination before collision and floor checks.", "net/minecraftforge/event/entity/living/EnderTeleportEvent.html"),
    d!("EnderTeleportEvent", "setTargetY", V_18, "teleport_target_y_write", "Entity#randomTeleport / teleport event", "events_entity", "adapter_required", "Apply Y mutation before collision and floor validation.", "net/minecraftforge/event/entity/living/EnderTeleportEvent.html"),
    d!("EnderTeleportEvent", "getTargetZ", V_18, "teleport_target_z", "Entity#randomTeleport / teleport event", "events_entity", "automatic_rename_candidate", "Preserve the proposed target coordinates before world-border/collision checks.", "net/minecraftforge/event/entity/living/EnderTeleportEvent.html"),
    d!("EnderTeleportEvent", "setTargetZ", V_18, "teleport_target_z_write", "Entity#randomTeleport / teleport event", "events_entity", "adapter_required", "Apply Z mutation before teleport validation; preserve event cancellation.", "net/minecraftforge/event/entity/living/EnderTeleportEvent.html"),
    d!("EnderTeleportEvent", "getAttackDamage", V_18, "teleport_attack_damage", "DamageSource / ender pearl teleport damage", "events_entity", "automatic_rename_candidate", "Preserve teleport-associated damage amount separately from target position and cancellation.", "net/minecraftforge/event/entity/living/EnderTeleportEvent.html"),

    // Player events: bow charging, critical hits, fishing, buckets, bonemeal and tooltips.
    d!("ArrowLooseEvent", "getBow", V_18, "bow_release_stack", "BowItem release context", "events_item_use", "automatic_rename_candidate", "Preserve exact charged bow stack and enchantments used by the release callback.", "net/minecraftforge/event/entity/player/ArrowLooseEvent.html"),
    d!("ArrowLooseEvent", "getWorld", V_18, "bow_release_world", "BowItem release context", "events_item_use", "automatic_rename_candidate", "Preserve the world used to construct and spawn the released projectile.", "net/minecraftforge/event/entity/player/ArrowLooseEvent.html"),
    d!("ArrowLooseEvent", "hasAmmo", V_18, "bow_release_has_ammo", "BowItem projectile ammunition predicate", "events_item_use", "automatic_rename_candidate", "Preserve ammunition availability independently from the charged bow stack.", "net/minecraftforge/event/entity/player/ArrowLooseEvent.html"),
    d!("ArrowLooseEvent", "getCharge", V_18, "bow_charge_ticks", "BowItem charge duration / projectile velocity", "events_item_use", "automatic_rename_candidate", "Preserve charge measured in ticks and the exact value used by the bow velocity curve.", "net/minecraftforge/event/entity/player/ArrowLooseEvent.html"),
    d!("ArrowLooseEvent", "setCharge", V_18, "bow_charge_ticks_write", "BowItem charge duration / projectile velocity", "events_item_use", "adapter_required", "Preserve modified charge before draw-strength calculation and projectile construction.", "net/minecraftforge/event/entity/player/ArrowLooseEvent.html"),
    d!("ArrowNockEvent", "getBow", V_18, "bow_nock_stack", "BowItem use-start interaction", "events_item_use", "automatic_rename_candidate", "Preserve the actual bow stack at use start, including custom mode and NBT state.", "net/minecraftforge/event/entity/player/ArrowNockEvent.html"),
    d!("ArrowNockEvent", "getWorld", V_18, "bow_nock_world", "BowItem use-start interaction", "events_item_use", "automatic_rename_candidate", "Preserve logical-side world context for starting bow use.", "net/minecraftforge/event/entity/player/ArrowNockEvent.html"),
    d!("ArrowNockEvent", "getHand", V_18, "bow_nock_hand", "InteractionHand / item use hand", "events_item_use", "automatic_rename_candidate", "Preserve main-hand/off-hand distinction when starting bow use.", "net/minecraftforge/event/entity/player/ArrowNockEvent.html"),
    d!("ArrowNockEvent", "hasAmmo", V_18, "bow_nock_has_ammo", "BowItem ammunition predicate", "events_item_use", "automatic_rename_candidate", "Preserve ammo check before beginning item use.", "net/minecraftforge/event/entity/player/ArrowNockEvent.html"),
    d!("ArrowNockEvent", "getAction", V_18, "bow_nock_action_result", "InteractionResultHolder<ItemStack> / item use result", "events_item_use", "adapter_required", "Translate the complete result and returned stack; do not collapse a replacement stack into SUCCESS/PASS.", "net/minecraftforge/event/entity/player/ArrowNockEvent.html"),
    d!("ArrowNockEvent", "setAction", V_18, "bow_nock_action_result_write", "InteractionResultHolder<ItemStack> / item use result", "events_item_use", "adapter_required", "Preserve custom action result and replacement stack returned by event listeners.", "net/minecraftforge/event/entity/player/ArrowNockEvent.html"),
    d!("CriticalHitEvent", "getTarget", V_112, "critical_hit_target", "LivingIncomingDamageEvent / attack context", "events_combat", "automatic_rename_candidate", "Preserve the exact attacked entity for critical-hit proc logic.", "net/minecraftforge/event/entity/player/CriticalHitEvent.html"),
    d!("CriticalHitEvent", "getDamageModifier", V_112, "critical_hit_multiplier", "Attack damage / critical hit modifier", "events_combat", "automatic_rename_candidate", "Preserve the active critical multiplier; vanilla defaults differ from custom listeners.", "net/minecraftforge/event/entity/player/CriticalHitEvent.html"),
    d!("CriticalHitEvent", "getOldDamageModifier", V_112, "critical_hit_original_multiplier", "Attack damage / critical hit modifier", "events_combat", "automatic_rename_candidate", "Retain the original multiplier separately from modifications applied by earlier listeners.", "net/minecraftforge/event/entity/player/CriticalHitEvent.html"),
    d!("CriticalHitEvent", "setDamageModifier", V_112, "critical_hit_multiplier_write", "Attack damage / critical hit modifier", "events_combat", "adapter_required", "Preserve zero multiplier behavior (critical visuals can remain while damage is suppressed).", "net/minecraftforge/event/entity/player/CriticalHitEvent.html"),
    d!("CriticalHitEvent", "isVanillaCritical", V_112, "critical_hit_vanilla_flag", "Attack critical state / attack context", "events_combat", "automatic_rename_candidate", "Distinguish vanilla critical conditions from a critical hit created by another mod.", "net/minecraftforge/event/entity/player/CriticalHitEvent.html"),
    d!("ItemFishedEvent", "getDrops", V_112, "fishing_event_drops", "LootTable fishing results", "events_loot", "manual_adapter_required", "The event exposes the resulting drops for observation; the JavaDoc explicitly directs loot modification to loot tables.", "net/minecraftforge/event/entity/player/ItemFishedEvent.html"),
    d!("ItemFishedEvent", "getHookEntity", V_112, "fishing_event_hook", "FishingHook bobber entity", "events_item_use", "automatic_rename_candidate", "Preserve bobber position, owner and caught-entity context.", "net/minecraftforge/event/entity/player/ItemFishedEvent.html"),
    d!("ItemFishedEvent", "getRodDamage", V_112, "fishing_rod_damage_read", "FishingHook retrieve / ItemStack durability", "events_item_use", "automatic_rename_candidate", "Preserve the pending fishing-rod durability cost.", "net/minecraftforge/event/entity/player/ItemFishedEvent.html"),
    d!("ItemFishedEvent", "damageRodBy", V_112, "fishing_rod_damage_write", "FishingHook retrieve / ItemStack durability", "events_item_use", "adapter_required", "The supplied amount replaces the pending rod damage; it is not additive to the previous amount.", "net/minecraftforge/event/entity/player/ItemFishedEvent.html"),
    d!("FillBucketEvent", "getEmptyBucket", V_18, "bucket_empty_stack", "BucketItem use context", "events_item_use", "automatic_rename_candidate", "Preserve the input empty-bucket stack and its count/tags.", "net/minecraftforge/event/entity/player/FillBucketEvent.html"),
    d!("FillBucketEvent", "getWorld", V_18, "bucket_fill_world", "BucketItem use context", "events_item_use", "automatic_rename_candidate", "Preserve world side and target fluid lookup context.", "net/minecraftforge/event/entity/player/FillBucketEvent.html"),
    d!("FillBucketEvent", "getTarget", V_18, "bucket_fill_raytrace", "BlockHitResult / ClipContext", "events_item_use", "adapter_required", "Preserve block-vs-fluid hit result, hit face and exact target position.", "net/minecraftforge/event/entity/player/FillBucketEvent.html"),
    d!("FillBucketEvent", "getFilledBucket", V_18, "bucket_fill_result_stack", "BucketItem result ItemStack", "events_item_use", "automatic_rename_candidate", "Preserve replacement filled-bucket stack and any custom fluid data.", "net/minecraftforge/event/entity/player/FillBucketEvent.html"),
    d!("FillBucketEvent", "setFilledBucket", V_18, "bucket_fill_result_stack_write", "BucketItem result ItemStack", "events_item_use", "adapter_required", "Return the replacement filled container and preserve stack/remainder semantics.", "net/minecraftforge/event/entity/player/FillBucketEvent.html"),
    d!("BonemealEvent", "getWorld", V_18, "bonemeal_world", "FertilizeContext / item interaction world", "worldgen", "automatic_rename_candidate", "Preserve world and logical side for custom fertilization checks.", "net/minecraftforge/event/entity/player/BonemealEvent.html"),
    d!("BonemealEvent", "getPos", V_18, "bonemeal_position", "FertilizeContext block position", "worldgen", "automatic_rename_candidate", "Preserve the exact fertilized block position.", "net/minecraftforge/event/entity/player/BonemealEvent.html"),
    d!("BonemealEvent", "getBlock", V_18, "bonemeal_block_state", "FertilizeContext BlockState", "worldgen", "automatic_rename_candidate", "Preserve the target block state and age/properties.", "net/minecraftforge/event/entity/player/BonemealEvent.html"),
    d!("BonemealEvent", "getHand", V_112, "bonemeal_hand", "InteractionHand / fertilizer item use", "worldgen", "automatic_rename_candidate", "Preserve which hand supplied the fertilizer; null is possible in non-hand contexts.", "net/minecraftforge/event/entity/player/BonemealEvent.html"),
    d!("BonemealEvent", "getStack", V_112, "bonemeal_stack", "Fertilizer item-use stack", "worldgen", "automatic_rename_candidate", "Preserve exact fertilizer stack and NBT state.", "net/minecraftforge/event/entity/player/BonemealEvent.html"),
    d!("ItemTooltipEvent", "getFlags", V_18, "tooltip_event_flags", "TooltipFlag / advanced tooltip mode", "tooltip", "automatic_rename_candidate", "Preserve advanced-vs-normal tooltip state and avoid assuming a non-null player.", "net/minecraftforge/event/entity/player/ItemTooltipEvent.html"),
    d!("ItemTooltipEvent", "getItemStack", V_18, "tooltip_event_stack", "ItemStack#appendHoverText", "tooltip", "automatic_rename_candidate", "Preserve the exact stack whose tooltip is being built, including components/tags.", "net/minecraftforge/event/entity/player/ItemTooltipEvent.html"),
    d!("ItemTooltipEvent", "getToolTip", V_18, "tooltip_event_lines", "Item#appendHoverText / tooltip Component list", "tooltip", "adapter_required", "Translate mutable formatted String lines to ordered Components while retaining formatting and insertion order.", "net/minecraftforge/event/entity/player/ItemTooltipEvent.html"),
    d!("ItemTooltipEvent", "getEntityPlayer", V_18, "tooltip_event_player", "TooltipContext / nullable player context", "tooltip", "adapter_required", "The JavaDoc allows a null player while search trees are populated; preserve this startup path.", "net/minecraftforge/event/entity/player/ItemTooltipEvent.html"),

    // Player lifecycle/tracking events and combat/drop interaction.
    d!("PlayerEvent.Clone", "isWasDeath", V_18, "player_clone_death_flag", "PlayerEvent.Clone / Player#restoreFrom", "events_player", "automatic_rename_candidate", "Preserve death-vs-dimension-travel clone reason when copying persistent capabilities/progression.", "net/minecraftforge/event/entity/player/PlayerEvent.Clone.html"),
    d!("PlayerEvent.StartTracking", "getTarget", V_18, "player_start_tracking_target", "PlayerEvent.StartTracking / entity tracking start", "events_player", "automatic_rename_candidate", "Preserve the tracked entity and tracking-start timing for sync initialization.", "net/minecraftforge/event/entity/player/PlayerEvent.StartTracking.html"),
    d!("PlayerEvent.StopTracking", "getTarget", V_18, "player_stop_tracking_target", "PlayerEvent.StopTracking / entity tracking stop", "events_player", "automatic_rename_candidate", "Preserve tracking-stop timing for per-viewer state cleanup.", "net/minecraftforge/event/entity/player/PlayerEvent.StopTracking.html"),
    d!("PlayerEvent.HarvestCheck", "canHarvest", V_18, "player_harvest_check", "Player#hasCorrectToolForDrops / block harvest predicate", "events_player", "automatic_rename_candidate", "Preserve the original harvest decision and the actual block/tool context.", "net/minecraftforge/event/entity/player/PlayerEvent.HarvestCheck.html"),
    d!("PlayerEvent.HarvestCheck", "setCanHarvest", V_18, "player_harvest_check_write", "Player#hasCorrectToolForDrops / block harvest predicate", "events_player", "adapter_required", "Translate forced harvest eligibility without bypassing adventure-mode or loot-tool rules.", "net/minecraftforge/event/entity/player/PlayerEvent.HarvestCheck.html"),
    d!("PlayerFlyableFallEvent", "getDistance", V_18, "player_flyable_fall_distance", "Player fall-damage / flying permission logic", "events_player", "automatic_rename_candidate", "Preserve fall distance for players whose flight-capable state changes fall damage.", "net/minecraftforge/event/entity/player/PlayerFlyableFallEvent.html"),
    d!("PlayerSleepInBedEvent", "getPos", V_18, "player_sleep_bed_position", "Player#startSleepInBed / sleep result", "events_player", "automatic_rename_candidate", "Preserve bed position and sleep-attempt context.", "net/minecraftforge/event/entity/player/PlayerSleepInBedEvent.html"),
    d!("PlayerSleepInBedEvent", "getResultStatus", V_18, "player_sleep_result", "Player#startSleepInBed / Either<SleepResult, Unit>", "events_player", "adapter_required", "Translate sleep result states, including failure reasons, instead of reducing to a boolean.", "net/minecraftforge/event/entity/player/PlayerSleepInBedEvent.html"),

    // Block lifecycle, crops, drops, placement, chunk persistence and explosions.
    d!("BlockEvent.HarvestDropsEvent", "getFortuneLevel", V_18, "block_drop_fortune", "Block drops / loot context fortune level", "events_blocks", "automatic_rename_candidate", "Preserve fortune level separately from silk touch and drop chance.", "net/minecraftforge/event/world/BlockEvent.HarvestDropsEvent.html"),
    d!("BlockEvent.HarvestDropsEvent", "getDrops", V_18, "block_harvest_drops", "Block loot table output", "events_blocks", "adapter_required", "Preserve ordered drop list and distinguish observing/modifying legacy drop output from modern loot-table generation.", "net/minecraftforge/event/world/BlockEvent.HarvestDropsEvent.html"),
    d!("BlockEvent.HarvestDropsEvent", "isSilkTouching", V_18, "block_drop_silk_touch", "Block loot context silk-touch predicate", "events_blocks", "automatic_rename_candidate", "Preserve silk-touch state independently from fortune and harvest tool.", "net/minecraftforge/event/world/BlockEvent.HarvestDropsEvent.html"),
    d!("BlockEvent.HarvestDropsEvent", "getDropChance", V_18, "block_drop_chance", "LootContext / loot function chance", "events_blocks", "automatic_rename_candidate", "Preserve per-event drop chance and random roll timing.", "net/minecraftforge/event/world/BlockEvent.HarvestDropsEvent.html"),
    d!("BlockEvent.HarvestDropsEvent", "setDropChance", V_18, "block_drop_chance_write", "LootContext / loot function chance", "events_blocks", "adapter_required", "Preserve the modified chance and exact point at which randomness is sampled.", "net/minecraftforge/event/world/BlockEvent.HarvestDropsEvent.html"),
    d!("BlockEvent.HarvestDropsEvent", "getHarvester", V_18, "block_drop_harvester", "LootContext player/tool actor", "events_blocks", "automatic_rename_candidate", "Retain null/non-player harvest context and the exact harvester identity.", "net/minecraftforge/event/world/BlockEvent.HarvestDropsEvent.html"),
    d!("BlockEvent.PlaceEvent", "getHand", V_112, "block_place_hand", "BlockPlaceContext hand / placement context", "events_blocks", "automatic_rename_candidate", "Preserve hand identity; the 1.12 compatibility getter is deprecated but still has behavior.", "net/minecraftforge/event/world/BlockEvent.PlaceEvent.html"),
    d!("BlockEvent.PlaceEvent", "getItemInHand", V_112, "block_place_item", "BlockPlaceContext item stack", "events_blocks", "automatic_rename_candidate", "Preserve the placement stack even though the old getter is deprecated.", "net/minecraftforge/event/world/BlockEvent.PlaceEvent.html"),
    d!("BlockEvent.EntityPlaceEvent", "getBlockSnapshot", V_112, "block_place_snapshot", "BlockSnapshot / placement rollback", "events_blocks", "adapter_required", "Preserve pre-placement state snapshot and rollback semantics for multi-block placements.", "net/minecraftforge/event/world/BlockEvent.EntityPlaceEvent.html"),
    d!("BlockEvent.EntityPlaceEvent", "getEntity", V_112, "block_place_entity", "BlockPlaceContext#player / placement actor", "events_blocks", "automatic_rename_candidate", "Preserve player-vs-non-player placement source.", "net/minecraftforge/event/world/BlockEvent.EntityPlaceEvent.html"),
    d!("BlockEvent.EntityPlaceEvent", "getPlacedAgainst", V_112, "block_place_against_state", "BlockPlaceContext clicked state", "events_blocks", "automatic_rename_candidate", "Preserve the block state clicked by the placer, separately from the newly placed state.", "net/minecraftforge/event/world/BlockEvent.EntityPlaceEvent.html"),
    d!("BlockEvent.EntityPlaceEvent", "getPlacedBlock", V_112, "block_place_placed_state", "BlockPlaceContext placed state", "events_blocks", "automatic_rename_candidate", "Preserve the actual placed state for custom placement side effects.", "net/minecraftforge/event/world/BlockEvent.EntityPlaceEvent.html"),
    d!("BlockEvent.NeighborNotifyEvent", "getForceRedstoneUpdate", V_18, "neighbor_notify_force_redstone", "neighbor update / redstone propagation", "events_blocks", "automatic_rename_candidate", "Preserve forced redstone notification separately from the notified side set.", "net/minecraftforge/event/world/BlockEvent.NeighborNotifyEvent.html"),
    d!("BlockEvent.NeighborNotifyEvent", "setForceRedstoneUpdate", V_18, "neighbor_notify_force_redstone_write", "neighbor update / redstone propagation", "events_blocks", "adapter_required", "Preserve forced notification behavior and prevent duplicate neighbor updates.", "net/minecraftforge/event/world/BlockEvent.NeighborNotifyEvent.html"),
    d!("BlockEvent.CropGrowEvent.Pre", "getState", V_112, "crop_growth_pre_state", "Crop growth state transition / random tick", "events_worldgen", "automatic_rename_candidate", "Preserve the pre-growth block state and age value.", "net/minecraftforge/event/world/BlockEvent.CropGrowEvent.Pre.html"),
    d!("BlockEvent.CropGrowEvent.Post", "getOriginalState", V_112, "crop_growth_original_state", "Crop growth state transition", "events_worldgen", "automatic_rename_candidate", "Preserve the original state separately from the successfully grown state.", "net/minecraftforge/event/world/BlockEvent.CropGrowEvent.Post.html"),
    d!("BlockEvent.CreateFluidSourceEvent", "getState", V_112, "fluid_source_candidate_state", "Fluid block source-generation predicate", "events_worldgen", "automatic_rename_candidate", "Preserve candidate state and world context for source-fluid creation rules.", "net/minecraftforge/event/world/BlockEvent.CreateFluidSourceEvent.html"),
    d!("BlockEvent.FluidPlaceBlockEvent", "getOriginalState", V_112, "fluid_placement_original_state", "LiquidBlock flow placement context", "events_blocks", "automatic_rename_candidate", "Preserve original state used when a flowing fluid replaces a block.", "net/minecraftforge/event/world/BlockEvent.FluidPlaceBlockEvent.html"),
    d!("ChunkDataEvent", "getData", V_18, "chunk_persistent_data", "ChunkDataEvent / chunk persistent attachment data", "persistence", "adapter_required", "Migrate chunk NBT payload into attachments/saved data while preserving load-vs-save timing.", "net/minecraftforge/event/world/ChunkDataEvent.html"),
    d!("ChunkWatchEvent", "getPlayer", V_18, "chunk_watch_player", "Chunk tracking / player-specific chunk synchronization", "networking", "automatic_rename_candidate", "Preserve the viewer player and per-player chunk watch lifecycle.", "net/minecraftforge/event/world/ChunkWatchEvent.html"),
    d!("ChunkWatchEvent.Watch", "getChunk", V_18, "chunk_watch_chunk", "Chunk tracking / chunk watch start", "networking", "automatic_rename_candidate", "Preserve the exact chunk as it becomes visible to this player.", "net/minecraftforge/event/world/ChunkWatchEvent.Watch.html"),
    d!("ChunkWatchEvent.UnWatch", "getChunk", V_18, "chunk_unwatch_chunk", "Chunk tracking / chunk watch stop", "networking", "automatic_rename_candidate", "Preserve per-player cleanup when a chunk is no longer tracked.", "net/minecraftforge/event/world/ChunkWatchEvent.UnWatch.html"),
    d!("ExplosionEvent", "getExplosion", V_18, "explosion_event_instance", "Explosion / explosion context", "events_combat", "automatic_rename_candidate", "Preserve the explosion object including source, radius, fire and affected-block context.", "net/minecraftforge/event/world/ExplosionEvent.html"),
    d!("ExplosionEvent.Detonate", "getAffectedBlocks", V_18, "explosion_affected_blocks", "Explosion#getToBlow", "events_combat", "adapter_required", "Preserve mutable affected-block ordering and block resistance/removal behavior.", "net/minecraftforge/event/world/ExplosionEvent.Detonate.html"),
    d!("ExplosionEvent.Detonate", "getAffectedEntities", V_18, "explosion_affected_entities", "Explosion affected-entity list", "events_combat", "adapter_required", "Preserve entity selection, damage and knockback order; do not treat as only an explosion radius.", "net/minecraftforge/event/world/ExplosionEvent.Detonate.html"),

    // World lifecycle and spawn discovery.
    d!("WorldEvent.Load", "getWorld", V_18, "world_load_event", "LevelEvent.Load / server level lifecycle", "events_world", "event_adapter_required", "Preserve per-world load timing; do not run once-per-server initialization once per dimension.", "net/minecraftforge/event/world/WorldEvent.Load.html"),
    d!("WorldEvent.Save", "getWorld", V_18, "world_save_event", "LevelEvent.Save / world data save lifecycle", "events_world", "event_adapter_required", "Preserve save timing and avoid blocking or mutating unsafe world state from the save callback.", "net/minecraftforge/event/world/WorldEvent.Save.html"),
    d!("WorldEvent.Unload", "getWorld", V_18, "world_unload_event", "LevelEvent.Unload / server level lifecycle", "events_world", "event_adapter_required", "Release per-world caches and capability state without treating dimension unload as server shutdown.", "net/minecraftforge/event/world/WorldEvent.Unload.html"),
    d!("WorldEvent.PotentialSpawns", "getPos", V_18, "potential_spawn_position", "MobSpawnSettings / biome spawn selection", "worldgen", "automatic_rename_candidate", "Preserve queried position and biome context for the spawn list.", "net/minecraftforge/event/world/WorldEvent.PotentialSpawns.html"),
    d!("WorldEvent.PotentialSpawns", "getType", V_18, "potential_spawn_category", "MobCategory / biome spawn settings", "worldgen", "automatic_rename_candidate", "Map creature category separately from individual entity type.", "net/minecraftforge/event/world/WorldEvent.PotentialSpawns.html"),
    d!("WorldEvent.PotentialSpawns", "getList", V_18, "potential_spawn_entries", "MobSpawnSettings.SpawnerData", "worldgen", "adapter_required", "Preserve weighted spawn entries, minimum/maximum group size and list ordering.", "net/minecraftforge/event/world/WorldEvent.PotentialSpawns.html"),

    // Terrain generation extension events.
    d!("DecorateBiomeEvent", "getWorld", V_18, "biome_decoration_world", "PlacedFeature placement context", "worldgen", "automatic_rename_candidate", "Preserve level and decoration-stage context for biome feature generation.", "net/minecraftforge/event/terraingen/DecorateBiomeEvent.html"),
    d!("DecorateBiomeEvent.Decorate", "getType", V_18, "biome_decoration_type", "Feature placement stage / Decoration step", "worldgen", "adapter_required", "Translate the exact decoration stage and CUSTOM events rather than flattening all decoration hooks.", "net/minecraftforge/event/terraingen/DecorateBiomeEvent.Decorate.html"),
    d!("OreGenEvent", "getWorld", V_18, "ore_gen_world", "Ore feature placement context", "worldgen", "automatic_rename_candidate", "Preserve the generating world and dimension context.", "net/minecraftforge/event/terraingen/OreGenEvent.html"),
    d!("OreGenEvent.GenerateMinable", "getGenerator", V_18, "ore_gen_generator", "ConfiguredFeature / placed ore feature", "worldgen", "manual_adapter_required", "Recover generator type, ore state, vein size and placement logic rather than mapping only the event name.", "net/minecraftforge/event/terraingen/OreGenEvent.GenerateMinable.html"),
    d!("OreGenEvent.GenerateMinable", "getType", V_18, "ore_gen_type", "Ore placement target / configured feature type", "worldgen", "adapter_required", "Preserve vanilla/custom ore generation category and custom event types.", "net/minecraftforge/event/terraingen/OreGenEvent.GenerateMinable.html"),
    d!("SaplingGrowTreeEvent", "getWorld", V_18, "sapling_tree_world", "SaplingGrowTreeEvent / tree feature placement context", "worldgen", "automatic_rename_candidate", "Preserve the world used for the tree placement attempt.", "net/minecraftforge/event/terraingen/SaplingGrowTreeEvent.html"),
    d!("SaplingGrowTreeEvent", "getPos", V_18, "sapling_tree_position", "Tree feature placement origin", "worldgen", "automatic_rename_candidate", "Preserve tree origin and sapling block identity.", "net/minecraftforge/event/terraingen/SaplingGrowTreeEvent.html"),
    d!("BiomeEvent.BiomeColor", "getOriginalColor", V_18, "biome_original_color", "BiomeSpecialEffects / client color handlers", "rendering", "automatic_rename_candidate", "Preserve original biome tint separately from modified output.", "net/minecraftforge/event/terraingen/BiomeEvent.BiomeColor.html"),
    d!("BiomeEvent.BiomeColor", "setNewColor", V_18, "biome_color_write", "BiomeSpecialEffects / client color handlers", "rendering", "adapter_required", "Translate event-driven biome color changes to registered client color handlers or biome effects.", "net/minecraftforge/event/terraingen/BiomeEvent.BiomeColor.html"),

    // Forge gameplay events not covered by entity subclasses.
    d!("AnvilUpdateEvent", "getLeft", V_112, "anvil_left_stack", "AnvilMenu input slot 0", "events_items", "automatic_rename_candidate", "Preserve the first anvil input stack and all custom data.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AnvilUpdateEvent", "getRight", V_112, "anvil_right_stack", "AnvilMenu input slot 1", "events_items", "automatic_rename_candidate", "Preserve the second anvil input stack and all custom data.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AnvilUpdateEvent", "getName", V_112, "anvil_output_name", "AnvilMenu rename input / ItemStack hover name", "events_items", "automatic_rename_candidate", "Preserve nullable custom rename text and translation/formatting semantics.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AnvilUpdateEvent", "getOutput", V_112, "anvil_output_stack", "AnvilMenu result ItemStack", "events_items", "automatic_rename_candidate", "Preserve custom output stack, enchantments and repair costs.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AnvilUpdateEvent", "setOutput", V_112, "anvil_output_stack_write", "AnvilMenu result ItemStack", "events_items", "adapter_required", "Preserve custom output and vanilla suppression semantics when output is non-null.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AnvilUpdateEvent", "getCost", V_112, "anvil_xp_cost", "AnvilMenu repair cost / XP levels", "events_items", "automatic_rename_candidate", "Preserve XP level cost and its ordering relative to material cost.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AnvilUpdateEvent", "setCost", V_112, "anvil_xp_cost_write", "AnvilMenu repair cost / XP levels", "events_items", "adapter_required", "Preserve modified XP cost and zero/negative handling.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AnvilUpdateEvent", "getMaterialCost", V_112, "anvil_material_cost", "AnvilMenu material consumption", "events_items", "automatic_rename_candidate", "Preserve consumed-material count separately from XP levels.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AnvilUpdateEvent", "setMaterialCost", V_112, "anvil_material_cost_write", "AnvilMenu material consumption", "events_items", "adapter_required", "Preserve material consumption count and stack shrink semantics.", "net/minecraftforge/event/AnvilUpdateEvent.html"),
    d!("AttachCapabilitiesEvent", "getObject", V_112, "capability_attach_target", "RegisterCapabilitiesEvent / attachment target", "capabilities", "automatic_rename_candidate", "Preserve the exact object receiving attached capabilities (entity, item, or tile entity).", "net/minecraftforge/event/AttachCapabilitiesEvent.html"),
    d!("AttachCapabilitiesEvent", "addCapability", V_112, "capability_attach_add", "RegisterCapabilitiesEvent / attachment registration", "capabilities", "adapter_required", "Translate namespaced capability attachment keys and lifecycle/serialization semantics.", "net/minecraftforge/event/AttachCapabilitiesEvent.html"),
    d!("LootTableLoadEvent", "getName", V_112, "loot_table_load_id", "LootTable load / ResourceLocation", "loot", "automatic_rename_candidate", "Preserve namespaced loot-table identity and load-time ordering.", "net/minecraftforge/event/LootTableLoadEvent.html"),
    d!("LootTableLoadEvent", "getTable", V_112, "loot_table_load_table", "LootTable / LootPool composition", "loot", "adapter_required", "Preserve pool order, conditions, functions and load-time modifications.", "net/minecraftforge/event/LootTableLoadEvent.html"),
    d!("LootTableLoadEvent", "setTable", V_112, "loot_table_load_table_write", "LootTable / LootPool composition", "loot", "adapter_required", "Replace the loaded table without dropping source conditions or modded pool additions.", "net/minecraftforge/event/LootTableLoadEvent.html"),
    d!("CommandEvent", "getParameters", V_112, "command_event_parameters", "CommandDispatcher parse results / command input", "commands", "adapter_required", "Preserve tokenized arguments and Brigadier parse context rather than flattening to a string.", "net/minecraftforge/event/CommandEvent.html"),
    d!("CommandEvent", "setParameters", V_112, "command_event_parameters_write", "CommandDispatcher parse results / command input", "commands", "manual_adapter_required", "Rebuild parsed arguments and source context consistently when the command is rewritten.", "net/minecraftforge/event/CommandEvent.html"),
    d!("CommandEvent", "getSender", V_112, "command_event_sender", "CommandSourceStack", "commands", "automatic_rename_candidate", "Preserve player/console sender and permission context.", "net/minecraftforge/event/CommandEvent.html"),
    d!("DifficultyChangeEvent", "getOldDifficulty", V_112, "difficulty_change_old", "ServerLevel difficulty / difficulty change event", "events_world", "automatic_rename_candidate", "Preserve old difficulty independently from the newly selected difficulty.", "net/minecraftforge/event/DifficultyChangeEvent.html"),
    d!("DifficultyChangeEvent", "getDifficulty", V_112, "difficulty_change_new", "ServerLevel difficulty / difficulty change event", "events_world", "automatic_rename_candidate", "Preserve requested new difficulty and world scope.", "net/minecraftforge/event/DifficultyChangeEvent.html"),
    d!("GameRuleChangeEvent", "getRuleName", V_112, "game_rule_change_name", "GameRules key / gamerule change event", "events_world", "automatic_rename_candidate", "Preserve namespaced/custom game-rule identity and exact case.", "net/minecraftforge/event/GameRuleChangeEvent.html"),
    d!("GameRuleChangeEvent", "getRuleValue", V_112, "game_rule_change_value", "GameRules value / gamerule change event", "events_world", "automatic_rename_candidate", "Preserve string representation and parse/type semantics for Boolean, Integer and custom rules.", "net/minecraftforge/event/GameRuleChangeEvent.html"),
    d!("ServerChatEvent", "getUsername", V_112, "server_chat_username", "ServerChatEvent / player identity", "chat", "automatic_rename_candidate", "Preserve sender identity separately from display component and message text.", "net/minecraftforge/event/ServerChatEvent.html"),
    d!("ServerChatEvent", "getMessage", V_112, "server_chat_message", "ServerChatEvent / chat Component", "chat", "automatic_rename_candidate", "Preserve original message text, formatting and command-vs-chat distinction.", "net/minecraftforge/event/ServerChatEvent.html"),
    d!("ServerChatEvent", "setMessage", V_112, "server_chat_message_write", "ServerChatEvent / chat Component", "chat", "adapter_required", "Translate rewritten text to Component while preserving event cancellation and broadcast order.", "net/minecraftforge/event/ServerChatEvent.html"),

    // Additional inherited event accessors and mutable gameplay fields.
    d!("LivingDropsEvent", "getSource", V_18, "living_drop_damage_source", "LivingDropsEvent damage source / loot context", "events_loot", "automatic_rename_candidate", "Preserve the direct/indirect damage source responsible for the death and resulting drops.", "net/minecraftforge/event/entity/living/LivingDropsEvent.html"),
    d!("LivingDropsEvent", "isRecentlyHit", V_18, "living_drop_recently_hit", "LivingDropsEvent recently-hit loot context", "events_loot", "automatic_rename_candidate", "Preserve the recently-hit flag used by player-attributed loot rules.", "net/minecraftforge/event/entity/living/LivingDropsEvent.html"),
    d!("PlayerFlyableFallEvent", "setDistance", V_18, "player_flyable_fall_distance_write", "Player fall-damage / flying permission logic", "events_player", "adapter_required", "Preserve modified fall distance before multiplier and damage calculation.", "net/minecraftforge/event/entity/player/PlayerFlyableFallEvent.html"),
    d!("PlayerFlyableFallEvent", "getMultiplier", V_18, "player_flyable_fall_multiplier", "Player fall-damage multiplier", "events_player", "automatic_rename_candidate", "Preserve the flyable-fall damage multiplier independently from distance.", "net/minecraftforge/event/entity/player/PlayerFlyableFallEvent.html"),
    d!("PlayerFlyableFallEvent", "setMultiplier", V_18, "player_flyable_fall_multiplier_write", "Player fall-damage multiplier", "events_player", "adapter_required", "Preserve custom fall multiplier, including zero-damage and partial-damage outcomes.", "net/minecraftforge/event/entity/player/PlayerFlyableFallEvent.html"),
    d!("PlayerEvent.HarvestCheck", "getTargetBlock", V_18, "player_harvest_target_block", "Player harvest predicate / BlockState", "events_player", "automatic_rename_candidate", "Preserve target block state used when computing whether this player can harvest it.", "net/minecraftforge/event/entity/player/PlayerEvent.HarvestCheck.html"),
    d!("PlayerSleepInBedEvent", "setResult", V_18, "player_sleep_result_write", "Player#startSleepInBed / sleep result", "events_player", "adapter_required", "Preserve the full SleepResult enum, including rejection reasons and forced success behavior.", "net/minecraftforge/event/entity/player/PlayerSleepInBedEvent.html"),
    d!("BlockEvent.FluidPlaceBlockEvent", "getLiquidPos", V_112, "fluid_placement_origin", "LiquidBlock flow origin position", "events_blocks", "automatic_rename_candidate", "Preserve the originating liquid position; it can differ from the position being replaced.", "net/minecraftforge/event/world/BlockEvent.FluidPlaceBlockEvent.html"),
    d!("BlockEvent.FluidPlaceBlockEvent", "getNewState", V_112, "fluid_placement_new_state", "LiquidBlock flow placement output", "events_blocks", "automatic_rename_candidate", "Preserve the proposed replacement block state after earlier listeners.", "net/minecraftforge/event/world/BlockEvent.FluidPlaceBlockEvent.html"),
    d!("BlockEvent.FluidPlaceBlockEvent", "setNewState", V_112, "fluid_placement_new_state_write", "LiquidBlock flow placement output", "events_blocks", "adapter_required", "Preserve the replacement state mutation used by custom cobblestone/obsidian generator behavior.", "net/minecraftforge/event/world/BlockEvent.FluidPlaceBlockEvent.html"),
    d!("AttachCapabilitiesEvent", "getCapabilities", V_112, "capability_attached_map", "Capability attachment providers / attachment serialization", "capabilities", "adapter_required", "The returned map is unmodifiable; preserve key uniqueness and provider serialization semantics.", "net/minecraftforge/event/AttachCapabilitiesEvent.html"),
    d!("WorldEvent.PotentialSpawns", "getWorld", V_18, "potential_spawn_world", "MobSpawnSettings / world spawn context", "worldgen", "automatic_rename_candidate", "Preserve the queried world and dimension for the spawn-list calculation.", "net/minecraftforge/event/world/WorldEvent.PotentialSpawns.html"),
    d!("ChunkDataEvent", "getChunk", V_18, "chunk_data_event_chunk", "ChunkDataEvent / chunk persistent data lifecycle", "persistence", "automatic_rename_candidate", "Preserve the chunk whose NBT payload is being loaded or saved.", "net/minecraftforge/event/world/ChunkDataEvent.html"),
    d!("ChunkDataEvent", "getWorld", V_18, "chunk_data_event_world", "ChunkDataEvent / level lifecycle", "persistence", "automatic_rename_candidate", "Preserve dimension/world identity for chunk data migration and attachment restoration.", "net/minecraftforge/event/world/ChunkDataEvent.html"),
    d!("ChunkWatchEvent", "getChunk", V_112, "chunk_watch_position_deprecated", "ChunkPos (deprecated) / ChunkWatchEvent#getChunkInstance", "networking", "adapter_required", "This deprecated accessor returns ChunkPos, not Chunk; use getChunkInstance for the affected Chunk. Preserve the distinct chunk-position and nullable loaded-chunk semantics.", "net/minecraftforge/event/world/ChunkWatchEvent.html"),
    d!("ExplosionEvent", "getWorld", V_18, "explosion_event_world", "Explosion level / world context", "events_combat", "automatic_rename_candidate", "Preserve the world in which the explosion executes, independently from its source entity.", "net/minecraftforge/event/world/ExplosionEvent.html"),

    // Base event inheritance and entity-wide lifecycle interception.
    d!("EntityEvent", "getEntity", V_112, "event_entity", "Entity / entity event payload", "events_entity", "automatic_rename_candidate", "Resolve the entity that caused this event; preserve exact entity identity and avoid substituting an event-specific actor.", "net/minecraftforge/event/entity/EntityEvent.html"),
    d!("EntityEvent.CanUpdate", "getCanUpdate", V_112, "entity_update_allowed_read", "Entity tick admission / entity lifecycle", "events_entity", "automatic_rename_candidate", "Preserve the pre-update admission flag used when an entity would otherwise be skipped.", "net/minecraftforge/event/entity/EntityEvent.CanUpdate.html"),
    d!("EntityEvent.CanUpdate", "setCanUpdate", V_112, "entity_update_allowed_write", "Entity tick admission / entity lifecycle", "events_entity", "adapter_required", "Preserve the ability to force an otherwise non-updating entity to tick; this is not event cancellation.", "net/minecraftforge/event/entity/EntityEvent.CanUpdate.html"),
    d!("EntityEvent.EnteringChunk", "getNewChunkX", V_112, "entity_new_chunk_x", "Entity chunk transition / chunk tracking", "events_entity", "automatic_rename_candidate", "Preserve destination chunk X independently from block coordinates and old chunk position.", "net/minecraftforge/event/entity/EntityEvent.EnteringChunk.html"),
    d!("EntityEvent.EnteringChunk", "getNewChunkZ", V_112, "entity_new_chunk_z", "Entity chunk transition / chunk tracking", "events_entity", "automatic_rename_candidate", "Preserve destination chunk Z independently from block coordinates and old chunk position.", "net/minecraftforge/event/entity/EntityEvent.EnteringChunk.html"),
    d!("EntityEvent.EnteringChunk", "getOldChunkX", V_112, "entity_old_chunk_x", "Entity chunk transition / chunk tracking", "events_entity", "automatic_rename_candidate", "Preserve source chunk X for boundary-crossing and tracking logic.", "net/minecraftforge/event/entity/EntityEvent.EnteringChunk.html"),
    d!("EntityEvent.EnteringChunk", "getOldChunkZ", V_112, "entity_old_chunk_z", "Entity chunk transition / chunk tracking", "events_entity", "automatic_rename_candidate", "Preserve source chunk Z for boundary-crossing and tracking logic.", "net/minecraftforge/event/entity/EntityEvent.EnteringChunk.html"),
    d!("EntityEvent.EnteringChunk", "setNewChunkX", V_112, "entity_new_chunk_x_write", "Entity chunk transition / chunk tracking", "events_entity", "adapter_required", "Preserve destination chunk mutation and ensure subsequent chunk membership is consistent.", "net/minecraftforge/event/entity/EntityEvent.EnteringChunk.html"),
    d!("EntityEvent.EnteringChunk", "setNewChunkZ", V_112, "entity_new_chunk_z_write", "Entity chunk transition / chunk tracking", "events_entity", "adapter_required", "Preserve destination chunk mutation and avoid leaving entity tracking in the old chunk.", "net/minecraftforge/event/entity/EntityEvent.EnteringChunk.html"),
    d!("EntityEvent.EnteringChunk", "setOldChunkX", V_112, "entity_old_chunk_x_write", "Entity chunk transition / chunk tracking", "events_entity", "adapter_required", "Preserve source chunk mutation if listeners intentionally redirect transition accounting.", "net/minecraftforge/event/entity/EntityEvent.EnteringChunk.html"),
    d!("EntityEvent.EnteringChunk", "setOldChunkZ", V_112, "entity_old_chunk_z_write", "Entity chunk transition / chunk tracking", "events_entity", "adapter_required", "Preserve source chunk mutation if listeners intentionally redirect transition accounting.", "net/minecraftforge/event/entity/EntityEvent.EnteringChunk.html"),
    d!("EntityMountEvent", "getEntityBeingMounted", V_112, "entity_mount_vehicle", "Entity#getVehicle / riding relationship", "events_entity", "automatic_rename_candidate", "This may be null during dismount; preserve nullable vehicle state and mount/dismount direction.", "net/minecraftforge/event/entity/EntityMountEvent.html"),
    d!("EntityMountEvent", "getEntityMounting", V_112, "entity_mount_passenger", "Entity#getPassengers / riding relationship", "events_entity", "automatic_rename_candidate", "Preserve the passenger entity independently from the vehicle entity.", "net/minecraftforge/event/entity/EntityMountEvent.html"),
    d!("EntityMountEvent", "getWorldObj", V_112, "entity_mount_world", "Entity level / riding relationship", "events_entity", "automatic_rename_candidate", "Preserve mount event world context and logical side.", "net/minecraftforge/event/entity/EntityMountEvent.html"),
    d!("EntityMountEvent", "isDismounting", V_112, "entity_dismount_predicate", "Entity#stopRiding / passenger removal", "events_entity", "automatic_rename_candidate", "Preserve dismount intent separately from mount intent; cancellation prevents either transition.", "net/minecraftforge/event/entity/EntityMountEvent.html"),
    d!("EntityMountEvent", "isMounting", V_112, "entity_mount_predicate", "Entity#startRiding / passenger addition", "events_entity", "automatic_rename_candidate", "Preserve mount intent separately from dismount intent and nullable vehicle state.", "net/minecraftforge/event/entity/EntityMountEvent.html"),
    d!("EntityTravelToDimensionEvent", "getDimension", V_112, "entity_destination_dimension", "Dimension travel / ResourceKey<Level>", "events_entity", "adapter_required", "Translate the numeric destination into a stable dimension ResourceKey and preserve cancellation before travel.", "net/minecraftforge/event/entity/EntityTravelToDimensionEvent.html"),
    d!("EntityStruckByLightningEvent", "getLightning", V_112, "entity_lightning_source", "LightningBolt entity impact", "events_entity", "automatic_rename_candidate", "Preserve the exact lightning entity, including channeler/source attribution.", "net/minecraftforge/event/entity/EntityStruckByLightningEvent.html"),
    d!("PlaySoundAtEntityEvent", "getCategory", V_112, "entity_sound_category", "SoundSource / entity sound context", "audio", "automatic_rename_candidate", "Translate SoundCategory to SoundSource without changing category-specific volume behavior.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "getDefaultPitch", V_112, "entity_sound_original_pitch", "SoundInstance pitch", "audio", "automatic_rename_candidate", "Preserve original pitch separately from the listener-modified output pitch.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "getDefaultVolume", V_112, "entity_sound_original_volume", "SoundInstance volume", "audio", "automatic_rename_candidate", "Preserve original volume separately from the listener-modified output volume.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "getPitch", V_112, "entity_sound_pitch", "SoundInstance pitch", "audio", "automatic_rename_candidate", "Read the effective pitch after any earlier event listener modifications.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "getSound", V_112, "entity_sound_event", "SoundEvent / ResourceLocation", "audio", "automatic_rename_candidate", "Preserve sound registry identity and support replacing the sound independently from volume/pitch.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "getVolume", V_112, "entity_sound_volume", "SoundInstance volume", "audio", "automatic_rename_candidate", "Read effective volume after earlier listener modifications.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "setCategory", V_112, "entity_sound_category_write", "SoundSource / entity sound context", "audio", "adapter_required", "Preserve category replacement and its downstream routing/volume behavior.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "setPitch", V_112, "entity_sound_pitch_write", "SoundInstance pitch", "audio", "automatic_rename_candidate", "Preserve pitch mutation and listener ordering.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "setSound", V_112, "entity_sound_event_write", "SoundEvent / ResourceLocation", "audio", "adapter_required", "Replace sound by registry identity while retaining event cancellation and sound source.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("PlaySoundAtEntityEvent", "setVolume", V_112, "entity_sound_volume_write", "SoundInstance volume", "audio", "automatic_rename_candidate", "Preserve volume mutation and sound attenuation behavior.", "net/minecraftforge/event/entity/PlaySoundAtEntityEvent.html"),
    d!("ProjectileImpactEvent", "getRayTraceResult", V_112, "projectile_impact_hit", "HitResult / projectile collision result", "projectiles", "adapter_required", "Translate block/entity hit results, hit location, side and collision target without losing impact type.", "net/minecraftforge/event/entity/ProjectileImpactEvent.html"),

    // Living entity events and effects, including breeding and effect lifecycle.
    d!("AnimalTameEvent", "getAnimal", V_112, "animal_tame_target", "TamableAnimal / tame interaction", "events_ai", "automatic_rename_candidate", "Preserve the animal being tamed and cancel taming when the event is denied.", "net/minecraftforge/event/entity/living/AnimalTameEvent.html"),
    d!("AnimalTameEvent", "getTamer", V_112, "animal_tamer_player", "Player / tame owner assignment", "events_ai", "automatic_rename_candidate", "Preserve the player responsible for the taming attempt.", "net/minecraftforge/event/entity/living/AnimalTameEvent.html"),
    d!("BabyEntitySpawnEvent", "getCausedByPlayer", V_112, "breeding_player", "Animal breeding cause / offspring initialization", "events_ai", "automatic_rename_candidate", "Preserve nullable player attribution for breeding and player-specific advancements/loot.", "net/minecraftforge/event/entity/living/BabyEntitySpawnEvent.html"),
    d!("BabyEntitySpawnEvent", "getChild", V_112, "breeding_child_entity", "AgeableMob offspring creation", "events_ai", "automatic_rename_candidate", "Preserve the proposed child entity class and its initialized variant state.", "net/minecraftforge/event/entity/living/BabyEntitySpawnEvent.html"),
    d!("BabyEntitySpawnEvent", "getParentA", V_112, "breeding_parent_a", "Animal breeding parent relationship", "events_ai", "automatic_rename_candidate", "Preserve initiating parent identity and breeding state.", "net/minecraftforge/event/entity/living/BabyEntitySpawnEvent.html"),
    d!("BabyEntitySpawnEvent", "getParentB", V_112, "breeding_parent_b", "Animal breeding parent relationship", "events_ai", "automatic_rename_candidate", "Preserve secondary parent identity and breeding state.", "net/minecraftforge/event/entity/living/BabyEntitySpawnEvent.html"),
    d!("BabyEntitySpawnEvent", "setChild", V_112, "breeding_child_entity_write", "AgeableMob offspring creation", "events_ai", "manual_adapter_required", "Preserve full child replacement, including entity type, synchronized data, variant and spawn initialization.", "net/minecraftforge/event/entity/living/BabyEntitySpawnEvent.html"),
    d!("LivingDestroyBlockEvent", "getPos", V_112, "mob_block_destroy_position", "Mob griefing / block destruction context", "events_blocks", "automatic_rename_candidate", "Preserve the exact position a dragon, wither or zombie is attempting to destroy.", "net/minecraftforge/event/entity/living/LivingDestroyBlockEvent.html"),
    d!("LivingDestroyBlockEvent", "getState", V_112, "mob_block_destroy_state", "Mob griefing / BlockState", "events_blocks", "automatic_rename_candidate", "Preserve the target block state and properties for block-destruction abilities.", "net/minecraftforge/event/entity/living/LivingDestroyBlockEvent.html"),
    d!("PotionEvent", "getPotionEffect", V_112, "potion_event_effect", "MobEffectInstance / effect event payload", "abilities", "automatic_rename_candidate", "Preserve the effect instance being added, checked, removed or expired, including amplifier/duration.", "net/minecraftforge/event/entity/living/PotionEvent.html"),
    d!("PotionEvent.PotionAddedEvent", "getOldPotionEffect", V_112, "potion_event_old_effect", "MobEffectInstance previous state", "abilities", "automatic_rename_candidate", "Preserve nullable previous effect when a duration/amplifier refresh replaces an existing instance.", "net/minecraftforge/event/entity/living/PotionEvent.PotionAddedEvent.html"),
    d!("PotionEvent.PotionApplicableEvent", "getPotionEffect", V_112, "potion_event_application_candidate", "MobEffectInstance application context", "abilities", "automatic_rename_candidate", "Preserve effect-specific application checks and tri-state DEFAULT/ALLOW/DENY result.", "net/minecraftforge/event/entity/living/PotionEvent.PotionApplicableEvent.html"),

    // Player events: anvil repairs, inventory lifecycle, advancement and gameplay.
    d!("AdvancementEvent", "getAdvancement", V_112, "player_advancement", "Advancement / advancement trigger event", "progression", "automatic_rename_candidate", "Preserve the exact advancement ResourceLocation and criterion/progression context.", "net/minecraftforge/event/entity/player/AdvancementEvent.html"),
    d!("AnvilRepairEvent", "getBreakChance", V_112, "anvil_break_chance", "AnvilMenu anvil durability damage chance", "events_items", "automatic_rename_candidate", "Preserve the probability of an anvil damage stage when a repaired output is taken.", "net/minecraftforge/event/entity/player/AnvilRepairEvent.html"),
    d!("AnvilRepairEvent", "getLeft", V_112, "anvil_repair_left_stack", "AnvilMenu input slot 0", "events_items", "automatic_rename_candidate", "Preserve the left input stack; these event stacks are read-only snapshots.", "net/minecraftforge/event/entity/player/AnvilRepairEvent.html"),
    d!("AnvilRepairEvent", "getOutput", V_112, "anvil_repair_output_stack", "AnvilMenu result stack", "events_items", "automatic_rename_candidate", "Preserve the exact repaired output after rename/enchantment/repair computation.", "net/minecraftforge/event/entity/player/AnvilRepairEvent.html"),
    d!("AnvilRepairEvent", "getRight", V_112, "anvil_repair_right_stack", "AnvilMenu input slot 1", "events_items", "automatic_rename_candidate", "Preserve the right input stack; these event stacks are read-only snapshots.", "net/minecraftforge/event/entity/player/AnvilRepairEvent.html"),
    d!("PlayerContainerEvent", "getContainer", V_112, "player_container_event_menu", "AbstractContainerMenu / menu lifecycle", "events_player", "automatic_rename_candidate", "Preserve the exact menu instance and its slot/state synchronization.", "net/minecraftforge/event/entity/player/PlayerContainerEvent.html"),
    d!("PlayerDestroyItemEvent", "getHand", V_112, "player_destroyed_item_hand", "InteractionHand / equipment replacement", "events_player", "automatic_rename_candidate", "Preserve which hand held the destroyed stack; null/legacy callers must remain distinguishable.", "net/minecraftforge/event/entity/player/PlayerDestroyItemEvent.html"),
    d!("PlayerDestroyItemEvent", "getOriginal", V_112, "player_destroyed_item_stack", "ItemStack destruction / break event", "events_player", "automatic_rename_candidate", "Preserve the original pre-break stack, including tags, enchantments and count.", "net/minecraftforge/event/entity/player/PlayerDestroyItemEvent.html"),
    d!("PlayerEvent.Visibility", "getVisibilityModifier", V_112, "player_visibility_modifier_read", "Mob#target visibility distance calculation", "events_ai", "automatic_rename_candidate", "Preserve the current multiplicative visibility modifier separately from vanilla sneak/light factors.", "net/minecraftforge/event/entity/player/PlayerEvent.Visibility.html"),
    d!("PlayerEvent.Visibility", "modifyVisibility", V_112, "player_visibility_modifier_write", "Mob#target visibility distance calculation", "events_ai", "adapter_required", "Apply the modifier multiplicatively and preserve the source cap at standard target distance.", "net/minecraftforge/event/entity/player/PlayerEvent.Visibility.html"),
    d!("PlayerPickupXpEvent", "getOrb", V_112, "player_xp_orb_pickup", "ExperienceOrb player collision/pickup", "events_player", "automatic_rename_candidate", "Preserve the exact XP orb and its value/position before collection.", "net/minecraftforge/event/entity/player/PlayerPickupXpEvent.html"),
    d!("PlayerSetSpawnEvent", "getNewSpawn", V_112, "player_new_spawn_position", "ServerPlayer respawn position", "events_player", "automatic_rename_candidate", "Preserve the new spawn location and dimension/forced flag when changing respawn behavior.", "net/minecraftforge/event/entity/player/PlayerSetSpawnEvent.html"),
    d!("PlayerSetSpawnEvent", "getOldSpawn", V_112, "player_old_spawn_position", "ServerPlayer respawn position", "events_player", "automatic_rename_candidate", "Preserve the prior spawn position when a player changes or clears respawn.", "net/minecraftforge/event/entity/player/PlayerSetSpawnEvent.html"),
    d!("UseHoeEvent", "getContext", V_112, "hoe_use_context", "UseOnContext / hoe block interaction", "events_item_use", "adapter_required", "Translate the legacy ItemUseContext to UseOnContext with precise level, hand, hit and stack semantics.", "net/minecraftforge/event/entity/player/UseHoeEvent.html"),
    d!("SleepingLocationCheckEvent", "getSleepingLocation", V_112, "player_sleeping_location_check", "Player sleep / bed position check", "events_player", "automatic_rename_candidate", "Preserve the checked bed position and result-bearing sleep validity decision.", "net/minecraftforge/event/entity/player/SleepingLocationCheckEvent.html"),
    d!("SleepingTimeCheckEvent", "getResult", V_112, "player_sleep_time_result", "Player sleep eligibility / daytime checks", "events_player", "adapter_required", "Preserve DEFAULT/ALLOW/DENY behavior that determines whether sleep may begin or wake sleeping players.", "net/minecraftforge/event/entity/player/SleepingTimeCheckEvent.html"),
    d!("SleepingTimeCheckEvent", "setResult", V_112, "player_sleep_time_result_write", "Player sleep eligibility / daytime checks", "events_player", "adapter_required", "Preserve result precedence and separate sleep-time permission from bed-location validation.", "net/minecraftforge/event/entity/player/SleepingTimeCheckEvent.html"),

    // Enchantment table and potion-color callbacks are behavior-bearing events.
    d!("EnchantmentLevelSetEvent", "getEnchantRow", V_112, "enchanting_table_row", "EnchantmentMenu enchantment option index", "enchantments", "automatic_rename_candidate", "Preserve row index (1-3) independently from computed enchantment level.", "net/minecraftforge/event/enchanting/EnchantmentLevelSetEvent.html"),
    d!("EnchantmentLevelSetEvent", "getItem", V_112, "enchanting_table_item", "EnchantmentMenu input ItemStack", "enchantments", "automatic_rename_candidate", "Preserve the exact input stack and its existing enchantments/tags.", "net/minecraftforge/event/enchanting/EnchantmentLevelSetEvent.html"),
    d!("EnchantmentLevelSetEvent", "getLevel", V_112, "enchanting_table_level", "EnchantmentMenu option level", "enchantments", "automatic_rename_candidate", "Read the mutable computed level after earlier listeners.", "net/minecraftforge/event/enchanting/EnchantmentLevelSetEvent.html"),
    d!("EnchantmentLevelSetEvent", "getOriginalLevel", V_112, "enchanting_table_original_level", "EnchantmentMenu vanilla option level", "enchantments", "automatic_rename_candidate", "Preserve original vanilla computed level separately from listener-mutated level.", "net/minecraftforge/event/enchanting/EnchantmentLevelSetEvent.html"),
    d!("EnchantmentLevelSetEvent", "getPos", V_112, "enchanting_table_position", "EnchantmentMenu block position", "enchantments", "automatic_rename_candidate", "Preserve the enchantment table block position.", "net/minecraftforge/event/enchanting/EnchantmentLevelSetEvent.html"),
    d!("EnchantmentLevelSetEvent", "getPower", V_112, "enchanting_table_power", "EnchantmentMenu bookshelf power", "enchantments", "automatic_rename_candidate", "Preserve bookshelf power (0-15) as a distinct input to the vanilla enchanting formula.", "net/minecraftforge/event/enchanting/EnchantmentLevelSetEvent.html"),
    d!("EnchantmentLevelSetEvent", "getWorld", V_112, "enchanting_table_world", "EnchantmentMenu level context", "enchantments", "automatic_rename_candidate", "Preserve the level used by the enchanting calculation and listener side effects.", "net/minecraftforge/event/enchanting/EnchantmentLevelSetEvent.html"),
    d!("EnchantmentLevelSetEvent", "setLevel", V_112, "enchanting_table_level_write", "EnchantmentMenu option level", "enchantments", "adapter_required", "Preserve the modified option level (0-30) and its timing before generated enchantment choices.", "net/minecraftforge/event/enchanting/EnchantmentLevelSetEvent.html"),

    // ChunkWatchEvent distinguishes the coordinate key from the loaded Chunk.
    d!("ChunkWatchEvent", "getChunkInstance", V_112, "chunk_watch_instance", "ChunkWatchEvent#getChunkInstance -> Chunk", "networking", "adapter_required", "Returns the affected Chunk instance and may be null; do not substitute deprecated getChunk(), which returns ChunkPos.", "net/minecraftforge/event/world/ChunkWatchEvent.html"),

    // Actual gameevent PlayerEvent item-pickup API; other legacy gameevent
    // PlayerEvent subclasses expose public fields rather than getter methods.
    d!("PlayerEvent.ItemPickupEvent", "getOriginalEntity", V_112, "player_pickup_original_entity", "ItemEntity / player pickup context", "events", "automatic_rename_candidate", "Preserve the original EntityItem being collected separately from the stack snapshot.", "net/minecraftforge/fml/common/gameevent/PlayerEvent.ItemPickupEvent.html"),
    d!("PlayerEvent.ItemPickupEvent", "getStack", V_112, "player_pickup_stack_snapshot", "ItemStack / player pickup context", "events", "automatic_rename_candidate", "Preserve the event's ItemStack snapshot and its tags/count at pickup time.", "net/minecraftforge/fml/common/gameevent/PlayerEvent.ItemPickupEvent.html"),

    // Client camera and view events.
    d!("EntityViewRenderEvent", "getEntity", V_112, "view_render_entity", "EntityRenderer view entity / camera context", "client_render", "automatic_rename_candidate", "Preserve the entity being rendered from the player's view, which may differ from the local player.", "net/minecraftforge/client/event/EntityViewRenderEvent.html"),
    d!("EntityViewRenderEvent", "getRenderer", V_112, "view_render_renderer", "EntityRenderer camera/render context", "client_render", "adapter_required", "Preserve the renderer instance and avoid treating it as a global singleton.", "net/minecraftforge/client/event/EntityViewRenderEvent.html"),
    d!("EntityViewRenderEvent", "getRenderPartialTicks", V_112, "view_render_partial_ticks", "Render partial tick interpolation", "client_render", "automatic_rename_candidate", "Preserve interpolation fraction used for camera, fog and view-dependent effects.", "net/minecraftforge/client/event/EntityViewRenderEvent.html"),
    d!("EntityViewRenderEvent", "getState", V_112, "view_render_block_state", "IBlockState / camera view context", "client_render", "automatic_rename_candidate", "Preserve the block state associated with the current view render event.", "net/minecraftforge/client/event/EntityViewRenderEvent.html"),
    d!("EntityViewRenderEvent.CameraSetup", "getPitch", V_112, "camera_pitch_read", "Camera angle / EntityRenderer camera setup", "client_render", "automatic_rename_candidate", "Preserve pitch in degrees and the camera setup timing.", "net/minecraftforge/client/event/EntityViewRenderEvent.CameraSetup.html"),
    d!("EntityViewRenderEvent.CameraSetup", "getRoll", V_112, "camera_roll_read", "Camera angle / EntityRenderer camera setup", "client_render", "automatic_rename_candidate", "Preserve roll independently from pitch and yaw.", "net/minecraftforge/client/event/EntityViewRenderEvent.CameraSetup.html"),
    d!("EntityViewRenderEvent.CameraSetup", "getYaw", V_112, "camera_yaw_read", "Camera angle / EntityRenderer camera setup", "client_render", "automatic_rename_candidate", "Preserve yaw in degrees and source interpolation state.", "net/minecraftforge/client/event/EntityViewRenderEvent.CameraSetup.html"),
    d!("EntityViewRenderEvent.CameraSetup", "setPitch", V_112, "camera_pitch_write", "Camera angle / EntityRenderer camera setup", "client_render", "adapter_required", "Apply pitch mutation at the camera setup point, preserving order with other camera listeners.", "net/minecraftforge/client/event/EntityViewRenderEvent.CameraSetup.html"),
    d!("EntityViewRenderEvent.CameraSetup", "setRoll", V_112, "camera_roll_write", "Camera angle / EntityRenderer camera setup", "client_render", "adapter_required", "Apply roll independently from player rotation and model head rotation.", "net/minecraftforge/client/event/EntityViewRenderEvent.CameraSetup.html"),
    d!("EntityViewRenderEvent.CameraSetup", "setYaw", V_112, "camera_yaw_write", "Camera angle / EntityRenderer camera setup", "client_render", "adapter_required", "Apply yaw mutation while preserving camera-versus-entity rotation semantics.", "net/minecraftforge/client/event/EntityViewRenderEvent.CameraSetup.html"),
    d!("EntityViewRenderEvent.FogColors", "getBlue", V_112, "fog_blue_read", "Fog color RGB channels", "client_render", "automatic_rename_candidate", "Preserve normalized blue fog channel before listener mutation.", "net/minecraftforge/client/event/EntityViewRenderEvent.FogColors.html"),
    d!("EntityViewRenderEvent.FogColors", "getGreen", V_112, "fog_green_read", "Fog color RGB channels", "client_render", "automatic_rename_candidate", "Preserve normalized green fog channel before listener mutation.", "net/minecraftforge/client/event/EntityViewRenderEvent.FogColors.html"),
    d!("EntityViewRenderEvent.FogColors", "getRed", V_112, "fog_red_read", "Fog color RGB channels", "client_render", "automatic_rename_candidate", "Preserve normalized red fog channel before listener mutation.", "net/minecraftforge/client/event/EntityViewRenderEvent.FogColors.html"),
    d!("EntityViewRenderEvent.FogColors", "setBlue", V_112, "fog_blue_write", "Fog color RGB channels", "client_render", "adapter_required", "Preserve floating-point channel values and apply before fog color is consumed.", "net/minecraftforge/client/event/EntityViewRenderEvent.FogColors.html"),
    d!("EntityViewRenderEvent.FogColors", "setGreen", V_112, "fog_green_write", "Fog color RGB channels", "client_render", "adapter_required", "Preserve floating-point channel values and apply before fog color is consumed.", "net/minecraftforge/client/event/EntityViewRenderEvent.FogColors.html"),
    d!("EntityViewRenderEvent.FogColors", "setRed", V_112, "fog_red_write", "Fog color RGB channels", "client_render", "adapter_required", "Preserve floating-point channel values and apply before fog color is consumed.", "net/minecraftforge/client/event/EntityViewRenderEvent.FogColors.html"),
    d!("EntityViewRenderEvent.FogDensity", "getDensity", V_112, "fog_density_read", "Fog density / RenderGlobal fog setup", "client_render", "automatic_rename_candidate", "Preserve the density value used by the fog render pass.", "net/minecraftforge/client/event/EntityViewRenderEvent.FogDensity.html"),
    d!("EntityViewRenderEvent.FogDensity", "setDensity", V_112, "fog_density_write", "Fog density / RenderGlobal fog setup", "client_render", "adapter_required", "Preserve custom density; the event's cancelability/reset semantics must be handled separately.", "net/minecraftforge/client/event/EntityViewRenderEvent.FogDensity.html"),
    d!("EntityViewRenderEvent.FOVModifier", "getFOV", V_112, "view_fov_read", "EntityRenderer field-of-view calculation", "client_render", "automatic_rename_candidate", "Preserve the currently calculated FOV and distinguish it from player-specific FOV update.", "net/minecraftforge/client/event/EntityViewRenderEvent.FOVModifier.html"),
    d!("EntityViewRenderEvent.FOVModifier", "setFOV", V_112, "view_fov_write", "EntityRenderer field-of-view calculation", "client_render", "adapter_required", "Preserve FOV override before projection matrix construction.", "net/minecraftforge/client/event/EntityViewRenderEvent.FOVModifier.html"),
    d!("FOVUpdateEvent", "getEntity", V_112, "player_fov_entity", "EntityPlayer / FOV update event", "client_render", "automatic_rename_candidate", "Preserve the player whose FOV is being calculated.", "net/minecraftforge/client/event/FOVUpdateEvent.html"),
    d!("FOVUpdateEvent", "getFov", V_112, "player_fov_original", "EntityPlayer FOV calculation", "client_render", "automatic_rename_candidate", "Preserve original computed FOV independently from mutable output FOV.", "net/minecraftforge/client/event/FOVUpdateEvent.html"),
    d!("FOVUpdateEvent", "getNewfov", V_112, "player_fov_current", "EntityPlayer FOV calculation", "client_render", "automatic_rename_candidate", "Read current FOV after earlier listeners have modified it.", "net/minecraftforge/client/event/FOVUpdateEvent.html"),
    d!("FOVUpdateEvent", "setNewfov", V_112, "player_fov_write", "EntityPlayer FOV calculation", "client_render", "adapter_required", "Preserve FOV modifier composition and exact event order.", "net/minecraftforge/client/event/FOVUpdateEvent.html"),
    d!("GuiOpenEvent", "getGui", V_112, "gui_open_target_read", "GuiScreen / client screen transition", "client_gui", "automatic_rename_candidate", "Preserve the requested screen, including null when closing the current GUI.", "net/minecraftforge/client/event/GuiOpenEvent.html"),
    d!("GuiOpenEvent", "setGui", V_112, "gui_open_target_write", "GuiScreen / client screen transition", "client_gui", "adapter_required", "Replace the requested screen before opening and preserve event cancellation and null-to-close behavior.", "net/minecraftforge/client/event/GuiOpenEvent.html"),
    d!("InputUpdateEvent", "getMovementInput", V_112, "player_movement_input", "MovementInput / client player movement", "client_input", "adapter_required", "Preserve mutable forward/strafe/jump/sneak inputs at the post-input-update point before movement is processed.", "net/minecraftforge/client/event/InputUpdateEvent.html"),
    d!("MouseEvent", "getButton", V_112, "mouse_button", "Mouse input event / GLFW button", "client_input", "automatic_rename_candidate", "Preserve button index and distinguish wheel events from button events.", "net/minecraftforge/client/event/MouseEvent.html"),
    d!("MouseEvent", "getDwheel", V_112, "mouse_wheel_delta", "Mouse scroll input", "client_input", "automatic_rename_candidate", "Preserve signed wheel delta and event ordering.", "net/minecraftforge/client/event/MouseEvent.html"),
    d!("MouseEvent", "getDx", V_112, "mouse_delta_x", "Mouse movement input", "client_input", "automatic_rename_candidate", "Preserve relative horizontal movement delta, not absolute screen position.", "net/minecraftforge/client/event/MouseEvent.html"),
    d!("MouseEvent", "getDy", V_112, "mouse_delta_y", "Mouse movement input", "client_input", "automatic_rename_candidate", "Preserve relative vertical movement delta and sign convention.", "net/minecraftforge/client/event/MouseEvent.html"),
    d!("MouseEvent", "getNanoseconds", V_112, "mouse_event_time", "Input event timestamp", "client_input", "automatic_rename_candidate", "Preserve event timestamp units in nanoseconds for debounce/double-click timing.", "net/minecraftforge/client/event/MouseEvent.html"),
    d!("MouseEvent", "getX", V_112, "mouse_position_x", "Mouse absolute input coordinates", "client_input", "automatic_rename_candidate", "Preserve current absolute cursor X position in the display coordinate space.", "net/minecraftforge/client/event/MouseEvent.html"),
    d!("MouseEvent", "getY", V_112, "mouse_position_y", "Mouse absolute input coordinates", "client_input", "automatic_rename_candidate", "Preserve current absolute cursor Y position and screen coordinate origin.", "net/minecraftforge/client/event/MouseEvent.html"),
    d!("MouseEvent", "isButtonstate", V_112, "mouse_button_state", "Mouse button input state", "client_input", "automatic_rename_candidate", "Preserve press/release boolean separately from the button index.", "net/minecraftforge/client/event/MouseEvent.html"),
    d!("DrawBlockHighlightEvent", "getContext", V_112, "block_highlight_render_context", "RenderGlobal block highlight context", "client_render", "automatic_rename_candidate", "Preserve the RenderGlobal instance used for block selection highlighting.", "net/minecraftforge/client/event/DrawBlockHighlightEvent.html"),
    d!("DrawBlockHighlightEvent", "getPartialTicks", V_112, "block_highlight_partial_ticks", "Render partial tick interpolation", "client_render", "automatic_rename_candidate", "Preserve interpolation fraction used to draw the targeted block outline.", "net/minecraftforge/client/event/DrawBlockHighlightEvent.html"),
    d!("DrawBlockHighlightEvent", "getPlayer", V_112, "block_highlight_player", "EntityPlayer targeted block context", "client_render", "automatic_rename_candidate", "Preserve the player whose targeted block is being highlighted.", "net/minecraftforge/client/event/DrawBlockHighlightEvent.html"),
    d!("DrawBlockHighlightEvent", "getSubID", V_112, "block_highlight_sub_id", "Block selection sub-hit index", "client_render", "automatic_rename_candidate", "Preserve sub-hit index for multipart/custom block selections.", "net/minecraftforge/client/event/DrawBlockHighlightEvent.html"),
    d!("DrawBlockHighlightEvent", "getTarget", V_112, "block_highlight_raytrace", "RayTraceResult targeted block/entity hit", "client_render", "automatic_rename_candidate", "Preserve the full hit result including block/entity, hit face and hit location.", "net/minecraftforge/client/event/DrawBlockHighlightEvent.html"),
    d!("RenderBlockOverlayEvent", "getBlockForOverlay", V_112, "block_overlay_state", "IBlockState overlay source", "client_render", "automatic_rename_candidate", "Preserve block state used when drawing the overlay texture; meaningful for BLOCK overlay type.", "net/minecraftforge/client/event/RenderBlockOverlayEvent.html"),
    d!("RenderBlockOverlayEvent", "getBlockPos", V_112, "block_overlay_position", "BlockPos overlay source", "client_render", "automatic_rename_candidate", "Preserve world block position associated with the overlay.", "net/minecraftforge/client/event/RenderBlockOverlayEvent.html"),
    d!("RenderBlockOverlayEvent", "getOverlayType", V_112, "block_overlay_type", "RenderBlockOverlayEvent.OverlayType", "client_render", "automatic_rename_candidate", "Distinguish BLOCK, FIRE and WATER overlay rendering paths.", "net/minecraftforge/client/event/RenderBlockOverlayEvent.html"),
    d!("RenderBlockOverlayEvent", "getPlayer", V_112, "block_overlay_player", "EntityPlayer overlay context", "client_render", "automatic_rename_candidate", "Preserve the player receiving the first-person overlay.", "net/minecraftforge/client/event/RenderBlockOverlayEvent.html"),
    d!("RenderBlockOverlayEvent", "getRenderPartialTicks", V_112, "block_overlay_partial_ticks", "Render partial tick interpolation", "client_render", "automatic_rename_candidate", "Preserve interpolation fraction for overlay animation.", "net/minecraftforge/client/event/RenderBlockOverlayEvent.html"),
    d!("RenderGameOverlayEvent", "getPartialTicks", V_112, "gui_overlay_partial_ticks", "ScaledResolution GUI overlay render", "client_render", "automatic_rename_candidate", "Preserve frame interpolation timing for HUD overlay rendering.", "net/minecraftforge/client/event/RenderGameOverlayEvent.html"),
    d!("RenderGameOverlayEvent", "getResolution", V_112, "gui_overlay_resolution", "ScaledResolution GUI overlay render", "client_render", "automatic_rename_candidate", "Preserve scaled dimensions and GUI scale used for the overlay pass.", "net/minecraftforge/client/event/RenderGameOverlayEvent.html"),
    d!("RenderGameOverlayEvent", "getType", V_112, "gui_overlay_element_type", "RenderGameOverlayEvent.ElementType", "client_render", "automatic_rename_candidate", "Preserve exact HUD element stage (ALL, HELMET, CROSSHAIRS, HOTBAR, TEXT, etc.) and Pre/Post timing.", "net/minecraftforge/client/event/RenderGameOverlayEvent.html"),
    d!("RenderHandEvent", "getContext", V_112, "hand_render_context", "RenderGlobal first-person hand context", "client_render", "automatic_rename_candidate", "Preserve rendering context used for first-person hand passes.", "net/minecraftforge/client/event/RenderHandEvent.html"),
    d!("RenderHandEvent", "getPartialTicks", V_112, "hand_render_partial_ticks", "First-person hand render interpolation", "client_render", "automatic_rename_candidate", "Preserve partial tick interpolation for hand animations.", "net/minecraftforge/client/event/RenderHandEvent.html"),
    d!("RenderHandEvent", "getRenderPass", V_112, "hand_render_pass", "First-person hand render pass index", "client_render", "automatic_rename_candidate", "Preserve render pass selection and avoid conflating pass IDs with GUI overlay types.", "net/minecraftforge/client/event/RenderHandEvent.html"),
    d!("RenderTooltipEvent", "getFontRenderer", V_112, "tooltip_render_font", "FontRenderer tooltip drawing", "client_render", "automatic_rename_candidate", "Preserve selected font renderer, including custom tooltip font behavior.", "net/minecraftforge/client/event/RenderTooltipEvent.html"),
    d!("RenderTooltipEvent", "getLines", V_112, "tooltip_render_lines", "Tooltip ordered String line list", "client_render", "adapter_required", "Preserve mutable line ordering and formatting, converting to Components only after all source callbacks run.", "net/minecraftforge/client/event/RenderTooltipEvent.html"),
    d!("RenderTooltipEvent", "getStack", V_112, "tooltip_render_stack", "ItemStack tooltip rendering", "client_render", "automatic_rename_candidate", "Preserve stack components/tags and item identity used to create the tooltip.", "net/minecraftforge/client/event/RenderTooltipEvent.html"),
    d!("RenderTooltipEvent", "getX", V_112, "tooltip_render_x", "Tooltip screen coordinate", "client_render", "automatic_rename_candidate", "Preserve tooltip X coordinate after source placement calculations.", "net/minecraftforge/client/event/RenderTooltipEvent.html"),
    d!("RenderTooltipEvent", "getY", V_112, "tooltip_render_y", "Tooltip screen coordinate", "client_render", "automatic_rename_candidate", "Preserve tooltip Y coordinate after source placement calculations.", "net/minecraftforge/client/event/RenderTooltipEvent.html"),
    d!("RenderItemInFrameEvent", "getEntityItemFrame", V_112, "rendered_item_frame", "EntityItemFrame / renderer context", "client_render", "automatic_rename_candidate", "Preserve item-frame entity orientation, world position and displayed stack context.", "net/minecraftforge/client/event/RenderItemInFrameEvent.html"),
    d!("RenderItemInFrameEvent", "getItem", V_112, "rendered_frame_item_stack", "ItemStack / item-frame rendering", "client_render", "automatic_rename_candidate", "Preserve exact displayed ItemStack including tags and model overrides.", "net/minecraftforge/client/event/RenderItemInFrameEvent.html"),
    d!("RenderItemInFrameEvent", "getRenderer", V_112, "render_item_frame_renderer", "RenderItemFrame renderer instance", "client_render", "automatic_rename_candidate", "Preserve renderer context for item-frame render customization.", "net/minecraftforge/client/event/RenderItemInFrameEvent.html"),
    d!("RenderPlayerEvent", "getPartialRenderTick", V_112, "player_render_partial_ticks", "RenderPlayer interpolation", "client_render", "automatic_rename_candidate", "Preserve partial render tick for player model/armor animation.", "net/minecraftforge/client/event/RenderPlayerEvent.html"),
    d!("RenderPlayerEvent", "getRenderer", V_112, "player_render_renderer", "RenderPlayer renderer", "client_render", "automatic_rename_candidate", "Preserve concrete player renderer and its model/layer state.", "net/minecraftforge/client/event/RenderPlayerEvent.html"),
    d!("RenderPlayerEvent", "getX", V_112, "player_render_x", "Player render world position", "client_render", "automatic_rename_candidate", "Preserve interpolated render X coordinate, not entity block position.", "net/minecraftforge/client/event/RenderPlayerEvent.html"),
    d!("RenderPlayerEvent", "getY", V_112, "player_render_y", "Player render world position", "client_render", "automatic_rename_candidate", "Preserve interpolated render Y coordinate, not entity block position.", "net/minecraftforge/client/event/RenderPlayerEvent.html"),
    d!("RenderPlayerEvent", "getZ", V_112, "player_render_z", "Player render world position", "client_render", "automatic_rename_candidate", "Preserve interpolated render Z coordinate, not entity block position.", "net/minecraftforge/client/event/RenderPlayerEvent.html"),
    d!("RenderWorldLastEvent", "getContext", V_112, "world_render_context", "RenderGlobal world render context", "client_render", "automatic_rename_candidate", "Preserve world renderer context for custom world-space overlays and effects.", "net/minecraftforge/client/event/RenderWorldLastEvent.html"),
    d!("RenderWorldLastEvent", "getPartialTicks", V_112, "world_render_partial_ticks", "World render partial tick interpolation", "client_render", "automatic_rename_candidate", "Preserve interpolation for world-space render effects.", "net/minecraftforge/client/event/RenderWorldLastEvent.html"),
    d!("GuiContainerEvent", "getGuiContainer", V_112, "container_gui_render_target", "GuiContainer / client menu render", "client_gui", "automatic_rename_candidate", "Preserve the open GuiContainer instance and its inventory/container state.", "net/minecraftforge/client/event/GuiContainerEvent.html"),
    d!("PotionBrewEvent", "getItem", V_112, "brewing_slot_stack_read", "BrewingStand container slot ItemStack", "brewing", "automatic_rename_candidate", "Preserve exact stack at the indexed brewing slot and return an empty stack for empty slots.", "net/minecraftforge/event/brewing/PotionBrewEvent.html"),
    d!("PotionBrewEvent", "getLength", V_112, "brewing_slot_count", "BrewingStand container slots", "brewing", "automatic_rename_candidate", "Preserve number of brew slots and source iteration bounds.", "net/minecraftforge/event/brewing/PotionBrewEvent.html"),
    d!("PotionBrewEvent", "setItem", V_112, "brewing_slot_stack_write", "BrewingStand container slot ItemStack", "brewing", "adapter_required", "Preserve replacement stack by slot index and brewing process timing; do not omit bottle/fuel slots.", "net/minecraftforge/event/brewing/PotionBrewEvent.html"),
    d!("PlayerBrewedPotionEvent", "getStack", V_112, "player_brewed_stack", "Potion result ItemStack / brewing pickup", "brewing", "automatic_rename_candidate", "Preserve the brewed ItemStack collected by a player and its potion NBT/effects.", "net/minecraftforge/event/brewing/PlayerBrewedPotionEvent.html"),

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

fn source_owner_doc_path(version: &str, owner: &str) -> Option<&'static str> {
    let modern_fml = version != "1.7.10";
    match owner {
        "EntityEvent" => Some("net/minecraftforge/event/entity/EntityEvent.html"),
        "Event" => Some(if modern_fml { "net/minecraftforge/fml/common/eventhandler/Event.html" } else { "cpw/mods/fml/common/eventhandler/Event.html" }),
        "EventBus" => Some(if modern_fml { "net/minecraftforge/fml/common/eventhandler/EventBus.html" } else { "cpw/mods/fml/common/eventhandler/EventBus.html" }),
        "GameRegistry" => Some(if modern_fml { "net/minecraftforge/fml/common/registry/GameRegistry.html" } else { "cpw/mods/fml/common/registry/GameRegistry.html" }),
        "IWorldGenerator" => Some(if modern_fml { "net/minecraftforge/fml/common/IWorldGenerator.html" } else { "cpw/mods/fml/common/IWorldGenerator.html" }),
        "IFuelHandler" => Some(if modern_fml { "net/minecraftforge/fml/common/IFuelHandler.html" } else { "cpw/mods/fml/common/IFuelHandler.html" }),
        "IMessage" => Some(if modern_fml { "net/minecraftforge/fml/common/network/simpleimpl/IMessage.html" } else { "cpw/mods/fml/common/network/simpleimpl/IMessage.html" }),
        "IMessageHandler" => Some(if modern_fml { "net/minecraftforge/fml/common/network/simpleimpl/IMessageHandler.html" } else { "cpw/mods/fml/common/network/simpleimpl/IMessageHandler.html" }),
        "SimpleNetworkWrapper" => Some(if modern_fml { "net/minecraftforge/fml/common/network/simpleimpl/SimpleNetworkWrapper.html" } else { "cpw/mods/fml/common/network/simpleimpl/SimpleNetworkWrapper.html" }),
        "IExtendedEntityProperties" => Some(if modern_fml { "net/minecraftforge/fml/common/IExtendedEntityProperties.html" } else { "cpw/mods/fml/common/IExtendedEntityProperties.html" }),
        "ITickHandler" => Some(if modern_fml { "net/minecraftforge/fml/common/ITickHandler.html" } else { "cpw/mods/fml/common/ITickHandler.html" }),
        "IPlayerTracker" => Some(if modern_fml { "net/minecraftforge/fml/common/IPlayerTracker.html" } else { "cpw/mods/fml/common/IPlayerTracker.html" }),
        "FMLCommonHandler" => Some(if modern_fml { "net/minecraftforge/fml/common/FMLCommonHandler.html" } else { "cpw/mods/fml/common/FMLCommonHandler.html" }),
        "Loader" => Some(if modern_fml { "net/minecraftforge/fml/common/Loader.html" } else { "cpw/mods/fml/common/Loader.html" }),
        "RenderingRegistry" => Some(if modern_fml { "net/minecraftforge/fml/client/registry/RenderingRegistry.html" } else { "cpw/mods/fml/client/registry/RenderingRegistry.html" }),
        "ClientRegistry" => Some(if modern_fml { "net/minecraftforge/fml/client/registry/ClientRegistry.html" } else { "cpw/mods/fml/client/registry/ClientRegistry.html" }),
        "IBlockState" => Some("net/minecraft/block/state/IBlockState.html"),
        "BlockPos" => Some(if version == "1.12.2" { "net/minecraft/util/math/BlockPos.html" } else { "net/minecraft/util/BlockPos.html" }),
        "AxisAlignedBB" => Some(if version == "1.12.2" { "net/minecraft/util/math/AxisAlignedBB.html" } else { "net/minecraft/util/AxisAlignedBB.html" }),
        "RayTraceResult" => Some("net/minecraft/util/math/RayTraceResult.html"),
        "Vec3d" => Some("net/minecraft/util/math/Vec3d.html"),
        "Vec3" => Some("net/minecraft/util/Vec3.html"),
        "WorldProvider" => Some("net/minecraft/world/WorldProvider.html"),
        "Chunk" => Some("net/minecraft/world/chunk/Chunk.html"),
        "BiomeGenBase" => Some("net/minecraft/world/biome/BiomeGenBase.html"),
        "EntityLiving" => Some("net/minecraft/entity/EntityLiving.html"),
        "PlayerCapabilities" => Some("net/minecraft/entity/player/PlayerCapabilities.html"),
        "NBTTagList" => Some("net/minecraft/nbt/NBTTagList.html"),
        "NBTBase" => Some("net/minecraft/nbt/NBTBase.html"),
        "ICapabilityProvider" => Some("net/minecraftforge/common/capabilities/ICapabilityProvider.html"),
        "CapabilityManager" => Some("net/minecraftforge/common/capabilities/CapabilityManager.html"),
        "Capability" => Some("net/minecraftforge/common/capabilities/Capability.html"),
        "Fluid" => Some("net/minecraftforge/fluids/Fluid.html"),
        "FluidStack" => Some("net/minecraftforge/fluids/FluidStack.html"),
        "IFluidHandler" => Some("net/minecraftforge/fluids/capability/IFluidHandler.html"),
        "IEnergyStorage" => Some("net/minecraftforge/energy/IEnergyStorage.html"),
        "ModelRegistryEvent" => Some("net/minecraftforge/client/event/ModelRegistryEvent.html"),
        "ModelBakeEvent" => Some("net/minecraftforge/client/event/ModelBakeEvent.html"),
        "TextureStitchEvent" => Some("net/minecraftforge/client/event/TextureStitchEvent.html"),
        "MinecraftForge" => Some("net/minecraftforge/common/MinecraftForge.html"),
        "EventBus" => Some(if modern_fml { "net/minecraftforge/eventbus/api/IEventBus.html" } else { "cpw/mods/fml/common/eventhandler/EventBus.html" }),
        "DimensionManager" => Some("net/minecraftforge/common/DimensionManager.html"),
        "Configuration" => Some("net/minecraftforge/common/config/Configuration.html"),
        "Property" => Some("net/minecraftforge/common/config/Property.html"),
        "EntityRegistry" => Some(if modern_fml { "net/minecraftforge/fml/common/registry/EntityRegistry.html" } else { "cpw/mods/fml/common/registry/EntityRegistry.html" }),
        "NetworkRegistry" => Some(if modern_fml { "net/minecraftforge/fml/common/network/NetworkRegistry.html" } else { "cpw/mods/fml/common/network/NetworkRegistry.html" }),
        "IGuiHandler" => Some(if modern_fml { "net/minecraftforge/fml/common/network/IGuiHandler.html" } else { "cpw/mods/fml/common/network/IGuiHandler.html" }),
        "IForgeRegistryEntry" => Some("net/minecraftforge/registries/IForgeRegistryEntry.html"),
        "ItemBlock" => Some("net/minecraft/item/ItemBlock.html"),
        "Enchantment" => Some("net/minecraft/enchantment/Enchantment.html"),
        "EntityAITasks" => Some("net/minecraft/entity/ai/EntityAITasks.html"),
        "EntitySenses" => Some("net/minecraft/entity/ai/EntitySenses.html"),
        "EntityMoveHelper" => Some("net/minecraft/entity/ai/EntityMoveHelper.html"),
        "EntityLookHelper" => Some("net/minecraft/entity/EntityLookHelper.html"),
        "PathNavigateGround" => Some("net/minecraft/pathfinding/PathNavigateGround.html"),
        "TileEntityFurnace" => Some("net/minecraft/tileentity/TileEntityFurnace.html"),
        "WorldServer" => Some("net/minecraft/world/WorldServer.html"),
        "ChunkProviderServer" => Some("net/minecraft/world/gen/ChunkProviderServer.html"),
        "Container" => Some("net/minecraft/inventory/Container.html"),
        "Slot" => Some("net/minecraft/inventory/Slot.html"),
        "OreDictionary" => Some("net/minecraftforge/oredict/OreDictionary.html"),
        _ => None,
    }
}

fn target_owner_doc_path(owner: &str) -> Option<&'static str> {
    let owner = owner.trim().split('<').next().unwrap_or(owner).trim();
    let owner = owner.strip_suffix(".Builder").unwrap_or(owner);
    let owner = owner.strip_suffix(".Properties").unwrap_or(owner);
    match owner {
        "Item" => Some("net/minecraft/world/item/Item.html"),
        "IItemExtension" => Some("net/neoforged/neoforge/common/extensions/IItemExtension.html"),
        "ItemStack" => Some("net/minecraft/world/item/ItemStack.html"),
        "ItemAttributeModifiers" => Some("net/minecraft/world/item/ItemAttributeModifiers.html"),
        "FoodProperties" => Some("net/minecraft/world/food/FoodProperties.html"),
        "DataComponents" => Some("net/minecraft/core/component/DataComponents.html"),
        "DataComponentType" => Some("net/minecraft/core/component/DataComponentType.html"),
        "ItemEnchantments" => Some("net/minecraft/world/item/enchantment/ItemEnchantments.html"),
        "Block" => Some("net/minecraft/world/level/block/Block.html"),
        "BlockHitResult" => Some("net/minecraft/world/phys/BlockHitResult.html"),
        "EntityHitResult" => Some("net/minecraft/world/phys/EntityHitResult.html"),
        "AABB" => Some("net/minecraft/world/phys/AABB.html"),
        "Vec3" => Some("net/minecraft/world/phys/Vec3.html"),
        "Shapes" => Some("net/minecraft/world/phys/shapes/Shapes.html"),
        "VoxelShape" => Some("net/minecraft/world/phys/shapes/VoxelShape.html"),
        "Heightmap" => Some("net/minecraft/world/level/levelgen/Heightmap.html"),
        "LevelChunk" => Some("net/minecraft/world/level/chunk/LevelChunk.html"),
        "ChunkPos" => Some("net/minecraft/world/level/ChunkPos.html"),
        "Biome" => Some("net/minecraft/world/level/biome/Biome.html"),
        "BiomeManager" => Some("net/minecraft/world/level/biome/BiomeManager.html"),
        "WorldBorder" => Some("net/minecraft/world/level/border/WorldBorder.html"),
        "Difficulty" => Some("net/minecraft/world/Difficulty.html"),
        "GameRules" => Some("net/minecraft/world/level/GameRules.html"),
        "Abilities" => Some("net/minecraft/world/entity/player/Abilities.html"),
        "Inventory" => Some("net/minecraft/world/entity/player/Inventory.html"),
        "FoodData" => Some("net/minecraft/world/food/FoodData.html"),
        "MobType" => Some("net/minecraft/world/entity/MobType.html"),
        "AttributeSupplier" => Some("net/minecraft/world/entity/ai/attributes/AttributeSupplier.html"),
        "AttributeInstance" => Some("net/minecraft/world/entity/ai/attributes/AttributeInstance.html"),
        "SpawnPlacements" => Some("net/minecraft/world/entity/SpawnPlacements.html"),
        "MoveControl" => Some("net/minecraft/world/entity/ai/control/MoveControl.html"),
        "LookControl" => Some("net/minecraft/world/entity/ai/control/LookControl.html"),
        "Sensing" => Some("net/minecraft/world/entity/ai/Sensing.html"),
        "DimensionType" => Some("net/minecraft/world/level/dimension/DimensionType.html"),
        "LevelStem" => Some("net/minecraft/world/level/dimension/LevelStem.html"),
        "FluidStack" => Some("net/neoforged/neoforge/fluids/FluidStack.html"),
        "FluidType" => Some("net/neoforged/neoforge/fluids/FluidType.html"),
        "Fluid" => Some("net/minecraft/world/level/material/Fluid.html"),
        "IFluidHandler" => Some("net/neoforged/neoforge/fluids/capability/IFluidHandler.html"),
        "IEnergyStorage" => Some("net/neoforged/neoforge/energy/IEnergyStorage.html"),
        "Capability" => Some("net/neoforged/neoforge/capabilities/BlockCapability.html"),
        "ICapabilityProvider" => Some("net/neoforged/neoforge/capabilities/ICapabilityProvider.html"),
        "ModList" => Some("net/neoforged/fml/ModList.html"),
        "IEventBus" => Some("net/neoforged/bus/api/IEventBus.html"),
        "EntityRenderersEvent" => Some("net/neoforged/neoforge/client/event/EntityRenderersEvent.html"),
        "ModelEvent" => Some("net/neoforged/neoforge/client/event/ModelEvent.html"),
        "TextureAtlas" => Some("net/minecraft/client/renderer/texture/TextureAtlas.html"),
        "FMLPaths" => Some("net/neoforged/fml/loading/FMLPaths.html"),
        "FMLEnvironment" => Some("net/neoforged/fml/loading/FMLEnvironment.html"),
        "BiomeModifier" => Some("net/neoforged/neoforge/common/world/BiomeModifier.html"),
        "BlockState" => Some("net/minecraft/world/level/block/state/BlockState.html"),
        "BlockBehaviour" => Some("net/minecraft/world/level/block/state/BlockBehaviour.html"),
        "BlockPos" => Some("net/minecraft/core/BlockPos.html"),
        "Level" => Some("net/minecraft/world/level/Level.html"),
        "LevelReader" => Some("net/minecraft/world/level/LevelReader.html"),
        "LevelAccessor" => Some("net/minecraft/world/level/LevelAccessor.html"),
        "Entity" => Some("net/minecraft/world/entity/Entity.html"),
        "LivingEntity" => Some("net/minecraft/world/entity/LivingEntity.html"),
        "Mob" => Some("net/minecraft/world/entity/Mob.html"),
        "Player" => Some("net/minecraft/world/entity/player/Player.html"),
        "ServerPlayer" => Some("net/minecraft/server/level/ServerPlayer.html"),
        "DamageSource" => Some("net/minecraft/world/damagesource/DamageSource.html"),
        "BlockEntity" => Some("net/minecraft/world/level/block/entity/BlockEntity.html"),
        "CompoundTag" => Some("net/minecraft/nbt/CompoundTag.html"),
        "Tag" => Some("net/minecraft/nbt/Tag.html"),
        "UseOnContext" => Some("net/minecraft/world/item/context/UseOnContext.html"),
        "InteractionResult" => Some("net/minecraft/world/InteractionResult.html"),
        "InteractionResultHolder" => Some("net/minecraft/world/InteractionResultHolder.html"),
        "TooltipFlag" => Some("net/minecraft/world/item/TooltipFlag.html"),
        "SoundEvent" => Some("net/minecraft/sounds/SoundEvent.html"),
        "SoundSource" => Some("net/minecraft/sounds/SoundSource.html"),
        "ResourceLocation" => Some("net/minecraft/resources/ResourceLocation.html"),
        "Holder" => Some("net/minecraft/core/Holder.html"),
        "Registry" => Some("net/minecraft/core/Registry.html"),
        "Recipe" => Some("net/minecraft/world/item/crafting/Recipe.html"),
        "ModConfigSpec" => Some("net/neoforged/neoforge/common/ModConfigSpec.html"),
        "ModConfigSpec.ConfigValue" => Some("net/neoforged/neoforge/common/ModConfigSpec.ConfigValue.html"),
        "ModConfigSpec.BooleanValue" => Some("net/neoforged/neoforge/common/ModConfigSpec.BooleanValue.html"),
        "ModConfigSpec.IntValue" => Some("net/neoforged/neoforge/common/ModConfigSpec.IntValue.html"),
        "ModConfigSpec.DoubleValue" => Some("net/neoforged/neoforge/common/ModConfigSpec.DoubleValue.html"),
        "ModLoadingContext" => Some("net/neoforged/fml/ModLoadingContext.html"),
        "ModConfigEvent" => Some("net/neoforged/fml/event/config/ModConfigEvent.html"),
        "ModConfig" => Some("net/neoforged/fml/config/ModConfig.html"),
        "GoalSelector" => Some("net/minecraft/world/entity/ai/goal/GoalSelector.html"),
        "GroundPathNavigation" => Some("net/minecraft/world/entity/ai/navigation/GroundPathNavigation.html"),
        "MoveControl" => Some("net/minecraft/world/entity/ai/control/MoveControl.html"),
        "LookControl" => Some("net/minecraft/world/entity/ai/control/LookControl.html"),
        "Sensing" => Some("net/minecraft/world/entity/ai/Sensing.html"),
        "ServerChunkCache" => Some("net/minecraft/server/level/ServerChunkCache.html"),
        "EntityType" => Some("net/minecraft/world/entity/EntityType.html"),
        "SpawnPlacements" => Some("net/minecraft/world/entity/SpawnPlacements.html"),
        "MenuProvider" => Some("net/minecraft/world/MenuProvider.html"),
        "NetworkHooks" => Some("net/neoforged/neoforge/network/NetworkHooks.html"),
        "MenuScreens" => Some("net/minecraft/client/gui/screens/MenuScreens.html"),
        "Slot" => Some("net/minecraft/world/inventory/Slot.html"),
        "BakedModel" => Some("net/minecraft/client/resources/model/BakedModel.html"),
        "IForgeBakedModel" => Some("net/neoforged/neoforge/client/extensions/IForgeBakedModel.html"),
        "ModelPart" => Some("net/minecraft/client/model/geom/ModelPart.html"),
        "CubeListBuilder" => Some("net/minecraft/client/model/geom/builders/CubeListBuilder.html"),
        "EntityModel" => Some("net/minecraft/client/model/EntityModel.html"),
        "PayloadRegistrar" => Some("net/neoforged/neoforge/network/registration/PayloadRegistrar.html"),
        "RegisterPayloadHandlersEvent" => Some("net/neoforged/neoforge/network/event/RegisterPayloadHandlersEvent.html"),
        "RegisterCommandsEvent" => Some("net/neoforged/neoforge/event/RegisterCommandsEvent.html"),
        "CommandSourceStack" => Some("net/minecraft/commands/CommandSourceStack.html"),
        "Capabilities.ItemHandler" => Some("net/neoforged/neoforge/capabilities/Capabilities.ItemHandler.html"),
        "Capabilities.FluidHandler" => Some("net/neoforged/neoforge/capabilities/Capabilities.FluidHandler.html"),
        "Capabilities.EnergyStorage" => Some("net/neoforged/neoforge/capabilities/Capabilities.EnergyStorage.html"),
        "RecipeType" => Some("net/minecraft/world/item/crafting/RecipeType.html"),
        "RecipeInput" => Some("net/minecraft/world/item/crafting/RecipeInput.html"),
        "AbstractContainerMenu" => Some("net/minecraft/world/inventory/AbstractContainerMenu.html"),
        "Container" => Some("net/minecraft/world/Container.html"),
        "DeferredRegister" => Some("net/neoforged/neoforge/registries/DeferredRegister.html"),
        "RegisterEvent" => Some("net/neoforged/neoforge/registries/RegisterEvent.html"),
        "RegisterCapabilitiesEvent" => Some("net/neoforged/neoforge/capabilities/RegisterCapabilitiesEvent.html"),
        "IItemHandler" => Some("net/neoforged/neoforge/items/IItemHandler.html"),
        "IItemHandlerModifiable" => Some("net/neoforged/neoforge/items/IItemHandlerModifiable.html"),
        "IFluidHandler" => Some("net/neoforged/neoforge/fluids/capability/IFluidHandler.html"),
        "PacketDistributor" => Some("net/neoforged/neoforge/network/PacketDistributor.html"),
        "CustomPacketPayload" => Some("net/minecraft/network/protocol/common/custom/CustomPacketPayload.html"),
        "StreamCodec" => Some("net/minecraft/network/codec/StreamCodec.html"),
        "BiomeModifier" => Some("net/neoforged/neoforge/common/world/BiomeModifier.html"),
        "Goal" => Some("net/minecraft/world/entity/ai/goal/Goal.html"),
        "PathNavigation" => Some("net/minecraft/world/entity/ai/navigation/PathNavigation.html"),
        "MobEffectInstance" => Some("net/minecraft/world/effect/MobEffectInstance.html"),
        "FoodData" => Some("net/minecraft/world/food/FoodData.html"),
        "Enchantment" => Some("net/minecraft/world/item/enchantment/Enchantment.html"),
        _ => None,
    }
}

fn target_documentation_urls(target: &str) -> Vec<String> {
    let mut urls = BTreeSet::new();
    for piece in target.split(" / ") {
        let owner = piece.split('#').next().unwrap_or(piece).trim();
        if let Some(path) = target_owner_doc_path(owner) {
            urls.insert(format!("https://lexxie.dev/neoforge/1.21.1/{path}"));
        }
    }
    urls.into_iter().collect()
}

fn resolve_target_contracts(definition: &Definition) -> Vec<TargetApiMethod> {
    let mut resolved = BTreeMap::<(String, String, String), TargetApiMethod>::new();
    let source_alias = format!("{}#{}", definition.owner, definition.method);

    for part in definition.target.split(" / ").flat_map(|value| value.split(';')) {
        let part = part.trim();
        let signature = part.split(" -> ").next().unwrap_or(part).trim();
        let Some((owner, method_signature)) = signature.split_once('#') else { continue; };
        let method = method_signature.split('(').next().unwrap_or(method_signature).trim();
        if method.is_empty() { continue; }
        for entry in find_target_api_methods(owner.trim(), method) {
            resolved.entry((entry.owner.clone(), entry.method.clone(), entry.signature.clone())).or_insert(entry);
        }
    }

    for entry in find_target_api_methods_by_legacy_alias(&source_alias) {
        resolved.entry((entry.owner.clone(), entry.method.clone(), entry.signature.clone())).or_insert(entry);
    }

    resolved.into_values().collect()
}

fn to_mapping(definition: &Definition) -> LegacyApiMethodMapping {
    let source_versions = definition.versions.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
    let mut documentation_urls = BTreeSet::new();
    for version in definition.versions {
        if let Some(path) = source_owner_doc_path(version, definition.owner) {
            if let Some(url) = documentation_url(version, path) {
                documentation_urls.insert(url);
            }
        } else if !definition.docs.is_empty() {
            if let Some(url) = documentation_url(version, definition.docs) {
                documentation_urls.insert(url);
            }
        }
    }
    LegacyApiMethodMapping {
        source_owner: definition.owner.to_owned(),
        source_method: definition.method.to_owned(),
        source_versions,
        normalized_behavior: definition.behavior.to_owned(),
        target_api: definition.target.to_owned(),
        category: definition.category.to_owned(),
        conversion_status: definition.status.to_owned(),
        notes: definition.notes.to_owned(),
        documentation_urls: documentation_urls.into_iter().collect(),
        target_documentation_urls: target_documentation_urls(definition.target),
        target_api_methods: resolve_target_contracts(definition),
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

fn inheritance_list_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"\b(?:extends|implements)\s+(?P<owners>[A-Za-z_$][A-Za-z0-9_$.]*(?:\s*,\s*[A-Za-z_$][A-Za-z0-9_$.]*)*)"
    ).expect("static Java inheritance list regex"))
}

fn inheritance_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"\b(?:extends|implements)\s+(?P<owner>[A-Za-z_$][A-Za-z0-9_$.]*)"
    ).expect("static legacy API inheritance regex"))
}

fn implemented_interfaces_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"\bimplements\s+(?P<owners>[A-Za-z_$][A-Za-z0-9_$.]*(?:\s*,\s*[A-Za-z_$][A-Za-z0-9_$.]*)*)"
    ).expect("static legacy API interface-list regex"))
}

fn declared_owner_candidates(source: &str) -> BTreeSet<String> {
    let mut owners = BTreeSet::new();
    for capture in inheritance_regex().captures_iter(source) {
        if let Some(owner) = capture.name("owner") {
            owners.extend(owner_candidates(owner.as_str()));
        }
    }
    for capture in implemented_interfaces_regex().captures_iter(source) {
        if let Some(list) = capture.name("owners") {
            for owner in list.as_str().split(',').map(str::trim) {
                owners.extend(owner_candidates(owner));
            }
        }
    }
    owners
}

fn owner_candidates(owner: &str) -> BTreeSet<String> {
    let parts: Vec<&str> = owner.split('.').collect();
    let short = parts.last().copied().unwrap_or(owner);
    let mut candidates = BTreeSet::new();
    for start in 0..parts.len() {
        candidates.insert(parts[start..].join("."));
    }
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
            if short == "EntityPlayerMP" {
                candidates.insert("EntityPlayer".to_owned());
            }
        }
        "EntityArrow" | "EntityThrowable" | "EntityItem" | "EntityXPOrb"
        | "EntityBoat" | "EntityMinecart" | "EntityFallingBlock" | "EntityFireball"
        | "EntityFishHook" => {
            candidates.insert("Entity".to_owned());
        }
        "EntityZombie" | "EntitySkeleton" | "EntityCreeper" | "EntitySpider"
        | "EntityBlaze" | "EntityGhast" | "EntityDragon" | "EntitySlime"
        | "EntityMagmaCube" | "EntityEnderman" | "EntityWitch" | "EntityPigZombie"
        | "EntityGuardian" | "EntityElderGuardian" | "EntityShulker" | "EntitySilverfish"
        | "EntityCaveSpider" | "EntityIronGolem" | "EntitySnowman" | "EntityWolf"
        | "EntityOcelot" | "EntityHorse" | "EntityPig" | "EntityCow" | "EntitySheep"
        | "EntityChicken" | "EntityRabbit" | "EntitySquid" | "EntityBat" => {
            candidates.insert("Entity".to_owned());
            candidates.insert("EntityLivingBase".to_owned());
            candidates.insert("EntityLiving".to_owned());
        }
        "ISidedInventory" => {
            candidates.insert("IInventory".to_owned());
        }
        "BlockFence" | "BlockWall" | "BlockSlab" | "BlockStairs" | "BlockTorch"
        | "BlockWorkbench" | "BlockFurnace" | "BlockChest" | "BlockDispenser"
        | "BlockHopper" | "BlockRailBase" | "BlockPressurePlate" | "BlockButton"
        | "BlockLever" | "BlockTrapDoor" | "BlockLadder" | "BlockVine"
        | "BlockCactus" | "BlockReed" | "BlockSnow" | "BlockIce" | "BlockGlass"
        | "BlockPane" | "BlockCarpet" | "BlockBed" | "BlockPortal" | "BlockEndPortal"
        | "BlockBeacon" | "BlockAnvil" | "BlockDragonEgg" | "BlockLeaves"
        | "BlockLog" | "BlockSapling" | "BlockRedstoneWire" | "BlockRedstoneDiode"
        | "BlockFenceGate" | "BlockColored" | "BlockBreakable" | "BlockSlime"
        | "BlockBookshelf" | "BlockTNT" | "BlockWeb" | "BlockSponge" | "BlockDirt"
        | "BlockGrass" | "BlockSand" | "BlockGravel" | "BlockStone" | "BlockOre"
        | "BlockRedstoneLight" | "BlockCommandBlock" | "BlockStructure" => {
            candidates.insert("Block".to_owned());
        }
        "WorldServer" | "WorldClient" => {
            candidates.insert("World".to_owned());
        }
        "TileEntityLockable" | "TileEntityChest" | "TileEntityFurnace"
        | "TileEntityMobSpawner" | "TileEntitySign" => {
            candidates.insert("TileEntity".to_owned());
        }
        // Forge event inheritance is important for usage matching: source
        // receivers are often typed as a concrete nested event while a method
        // such as getPlayer/getWorld is declared on one of its event bases.
        "InputUpdateEvent" | "RenderPlayerEvent" => {
            candidates.insert("PlayerEvent".to_owned());
            candidates.insert("LivingEvent".to_owned());
            candidates.insert("EntityEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "EntityViewRenderEvent.CameraSetup" | "EntityViewRenderEvent.FogColors"
        | "EntityViewRenderEvent.FogDensity" | "EntityViewRenderEvent.FOVModifier"
        | "EntityViewRenderEvent.RenderFogEvent" => {
            candidates.insert("EntityViewRenderEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "GuiScreenEvent" | "GuiScreenEvent.ActionPerformedEvent"
        | "GuiScreenEvent.ActionPerformedEvent.Pre" | "GuiScreenEvent.ActionPerformedEvent.Post"
        | "GuiScreenEvent.BackgroundDrawnEvent" | "GuiScreenEvent.DrawScreenEvent"
        | "GuiScreenEvent.DrawScreenEvent.Pre" | "GuiScreenEvent.DrawScreenEvent.Post"
        | "GuiScreenEvent.InitGuiEvent" | "GuiScreenEvent.InitGuiEvent.Pre"
        | "GuiScreenEvent.InitGuiEvent.Post" | "GuiScreenEvent.KeyboardInputEvent"
        | "GuiScreenEvent.KeyboardInputEvent.Pre" | "GuiScreenEvent.KeyboardInputEvent.Post"
        | "GuiScreenEvent.MouseInputEvent" | "GuiScreenEvent.MouseInputEvent.Pre"
        | "GuiScreenEvent.MouseInputEvent.Post" | "GuiScreenEvent.PotionShiftEvent" => {
            candidates.insert("GuiScreenEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "RenderTooltipEvent.Color" | "RenderTooltipEvent.Pre"
        | "RenderTooltipEvent.Post" | "RenderTooltipEvent.PostBackground"
        | "RenderTooltipEvent.PostText" => {
            candidates.insert("RenderTooltipEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "RenderGameOverlayEvent.BossInfo" | "RenderGameOverlayEvent.Chat"
        | "RenderGameOverlayEvent.Pre" | "RenderGameOverlayEvent.Post"
        | "RenderGameOverlayEvent.Text" => {
            candidates.insert("RenderGameOverlayEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "ColorHandlerEvent.Block" | "ColorHandlerEvent.Item" => {
            candidates.insert("ColorHandlerEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "GuiContainerEvent.DrawForeground" => {
            candidates.insert("GuiContainerEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "LivingEvent.LivingJumpEvent" | "LivingEvent.LivingUpdateEvent"
        | "AnimalTameEvent" | "LivingDestroyBlockEvent" | "LivingKnockBackEvent"
        | "LivingAttackEvent" | "LivingDeathEvent" | "LivingDropsEvent"
        | "LivingExperienceDropEvent" | "LivingHurtEvent" | "LivingDamageEvent"
        | "LivingFallEvent" | "LivingHealEvent" | "LivingPackSizeEvent"
        | "LivingSetAttackTargetEvent" | "LivingSpawnEvent" | "LivingEntityUseItemEvent"
        | "LivingJumpEvent" | "LivingUpdateEvent" | "EnderTeleportEvent"
        | "ZombieEvent" | "PotionEvent" | "PotionEvent.PotionAddedEvent"
        | "PotionEvent.PotionApplicableEvent" | "PotionEvent.PotionExpiryEvent"
        | "PotionEvent.PotionRemoveEvent" => {
            candidates.insert("LivingEvent".to_owned());
            candidates.insert("EntityEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "PlayerEvent" | "ArrowLooseEvent" | "ArrowNockEvent" | "AttackEntityEvent"
        | "BonemealEvent" | "EntityInteractEvent" | "EntityItemPickupEvent"
        | "FillBucketEvent" | "ItemTooltipEvent" | "PlayerDestroyItemEvent"
        | "PlayerDropsEvent" | "PlayerFlyableFallEvent" | "PlayerPickupXpEvent"
        | "PlayerSetSpawnEvent" | "PlayerSleepInBedEvent" | "PlayerWakeUpEvent"
        | "PlayerEvent.Clone" | "PlayerEvent.BreakSpeed" | "PlayerEvent.HarvestCheck"
        | "PlayerEvent.LoadFromFile" | "PlayerEvent.NameFormat" | "PlayerEvent.SaveToFile"
        | "PlayerEvent.StartTracking" | "PlayerEvent.StopTracking" | "CriticalHitEvent"
        | "ItemFishedEvent" => {
            candidates.insert("PlayerEvent".to_owned());
            candidates.insert("LivingEvent".to_owned());
            candidates.insert("EntityEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "BlockEvent" | "BlockEvent.BreakEvent" | "BlockEvent.PlaceEvent"
        | "BlockEvent.EntityPlaceEvent" | "BlockEvent.MultiPlaceEvent"
        | "BlockEvent.EntityMultiPlaceEvent" | "BlockEvent.HarvestDropsEvent"
        | "BlockEvent.NeighborNotifyEvent" | "BlockEvent.CropGrowEvent"
        | "BlockEvent.CropGrowEvent.Pre" | "BlockEvent.CropGrowEvent.Post"
        | "BlockEvent.CreateFluidSourceEvent" | "BlockEvent.FarmlandTrampleEvent"
        | "BlockEvent.FluidPlaceBlockEvent" | "BlockEvent.PortalSpawnEvent" => {
            candidates.insert("BlockEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "ChunkDataEvent" | "ChunkDataEvent.Load" | "ChunkDataEvent.Save" => {
            candidates.insert("ChunkDataEvent".to_owned());
            candidates.insert("ChunkEvent".to_owned());
            candidates.insert("WorldEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "ChunkEvent" | "ChunkEvent.Load" | "ChunkEvent.Unload" => {
            candidates.insert("ChunkEvent".to_owned());
            candidates.insert("WorldEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "ChunkWatchEvent" | "ChunkWatchEvent.Watch" | "ChunkWatchEvent.UnWatch" => {
            candidates.insert("ChunkWatchEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "ExplosionEvent" | "ExplosionEvent.Start" | "ExplosionEvent.Detonate" => {
            candidates.insert("ExplosionEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "EntityEvent" | "EntityEvent.CanUpdate" | "EntityEvent.EnteringChunk"
        | "EntityEvent.EntityConstructing" | "EntityJoinWorldEvent"
        | "EntityMountEvent" | "EntityMobGriefingEvent" | "EntityStruckByLightningEvent"
        | "EntityTravelToDimensionEvent" | "PlaySoundAtEntityEvent"
        | "ProjectileImpactEvent" | "ProjectileImpactEvent.Arrow"
        | "ProjectileImpactEvent.Fireball" | "ProjectileImpactEvent.Throwable" => {
            candidates.insert("EntityEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "BabyEntitySpawnEvent" | "EnchantmentLevelSetEvent" => {
            candidates.insert("Event".to_owned());
        }
        "PlayerContainerEvent" | "PlayerContainerEvent.Open"
        | "PlayerContainerEvent.Close" | "AdvancementEvent" | "AnvilRepairEvent"
        | "PlayerEvent.Visibility" | "SleepingLocationCheckEvent"
        | "SleepingTimeCheckEvent" | "UseHoeEvent" => {
            candidates.insert("PlayerEvent".to_owned());
            candidates.insert("LivingEvent".to_owned());
            candidates.insert("EntityEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "WorldEvent" | "WorldEvent.Load" | "WorldEvent.Save" | "WorldEvent.Unload"
        | "WorldEvent.PotentialSpawns" | "WorldEvent.CreateSpawnPosition" => {
            candidates.insert("WorldEvent".to_owned());
            candidates.insert("Event".to_owned());
        }
        "DecorateBiomeEvent" | "DecorateBiomeEvent.Decorate" | "DecorateBiomeEvent.Pre"
        | "DecorateBiomeEvent.Post" | "OreGenEvent" | "OreGenEvent.Pre"
        | "OreGenEvent.Post" | "OreGenEvent.GenerateMinable" | "BiomeEvent"
        | "BiomeEvent.BiomeColor" | "BiomeEvent.GetGrassColor"
        | "BiomeEvent.GetFoliageColor" | "BiomeEvent.GetWaterColor"
        | "SaplingGrowTreeEvent" | "ChunkGeneratorEvent" | "PopulateChunkEvent"
        | "WorldTypeEvent" => {
            candidates.insert("Event".to_owned());
        }
        "AnvilUpdateEvent" | "AttachCapabilitiesEvent" | "CommandEvent"
        | "DifficultyChangeEvent" | "GameRuleChangeEvent" | "LootTableLoadEvent"
        | "ServerChatEvent" | "ClientChatEvent" | "ClientChatReceivedEvent"
        | "GenericEvent" => {
            candidates.insert("Event".to_owned());
        }
        _ => {}
    }
    candidates
}

#[derive(Debug)]
struct ClassScope {
    name: String,
    start: usize,
    end: usize,
    parents: Vec<String>,
    owners: BTreeSet<String>,
    receivers: BTreeMap<String, BTreeSet<String>>,
}

fn class_declaration_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"(?m)\b(?:class|interface|enum|record)\s+(?P<name>[A-Za-z_$][A-Za-z0-9_$]*)(?:\s*<[^{}]*>)?(?P<header>[^{};]*)\{"
    ).expect("static Java class declaration regex"))
}

/// Find the matching class brace while ignoring braces inside comments and
/// Java string/character/text-block literals.
fn matching_class_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'{') { return None; }
    let mut depth = 1usize;
    let mut index = open + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' { index += 1; }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'"' if bytes.get(index + 1) == Some(&b'"') && bytes.get(index + 2) == Some(&b'"') => {
                index += 3;
                while index + 2 < bytes.len()
                    && !(bytes[index] == b'"' && bytes[index + 1] == b'"' && bytes[index + 2] == b'"')
                {
                    if bytes[index] == b'\\' { index = (index + 2).min(bytes.len()); }
                    else { index += 1; }
                }
                index = (index + 3).min(bytes.len());
            }
            b'"' | b'\'' => {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' { index = (index + 2).min(bytes.len()); }
                    else if bytes[index] == quote { index += 1; break; }
                    else { index += 1; }
                }
            }
            b'{' => { depth += 1; index += 1; }
            b'}' => {
                depth -= 1;
                if depth == 0 { return Some(index); }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn class_scopes(source: &str) -> Vec<ClassScope> {
    let known_owners: BTreeSet<&str> = DEFINITIONS.iter().map(|entry| entry.owner).collect();
    let mut scopes = Vec::new();
    for declaration in class_declaration_regex().captures_iter(source) {
        let (Some(whole), Some(class_name), Some(header)) = (
            declaration.get(0),
            declaration.name("name"),
            declaration.name("header"),
        ) else { continue; };
        let Some(open) = whole.end().checked_sub(1) else { continue; };
        let Some(close) = matching_class_brace(source, open) else { continue; };
        let body = &source[open + 1..close];
        let class_name = class_name.as_str().to_owned();

        // Track every declared superclass/interface so inherited mod base classes
        // can be resolved after the complete source file has been indexed.
        let mut parents = Vec::new();
        for capture in inheritance_list_regex().captures_iter(header.as_str()) {
            if let Some(list) = capture.name("owners") {
                parents.extend(list.as_str().split(',').map(str::trim)
                    .map(|owner| owner.split('<').next().unwrap_or(owner).trim().to_owned())
                    .filter(|owner| !owner.is_empty()));
            }
        }

        let owners: BTreeSet<String> = parents.iter()
            .flat_map(|parent| owner_candidates(parent))
            .filter(|owner| known_owners.contains(owner.as_str()))
            .collect();

        let mut receivers = BTreeMap::<String, BTreeSet<String>>::new();
        for capture in typed_variable_regex().captures_iter(body) {
            let (Some(owner), Some(variable)) = (capture.name("owner"), capture.name("variable")) else { continue; };
            let candidates = owner_candidates(owner.as_str());
            if candidates.iter().any(|candidate| known_owners.contains(candidate.as_str())) {
                receivers.entry(variable.as_str().to_owned()).or_default().extend(candidates);
            }
        }
        scopes.push(ClassScope {
            name: class_name,
            start: open + 1,
            end: close,
            parents,
            owners,
            receivers,
        });
    }

    // Build a same-file inheritance graph and resolve it transitively. The
    // visiting set prevents malformed/cyclic bytecode decompilation output from
    // causing recursion loops.
    let parent_map: BTreeMap<String, Vec<String>> = scopes.iter()
        .map(|scope| (scope.name.clone(), scope.parents.clone()))
        .collect();

    fn inherited_owners(
        parents: &[String],
        parent_map: &BTreeMap<String, Vec<String>>,
        known_owners: &BTreeSet<&str>,
        visiting: &mut BTreeSet<String>,
        output: &mut BTreeSet<String>,
    ) {
        for parent in parents {
            let candidates = owner_candidates(parent);
            output.extend(candidates.into_iter().filter(|owner| known_owners.contains(owner.as_str())));
            let short = parent.rsplit('.').next().unwrap_or(parent);
            if let Some(next) = parent_map.get(short) {
                if visiting.insert(short.to_owned()) {
                    inherited_owners(next, parent_map, known_owners, visiting, output);
                    visiting.remove(short);
                }
            }
        }
    }

    for scope in &mut scopes {
        let mut visiting = BTreeSet::new();
        visiting.insert(scope.name.clone());
        inherited_owners(&scope.parents, &parent_map, &known_owners, &mut visiting, &mut scope.owners);
    }
    scopes
}

fn innermost_class_scope(scopes: &[ClassScope], position: usize) -> Option<&ClassScope> {
    scopes.iter()
        .filter(|scope| scope.start <= position && position < scope.end)
        .min_by_key(|scope| scope.end.saturating_sub(scope.start))
}

/// Identify API calls using the declared receiver type or a static API owner.
/// This supplements method-override detection so ItemStack/NBT/World/registry
/// calls inside ordinary methods can be normalized too.
pub fn map_legacy_api_usages(source: &str, version: Option<&str>) -> Vec<LegacyApiMethodMapping> {
    let Some(version) = version.map(canonical_version).transpose_option() else {
        return Vec::new();
    };

    let known_owners: BTreeSet<&str> = DEFINITIONS.iter().map(|entry| entry.owner).collect();
    let scopes = class_scopes(source);
    let mut matched = BTreeMap::<(String, String), LegacyApiMethodMapping>::new();

    for capture in invocation_regex().captures_iter(source) {
        let (Some(receiver), Some(method), Some(whole)) = (
            capture.name("receiver"),
            capture.name("name"),
            capture.get(0),
        ) else { continue; };
        let receiver = receiver.as_str();
        let method = method.as_str();
        let scope = innermost_class_scope(&scopes, whole.start());

        let owners = if known_owners.contains(receiver) {
            owner_candidates(receiver)
        } else if receiver == "this" || receiver == "super" {
            scope.map(|scope| scope.owners.clone()).unwrap_or_default()
        } else {
            scope.and_then(|scope| scope.receivers.get(receiver).cloned()).unwrap_or_default()
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
        r"(?m)^[ \t]*(?:(?:@[A-Za-z_$][A-Za-z0-9_$.]*(?:\([^)]*\))?)[ \t]*\r?\n[ \t]*)*(?:(?:public|protected|private)\s+)?(?:(?:static|final|synchronized|native|abstract|default)\s+)*(?:<[^>\r\n]+>\s*)?[A-Za-z_$][A-Za-z0-9_$.<>\[\], ?]*[ \t]+(?P<name>[A-Za-z_$][A-Za-z0-9_$]*)[ \t]*\("
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

    let scopes = class_scopes(source);
    let mut matched = BTreeMap::<(String, String), LegacyApiMethodMapping>::new();
    for capture in declaration_regex().captures_iter(source) {
        let (Some(name), Some(whole)) = (capture.name("name"), capture.get(0)) else { continue; };
        let Some(scope) = innermost_class_scope(&scopes, whole.start()) else { continue; };
        for definition in DEFINITIONS.iter().filter(|entry| {
            entry.method == name.as_str()
                && scope.owners.contains(entry.owner)
                && version.map(|v| entry.versions.contains(&v)).unwrap_or(true)
        }) {
            matched.entry((definition.owner.to_owned(), definition.method.to_owned()))
                .or_insert_with(|| to_mapping(definition));
        }
    }
    matched.into_values().collect()
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
        assert!(all.len() >= 700, "expected broad API coverage, got {}", all.len());
        for (version, minimum) in [("1.7.10", 540), ("1.8.9", 660), ("1.12.2", 680)] {
            assert!(legacy_api_dictionary_for_version(version).len() >= minimum,
                "coverage too small for {version}: {}", legacy_api_dictionary_for_version(version).len());
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
    fn matches_methods_only_against_their_own_class_hierarchy() {
        let source = r#"
            class ItemOverride extends Item {
                public boolean hitEntity(ItemStack stack, EntityLivingBase target, EntityLivingBase attacker) { return true; }
            }
            class UnrelatedHelper {
                public boolean hitEntity(ItemStack stack, EntityLivingBase target, EntityLivingBase attacker) { return false; }
            }
            class BlockOverride extends Block {
                public void onBlockActivated(World world, BlockPos pos) { }
            }
        "#;
        let found = map_legacy_api_methods(source, Some("1.12.2"));
        assert!(found.iter().any(|entry| entry.source_owner == "Item" && entry.source_method == "hitEntity"));
        assert!(found.iter().any(|entry| entry.source_owner == "Block" && entry.source_method == "onBlockActivated"));
    }

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

    #[test]
    fn recognizes_package_private_overrides_from_decompiled_sources() {
        let source = r#"
            class ModItemBase extends Item {
                void sharedHelper() { }
            }
            class ExampleItem extends ModItemBase {
                boolean hitEntity(ItemStack stack, EntityLivingBase target, EntityLivingBase attacker) {
                    return true;
                }
            }
        "#;
        let found = map_legacy_api_methods(source, Some("1.12.2"));
        assert!(found.iter().any(|entry| entry.source_owner == "Item" && entry.source_method == "hitEntity"));
    }

    #[test]
    fn detects_methods_from_multiple_implemented_interfaces() {
        let source = r#"
            class ExampleInventory implements ISidedInventory, IInventory {
                public boolean canInsertItem(int slot, ItemStack stack, int side) { return true; }
            }
        "#;
        let found = map_legacy_api_methods(source, Some("1.12.2"));
        assert!(found.iter().any(|entry| entry.source_owner == "ISidedInventory"
            && entry.source_method == "canInsertItem"));
    }

    #[test]
    fn legacy_mappings_resolve_target_signatures_and_semantics() {
        let mapping = legacy_api_dictionary().into_iter()
            .find(|entry| entry.source_owner == "Item" && entry.source_method == "onItemRightClick")
            .unwrap();
        assert!(mapping.target_api_methods.iter().any(|target|
            target.owner == "Item"
                && target.method == "use"
                && target.signature.contains("InteractionHand")
                && target.semantic_contract.contains("resulting stack")
        ));
        assert!(mapping.target_api_methods.iter().all(|target|
            target.documentation_url.contains("/neoforge/1.21.1/")
        ));
    }

    #[test]
    fn special_ability_hooks_are_version_scoped_and_documented() {
        let v17 = legacy_api_dictionary_for_version("1.7.10");
        let v18 = legacy_api_dictionary_for_version("1.8.9");
        let v112 = legacy_api_dictionary_for_version("1.12.2");
        let has = |entries: &[LegacyApiMethodMapping], owner: &str, method: &str| {
            entries.iter().any(|entry| entry.source_owner == owner && entry.source_method == method)
        };
        assert!(has(&v17, "Potion", "performEffect"));
        assert!(has(&v17, "EntityThrowable", "onImpact"));
        assert!(has(&v17, "EntityLivingBase", "getActivePotionEffects"));
        assert!(has(&v17, "PotionEffect", "getPotionID"));
        assert!(!has(&v17, "EntityLivingBase", "getRevengeTarget"));
        assert!(has(&v18, "PotionEffect", "getIsShowParticles"));
        assert!(!has(&v18, "EntityLivingBase", "getRevengeTarget"));
        assert!(has(&v112, "EntityLivingBase", "getRevengeTarget"));
        assert!(has(&v112, "EntityLivingBase", "getLastAttackedEntity"));
        assert!(has(&v112, "EntityLivingBase", "setActiveHand"));
        assert!(has(&v112, "EntityLivingBase", "getLastDamageSource"));
        assert!(has(&v112, "PotionEffect", "getCurativeItems"));
        assert!(has(&v112, "PotionEffect", "setCurativeItems"));
        assert!(has(&v112, "Potion", "getPotionFromResourceLocation"));
        assert!(!has(&v112, "PotionEffect", "getPotionID"));
        assert!(has(&v112, "Enchantment", "onEntityDamaged"));
        assert!(has(&v112, "Enchantment", "isCurse"));
        assert!(has(&v112, "EntityArrow", "onHit"));
        assert!(has(&v112, "DamageSource", "setDamageBypassesArmor"));
    }


    #[test]
    fn forge_event_contracts_are_version_scoped() {
        let v17 = legacy_api_dictionary_for_version("1.7.10");
        let v18 = legacy_api_dictionary_for_version("1.8.9");
        let v112 = legacy_api_dictionary_for_version("1.12.2");
        let has = |entries: &[LegacyApiMethodMapping], owner: &str, method: &str| {
            entries.iter().any(|entry| entry.source_owner == owner && entry.source_method == method)
        };
        assert!(has(&v17, "Event", "setCanceled"));
        assert!(has(&v18, "LivingFallEvent", "setDamageMultiplier"));
        assert!(has(&v18, "ArrowLooseEvent", "getCharge"));
        assert!(has(&v18, "BlockEvent.HarvestDropsEvent", "getDrops"));
        assert!(has(&v112, "LivingEntityUseItemEvent", "setDuration"));
        assert!(has(&v112, "CriticalHitEvent", "setDamageModifier"));
        assert!(has(&v112, "ItemFishedEvent", "damageRodBy"));
        assert!(has(&v112, "BlockEvent.CropGrowEvent.Pre", "getState"));
        assert!(has(&v112, "ExplosionEvent.Detonate", "getAffectedBlocks"));
        assert!(has(&v112, "LootTableLoadEvent", "setTable"));
        assert!(!has(&v17, "LootTableLoadEvent", "setTable"));
    }


    #[test]
    fn concrete_forge_event_types_resolve_inherited_api_usages() {
        let source = r#"
            class EventConsumer {
                void consume(LivingHealEvent heal, PlayerEvent.Clone clone,
                             BlockEvent.CropGrowEvent.Pre grow,
                             ChunkDataEvent.Load chunk) {
                    heal.getEntityLiving();
                    clone.getPlayer();
                    grow.getWorld();
                    chunk.getChunk();
                    chunk.getWorld();
                }
            }
        "#;
        let found = map_legacy_api_usages(source, Some("1.12.2"));
        let has = |owner: &str, method: &str| found.iter()
            .any(|entry| entry.source_owner == owner && entry.source_method == method);
        assert!(has("LivingEvent", "getEntityLiving"));
        assert!(has("PlayerEvent", "getPlayer"));
        assert!(has("BlockEvent", "getWorld"));
        assert!(has("ChunkEvent", "getChunk"));
        assert!(has("WorldEvent", "getWorld"));
    }


    #[test]
    fn entity_lifecycle_and_ability_events_are_indexed() {
        let v112 = legacy_api_dictionary_for_version("1.12.2");
        let has = |owner: &str, method: &str| v112.iter()
            .any(|entry| entry.source_owner == owner && entry.source_method == method);
        for (owner, method) in [
            ("EntityEvent", "getEntity"),
            ("EntityEvent.CanUpdate", "setCanUpdate"),
            ("EntityEvent.EnteringChunk", "getNewChunkX"),
            ("EntityMountEvent", "getEntityBeingMounted"),
            ("EntityTravelToDimensionEvent", "getDimension"),
            ("PlaySoundAtEntityEvent", "setSound"),
            ("ProjectileImpactEvent", "getRayTraceResult"),
            ("AnimalTameEvent", "getTamer"),
            ("BabyEntitySpawnEvent", "setChild"),
            ("LivingDestroyBlockEvent", "getState"),
            ("PotionEvent.PotionApplicableEvent", "getPotionEffect"),
            ("PlayerDestroyItemEvent", "getOriginal"),
            ("PlayerContainerEvent", "getContainer"),
            ("PlayerEvent.Visibility", "modifyVisibility"),
            ("EnchantmentLevelSetEvent", "setLevel"),
        ] {
            assert!(has(owner, method), "missing {owner}#{method}");
        }
    }


    #[test]
    fn forge_client_render_and_brewing_hooks_are_indexed() {
        let v112 = legacy_api_dictionary_for_version("1.12.2");
        let has = |owner: &str, method: &str| v112.iter()
            .any(|entry| entry.source_owner == owner && entry.source_method == method);
        for (owner, method) in [
            ("EntityViewRenderEvent", "getRenderPartialTicks"),
            ("EntityViewRenderEvent.CameraSetup", "setRoll"),
            ("EntityViewRenderEvent.FogColors", "setRed"),
            ("EntityViewRenderEvent.FogDensity", "setDensity"),
            ("EntityViewRenderEvent.FOVModifier", "setFOV"),
            ("FOVUpdateEvent", "setNewfov"),
            ("GuiOpenEvent", "setGui"),
            ("InputUpdateEvent", "getMovementInput"),
            ("MouseEvent", "getDwheel"),
            ("DrawBlockHighlightEvent", "getTarget"),
            ("RenderGameOverlayEvent", "getType"),
            ("RenderTooltipEvent", "getLines"),
            ("RenderPlayerEvent", "getRenderer"),
            ("RenderWorldLastEvent", "getContext"),
            ("PotionBrewEvent", "setItem"),
            ("PlayerBrewedPotionEvent", "getStack"),
            ("PlayerEvent.ItemPickupEvent", "getOriginalEntity"),
        ] {
            assert!(has(owner, method), "missing {owner}#{method}");
        }
        assert!(!has("PlayerEvent.PlayerChangedDimensionEvent", "getFromDim"));
        assert!(!has("PlayerEvent.ItemSmeltedEvent", "getSmelting"));
    }

    #[test]
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
