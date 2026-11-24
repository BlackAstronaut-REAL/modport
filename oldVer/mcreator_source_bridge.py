from __future__ import annotations

import json
import re
import shutil
from pathlib import Path
from typing import Any, Iterable


def _pascal(value: str) -> str:
    words = re.findall(r"[A-Za-z0-9]+", re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", value))
    result = "".join(w[:1].upper() + w[1:] for w in words) or "Converted"
    if result[0].isdigit():
        result = "Converted" + result
    return result


def _snake(value: str) -> str:
    value = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", value)
    value = re.sub(r"[^A-Za-z0-9]+", "_", value).strip("_").lower()
    return value or "converted"


def _java_package_path(package: str) -> Path:
    return Path(*package.split("."))


def _write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.rstrip() + "\n", encoding="utf-8")


def _find_init_file(output_root: Path, package: str, suffix: str) -> Path | None:
    init = output_root / "src" / "main" / "java" / _java_package_path(package) / "init"
    if not init.exists():
        return None
    candidates = sorted(init.glob(f"*{suffix}.java"))
    return candidates[0] if candidates else None


def _copy_if_missing(src: Path, dst: Path) -> bool:
    if not src.exists() or not src.is_file():
        return False
    dst.parent.mkdir(parents=True, exist_ok=True)
    if dst.exists():
        return False
    shutil.copy2(src, dst)
    return True


def _source_path(root: Path, package: str, rel: str) -> Path:
    return root / "src" / "main" / "java" / _java_package_path(package) / rel


def _expected_mod_class(modid: str, suffix: str) -> str:
    """Return MCreator's conventional generated class name for a mod id."""
    return f"{_pascal(modid)}Mod{suffix}"


def _init_class_name(output_root: Path, package: str, suffix: str, modid: str) -> str:
    path = _find_init_file(output_root, package, f"Mod{suffix}")
    return path.stem if path else _expected_mod_class(modid, suffix)


def _copy_animation_metadata(source: Path, target: Path) -> bool:
    """Copy a sibling ``.png.mcmeta`` file alongside a copied texture."""
    source_meta = source.with_name(source.name + ".mcmeta")
    if not source_meta.exists():
        return False
    target_meta = target.with_name(target.name + ".mcmeta")
    target_meta.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_meta, target_meta)
    return True


def _resource_roots(root: Path) -> list[Path]:
    roots: list[Path] = []
    for candidate in (
        root / "src" / "main" / "resources" / "assets",
        root / "assets",
        root / "src" / "main" / "resources",
    ):
        if candidate.exists():
            roots.append(candidate)
    for path in root.rglob("assets"):
        if path.is_dir():
            roots.append(path)
    return roots


def _rewrite_data_namespace(value: Any, source_modid: str, target_modid: str) -> Any:
    """Rewrite exact ResourceLocations from the imported namespace to the target namespace."""
    source = str(source_modid or "").strip().lower()
    target = str(target_modid or "").strip().lower()
    if not source or not target or source == target:
        return value
    prefix = source + ":"
    if isinstance(value, str):
        if value.lower().startswith(prefix):
            return target + value[len(source):]
        if value.lower().startswith("#" + prefix):
            return "#" + target + value[len(source)+1:]
        return value
    if isinstance(value, dict):
        return {k: _rewrite_data_namespace(v, source, target) for k, v in value.items()}
    if isinstance(value, list):
        return [_rewrite_data_namespace(v, source, target) for v in value]
    return value


def _rewrite_sound_json(value: Any, source_modid: str, target_modid: str) -> Any:
    """Rewrite sounds.json sample references and canonicalize sample filenames."""
    source = str(source_modid or "").strip().lower()
    target = str(target_modid or "").strip().lower()
    if isinstance(value, str):
        out = value
        if ":" in out:
            ns, path = out.split(":", 1)
            if ns.lower() == source:
                out = f"{target}:{path}"
        if out.startswith("sounds/"):
            out = out[:7] + out[7:].lower()
        elif ":" in out:
            ns, path = out.split(":", 1)
            if ns.lower() == target and path.startswith("sounds/"):
                out = f"{ns}:{path[:7] + path[7:].lower()}"
        return out
    if isinstance(value, list):
        return [_rewrite_sound_json(v, source_modid, target_modid) for v in value]
    if isinstance(value, dict):
        return {k: _rewrite_sound_json(v, source_modid, target_modid) for k, v in value.items()}
    return value


