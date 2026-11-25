//! Structured NeoForge 1.21.1 API dictionary used as the target-side porting reference.
//!
//! This complements the versioned Forge source dictionary. Entries record the
//! target method signature, semantic contract and JavaDoc class page. Signatures
//! intentionally cover the overload used by the converter; callers still need
//! to inspect source call sites where overload resolution is ambiguous.

use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetApiMethod {
    pub minecraft_version: String,
    pub loader: String,
    pub owner: String,
    pub method: String,
    pub signature: String,
    pub category: String,
    pub semantic_contract: String,
    pub documentation_url: String,
    pub legacy_aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Definition {
    owner: &'static str,
    method: &'static str,
    signature: &'static str,
    category: &'static str,
    contract: &'static str,
    docs: &'static str,
    aliases: &'static str,
}

macro_rules! t {
    ($owner:literal, $method:literal, $signature:literal, $category:literal, $contract:literal, $docs:literal, $aliases:literal) => {
        Definition {
            owner: $owner,
            method: $method,
            signature: $signature,
            category: $category,
            contract: $contract,
            docs: $docs,
            aliases: $aliases,
        }
    };
}

const DEFINITIONS: &[Definition] = &[
    // Item lifecycle, interaction, tooltip and durability.
    t!("Item", "use", "InteractionResultHolder<ItemStack> use(Level, Player, InteractionHand)", "item_interaction", "Return both the interaction result and the resulting stack; preserve client prediction and hand selection.", "net/minecraft/world/item/Item.html", "Item#onItemRightClick"),
    t!("Item", "useOn", "InteractionResult useOn(UseOnContext)", "item_interaction", "UseOnContext carries level, player, hand, stack, clicked position, face and hit location.", "net/minecraft/world/item/Item.html", "Item#onItemUse"),
    t!("Item", "inventoryTick", "void inventoryTick(ItemStack, Level, Entity, int, boolean)", "item_lifecycle", "Inventory tick is not the dropped ItemEntity tick; preserve slot and selected state.", "net/minecraft/world/item/Item.html", "Item#onUpdate"),
    t!("Item", "finishUsingItem", "ItemStack finishUsingItem(ItemStack, Level, LivingEntity)", "item_lifecycle", "Return the resulting stack; consumption, remainder and effects belong in this hook.", "net/minecraft/world/item/Item.html", "Item#onItemUseFinish; ItemFood#onEaten; Item#onEaten"),
    t!("Item", "releaseUsing", "void releaseUsing(ItemStack, Level, LivingEntity, int)", "item_lifecycle", "The final int is remaining use time; preserve charge duration and use-animation timing.", "net/minecraft/world/item/Item.html", "Item#onPlayerStoppedUsing"),
    t!("Item", "onUseTick", "void onUseTick(Level, LivingEntity, ItemStack, int)", "item_lifecycle", "Receives remaining use duration; avoid reversing countdown-based source conditions.", "net/minecraft/world/item/Item.html", "Item#onUsingTick"),
    t!("Item", "hurtEnemy", "boolean hurtEnemy(ItemStack, LivingEntity, LivingEntity)", "combat", "The boolean reports whether the item handled durability/attack behavior; translate attacker and target separately.", "net/minecraft/world/item/Item.html", "Item#hitEntity"),
    t!("Item", "mineBlock", "boolean mineBlock(ItemStack, Level, BlockState, BlockPos, LivingEntity)", "tool", "Preserve block state, position, mining entity and stack damage semantics.", "net/minecraft/world/item/Item.html", "Item#onBlockDestroyed"),
    t!("Item", "interactLivingEntity", "InteractionResult interactLivingEntity(ItemStack, Player, LivingEntity, InteractionHand)", "item_interaction", "Use a typed result and preserve hand, target and client-side prediction.", "net/minecraft/world/item/Item.html", "Item#itemInteractionForEntity"),
    t!("Item", "appendHoverText", "void appendHoverText(ItemStack, Item.TooltipContext, List<Component>, TooltipFlag)", "tooltip", "Build Components, not legacy formatted Strings; preserve tooltip order and flag-dependent content.", "net/minecraft/world/item/Item.html", "Item#addInformation"),
    t!("Item", "getName", "Component getName(ItemStack)", "localization", "Return a stack-sensitive Component; custom names and translation keys remain distinct.", "net/minecraft/world/item/Item.html", "Item#getItemStackDisplayName"),
    t!("Item", "isFoil", "boolean isFoil(ItemStack)", "rendering", "The query is stack-sensitive; constant foil can often use ENCHANTMENT_GLINT_OVERRIDE.", "net/minecraft/world/item/Item.html", "Item#hasEffect"),
    t!("Item", "isBarVisible", "boolean isBarVisible(ItemStack)", "durability", "Controls durability bar visibility independently of width and color.", "net/minecraft/world/item/Item.html", "Item#showDurabilityBar"),
    t!("Item", "getBarWidth", "int getBarWidth(ItemStack)", "durability", "Return the 0-13 bar width; convert legacy normalized remaining-durability fractions.", "net/minecraft/world/item/Item.html", "Item#getDurabilityForDisplay"),
    t!("Item", "getBarColor", "int getBarColor(ItemStack)", "durability", "Return packed RGB color; preserve source hue interpolation.", "net/minecraft/world/item/Item.html", "Item#getRGBDurabilityForDisplay"),
    t!("Item", "getUseAnimation", "UseAnim getUseAnimation(ItemStack)", "item_presentation", "Map legacy EnumAction to UseAnim while retaining stack-dependent animation.", "net/minecraft/world/item/Item.html", "Item#getItemUseAction"),
    t!("Item", "getUseDuration", "int getUseDuration(ItemStack, LivingEntity)", "item_presentation", "The target query is stack/entity-sensitive; preserve legacy duration and charge logic.", "net/minecraft/world/item/Item.html", "Item#getMaxItemUseDuration"),
    t!("Item", "getDestroySpeed", "float getDestroySpeed(ItemStack, BlockState)", "tool", "Preserve stack-sensitive mining speed and block-state conditions.", "net/minecraft/world/item/Item.html", "Item#getStrVsBlock"),
    t!("Item", "isCorrectToolForDrops", "boolean isCorrectToolForDrops(ItemStack, BlockState)", "tool", "Translate harvest-level logic to Tool rules and tags rather than a single numeric tier.", "net/minecraft/world/item/Item.html", "Item#canHarvestBlock"),
    t!("Item", "onCraftedBy", "void onCraftedBy(ItemStack, Level, Player)", "item_lifecycle", "Preserve crafting context and item stack initialization.", "net/minecraft/world/item/Item.html", "Item#onCreated"),
    t!("IItemExtension", "getBurnTime", "int getBurnTime(ItemStack, RecipeType<?>)", "fuel", "Recipe-type-aware and stack-aware burn time; do not collapse custom fuel logic into a single global integer.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#getItemBurnTime; IFuelHandler#getBurnTime"),
    t!("IItemExtension", "getCraftingRemainingItem", "ItemStack getCraftingRemainingItem(ItemStack)", "crafting", "The stack-aware extension supports dynamic container items; static remainders use Item.Properties#craftRemainder.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#getContainerItem; Item#hasContainerItem"),
    t!("IItemExtension", "getDefaultAttributeModifiers", "ItemAttributeModifiers getDefaultAttributeModifiers(ItemStack)", "attributes", "Build modern attribute entries with equipment slots, operations and stack-sensitive conditions.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#getItemAttributeModifiers; Item#getAttributeModifiers"),
    t!("IItemExtension", "getEnchantmentValue", "int getEnchantmentValue(ItemStack)", "enchanting", "Use the stack-sensitive extension for item enchantability.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#getItemEnchantability"),
    t!("IItemExtension", "getArmorTexture", "ResourceLocation getArmorTexture(ItemStack, Entity, EquipmentSlot, ArmorMaterial.Layer, boolean)", "armor", "Preserve equipment slot, material layer and inner-model selection.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#getArmorTexture"),
    t!("IItemExtension", "doesSneakBypassUse", "boolean doesSneakBypassUse(ItemStack, LevelReader, BlockPos, Player)", "item_interaction", "The modern hook has read-only level and stack-aware arguments.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#doesSneakBypassUse"),
    t!("IItemExtension", "onItemUseFirst", "InteractionResult onItemUseFirst(ItemStack, UseOnContext)", "item_interaction", "Runs before normal block interaction; preserve PASS versus terminal results.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#onItemUseFirst"),
    t!("IItemExtension", "onEntityItemUpdate", "boolean onEntityItemUpdate(ItemStack, ItemEntity)", "item_lifecycle", "True means the dropped item update is handled; preserve cancellation semantics.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#onEntityItemUpdate"),
    t!("IItemExtension", "onLeftClickEntity", "boolean onLeftClickEntity(ItemStack, Player, Entity)", "combat", "The boolean controls/cancels normal left-click entity processing; preserve attacker context.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#onLeftClickEntity"),
    t!("IItemExtension", "getEntityLifespan", "int getEntityLifespan(ItemStack, Level)", "item_entity", "The value is stack- and level-sensitive; preserve per-stack dropped item lifetime.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#getEntityLifespan"),
    t!("IItemExtension", "hasCustomEntity", "boolean hasCustomEntity(ItemStack)", "item_entity", "Pair with createEntity; preserve custom dropped item entity detection.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#hasCustomEntity"),
    t!("IItemExtension", "createEntity", "Entity createEntity(Level, Entity, ItemStack)", "item_entity", "Return a replacement entity or null to keep the original ItemEntity.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#createEntity"),
    t!("IItemExtension", "getFoodProperties", "FoodProperties getFoodProperties(ItemStack, LivingEntity)", "food", "The returned properties are stack/entity-sensitive; represent nutrition, saturation, effects and always-edible behavior.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "ItemFood#getHealAmount; ItemFood#getSaturationModifier"),
    t!("IItemExtension", "canPerformAction", "boolean canPerformAction(ItemStack, ItemAbility)", "tool", "Use item abilities for tool actions instead of legacy tool-class strings.", "net/neoforged/neoforge/common/extensions/IItemExtension.html", "Item#isItemTool; Item#canItemEditBlocks"),

    // Block behavior, shape, placement, redstone and block entities.
    t!("Block", "use", "InteractionResult use(BlockState, Level, BlockPos, Player, InteractionHand, BlockHitResult)", "block_interaction", "Translate right-click behavior to UseOnContext-like hit data and typed interaction results.", "net/minecraft/world/level/block/Block.html", "Block#onBlockActivated"),
    t!("Block", "getStateForPlacement", "BlockState getStateForPlacement(BlockPlaceContext)", "block_state", "Return the state to place; do not mutate metadata after placement.", "net/minecraft/world/level/block/Block.html", "Block#onBlockPlaced; Block#getStateFromMeta"),
    t!("Block", "onPlace", "void onPlace(BlockState, Level, BlockPos, BlockState, boolean)", "block_lifecycle", "Called when the new state is placed; distinguish from chunk-load restoration.", "net/minecraft/world/level/block/Block.html", "Block#onBlockAdded"),
    t!("Block", "onRemove", "void onRemove(BlockState, Level, BlockPos, BlockState, boolean)", "block_lifecycle", "Called for state replacement/removal; compare old and new states before tearing down block entities.", "net/minecraft/world/level/block/Block.html", "Block#breakBlock"),
    t!("Block", "setPlacedBy", "void setPlacedBy(Level, BlockPos, BlockState, LivingEntity, ItemStack)", "block_lifecycle", "Runs after placement with placer and item stack context.", "net/minecraft/world/level/block/Block.html", "Block#onBlockPlacedBy"),
    t!("Block", "neighborChanged", "void neighborChanged(BlockState, Level, BlockPos, Block, BlockPos, boolean)", "block_updates", "Preserve neighbor source position and update ordering.", "net/minecraft/world/level/block/Block.html", "Block#onNeighborBlockChange"),
    t!("Block", "entityInside", "void entityInside(BlockState, Level, BlockPos, Entity)", "block_interaction", "Runs when an entity intersects the block; preserve per-tick and logical-side behavior.", "net/minecraft/world/level/block/Block.html", "Block#onEntityCollidedWithBlock"),
    t!("Block", "stepOn", "void stepOn(Level, BlockPos, BlockState, Entity)", "block_interaction", "Runs when an entity walks on the block; preserve contact timing and velocity changes.", "net/minecraft/world/level/block/Block.html", "Block#onEntityWalking"),
    t!("Block", "fallOn", "void fallOn(Level, BlockState, BlockPos, Entity, float)", "block_interaction", "Preserve fall distance and damage semantics.", "net/minecraft/world/level/block/Block.html", "Block#onFallenUpon"),
    t!("Block", "tick", "void tick(BlockState, ServerLevel, BlockPos, RandomSource)", "block_updates", "Scheduled ticks run on the server; preserve delay and state transitions.", "net/minecraft/world/level/block/Block.html", "Block#updateTick"),
    t!("Block", "randomTick", "void randomTick(BlockState, ServerLevel, BlockPos, RandomSource)", "block_updates", "Random tick execution is server-side; preserve random tick speed assumptions.", "net/minecraft/world/level/block/Block.html", "Block#randomTick"),
    t!("Block", "animateTick", "void animateTick(BlockState, Level, BlockPos, RandomSource)", "block_rendering", "Client visual particles/sounds belong here, not in server tick logic.", "net/minecraft/world/level/block/Block.html", "Block#randomDisplayTick"),
    t!("Block", "playerWillDestroy", "void playerWillDestroy(Level, BlockPos, BlockState, Player)", "block_lifecycle", "Runs before player destruction; preserve harvest-tool and drop ordering.", "net/minecraft/world/level/block/Block.html", "Block#onBlockPreDestroy; Block#onBlockHarvested"),
    t!("Block", "wasExploded", "void wasExploded(Level, BlockPos, Explosion)", "block_lifecycle", "Keep explosion context and block interaction semantics.", "net/minecraft/world/level/block/Block.html", "Block#onBlockDestroyedByExplosion"),
    t!("Block", "getDrops", "List<ItemStack> getDrops(BlockState, LootParams.Builder)", "block_drops", "Prefer loot tables for static rules; preserve tool/looting/entity context in LootParams.", "net/minecraft/world/level/block/Block.html", "Block#getDrops"),
    t!("Block", "getExpDrop", "int getExpDrop(BlockState, LevelReader, RandomSource, BlockPos, int, int)", "block_drops", "Preserve silk touch, fortune and random XP behavior.", "net/minecraft/world/level/block/Block.html", "Block#getExpDrop"),
    t!("Block", "getCloneItemStack", "ItemStack getCloneItemStack(LevelReader, BlockPos, BlockState)", "block_drops", "The result is the stack returned by middle-click/pick-block; keep hit and state context.", "net/minecraft/world/level/block/Block.html", "Block#getPickBlock"),
    t!("Block", "getShape", "VoxelShape getShape(BlockState, BlockGetter, BlockPos, CollisionContext)", "block_shape", "Defines selection/outline geometry and may depend on state and context.", "net/minecraft/world/level/block/Block.html", "Block#getSelectedBoundingBox"),
    t!("Block", "getCollisionShape", "VoxelShape getCollisionShape(BlockState, BlockGetter, BlockPos, CollisionContext)", "block_shape", "Defines entity collision independently from visual/outline shape.", "net/minecraft/world/level/block/Block.html", "Block#getCollisionBoundingBox; Block#addCollisionBoxesToList"),
    t!("Block", "getOcclusionShape", "VoxelShape getOcclusionShape(BlockState, BlockGetter, BlockPos)", "block_shape", "Controls face occlusion and lighting; it is not the collision shape.", "net/minecraft/world/level/block/Block.html", "Block#isOpaqueCube; Block#isFullCube"),
    t!("Block", "skipRendering", "boolean skipRendering(BlockState, BlockState, Direction)", "block_rendering", "Neighbor-aware face culling replaces old side-invisibility checks.", "net/minecraft/world/level/block/Block.html", "Block#isSideInvisible"),
    t!("Block", "getSignal", "int getSignal(BlockState, BlockGetter, BlockPos, Direction)", "redstone", "Weak redstone signal; preserve direction and state-dependent strength.", "net/minecraft/world/level/block/Block.html", "Block#isProvidingWeakPower; Block#getWeakPower"),
    t!("Block", "getDirectSignal", "int getDirectSignal(BlockState, BlockGetter, BlockPos, Direction)", "redstone", "Strong/direct signal is distinct from weak signal.", "net/minecraft/world/level/block/Block.html", "Block#isProvidingStrongPower; Block#getStrongPower"),
    t!("Block", "getAnalogOutputSignal", "int getAnalogOutputSignal(BlockState, Level, BlockPos)", "redstone", "Comparator output is state and level aware.", "net/minecraft/world/level/block/Block.html", "Block#getComparatorInputOverride"),
    t!("Block", "hasAnalogOutputSignal", "boolean hasAnalogOutputSignal(BlockState)", "redstone", "Declare analog output separately from the current signal value.", "net/minecraft/world/level/block/Block.html", "Block#hasComparatorInputOverride"),
    t!("Block", "isSignalSource", "boolean isSignalSource(BlockState)", "redstone", "Signal source predicate is distinct from emitted strength.", "net/minecraft/world/level/block/Block.html", "Block#canProvidePower"),
    t!("EntityBlock", "newBlockEntity", "BlockEntity newBlockEntity(BlockPos, BlockState)", "block_entity", "Create the block entity for this block; registration ties it to a BlockEntityType.", "net/minecraft/world/level/block/EntityBlock.html", "Block#createTileEntity"),
    t!("EntityBlock", "getTicker", "BlockEntityTicker<T> getTicker(Level, BlockState, BlockEntityType<T>)", "block_entity", "Return a ticker only for the intended type and side; client/server tick separation matters.", "net/minecraft/world/level/block/EntityBlock.html", "TileEntity#updateEntity"),

    // Entity, living entity, mob and AI.
    t!("Entity", "tick", "void tick()", "entity_lifecycle", "Preserve superclass tick ordering; use baseTick only for equivalent environmental updates.", "net/minecraft/world/entity/Entity.html", "Entity#onUpdate"),
    t!("Entity", "baseTick", "void baseTick()", "entity_lifecycle", "Environmental/base tick work is distinct from subclass tick logic.", "net/minecraft/world/entity/Entity.html", "Entity#onEntityUpdate"),
    t!("Entity", "defineSynchedData", "void defineSynchedData(SynchedEntityData.Builder)", "entity_data", "Declare typed EntityDataAccessor values in the builder before instances are tracked.", "net/minecraft/world/entity/Entity.html", "Entity#entityInit"),
    t!("Entity", "readAdditionalSaveData", "void readAdditionalSaveData(CompoundTag)", "persistence", "Read only custom persistent fields; preserve inherited entity save data.", "net/minecraft/world/entity/Entity.html", "Entity#readEntityFromNBT"),
    t!("Entity", "addAdditionalSaveData", "void addAdditionalSaveData(CompoundTag)", "persistence", "Write only custom persistent fields; preserve inherited entity save data.", "net/minecraft/world/entity/Entity.html", "Entity#writeEntityToNBT"),
    t!("Entity", "move", "void move(MoverType, Vec3)", "entity_movement", "The movement type and clipped displacement affect collision/step behavior.", "net/minecraft/world/entity/Entity.html", "Entity#moveEntity"),
    t!("Entity", "push", "void push(Entity)", "entity_collision", "Preserve reciprocal entity push behavior and avoid double-applying impulses.", "net/minecraft/world/entity/Entity.html", "Entity#applyEntityCollision"),
    t!("Entity", "isPickable", "boolean isPickable()", "entity_collision", "Selection/picking is different from physical collision.", "net/minecraft/world/entity/Entity.html", "Entity#canBeCollidedWith"),
    t!("Entity", "isPushable", "boolean isPushable()", "entity_collision", "Physical pushability is different from selection/picking.", "net/minecraft/world/entity/Entity.html", "Entity#canBePushed"),
    t!("Entity", "playerTouch", "void playerTouch(Player)", "entity_interaction", "Preserve server-side pickup/interaction timing.", "net/minecraft/world/entity/Entity.html", "Entity#onCollideWithPlayer"),
    t!("Entity", "discard", "void discard()", "entity_lifecycle", "Remove the entity without implying death/drop behavior.", "net/minecraft/world/entity/Entity.html", "Entity#setDead"),
    t!("Entity", "distanceTo", "float distanceTo(Entity)", "entity_math", "Returns linear distance; do not substitute squared distance.", "net/minecraft/world/entity/Entity.html", "Entity#getDistanceToEntity"),
    t!("Entity", "distanceToSqr", "double distanceToSqr(Entity)", "entity_math", "Returns squared distance; preserve callers' squared thresholds.", "net/minecraft/world/entity/Entity.html", "Entity#getDistanceSqToEntity"),
    t!("LivingEntity", "aiStep", "void aiStep()", "living_lifecycle", "Living tick includes AI, effects, movement and animation; separate client/server sections.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#onLivingUpdate"),
    t!("LivingEntity", "heal", "void heal(float)", "living_combat", "Preserve healing source and max-health clamping.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#heal"),
    t!("LivingEntity", "addEffect", "boolean addEffect(MobEffectInstance, Entity)", "effects", "Preserve source entity, hidden effects, ambient flags and application events.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#addPotionEffect"),
    t!("LivingEntity", "removeEffect", "boolean removeEffect(MobEffect)", "effects", "Remove by registered MobEffect and preserve removal callbacks.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#removePotionEffect"),
    t!("LivingEntity", "hasEffect", "boolean hasEffect(MobEffect)", "effects", "Query active effect by registered effect identity.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#isPotionActive"),
    t!("LivingEntity", "getEffect", "MobEffectInstance getEffect(MobEffect)", "effects", "Return the active effect instance or null; preserve duration/amplifier checks.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#getActivePotionEffect"),
    t!("LivingEntity", "getItemInHand", "ItemStack getItemInHand(InteractionHand)", "equipment", "Hand selection is explicit; main hand is not always the intended source slot.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#getHeldItem"),
    t!("LivingEntity", "getItemBySlot", "ItemStack getItemBySlot(EquipmentSlot)", "equipment", "Translate legacy numeric armor slots to EquipmentSlot.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#getEquipmentInSlot"),
    t!("LivingEntity", "setItemSlot", "void setItemSlot(EquipmentSlot, ItemStack)", "equipment", "Preserve equipment-change events and item attribute modifier refresh.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#setCurrentItemOrArmor"),
    t!("LivingEntity", "swing", "void swing(InteractionHand)", "living_animation", "Map hand and preserve swing animation/network packet behavior.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#swingItem"),
    t!("LivingEntity", "knockback", "void knockback(double, double, double)", "living_combat", "Preserve source direction, strength and knockback resistance.", "net/minecraft/world/entity/LivingEntity.html", "EntityLivingBase#knockBack"),
    t!("LivingEntity", "startUsingItem", "void startUsingItem(InteractionHand)", "item_lifecycle", "Starts item use for the selected hand and synchronizes use state.", "net/minecraft/world/entity/LivingEntity.html", "Item#onItemRightClick"),
    t!("LivingEntity", "stopUsingItem", "void stopUsingItem()", "item_lifecycle", "Stop active use and invoke the held item's releaseUsing behavior.", "net/minecraft/world/entity/LivingEntity.html", "Item#onPlayerStoppedUsing"),
    t!("LivingEntity", "getUseItem", "ItemStack getUseItem()", "item_lifecycle", "Return the current use stack, which may not be the current main-hand stack.", "net/minecraft/world/entity/LivingEntity.html", "Item#getMaxItemUseDuration"),
    t!("Mob", "registerGoals", "void registerGoals()", "entity_ai", "Register Goal instances with explicit priority and target selector ordering.", "net/minecraft/world/entity/Mob.html", "EntityLiving#initEntityAI"),
    t!("Mob", "createAttributes", "AttributeSupplier.Builder createAttributes()", "living_attributes", "Declare default attributes during entity type attribute registration.", "net/minecraft/world/entity/Mob.html", "EntityLiving#applyEntityAttributes"),
    t!("Mob", "finalizeSpawn", "SpawnGroupData finalizeSpawn(ServerLevelAccessor, DifficultyInstance, MobSpawnType, SpawnGroupData)", "spawning", "Preserve spawn reason, local difficulty and group data.", "net/minecraft/world/entity/Mob.html", "EntityLiving#onInitialSpawn"),
    t!("Mob", "checkSpawnRules", "boolean checkSpawnRules(LevelAccessor, MobSpawnType)", "spawning", "Use SpawnPlacements and preserve spawn reason/light/difficulty restrictions.", "net/minecraft/world/entity/Mob.html", "EntityLiving#getCanSpawnHere"),
    t!("Mob", "setTarget", "void setTarget(LivingEntity)", "entity_ai", "Preserve target selector callbacks and last-hurt-by state.", "net/minecraft/world/entity/Mob.html", "EntityLiving#setAttackTarget"),
    t!("Mob", "getTarget", "LivingEntity getTarget()", "entity_ai", "Current combat target is distinct from revenge/last-attacker state.", "net/minecraft/world/entity/Mob.html", "EntityLiving#getAttackTarget"),
    t!("Mob", "getNavigation", "PathNavigation getNavigation()", "entity_ai", "Use the appropriate ground/flying/water navigation implementation.", "net/minecraft/world/entity/Mob.html", "EntityLiving#getNavigator"),
    t!("Mob", "getSensing", "Sensing getSensing()", "entity_ai", "Preserve cached visibility checks and selector tick timing.", "net/minecraft/world/entity/Mob.html", "EntityLiving#getEntitySenses"),
    t!("Mob", "getMoveControl", "MoveControl getMoveControl()", "entity_ai", "Port movement helper logic to MoveControl operations.", "net/minecraft/world/entity/Mob.html", "EntityLiving#getMoveHelper"),
    t!("Mob", "getLookControl", "LookControl getLookControl()", "entity_ai", "Port look helper logic with rotation limits and desired look position.", "net/minecraft/world/entity/Mob.html", "EntityLiving#getLookHelper"),
    t!("Goal", "canUse", "boolean canUse()", "entity_ai", "Initial goal eligibility; do not conflate with canContinueToUse.", "net/minecraft/world/entity/ai/goal/Goal.html", "EntityAIBase#shouldExecute"),
    t!("Goal", "canContinueToUse", "boolean canContinueToUse()", "entity_ai", "Continuation predicate is evaluated after the goal starts.", "net/minecraft/world/entity/ai/goal/Goal.html", "EntityAIBase#shouldContinueExecuting"),
    t!("Goal", "start", "void start()", "entity_ai", "One-time start effects; preserve selector scheduling.", "net/minecraft/world/entity/ai/goal/Goal.html", "EntityAIBase#startExecuting"),
    t!("Goal", "stop", "void stop()", "entity_ai", "Cleanup and navigation cancellation; preserve reset timing.", "net/minecraft/world/entity/ai/goal/Goal.html", "EntityAIBase#resetTask"),
    t!("Goal", "tick", "void tick()", "entity_ai", "Per-tick goal behavior; preserve selector order and tick cadence.", "net/minecraft/world/entity/ai/goal/Goal.html", "EntityAIBase#updateTask"),
    t!("Goal", "setFlags", "void setFlags(EnumSet<Goal.Flag>)", "entity_ai", "Translate legacy mutex bits to MOVE/LOOK/JUMP/TARGET flags.", "net/minecraft/world/entity/ai/goal/Goal.html", "EntityAIBase#setMutexBits"),

    // Level, chunks, biome, weather and redstone.
    t!("Level", "setBlock", "boolean setBlock(BlockPos, BlockState, int)", "world", "Update flags control client sync, neighbor notifications and block entity updates.", "net/minecraft/world/level/Level.html", "World#setBlock; World#setBlockState"),
    t!("Level", "getBlockState", "BlockState getBlockState(BlockPos)", "world", "Returns immutable BlockState; replace metadata/material checks with properties/tags.", "net/minecraft/world/level/Level.html", "World#getBlock; World#getBlockState"),
    t!("Level", "removeBlock", "boolean removeBlock(BlockPos, boolean)", "world", "Preserve moving flag and block entity removal/update behavior.", "net/minecraft/world/level/Level.html", "World#setBlockToAir"),
    t!("Level", "destroyBlock", "boolean destroyBlock(BlockPos, boolean, Entity, int)", "world", "Preserve drops, breaker entity and recursion guard behavior.", "net/minecraft/world/level/Level.html", "World#destroyBlock"),
    t!("Level", "hasChunkAt", "boolean hasChunkAt(BlockPos)", "world", "Use non-loading checks when source only tested loaded chunks.", "net/minecraft/world/level/Level.html", "World#isBlockLoaded"),
    t!("Level", "getChunk", "LevelChunk getChunk(int, int)", "world", "Explicitly choose whether chunk lookup may load missing chunks.", "net/minecraft/world/level/Level.html", "World#getChunk; World#getChunkFromChunkCoords"),
    t!("Level", "addFreshEntity", "boolean addFreshEntity(Entity)", "world", "Respects entity validity, side and level insertion rules.", "net/minecraft/world/level/Level.html", "World#spawnEntityInWorld"),
    t!("Level", "playSound", "void playSound(Player, BlockPos, SoundEvent, SoundSource, float, float)", "audio", "Preserve excluded/recipient player, sound category, volume and pitch.", "net/minecraft/world/level/Level.html", "World#playSoundAtEntity; World#playSoundEffect"),
    t!("Level", "addParticle", "void addParticle(ParticleOptions, double, double, double, double, double, double)", "particles", "Client-only particle addition; server broadcasts use sendParticles.", "net/minecraft/world/level/Level.html", "World#spawnParticle"),
    t!("Level", "getGameTime", "long getGameTime()", "world", "Total game time is distinct from daylight-cycle time.", "net/minecraft/world/level/Level.html", "World#getTotalWorldTime"),
    t!("Level", "getDayTime", "long getDayTime()", "world", "Dimension-local day time; preserve daylight-cycle behavior.", "net/minecraft/world/level/Level.html", "World#getWorldTime"),
    t!("Level", "hasNeighborSignal", "boolean hasNeighborSignal(BlockPos)", "redstone", "Neighbor signal predicate is distinct from direct signal strength.", "net/minecraft/world/level/Level.html", "World#isBlockPowered"),
    t!("Level", "getSignal", "int getSignal(BlockPos, Direction)", "redstone", "Read signal strength from a specific neighbor direction.", "net/minecraft/world/level/Level.html", "World#getStrongPower"),
    t!("Level", "scheduleTick", "void scheduleTick(BlockPos, Block, int)", "block_updates", "Preserve delay and target tick scheduling rules.", "net/minecraft/world/level/Level.html", "World#scheduleBlockUpdate"),
    t!("Level", "sendBlockUpdated", "void sendBlockUpdated(BlockPos, BlockState, BlockState, int)", "networking", "Update flags govern state packet and neighbor propagation.", "net/minecraft/world/level/Level.html", "World#markBlockForUpdate"),
    t!("Level", "getBlockEntity", "BlockEntity getBlockEntity(BlockPos)", "block_entity", "Returns the currently attached block entity or null; preserve unloaded chunk behavior.", "net/minecraft/world/level/Level.html", "World#getTileEntity"),
    t!("Level", "setBlockEntity", "void setBlockEntity(BlockEntity)", "block_entity", "Attach entity at its BlockPos and ensure state/type compatibility.", "net/minecraft/world/level/Level.html", "World#setTileEntity"),
    t!("Level", "getBiome", "Holder<Biome> getBiome(BlockPos)", "biomes", "Use holder/key identity and climate settings rather than raw Biome singleton checks.", "net/minecraft/world/level/Level.html", "World#getBiomeGenForCoords"),
    t!("Level", "getHeightmapPos", "BlockPos getHeightmapPos(Heightmap.Types, BlockPos)", "world", "Select the correct heightmap type: motion blocking, ocean floor, world surface, etc.", "net/minecraft/world/level/Level.html", "World#getTopSolidOrLiquidBlock; World#getPrecipitationHeight"),
    t!("Level", "getCurrentDifficultyAt", "DifficultyInstance getCurrentDifficultyAt(BlockPos)", "world", "Local difficulty depends on inhabited time and world difficulty.", "net/minecraft/world/level/Level.html", "World#getDifficultyForLocation"),
    t!("Level", "isRaining", "boolean isRaining()", "world_weather", "Preserve dimension weather and biome precipitation distinctions.", "net/minecraft/world/level/Level.html", "World#isRaining"),
    t!("Level", "isThundering", "boolean isThundering()", "world_weather", "Preserve thunder state independent from rain level.", "net/minecraft/world/level/Level.html", "World#isThundering"),
    t!("Level", "getRainLevel", "float getRainLevel(float)", "world_weather", "The argument is partial tick; preserve interpolation.", "net/minecraft/world/level/Level.html", "World#getRainStrength"),
    t!("Level", "getThunderLevel", "float getThunderLevel(float)", "world_weather", "The argument is partial tick; preserve interpolation.", "net/minecraft/world/level/Level.html", "World#getThunderStrength"),
    t!("ServerLevel", "sendParticles", "int sendParticles(ServerPlayer, ParticleOptions, boolean, double, double, double, int, double, double, double, double)", "particles", "Broadcast particles to selected recipients with server-authoritative position/count.", "net/minecraft/server/level/ServerLevel.html", "World#spawnParticle"),
    t!("LevelChunk", "getBlockState", "BlockState getBlockState(BlockPos)", "chunk", "Prefer Level access where load and heightmap side effects matter.", "net/minecraft/world/level/chunk/LevelChunk.html", "Chunk#getBlockState"),
    t!("LevelChunk", "setBlockState", "BlockState setBlockState(BlockPos, BlockState, boolean)", "chunk", "Direct chunk writes bypass some level update paths; use only when preserving chunk-internal behavior.", "net/minecraft/world/level/chunk/LevelChunk.html", "Chunk#setBlockState"),
    t!("WorldBorder", "isWithinBounds", "boolean isWithinBounds(BlockPos)", "world", "Use world border APIs rather than hard-coded coordinate limits.", "net/minecraft/world/level/border/WorldBorder.html", "WorldBorder#contains"),

    // Block entities and persistence.
    t!("BlockEntity", "loadAdditional", "void loadAdditional(CompoundTag, HolderLookup.Provider)", "persistence", "Modern data loading receives a registry lookup provider; do not replace base load behavior.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#readFromNBT"),
    t!("BlockEntity", "saveAdditional", "void saveAdditional(CompoundTag, HolderLookup.Provider)", "persistence", "Write only custom fields and preserve registry-aware serialization.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#writeToNBT"),
    t!("BlockEntity", "getUpdatePacket", "ClientboundBlockEntityDataPacket getUpdatePacket()", "networking", "Provide client update data separately from disk save data.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#getDescriptionPacket"),
    t!("BlockEntity", "getUpdateTag", "CompoundTag getUpdateTag(HolderLookup.Provider)", "networking", "Only serialize client-visible state; don't expose private persistent data.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#getUpdateTag"),
    t!("BlockEntity", "handleUpdateTag", "void handleUpdateTag(CompoundTag, HolderLookup.Provider)", "networking", "Apply client update data without overwriting server-only state.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#handleUpdateTag"),
    t!("BlockEntity", "setRemoved", "void setRemoved()", "block_entity", "Invalidate capabilities and stop tick/render participation on removal.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#invalidate"),
    t!("BlockEntity", "setChanged", "void setChanged()", "block_entity", "Mark chunk data dirty; request client updates where required.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#markDirty"),
    t!("BlockEntity", "onLoad", "void onLoad()", "block_entity", "Runs after attachment/load; preserve once-only initialization.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#validate"),
    t!("BlockEntity", "getBlockPos", "BlockPos getBlockPos()", "block_entity", "Use the canonical position rather than storing duplicate coordinates.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#getPos"),
    t!("BlockEntity", "getLevel", "Level getLevel()", "block_entity", "Can be null before attachment or after removal.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#getWorldObj"),
    t!("BlockEntity", "getBlockState", "BlockState getBlockState()", "block_entity", "Use current state to inspect state properties rather than numeric metadata.", "net/minecraft/world/level/block/entity/BlockEntity.html", "TileEntity#getBlockMetadata"),

    // Inventories, fluids, energy and capabilities.
    t!("IItemHandler", "getSlots", "int getSlots()", "inventory", "Expose a stable slot count; do not assume a backing Container exists.", "net/neoforged/neoforge/items/IItemHandler.html", "IInventory#getSizeInventory"),
    t!("IItemHandler", "getStackInSlot", "ItemStack getStackInSlot(int)", "inventory", "Return a safe stack view/copy according to handler contract.", "net/neoforged/neoforge/items/IItemHandler.html", "IInventory#getStackInSlot"),
    t!("IItemHandler", "insertItem", "ItemStack insertItem(int, ItemStack, boolean)", "inventory", "Return the remaining stack; simulate must not mutate inventory.", "net/neoforged/neoforge/items/IItemHandler.html", "IInventory#setInventorySlotContents"),
    t!("IItemHandler", "extractItem", "ItemStack extractItem(int, int, boolean)", "inventory", "Simulated extraction must not mutate inventory; honor slot and item limits.", "net/neoforged/neoforge/items/IItemHandler.html", "IInventory#decrStackSize"),
    t!("IItemHandler", "getSlotLimit", "int getSlotLimit(int)", "inventory", "Per-slot limits may differ from the inventory-wide stack limit.", "net/neoforged/neoforge/items/IItemHandler.html", "IInventory#getInventoryStackLimit"),
    t!("IItemHandler", "isItemValid", "boolean isItemValid(int, ItemStack)", "inventory", "Slot validity is independent of current slot contents.", "net/neoforged/neoforge/items/IItemHandler.html", "IInventory#isItemValidForSlot"),
    t!("IFluidHandler", "getTanks", "int getTanks()", "fluids", "Tank count and per-tank properties should be preserved.", "net/neoforged/neoforge/fluids/capability/IFluidHandler.html", "IFluidHandler#getTankProperties"),
    t!("IFluidHandler", "getFluidInTank", "FluidStack getFluidInTank(int)", "fluids", "Return the contents of an indexed tank without mutating it.", "net/neoforged/neoforge/fluids/capability/IFluidHandler.html", "IFluidHandler#getTankProperties"),
    t!("IFluidHandler", "getTankCapacity", "int getTankCapacity(int)", "fluids", "Capacity is per tank and uses Forge/NeoForge fluid units.", "net/neoforged/neoforge/fluids/capability/IFluidHandler.html", "IFluidHandler#getTankProperties"),
    t!("IFluidHandler", "fill", "int fill(FluidStack, FluidAction)", "fluids", "EXECUTE mutates; SIMULATE must be side-effect free.", "net/neoforged/neoforge/fluids/capability/IFluidHandler.html", "IFluidHandler#fill"),
    t!("IFluidHandler", "drain", "FluidStack drain(FluidStack, FluidAction)", "fluids", "Preserve exact fluid matching and simulation semantics.", "net/neoforged/neoforge/fluids/capability/IFluidHandler.html", "IFluidHandler#drain"),
    t!("IFluidHandler", "drain", "FluidStack drain(int, FluidAction)", "fluids", "Amount-based drain may choose the available fluid; stack-based drain must match fluid identity.", "net/neoforged/neoforge/fluids/capability/IFluidHandler.html", "IFluidHandler#drain"),
    t!("IEnergyStorage", "receiveEnergy", "int receiveEnergy(int, boolean)", "energy", "The boolean selects simulation; preserve Forge Energy units.", "net/neoforged/neoforge/energy/IEnergyStorage.html", "IEnergyStorage#receiveEnergy"),
    t!("IEnergyStorage", "extractEnergy", "int extractEnergy(int, boolean)", "energy", "The boolean selects simulation; preserve max extraction and storage rules.", "net/neoforged/neoforge/energy/IEnergyStorage.html", "IEnergyStorage#extractEnergy"),
    t!("IEnergyStorage", "getEnergyStored", "int getEnergyStored()", "energy", "Return stored energy, not capacity or transfer rate.", "net/neoforged/neoforge/energy/IEnergyStorage.html", "IEnergyStorage#getEnergyStored"),
    t!("IEnergyStorage", "getMaxEnergyStored", "int getMaxEnergyStored()", "energy", "Return capacity, not the current stored value.", "net/neoforged/neoforge/energy/IEnergyStorage.html", "IEnergyStorage#getMaxEnergyStored"),
    t!("RegisterCapabilitiesEvent", "registerBlock", "void registerBlock(Capability<T>, BlockCapability<T, C>, BlockCapability.Collector<T, C>, Block...)", "capabilities", "Register typed block capabilities during the capability registration event.", "net/neoforged/neoforge/capabilities/RegisterCapabilitiesEvent.html", "CapabilityManager#register; ICapabilityProvider#getCapability"),
    t!("RegisterCapabilitiesEvent", "registerItem", "void registerItem(Capability<T>, ItemCapability<T, C>, ItemCapability.Collector<T, C>, Item...)", "capabilities", "Register typed item capabilities; avoid per-stack legacy provider construction at arbitrary times.", "net/neoforged/neoforge/capabilities/RegisterCapabilitiesEvent.html", "CapabilityManager#register; ICapabilityProvider#getCapability"),

    // Registries, events and networking.
    t!("DeferredRegister", "register", "RegistryObject<T> register(String, Supplier<? extends T>)", "registration", "Create deferred registry entries from stable names and suppliers.", "net/neoforged/neoforge/registries/DeferredRegister.html", "GameRegistry#registerItem; GameRegistry#registerBlock"),
    t!("DeferredRegister", "register", "void register(IEventBus)", "registration", "Attach the deferred register to the mod event bus during construction.", "net/neoforged/neoforge/registries/DeferredRegister.html", "GameRegistry#registerItem; GameRegistry#registerBlock"),
    t!("RegisterEvent", "register", "void register(ResourceKey<? extends Registry<T>>, ResourceLocation, Supplier<T>)", "registration", "Register by registry key and ResourceLocation during the registration event.", "net/neoforged/neoforge/registries/RegisterEvent.html", "GameRegistry#register"),
    t!("IEventBus", "addListener", "void addListener(Consumer<E>)", "events", "Register typed listeners on the correct mod or game event bus.", "net/neoforged/bus/api/IEventBus.html", "EventBus#register"),
    t!("IEventBus", "register", "void register(Object)", "events", "Register annotated subscribers on the correct bus; static subscribers need lifecycle care.", "net/neoforged/bus/api/IEventBus.html", "MinecraftForge#EVENT_BUS; EventBus#register"),
    t!("IEventBus", "post", "boolean post(Event)", "events", "Preserve cancellation and event result handling; post on the intended bus.", "net/neoforged/bus/api/IEventBus.html", "EventBus#post"),
    t!("CustomPacketPayload", "type", "Type<? extends CustomPacketPayload> type()", "networking", "Payload type is a stable namespaced identifier used by registration and codec dispatch.", "net/minecraft/network/protocol/common/custom/CustomPacketPayload.html", "IMessage"),
    t!("PayloadRegistrar", "playToServer", "void playToServer(Type<T>, StreamCodec<? super RegistryFriendlyByteBuf,T>, handler)", "networking", "Register a typed client-to-server payload with validation and thread scheduling.", "net/neoforged/neoforge/network/registration/PayloadRegistrar.html", "SimpleNetworkWrapper#sendToServer; IMessageHandler#onMessage"),
    t!("PayloadRegistrar", "playToClient", "void playToClient(Type<T>, StreamCodec<? super RegistryFriendlyByteBuf,T>, handler)", "networking", "Register a typed server-to-client payload and validate recipient/direction.", "net/neoforged/neoforge/network/registration/PayloadRegistrar.html", "SimpleNetworkWrapper#sendTo; SimpleNetworkWrapper#sendToAll"),
    t!("PacketDistributor", "sendToPlayer", "void sendToPlayer(ServerPlayer, CustomPacketPayload)", "networking", "Distribute a payload to a specific server player.", "net/neoforged/neoforge/network/PacketDistributor.html", "SimpleNetworkWrapper#sendTo"),
    t!("PacketDistributor", "sendToServer", "void sendToServer(CustomPacketPayload)", "networking", "Send a client payload to server; server handler must validate the sender.", "net/neoforged/neoforge/network/PacketDistributor.html", "SimpleNetworkWrapper#sendToServer"),
    t!("PacketDistributor", "sendToAllPlayers", "void sendToAllPlayers(CustomPacketPayload)", "networking", "Broadcast to connected players and preserve level/dimension filtering where required.", "net/neoforged/neoforge/network/PacketDistributor.html", "SimpleNetworkWrapper#sendToAll"),
    t!("StreamCodec", "encode", "void encode(B, V)", "networking", "Encode fields in the same order expected by decode and validate bounds.", "net/minecraft/network/codec/StreamCodec.html", "IMessage#toBytes"),
    t!("StreamCodec", "decode", "V decode(B)", "networking", "Decode a typed payload and validate lengths, enum ordinals and registry references.", "net/minecraft/network/codec/StreamCodec.html", "IMessage#fromBytes"),

    // Menus, slots and recipes.
    t!("AbstractContainerMenu", "quickMoveStack", "ItemStack quickMoveStack(Player, int)", "menus", "Translate slot ranges and merge direction; preserve the empty-stack return contract.", "net/minecraft/world/inventory/AbstractContainerMenu.html", "Container#transferStackInSlot"),
    t!("AbstractContainerMenu", "stillValid", "boolean stillValid(Player)", "menus", "Validate distance, block identity, ownership and spectator access.", "net/minecraft/world/inventory/AbstractContainerMenu.html", "Container#canInteractWith"),
    t!("AbstractContainerMenu", "slotsChanged", "void slotsChanged(Container)", "menus", "Recompute crafting and other derived state after backing inventory changes.", "net/minecraft/world/inventory/AbstractContainerMenu.html", "Container#detectAndSendChanges"),
    t!("AbstractContainerMenu", "removed", "void removed(Player)", "menus", "Close menu and return/drop carried items where needed.", "net/minecraft/world/inventory/AbstractContainerMenu.html", "Container#onContainerClosed"),
    t!("AbstractContainerMenu", "broadcastChanges", "void broadcastChanges()", "menus", "Synchronize slots and data slots to listeners.", "net/minecraft/world/inventory/AbstractContainerMenu.html", "Container#detectAndSendChanges"),
    t!("AbstractContainerMenu", "clicked", "void clicked(int, int, ClickType, Player)", "menus", "Preserve click type, carried stack and slot index semantics.", "net/minecraft/world/inventory/AbstractContainerMenu.html", "Container#slotClick"),
    t!("Slot", "mayPlace", "boolean mayPlace(ItemStack)", "menus", "Slot-specific insertion filter; inventory-wide validity is not enough.", "net/minecraft/world/inventory/Slot.html", "IInventory#isItemValidForSlot"),
    t!("Slot", "mayPickup", "boolean mayPickup(Player)", "menus", "Preserve ownership, lock and extraction restrictions.", "net/minecraft/world/inventory/Slot.html", "ISidedInventory#canExtractItem"),
    t!("Recipe", "matches", "boolean matches(I, Level)", "recipes", "Use the correct RecipeInput type and level context.", "net/minecraft/world/item/crafting/Recipe.html", "IRecipe#matches"),
    t!("Recipe", "assemble", "ItemStack assemble(I, HolderLookup.Provider)", "recipes", "Return the crafting result and use registry-aware lookup for dynamic outputs.", "net/minecraft/world/item/crafting/Recipe.html", "IRecipe#getCraftingResult"),
    t!("Recipe", "canCraftInDimensions", "boolean canCraftInDimensions(int, int)", "recipes", "Preserve shaped/shapeless dimension constraints.", "net/minecraft/world/item/crafting/Recipe.html", "IRecipe#getRecipeSize"),
    t!("Recipe", "getResultItem", "ItemStack getResultItem(HolderLookup.Provider)", "recipes", "Return the preview/result stack with registry-aware components.", "net/minecraft/world/item/crafting/Recipe.html", "IRecipe#getRecipeOutput"),
    t!("Recipe", "getIngredients", "NonNullList<Ingredient> getIngredients()", "recipes", "Preserve ingredient ordering and empty-slot semantics.", "net/minecraft/world/item/crafting/Recipe.html", "IRecipe#getRecipeSize"),
    t!("Recipe", "getId", "ResourceLocation getId()", "recipes", "Use namespaced identifiers for serializer/recipe registry identity.", "net/minecraft/world/item/crafting/Recipe.html", "IRecipe#getRegistryName"),

    // Expanded event, networking, models, screens, commands and capability APIs.
    t!("RegisterPayloadHandlersEvent", "registrar", "PayloadRegistrar registrar(String)", "networking", "Obtain a versioned payload registrar during the mod-bus registration event.", "net/neoforged/neoforge/network/event/RegisterPayloadHandlersEvent.html", "SimpleNetworkWrapper#registerMessage"),
    t!("PayloadRegistrar", "playBidirectional", "void playBidirectional(CustomPacketPayload.Type<T>, StreamCodec<B,T>, handler)", "networking", "Use a typed CustomPacketPayload and StreamCodec for both directions; enqueue game-state work on the correct thread.", "net/neoforged/neoforge/network/registration/PayloadRegistrar.html", "SimpleNetworkWrapper#registerMessage; IMessageHandler#onMessage"),
    t!("BakedModel", "getQuads", "List<BakedQuad> getQuads(BlockState, Direction, RandomSource, ModelData, RenderType)", "models", "May receive null state/direction for item or unculled queries; cache expensive geometry because this is called frequently.", "net/minecraft/client/resources/model/BakedModel.html", "Block#getExtendedState; ModelBakeEvent#getModelRegistry"),
    t!("BakedModel", "getModelData", "ModelData getModelData(BlockAndTintGetter, BlockPos, BlockState, ModelData)", "models", "Supply block-entity-driven model properties separately from block state; keep data immutable/cached.", "net/minecraft/client/resources/model/BakedModel.html", "Block#getExtendedState"),
    t!("BakedModel", "getRenderTypes", "ChunkRenderTypeSet getRenderTypes(BlockState, RandomSource, ModelData)", "models", "Return render layers used by the model; preserve cutout/translucent behavior.", "net/minecraft/client/resources/model/BakedModel.html", "Block#getBlockLayer"),
    t!("BakedModel", "applyTransform", "BakedModel applyTransform(ItemDisplayContext, PoseStack, boolean)", "models", "Translate legacy item transforms into modern display contexts and pose-stack transforms.", "net/minecraft/client/resources/model/BakedModel.html", "Item#isFull3D; Item#setFull3D"),
    t!("IForgeBakedModel", "getModelData", "ModelData getModelData(BlockAndTintGetter, BlockPos, BlockState, ModelData)", "models", "NeoForge extension hook exposes additional model data to baked models.", "net/neoforged/neoforge/client/extensions/IForgeBakedModel.html", "Block#getExtendedState"),
    t!("ModelEvent.RegisterAdditional", "register", "void register(ResourceLocation)", "models", "Register additional unbaked model locations before model baking.", "net/neoforged/neoforge/client/event/ModelEvent.RegisterAdditional.html", "ModelRegistryEvent"),
    t!("ModelEvent.RegisterGeometryLoaders", "register", "void register(ResourceLocation, IGeometryLoader<?>)", "models", "Register a named custom geometry loader for resource model JSON.", "net/neoforged/neoforge/client/event/ModelEvent.RegisterGeometryLoaders.html", "ModelLoader#setCustomModelResourceLocation"),
    t!("ModelEvent.ModifyBakingResult", "getModels", "Map<ModelResourceLocation, BakedModel> getModels()", "models", "Modify baked model results after model loading; preserve registry key and bake ordering.", "net/neoforged/neoforge/client/event/ModelEvent.ModifyBakingResult.html", "ModelBakeEvent#getModelRegistry"),
    t!("EntityRenderersEvent.RegisterRenderers", "registerEntityRenderer", "void registerEntityRenderer(EntityType<T>, EntityRendererProvider<T>)", "rendering", "Register an EntityRendererProvider during the client mod event bus lifecycle.", "net/neoforged/neoforge/client/event/EntityRenderersEvent.RegisterRenderers.html", "RenderingRegistry#registerEntityRenderingHandler"),
    t!("EntityRenderersEvent.RegisterRenderers", "registerBlockEntityRenderer", "void registerBlockEntityRenderer(BlockEntityType<T>, BlockEntityRendererProvider<T>)", "rendering", "Register a BlockEntityRenderer provider for a registered BlockEntityType.", "net/neoforged/neoforge/client/event/EntityRenderersEvent.RegisterRenderers.html", "ClientRegistry#bindTileEntitySpecialRenderer"),
    t!("BlockEntityRenderer", "render", "void render(T, float, PoseStack, MultiBufferSource, int, int)", "rendering", "Modern block entity renderers receive pose stack, buffers, packed light and overlay.", "net/minecraft/client/renderer/blockentity/BlockEntityRenderer.html", "TileEntitySpecialRenderer#render"),
    t!("LivingEntityRenderer", "render", "void render(T, float, float, PoseStack, MultiBufferSource, int)", "rendering", "Preserve model layers, animation setup and render-state extraction.", "net/minecraft/client/renderer/entity/LivingEntityRenderer.html", "RenderLivingBase#doRender"),
    t!("ModelPart", "render", "void render(PoseStack, VertexConsumer, int, int, int)", "models", "Render model parts using the modern pose stack and vertex consumer.", "net/minecraft/client/model/geom/ModelPart.html", "ModelRenderer#render"),
    t!("ModelPart", "setPos", "void setPos(float, float, float)", "models", "Set a pivot position; preserve parent/child hierarchy and coordinate units.", "net/minecraft/client/model/geom/ModelPart.html", "ModelRenderer#setRotationPoint"),
    t!("CubeListBuilder", "addBox", "CubeListBuilder addBox(float, float, float, float, float, float)", "models", "Build a model cube with local coordinates; preserve inflate and mirror settings where applicable.", "net/minecraft/client/model/geom/builders/CubeListBuilder.html", "ModelRenderer#addBox"),
    t!("EntityModel", "setupAnim", "void setupAnim(T, float, float, float, float, float)", "models", "Reconstruct animation state from limb swing, amount, age, yaw and pitch.", "net/minecraft/client/model/EntityModel.html", "ModelBase#render"),
    t!("IClientItemExtensions", "initializeClient", "void initializeClient(Consumer<IClientItemExtensions>)", "rendering", "Register item-specific client extensions without loading client-only classes on a dedicated server.", "net/neoforged/neoforge/client/extensions/common/IClientItemExtensions.html", "Item#getColorFromItemStack; Item#requiresMultipleRenderPasses"),
    t!("Screen", "init", "void init()", "client_gui", "Initialize widgets after scaled screen dimensions are calculated.", "net/minecraft/client/gui/screens/Screen.html", "GuiScreen#initGui"),
    t!("Screen", "render", "void render(GuiGraphics, int, int, float)", "client_gui", "Render with GuiGraphics and scaled mouse coordinates.", "net/minecraft/client/gui/screens/Screen.html", "GuiScreen#drawScreen"),
    t!("Screen", "mouseClicked", "boolean mouseClicked(double, double, int)", "client_gui", "Coordinates are GUI-scaled; return whether an input component handled the click.", "net/minecraft/client/gui/screens/Screen.html", "GuiScreen#mouseClicked"),
    t!("Screen", "keyPressed", "boolean keyPressed(int, int, int)", "client_gui", "Key codes and modifiers are separate from typed characters.", "net/minecraft/client/gui/screens/Screen.html", "GuiScreen#keyTyped"),
    t!("Screen", "charTyped", "boolean charTyped(char, int)", "client_gui", "Handle text input separately from key presses.", "net/minecraft/client/gui/screens/Screen.html", "GuiScreen#keyTyped"),
    t!("Minecraft", "setScreen", "void setScreen(Screen)", "client_gui", "Screen changes are client-only and must not replace server-authoritative menu state.", "net/minecraft/client/Minecraft.html", "Minecraft#displayGuiScreen"),
    t!("RegisterCommandsEvent", "getDispatcher", "CommandDispatcher<CommandSourceStack> getDispatcher()", "commands", "Register Brigadier commands on the server command registration event.", "net/neoforged/neoforge/event/RegisterCommandsEvent.html", "ICommand#processCommand"),
    t!("CommandSourceStack", "getLevel", "ServerLevel getLevel()", "commands", "Resolve the command's current server level from the execution source.", "net/minecraft/commands/CommandSourceStack.html", "ICommandSender#getEntityWorld"),
    t!("CommandSourceStack", "getPlayerOrException", "ServerPlayer getPlayerOrException()", "commands", "Resolve a player-only command source with explicit exception behavior.", "net/minecraft/commands/CommandSourceStack.html", "ICommandSender#getName"),
    t!("CommandSourceStack", "hasPermission", "boolean hasPermission(int)", "commands", "Use permission requirements in Brigadier command builders.", "net/minecraft/commands/CommandSourceStack.html", "ICommand#canCommandSenderUseCommand"),
    t!("CommandSourceStack", "sendSuccess", "void sendSuccess(Supplier<Component>, boolean)", "commands", "Send feedback with broadcast-to-operators semantics.", "net/minecraft/commands/CommandSourceStack.html", "ICommand#processCommand"),
    t!("Capabilities.ItemHandler", "BLOCK", "BlockCapability<IItemHandler, @Nullable Direction>", "capabilities", "Query a block inventory capability with optional side context.", "net/neoforged/neoforge/capabilities/Capabilities.ItemHandler.html", "ICapabilityProvider#getCapability; IInventory"),
    t!("Capabilities.ItemHandler", "ITEM", "ItemCapability<IItemHandler, Void>", "capabilities", "Query the inventory exposed by an item stack.", "net/neoforged/neoforge/capabilities/Capabilities.ItemHandler.html", "ICapabilityProvider#getCapability; IInventory"),
    t!("Capabilities.FluidHandler", "BLOCK", "BlockCapability<IFluidHandler, @Nullable Direction>", "capabilities", "Query a block fluid handler with optional side context.", "net/neoforged/neoforge/capabilities/Capabilities.FluidHandler.html", "ICapabilityProvider#getCapability; IFluidHandler"),
    t!("Capabilities.EnergyStorage", "BLOCK", "BlockCapability<IEnergyStorage, @Nullable Direction>", "capabilities", "Query a block energy handler with optional side context.", "net/neoforged/neoforge/capabilities/Capabilities.EnergyStorage.html", "ICapabilityProvider#getCapability; IEnergyStorage"),

    // Attributes, data components, effects and world generation.
    t!("AttributeSupplier.Builder", "add", "Builder add(Attribute, double)", "living_attributes", "Register base entity attributes when constructing EntityType attribute suppliers.", "net/minecraft/world/entity/ai/attributes/AttributeSupplier.html", "EntityLiving#applyEntityAttributes"),
    t!("AttributeInstance", "addPermanentModifier", "void addPermanentModifier(AttributeModifier)", "living_attributes", "Preserve modifier UUID/key, amount, operation and permanence.", "net/minecraft/world/entity/ai/attributes/AttributeInstance.html", "EntityLivingBase#getEntityAttribute"),
    t!("AttributeInstance", "removeModifier", "void removeModifier(ResourceLocation)", "living_attributes", "Remove the exact modifier key rather than a same-valued modifier.", "net/minecraft/world/entity/ai/attributes/AttributeInstance.html", "EntityLivingBase#getEntityAttribute"),
    t!("DataComponents", "MAX_DAMAGE", "DataComponentType<Integer> MAX_DAMAGE", "item_data", "Stack-level durability component; preserve dynamic per-stack overrides.", "net/minecraft/core/component/DataComponents.html", "Item#setMaxDamage; Item#getMaxDamage"),
    t!("DataComponents", "MAX_STACK_SIZE", "DataComponentType<Integer> MAX_STACK_SIZE", "item_data", "Stack-level max stack size; static limits belong in Item.Properties#stacksTo.", "net/minecraft/core/component/DataComponents.html", "Item#setMaxStackSize; Item#getItemStackLimit"),
    t!("DataComponents", "CUSTOM_NAME", "DataComponentType<Component> CUSTOM_NAME", "localization", "Represent custom names with Components and preserve translation fallback.", "net/minecraft/core/component/DataComponents.html", "ItemStack#setStackDisplayName"),
    t!("DataComponents", "CUSTOM_DATA", "DataComponentType<CustomData> CUSTOM_DATA", "persistence", "Store mod-owned custom stack data without replacing other components.", "net/minecraft/core/component/DataComponents.html", "ItemStack#getTagCompound; ItemStack#setTagCompound"),
    t!("DataComponents", "ENCHANTMENT_GLINT_OVERRIDE", "DataComponentType<Boolean> ENCHANTMENT_GLINT_OVERRIDE", "rendering", "Use only for constant/stack-level glint override behavior.", "net/minecraft/core/component/DataComponents.html", "Item#hasEffect"),
    t!("MobEffectInstance", "getDuration", "int getDuration()", "effects", "Preserve tick units and infinite-duration representation.", "net/minecraft/world/effect/MobEffectInstance.html", "PotionEffect#getDuration"),
    t!("MobEffectInstance", "getAmplifier", "int getAmplifier()", "effects", "Preserve zero-based amplifier.", "net/minecraft/world/effect/MobEffectInstance.html", "PotionEffect#getAmplifier"),
    t!("BiomeModifier", "modify", "void modify(Holder<Biome>, Phase, ModifiableBiomeInfo.Builder)", "worldgen", "Use biome modifiers to alter spawn settings/features during datapack loading.", "net/neoforged/neoforge/common/world/BiomeModifier.html", "IWorldGenerator#generate; GameRegistry#registerWorldGenerator"),
    t!("FMLCommonSetupEvent", "enqueueWork", "void enqueueWork(Runnable)", "lifecycle", "Queue common setup work after registries are ready; don't register objects late.", "net/neoforged/fml/event/lifecycle/FMLCommonSetupEvent.html", "FMLPreInitializationEvent; FMLInitializationEvent"),
    t!("InterModEnqueueEvent", "enqueueWork", "void enqueueWork(Runnable)", "lifecycle", "Enqueue inter-mod messages in the dedicated lifecycle phase.", "net/neoforged/fml/event/lifecycle/InterModEnqueueEvent.html", "FMLPostInitializationEvent"),
];

fn to_public(entry: &Definition) -> TargetApiMethod {
    let aliases = entry.aliases.split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    TargetApiMethod {
        minecraft_version: "1.21.1".to_owned(),
        loader: "NeoForge".to_owned(),
        owner: entry.owner.to_owned(),
        method: entry.method.to_owned(),
        signature: entry.signature.to_owned(),
        category: entry.category.to_owned(),
        semantic_contract: entry.contract.to_owned(),
        documentation_url: format!("https://lexxie.dev/neoforge/1.21.1/{}", entry.docs),
        legacy_aliases: aliases,
    }
}

/// Return the full structured target API reference used for NeoForge 1.21.1 ports.
pub fn target_api_dictionary() -> Vec<TargetApiMethod> {
    DEFINITIONS.iter().map(to_public).collect()
}

/// Return target methods matching the owner and method name exactly.
pub fn find_target_api_methods(owner: &str, method: &str) -> Vec<TargetApiMethod> {
    DEFINITIONS.iter()
        .filter(|entry| entry.owner == owner && entry.method == method)
        .map(to_public)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_dictionary_has_broad_cross_domain_coverage() {
        let entries = target_api_dictionary();
        assert!(entries.len() >= 150, "target API reference too small: {}", entries.len());
        for category in [
            "item_interaction", "block_shape", "entity_ai", "world", "block_entity",
            "inventory", "fluids", "networking", "menus", "worldgen",
        ] {
            assert!(entries.iter().any(|entry| entry.category == category), "missing target category {category}");
        }
    }

    #[test]
    fn target_symbols_have_javadoc_links_and_legacy_cross_references() {
        for entry in target_api_dictionary() {
            assert!(entry.documentation_url.starts_with("https://lexxie.dev/neoforge/1.21.1/"));
            assert!(entry.documentation_url.ends_with(".html"));
            assert!(!entry.signature.is_empty());
            assert!(!entry.semantic_contract.is_empty());
        }
        let matches = find_target_api_methods("IItemExtension", "getBurnTime");
        assert_eq!(matches.len(), 1);
        assert!(matches[0].legacy_aliases.iter().any(|alias| alias == "Item#getItemBurnTime"));
    }

    #[test]
    fn dictionary_does_not_repeat_same_owner_and_method_contract() {
        let mut seen = BTreeSet::new();
        for entry in target_api_dictionary() {
            let key = format!("{}#{}#{}", entry.owner, entry.method, entry.signature);
            assert!(seen.insert(key.clone()), "duplicate target API entry {key}");
        }
    }
}
