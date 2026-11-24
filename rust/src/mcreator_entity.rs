use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::mod_graph::{EntityMetadata, RendererRegistration};
use crate::mcreator_model::McreatorAnimationRecord;

fn default_living_definition() -> Value {
    serde_json::from_str(r#"{
        "mobName":"Converted Entity",
        "mobLabel":"",
        "mobModelName":"ModelConvertedEntity",
        "mobModelTexture":"converted_entity.png",
        "transparentModelCondition":{"fixedValue":false},
        "isShakingCondition":{"fixedValue":false},
        "solidBoundingBox":{"fixedValue":false},
        "visualScale":{"fixedValue":1.0},
        "boundingBoxScale":{"fixedValue":1.0},
        "modelLayers":[],
        "animations":[],
        "modelWidth":0.6,
        "modelHeight":1.8,
        "modelShadowSize":0.5,
        "mountedYOffset":0.0,
        "hasSpawnEgg":true,
        "spawnEggBaseColor":{"value":-8355712,"falpha":0.0},
        "spawnEggDotColor":{"value":-16777216,"falpha":0.0},
        "creativeTabs":["MISC"],
        "isBoss":false,
        "bossBarColor":"PURPLE",
        "bossBarType":"PROGRESS",
        "equipmentMainHand":"",
        "equipmentOffHand":"",
        "equipmentHelmet":"",
        "equipmentBody":"",
        "equipmentLeggings":"",
        "equipmentBoots":"",
        "mobBehaviourType":"Mob",
        "mobCreatureType":"UNDEFINED",
        "attackStrength":3.0,
        "attackKnockback":0.0,
        "knockbackResistance":0.0,
        "movementSpeed":0.3,
        "stepHeight":0.6,
        "armorBaseValue":0.0,
        "trackingRange":64,
        "followRange":16.0,
        "health":10.0,
        "xpAmount":0,
        "waterMob":false,
        "breatheUnderwater":{"fixedValue":false},
        "pushedByFluids":{"fixedValue":true},
        "flyingMob":false,
        "inventorySize":0,
        "inventoryStackSize":64,
        "disableCollisions":false,
        "ridable":false,
        "canControlForward":false,
        "canControlStrafe":false,
        "immuneToFire":false,
        "immuneToArrows":false,
        "immuneToFallDamage":false,
        "immuneToCactus":false,
        "immuneToDrowning":false,
        "immuneToLightning":false,
        "immuneToPotions":false,
        "immuneToPlayer":false,
        "immuneToExplosion":false,
        "immuneToTrident":false,
        "immuneToAnvil":false,
        "immuneToWither":false,
        "immuneToDragonBreath":false,
        "mobDrop":"",
        "livingSound":"",
        "hurtSound":"entity.generic.hurt",
        "deathSound":"entity.generic.death",
        "stepSound":"",
        "raidCelebrationSound":"",
        "entityDataEntries":[],
        "hasAI":true,
        "aiBase":"(none)",
        "aixml":"",
        "breedable":false,
        "tameable":false,
        "breedTriggerItems":[],
        "ranged":false,
        "rangedAttackItem":"",
        "rangedItemType":"Default item",
        "rangedAttackInterval":20,
        "rangedAttackRadius":10.0,
        "spawnThisMob":false,
        "doesDespawnWhenIdle":true,
        "spawningProbability":10,
        "mobSpawningType":"creature",
        "minNumberOfMobsPerGroup":1,
        "maxNumberOfMobsPerGroup":1,
        "restrictionBiomes":[],
        "spawnInDungeons":false,
        "raidSpawnsCount":[0,0,0,0,0,0,0],
        "sensitiveToVibration":false,
        "vibrationalEvents":[],
        "vibrationSensitivityRadius":{"fixedValue":7.0}
    }"#).expect("embedded MCreator FV89 living entity schema is valid JSON")
}

fn set_fixed_value(definition: &mut Map<String, Value>, key: &str, value: Value) {
    let entry = definition.entry(key.to_owned()).or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() { *entry = Value::Object(Map::new()); }
    entry.as_object_mut().expect("fixed-value object initialized")
        .insert("fixedValue".to_owned(), value);
}

