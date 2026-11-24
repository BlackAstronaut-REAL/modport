use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceWorldgenDefinitions {
    pub biomes: BTreeMap<String, Value>,
    pub dimensions: BTreeMap<String, Value>,
    pub dimension_types: BTreeMap<String, Value>,
}

fn snake_case(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            if previous.is_ascii_lowercase() || previous.is_ascii_digit() {
                output.push('_');
            }
        }
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else if !output.is_empty() && !output.ends_with('_') {
            output.push('_');
        }
    }
    let output = output.trim_matches('_').to_owned();
    if output.is_empty() { "converted_content".to_owned() } else { output }
}

fn pascal_case(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut words = Vec::<String>::new();
    let mut current = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !current.is_empty() { words.push(std::mem::take(&mut current)); }
            continue;
        }
        if ch.is_ascii_uppercase() && !current.is_empty() {
            let previous = chars[index - 1];
            let next_is_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_is_lower) {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(ch);
    }
    if !current.is_empty() { words.push(current); }
    let mut result = String::new();
    for word in words {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.push(first.to_ascii_uppercase());
            result.extend(chars);
        }
    }
    if result.is_empty() { result = "ConvertedContent".to_owned(); }
    if result.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        result.insert_str(0, "Content");
    }
    result
}

fn display_name(value: &str) -> String {
    let chars: Vec<char> = value.replace(['-', '.'], "_").chars().collect();
    let mut separated = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            let next_is_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_is_lower) {
                separated.push(' ');
            }
        }
        if ch == '_' || ch.is_whitespace() {
            separated.push(' ');
        } else {
            separated.push(ch);
        }
    }
    separated.split_whitespace().map(|word| {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => {
                let mut value = first.to_ascii_uppercase().to_string();
                value.extend(chars.map(|ch| ch.to_ascii_lowercase()));
                value
            }
            None => String::new(),
        }
    }).collect::<Vec<_>>().join(" ")
}

fn data_roots(source_root: &Path) -> Vec<PathBuf> {
    let candidates = [
        source_root.join("src/main/resources/data"),
        source_root.join("data"),
    ];
    let mut roots = Vec::new();
    for path in candidates {
        if path.is_dir() && !roots.contains(&path) { roots.push(path); }
    }
    for entry in WalkDir::new(source_root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
        if entry.file_type().is_dir()
            && entry.file_name().to_string_lossy().eq_ignore_ascii_case("data")
            && !roots.contains(&entry.path().to_path_buf()) {
            roots.push(entry.path().to_path_buf());
        }
    }
    roots
}

fn relative_parts(path: &Path) -> Vec<String> {
    path.components().filter_map(|component| match component {
        Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
        _ => None,
    }).collect()
}

fn read_json_object(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    value.as_object()?;
    Some(value)
}

fn json_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root).follow_links(false).into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .map(|entry| entry.into_path()).collect();
    files.sort();
    files
}

/// Discover resource-authored biome JSON under data/<modid>/worldgen/biome.
pub fn resource_biome_definitions(source_root: &Path, source_modid: &str) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    for root in data_roots(source_root) {
        for path in json_files(&root) {
            let Ok(relative) = path.strip_prefix(&root) else { continue };
            let parts = relative_parts(relative);
            if parts.len() < 3 || !parts[0].eq_ignore_ascii_case(source_modid)
                || !parts[1].eq_ignore_ascii_case("worldgen")
                || !parts[2].eq_ignore_ascii_case("biome") {
                continue;
            }
            if let Some(payload) = read_json_object(&path) {
                result.insert(snake_case(path.file_stem().and_then(|value| value.to_str()).unwrap_or_default()), payload);
            }
        }
    }
    result
}

