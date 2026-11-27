//! Legacy item method dictionary for the NeoForge 1.21.1 target.
//!
//! This is a semantic migration catalogue, not a blind text-rewriter. Every
//! entry identifies the modern hook and the adapter work required to preserve
//! the old method's behavior. The scanner only matches method declarations;
//! calls to the same name inside method bodies are not considered overrides.

use regex::Regex;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemMethodMapping {
    pub source_method: String,
    pub target_api: String,
    pub category: String,
    pub conversion_status: String,
    pub notes: String,
    pub documentation_path: String,
}

#[derive(Debug, Clone, Copy)]
struct Definition {
    source: &'static str,
    target: &'static str,
    category: &'static str,
    status: &'static str,
    notes: &'static str,
    docs: &'static str,
}

const DEFINITIONS: &[Definition] = &[
    Definition { source: "onItemRightClick", target: "Item#use(Level, Player, InteractionHand) -> InteractionResultHolder<ItemStack>", category: "interaction", status: "manual_adapter_required", notes: "Translate the legacy world/player/hand arguments and ActionResult semantics. Preserve the replacement ItemStack and logical-side behavior.", docs: "items/interactionpipeline.md" },
    Definition { source: "onItemUse", target: "Item#useOn(UseOnContext) -> InteractionResult", category: "interaction", status: "manual_adapter_required", notes: "Build behavior around UseOnContext and translate legacy coordinates, facing, player, hand, and result semantics.", docs: "items/interactionpipeline.md" },
    Definition { source: "onItemUseFirst", target: "IItemExtension#onItemUseFirst(ItemStack, UseOnContext) -> InteractionResult", category: "interaction", status: "manual_adapter_required", notes: "Runs before normal block activation. Preserve PASS versus definitive SUCCESS or FAIL behavior.", docs: "items/interactionpipeline.md" },
    Definition { source: "onUpdate", target: "Item#inventoryTick(ItemStack, Level, Entity, int, boolean)", category: "lifecycle", status: "manual_adapter_required", notes: "Translate EntityItem/player inventory context and slot/selected state. Dropped-item ticking is a separate extension hook.", docs: "items/index.md" },
    Definition { source: "onEntityItemUpdate", target: "IItemExtension#onEntityItemUpdate(ItemStack, ItemEntity) -> boolean", category: "lifecycle", status: "manual_adapter_required", notes: "The modern hook receives ItemEntity directly; preserve the true-means-skip-remaining-update behavior.", docs: "items/index.md" },
    Definition { source: "onItemUseFinish", target: "Item#finishUsingItem(ItemStack, Level, LivingEntity) -> ItemStack", category: "lifecycle", status: "manual_adapter_required", notes: "Return the resulting ItemStack and preserve consumption, container conversion, and effects.", docs: "items/index.md" },
    Definition { source: "onUsingTick", target: "Item#onUseTick(Level, LivingEntity, ItemStack, int)", category: "lifecycle", status: "signature_adapter_required", notes: "The target remaining-use-duration argument may have a different countdown convention; preserve timing and side effects.", docs: "items/index.md" },
    Definition { source: "getItemUseAction", target: "Item#getUseAnimation(ItemStack) -> UseAnim", category: "use_animation", status: "enum_adapter_required", notes: "Map legacy EnumAction values to UseAnim and preserve stack-sensitive animation selection.", docs: "items/index.md" },
    Definition { source: "getMaxItemUseDuration", target: "Item#getUseDuration(ItemStack, LivingEntity) -> int", category: "use_animation", status: "signature_adapter_required", notes: "The modern method is stack- and entity-sensitive. Preserve charge duration and use timing.", docs: "items/index.md" },
    Definition { source: "hitEntity", target: "Item#hurtEnemy(ItemStack, LivingEntity, LivingEntity) -> boolean", category: "combat", status: "manual_adapter_required", notes: "Translate target and attacker types, durability damage, and returned statistic behavior.", docs: "items/index.md" },
    Definition { source: "onBlockDestroyed", target: "Item#mineBlock(ItemStack, Level, BlockState, BlockPos, LivingEntity) -> boolean", category: "mining", status: "manual_adapter_required", notes: "Translate World/IBlockState/EntityLivingBase to Level/BlockState/LivingEntity and preserve tool wear and return behavior.", docs: "items/tools.md" },
    Definition { source: "getStrVsBlock", target: "Item#getDestroySpeed(ItemStack, BlockState) -> float", category: "mining", status: "manual_adapter_required", notes: "Preserve stack-sensitive mining speed. Tool data components may better represent tiered or multi-tool rules.", docs: "items/tools.md" },
    Definition { source: "canHarvestBlock", target: "Item#isCorrectToolForDrops(ItemStack, BlockState) -> boolean; Tool data component rules", category: "mining", status: "manual_adapter_required", notes: "Reconstruct harvest level and block-state rules through Tool rules, tags, or the target query.", docs: "items/tools.md" },
    Definition { source: "addInformation", target: "Item#appendHoverText(ItemStack, Item.TooltipContext, List<Component>, TooltipFlag)", category: "tooltip", status: "partial_tooltip_extraction", notes: "Static formatted lore extraction is supported separately. Dynamic, conditional, localized, and state-dependent tooltip logic still needs an adapter.", docs: "items/index.md" },
    Definition { source: "onCreated", target: "Item#onCraftedBy(ItemStack, Level, Player)", category: "lifecycle", status: "manual_adapter_required", notes: "Translate crafting context and preserve stack initialization and player-specific behavior.", docs: "items/index.md" },
    Definition { source: "hasEffect", target: "Item#isFoil(ItemStack) -> boolean; DataComponents#ENCHANTMENT_GLINT_OVERRIDE", category: "rendering", status: "manual_adapter_required", notes: "The modern query is stack-sensitive. Constant glint can often be represented by ENCHANTMENT_GLINT_OVERRIDE.", docs: "items/datacomponents.md" },
    Definition { source: "showDurabilityBar", target: "Item#isBarVisible(ItemStack) -> boolean", category: "durability", status: "manual_adapter_required", notes: "Translate visibility conditions using the current stack's damage components.", docs: "items/index.md" },
    Definition { source: "getDurabilityForDisplay", target: "Item#getBarWidth(ItemStack) -> int", category: "durability", status: "manual_adapter_required", notes: "Convert the legacy normalized remaining-durability fraction to the target 0-13 bar width.", docs: "items/index.md" },
    Definition { source: "getRGBDurabilityForDisplay", target: "Item#getBarColor(ItemStack) -> int", category: "durability", status: "manual_adapter_required", notes: "Preserve the legacy color calculation while adapting the target integer color query.", docs: "items/index.md" },
    Definition { source: "getItemEnchantability", target: "IItemExtension#getEnchantmentValue(ItemStack) -> int", category: "enchanting", status: "stack_sensitive_adapter_required", notes: "Use the NeoForge stack-sensitive extension in 21.1; do not assume a parameterless vanilla query preserves stack behavior.", docs: "items/tools.md" },
    Definition { source: "getItemAttributeModifiers", target: "IItemExtension#getDefaultAttributeModifiers(ItemStack) -> ItemAttributeModifiers", category: "attributes", status: "stack_sensitive_adapter_required", notes: "Translate legacy attribute Multimap and equipment-slot logic to ItemAttributeModifiers or stack-sensitive extension behavior.", docs: "items/tools.md" },
    Definition { source: "getAttributeModifiers", target: "ItemAttributeModifiers / Item.Properties#attributes", category: "attributes", status: "manual_adapter_required", notes: "Translate legacy attribute keys, UUIDs, operation types, and equipment slots to modern attribute modifier entries.", docs: "items/tools.md" },
    Definition { source: "getContainerItem", target: "IItemExtension#getCraftingRemainingItem(ItemStack) -> ItemStack; Item.Properties#craftRemainder", category: "crafting", status: "stack_sensitive_adapter_required", notes: "Dynamic remainders require the stack-aware extension. Static remainders can use Item.Properties#craftRemainder.", docs: "items/index.md" },
    Definition { source: "hasContainerItem", target: "Item#hasCraftingRemainingItem() / IItemExtension#getCraftingRemainingItem(ItemStack)", category: "crafting", status: "manual_adapter_required", notes: "Reconstruct both the presence predicate and the actual remainder stack. A static remainder may use Item.Properties#craftRemainder.", docs: "items/index.md" },
    Definition { source: "getItemBurnTime", target: "IItemExtension#getBurnTime(ItemStack, RecipeType<?>) -> int", category: "fuel", status: "stack_sensitive_adapter_required", notes: "Translate recipe type and preserve any stack-dependent burn-time calculation.", docs: "items/index.md" },
    Definition { source: "getArmorTexture", target: "IItemExtension#getArmorTexture(ItemStack, Entity, EquipmentSlot, ArmorMaterial.Layer, boolean)", category: "armor", status: "manual_adapter_required", notes: "Adapt the target material layer and inner-model flag; return a ResourceLocation or null for the default texture.", docs: "items/tools.md" },
    Definition { source: "onArmorTick", target: "Armor tick event adapter / LivingEquipmentChangeEvent and player tick handling", category: "armor", status: "event_adapter_required", notes: "There is no direct one-to-one modern Item override. Determine whether the old behavior is per-tick, equip-change, or attribute based.", docs: "items/tools.md" },
    Definition { source: "doesSneakBypassUse", target: "IItemExtension#doesSneakBypassUse(ItemStack, LevelReader, BlockPos, Player) -> boolean", category: "interaction", status: "signature_adapter_required", notes: "Translate world and position arguments and retain item-stack-aware behavior.", docs: "items/interactionpipeline.md" },
    Definition { source: "onLeftClickEntity", target: "IItemExtension#onLeftClickEntity(ItemStack, Player, Entity) -> boolean", category: "combat", status: "manual_adapter_required", notes: "The target receives Player and Entity separately; preserve true-means-cancel attack processing.", docs: "items/index.md" },
    Definition { source: "onEntitySwing", target: "IItemExtension#onEntitySwing(ItemStack, LivingEntity) -> boolean", category: "combat", status: "deprecated_target", notes: "The legacy-compatible overload is deprecated for removal in NeoForge 21.1. Prefer a hand-aware event/adapter when available.", docs: "items/index.md" },
    Definition { source: "getIsRepairable", target: "Item#isValidRepairItem(ItemStack, ItemStack) -> boolean", category: "repair", status: "manual_adapter_required", notes: "Translate the candidate repair stack and preserve material/tag checks. Static no-repair behavior can use Item.Properties#setNoRepair.", docs: "items/tools.md" },
    Definition { source: "getEntityLifespan", target: "IItemExtension#getEntityLifespan(ItemStack, Level) -> int", category: "item_entity", status: "stack_sensitive_adapter_required", notes: "The target lifespan calculation is stack- and level-sensitive.", docs: "items/index.md" },
    Definition { source: "hasCustomEntity", target: "IItemExtension#hasCustomEntity(ItemStack) and Item#createEntity(Level, Entity, ItemStack)", category: "item_entity", status: "manual_adapter_required", notes: "Port both the custom-entity flag and entity construction. Returning null leaves the dropped item entity unchanged.", docs: "items/index.md" },
    Definition { source: "onDroppedByPlayer", target: "ItemTossEvent / player item-drop event adapter", category: "events", status: "event_adapter_required", notes: "Move behavior to the relevant NeoForge event path and preserve cancellation semantics.", docs: "items/index.md" },
    Definition { source: "onItemPickup", target: "Player/item pickup event adapter", category: "events", status: "event_adapter_required", notes: "Move pickup behavior to the relevant event and preserve cancellation and stack-transfer semantics.", docs: "items/index.md" },
    Definition { source: "onBlockStartBreak", target: "PlayerInteractEvent.LeftClickBlock / block-break event adapter", category: "events", status: "event_adapter_required", notes: "This is not a direct Item override in the target API. Reconstruct intended timing and cancellation behavior with the appropriate event.", docs: "items/interactionpipeline.md" },
    Definition { source: "setPotionEffect", target: "FoodProperties.Builder#effect(Supplier<MobEffectInstance>, float)", category: "food_effects", status: "effect_adapter_required", notes: "Recover the effect registry entry, duration, amplifier, and chance. Use the supplier overload to avoid premature effect classloading.", docs: "items/index.md" },
    Definition { source: "setAlwaysEdible", target: "FoodProperties.Builder#alwaysEdible()", category: "food", status: "automatic_property", notes: "A static always-edible flag can be emitted on the modern food builder.", docs: "items/index.md" },
    Definition { source: "getItemStackDisplayName", target: "Item#getName(ItemStack) -> Component", category: "localization", status: "signature_adapter_required", notes: "Translate the legacy String-returning display name to a text Component and preserve stack-sensitive naming.", docs: "items/index.md" },
    Definition { source: "getMaxDamage", target: "Item#getMaxDamage(ItemStack) / DataComponents#MAX_DAMAGE", category: "durability", status: "stack_sensitive_adapter_required", notes: "The maximum damage is represented by the stack's MAX_DAMAGE component in the modern item model; preserve any dynamic per-stack calculation.", docs: "items/datacomponents.md" },
    Definition { source: "getDamageVsEntity", target: "ItemAttributeModifiers / Tier#getAttackDamageBonus()", category: "attributes", status: "manual_adapter_required", notes: "Translate legacy flat attack damage into modern attribute modifiers or the appropriate tool tier bonus; preserve entity- and stack-specific behavior if present.", docs: "items/tools.md" },
    Definition { source: "getHealAmount", target: "FoodProperties#nutrition()", category: "food", status: "property_recovery", notes: "Compiled getter stubs expose the legacy hunger value; transfer it into FoodProperties.Builder#nutrition when it is a constant.", docs: "items/index.md" },
    Definition { source: "getSaturationModifier", target: "FoodProperties#saturationMod()", category: "food", status: "property_recovery", notes: "Compiled getter stubs expose the legacy saturation modifier; transfer constant values into FoodProperties.Builder#saturationMod.", docs: "items/index.md" },
    Definition { source: "getItemStackLimit", target: "DataComponents#MAX_STACK_SIZE / Item.Properties#stacksTo(int)", category: "stacking", status: "manual_adapter_required", notes: "Static values can use stacksTo; conditional stack limits need a stack-aware component or custom item behavior.", docs: "items/datacomponents.md" },
    Definition { source: "initCapabilities", target: "RegisterCapabilitiesEvent#registerItem(...) / item capability provider", category: "capabilities", status: "capability_adapter_required", notes: "Move per-stack capability construction into NeoForge capability registration and preserve provider lifetime and invalidation semantics.", docs: "inventories/capabilities.md" },
    Definition { source: "getShareTag", target: "DataComponentType persistent/network codec", category: "networking", status: "component_codec_adapter_required", notes: "Replace ad-hoc item-stack share tags with persistent and/or network-synchronized data components as appropriate.", docs: "items/datacomponents.md" },
    Definition { source: "readShareTag", target: "DataComponentType persistent/network codec", category: "networking", status: "component_codec_adapter_required", notes: "Reconstruct incoming custom stack state through the matching data component codec rather than legacy NBT share-tag mutation.", docs: "items/datacomponents.md" },
    Definition { source: "setNoRepair", target: "Item.Properties#setNoRepair()", category: "repair", status: "automatic_property", notes: "A static no-repair setting can be emitted as an Item.Properties builder call.", docs: "items/index.md" },
    Definition { source: "setMaxDamage", target: "Item.Properties#durability(int) / DataComponents#MAX_DAMAGE", category: "durability", status: "automatic_property", notes: "Static max-damage values can be transferred to Item.Properties#durability; per-stack dynamic behavior requires a component adapter.", docs: "items/index.md" },
    Definition { source: "setMaxStackSize", target: "Item.Properties#stacksTo(int) / DataComponents#MAX_STACK_SIZE", category: "stacking", status: "automatic_property", notes: "Static stack limits can be transferred to Item.Properties#stacksTo.", docs: "items/index.md" },
    Definition { source: "setContainerItem", target: "Item.Properties#craftRemainder(Item) / IItemExtension#getCraftingRemainingItem(ItemStack)", category: "crafting", status: "automatic_or_dynamic_adapter", notes: "Use a property for fixed vanilla remainder items; dynamic stack-dependent remainder behavior needs the extension hook.", docs: "items/index.md" },
    Definition { source: "onPlayerStoppedUsing", target: "Item#releaseUsing(ItemStack, Level, LivingEntity, int)", category: "use_animation", status: "signature_adapter_required", notes: "Translate the remaining-use-ticks convention and preserve charge release, bow firing, and side-specific effects.", docs: "items/index.md" },
    Definition { source: "getToolClasses", target: "DataComponents#TOOL / Item.Properties#tool(Tool)", category: "mining", status: "manual_adapter_required", notes: "Convert legacy tool-class strings into Tool rules and tags; retain modded block compatibility rather than reducing the item to a vanilla tier.", docs: "items/tools.md" },
    Definition { source: "getHarvestLevel", target: "DataComponents#TOOL / Tool.Rule", category: "mining", status: "manual_adapter_required", notes: "Translate per-tool-class harvest levels into ordered block-tag or state predicates and correct-drop flags.", docs: "items/tools.md" },
    Definition { source: "getArmorModel", target: "IClientItemExtensions#getHumanoidArmorModel(ItemStack, EquipmentSlot, HumanoidModel)", category: "armor_rendering", status: "client_adapter_required", notes: "Move custom armor model selection into client item extensions and preserve slot-specific model geometry.", docs: "items/tools.md" },
    Definition { source: "getArmorColor", target: "IItemExtension#getArmorTexture(...) / client armor layer tint adapter", category: "armor_rendering", status: "client_adapter_required", notes: "Recreate per-stack armor tinting in the client armor rendering path; preserve the legacy sentinel for no tint.", docs: "items/tools.md" },
    Definition { source: "getColorFromItemStack", target: "IClientItemExtensions# getCustomRenderer / item tint registration", category: "rendering", status: "client_adapter_required", notes: "Determine whether the old color is a static layer tint or dynamic stack color, then register the corresponding modern tint source.", docs: "items/index.md" },
    Definition { source: "getIcon", target: "Item model JSON and model predicate / override definitions", category: "rendering", status: "resource_adapter_required", notes: "Legacy icon selection is resource-driven in modern clients. Recover metadata and state predicates into model overrides or custom model registration.", docs: "items/index.md" },
    Definition { source: "getIconIndex", target: "Item model JSON and model predicate / override definitions", category: "rendering", status: "resource_adapter_required", notes: "Translate metadata-dependent icon selection into model overrides; do not emit a runtime icon-index call.", docs: "items/index.md" },
    Definition { source: "shouldCauseReequipAnimation", target: "IClientItemExtensions#shouldCauseReequipAnimation(ItemStack, ItemStack, boolean)", category: "rendering", status: "client_adapter_required", notes: "Preserve slot-sensitive hand animation suppression when stack state changes without changing the equipped item.", docs: "items/index.md" },
    Definition { source: "canElytraFly", target: "IItemExtension#canElytraFly(ItemStack, LivingEntity)", category: "equipment", status: "signature_adapter_required", notes: "Preserve stack and entity checks and ensure the item is equipped in the chest slot before granting flight.", docs: "items/index.md" },
    Definition { source: "elytraFlightTick", target: "IItemExtension#elytraFlightTick(ItemStack, LivingEntity, int)", category: "equipment", status: "signature_adapter_required", notes: "Translate flight tick parameters and preserve durability consumption, boost behavior, and server-authoritative state.", docs: "items/index.md" },
    Definition { source: "isBookEnchantable", target: "IItemExtension#isBookEnchantable(ItemStack, ItemStack)", category: "enchanting", status: "signature_adapter_required", notes: "Preserve restrictions on applying enchanted books, including stack-specific and enchantment-specific conditions.", docs: "items/tools.md" },
    Definition { source: "isRepairable", target: "Item#isValidRepairItem(ItemStack, ItemStack)", category: "repair", status: "signature_adapter_required", notes: "Map the legacy stack repairability predicate to the modern candidate-repair-stack query without losing custom material checks.", docs: "items/tools.md" },
    Definition { source: "onHorseArmorTick", target: "LivingEntity/horse tick event adapter", category: "equipment", status: "event_adapter_required", notes: "Move horse armor periodic behavior to a server-side entity tick handler and retain the equipped stack and horse type checks.", docs: "items/index.md" },
    Definition { source: "onItemExpire", target: "ItemEntity lifespan / ItemExpireEvent adapter", category: "item_entity", status: "event_adapter_required", notes: "Preserve the legacy ability to cancel or alter item despawning using the modern item-entity lifecycle event.", docs: "items/index.md" },
    Definition { source: "createEntity", target: "IItemExtension#hasCustomEntity(ItemStack) and Item#createEntity(Level, Entity, ItemStack)", category: "item_entity", status: "manual_adapter_required", notes: "Port custom dropped-entity construction and keep it paired with hasCustomEntity so the hook is actually invoked.", docs: "items/index.md" },
    Definition { source: "getItemUseSound", target: "Item#getDrinkingSound() / Item#getEatingSound()", category: "use_animation", status: "semantic_adapter_required", notes: "Choose the modern eating or drinking sound based on the recovered use action; custom sounds require registered SoundEvent references.", docs: "items/index.md" },
    Definition { source: "canFitInsideContainerItems", target: "Item#canFitInsideContainerItems()", category: "stacking", status: "signature_adapter_required", notes: "Preserve container nesting restrictions; do not assume all items are valid contents of shulker-like containers.", docs: "items/index.md" },
    Definition { source: "getFoodProperties", target: "Item#getFoodProperties(ItemStack, LivingEntity) / FoodProperties", category: "food", status: "stack_sensitive_adapter_required", notes: "Recover dynamic food values and effects per stack; static nutrition and saturation belong in the registered FoodProperties definition.", docs: "items/index.md" },
    Definition { source: "getEnchantmentValue", target: "IItemExtension#getEnchantmentValue(ItemStack)", category: "enchanting", status: "stack_sensitive_adapter_required", notes: "Retain the stack-sensitive enchantability calculation and avoid confusing it with enchantment level or enchantment eligibility.", docs: "items/tools.md" },
];