fn ai_xml(ai_classes: &[String]) -> String {
    let classes: BTreeSet<String> = ai_classes.iter().map(|name| name.to_ascii_lowercase()).collect();
    let contains_any = |needles: &[&str]| classes.iter()
        .any(|name| needles.iter().any(|needle| name.contains(needle)));
    let mut blocks: Vec<(&str, Vec<(&str, &str)>)> = Vec::new();
    if contains_any(&["swimming"]) {
        blocks.push(("swim_in_water", vec![("condition", "null,null")]));
    }
    if contains_any(&["attackoncollide", "meleeattack"]) {
        blocks.push(("attack_on_collide", vec![
            ("speed", "1.2"), ("longmemory", "FALSE"), ("condition", "null,null")
        ]));
    }
    if contains_any(&["wander"]) {
        blocks.push(("wander", vec![("speed", "1"), ("condition", "null,null")]));
    }
    if contains_any(&["hurtbytarget", "nearestattackabletarget"]) {
        blocks.push(("attack_action", vec![("callhelp", "TRUE"), ("condition", "null,null")]));
    }
    if contains_any(&["lookidle", "watchclosest"]) {
        blocks.push(("look_around", vec![("condition", "null,null")]));
    }
    if blocks.is_empty() {
        blocks.push(("wander", vec![("speed", "1"), ("condition", "null,null")]));
        blocks.push(("look_around", vec![("condition", "null,null")]));
    }
    let mut nested = String::new();
    for (block_type, fields) in blocks.into_iter().rev() {
        let field_xml: String = fields.iter()
            .map(|(name, value)| format!("<field name=\"{name}\">{value}</field>"))
            .collect();
        let next_xml = if nested.is_empty() { String::new() } else { format!("<next>{nested}</next>") };
        nested = format!("<block type=\"{block_type}\">{field_xml}{next_xml}</block>");
    }
    format!("<xml xmlns=\"https://developers.google.com/blockly/xml\"><block type=\"aitasks_container\" deletable=\"false\" x=\"40\" y=\"40\"><next>{nested}</next></block></xml>")
}

/// Find the first sorted living-entity element definition in a workspace.
pub fn find_living_entity_template(workspace_root: &Path) -> Result<Option<(u32, Value, PathBuf)>> {
    let elements = workspace_root.join("elements");
    if !elements.is_dir() { return Ok(None); }
    let mut paths: Vec<PathBuf> = fs::read_dir(&elements)
        .with_context(|| format!("Reading elements directory {}", elements.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.file_name().and_then(|value| value.to_str())
            .map(|name| name.ends_with(".mod.json")).unwrap_or(false))
        .collect();
    paths.sort_by_key(|path| path.file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase()));
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else { continue; };
        let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { continue; };
        if value.get("_type").and_then(Value::as_str) != Some("livingentity")
            || !value.get("definition").map(Value::is_object).unwrap_or(false) { continue; }
        let version = value.get("_fv").and_then(|value|
            value.as_u64().or_else(|| value.as_str()?.parse::<u64>().ok()))
            .unwrap_or(89).min(u32::MAX as u64) as u32;
        return Ok(Some((version, value, path)));
    }
    Ok(None)
}

