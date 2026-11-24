
        class Tools {
          static WeaponStats emerald_stats = get_weaponstats(config, weapons, "Emerald", 3, 1300, 10, 6, 75);
          static Tier toolEMERALD = EnumHelper.addToolMaterial("REALEMERALD", emerald_stats.harvestlevel, emerald_stats.maxuses, emerald_stats.efficiency, emerald_stats.damage, emerald_stats.enchantability);
          static void r() { ITEMS.register("experiencesword", () -> new ExperienceSword(toolEMERALD)); ITEMS.register("fairysword", () -> new FairySword(toolEMERALD)); }
        }
    