/// Discover resource-authored dimension and dimension_type JSON definitions.
pub fn resource_dimension_definitions(
    source_root: &Path,
    source_modid: &str,
) -> (BTreeMap<String, Value>, BTreeMap<String, Value>) {
    let mut dimensions = BTreeMap::new();
    let mut types = BTreeMap::new();
    for root in data_roots(source_root) {
        for path in json_files(&root) {
            let Ok(relative) = path.strip_prefix(&root) else { continue };
            let parts = relative_parts(relative);
            if parts.len() < 3 || !parts[0].eq_ignore_ascii_case(source_modid) {
                continue;
            }
            if !parts[1].eq_ignore_ascii_case("dimension") && !parts[1].eq_ignore_ascii_case("dimension_type") {
                continue;
            }
            let Some(payload) = read_json_object(&path) else { continue };
            let key = snake_case(path.file_stem().and_then(|value| value.to_str()).unwrap_or_default());
            if parts[1].eq_ignore_ascii_case("dimension") {
                dimensions.insert(key, payload);
            } else {
                types.insert(key, payload);
            }
        }
    }
    (dimensions, types)
}

/// Resolve a JSON biome ResourceLocation to a registry path belonging to source_modid.
pub fn resource_namespaced_registry(value: &Value, source_modid: &str) -> String {
    let Some(raw) = value.as_str() else { return String::new() };
    let value = raw.trim();
    if let Some((namespace, path)) = value.split_once(':') {
        if namespace.eq_ignore_ascii_case(source_modid) { snake_case(path) } else { String::new() }
    } else {
        snake_case(value)
    }
}

fn object_at<'a>(value: &'a Value, key: &str) -> Option<&'a serde_json::Map<String, Value>> {
    value.get(key).and_then(Value::as_object)
}

fn number(value: Option<&Value>, default: f64) -> f64 {
    value.and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok())).unwrap_or(default)
}

fn integer(value: Option<&Value>, default: i64) -> i64 {
    value.and_then(|v| v.as_i64().or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| v.as_f64().map(|n| n as i64)).or_else(|| v.as_str()?.trim().parse::<i64>().ok()))
        .unwrap_or(default)
}

fn boolean(value: Option<&Value>, default: bool) -> bool {
    value.and_then(Value::as_bool).unwrap_or(default)
}

fn truthy(value: Option<&Value>, default: bool) -> bool {
    match value {
        None => default,
        Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(value)) => !value.is_empty(),
        Some(Value::Object(value)) => !value.is_empty(),
    }
}

fn string_value(value: Option<&Value>, default: &str) -> String {
    value.and_then(Value::as_str).unwrap_or(default).to_owned()
}