def _copy_source_resources(
    source_root: Path,
    output_resources: Path,
    log=None,
    source_modid: str | None = None,
    target_modid: str | None = None,
) -> int:
    """Copy mod assets/data without overwriting known-good files.

    Template imports remap source data namespace paths/references to the target
    workspace namespace. This prevents NeoForge worldgen from asking for e.g.
    ``source_modid:deepslate_oresalt`` when the imported block is registered as
    ``target_modid:deepslate_oresalt``.
    """
    copied = 0
    source_modid = str(source_modid or "").strip()
    target_modid = str(target_modid or "").strip()
    seen: set[str] = set()
    for assets_root in _resource_roots(source_root):
        try:
            iterator = assets_root.rglob("*")
        except OSError:
            continue
        for src in iterator:
            if not src.is_file():
                continue
            try:
                rel = src.relative_to(assets_root)
            except ValueError:
                continue
            key = str(rel).replace("\\", "/").lower()
            if key in seen:
                continue
            # Preserve source-authored model/blockstate resources. These are part of the
            # visual contract, especially for multi-state blocks and source display overrides.
            rel_parts = [part.lower() for part in rel.parts]
            seen.add(key)
            # Asset namespaces are part of the path, so unlike data JSON values they
            # must be remapped before the destination path is constructed. This was
            # the source of stale chaospersists: sound/resource references surviving
            # imports into a cndcc workspace.
            if target_modid and source_modid and len(rel.parts) >= 2 and rel.parts[0].lower() == source_modid.lower():
                rel = Path(target_modid) / Path(*rel.parts[1:])
            dst = output_resources / "assets" / rel
            if src.suffix.lower() == ".json" and target_modid and source_modid:
                try:
                    payload = json.loads(src.read_text(encoding="utf-8-sig"))
                    payload = _rewrite_data_namespace(payload, source_modid, target_modid)
                    # sounds.json is special: its keys remain event ids, but sound
                    # sample ResourceLocations need the imported target namespace.
                    if rel.name.lower() == "sounds.json" and isinstance(payload, dict):
                        payload = _rewrite_sound_json(payload, source_modid, target_modid)
                    dst.parent.mkdir(parents=True, exist_ok=True)
                    dst.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
                    copied += 1
                    continue
                except Exception:
                    pass
            # Sound sample filenames are ResourceLocation paths and therefore must be
            # lower-case in Minecraft 1.21.x. Preserve every source file but expose a
            # canonical lower-case copy for runtime lookup.
            if len(rel.parts) >= 3 and rel.parts[1].lower() == "sounds" and src.suffix.lower() == ".ogg":
                rel = Path(rel.parts[0]) / rel.parts[1] / rel.name.lower()
                dst = output_resources / "assets" / rel
            if _copy_if_missing(src, dst):
                copied += 1

    data_roots: list[Path] = []
    for candidate in (source_root / "src" / "main" / "resources" / "data", source_root / "data"):
        if candidate.exists():
            data_roots.append(candidate)
    for path in source_root.rglob("data"):
        if path.is_dir():
            data_roots.append(path)
    seen.clear()
    for data_root in data_roots:
        try:
            iterator = data_root.rglob("*")
        except OSError:
            continue
        for src in iterator:
            if not src.is_file():
                continue
            try:
                rel = src.relative_to(data_root)
            except ValueError:
                continue
            key = str(rel).replace("\\", "/").lower()
            if key in seen:
                continue
            seen.add(key)
            parts = rel.parts
            lower_parts = [part.lower() for part in parts]
            # These are generated by the MCreator 1.21.1 element generator.
            # Copying legacy/source JSON for them can override generated resources
            # and crash registry loading (especially dimension_type and biome JSON).
            # Recipes are emitted as MCreator recipe elements separately.
            generated_data_dirs = {"dimension", "dimension_type", "biome", "biomes", "recipes"}
            if any(part in generated_data_dirs for part in lower_parts[1:]):
                continue
            # Source-authored worldgen feature JSON is retained verbatim (with namespace rewrite below).
            if target_modid and source_modid and parts and parts[0].lower() == source_modid.lower():
                rel = Path(target_modid) / Path(*parts[1:])
            if target_modid and len(rel.parts) >= 3 and rel.parts[1].lower() == "forge" and rel.parts[2].lower() == "biome_modifier":
                rel = Path(target_modid) / "neoforge" / "biome_modifier" / Path(*rel.parts[3:])
            dst = output_resources / "data" / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            if target_modid and source_modid and src.suffix.lower() == ".json":
                try:
                    payload = json.loads(src.read_text(encoding="utf-8-sig"))
                    payload = _rewrite_data_namespace(payload, source_modid, target_modid)
                    def _rewrite_biome_modifier(v):
                        if isinstance(v, dict): return {k: _rewrite_biome_modifier(val) for k,val in v.items()}
                        if isinstance(v, list): return [_rewrite_biome_modifier(x) for x in v]
                        if isinstance(v, str): return re.sub(r"(?i)^forge:add_features$", "neoforge:add_features", v)
                        return v
                    payload = _rewrite_biome_modifier(payload)
                    dst.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
                    copied += 1
                    continue
                except Exception:
                    pass
            if _copy_if_missing(src, dst):
                copied += 1
    if log:
        log(f"MCreator resource bridge: copied {copied} source resource/data files")
    return copied


def _model_package_imports(package: str, animation: bool = False) -> str:
    if animation:
        return f'''package {package}.client.model.animations;\n\nimport net.minecraft.client.animation.*;\n\n'''
    return f'''package {package}.client.model;\n\nimport com.mojang.blaze3d.vertex.*;\nimport net.minecraft.client.model.*;\nimport net.minecraft.client.model.geom.*;\nimport net.minecraft.client.model.geom.builders.*;\nimport net.minecraft.resources.*;\nimport net.minecraft.util.*;\nimport net.minecraft.world.entity.*;\n\n'''