fn property_call_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"\b(?P<name>setMaxDamage|setMaxStackSize|setNoRepair|setContainerItem|setPotionEffect|setAlwaysEdible)\s*\("
    ).expect("static legacy item property-call regex"))
}

fn declaration_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(
        r"(?m)^[ \t]*(?:(?:@[A-Za-z_$][A-Za-z0-9_$.]*(?:\([^)]*\))?)[ \t]*\r?\n[ \t]*)*(?:(?:public|protected|private)\s+)(?:(?:static|final|synchronized|native|abstract)\s+)*(?:<[^>\r\n]+>\s*)?[A-Za-z_$][A-Za-z0-9_$.<>\[\], ?]*[ \t]+(?P<name>[A-Za-z_$][A-Za-z0-9_$]*)[ \t]*\("
    ).expect("static legacy item method declaration regex"))
}

pub fn item_method_dictionary() -> Vec<ItemMethodMapping> {
    DEFINITIONS.iter().map(to_mapping).collect()
}

/// Map declared method overrides only; ordinary invocations are ignored.
pub fn map_legacy_item_methods(source: &str) -> Vec<ItemMethodMapping> {
    let declared: BTreeSet<String> = declaration_regex()
        .captures_iter(source)
        .filter_map(|capture| capture.name("name").map(|value| value.as_str().to_owned()))
        .collect();
    // These legacy Item methods are often invoked from constructors rather than
    // overridden. Track the calls as well so their property migrations are visible.
    let property_calls: BTreeSet<String> = property_call_regex()
        .captures_iter(source)
        .filter_map(|capture| capture.name("name").map(|value| value.as_str().to_owned()))
        .collect();

    DEFINITIONS.iter()
        .filter(|definition| declared.contains(definition.source) || property_calls.contains(definition.source))
        .map(to_mapping)
        .collect()
}