/// Beta 12 living-entity element construction, preserving extra fields from an
/// existing template and binding the generated model and animation references.
pub fn build_living_entity_element(
    template: Option<&Value>,
    metadata: &EntityMetadata,
    registration: &RendererRegistration,
    model_class: &str,
    texture_name: &str,
    animations: &[McreatorAnimationRecord],
) -> Value {
    let version = template.and_then(|value| value.get("_fv"))
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse::<u64>().ok()))
        .unwrap_or(89).min(u32::MAX as u64) as u32;
    let version = if template.is_some() { version } else { 89 };
    let mut definition = template.and_then(|value| value.get("definition"))
        .and_then(Value::as_object).cloned()
        .unwrap_or_else(|| default_living_definition().as_object().cloned().unwrap_or_default());

    definition.insert("mobName".to_owned(), json!(metadata.display_name));
    definition.insert("mobLabel".to_owned(), json!(""));
    definition.insert("mobModelName".to_owned(), json!(model_class));
    definition.insert("mobModelTexture".to_owned(), json!(texture_name));
    set_fixed_value(&mut definition, "visualScale", json!(
        if registration.visual_scale == 0.0 { 1.0 } else { registration.visual_scale }
    ));
    set_fixed_value(&mut definition, "boundingBoxScale", json!(1.0));
    definition.insert("modelWidth".to_owned(), json!(metadata.width.max(0.01)));
    definition.insert("modelHeight".to_owned(), json!(metadata.height.max(0.01)));
    definition.insert("modelShadowSize".to_owned(), json!(registration.shadow_size.max(0.0)));
    definition.insert("hasSpawnEgg".to_owned(), json!(true));
    if !definition.get("creativeTabs").map(Value::is_array).unwrap_or(false)
        || definition.get("creativeTabs").and_then(Value::as_array).map(Vec::is_empty).unwrap_or(true) {
        definition.insert("creativeTabs".to_owned(), json!(["MISC"]));
    }
    definition.insert("isBoss".to_owned(), json!(
        metadata.xp >= 500 || metadata.health >= 200.0 || metadata.width >= 4.0 || metadata.height >= 4.0
    ));
    definition.insert("mobBehaviourType".to_owned(), json!(
        if metadata.superclass.to_ascii_lowercase().contains("mob") { "Mob" } else { "Creature" }
    ));
    definition.insert("mobCreatureType".to_owned(), json!("UNDEFINED"));
    definition.insert("attackStrength".to_owned(), json!(metadata.attack_damage.max(0.0)));
    definition.insert("movementSpeed".to_owned(), json!(metadata.movement_speed.max(0.0)));
    definition.insert("armorBaseValue".to_owned(), json!(metadata.armor.max(0.0)));
    definition.insert("followRange".to_owned(), json!(metadata.follow_range.max(1.0)));
    definition.insert("health".to_owned(), json!(metadata.health.max(1.0)));
    definition.insert("xpAmount".to_owned(), json!(metadata.xp.max(0)));
    definition.insert("waterMob".to_owned(), json!(metadata.water_mob));
    set_fixed_value(&mut definition, "breatheUnderwater", json!(metadata.water_mob));
    definition.insert("flyingMob".to_owned(), json!(metadata.flying_mob));
    definition.insert("immuneToFire".to_owned(), json!(metadata.fire_immune));
    definition.insert("hasAI".to_owned(), json!(true));
    definition.insert("aixml".to_owned(), json!(ai_xml(&metadata.ai_classes)));
    definition.insert("spawnThisMob".to_owned(), json!(false));
    definition.insert("animations".to_owned(), Value::Array(animations.iter().map(|animation| json!({
        "animation":format!("CUSTOM:{model_class}Animation.{}", animation.field),
        "speed":1.0,"walking":animation.walking,"amplitude":1.0
    })).collect()));

    json!({"_fv":version,"_type":"livingentity","definition":Value::Object(definition)})
}