def _copy_model_sources(output_root: Path, package: str, model_files: Iterable[Path], animation_files: Iterable[Path]) -> dict[str, str]:
    written: dict[str, str] = {}
    model_dest = output_root / "src" / "main" / "java" / _java_package_path(package) / "client" / "model"
    anim_dest = model_dest / "animations"
    for src in model_files:
        text = src.read_text(encoding="utf-8", errors="ignore")
        if not re.search(r"^\s*package\s+", text, re.MULTILINE):
            text = _model_package_imports(package) + text
        dst = model_dest / src.name
        _write(dst, text)
        written[src.stem] = str(dst.relative_to(output_root))
    for src in animation_files:
        text = src.read_text(encoding="utf-8", errors="ignore")
        if not re.search(r"^\s*package\s+", text, re.MULTILINE):
            text = _model_package_imports(package, animation=True) + text
        dst = anim_dest / src.name
        _write(dst, text)
        written[src.stem] = str(dst.relative_to(output_root))
    return written


def _ensure_workspace_metadata(workspace: dict[str, Any], element_name: str, files: Iterable[str]) -> None:
    for item in workspace.get("mod_elements", []):
        if str(item.get("name", "")).lower() == element_name.lower():
            meta = item.setdefault("metadata", {})
            existing = list(meta.get("files", []))
            for file in files:
                if file and file not in existing:
                    existing.append(file)
            meta["files"] = existing
            return