fn to_mapping(definition: &Definition) -> ItemMethodMapping {
    ItemMethodMapping {
        source_method: definition.source.to_owned(),
        target_api: definition.target.to_owned(),
        category: definition.category.to_owned(),
        conversion_status: definition.status.to_owned(),
        notes: definition.notes.to_owned(),
        documentation_path: definition.docs.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_covers_item_method_families() {
        let names: BTreeSet<String> = item_method_dictionary().into_iter()
            .map(|entry| entry.source_method).collect();
        for expected in [
            "onItemRightClick", "onItemUse", "onUpdate", "onItemUseFinish",
            "getItemUseAction", "getMaxItemUseDuration", "hitEntity",
            "onBlockDestroyed", "addInformation", "getArmorTexture",
            "onEntityItemUpdate", "getIsRepairable", "getItemBurnTime",
            "getItemAttributeModifiers", "onArmorTick", "hasCustomEntity",
            "getMaxDamage", "getItemStackLimit", "getDamageVsEntity",
            "getHealAmount", "getSaturationModifier",
            "onPlayerStoppedUsing", "getToolClasses", "getHarvestLevel",
            "getArmorModel", "getArmorColor", "getColorFromItemStack", "getIcon",
            "getIconIndex", "shouldCauseReequipAnimation", "canElytraFly",
            "elytraFlightTick", "isBookEnchantable", "isRepairable",
            "onHorseArmorTick", "onItemExpire", "createEntity", "getItemUseSound",
            "canFitInsideContainerItems", "getFoodProperties", "getEnchantmentValue",
        ] {
            assert!(names.contains(expected), "missing dictionary entry: {expected}");
        }
    }

    #[test]
    fn detects_declarations_but_not_method_invocations() {
        let source = r#"
            class ExampleItem extends Item {
                @Override
                public ActionResult<ItemStack> onItemRightClick(World world, EntityPlayer player, EnumHand hand) {
                    return super.onItemRightClick(world, player, hand);
                }
                public void tick() {
                    onUpdate(stack, world, entity, slot, selected);
                }
                @Override
                public void onUpdate(ItemStack stack, World world, Entity entity, int slot, boolean selected) { }
                @Override
                public boolean getIsRepairable(ItemStack stack, ItemStack repair) { return false; }
            }
        "#;
        let detected = map_legacy_item_methods(source);
        let names: BTreeSet<String> = detected.into_iter().map(|entry| entry.source_method).collect();
        assert!(names.contains("onItemRightClick"));
        assert!(names.contains("onUpdate"));
        assert!(names.contains("getIsRepairable"));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn maps_automatic_properties_and_component_backed_features() {
        let mappings = map_legacy_item_methods(r#"
            class ExampleItem extends Item {
                ExampleItem() {
                    setMaxDamage(32);
                    setMaxStackSize(16);
                    setNoRepair();
                    setContainerItem(Items.BUCKET);
                }
            }
        "#);
        let called: BTreeSet<String> = mappings.into_iter().map(|entry| entry.source_method).collect();
        assert!(called.contains("setMaxDamage"));
        assert!(called.contains("setMaxStackSize"));
        assert!(called.contains("setNoRepair"));
        assert!(called.contains("setContainerItem"));
        let food_calls = map_legacy_item_methods(r#"
            class FoodItem extends ItemFood {
                FoodItem() {
                    setPotionEffect(new PotionEffect(Potion.getPotionById(10), 120, 1), 0.25F);
                    setAlwaysEdible();
                }
            }
        "#);
        let food_names: BTreeSet<String> = food_calls.into_iter().map(|entry| entry.source_method).collect();
        assert!(food_names.contains("setPotionEffect"));
        assert!(food_names.contains("setAlwaysEdible"));

        let entries = item_method_dictionary();
        assert!(entries.iter().any(|entry| entry.source_method == "setMaxDamage"
            && entry.target_api.contains("DataComponents#MAX_DAMAGE")));
        assert!(entries.iter().any(|entry| entry.source_method == "hasEffect"
            && entry.target_api.contains("ENCHANTMENT_GLINT_OVERRIDE")));
    }
}