/// Convert a source dimension and dimension_type JSON pair into MCreator definition fields.
pub fn resource_dimension_updates(
    payload: &Value,
    dimension_type: &Value,
    biome_registry: &str,
    source_modid: &str,
) -> Value {
    let generator = object_at(payload, "generator");
    let settings = generator.and_then(|value| value.get("settings")).and_then(Value::as_object);
    let biome_source = generator.and_then(|value| value.get("biome_source")).and_then(Value::as_object);
    let mut biomes = Vec::new();
    if let Some(entries) = biome_source.and_then(|source| source.get("biomes")).and_then(Value::as_array) {
        for entry in entries {
            if let Some(entry) = entry.as_object() {
                let registry = resource_namespaced_registry(entry.get("biome").unwrap_or(&Value::Null), source_modid);
                if !registry.is_empty() { biomes.push(format!("CUSTOM:{}", pascal_case(&registry))); }
            }
        }
    }
    if !biome_registry.is_empty() && biomes.is_empty() {
        biomes.push(format!("CUSTOM:{}", pascal_case(biome_registry)));
    }
    let dtype = dimension_type.as_object();
    let has_ceiling = truthy(dtype.and_then(|value| value.get("has_ceiling")), false);
    let natural = truthy(dtype.and_then(|value| value.get("natural")), false);
    let bed_works = boolean(dtype.and_then(|value| value.get("bed_works")), true);
    let ambient_light = number(dtype.and_then(|value| value.get("ambient_light")), 0.5);
    let ambient_light = if ambient_light == 0.0 { 0.5 } else { ambient_light };
    let coordinate_scale = number(dtype.and_then(|value| value.get("coordinate_scale")), 1.0);
    let coordinate_scale = if coordinate_scale == 0.0 { 1.0 } else { coordinate_scale };
    let infiniburn = string_value(dtype.and_then(|value| value.get("infiniburn")), "#minecraft:infiniburn_overworld");
    let noise = settings.and_then(|value| value.get("noise")).and_then(Value::as_object);
    let sea_level = integer(settings.and_then(|value| value.get("sea_level")), 63);
    let sea_level = if sea_level == 0 { 63 } else { sea_level };
    let monster_light = dtype.and_then(|value| value.get("monster_spawn_light_level"));
    let min_monster_light = monster_light.and_then(Value::as_i64).unwrap_or(0);
    let max_monster_light = monster_light.and_then(Value::as_i64).unwrap_or(7);
    let fixed_time_present = dtype.is_some_and(|value| value.contains_key("fixed_time"));
    let fixed_time = integer(dtype.and_then(|value| value.get("fixed_time")), 0);
    let can_respawn = natural && dtype.and_then(|value| value.get("bed_works")).is_some_and(|value| value == false);
    let name_seed = biome_registry.split_whitespace().next().unwrap_or("dimension");

    json!({
        "biomesInDimension": if biomes.is_empty() { vec!["Plains".to_owned()] } else { biomes },
        "biomesInDimensionCaves": [],
        "worldGenType": if has_ceiling { "Nether like gen" } else { "Normal world gen" },
        "mainFillerBlock": "Blocks.STONE#0",
        "fluidBlock": "Blocks.AIR",
        "seaLevel": sea_level,
        "generateOreVeins": truthy(settings.and_then(|value| value.get("ore_veins_enabled")), true),
        "generateAquifers": truthy(settings.and_then(|value| value.get("aquifers_enabled")), true),
        "horizontalNoiseSize": integer(noise.and_then(|value| value.get("size_horizontal")), 1),
        "verticalNoiseSize": integer(noise.and_then(|value| value.get("size_vertical")), 2),
        "defaultEffects": "overworld",
        "useCustomEffects": false,
        "hasClouds": !has_ceiling,
        "cloudHeight": 192,
        "skyType": if has_ceiling { "NONE" } else { "NORMAL" },
        "sunHeightAffectsFog": true,
        "canRespawnHere": can_respawn,
        "hasFog": false,
        "ambientLight": ambient_light,
        "doesWaterVaporize": truthy(dtype.and_then(|value| value.get("ultrawarm")), false),
        "hasFixedTime": fixed_time_present,
        "fixedTimeValue": fixed_time,
        "coordinateScale": coordinate_scale,
        "infiniburnTag": infiniburn.trim_start_matches('#'),
        "enableCustomSkyboxTextures": false,
        "enableCustomSunMoonTextures": false,
        "bedWorks": bed_works,
        "hasSkyLight": truthy(dtype.and_then(|value| value.get("has_skylight")), true),
        "imitateOverworldBehaviour": natural,
        "piglinSafe": truthy(dtype.and_then(|value| value.get("piglin_safe")), false),
        "hasRaids": truthy(dtype.and_then(|value| value.get("has_raids")), true),
        "minMonsterSpawnLightLimit": min_monster_light,
        "maxMonsterSpawnLightLimit": max_monster_light,
        "monsterSpawnBlockLightLimit": integer(dtype.and_then(|value| value.get("monster_spawn_block_light_limit")), 0),
        "creativeTabs": ["TOOLS"],
        "texture": "",
        "portalTexture": "",
        "enablePortal": false,
        "name": display_name(name_seed)
    })
}