def _method_body(text: str, signature_fragment: str) -> tuple[int, int] | None:
    start = text.find(signature_fragment)
    if start < 0:
        return None
    brace = text.find("{", start)
    if brace < 0:
        return None
    depth = 0
    for i in range(brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return brace, i
    return None


def _insert_before_method_close(path: Path, signature_fragment: str, content: str, marker: str) -> bool:
    if not path.exists() or not content.strip():
        return False
    text = path.read_text(encoding="utf-8", errors="ignore")
    if marker in text:
        return False
    span = _method_body(text, signature_fragment)
    if not span:
        return False
    _, close = span
    insertion = f"\n{marker}\n{content.rstrip()}\n"
    text = text[:close] + insertion + text[close:]
    path.write_text(text, encoding="utf-8")
    return True


def _insert_before_token(path: Path, token: str, content: str, marker: str) -> bool:
    if not path.exists() or not content.strip():
        return False
    text = path.read_text(encoding="utf-8", errors="ignore")
    if marker in text:
        return False
    idx = text.find(token)
    if idx < 0:
        return False
    insertion = f"{content.rstrip()}\n\t{marker}\n"
    text = text[:idx] + insertion + text[idx:]
    path.write_text(text, encoding="utf-8")
    return True


def _entity_registry_field(registry: str) -> str:
    return _snake(registry).upper()


def _entity_java(package: str, entity_class: str, registry: str, meta: dict[str, Any], entities_init: str) -> str:
    health = float(meta.get("health", 10.0) or 10.0)
    speed = float(meta.get("movement_speed", 0.3) or 0.3)
    attack = float(meta.get("attack_damage", 3.0) or 3.0)
    armor = float(meta.get("armor", 0.0) or 0.0)
    follow = float(meta.get("follow_range", 16.0) or 16.0)
    field = _entity_registry_field(registry)
    return f'''package {package}.entity;

import net.neoforged.neoforge.network.NetworkHooks;
import net.neoforged.neoforge.network.PlayMessages;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.MobType;
import net.minecraft.world.entity.PathfinderMob;
import net.minecraft.world.entity.ai.attributes.AttributeSupplier;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.ai.goal.RandomLookAroundGoal;
import net.minecraft.world.entity.ai.goal.RandomStrollGoal;
import net.minecraft.world.level.Level;
import {package}.init.{entities_init};

public class {entity_class} extends PathfinderMob {{
    public {entity_class}(PlayMessages.SpawnEntity packet, Level level) {{
        this({entities_init}.{field}.get(), level);
    }}

    public {entity_class}(EntityType<{entity_class}> type, Level level) {{
        super(type, level);
        this.setPersistenceRequired();
    }}

    @Override
    public Packet<ClientGamePacketListener> getAddEntityPacket() {{
        return NetworkHooks.getEntitySpawningPacket(this);
    }}

    @Override
    protected void registerGoals() {{
        super.registerGoals();
        this.goalSelector.addGoal(7, new RandomStrollGoal(this, 1.0));
        this.goalSelector.addGoal(8, new RandomLookAroundGoal(this));
    }}

    @Override
    public MobType getMobType() {{
        return MobType.UNDEFINED;
    }}

    public static void init() {{
    }}

    public static AttributeSupplier.Builder createAttributes() {{
        return Mob.createMobAttributes()
            .add(Attributes.MOVEMENT_SPEED, {speed})
            .add(Attributes.MAX_HEALTH, {health})
            .add(Attributes.ARMOR, {armor})
            .add(Attributes.ATTACK_DAMAGE, {attack})
            .add(Attributes.FOLLOW_RANGE, {follow});
    }}
}}
'''


def _projectile_java(package: str, entity_class: str, registry: str, entities_init: str) -> str:
    field = _entity_registry_field(registry)
    return f'''package {package}.entity;

import net.neoforged.neoforge.network.NetworkHooks;
import net.neoforged.neoforge.network.PlayMessages;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.projectile.Snowball;
import net.minecraft.world.level.Level;
import {package}.init.{entities_init};

public class {entity_class} extends Snowball {{
    public {entity_class}(PlayMessages.SpawnEntity packet, Level level) {{
        this({entities_init}.{field}.get(), level);
    }}

    public {entity_class}(EntityType<{entity_class}> type, Level level) {{
        super(type, level);
    }}

    @Override
    public Packet<ClientGamePacketListener> getAddEntityPacket() {{
        return NetworkHooks.getEntitySpawningPacket(this);
    }}
}}
'''


def _renderer_java(modid: str, package: str, entity_class: str, model_class: str, texture: str, shadow: float) -> str:
    renderer_class = entity_class[:-6] if entity_class.endswith("Entity") else entity_class
    return f'''package {package}.client.renderer;

import net.minecraft.client.renderer.entity.EntityRendererProvider;
import net.minecraft.client.renderer.entity.MobRenderer;
import net.minecraft.resources.ResourceLocation;
import {package}.client.model.{model_class};
import {package}.entity.{entity_class};

public class {renderer_class}Renderer extends MobRenderer<{entity_class}, {model_class}<{entity_class}>> {{
    private static final ResourceLocation TEXTURE = ResourceLocation.fromNamespaceAndPath("{modid}", "textures/entities/{texture}");

    public {renderer_class}Renderer(EntityRendererProvider.Context context) {{
        super(context, new {model_class}<>(context.bakeLayer({model_class}.LAYER_LOCATION)), {float(shadow or 0.5)}F);
    }}

    @Override
    public ResourceLocation getTextureLocation({entity_class} entity) {{
        return TEXTURE;
    }}
}}
'''


def _item_java(package: str, class_name: str, crafted_procedure: str | None = None) -> str:
    imports = "import net.minecraft.world.item.Item;\n"
    hook = ""
    if crafted_procedure:
        imports += (
            "import net.minecraft.world.item.ItemStack;\n"
            "import net.minecraft.world.level.Level;\n"
            "import net.minecraft.world.entity.player.Player;\n"
            f"import {package}.procedures.{crafted_procedure}Procedure;\n"
        )
        hook = (
            "\n    @Override\n"
            "    public void onCraftedBy(ItemStack itemstack, Level world, Player entity) {\n"
            "        super.onCraftedBy(itemstack, world, entity);\n"
            f"        {crafted_procedure}Procedure.execute(world, itemstack);\n"
            "    }\n"
        )
    return (
        f"package {package}.item;\n\n{imports}\n"
        f"public class {class_name} extends Item {{\n"
        f"    public {class_name}() {{\n"
        "        super(new Item.Properties());\n"
        "    }\n"
        f"{hook}}}\n"
    )


def _armor_java(package: str, class_name: str) -> str:
    return f'''package {package}.item;

import net.minecraft.world.item.ArmorItem;
import net.minecraft.world.item.ArmorMaterial;
import net.minecraft.world.item.ArmorMaterials;
import net.minecraft.world.item.Item;

public class {class_name} {{
    private static final ArmorMaterial MATERIAL = ArmorMaterials.DIAMOND;

    public static class Helmet extends ArmorItem {{
        public Helmet() {{ super(MATERIAL, ArmorItem.Type.HELMET, new Item.Properties()); }}
    }}
    public static class Chestplate extends ArmorItem {{
        public Chestplate() {{ super(MATERIAL, ArmorItem.Type.CHESTPLATE, new Item.Properties()); }}
    }}
    public static class Leggings extends ArmorItem {{
        public Leggings() {{ super(MATERIAL, ArmorItem.Type.LEGGINGS, new Item.Properties()); }}
    }}
    public static class Boots extends ArmorItem {{
        public Boots() {{ super(MATERIAL, ArmorItem.Type.BOOTS, new Item.Properties()); }}
    }}
}}
'''


def _block_java(package: str, class_name: str) -> str:
    return f'''package {package}.block;

import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;

public class {class_name} extends Block {{
    public {class_name}() {{
        super(BlockBehaviour.Properties.of().strength(1.0F, 10.0F));
    }}
}}
'''


def _write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def _resource_assets_root(output_root: Path, modid: str) -> Path:
    return output_root / "src" / "main" / "resources" / "assets" / modid


def _texture_resource_path(texture: str) -> str:
    """Return a Minecraft model texture path without a file extension.

    MCreator element definitions generally store PNG filenames, while generated
    model JSON must reference an extensionless ResourceLocation.
    """
    value = str(texture or "").strip().replace("\\", "/")
    if value.lower().endswith(".png"):
        value = value[:-4]
    return value.lstrip("/")


def _write_item_model_resource(output_root: Path, modid: str, registry: str, texture: str) -> str:
    path = _resource_assets_root(output_root, modid) / "models" / "item" / f"{registry}.json"
    _write_json(path, {
        "parent": "item/generated",
        "textures": {"layer0": f"{modid}:item/{_texture_resource_path(texture)}"},
    })
    return str(path.relative_to(output_root))


def _write_block_model_resources(output_root: Path, modid: str, registry: str, texture: str, plant: bool = False) -> list[str]:
    assets = _resource_assets_root(output_root, modid)
    model_path = assets / "models" / "block" / f"{registry}.json"
    state_path = assets / "blockstates" / f"{registry}.json"
    item_path = assets / "models" / "item" / f"{registry}.json"
    if plant:
        model = {"parent": "block/cross", "textures": {"cross": f"{modid}:block/{_texture_resource_path(texture)}"}}
    else:
        model = {"parent": "block/cube_all", "textures": {"all": f"{modid}:block/{_texture_resource_path(texture)}"}}
    _write_json(model_path, model)
    _write_json(state_path, {"variants": {"": {"model": f"{modid}:block/{registry}"}}})
    _write_json(item_path, {"parent": f"{modid}:block/{registry}"})
    return [
        str(model_path.relative_to(output_root)),
        str(state_path.relative_to(output_root)),
        str(item_path.relative_to(output_root)),
    ]


def _write_armor_model_resources(output_root: Path, modid: str, registry: str, element: dict[str, Any]) -> list[str]:
    assets = _resource_assets_root(output_root, modid)
    definition = element.get("definition", {}) if isinstance(element, dict) else {}
    written: list[str] = []
    mapping = (("helmet", "textureHelmet"), ("chestplate", "textureBody"), ("leggings", "textureLeggings"), ("boots", "textureBoots"))
    for suffix, key in mapping:
        if not definition.get(f"enable{suffix.capitalize()}"):
            continue
        texture = str(definition.get(key, "")).strip()
        if not texture:
            continue
        path = assets / "models" / "item" / f"{registry}_{suffix}.json"
        _write_json(path, {
            "parent": "item/generated",
            "textures": {"layer0": f"{modid}:item/{_texture_resource_path(texture)}"},
        })
        written.append(str(path.relative_to(output_root)))
    return written


def _patch_entity_registry(output_root: Path, package: str, modid: str, entity_exports: list[Any], content_exports: list[Any], workspace: dict[str, Any]) -> dict[str, int]:
    entities_path = _find_init_file(output_root, package, "ModEntities")
    entities_init = entities_path.stem if entities_path else _expected_mod_class(modid, "Entities")
    renderers_path = _find_init_file(output_root, package, "ModEntityRenderers")
    models_path = _find_init_file(output_root, package, "ModModels")
    if not entities_path:
        return {"entities": 0, "projectiles": 0, "renderers": 0, "models": 0}

    text = entities_path.read_text(encoding="utf-8", errors="ignore")
    existing_regs = set(re.findall(r'\bregister\("([^"]+)"', text))
    entity_fields: list[str] = []
    init_calls: list[str] = []
    attr_calls: list[str] = []
    renderer_calls: list[str] = []
    model_calls: list[str] = []
    counts = {"entities": 0, "projectiles": 0, "renderers": 0, "models": 0}

    projectile_regs = {str(x.registry_name).lower() for x in content_exports if x.element_type == "projectile"}

    for export in entity_exports:
        registry = str(export.registry_name)
        if registry.lower() in {x.lower() for x in existing_regs}:
            continue
        entity_class = f"{_pascal(export.element_name)}Entity"
        field = _entity_registry_field(registry)
        is_projectile = registry.lower() in projectile_regs
        category = "MobCategory.MISC" if is_projectile else ("MobCategory.WATER_CREATURE" if "water" in registry.lower() else "MobCategory.CREATURE")
        w, h = (0.5, 0.5) if is_projectile else (max(0.2, 0.6), max(0.2, 1.8))
        entity_fields.append(
            f'\tpublic static final RegistryObject<EntityType<{entity_class}>> {field} = register("{registry}", '
            f'EntityType.Builder.<{entity_class}>of({entity_class}::new, {category}).setShouldReceiveVelocityUpdates(true).setTrackingRange(64).setUpdateInterval(3).setCustomClientFactory({entity_class}::new).sized({w}f, {h}f));'
        )
        init_calls.append(f'\t\t\t{entity_class}.init();')
        if not is_projectile:
            attr_calls.append(f'\t\tevent.put({field}.get(), {entity_class}.createAttributes().build());')
            renderer_class = f"{_pascal(export.element_name)}Renderer"
            renderer_calls.append(f'\t\tevent.registerEntityRenderer({entities_init}.{field}.get(), {renderer_class}::new);')
            if export.generated_model and not str(export.source_model).startswith("ModelBiped"):
                model_calls.append(f'\t\tevent.registerLayerDefinition({export.generated_model}.LAYER_LOCATION, {export.generated_model}::createBodyLayer);')
                counts["models"] += 1
            counts["renderers"] += 1
            counts["entities"] += 1
        else:
            renderer_calls.append(f'\t\tevent.registerEntityRenderer({entities_init}.{field}.get(), ThrownItemRenderer::new);')
            counts["renderers"] += 1
            counts["projectiles"] += 1
        existing_regs.add(registry)

    for item in content_exports:
        if item.element_type != "projectile" or str(item.registry_name).lower() in {x.lower() for x in existing_regs}:
            continue
        registry = str(item.registry_name)
        entity_class = f"{_pascal(item.element_name)}Entity"
        field = _entity_registry_field(registry)
        entity_fields.append(
            f'\tpublic static final RegistryObject<EntityType<{entity_class}>> {field} = register("{registry}", '
            f'EntityType.Builder.<{entity_class}>of({entity_class}::new, MobCategory.MISC).setCustomClientFactory({entity_class}::new).setShouldReceiveVelocityUpdates(true).setTrackingRange(64).setUpdateInterval(1).sized(0.5f, 0.5f));'
        )
        renderer_calls.append(f'\t\tevent.registerEntityRenderer({entities_init}.{field}.get(), ThrownItemRenderer::new);')
        counts["projectiles"] += 1
        counts["renderers"] += 1
        existing_regs.add(registry)

    if entity_fields:
        # fields immediately before the generated/user-custom marker or register helper
        marker = "\t// Start of user code block custom entities"
        if marker in text:
            text = text.replace(marker, "\n\n".join(entity_fields) + "\n\n" + marker, 1)
        else:
            idx = text.find("\tprivate static <T extends Entity> RegistryObject")
            if idx >= 0:
                text = text[:idx] + "\n".join(entity_fields) + "\n" + text[idx:]
        if init_calls and "// MojModelBridge generated entity init" not in text:
            span = _method_body(text, "public static void init(FMLCommonSetupEvent event)")
            if span:
                _, close = span
                insertion = "\n\t\t\t// MojModelBridge generated entity init\n" + "\n".join(init_calls) + "\n"
                text = text[:close] + insertion + text[close:]
        if attr_calls and "// MojModelBridge generated attributes" not in text:
            span = _method_body(text, "public static void registerAttributes(EntityAttributeCreationEvent event)")
            if span:
                _, close = span
                insertion = "\n\t\t// MojModelBridge generated attributes\n" + "\n".join(attr_calls) + "\n"
                text = text[:close] + insertion + text[close:]
        entities_path.write_text(text, encoding="utf-8")

    if renderers_path and renderer_calls:
        if "// MojModelBridge generated renderers" not in renderers_path.read_text(encoding="utf-8", errors="ignore"):
            _insert_before_method_close(renderers_path, "public static void registerEntityRenderers(EntityRenderersEvent.RegisterRenderers event)", "\n".join(renderer_calls), "// MojModelBridge generated renderers")
    if models_path and model_calls:
        if "// MojModelBridge generated models" not in models_path.read_text(encoding="utf-8", errors="ignore"):
            _insert_before_method_close(models_path, "public static void registerLayerDefinitions(EntityRenderersEvent.RegisterLayerDefinitions event)", "\n".join(model_calls), "// MojModelBridge generated models")

    return counts


def _patch_item_block_registries(output_root: Path, package: str, modid: str, content_exports: list[Any]) -> dict[str, int]:
    items_path = _find_init_file(output_root, package, "ModItems")
    blocks_path = _find_init_file(output_root, package, "ModBlocks")
    blocks_init = blocks_path.stem if blocks_path else _expected_mod_class(modid, "Blocks")
    counts = {"items": 0, "blocks": 0, "block_items": 0}
    item_fields: list[str] = []
    item_regs: list[str] = []
    block_fields: list[str] = []
    block_regs: list[str] = []

    items_text = items_path.read_text(encoding="utf-8", errors="ignore") if items_path else ""
    blocks_text = blocks_path.read_text(encoding="utf-8", errors="ignore") if blocks_path else ""
    existing_item_regs = set(re.findall(r'REGISTRY\.register\("([^"]+)"', items_text))
    existing_block_regs = set(re.findall(r'REGISTRY\.register\("([^"]+)"', blocks_text))
    # Fields may be declared even if registration parsing missed a custom helper.
    existing_item_fields = set(re.findall(r'RegistryObject<Item>\s+([A-Z0-9_]+)', items_text))
    existing_block_fields = set(re.findall(r'RegistryObject<Block>\s+([A-Z0-9_]+)', blocks_text))

    # Explicit item registrations must be resolved before blocks.  Minecraft has
    # separate block and item registries, so a block and a custom placement/food
    # item may legitimately share a path.  In that case the custom item owns the
    # item-registry entry and the block must not receive an automatic BlockItem.
    priority = {"item": 0, "tool": 0, "armor": 0, "block": 1, "plant": 1}
    ordered_exports = sorted(content_exports, key=lambda value: (priority.get(str(value.element_type), 2), str(value.registry_name)))
    for item in ordered_exports:
        kind = str(item.element_type)
        registry = str(item.registry_name)
        if kind in {"item", "tool"}:
            if registry in existing_item_regs or _entity_registry_field(registry) in existing_item_fields:
                continue
            cls = f"{_pascal(item.element_name)}Item"
            field = _entity_registry_field(registry)
            item_fields.append(f'\tpublic static final RegistryObject<Item> {field};')
            item_regs.append(f'\t\t{field} = REGISTRY.register("{registry}", {cls}::new);')
            counts["items"] += 1
        elif kind == "armor":
            cls = f"{_pascal(item.element_name)}ArmorItem"
            for suffix, nested in (("helmet", "Helmet"), ("chestplate", "Chestplate"), ("leggings", "Leggings"), ("boots", "Boots")):
                piece = f"{registry}_{suffix}"
                if piece in existing_item_regs:
                    continue
                field = _entity_registry_field(piece)
                item_fields.append(f'\tpublic static final RegistryObject<Item> {field};')
                item_regs.append(f'\t\t{field} = REGISTRY.register("{piece}", {cls}.{nested}::new);')
                counts["items"] += 1
        elif kind in {"block", "plant"}:
            field = _entity_registry_field(registry)
            cls = f"{_pascal(item.element_name)}Block"
            if registry not in existing_block_regs and field not in existing_block_fields:
                block_fields.append(f'\tpublic static final RegistryObject<Block> {field};')
                block_regs.append(f'\t\t{field} = REGISTRY.register("{registry}", {cls}::new);')
                counts["blocks"] += 1
            if registry not in existing_item_regs and field not in existing_item_fields:
                item_fields.append(f'\tpublic static final RegistryObject<Item> {field};')
                item_regs.append(f'\t\t{field} = REGISTRY.register("{registry}", () -> new BlockItem({blocks_init}.{field}.get(), new Item.Properties()));')
                counts["items"] += 1
                counts["block_items"] += 1

    if items_path and item_fields:
        text = items_path.read_text(encoding="utf-8", errors="ignore")
        marker = "// MojModelBridge generated item fields"
        if marker not in text:
            insert_at = text.find("\tstatic {")
            if insert_at >= 0:
                text = text[:insert_at] + "\n\t" + marker + "\n" + "\n".join(item_fields) + "\n" + text[insert_at:]
                items_path.write_text(text, encoding="utf-8")
            # re-read after field insertion
        if "// MojModelBridge generated item registrations" not in items_path.read_text(encoding="utf-8", errors="ignore"):
            _insert_before_method_close(items_path, "static {", "\n".join(item_regs), "// MojModelBridge generated item registrations")
    if blocks_path and block_fields:
        text = blocks_path.read_text(encoding="utf-8", errors="ignore")
        marker = "// MojModelBridge generated block fields"
        if marker not in text:
            insert_at = text.find("\tstatic {")
            if insert_at >= 0:
                text = text[:insert_at] + "\n\t" + marker + "\n" + "\n".join(block_fields) + "\n" + text[insert_at:]
                blocks_path.write_text(text, encoding="utf-8")
        if "// MojModelBridge generated block registrations" not in blocks_path.read_text(encoding="utf-8", errors="ignore"):
            _insert_before_method_close(blocks_path, "static {", "\n".join(block_regs), "// MojModelBridge generated block registrations")
    return counts



def _patch_crafted_procedure_calls(output_root: Path) -> int:
    """Repair stale zero-argument crafted-procedure calls in generated item classes."""
    java_root = output_root / "src" / "main" / "java"
    if not java_root.exists():
        return 0
    changed = 0
    method_re = re.compile(
        r"(public\s+void\s+onCraftedBy\s*\(\s*ItemStack\s+(\w+)\s*,\s*Level\s+(\w+)\s*,.*?\)\s*\{)(.*?)(\n\s*\})",
        re.DOTALL,
    )
    call_re = re.compile(r"(\b[A-Za-z_$][\w$]*Procedure\.execute)\s*\(\s*\)\s*;")
    for path in java_root.rglob("*.java"):
        text = path.read_text(encoding="utf-8", errors="ignore")
        def replace_method(match: re.Match[str]) -> str:
            nonlocal changed
            head, item_var, level_var, body, tail = match.groups()
            body2, count = call_re.subn(lambda m: f"{m.group(1)}({level_var}, {item_var});", body)
            changed += count
            return head + body2 + tail
        patched = method_re.sub(replace_method, text)
        if patched != text:
            path.write_text(patched, encoding="utf-8")
    return changed

def generate_compile_bridge(
    output_root: Path,
    package: str,
    modid: str,
    workspace: dict[str, Any],
    entity_exports: list[Any],
    content_exports: list[Any],
    entity_meta: dict[str, dict[str, Any]],
    log=None,
    target_generator: str = "neoforge-1.21.1",
) -> dict[str, Any]:
    """Make the generated MCreator workspace self-consistent enough to compile before regeneration."""
    # This patch is generator-agnostic: NeoForge workspaces imported into an existing
    # workspace can contain stale item Java classes that still call crafted procedures
    # with the old zero-argument signature. Repair those before the NeoForge early-return.
    patched_crafted_calls = _patch_crafted_procedure_calls(output_root)
    if target_generator == "neoforge-1.21.1":
        if log:
            log(f"NeoForge 1.21.1 target: skipped legacy compile bridge; repaired {patched_crafted_calls} stale crafted-procedure call(s).")
        bridge_report = output_root / "MOJMODELBRIDGE_SOURCE_BRIDGE_REPORT.json"
        bridge_report.write_text(json.dumps({
            "generated_files": {},
            "new_counts": {},
            "registry_stats": {},
            "copied_model_sources": 0,
            "patched_crafted_procedure_calls": patched_crafted_calls,
            "skipped": True,
            "reason": "native_neoforge_generator",
        }, indent=2) + "\n", encoding="utf-8")
        return {"skipped": True, "reason": "native_neoforge_generator", "generated_files": {}, "patched_crafted_procedure_calls": patched_crafted_calls}
    java_root = output_root / "src" / "main" / "java" / _java_package_path(package)
    models_dir = output_root / "models" / "mojmap-1.20.x"
    animations_dir = output_root / "models" / "animations"
    model_files = list(models_dir.glob("*.java")) if models_dir.exists() else []
    animation_files = list(animations_dir.glob("*.java")) if animations_dir.exists() else []
    copied_models = _copy_model_sources(output_root, package, model_files, animation_files)
    generated_files: dict[str, list[str]] = {}
    new_counts = {"entity": 0, "projectile": 0, "renderer": 0, "item": 0, "block": 0}
    entities_init = _init_class_name(output_root, package, "Entities", modid)

    built_in_models = {"ModelBiped", "ModelCow", "ModelSpider", "ModelPig", "ModelSheep1", "ModelSheep2", "ModelChicken", "ModelWolf", "ModelZombie", "ModelSkeleton"}

    for export in entity_exports:
        if getattr(export, "status", "created") not in {"created", "resumed"}:
            continue
        element_name = str(export.element_name)
        registry = str(export.registry_name)
        entity_class = f"{_pascal(element_name)}Entity"
        entity_path = java_root / "entity" / f"{entity_class}.java"
        meta = entity_meta.get(element_name.lower(), {})
        if not entity_path.exists():
            _write(entity_path, _entity_java(package, entity_class, registry, meta, entities_init))
            new_counts["entity"] += 1
        files = [str(entity_path.relative_to(output_root))]

        if export.generated_model and export.source_model not in built_in_models:
            model_path = java_root / "client" / "model" / f"{export.generated_model}.java"
            anim_path = java_root / "client" / "model" / "animations" / f"{export.generated_model}Animation.java"
            if model_path.exists():
                files.append(str(model_path.relative_to(output_root)))
            if anim_path.exists():
                files.append(str(anim_path.relative_to(output_root)))
            texture = Path(export.texture_output).name if export.texture_output else f"{registry}.png"
            renderer_class = f"{_pascal(element_name)}Renderer"
            renderer_path = java_root / "client" / "renderer" / f"{renderer_class}.java"
            if not renderer_path.exists():
                _write(renderer_path, _renderer_java(modid, package, entity_class, export.generated_model, texture, 0.5))
                new_counts["renderer"] += 1
            files.append(str(renderer_path.relative_to(output_root)))
        _ensure_workspace_metadata(workspace, element_name, files)
        generated_files[element_name] = files

    for item in content_exports:
        if getattr(item, "status", "created") not in {"created", "resumed"}:
            continue
        element_name = str(item.element_name)
        registry = str(item.registry_name)
        kind = str(item.element_type)
        files = list(generated_files.get(element_name, []))
        texture = Path(item.texture_output).name if item.texture_output else f"{registry}.png"
        if kind in {"item", "tool"}:
            cls = f"{_pascal(element_name)}Item"
            path = java_root / "item" / f"{cls}.java"
            element_path = output_root / "elements" / f"{element_name}.mod.json"
            crafted_procedure = None
            if element_path.exists():
                try:
                    element_data = json.loads(element_path.read_text(encoding="utf-8"))
                    crafted_procedure = str(element_data.get("definition", {}).get("onCrafted") or "").strip() or None
                except Exception:
                    crafted_procedure = None
            # Do not emit the old zero-argument procedure call. MCreator's generated
            # procedure signature requires the crafting Level and ItemStack context.
            if not path.exists():
                _write(path, _item_java(package, cls, crafted_procedure))
                new_counts["item"] += 1
            elif crafted_procedure:
                text = path.read_text(encoding="utf-8", errors="ignore")
                if f"{crafted_procedure}Procedure.execute();" in text or "onCraftedBy(ItemStack itemstack, Level world" not in text:
                    _write(path, _item_java(package, cls, crafted_procedure))
            files.append(str(path.relative_to(output_root)))
            if texture:
                files.append(_write_item_model_resource(output_root, modid, registry, texture))
        elif kind == "armor":
            cls = f"{_pascal(element_name)}ArmorItem"
            path = java_root / "item" / f"{cls}.java"
            if not path.exists():
                _write(path, _armor_java(package, cls))
                new_counts["item"] += 4
            files.append(str(path.relative_to(output_root)))
            element_path = output_root / "elements" / f"{element_name}.mod.json"
            if element_path.exists():
                try:
                    files.extend(_write_armor_model_resources(output_root, modid, registry, json.loads(element_path.read_text(encoding="utf-8"))))
                except Exception:
                    pass
        elif kind in {"block", "plant"}:
            cls = f"{_pascal(element_name)}Block"
            path = java_root / "block" / f"{cls}.java"
            if not path.exists():
                _write(path, _block_java(package, cls))
                new_counts["block"] += 1
            files.append(str(path.relative_to(output_root)))
            if texture:
                files.extend(_write_block_model_resources(output_root, modid, registry, texture, plant=(kind == "plant")))
        elif kind == "projectile":
            cls = f"{_pascal(element_name)}Entity"
            path = java_root / "entity" / f"{cls}.java"
            if not path.exists():
                _write(path, _projectile_java(package, cls, registry, entities_init))
                new_counts["projectile"] += 1
            files.append(str(path.relative_to(output_root)))
        _ensure_workspace_metadata(workspace, element_name, files)
        generated_files[element_name] = files

    created_entities = [export for export in entity_exports if getattr(export, "status", "created") in {"created", "resumed"}]
    created_content = [item for item in content_exports if getattr(item, "status", "created") in {"created", "resumed"}]
    registry_stats = _patch_entity_registry(output_root, package, modid, created_entities, created_content, workspace)
    registry_stats.update(_patch_item_block_registries(output_root, package, modid, created_content))

    bridge_report = output_root / "MOJMODELBRIDGE_SOURCE_BRIDGE_REPORT.json"
    bridge_report.write_text(json.dumps({"generated_files": generated_files, "new_counts": new_counts, "registry_stats": registry_stats, "copied_model_sources": len(copied_models), "patched_crafted_procedure_calls": patched_crafted_calls}, indent=2) + "\n", encoding="utf-8")
    if log:
        log(f"MCreator source bridge: generated {new_counts}; registry patches={registry_stats}; copied model/animation sources={len(copied_models)}; crafted procedure calls patched={patched_crafted_calls}")
    return {"generated_files": generated_files, "new_counts": new_counts, "registry_stats": registry_stats, "copied_model_sources": len(copied_models), "patched_crafted_procedure_calls": patched_crafted_calls}