/// Persist a generated living-entity element to the workspace elements folder.
pub fn write_living_entity_element(output_elements: &Path, element_name: &str, element: &Value) -> Result<PathBuf> {
    let path = output_elements.join(format!("{element_name}.mod.json"));
    fs::create_dir_all(output_elements)
        .with_context(|| format!("Creating MCreator elements directory {}", output_elements.display()))?;
    fs::write(&path, format!("{}\n", serde_json::to_string_pretty(element)?))
        .with_context(|| format!("Writing MCreator element {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn metadata() -> EntityMetadata {
        EntityMetadata {
            class_name:"EntityDragon".into(),display_name:"Dragon".into(),registry_name:"dragon".into(),
            superclass:"EntityMob".into(),width:4.0,height:3.0,health:250.0,movement_speed:0.28,
            attack_damage:12.0,armor:8.0,follow_range:48.0,xp:750,fire_immune:true,
            water_mob:false,flying_mob:true,
            ai_classes:vec!["EntityAIMoveTowardsTarget".into(),"EntityAIWander".into(),
                "EntityAILookIdle".into(),"EntityAIAttackMelee".into()],
            warnings:Vec::new()
        }
    }

    fn registration() -> RendererRegistration {
        RendererRegistration {
            entity_class:"EntityDragon".into(),renderer_class:"RenderDragon".into(),
            model_class:"ModelDragon".into(),model_args:Vec::new(),renderer_args:Vec::new(),
            shadow_size:1.2,visual_scale:1.5,texture_refs:Vec::new(),
            source_class:"RenderRegistry".into(),source_offset:0
        }
    }

    #[test]
    fn builds_entity_element_from_recovered_stats_and_animations() {
        let animations = vec![McreatorAnimationRecord {
            field:"ModelDragonWalk".into(),source_name:"animation.model.walk".into(),
            length:1.2,channel_count:4,helper_count:1,walking:true
        }];
        let element = build_living_entity_element(None,&metadata(),&registration(),
            "ModelDragon","dragon_skin.png",&animations);
        assert_eq!(element["_fv"],89);
        assert_eq!(element["_type"],"livingentity");
        let def=&element["definition"];
        assert_eq!(def["mobName"],"Dragon");
        assert_eq!(def["mobModelName"],"ModelDragon");
        assert_eq!(def["mobModelTexture"],"dragon_skin.png");
        assert_eq!(def["visualScale"]["fixedValue"],1.5);
        assert_eq!(def["modelWidth"],4.0);
        assert_eq!(def["health"],250.0);
        assert_eq!(def["isBoss"],true);
        assert_eq!(def["immuneToFire"],true);
        assert_eq!(def["flyingMob"],true);
        assert_eq!(def["animations"][0]["animation"],"CUSTOM:ModelDragonAnimation.ModelDragonWalk");
        assert!(def["aixml"].as_str().unwrap().contains("attack_on_collide"));
        assert!(def["aixml"].as_str().unwrap().contains("wander"));
        assert!(def["aixml"].as_str().unwrap().contains("look_around"));
        assert_eq!(def["creativeTabs"],json!(["MISC"]));
        assert!(def.get("rangedAttackInterval").is_some());
    }

    #[test]
    fn preserves_template_specific_fields_and_format_version() {
        let template=json!({
            "_fv":88,"_type":"livingentity",
            "definition":{"creativeTabs":["CUSTOM:Example"],"visualScale":{"fixedValue":0.8},
                "breatheUnderwater":{"fixedValue":true},"templateSpecificField":{"retained":true}}
        });
        let element=build_living_entity_element(Some(&template),&metadata(),&registration(),
            "ModelDragon","dragon_skin.png",&[]);
        assert_eq!(element["_fv"],88);
        assert_eq!(element["definition"]["creativeTabs"],json!(["CUSTOM:Example"]));
        assert_eq!(element["definition"]["templateSpecificField"]["retained"],true);
        assert_eq!(element["definition"]["visualScale"]["fixedValue"],1.5);
        assert_eq!(element["definition"]["breatheUnderwater"]["fixedValue"],false);
        assert_eq!(element["definition"]["animations"],json!([]));
    }

    #[test]
    fn finds_sorted_living_entity_template() {
        let root=tempdir().unwrap();
        let elements=root.path().join("elements");
        fs::create_dir_all(&elements).unwrap();
        fs::write(elements.join("B.mod.json"),serde_json::to_vec(&json!({
            "_fv":89,"_type":"livingentity","definition":{"mobName":"B"}
        })).unwrap()).unwrap();
        fs::write(elements.join("A.mod.json"),serde_json::to_vec(&json!({
            "_fv":87,"_type":"livingentity","definition":{"mobName":"A"}
        })).unwrap()).unwrap();
        let (version,template,path)=find_living_entity_template(root.path()).unwrap().unwrap();
        assert_eq!(version,87);
        assert_eq!(template["definition"]["mobName"],"A");
        assert!(path.ends_with("A.mod.json"));
    }

    #[test]
    fn writes_element_json_to_workspace_elements() {
        let root=tempdir().unwrap();
        let element=build_living_entity_element(None,&metadata(),&registration(),
            "ModelDragon","dragon.png",&[]);
        let path=write_living_entity_element(&root.path().join("elements"),"Dragon",&element).unwrap();
        let parsed:Value=serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(parsed["_type"],"livingentity");
        assert_eq!(parsed["definition"]["health"],250.0);
    }
}