/// Convert a source biome JSON definition into MCreator biome fields.
pub fn resource_biome_updates(payload: &Value) -> Value {
    let effects = object_at(payload, "effects");
    let spawners = object_at(payload, "spawners");
    let spawn_types = ["monster", "creature", "ambient", "axolotls", "underground_water_creature",
        "water_creature", "water_ambient", "misc"];
    let mut spawn_entries = Vec::new();
    for spawn_type in spawn_types {
        if let Some(entries) = spawners.and_then(|value| value.get(spawn_type)).and_then(Value::as_array) {
            for entry in entries {
                let Some(entry) = entry.as_object() else { continue };
                let Some(entity) = entry.get("type").and_then(Value::as_str) else { continue };
                let min_group = integer(entry.get("minCount"), 1);
                let max_group = integer(entry.get("maxCount"), 1);
                let weight = integer(entry.get("weight"), 1);
                spawn_entries.push(json!({
                    "entity": entity,
                    "minGroup": if min_group == 0 { 1 } else { min_group },
                    "maxGroup": if max_group == 0 { 1 } else { max_group },
                    "weight": if weight == 0 { 1 } else { weight },
                    "spawnType": if spawn_type == "monster" { "monster" } else { "creature" }
                }));
            }
        }
    }
    let particle = effects.and_then(|value| value.get("particle"));
    let additions = effects.and_then(|value| value.get("additions_sound")).and_then(Value::as_object);
    let music = effects.and_then(|value| value.get("music")).and_then(Value::as_object);
    let mood = effects.and_then(|value| value.get("mood_sound")).and_then(Value::as_object);
    let particle_probability = if particle.is_some_and(Value::is_object) {
        number(particle.and_then(|value| value.get("probability")), 0.0) * 100.0
    } else { 0.0 };
    let downfall = number(payload.get("downfall"), 0.5);
    let downfall = if downfall == 0.0 { 0.5 } else { downfall };
    let temperature = number(payload.get("temperature"), 0.5);
    let temperature = if temperature == 0.0 { 0.5 } else { temperature };
    json!({
        "groundBlock": "Blocks.GRASS_BLOCK",
        "undergroundBlock": "Blocks.DIRT",
        "ambientSound": string_value(effects.and_then(|value| value.get("ambient_sound")), ""),
        "additionsSound": string_value(additions.and_then(|value| value.get("sound")), ""),
        "music": string_value(music.and_then(|value| value.get("sound")), ""),
        "moodSound": string_value(mood.and_then(|value| value.get("sound")), ""),
        "moodSoundDelay": integer(mood.and_then(|value| value.get("tick_delay")), 6000),
        "spawnParticles": truthy(particle, false),
        "particleToSpawn": "CLOUD",
        "particlesProbability": particle_probability,
        "rainingPossibility": downfall,
        "temperature": temperature,
        "spawnBiome": false,
        "spawnInCaves": false,
        "spawnBiomeNether": false,
        "genTemperature": {"min": -0.2, "max": 0.2},
        "genHumidity": {"min": -0.2, "max": 0.2},
        "genContinentalness": {"min": 0.31, "max": 0.71},
        "genErosion": {"min": 0.6, "max": 1.0},
        "genWeirdness": {"min": -0.86, "max": -0.46},
        "genDepth": {"min": 0.2, "max": 0.9},
        "treesPerChunk": 0,
        "vanillaTreeType": "Default",
        "treeType": 0,
        "minHeight": 2,
        "treeStem": "",
        "treeBranch": "",
        "treeVines": "",
        "treeFruits": "",
        "defaultFeatures": [],
        "spawnStronghold": false,
        "spawnMineshaft": false,
        "spawnMineshaftMesa": false,
        "spawnPillagerOutpost": false,
        "villageType": "none",
        "spawnWoodlandMansion": false,
        "spawnJungleTemple": false,
        "spawnDesertPyramid": false,
        "spawnSwampHut": false,
        "spawnIgloo": false,
        "spawnOceanMonument": false,
        "spawnShipwreck": false,
        "spawnShipwreckBeached": false,
        "spawnBuriedTreasure": false,
        "oceanRuinType": "NONE",
        "spawnNetherBridge": false,
        "spawnNetherFossil": false,
        "spawnBastionRemnant": false,
        "spawnEndCity": false,
        "spawnRuinedPortal": "NONE",
        "spawnEntries": spawn_entries
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discovers_resource_only_biomes_dimensions_and_dimension_types() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let biome = root.join("data/demo/worldgen/biome/test_biome.json");
        let dimension = root.join("data/demo/dimension/test_dim.json");
        let dimension_type = root.join("data/demo/dimension_type/test.json");
        let unrelated = root.join("data/other/worldgen/biome/foreign.json");
        for path in [&biome, &dimension, &dimension_type, &unrelated] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        fs::write(&biome, br#"{"temperature":0.8,"downfall":0.2,"effects":{},"spawners":{}}"#).unwrap();
        fs::write(&dimension, br#"{"generator":{"biome_source":{"type":"minecraft:multi_noise","biomes":[{"biome":"demo:test_biome"}]},"settings":{"sea_level":63}}}"#).unwrap();
        fs::write(&dimension_type, br#"{"bed_works":true,"has_skylight":true,"ambient_light":0.5}"#).unwrap();
        fs::write(&unrelated, br#"{"temperature":1.0}"#).unwrap();

        let biomes = resource_biome_definitions(root, "demo");
        let (dimensions, types) = resource_dimension_definitions(root, "demo");
        assert_eq!(biomes["test_biome"]["temperature"], 0.8);
        assert!(dimensions.contains_key("test_dim"));
        assert!(types.contains_key("test"));
        assert!(!biomes.contains_key("foreign"));
    }

    #[test]
    fn converts_dimension_worldgen_and_type_flags_to_mcreator_fields() {
        let dimension = json!({
            "generator":{
                "biome_source":{"biomes":[
                    {"biome":"demo:test_biome"},
                    {"biome":"minecraft:plains"}
                ]},
                "settings":{"sea_level":63,"ore_veins_enabled":false,"noise":{"size_horizontal":2,"size_vertical":3}}
            }
        });
        let dimension_type = json!({
            "natural":true,"bed_works":false,"has_ceiling":false,"has_skylight":true,
            "ambient_light":0.25,"coordinate_scale":8.0,"infiniburn":"#minecraft:infiniburn_nether",
            "ultrawarm":true,"fixed_time":18000,"piglin_safe":true
        });
        let updates = resource_dimension_updates(&dimension, &dimension_type, "test_biome", "demo");
        assert_eq!(updates["biomesInDimension"], json!(["CUSTOM:TestBiome"]));
        assert_eq!(updates["seaLevel"], 63);
        assert_eq!(updates["generateOreVeins"], false);
        assert_eq!(updates["horizontalNoiseSize"], 2);
        assert_eq!(updates["canRespawnHere"], true);
        assert_eq!(updates["doesWaterVaporize"], true);
        assert_eq!(updates["fixedTimeValue"], 18000);
        assert_eq!(updates["infiniburnTag"], "minecraft:infiniburn_nether");
    }

    #[test]
    fn converts_biome_effects_and_spawn_tables() {
        let biome = json!({
            "temperature":0.8,"downfall":0.2,
            "effects":{
                "ambient_sound":"demo:ambient",
                "music":{"sound":"demo:music"},
                "mood_sound":{"sound":"demo:mood","tick_delay":4200},
                "particle":{"probability":0.15,"options":{"type":"minecraft:ash"}},
                "additions_sound":{"sound":"demo:additions"}
            },
            "spawners":{
                "monster":[{"type":"demo:beast","minCount":2,"maxCount":4,"weight":12}],
                "ambient":[{"type":"minecraft:bat","minCount":0,"maxCount":1,"weight":5}]
            }
        });
        let updates = resource_biome_updates(&biome);
        assert_eq!(updates["temperature"], 0.8);
        assert_eq!(updates["rainingPossibility"], 0.2);
        assert_eq!(updates["ambientSound"], "demo:ambient");
        assert_eq!(updates["moodSoundDelay"], 4200);
        assert_eq!(updates["particlesProbability"], 15.0);
        assert_eq!(updates["spawnEntries"][0]["spawnType"], "monster");
        assert_eq!(updates["spawnEntries"][0]["minGroup"], 2);
        assert_eq!(updates["spawnEntries"][1]["spawnType"], "creature");
        assert_eq!(updates["spawnEntries"][1]["minGroup"], 1);
    }
}
