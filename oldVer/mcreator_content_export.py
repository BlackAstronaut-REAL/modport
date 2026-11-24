from __future__ import annotations
import copy
import json
import math
import re
import struct
import zlib
from difflib import SequenceMatcher
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Optional
_TEXTURE_FILE_INDEX_CACHE: dict[tuple[str, ...], list[tuple[int, Path]]] = {}
from legacy_bridge import _method_info, _parse_methods, _run_javap, prime_javap_cache
from legacy_mod_graph import (
    ConstructedObject,
    LegacyModGraph,
    _class_from_comment,
    _descriptor_args,
    _iter_java_calls,
    _push_constant,
    _split_java_arguments,
)
@dataclass
class MCreatorContentExport:
    element_name: str
    registry_name: str
    element_type: str
    source_class: str
    status: str
    warnings: list[str] = field(default_factory=list)
    texture_ref: str = ""
    texture_output: str = ""
    sound_bindings: list[str] = field(default_factory=list)
@dataclass
class MCreatorContentResult:
    exports: list[MCreatorContentExport] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
def _log(log: Optional[Callable[[str], None]], message: str) -> None:
    if log:
        log(message)
def _simple(name: str) -> str:
    return name.replace("/", ".").split(".")[-1].split("$")[0]
def _snake(value: str) -> str:
    value = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", value)
    value = re.sub(r"[^A-Za-z0-9]+", "_", value).strip("_").lower()
    return value or "converted_content"
def _compact_registry(value: str) -> str:
    return re.sub(r"[^a-z0-9]", "", str(value).lower())
def _normalise_source_registry(simple: str, raw_registry: str) -> str:
    """Normalize a legacy registry key while preserving unknown aliases.
    Many 1.7.10 registries store keys like ``ultimatesword`` while the modern
    MCreator workspace uses ``ultimate_sword``. If the raw key is the compact form
    of the class name, prefer the class-derived snake_case value. For distinct aliases
    (e.g. multiple registrations backed by one Item class), retain the raw key.
    """
    stripped = re.sub(r"^(Item|Block|WorldProvider|BiomeGen)", "", _simple(simple)) or _simple(simple)
    if _compact_registry(stripped) == _compact_registry(raw_registry):
        return _snake(stripped)
    return _snake(raw_registry)
def _pascal(value: str) -> str:
    words = re.findall(r"[A-Za-z0-9]+", re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", value))
    result = "".join(word[:1].upper() + word[1:] for word in words)
    if not result:
        result = "ConvertedContent"
    if result[0].isdigit():
        result = "Content" + result
    return result
def _java_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)
def _sound_inventory(workspace: dict[str, Any]) -> dict[str, dict[str, Any]]:
    inventory: dict[str, dict[str, Any]] = {}
    for item in workspace.get("sound_elements", []):
        if not isinstance(item, dict):
            continue
        name = str(item.get("name", "")).strip()
        if name:
            inventory[name.lower()] = item
    return inventory


def _sound_role_keywords(kind: str) -> dict[str, tuple[str, ...]]:
    kind = str(kind or "").lower()
    if kind == "livingentity":
        return {
            "livingSound": ("living", "ambient", "idle", "loop", "roar", "growl", "breath", "mob"),
            "hurtSound": ("hurt", "damage", "hit", "injur"),
            "deathSound": ("death", "die", "kill", "slay"),
            "stepSound": ("step", "walk", "foot", "move"),
            "raidCelebrationSound": ("celebrat", "victory", "raid"),
        }
    if kind == "armor":
        return {"equipSound": ("equip", "armor", "wear", "don", "put on")}
    if kind == "projectile":
        return {"actionSound": ("shoot", "fire", "launch", "throw", "release", "projectile")}
    if kind == "block":
        return {"soundOnStep": ("step", "walk", "foot", "stone", "grass", "wood", "metal", "block")}
    return {}


def _best_sound_event(source_text: str, registry: str, candidates: dict[str, dict[str, Any]], role_terms: tuple[str, ...]) -> str:
    source = re.sub(r"[^a-z0-9]+", " ", str(source_text or "").lower())
    compact_source = re.sub(r"[^a-z0-9]", "", source)
    compact_registry = re.sub(r"[^a-z0-9]", "", str(registry or "").lower())
    best_name = ""
    best_score = -1
    for name, data in candidates.items():
        candidate = re.sub(r"[^a-z0-9]+", " ", name.lower())
        compact_name = re.sub(r"[^a-z0-9]", "", name.lower())
        score = 0
        if compact_registry and (compact_registry in compact_name or compact_name in compact_registry):
            score += 8
        if any(term in compact_source for term in role_terms):
            score += 5
        if any(term in candidate for term in role_terms):
            score += 4
        score += min(len(set(compact_name) & set(compact_source)), 5)
        if score > best_score:
            best_name = data.get("name", name) or name
            best_score = score
    return best_name


_SOUND_TYPE_MAP = {
    "STONE": "STONE", "GRASS": "GRASS", "WOOD": "WOOD", "LEAVES": "LEAVES",
    "GLASS": "GLASS", "METAL": "METAL", "PISTON": "PISTON", "CLOTH": "WOOL",
    "SAND": "SAND", "SNOW": "SNOW", "LADDER": "LADDER", "ANVIL": "ANVIL",
    "SLIME_BLOCK": "SLIME", "HONEY_BLOCK": "HONEY", "NETHERRACK": "NETHERRACK",
    "NETHER_BRICKS": "NETHER_BRICKS", "BONE_BLOCK": "BONE", "NETHERITE_BLOCK": "NETHERITE",
    "BAMBOO": "BAMBOO", "BAMBOO_SAPLING": "BAMBOO", "CROP": "PLANT", "FLOWERING_AZALEA": "PLANT",
    "ROOTED_DIRT": "GROUND", "SCULK": "SCULK", "WET_GRASS": "GRASS", "FUNGUS": "PLANT",
    "CANDLE": "CANDLE", "AMETHYST": "AMETHYST", "COPPER": "COPPER", "POWDER_SNOW": "POWDER_SNOW",
}

def _extract_block_sound_type(source_text: str) -> str:
    for m in re.finditer(r"\.sound\s*\(\s*(?:net\.minecraft\.world\.level\.block\.)?SoundType\.([A-Za-z0-9_]+)", source_text):
        raw = m.group(1).upper()
        return _SOUND_TYPE_MAP.get(raw, raw if raw in _SOUND_TYPE_MAP.values() else "STONE")
    return ""

def _extract_block_drop(source_text: str, field_registry_map: Optional[dict[str, set[str]]] = None) -> tuple[str, int, bool]:
    # Empty-drop blocks should explicitly disable drops.
    if re.search(r"return\s+Collections\.emptyList\s*\(\s*\)", source_text):
        return "", 0, True
    # Return a concrete ItemStack(X, count) and resolve source fields through the registry map when possible.
    m = re.search(r"new\s+ItemStack\s*\(\s*([A-Za-z_$][\w$\.]*)\s*(?:,\s*(\d+)\s*)?\)", source_text)
    if not m:
        return "", 1, False
    ref = m.group(1).split(".")[-1]
    count = int(m.group(2) or 1)
    if ref in {"this", "self"}:
        return "", count, True
    if ref == "this" or ref == source_text:
        return "", count, True
    if field_registry_map and ref in field_registry_map:
        regs = sorted(field_registry_map[ref])
        if regs:
            return f"Items.{re.sub(r"[^A-Za-z0-9]+", "_", regs[0]).upper()}", count, True
    # Vanilla static references (Items.X / Blocks.X) can be passed directly.
    if ref.startswith("Items."):
        return f"Items.{ref.split('.',1)[1]}", count, True
    if ref.startswith("Blocks."):
        return f"Items.{ref.split('.',1)[1]}", count, True
    return "", count, False

def _block_custom_drop_updates(source_text: str, field_registry_map: Optional[dict[str, set[str]]] = None) -> dict[str, Any]:
    drop, count, found = _extract_block_drop(source_text, field_registry_map)
    if not found:
        return {}
    return {"customDrop": drop, "dropAmount": max(0, int(count)), "useLootTableForDrops": False}

def infer_sound_bindings(source_text: str, registry: str, kind: str, workspace: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    inventory = _sound_inventory(workspace)
    if not inventory:
        return {}, []
    updates: dict[str, Any] = {}
    bindings: list[str] = []
    role_map = _sound_role_keywords(kind)
    for field, terms in role_map.items():
        chosen = _best_sound_event(f"{source_text} {registry}", registry, inventory, terms)
        if chosen:
            updates[field] = chosen
            bindings.append(f"{field} -> {chosen}")
    if kind == "block":
        source_sound = _extract_block_sound_type(source_text)
        if source_sound:
            updates = {
                "soundOnStep": source_sound,
                "isCustomSoundType": False,
                "breakSound": "", "fallSound": "", "hitSound": "", "placeSound": "", "stepSound": "",
            }
            bindings = [f"soundOnStep -> {source_sound} (source SoundType)"]
        else:
            updates.setdefault("soundOnStep", "STONE")
            updates["isCustomSoundType"] = False
            for key in ("breakSound", "fallSound", "hitSound", "placeSound", "stepSound"):
                updates.setdefault(key, "")
    return updates, bindings


def _write_placeholder_png(path: Path, size: int = 16) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            checker = ((x // 4) + (y // 4)) % 2
            rows.extend((255, 0, 255, 255) if checker == 0 else (20, 20, 20, 255))
    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    path.write_bytes(png)
def _copy_texture_with_metadata(source: Path, target: Path) -> None:
    """Copy a PNG and its optional vanilla/Forge animation metadata sidecar."""
    target.parent.mkdir(parents=True, exist_ok=True)
    if source.resolve() != target.resolve():
        target.write_bytes(source.read_bytes())
    source_meta = source.with_name(source.name + ".mcmeta")
    if source_meta.exists():
        target_meta = target.with_name(target.name + ".mcmeta")
        target_meta.write_bytes(source_meta.read_bytes())
def _read_json(path: Path) -> Optional[dict[str, Any]]:
    try:
        data = json.loads(path.read_text(encoding="utf-8-sig"))
    except Exception:
        return None
    return data if isinstance(data, dict) else None
def _template_definition(workspace_root: Path, element_type: str) -> tuple[int, dict[str, Any]]:
    elements_dir = workspace_root / "elements"
    preferred = {
        "item": ["ItemSchema.mod.json"],
        "tool": ["ToolSchema.mod.json"],
        "armor": ["ArmorSchema.mod.json"],
        "block": ["BlockSchema.mod.json"],
        "dimension": ["DimensionSchema.mod.json"],
        "biome": ["BiomeSchema.mod.json"],
        "plant": ["PlantSchema.mod.json"],
        "projectile": ["ProjectileSchema.mod.json"],
    }
    # Prefer the supplied workspace's real element schema. For standalone mode,
    # fall back to compact schema exemplars shipped with MojModelBridge so the
    # full-port path can still create non-entity elements without requiring a template.
    search_roots = [elements_dir]
    bundled = Path(__file__).resolve().parent / "mcreator_templates"
    if bundled.exists():
        search_roots.append(bundled)
    # Always prefer the bundled canonical schema for supported core element types.
    # Template workspaces often contain a complex example element of the same type
    # (custom model, states, procedures, etc.); using that element as a schema can
    # silently carry incompatible data into every imported element.
    if bundled.exists():
        for filename in preferred.get(element_type, []):
            data = _read_json(bundled / filename)
            if data and data.get("_type") == element_type and isinstance(data.get("definition"), dict):
                return int(data.get("_fv", 89)), copy.deepcopy(data["definition"])
    for root in search_roots:
        for filename in preferred.get(element_type, []):
            data = _read_json(root / filename)
            if data and data.get("_type") == element_type and isinstance(data.get("definition"), dict):
                return int(data.get("_fv", 89)), copy.deepcopy(data["definition"])
        for path in sorted(root.glob("*.mod.json")):
            data = _read_json(path)
            if data and data.get("_type") == element_type and isinstance(data.get("definition"), dict):
                return int(data.get("_fv", 89)), copy.deepcopy(data["definition"])
    raise RuntimeError(f"No MCreator template definition for element type '{element_type}'.")
def _class_header_details(source: Path) -> tuple[str, str, int]:
    """Read class/superclass names and JVM access flags from a class file."""
    data = source.read_bytes()
    if len(data) < 10 or data[:4] != b"\xCA\xFE\xBA\xBE":
        return _simple(source.stem), "", 0
    pos = 8
    cp_count = int.from_bytes(data[pos:pos+2], "big"); pos += 2
    cp: list[Any] = [None] * cp_count
    i = 1
    while i < cp_count:
        tag = data[pos]; pos += 1
        if tag == 1:
            length = int.from_bytes(data[pos:pos+2], "big"); pos += 2
            cp[i] = data[pos:pos+length].decode("utf-8", errors="replace"); pos += length
        elif tag in (3, 4):
            pos += 4
        elif tag in (5, 6):
            pos += 8; i += 1
        elif tag in (7, 8, 16, 19, 20):
            cp[i] = int.from_bytes(data[pos:pos+2], "big"); pos += 2
        elif tag in (9, 10, 11, 12, 17, 18):
            pos += 4
        elif tag == 15:
            pos += 3
        else:
            return _simple(source.stem), "", 0
        i += 1
    access_flags = int.from_bytes(data[pos:pos+2], "big"); pos += 2
    this_idx = int.from_bytes(data[pos:pos+2], "big"); pos += 2
    super_idx = int.from_bytes(data[pos:pos+2], "big"); pos += 2
    def resolve_class(idx: int) -> str:
        if not idx or idx >= len(cp):
            return ""
        name_idx = cp[idx]
        if not isinstance(name_idx, int) or name_idx >= len(cp):
            return ""
        value = cp[name_idx]
        if not isinstance(value, str):
            return ""
        return value.replace("/", ".")
    return _simple(resolve_class(this_idx) or source.stem), _simple(resolve_class(super_idx)), access_flags
def _class_header_info(source: Path) -> tuple[str, str]:
    simple, superclass, _flags = _class_header_details(source)
    return simple, superclass
def _source_class_header_details(source: Path) -> tuple[str, str, int, str]:
    """Read a decompiled Java class declaration without requiring bytecode.
    Returns ``(simple_name, superclass, access_flags, source_text)``.  This is
    intentionally conservative: it only needs enough information to classify
    content types and skip abstract/interface implementation classes.
    """
    try:
        text = source.read_text(encoding="utf-8-sig", errors="replace")
    except OSError:
        return _simple(source.stem), "", 0, ""
    header = re.search(
        r"(?m)^(?:\s*(?:public|protected|private|static|final|abstract|strictfp)\s+)*"
        r"(class|interface|enum)\s+([A-Za-z_$][\w$]*)"
        r"(?:\s+extends\s+([A-Za-z_$][\w.$]*))?",
        text,
    )
    if not header:
        return _simple(source.stem), "", 0, text
    kind, simple, superclass = header.groups()
    modifiers = header.group(0).split("class", 1)[0].split("interface", 1)[0].split("enum", 1)[0]
    flags = 0
    if kind == "interface":
        flags |= 0x0200
    if re.search(r"\babstract\s+", modifiers):
        flags |= 0x0400
    return _simple(simple), _simple(superclass or ""), flags, text
def _class_header_info_any(source: Path) -> tuple[str, str]:
    if source.suffix.lower() == ".java":
        simple, superclass, _flags, _text = _source_class_header_details(source)
        return simple, superclass
    return _class_header_info(source)
def _classify_header(superclass: str, simple: str) -> tuple[str, str, str]:
    super_lower = superclass.lower()
    simple_lower = simple.lower()
    # Registry/initializer containers often start with Item/Block but extend
    # Object and do not represent content themselves (ItemInit, BlockRegistry).
    if super_lower in {"", "object"} and simple_lower.endswith(("init", "registry", "registrar")):
        return "unknown", simple, superclass
    if "itemarmor" in super_lower or ("armor" in simple_lower and simple_lower.endswith(("helmet", "body", "legs", "boots", "chestplate"))):
        return "armor_piece", simple, superclass
    tool_type = _classify_tool(superclass, simple)
    if tool_type:
        return "tool", simple, superclass
    if "itemfood" in super_lower or "itemseedfood" in super_lower:
        return "item", simple, superclass
    # Custom item bases conventionally retain the Item prefix. Avoid broad
    # substring matching: renderer layers such as LayerHeldItem are not content.
    if super_lower == "item" or super_lower.startswith("item") or simple_lower.startswith("item"):
        return "item", simple, superclass
    if any(token in super_lower for token in ("blockflower", "blockbush", "blockcrops", "blocksapling", "blocktallgrass", "blockplant", "blockreed")) or any(token in simple_lower for token in ("plant", "sapling", "flower", "bush", "crop")):
        return "plant", simple, superclass
    if super_lower == "block" or super_lower.startswith("block") or simple_lower.startswith("block"):
        return "block", simple, superclass
    if "worldprovider" in super_lower:
        return "dimension", simple, superclass
    if "biomegenbase" in super_lower or ("biome" in super_lower and "worldgen" not in simple_lower):
        return "biome", simple, superclass
    if any(token in super_lower for token in ("entityarrow", "entitythrowable", "entityprojectile")):
        return "projectile", simple, superclass
    return "unknown", simple, superclass
def _element_name_from_class(simple: str, registry: str) -> str:
    stripped = re.sub(r"^(Item|Block|WorldProvider|BiomeGen)", "", simple) or simple
    compact_simple = re.sub(r"[^a-z0-9]", "", stripped.lower())
    compact_registry = re.sub(r"[^a-z0-9]", "", registry.lower())
    # MCreator's internal element name is not the runtime registry path. Keep the
    # readable source class name when the registered key merely adds a mod prefix
    # or a type suffix (OreSpawn_AmethystAxe, OreSpawn_PizzaBlock). Reusable
    # classes whose keys represent genuinely different content still use the
    # registry-derived name and are deduplicated normally.
    registry_variants = {compact_registry}
    for suffix in ("item", "block", "tool", "weapon"):
        if compact_registry.endswith(suffix):
            registry_variants.add(compact_registry[: -len(suffix)])
    if compact_simple:
        if compact_registry == compact_simple or (compact_registry.endswith(compact_simple) and compact_registry != compact_simple):
            return stripped
        if any(value.endswith(compact_simple) and value != compact_simple for value in registry_variants):
            return stripped
    return _pascal(registry)
def _float_constants(text: str, method_names: tuple[str, ...]) -> list[float]:
    values: list[float] = []
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if not any(name in line for name in method_names):
            continue
        chunk = "\n".join(lines[max(0, i - 16):i + 12])
        for match in re.finditer(r"(?m)^\s*\d+:\s+(?:ldc|ldc_w|fconst_[0-9]+|dconst_[01]|bipush|sipush)\s+(-?[0-9]+(?:\.[0-9]+)?)(?:[fFdD])?", chunk):
            try:
                values.append(float(match.group(1)))
            except Exception:
                pass
    return values
def _extract_food_info(text: str, superclass: str, constructor_records: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    """Recover both legacy and clean Mojmap FoodProperties definitions.

    Clean Chaos Persists classes commonly receive nutrition/saturation/alwaysEat as
    constructor parameters and pass them through a helper method.  The old extractor
    only recognized ItemFood/ItemSeedFood, so virtually all modernized foods became
    plain items.
    """
    lower = superclass.lower()
    info: dict[str, Any] = {}
    records = constructor_records or []
    record = records[0] if records else {}
    raw_args = list(record.get("args", []))
    args = [str(v).strip() for v in raw_args]
    primitive = [v for v in raw_args if isinstance(v, (int, float, bool))]
    if "itemfood" in lower or "itemseedfood" in lower:
        info["isFood"] = True
    if "itemfood" in lower and len(primitive) >= 3:
        info["nutritionalValue"] = int(primitive[-3]); info["saturation"] = float(primitive[-2]); info["isMeat"] = bool(primitive[-1])
    elif "itemseedfood" in lower and len(primitive) >= 2:
        info["nutritionalValue"] = int(primitive[-2]); info["saturation"] = float(primitive[-1])

    modern_food = bool(re.search(r"\.food\s*\(\s*new\s+FoodProperties\.Builder|FoodProperties\.Builder", text))
    if modern_food:
        info["isFood"] = True
        # Determine constructor parameter names and map registration literals to them.
        ctor_params: list[str] = []
        cls_match = re.search(r"\bpublic\s+([A-Za-z_$][\w$]*)\s*\(([^)]*)\)", text)
        if cls_match:
            for param in _split_java_arguments(cls_match.group(2)):
                m = re.search(r"([A-Za-z_$][\w$]*)\s*$", param.strip())
                if m: ctor_params.append(m.group(1))
        literal_by_param: dict[str, Any] = {}
        for i, name in enumerate(ctor_params):
            if i < len(args):
                token = args[i] 
                m = re.fullmatch(r"(-?\d+(?:\.\d+)?)(?:[fFdD])?", token)
                if m:
                    value=float(m.group(1)); literal_by_param[name]=int(value) if value.is_integer() else value
                elif token.lower() in {"true","false"}: literal_by_param[name]=token.lower()=="true"
        # Walk the source for exact parameter uses; this also works through private helpers.
        nm = re.search(r"\.nutrition\s*\(\s*([A-Za-z_$][\w$]*)\s*\)", text)
        sm = re.search(r"\.saturationMod\s*\(\s*([A-Za-z_$][\w$]*)\s*\)", text)
        if nm:
            key=nm.group(1)
            if key in literal_by_param: info["nutritionalValue"]=int(literal_by_param[key])
        if sm:
            key=sm.group(1)
            if key in literal_by_param: info["saturation"]=float(literal_by_param[key])
        # The bool parameter controls conditional alwaysEat calls in GenericFish/etc.
        am = re.search(r"if\s*\(\s*([A-Za-z_$][\w$]*)\s*\)\s*\{[^{}]*?\.alwaysEat\s*\(\s*\)", text, re.DOTALL)
        if am and am.group(1) in literal_by_param: info["isAlwaysEdible"]=bool(literal_by_param[am.group(1)])
        elif re.search(r"\.alwaysEat\s*\(\s*\)", text): info["isAlwaysEdible"]=True
        if re.search(r"\.meat\s*\(\s*\)", text): info["isMeat"]=True
    if "func_77848_i" in text or "setAlwaysEdible" in text:
        info["isAlwaysEdible"] = True
    return info
def _class_info(source: Path) -> tuple[str, str, str]:
    text = _run_javap(source)
    header = next((line.strip() for line in text.splitlines() if line.strip().startswith(("public ", "protected ", "private ", "class ", "abstract "))), "")
    match = re.search(r"\bclass\s+([A-Za-z0-9_.$]+)(?:\s+extends\s+([A-Za-z0-9_.$]+))?", header)
    simple = _simple(match.group(1)) if match else _simple(source.stem)
    superclass = _simple(match.group(2)) if match and match.group(2) else ""
    return simple, superclass, text
def _value_constants(text: str) -> list[float]:
    values: list[float] = []
    for match in re.finditer(r"(?m)^\s*(?:[0-9]+:\s+)?(?:bipush|sipush|iconst_[0-5]|iconst_m1|fconst_[0-5]|dconst_[0-1]|ldc2_w|ldc)\s+([^\n]+)", text):
        raw = match.group(1).strip()
        raw = re.sub(r"\s+//.*$", "", raw).strip()
        try:
            values.append(float(raw.rstrip("fFdD")))
        except Exception:
            continue
    return values
def _find_int_after(text: str, method_names: tuple[str, ...]) -> Optional[int]:
    # Decompiled/source form: the argument is before the call name, unlike javap
    # bytecode where the pushed constant precedes the invoke instruction.
    if not re.search(r"(?m)^\s*\d+:\s+", text):
        for name in method_names:
            match = re.search(rf"\b{re.escape(name)}\s*\(\s*(-?\d+)\s*\)", text)
            if match:
                return int(match.group(1))
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if any(name in line for name in method_names):
            # Invocation arguments are pushed immediately *before* the invoke.
            chunk = "\n".join(lines[max(0, i - 12) : i + 1])
            vals = _value_constants(chunk)
            for value in reversed(vals):
                if abs(value - int(value)) < 1e-8:
                    return int(value)
    return None
def _extract_field_int(text: str, field_names: tuple[str, ...]) -> Optional[int]:
    if not re.search(r"(?m)^\s*\d+:\s+", text):
        for field in field_names:
            matches = list(re.finditer(rf"\b(?:this\.)?{re.escape(field)}\s*=\s*(-?\d+(?:\.\d+)?)\s*[fFdD]?", text))
            if matches:
                return int(float(matches[-1].group(1)))
    # Conservative fallback: look for a putfield preceded shortly by a numeric constant.
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if "putfield" in line and any(name in line for name in field_names):
            chunk = "\n".join(lines[max(0, i - 6):i + 1])
            vals = _value_constants(chunk)
            if vals:
                return int(vals[-1])
    return None
def _extract_method_return_number(text: str, method_names: tuple[str, ...]) -> Optional[float]:
    """Recover a constant returned by one specifically named method.
    This deliberately never scans unrelated methods; old code commonly puts
    7,200-tick use durations beside a 12-damage weapon method.
    """
    if not re.search(r"(?m)^\s*\d+:\s+", text):
        for name in method_names:
            match = re.search(
                rf"\b(?:public|protected|private)?\s*(?:final\s+)?(?:int|float|double)\s+{re.escape(name)}\s*\([^)]*\)\s*\{{(.*?)\}}",
                text,
                re.DOTALL,
            )
            if not match:
                continue
            returned = re.search(r"\breturn\s+(-?\d+(?:\.\d+)?)\s*[fFdD]?\s*;", match.group(1))
            if returned:
                return float(returned.group(1))
        return None
    methods = _parse_methods(text)
    for method in methods:
        name_match = re.search(r"([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*;$", method.header)
        method_name = name_match.group(1) if name_match else ""
        if method_name not in method_names:
            continue
        instruction_text = "\n".join(
            f"{offset}: {op}" + (f" // {comment}" if comment else "")
            for offset, op, comment in method.instructions
        )
        values = _value_constants(instruction_text)
        if values:
            return float(values[-1])
    return None
def _recover_tool_stats(
    text: str,
    tool_type: str,
    material_stats: Optional[dict[str, float]] = None,
) -> tuple[int, float, float, int, list[str]]:
    warnings: list[str] = []
    durability = _find_int_after(text, ("func_77656_e", "setMaxDamage"))
    # Clean 1.20-style constructors commonly inline durability in new Properties().durability(...).
    # Prefer this explicit class-authored value over a material's maxuses when both exist.
    if durability is None:
        ctor = re.search(r"\.durability\s*\(\s*(-?\d+)\s*\)", text)
        if ctor:
            durability = int(ctor.group(1))
    damage_value = _extract_field_int(text, ("weaponDamage", "WEAPON_DAMAGE", "attackDamage", "ATTACK_DAMAGE", "damageVsEntity", "DAMAGE", "damage"))
    if damage_value is None:
        returned = _extract_method_return_number(text, ("getDamageVsEntity", "getAttackDamage", "func_150931_i"))
        damage_value = int(returned) if returned is not None else None
    # Material-backed vanilla tools often do not store an explicit value in the
    # subclass. Preserve a conservative vanilla-equivalent baseline rather than
    # misreading durability/use-duration constants as thousands of damage.
    default_damage = {
        "Sword": 4,
        "Axe": 5,
        "Pickaxe": 3,
        "Shovel": 2,
        "Hoe": 1,
        "Bow": 2,
    }.get(tool_type, 3)
    efficiency = 4.0
    enchantability = 10
    if material_stats:
        if durability is None and material_stats.get("uses") is not None:
            durability = int(material_stats["uses"])
        efficiency = float(material_stats.get("efficiency", efficiency))
        # A class-level getEnchantmentValue() is authoritative for custom items/bows/rods.
        ench_method = _extract_method_return_number(text, ("getEnchantmentValue", "getItemEnchantability"))
        enchantability = int(ench_method) if ench_method is not None else int(material_stats.get("enchantability", enchantability))
        if damage_value is None and material_stats.get("damage") is not None:
            offsets = {"Sword": 4.0, "Axe": 3.0, "Pickaxe": 2.0, "Shovel": 1.0, "Hoe": 0.0}
            damage_value = float(material_stats["damage"]) + offsets.get(tool_type, 0.0)
    else:
        ench_method = _extract_method_return_number(text, ("getEnchantmentValue", "getItemEnchantability"))
        if ench_method is not None:
            enchantability = int(ench_method)
    if damage_value is None:
        damage_value = default_damage
        warnings.append(f"Tool attack damage was not explicit; used a {tool_type.lower()} baseline of {default_damage} instead of guessing from durability/use duration.")
    if durability is None:
        durability = 250
        warnings.append("Tool durability could not be recovered exactly; defaulted to 250 uses.")
    return max(1, int(durability)), max(0.0, float(damage_value)), max(0.0, efficiency), max(0, enchantability), warnings
def _source_tool_materials(roots: list[Path]) -> dict[str, dict[str, float]]:
    """Recover registry -> ToolMaterial stats from Java initializers."""
    field_registries: dict[str, set[str]] = {}
    texts: list[str] = []
    assignments: list[tuple[str, str, str]] = []
    for root in roots:
        for field, values in _source_field_registered_names(root).items():
            field_registries.setdefault(field, set()).update(values)
        for path in root.rglob("*.java"):
            try:
                text = path.read_text(encoding="utf-8-sig", errors="replace")
            except OSError:
                continue
            texts.append(text)
            assignments.extend(_source_constructor_assignments(text))
    combined = "\n".join(texts)
    stat_objects: dict[str, dict[str, float]] = {}
    for match in re.finditer(r"\b([A-Za-z_$][\w$]*)\s*=\s*(?:this\.)?(?:get|load)_[A-Za-z0-9_]*stats\s*\((.*?)\)\s*;", combined, re.DOTALL):
        args = _split_java_arguments(match.group(2))
        numbers: list[float] = []
        for arg in args:
            numeric = re.fullmatch(r"\s*(-?\d+(?:\.\d+)?)\s*[fFdD]?\s*", arg)
            if numeric:
                numbers.append(float(numeric.group(1)))
        if len(numbers) >= 5:
            harvest, uses, efficiency, damage, enchantability = numbers[-5:]
            stat_objects[match.group(1)] = {
                "harvest": harvest,
                "uses": uses,
                "efficiency": efficiency,
                "damage": damage,
                "enchantability": enchantability,
            }
    material_fields: dict[str, dict[str, float]] = {}
    for match in re.finditer(r"\b([A-Za-z_$][\w$]*)\s*=\s*(?:EnumHelper\.)?addToolMaterial\s*\((.*?)\)\s*;", combined, re.DOTALL):
        field_name, raw_args = match.groups()
        args = _split_java_arguments(raw_args)
        joined = " ".join(args)
        ref = re.search(r"\b([A-Za-z_$][\w$]*)\.(?:harvestlevel|maxuses|efficiency|damage|enchantability)\b", joined, re.IGNORECASE)
        if ref and ref.group(1) in stat_objects:
            material_fields[field_name] = dict(stat_objects[ref.group(1)])
            continue
        numeric: list[float] = []
        for arg in args[1:]:
            value = re.fullmatch(r"\s*(?:\(float\)\s*)?(-?\d+(?:\.\d+)?)\s*[fFdD]?\s*", arg)
            if value:
                numeric.append(float(value.group(1)))
        if len(numeric) >= 5:
            harvest, uses, efficiency, damage, enchantability = numeric[-5:]
            material_fields[field_name] = {
                "harvest": harvest,
                "uses": uses,
                "efficiency": efficiency,
                "damage": damage,
                "enchantability": enchantability,
            }
    result: dict[str, dict[str, float]] = {}
    # Clean source frequently registers items inline as
    # ITEMS.register("experiencesword", () -> new ExperienceSword(toolEMERALD)).
    # Those registrations do not create an assignment that the older field parser
    # can see, so resolve the constructor material argument directly.
    for match in re.finditer(
        r"\b(?:ITEMS|itemRegistry|registerItems?|REGISTRY)\s*\.register\s*\(\s*[\"']([^\"']+)[\"']\s*,\s*\(\s*\)\s*->\s*new\s+[A-Za-z_$][\w$]*\s*\(([^)]*)\)",
        combined,
        re.DOTALL,
    ):
        registry, raw_args = match.groups()
        tokens = re.findall(r"\b[A-Za-z_$][\w$]*\b", raw_args)
        material = next((material_fields[token] for token in tokens if token in material_fields), None)
        if material and _looks_like_registry_path(_registry_path(registry)):
            result[_compact_registry(registry)] = dict(material)
    # First use explicit constructor/material field references when available.
    for field_name, _class_name, raw_args in assignments:
        tokens = set(re.findall(r"\b[A-Za-z_$][\w$]*\b", raw_args))
        material = next((material_fields[token] for token in tokens if token in material_fields), None)
        if material:
            for registry in field_registries.get(field_name, set()):
                result[_compact_registry(registry)] = dict(material)
    # Clean-source fallback: registry field names themselves encode the material
    # (MyUltimateSword -> ultimate_stats, MyCrystalPinkPickaxe -> crystalpink_stats).
    normalized_stats = {re.sub(r"[^a-z0-9]", "", k.removesuffix("_stats").lower()): v for k,v in stat_objects.items()}
    for field_name, registries in field_registries.items():
        nf = re.sub(r"[^a-z0-9]", "", field_name.lower())
        candidates=[]
        for key,val in normalized_stats.items():
            if key and (key in nf or nf.replace("my", "").startswith(key)):
                candidates.append((len(key),val))
        if not candidates: continue
        material=max(candidates,key=lambda x:x[0])[1]
        for registry in registries:
            result[_compact_registry(registry)] = dict(material)
    return result
def _bytecode_equipment_stats(
    registry_texts: dict[Path, str],
) -> tuple[dict[str, dict[str, float]], dict[str, dict[str, float]], dict[str, set[str]]]:
    """Recover tool/armor material tables from compiled registry bytecode.
    This mirrors :func:`_source_tool_materials` and :func:`_source_armor_stats`
    for compiled-only legacy mods.  It follows generic helper/material calls
    and never relies on a particular mod, field name, or registry namespace.
    """
    tool_results: dict[str, dict[str, float]] = {}
    armor_results: dict[str, dict[str, float]] = {}
    registered_names: dict[str, set[str]] = {}
    def field_name(comment: str) -> str:
        match = re.search(r"Field\s+(.+?):", comment)
        if not match:
            return ""
        return match.group(1).replace("/", ".").split(".")[-1].split("$")[0]
    def next_putstatic(instructions: list[tuple[int, str, str]], start: int, limit: int = 12) -> str:
        for _offset, operation, comment in instructions[start + 1 : start + 1 + limit]:
            if operation.split()[0] == "putstatic":
                return field_name(comment)
        return ""
    def previous_constants(
        instructions: list[tuple[int, str, str]],
        index: int,
        count: int,
        limit: int = 80,
    ) -> tuple[list[float], str]:
        numeric: list[float] = []
        label = ""
        for _offset, operation, comment in reversed(instructions[max(0, index - limit) : index]):
            base = operation.split()[0]
            if base.startswith(("ldc", "bipush", "sipush", "iconst_", "fconst_", "dconst_")):
                value = _push_constant(operation, comment)
                if isinstance(value, str):
                    if not label:
                        label = value
                    if len(numeric) >= count:
                        break
                elif isinstance(value, (int, float)) and len(numeric) < count:
                    numeric.append(float(value))
            if len(numeric) >= count and label:
                break
        numeric.reverse()
        return numeric, label
    for javap_text in registry_texts.values():
        parsed_methods = _parse_methods(javap_text)
        class_item_fields: dict[str, tuple[str, Optional[dict[str, float]]]] = {}
        for method in parsed_methods:
            instructions = method.instructions
            tool_stat_fields: dict[str, dict[str, float]] = {}
            armor_stat_fields: dict[str, dict[str, float]] = {}
            tool_material_fields: dict[str, dict[str, float]] = {}
            armor_material_fields: dict[str, dict[str, float]] = {}
            # First pass: helper-produced stat objects.
            for index, (_offset, operation, comment) in enumerate(instructions):
                if not operation.startswith("invoke"):
                    continue
                info = _method_info(comment)
                method_name = info[1] if info else ""
                if re.search(r"(?i)(?:get|load)_?weaponstats$", method_name):
                    values, label = previous_constants(instructions, index, 5)
                    target = next_putstatic(instructions, index)
                    if len(values) == 5 and target:
                        harvest, uses, efficiency, damage, enchantability = values
                        stats = {
                            "harvest": harvest,
                            "uses": uses,
                            "efficiency": efficiency,
                            "damage": damage,
                            "enchantability": enchantability,
                        }
                        tool_stat_fields[target] = stats
                        if label:
                            tool_results.setdefault(_compact_registry(label), dict(stats))
                elif re.search(r"(?i)(?:get|load)_?armorstats$", method_name):
                    values, label = previous_constants(instructions, index, 14)
                    target = next_putstatic(instructions, index)
                    if len(values) >= 6 and target:
                        durability, helmet, chest, leggings, boots, enchantability = values[:6]
                        stats = {
                            "durability": durability,
                            "helmet": helmet,
                            "body": chest,
                            "leggings": leggings,
                            "boots": boots,
                            "enchantability": enchantability,
                        }
                        armor_stat_fields[target] = stats
                        for key in (label, target, re.sub(r"(?i)_?armorstats$", "", target)):
                            compact = _compact_registry(key)
                            if compact:
                                armor_results.setdefault(compact, dict(stats))
            # Second pass: ToolMaterial/ArmorMaterial fields referencing those
            # helper objects (or direct numeric EnumHelper calls).
            for index, (_offset, operation, comment) in enumerate(instructions):
                if not operation.startswith("invoke"):
                    continue
                info = _method_info(comment)
                method_name = info[1] if info else ""
                if method_name == "addToolMaterial":
                    target = next_putstatic(instructions, index)
                    window = instructions[max(0, index - 50) : index]
                    refs = [field_name(c) for _o, op, c in window if op.split()[0] == "getstatic"]
                    stats = next((tool_stat_fields[name] for name in reversed(refs) if name in tool_stat_fields), None)
                    if stats is None:
                        values, _label = previous_constants(instructions, index, 5)
                        if len(values) == 5:
                            stats = dict(zip(("harvest", "uses", "efficiency", "damage", "enchantability"), values))
                    if target and stats:
                        tool_material_fields[target] = dict(stats)
                        tool_results.setdefault(_compact_registry(target), dict(stats))
                elif method_name == "addArmorMaterial":
                    target = next_putstatic(instructions, index)
                    window = instructions[max(0, index - 60) : index]
                    refs = [field_name(c) for _o, op, c in window if op.split()[0] == "getstatic"]
                    stats = next((armor_stat_fields[name] for name in reversed(refs) if name in armor_stat_fields), None)
                    if target and stats:
                        armor_material_fields[target] = dict(stats)
                        armor_results.setdefault(_compact_registry(target), dict(stats))
            # Third pass: map static content fields to their concrete class and
            # material, then follow later GameRegistry calls to the authoritative
            # registry key. Legacy mods often set an unrelated unlocalized icon
            # name during construction and register the item hundreds of bytecode
            # instructions later, so a per-`new` window alone is insufficient.
            item_fields: dict[str, tuple[str, Optional[dict[str, float]]]] = {}
            for index, (_offset, operation, comment) in enumerate(instructions):
                if operation.split()[0] != "putstatic":
                    continue
                target = field_name(comment)
                if not target or target in tool_material_fields or target in armor_material_fields:
                    continue
                start = max(0, index - 160)
                new_index = None
                for probe in range(index - 1, start - 1, -1):
                    if instructions[probe][1].split()[0] == "new" and "class " in instructions[probe][2]:
                        new_index = probe
                        break
                    if instructions[probe][1].split()[0] == "putstatic":
                        break
                if new_index is None:
                    continue
                class_name = _simple(_class_from_comment(instructions[new_index][2]))
                refs = [
                    field_name(c)
                    for _o, op, c in instructions[new_index:index]
                    if op.split()[0] == "getstatic"
                ]
                material_stats = next(
                    (tool_material_fields[name] for name in reversed(refs) if name in tool_material_fields),
                    None,
                )
                item_fields[target] = (class_name, material_stats)
                class_item_fields[target] = (class_name, material_stats)
                if material_stats and class_name:
                    tool_results.setdefault(_compact_registry(class_name), dict(material_stats))
            for index, (_offset, operation, comment) in enumerate(instructions):
                if not operation.startswith("invoke"):
                    continue
                info = _method_info(comment)
                called = info[1] if info else ""
                if called not in {"registerItem", "registerBlock", "register"}:
                    continue
                source_field = ""
                registry_name = ""
                for _o, op, prior_comment in reversed(instructions[max(0, index - 16):index]):
                    base = op.split()[0]
                    if not source_field and base == "getstatic":
                        candidate = field_name(prior_comment)
                        if candidate in class_item_fields:
                            source_field = candidate
                    if not registry_name and base.startswith("ldc"):
                        value = _push_constant(op, prior_comment)
                        if isinstance(value, str):
                            registry_name = value
                    if source_field and registry_name:
                        break
                if not source_field or not registry_name:
                    continue
                class_name, material_stats = class_item_fields[source_field]
                registry_path = _registry_path(registry_name)
                if class_name and registry_path:
                    registered_names.setdefault(class_name, set()).add(registry_path)
                if material_stats and registry_path:
                    tool_results[_compact_registry(registry_path)] = dict(material_stats)
            # Preserve unlocalized-name mappings as a fallback when a compiled
            # corpus genuinely has no explicit registry call.
            new_indices = [i for i, (_o, op, comment) in enumerate(instructions) if op.split()[0] == "new" and "class " in comment]
            for position, start in enumerate(new_indices):
                end = new_indices[position + 1] if position + 1 < len(new_indices) else min(len(instructions), start + 140)
                chunk = instructions[start:end]
                material_stats: Optional[dict[str, float]] = None
                registry_name = ""
                strings: list[str] = []
                class_name = _simple(_class_from_comment(chunk[0][2])) if chunk else ""
                for _off, op, prior_comment in chunk:
                    base = op.split()[0]
                    if base == "getstatic":
                        ref = field_name(prior_comment)
                        if ref in tool_material_fields:
                            material_stats = tool_material_fields[ref]
                    if base.startswith("ldc"):
                        value = _push_constant(op, prior_comment)
                        if isinstance(value, str):
                            strings.append(value)
                    if base.startswith("invoke"):
                        info = _method_info(prior_comment)
                        invoked = info[1] if info else ""
                        if invoked in {"func_77655_b", "setUnlocalizedName", "setRegistryName"} and strings:
                            registry_name = strings[-1]
                if material_stats:
                    if registry_name:
                        tool_results.setdefault(_compact_registry(_registry_path(registry_name)), dict(material_stats))
                    if class_name:
                        tool_results.setdefault(_compact_registry(class_name), dict(material_stats))
        # Registration commonly lives in a different lifecycle method than
        # construction. Re-run only the registration lookup after every method
        # has contributed its static item-field mapping.
        for method in parsed_methods:
            instructions = method.instructions
            for index, (_offset, operation, comment) in enumerate(instructions):
                if not operation.startswith("invoke"):
                    continue
                info = _method_info(comment)
                called = info[1] if info else ""
                if called not in {"registerItem", "registerBlock", "register"}:
                    continue
                source_field = ""
                registry_name = ""
                for _o, op, prior_comment in reversed(instructions[max(0, index - 20):index]):
                    base = op.split()[0]
                    if not source_field and base == "getstatic":
                        candidate = field_name(prior_comment)
                        if candidate in class_item_fields:
                            source_field = candidate
                    if not registry_name and base.startswith("ldc"):
                        value = _push_constant(op, prior_comment)
                        if isinstance(value, str):
                            registry_name = value
                    if source_field and registry_name:
                        break
                if not source_field or not registry_name:
                    continue
                class_name, material_stats = class_item_fields[source_field]
                registry_path = _registry_path(registry_name)
                if class_name and registry_path:
                    registered_names.setdefault(class_name, set()).add(registry_path)
                if material_stats and registry_path:
                    tool_results[_compact_registry(registry_path)] = dict(material_stats)
    return tool_results, armor_results, registered_names
def _source_armor_stats(roots: list[Path]) -> dict[str, dict[str, float]]:
    """Recover armor durability/defense/enchantability from source stat tables."""
    result: dict[str, dict[str, float]] = {}
    for root in roots:
        for path in root.rglob("*.java"):
            try:
                text = path.read_text(encoding="utf-8-sig", errors="replace")
            except OSError:
                continue
            for match in re.finditer(
                r"\b([A-Za-z_$][\w$]*)\s*=\s*(?:this\.)?(?:get|load)_armorstats\s*\((.*?)\)\s*;",
                text,
                re.DOTALL | re.IGNORECASE,
            ):
                field_name, raw_args = match.groups()
                args = _split_java_arguments(raw_args)
                label = ""
                for arg in args:
                    quoted = re.fullmatch(r"\s*[\"']([^\"']+)[\"']\s*", arg)
                    if quoted:
                        label = quoted.group(1)
                numbers: list[float] = []
                for arg in args:
                    numeric = re.fullmatch(r"\s*(-?\d+(?:\.\d+)?)\s*[fFdD]?\s*", arg)
                    if numeric:
                        numbers.append(float(numeric.group(1)))
                if len(numbers) < 6:
                    continue
                durability, helmet, chest, leggings, boots, enchantability = numbers[:6]
                stats = {
                    "durability": durability,
                    "helmet": helmet,
                    "body": chest,
                    "leggings": leggings,
                    "boots": boots,
                    "enchantability": enchantability,
                }
                # get_armorstats(..., e_resp, e_aqua, e_prot, e_fire, e_blast,
                # e_proj, e_unbreak, e_feather) is the source of the enchantments
                # applied by ItemChaosArmor.onCraftedBy.
                if len(numbers) >= 14:
                    for key, value in zip((
                        "respiration", "aqua_affinity", "protection",
                        "fire_protection", "blast_protection",
                        "projectile_protection", "unbreaking", "feather_falling"
                    ), numbers[6:14]):
                        if int(value) != 0:
                            stats["enchant_" + key] = value
                for key in (label, field_name, re.sub(r"(?i)_?armorstats$", "", field_name)):
                    compact = _compact_registry(key)
                    if compact:
                        result[compact] = dict(stats)
    return result
def _classify_tool(superclass: str, simple: str) -> Optional[str]:
    """Classify a legacy Item* tool/weapon without confusing models/renderers."""
    super_lower = superclass.lower()
    simple_lower = simple.lower()
    # Prefer the actual superclass. This is the reliable signal for legacy Forge.
    superclass_tokens = (
        ("itemaxe", "Axe"),
        ("itemsword", "Sword"),
        ("itempickaxe", "Pickaxe"),
        ("itemspade", "Shovel"),
        ("itemshovel", "Shovel"),
        ("itemhoe", "Hoe"),
        ("itemshears", "Shears"),
        ("itemfishingrod", "Fishing Rod"),
        ("itembow", "Bow"),
    )
    for token, result in superclass_tokens:
        if token in super_lower:
            return result
    # Only use the class name as a fallback when this is clearly an Item-style class.
    if super_lower not in {"item", "", "java.lang.object"} and "item" not in super_lower:
        return None
    suffixes = (
        ("pickaxe", "Pickaxe"),
        ("fishingrod", "Fishing Rod"),
        ("shears", "Shears"),
        ("shovel", "Shovel"),
        ("spade", "Shovel"),
        ("sword", "Sword"),
        ("axe", "Axe"),
        ("hoe", "Hoe"),
        ("bow", "Bow"),
    )
    for suffix, result in suffixes:
        if simple_lower.endswith(suffix):
            return result
    return None
def _classify(source: Path) -> tuple[str, str, str]:
    if source.suffix.lower() == ".java":
        simple, superclass, _flags, text = _source_class_header_details(source)
        kind, actual_simple, _ = _classify_header(superclass, simple)
        return kind, actual_simple, text if kind != "unknown" else ""
    simple, superclass = _class_header_info(source)
    kind, actual_simple, _ = _classify_header(superclass, simple)
    return kind, actual_simple, _run_javap(source) if kind != "unknown" else ""
def _normalise_texture_stem(value: str) -> str:
    value = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", value)
    return re.sub(r"[^a-z0-9]+", "_", value.lower()).strip("_")
def _png_refs_from_javap(text: str) -> list[str]:
    refs: list[str] = []
    for match in re.finditer(r'(?i)"([^"\n]+\.png)"', text):
        value = match.group(1).replace("\\", "/")
        if value not in refs:
            refs.append(value)
    return refs
def _declared_texture_refs(text: str) -> list[str]:
    """Recover explicit item/block icon names from source or decompiled text."""
    refs = _png_refs_from_javap(text)
    patterns = (
        r"\b(?:setTextureName|func_111206_d)\s*\(\s*[\"']([^\"']+)[\"']\s*\)",
        r"\b(?:registerIcon|func_94245_a)\s*\(\s*[\"']([^\"']+)[\"']\s*\)",
        r"\bnew\s+ResourceLocation\s*\(\s*[\"']([^\"']+)[\"'](?:\s*,\s*[\"']([^\"']+)[\"'])?",
    )
    for pattern in patterns:
        for match in re.finditer(pattern, text):
            value = (match.group(2) if match.lastindex and match.lastindex >= 2 and match.group(2) else match.group(1)).replace("\\", "/")
            value = value.split(":", 1)[-1]
            value = value.rsplit("/", 1)[-1]
            value = re.sub(r"\.png$", "", value, flags=re.IGNORECASE)
            if value and value not in refs:
                refs.append(value)
    return refs
def _extract_source_constructor_records(source_text: str) -> dict[str, list[dict[str, Any]]]:
    """Recover literal constructor arguments from clean Java registry source."""
    result: dict[str,list[dict[str,Any]]] = {}
    for m in re.finditer(r'\bnew\s+([A-Za-z_$][\w$]*)\s*\(', source_text):
        cls=m.group(1)
        start=m.end(); depth=1; i=start; in_str=None; esc=False
        while i < len(source_text) and depth:
            ch=source_text[i]
            if in_str:
                if esc: esc=False
                elif ch=='\\\\': esc=True
                elif ch==in_str: in_str=None
            else:
                if ch in '\"\\\'': in_str=ch
                elif ch=='(': depth+=1
                elif ch==')': depth-=1
            i+=1
        if depth: continue
        raw=source_text[start:i-1]
        args=_split_java_arguments(raw)
        result.setdefault(cls,[]).append({"args": args, "desc": "source"})
    return result

def _extract_constructor_records(javap_text: str) -> dict[str, list[dict[str, Any]]]:
    records: dict[str, list[dict[str, Any]]] = {}
    lines = javap_text.splitlines()
    for start in [i for i, line in enumerate(lines) if " new " in line and "// class " in line]:
        class_match = re.search(r"// class ([A-Za-z0-9_/$]+)", lines[start])
        if not class_match:
            continue
        simple = _simple(class_match.group(1))
        stack: list[Any] = []
        for line in lines[start : min(len(lines), start + 120)]:
            op_match = re.match(r"\s*\d+:\s+([^ ]+(?:\s+[^ ]+)*)", line)
            if not op_match:
                continue
            op = op_match.group(1)
            base = op.split()[0]
            comment = line.split("//", 1)[1].strip() if "//" in line else ""
            if base == "new":
                stack.append(ConstructedObject(_class_from_comment(comment)))
                continue
            if base == "dup":
                if stack:
                    stack.append(stack[-1])
                continue
            if base.startswith(("ldc", "bipush", "sipush", "iconst_", "fconst_", "dconst_")):
                value = _push_constant(base, comment)
                stack.append(value)
                continue
            if base == "getstatic":
                field_match = re.search(r"Field\s+.+?([A-Za-z0-9_$]+):", comment)
                stack.append(("field", field_match.group(1)) if field_match else None)
                continue
            if not base.startswith("invoke"):
                if base in {"iadd", "isub", "imul", "fadd", "fsub", "fmul", "dadd", "dsub", "dmul"} and len(stack) >= 2:
                    b, a = stack.pop(), stack.pop()
                    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
                        result = {"iadd": a+b, "fadd": a+b, "dadd": a+b, "isub": a-b, "fsub": a-b, "dsub": a-b, "imul": a*b, "fmul": a*b, "dmul": a*b}.get(base)
                        stack.append(result)
                    else:
                        stack.append(None)
                continue
            info = _method_info(comment)
            if not info:
                continue
            owner, name, desc = info
            argc = len(_descriptor_args(desc))
            args = [stack.pop() if stack else None for _ in range(argc)][::-1]
            if name == "<init>":
                receiver = stack.pop() if stack else None
                instance = ConstructedObject(owner.replace("/", "."), args)
                if stack and isinstance(stack[-1], ConstructedObject) and _simple(stack[-1].class_name) == _simple(owner):
                    stack[-1] = instance
                else:
                    stack.append(instance)
                records.setdefault(_simple(owner), []).append({"args": args, "desc": desc})
                continue
            if name in {"func_77655_b", "setUnlocalizedName"} and args:
                receiver = stack.pop() if stack and not isinstance(stack[-1], ConstructedObject) else (stack.pop() if stack else None)
                stack.append(receiver)
                continue
    return records
def _extract_registered_names(javap_text: str) -> dict[str, set[str]]:
    """Recover repeated legacy 1.7-1.12 registry names from constructor/registration bytecode."""
    result: dict[str, set[str]] = {}
    lines = javap_text.splitlines()
    new_indices = [i for i, line in enumerate(lines) if " new " in line and "// class " in line]
    for pos, start in enumerate(new_indices):
        end = new_indices[pos + 1] if pos + 1 < len(new_indices) else min(len(lines), start + 140)
        class_match = re.search(r"// class ([A-Za-z0-9_/$]+)", lines[start])
        if not class_match:
            continue
        simple = class_match.group(1).replace("/", ".").split(".")[-1].split("$")[0]
        chunk = lines[start:end]
        strings: list[tuple[int, str]] = []
        for j, line in enumerate(chunk):
            match = re.search(r"// String (.+)$", line.strip())
            if match:
                value = match.group(1).strip()
                # Namespace-only literals (for example ``mymod:``) and display-name
                # constants are not registry paths. Decompiled source registrations
                # remain authoritative when present, but compiled-only mods still
                # need a generic filter that does not name any particular mod.
                if value and not re.fullmatch(r"[a-z0-9_.-]+:", value.lower()):
                    strings.append((j, value))
            # The last string before a registry/name assignment is normally the actual key.
            if any(token in line for token in (
                "func_77655_b", "func_149663_c", "setRegistryName",
                "GameRegistry.registerItem", "GameRegistry.registerBlock",
                "registerItem", "registerBlock",
            )):
                prior = [value for j, value in strings if j <= len(chunk)]
                if prior:
                    candidate = prior[-1]
                    if 1 <= len(candidate) <= 96 and not candidate.replace("_", "").isdigit():
                        result.setdefault(simple, set()).add(candidate)
    return result
_SOURCE_REGISTRY_CACHE: dict[str, dict[str, set[str]]] = {}
_SOURCE_FIELD_REGISTRY_CACHE: dict[str, dict[str, set[str]]] = {}
_SOURCE_WORLDGEN_CACHE: dict[str, dict[str, dict[str, Any]]] = {}
_SOURCE_CREATIVE_TAB_CACHE: dict[str, dict[str, str]] = {}
_SOURCE_REGISTRY_TAB_CACHE: dict[str, dict[str, str]] = {}
_SOURCE_CUSTOM_TAB_CACHE: dict[str, dict[str, dict[str, str]]] = {}
_SOURCE_ARMOR_SET_CACHE: dict[str, dict[str, str]] = {}
_SOURCE_DIMENSION_BIOME_CACHE: dict[str, dict[str, str]] = {}
_LANGUAGE_NAME_CACHE: dict[str, dict[str, str]] = {}
_LANGUAGE_DESCRIPTION_CACHE: dict[str, dict[str, list[str]]] = {}
def _registry_path(value: str) -> str:
    value = str(value).strip().strip('"\'')
    if ":" in value:
        value = value.split(":", 1)[1]
    return value.strip()
def _looks_like_registry_path(value: str) -> bool:
    return bool(
        value
        and 1 <= len(value) <= 128
        and re.fullmatch(r"[A-Za-z0-9_.\-/]+", value)
        and not value.replace("_", "").isdigit()
        and "%" not in value
    )
def _source_constructor_assignments(text: str) -> list[tuple[str, str, str]]:
    """Return ``(field, class, constructor-args)`` assignments from Java source.
    Decompiled legacy source often inserts casts and grouping parentheses before
    ``new``.  Keeping this parser in one place makes field-level and class-level
    registry recovery agree with each other.
    """
    constructor_prefix = (
        r"\s*(?:\([A-Za-z_$][\w.$<>?, \[\]]*\)\s*)*"
        r"\(*\s*new\s+"
    )
    patterns = (
        re.compile(
            r"\b([A-Za-z_$][\w$]*)\s*=" + constructor_prefix
            + r"([A-Za-z_$][\w.$]*)\s*\(([^;]{0,1600})",
            re.DOTALL,
        ),
        re.compile(
            r"\b(?:public|protected|private|static|final|volatile|transient|\s)+"
            r"[A-Za-z_$][\w.$<>?, \[\]]*\s+([A-Za-z_$][\w$]*)\s*="
            + constructor_prefix
            + r"([A-Za-z_$][\w.$]*)\s*\(([^;]{0,1600})",
            re.DOTALL,
        ),
    )
    found: dict[tuple[str, str], tuple[str, str, str]] = {}
    for pattern in patterns:
        for match in pattern.finditer(text):
            field_name, class_name, args = match.groups()
            found.setdefault((field_name, _simple(class_name)), (field_name, _simple(class_name), args))
    return list(found.values())
def _quoted_registry_argument(arguments: list[str]) -> str:
    """Return the final quoted registration argument from a Java call."""
    for argument in reversed(arguments[1:]):
        match = re.fullmatch(r"\s*[\"']([^\"']+)[\"']\s*", argument, re.DOTALL)
        if match:
            return match.group(1)
    return ""
def _source_field_registered_names(root: Path) -> dict[str, set[str]]:
    """Recover field -> authoritative registry paths from decompiled Java."""
    root = Path(root).expanduser().resolve()
    key = str(root)
    if key in _SOURCE_FIELD_REGISTRY_CACHE:
        return {name: set(values) for name, values in _SOURCE_FIELD_REGISTRY_CACHE[key].items()}
    result: dict[str, set[str]] = {}
    texts: list[str] = []
    for path in sorted(root.rglob("*.java")):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        texts.append(text)
        for field_name, _class_name, args in _source_constructor_assignments(text):
            resource = re.search(
                r"new\s+ResourceLocation\s*\(\s*[\"']([^\"']+)[\"']\s*,\s*[\"']([^\"']+)[\"']",
                args,
            )
            if resource:
                raw_name = resource.group(2)
            else:
                first_string = re.search(r"[\"']([^\"'\r\n]+)[\"']", args)
                raw_name = first_string.group(1) if first_string else ""
            clean_name = _registry_path(raw_name)
            if _looks_like_registry_path(clean_name):
                result.setdefault(field_name, set()).add(clean_name)
    combined = "\n".join(texts)
    # Modernized legacy ports often cache registries in fields via BuiltInRegistries.<type>.get(
    # ResourceLocation(...)). Treat those assignments as authoritative field -> registry mappings.
    for match in re.finditer(
        r"\b([A-Za-z_$][\w$]*)\s*=\s*(?:\([^;]+?\)\s*)?BuiltInRegistries\.(?:ITEM|BLOCK)\.get\s*\(\s*ResourceLocation\.fromNamespaceAndPath\s*\(\s*[^,]+,\s*\"([^\"]+)\"",
        combined, re.DOTALL,
    ):
        field_name, raw_name = match.groups()
        clean_name = _registry_path(raw_name)
        if _looks_like_registry_path(clean_name):
            result.setdefault(field_name, set()).add(clean_name)
    for call_name, raw_args, _offset in _iter_java_calls(combined, {"registerItem", "registerBlock"}):
        arguments = _split_java_arguments(raw_args)
        if len(arguments) < 2:
            continue
        field_match = re.fullmatch(r"\s*(?:[A-Za-z_$][\w$]*\.)*([A-Za-z_$][\w$]*)\s*", arguments[0])
        raw_name = _quoted_registry_argument(arguments)
        if not field_match or not raw_name:
            continue
        clean_name = _registry_path(raw_name)
        if _looks_like_registry_path(clean_name):
            # Explicit registration is authoritative for this field.
            result[field_match.group(1)] = {clean_name}
    _SOURCE_FIELD_REGISTRY_CACHE[key] = {name: set(values) for name, values in result.items()}
    return result
def _source_worldgen_by_registry(root: Path) -> dict[str, dict[str, Any]]:
    """Recover source-authored world-generation settings keyed by compact registry.

    The preferred source of truth is a modern ``configured_feature`` +
    ``placed_feature`` pair when the source mod already contains those resources.
    This preserves exact vein size, placement count, height range, and height shape.
    Legacy ``WorldGenMinable`` statistics remain as a fallback for older sources.
    """
    root = Path(root).expanduser().resolve()
    key = str(root)
    if key in _SOURCE_WORLDGEN_CACHE:
        return copy.deepcopy(_SOURCE_WORLDGEN_CACHE[key])
    texts: list[str] = []
    for path in sorted(root.rglob("*.java")):
        try:
            texts.append(path.read_text(encoding="utf-8-sig", errors="replace"))
        except OSError:
            continue
    combined = "\n".join(texts)
    field_names = _source_field_registered_names(root)
    stats: dict[str, dict[str, int]] = {}
    # Common Forge 1.7 pattern: get_orestats(..., defaultRate, clump, minY, maxY)
    for match in re.finditer(
        r"\b([A-Za-z_$][\w$]*_stats)\s*=\s*(?:this\.)?[A-Za-z_$][\w$]*orestats\s*\("
        r"[^;]{0,1200}?,\s*(-?\d+)\s*,\s*(-?\d+)\s*,\s*(-?\d+)\s*,\s*(-?\d+)\s*\)",
        combined,
        re.IGNORECASE | re.DOTALL,
    ):
        stats_name, rate, clump, min_y, max_y = match.groups()
        stats[stats_name] = {
            "frequencyPerChunks": max(1, int(rate)),
            "frequencyOnChunk": max(1, int(clump)),
            "minGenerateHeight": max(-64, int(min_y)),
            "maxGenerateHeight": min(320, int(max_y)),
        }
    result: dict[str, dict[str, Any]] = {}

    # Modern source packs may already carry the exact configured/placed feature
    # resources. Read them before falling back to legacy Java heuristics. MCreator's
    # native block generator cannot represent every JSON detail (notably target
    # predicates and vein size), but the recoverable placement fields should still
    # be taken verbatim instead of guessing from old statistics.
    resource_root = root / "src" / "main" / "resources" / "data"
    if resource_root.is_dir():
        for namespace_root in resource_root.iterdir():
            if not namespace_root.is_dir():
                continue
            configured_dir = namespace_root / "worldgen" / "configured_feature"
            placed_dir = namespace_root / "worldgen" / "placed_feature"
            if not configured_dir.is_dir() or not placed_dir.is_dir():
                continue
            configured: dict[str, tuple[str, int]] = {}
            for path in sorted(configured_dir.glob("*.json")):
                try:
                    payload = json.loads(path.read_text(encoding="utf-8"))
                except (OSError, ValueError, TypeError):
                    continue
                if payload.get("type") != "minecraft:ore":
                    continue
                config = payload.get("config") or {}
                targets = config.get("targets") or []
                for target in targets:
                    target = target or {}
                    # Mojang ore target entries put the output block state on the
                    # nested target object: {"target": {...}, "state": {...}}.
                    # Accept both that modern form and older flattened fixtures.
                    state_obj = target.get("state") or (target.get("target") or {}).get("state") or {}
                    state = state_obj.get("Name") if isinstance(state_obj, dict) else None
                    if isinstance(state, str) and ":" in state:
                        configured[path.stem] = (state, int(config.get("size") or 0))
                        break
            for path in sorted(placed_dir.glob("*.json")):
                try:
                    payload = json.loads(path.read_text(encoding="utf-8"))
                except (OSError, ValueError, TypeError):
                    continue
                feature_ref = payload.get("feature")
                if not isinstance(feature_ref, str):
                    continue
                feature_name = feature_ref.rsplit(":", 1)[-1]
                configured_entry = configured.get(feature_name)
                if not configured_entry:
                    continue
                state_name, vein_size = configured_entry
                # For ore blocks MCreator should replace exactly one base block.
                # Source deepslate ores replace deepslate only; all other ores replace stone only.
                state_path = state_name.split(":", 1)[-1].lower()
                replacement = "Blocks.DEEPSLATE" if "deepslate" in state_path else "Blocks.STONE"
                placement = payload.get("placement") or []
                count = 1
                min_y = -64
                max_y = 320
                shape = "UNIFORM"
                biome_restricted = False
                for placement_entry in placement:
                    if not isinstance(placement_entry, dict):
                        continue
                    ptype = placement_entry.get("type")
                    if ptype == "minecraft:count":
                        try:
                            count = max(1, int(placement_entry.get("count", 1)))
                        except (TypeError, ValueError):
                            count = 1
                    elif ptype == "minecraft:height_range":
                        height = placement_entry.get("height") or {}
                        shape_type = str(height.get("type") or "uniform").lower()
                        shape = "TRIANGLE" if shape_type == "trapezoid" else "UNIFORM"
                        lo = height.get("min_inclusive") or {}
                        hi = height.get("max_inclusive") or {}
                        try:
                            min_y = int(lo.get("absolute", min_y))
                        except (TypeError, ValueError):
                            pass
                        try:
                            max_y = int(hi.get("absolute", max_y))
                        except (TypeError, ValueError):
                            pass
                    elif ptype == "minecraft:biome":
                        biome_restricted = True
                compact_full = _compact_registry(state_name)
                compact_path = _compact_registry(state_name.rsplit(":", 1)[-1])
                if compact_path:
                    feature_data = {
                        "generateFeature": True,
                        "generationShape": shape,
                        "frequencyPerChunks": count,
                        "frequencyOnChunk": max(1, int(vein_size)),
                        "minGenerateHeight": max(-64, min_y),
                        "maxGenerateHeight": min(320, max_y),
                        "blocksToReplace": [replacement],
                        "sourceFeature": f"{namespace_root.name}:worldgen/placed_feature/{path.stem}",
                        "sourceConfiguredFeature": f"{namespace_root.name}:worldgen/configured_feature/{feature_name}",
                        "veinSize": vein_size,
                        "biomePlacement": biome_restricted,
                        "sourceResourceExact": True,
                        "approximation": (
                            f"Source placed feature is authoritative ({path.name}); MCreator retains its count/Y range. "
                            f"The source configured vein size is {vein_size}, which MCreator's block schema does not expose."
                        ),
                    }
                    result[compact_path] = dict(feature_data)
                    # Keep the fully qualified compact key for callers that supply namespaced
                    # registry references directly. Normal MCreator registry descriptors use
                    # the path-only form above.
                    if compact_full and compact_full != compact_path:
                        result[compact_full] = dict(feature_data)

    # Match a dedicated source field and a named stats record. This excludes
    # aggregate loops where the generated block is selected through a temporary
    # variable/switch, which cannot safely map to independent MCreator features.
    minable_pattern = re.compile(
        r"(?:new\s+)?WorldGenMinable\s*\(\s*"
        r"(?:[A-Za-z_$][\w$]*\.)*([A-Za-z_$][\w$]*)\s*,\s*"
        r"(?:[A-Za-z_$][\w$]*\.)*([A-Za-z_$][\w$]*_stats)\.clumpsize",
        re.DOTALL,
    )
    for field_name, stats_name in minable_pattern.findall(combined):
        settings = stats.get(stats_name)
        if not settings:
            continue
        for raw_registry in field_names.get(field_name, set()):
            compact = _compact_registry(raw_registry)
            if not compact:
                continue
            result[compact] = {
                **settings,
                "generateFeature": True,
                "generationShape": "UNIFORM",
                "restrictionBiomes": [],
                "sourceField": field_name,
                "sourceStats": stats_name,
                "approximation": "Uses the source default rate; source-side random variation/config overrides are not representable directly.",
            }
    _SOURCE_WORLDGEN_CACHE[key] = copy.deepcopy(result)
    return result
_LEGACY_CREATIVE_TABS = {
    "field_78030_b": "BUILDING_BLOCKS",
    "field_78031_c": "DECORATIONS",
    "field_78028_d": "REDSTONE",
    "field_78029_e": "TRANSPORTATION",
    "field_78026_f": "MISC",
    "field_78027_g": "MISC",
    "field_78039_h": "FOOD",
    "field_78040_i": "TOOLS",
    "field_78037_j": "COMBAT",
    "field_78038_k": "BREWING",
    "field_78035_l": "MATERIALS",
    "field_78036_m": "MISC",
    "tabBlock": "BUILDING_BLOCKS",
    "tabDecorations": "DECORATIONS",
    "tabRedstone": "REDSTONE",
    "tabTransport": "TRANSPORTATION",
    "tabMisc": "MISC",
    "tabFood": "FOOD",
    "tabTools": "TOOLS",
    "tabCombat": "COMBAT",
    "tabBrewing": "BREWING",
    "tabMaterials": "MATERIALS",
    "BUILDING_BLOCKS": "BUILDING_BLOCKS",
    "NATURAL_BLOCKS": "DECORATIONS",
    "REDSTONE_BLOCKS": "REDSTONE",
    "FUNCTIONAL_BLOCKS": "TRANSPORTATION",
    "FOOD_AND_DRINKS": "FOOD",
    "TOOLS_AND_UTILITIES": "TOOLS",
    "COMBAT": "COMBAT",
    "INGREDIENTS": "MATERIALS",
    "SPAWN_EGGS": "MISC",
    "OP_BLOCKS": "OP_BLOCKS",
}

def _resource_data_roots(source_root: Path) -> list[Path]:
    roots: list[Path] = []
    for candidate in (source_root / "src" / "main" / "resources" / "data", source_root / "data"):
        if candidate.exists() and candidate.is_dir():
            roots.append(candidate)
    for path in source_root.rglob("data"):
        if path.is_dir() and path not in roots:
            roots.append(path)
    return roots

def _resource_biome_definitions(root: Path, source_modid: str) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for data_root in _resource_data_roots(root):
        for path in data_root.rglob("*.json"):
            parts = path.relative_to(data_root).parts
            if len(parts) < 3 or parts[0].lower() != source_modid.lower():
                continue
            lower = [p.lower() for p in parts]
            if lower[1] != "worldgen" or lower[2] != "biome":
                continue
            try:
                payload = json.loads(path.read_text(encoding="utf-8-sig", errors="replace"))
            except Exception:
                continue
            if not isinstance(payload, dict):
                continue
            result[_snake(path.stem)] = payload
    return result

def _resource_dimension_definitions(root: Path, source_modid: str) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    dimensions: dict[str, dict[str, Any]] = {}
    types: dict[str, dict[str, Any]] = {}
    for data_root in _resource_data_roots(root):
        for path in data_root.rglob("*.json"):
            parts = path.relative_to(data_root).parts
            if len(parts) < 3 or parts[0].lower() != source_modid.lower():
                continue
            lower = [p.lower() for p in parts]
            if lower[1] not in {"dimension", "dimension_type"}:
                continue
            try:
                payload = json.loads(path.read_text(encoding="utf-8-sig", errors="replace"))
            except Exception:
                continue
            if not isinstance(payload, dict):
                continue
            target = dimensions if lower[1] == "dimension" else types
            target[_snake(path.stem)] = payload
    return dimensions, types

def _resource_namespaced_registry(payload_value: Any, source_modid: str) -> str:
    if not isinstance(payload_value, str):
        return ""
    value = payload_value.strip()
    if ":" in value:
        ns, path = value.split(":", 1)
        return _snake(path) if ns.lower() == source_modid.lower() else ""
    return _snake(value)

def _copy_source_block_model_resources(source_roots: list[Path], source_modid: str, target_modid: str, registry: str, workspace_root: Path) -> list[str]:
    """Copy exact source blockstate/model JSONs into the generated 1.20+/1.21 resource tree.

    Vanilla/Mojmap blockstate and model JSON is already the compatible format for modern
    NeoForge. This preserves stateful models such as Duct Tape/Pizza instead of collapsing
    them to an inventory icon. Source namespace references are rewritten only for the ported
    mod namespace; vanilla ``minecraft:block/*`` references stay vanilla.
    """
    outputs=[]
    resource_assets = workspace_root / "src" / "main" / "resources" / "assets" / target_modid
    resource_assets.mkdir(parents=True, exist_ok=True)
    wanted_models=set()
    for root in source_roots:
        for base in (
            root / "src" / "main" / "resources" / "assets" / source_modid,
            root / "assets" / source_modid,
        ):
            if not base.exists():
                continue
            bs=base / "blockstates" / f"{_snake(registry)}.json"
            if bs.exists():
                try:
                    payload=_read_json(bs) or {}
                    out_payload=_rewrite_model_namespace(payload, {source_modid}, target_modid)
                    out=resource_assets/"blockstates"/bs.name
                    out.parent.mkdir(parents=True, exist_ok=True)
                    out.write_text(json.dumps(out_payload, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
                    outputs.append(str(out));
                    # Gather referenced models, including multipart/variants.
                    refs=[]
                    variants=payload.get("variants",{}) if isinstance(payload,dict) else {}
                    for value in variants.values() if isinstance(variants,dict) else []:
                        if isinstance(value,dict) and isinstance(value.get("model"),str): refs.append(value["model"])
                        elif isinstance(value,list): refs.extend(str(x.get("model")) for x in value if isinstance(x,dict) and isinstance(x.get("model"),str))
                    for part in payload.get("multipart",[]) if isinstance(payload,dict) else []:
                        apply=part.get("apply",{}) if isinstance(part,dict) else {}
                        if isinstance(apply,dict) and isinstance(apply.get("model"),str): refs.append(apply["model"])
                    wanted_models.update(refs)
                except Exception:
                    pass
            model_roots=[base/"models"/"block"]
            for ref in list(wanted_models)+[_snake(registry)]:
                model_name=ref.split(":",1)[-1].removeprefix("models/").removeprefix("block/")
                for mr in model_roots:
                    src=mr / f"{model_name}.json"
                    if not src.exists():
                        continue
                    try: payload=_read_json(src) or {}
                    except Exception: continue
                    out_payload=_rewrite_model_namespace(payload,{source_modid},target_modid)
                    out=resource_assets/"models"/"block"/f"{model_name}.json"
                    out.parent.mkdir(parents=True,exist_ok=True)
                    out.write_text(json.dumps(out_payload, indent=2, ensure_ascii=False)+"\n",encoding="utf-8")
                    outputs.append(str(out))
    return outputs

def _mcreator_block_model_info(source_roots: list[Path], source_modid: str, registry: str) -> dict[str, Any]:
    """Infer MCreator's native block render type and concrete textures from model/blockstate JSON."""
    registry = _snake(registry)
    candidates: list[Path] = []
    for root in source_roots:
        for base in (
            root / "src" / "main" / "resources" / "assets" / source_modid / "models" / "block",
            root / "assets" / source_modid / "models" / "block",
        ):
            if base.exists():
                candidates.extend([base / f"{registry}.json", base / f"{registry}_inventory.json"])
        for base in (
            root / "src" / "main" / "resources" / "assets" / source_modid / "blockstates",
            root / "assets" / source_modid / "blockstates",
        ):
            state = base / f"{registry}.json"
            if not state.exists():
                continue
            try:
                state_payload = json.loads(state.read_text(encoding="utf-8-sig", errors="replace"))
            except Exception:
                continue
            refs: list[str] = []
            variants = state_payload.get("variants", {}) if isinstance(state_payload, dict) else {}
            if isinstance(variants, dict):
                for variant in variants.values():
                    if isinstance(variant, dict) and isinstance(variant.get("model"), str):
                        refs.append(variant["model"])
                    elif isinstance(variant, list):
                        refs.extend(str(v.get("model")) for v in variant if isinstance(v, dict) and isinstance(v.get("model"), str))
            multipart = state_payload.get("multipart", []) if isinstance(state_payload, dict) else []
            if isinstance(multipart, list):
                for part in multipart:
                    apply = part.get("apply", {}) if isinstance(part, dict) else {}
                    if isinstance(apply, dict) and isinstance(apply.get("model"), str):
                        refs.append(apply["model"])
            for ref in refs:
                ref_path = ref.split(":", 1)[-1].removeprefix("models/")
                for mr in (root / "src" / "main" / "resources", root):
                    candidate = mr / "assets" / source_modid / "models" / f"{ref_path}.json"
                    if candidate.exists():
                        candidates.append(candidate)
                        break
    if not candidates:
        for root in source_roots:
            for path in root.rglob("*.json"):
                parts = [x.lower() for x in path.parts]
                if "models" in parts and "block" in parts and _compact_registry(path.stem) == _compact_registry(registry):
                    candidates.append(path)
    unique=[]; seen=set()
    for path in candidates:
        key=str(path.resolve()).lower()
        if key not in seen:
            seen.add(key); unique.append(path)
    for path in unique:
        try:
            payload = json.loads(path.read_text(encoding="utf-8-sig", errors="replace"))
        except Exception:
            continue
        if not isinstance(payload, dict):
            continue
        parent = str(payload.get("parent", "")).lower()
        textures = payload.get("textures") if isinstance(payload.get("textures"), dict) else {}
        if parent.endswith("cross") or parent == "minecraft:block/cross":
            return {"renderType": 12, "customModelName":"Cross model", "texture": str(textures.get("cross") or textures.get("plant") or textures.get("particle") or ""), "modelPath": str(path)}
        if parent.endswith("grass_block") or ("side" in textures and "top" in textures and "bottom" in textures and "overlay" in textures):
            side=str(textures.get("side") or textures.get("all") or "")
            return {"renderType":14,"customModelName":"Grass block","textureTop":str(textures.get("top") or ""),"textureBottom":str(textures.get("bottom") or ""),"texture":side,"textureLeft":side,"textureFront":side,"textureRight":side,"textureBack":side,"modelPath":str(path)}
        if parent.endswith("crop"):
            return {"renderType":12,"customModelName":"Cross model","texture":str(textures.get("crop") or textures.get("all") or textures.get("particle") or ""),"modelPath":str(path)}
        if parent.endswith("flower_pot_cross"):
            return {"renderType":12,"customModelName":"Cross model","texture":str(textures.get("plant") or textures.get("all") or textures.get("particle") or ""),"modelPath":str(path)}
        if parent.endswith("cube_all"):
            tex=str(textures.get("all") or textures.get("particle") or "")
            return {"renderType":11,"customModelName":"Single texture","texture":tex,"modelPath":str(path)}
        if parent.endswith("cube"):
            tex=str(textures.get("all") or textures.get("particle") or "")
            return {"renderType":10,"customModelName":"Normal","texture":tex,"textureTop":str(textures.get("up") or textures.get("top") or tex),"textureBottom":str(textures.get("down") or textures.get("bottom") or tex),"textureLeft":str(textures.get("west") or tex),"textureFront":str(textures.get("north") or tex),"textureRight":str(textures.get("east") or tex),"textureBack":str(textures.get("south") or tex),"modelPath":str(path)}
        if any(k in textures for k in ("up","down","north","south","east","west","side","top","bottom")):
            side=str(textures.get("side") or textures.get("north") or textures.get("particle") or "")
            return {"renderType":11,"texture":side,"textureTop":_mcreator_texture_ref(str(textures.get("top") or textures.get("up") or side)),"textureBottom":_mcreator_texture_ref(str(textures.get("bottom") or textures.get("down") or side)),"textureLeft":_mcreator_texture_ref(str(textures.get("west") or side)),"textureFront":_mcreator_texture_ref(str(textures.get("north") or side)),"textureRight":_mcreator_texture_ref(str(textures.get("east") or side)),"textureBack":_mcreator_texture_ref(str(textures.get("south") or side)),"modelPath":str(path)}
    return {}

def _normalize_block_model_info_textures(info: dict[str, Any]) -> dict[str, Any]:
    out = dict(info or {})
    for key in ("texture", "textureTop", "textureBottom", "textureLeft", "textureFront", "textureRight", "textureBack", "itemTexture", "particleTexture"):
        if key in out:
            out[key] = _mcreator_texture_ref(str(out.get(key) or ""))
    return out


def _resource_dimension_updates(payload: dict[str, Any], dtype: dict[str, Any], biome_registry: str, source_modid: str) -> dict[str, Any]:
    generator = payload.get("generator", {}) if isinstance(payload.get("generator"), dict) else {}
    settings = generator.get("settings", {}) if isinstance(generator.get("settings"), dict) else {}
    biome_source = generator.get("biome_source", {}) if isinstance(generator.get("biome_source"), dict) else {}
    biomes: list[str] = []
    for entry in biome_source.get("biomes", []) if isinstance(biome_source.get("biomes"), list) else []:
        if isinstance(entry, dict):
            b = _resource_namespaced_registry(entry.get("biome"), source_modid)
            if b:
                biomes.append(f"CUSTOM:{_pascal(b)}")
    if biome_registry and not biomes:
        biomes = [f"CUSTOM:{_pascal(biome_registry)}"]
    d = dtype if isinstance(dtype, dict) else {}
    return {
        "biomesInDimension": biomes or ["Plains"],
        "biomesInDimensionCaves": [],
        "worldGenType": "Nether like gen" if bool(d.get("has_ceiling")) else "Normal world gen",
        "mainFillerBlock": "Blocks.STONE#0",
        "fluidBlock": "Blocks.AIR",
        "seaLevel": int(settings.get("sea_level", 63) or 63),
        "generateOreVeins": bool(settings.get("ore_veins_enabled", True)),
        "generateAquifers": bool(settings.get("aquifers_enabled", True)),
        "horizontalNoiseSize": int(settings.get("noise", {}).get("size_horizontal", 1) if isinstance(settings.get("noise"), dict) else 1),
        "verticalNoiseSize": int(settings.get("noise", {}).get("size_vertical", 2) if isinstance(settings.get("noise"), dict) else 2),
        "defaultEffects": "overworld",
        "useCustomEffects": False,
        "hasClouds": not bool(d.get("has_ceiling")),
        "cloudHeight": 192,
        "skyType": "NONE" if bool(d.get("has_ceiling")) else "NORMAL",
        "sunHeightAffectsFog": True,
        "canRespawnHere": bool(d.get("natural", False) and d.get("bed_works", True) is False),
        "hasFog": False,
        "ambientLight": float(d.get("ambient_light", 0.5) or 0.5),
        "doesWaterVaporize": bool(d.get("ultrawarm", False)),
        "hasFixedTime": "fixed_time" in d,
        "fixedTimeValue": int(d.get("fixed_time", 0) or 0),
        "coordinateScale": float(d.get("coordinate_scale", 1.0) or 1.0),
        "infiniburnTag": str(d.get("infiniburn", "#minecraft:infiniburn_overworld")).lstrip("#"),
        "enableCustomSkyboxTextures": False,
        "enableCustomSunMoonTextures": False,
        "bedWorks": bool(d.get("bed_works", True)),
        "hasSkyLight": bool(d.get("has_skylight", True)),
        "imitateOverworldBehaviour": bool(d.get("natural", False)),
        "piglinSafe": bool(d.get("piglin_safe", False)),
        "hasRaids": bool(d.get("has_raids", True)),
        "minMonsterSpawnLightLimit": int(d.get("monster_spawn_light_level", 0) if isinstance(d.get("monster_spawn_light_level"), int) else 0),
        "maxMonsterSpawnLightLimit": int(d.get("monster_spawn_light_level", 7) if isinstance(d.get("monster_spawn_light_level"), int) else 7),
        "monsterSpawnBlockLightLimit": int(d.get("monster_spawn_block_light_limit", 0) or 0),
        "creativeTabs": ["TOOLS"], "texture": "", "portalTexture": "", "enablePortal": False,
        "name": _display_name(next(iter(biome_registry.split()), "dimension")),
    }

def _resource_biome_updates(payload: dict[str, Any]) -> dict[str, Any]:
    effects = payload.get("effects", {}) if isinstance(payload.get("effects"), dict) else {}
    spawners = payload.get("spawners", {}) if isinstance(payload.get("spawners"), dict) else {}
    spawn_entries: list[dict[str, Any]] = []
    for spawn_type in ("monster", "creature", "ambient", "axolotls", "underground_water_creature", "water_creature", "water_ambient", "misc"):
        entries = spawners.get(spawn_type, []) if isinstance(spawners.get(spawn_type), list) else []
        for entry in entries:
            if not isinstance(entry, dict) or not isinstance(entry.get("type"), str):
                continue
            spawn_entries.append({"entity": entry["type"], "minGroup": int(entry.get("minCount", 1) or 1), "maxGroup": int(entry.get("maxCount", 1) or 1), "weight": int(entry.get("weight", 1) or 1), "spawnType": "monster" if spawn_type == "monster" else "creature"})
    return {
        "groundBlock": "Blocks.GRASS_BLOCK", "undergroundBlock": "Blocks.DIRT", "ambientSound": str(effects.get("ambient_sound", "") or ""),
        "additionsSound": str(effects.get("additions_sound", {}).get("sound", "") if isinstance(effects.get("additions_sound"), dict) else ""),
        "music": str(effects.get("music", {}).get("sound", "") if isinstance(effects.get("music"), dict) else ""),
        "moodSound": str(effects.get("mood_sound", {}).get("sound", "") if isinstance(effects.get("mood_sound"), dict) else ""),
        "moodSoundDelay": int(effects.get("mood_sound", {}).get("tick_delay", 6000) if isinstance(effects.get("mood_sound"), dict) else 6000),
        "spawnParticles": bool(effects.get("particle")), "particleToSpawn": "CLOUD", "particlesProbability": float(effects.get("particle", {}).get("probability", 0) * 100 if isinstance(effects.get("particle"), dict) else 0),
        "rainingPossibility": float(payload.get("downfall", 0.5) or 0.5), "temperature": float(payload.get("temperature", 0.5) or 0.5), "spawnBiome": False, "spawnInCaves": False, "spawnBiomeNether": False,
        "genTemperature": {"min": -0.2, "max": 0.2}, "genHumidity": {"min": -0.2, "max": 0.2}, "genContinentalness": {"min": 0.31, "max": 0.71}, "genErosion": {"min": 0.6, "max": 1.0}, "genWeirdness": {"min": -0.86, "max": -0.46}, "genDepth": {"min": 0.2, "max": 0.9},
        "treesPerChunk": 0, "vanillaTreeType": "Default", "treeType": 0, "minHeight": 2, "treeStem": "", "treeBranch": "", "treeVines": "", "treeFruits": "", "defaultFeatures": [],
        "spawnStronghold": False, "spawnMineshaft": False, "spawnMineshaftMesa": False, "spawnPillagerOutpost": False, "villageType": "none", "spawnWoodlandMansion": False, "spawnJungleTemple": False, "spawnDesertPyramid": False, "spawnSwampHut": False, "spawnIgloo": False, "spawnOceanMonument": False, "spawnShipwreck": False, "spawnShipwreckBeached": False, "spawnBuriedTreasure": False, "oceanRuinType": "NONE", "spawnNetherBridge": False, "spawnNetherFossil": False, "spawnBastionRemnant": False, "spawnEndCity": False, "spawnRuinedPortal": "NONE", "spawnEntries": spawn_entries,
    }

def _source_dimension_biomes(root: Path) -> dict[str, str]:
    """Recover a provider-class -> biome-registry mapping from Java source."""
    root = Path(root).expanduser().resolve()
    key = str(root)
    if key in _SOURCE_DIMENSION_BIOME_CACHE:
        return dict(_SOURCE_DIMENSION_BIOME_CACHE[key])
    result: dict[str, str] = {}
    named_vanilla = {
        "plains": "plains", "desert": "desert", "forest": "forest",
        "taiga": "taiga", "swamp": "swamp", "jungle": "jungle",
        "savanna": "savanna", "badlands": "badlands", "theend": "the_end",
        "netherwastes": "nether_wastes",
    }
    for path in sorted(root.rglob("*.java")):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        if "WorldProvider" not in text and "DimensionType" not in text:
            continue
        # Legacy custom biomes commonly name the instance through the fluent
        # setBiomeName/func_76735_a call before passing it to WorldChunkManager.
        custom = re.search(
            r"(?:setBiomeName|func_76735_a)\s*\(\s*[\"']([^\"']+)[\"']\s*\)",
            text,
        )
        if custom:
            result[path.stem] = _snake(custom.group(1))
            continue
        direct = re.search(
            r"new\s+WorldChunkManager(?:Hell)?\s*\(\s*(?:BiomeGenBase|Biomes)\.([A-Za-z_$][\w$]*)",
            text,
        )
        if direct:
            token = _compact_registry(direct.group(1))
            if token in named_vanilla:
                result[path.stem] = named_vanilla[token]
    _SOURCE_DIMENSION_BIOME_CACHE[key] = dict(result)
    return result
def _source_creative_tabs(root: Path) -> dict[str, str]:
    """Recover vanilla or custom creative-tab assignments by source class.
    Custom source tabs are represented internally as ``SOURCE_TAB:<field>`` and
    are resolved to generated MCreator tab elements after content discovery.
    """
    root = Path(root).expanduser().resolve()
    key = str(root)
    if key in _SOURCE_CREATIVE_TAB_CACHE:
        return dict(_SOURCE_CREATIVE_TAB_CACHE[key])
    result: dict[str, str] = {}
    parents: dict[str, str] = {}
    vanilla_pattern = re.compile(
        r"(?:setCreativeTab|func_77637_a|func_149647_a)\s*\(\s*"
        r"(?:CreativeTabs|CreativeModeTabs)\.([A-Za-z_$][\w$]*)",
        re.DOTALL,
    )
    custom_pattern = re.compile(
        r"(?:setCreativeTab|func_77637_a|func_149647_a|this|super)\s*\("
        r"[^;{}]{0,800}?\b(?:[A-Za-z_$][\w$]*\.)+(TAB_[A-Za-z0-9_$]+)\b",
        re.DOTALL,
    )
    for path in sorted(root.rglob("*.java")):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        header = re.search(
            r"\bclass\s+([A-Za-z_$][\w$]*)\s+extends\s+([A-Za-z_$][\w.$]*)",
            text,
        )
        if header:
            parents[header.group(1)] = _simple(header.group(2))
        match = vanilla_pattern.search(text)
        if match:
            tab = _LEGACY_CREATIVE_TABS.get(match.group(1))
            if tab:
                result[path.stem] = tab
                continue
        custom = custom_pattern.search(text)
        if custom:
            result[path.stem] = f"SOURCE_TAB:{custom.group(1)}"
    # Propagate direct tab assignments through source-only inheritance chains.
    # Some example corpora intentionally omit compiled item classes, so Java
    # inheritance must remain a first-class source of truth.
    changed = True
    while changed:
        changed = False
        for child, parent in parents.items():
            if child not in result and parent in result:
                result[child] = result[parent]
                changed = True
    _SOURCE_CREATIVE_TAB_CACHE[key] = dict(result)
    return result
def _source_registry_creative_tabs(root: Path) -> dict[str, str]:
    """Recover custom creative tabs passed to individual registered instances."""
    root = Path(root).expanduser().resolve()
    key = str(root)
    if key in _SOURCE_REGISTRY_TAB_CACHE:
        return dict(_SOURCE_REGISTRY_TAB_CACHE[key])
    field_registries = _source_field_registered_names(root)
    result: dict[str, str] = {}
    tab_pattern = re.compile(r"\b(?:[A-Za-z_$][\w$]*\.)+(TAB_[A-Za-z0-9_$]+)\b")
    for path in sorted(root.rglob("*.java")):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        for field_name, _class_name, arguments in _source_constructor_assignments(text):
            match = tab_pattern.search(arguments)
            if not match:
                continue
            for registry in field_registries.get(field_name, set()):
                result[_compact_registry(registry)] = match.group(1)
    _SOURCE_REGISTRY_TAB_CACHE[key] = dict(result)
    return result
def _source_custom_tabs(root: Path, language_names: dict[str, str]) -> dict[str, dict[str, str]]:
    """Recover legacy ``new CreativeTabs`` declarations, labels, and icons."""
    root = Path(root).expanduser().resolve()
    key = str(root)
    cached = _SOURCE_CUSTOM_TAB_CACHE.get(key)
    if cached is not None:
        return copy.deepcopy(cached)
    field_registries = _source_field_registered_names(root)
    result: dict[str, dict[str, str]] = {}
    declaration = re.compile(
        r"(?:public|protected|private)?\s*static\s+final\s+CreativeTabs\s+"
        r"([A-Za-z_$][\w$]*)\s*=\s*new\s+CreativeTabs\s*\(\s*[\"']([^\"']+)[\"']\s*\)",
        re.DOTALL,
    )
    for path in sorted(root.rglob("*.java")):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        for match in declaration.finditer(text):
            field_name, label_key = match.groups()
            tail = text[match.end(): match.end() + 1800]
            icon_expr = ""
            icon_call = re.search(r"new\s+ItemStack\s*\(([^;]{0,700})\)", tail, re.DOTALL)
            if icon_call:
                icon_expr = icon_call.group(1)
            field_refs = re.findall(r"(?:[A-Za-z_$][\w$]*\.)+([A-Za-z_$][\w$]*)", icon_expr)
            icon_field = field_refs[-1] if field_refs else ""
            icon_registry = sorted(field_registries.get(icon_field, set()))[0] if field_registries.get(icon_field) else ""
            display_name = (
                language_names.get(label_key.lower())
                or language_names.get(_compact_registry(label_key))
                or _display_name(_snake(re.sub(r"^tab[_-]?", "", label_key, flags=re.IGNORECASE)))
            )
            result.setdefault(field_name, {
                "field": field_name,
                "label_key": label_key,
                "display_name": display_name,
                "icon_field": icon_field,
                "icon_registry": icon_registry,
            })
    _SOURCE_CUSTOM_TAB_CACHE[key] = copy.deepcopy(result)
    return result
def _source_armor_set_classes(root: Path) -> dict[str, str]:
    """Map source armor-set registry prefixes to their concrete armor class."""
    root = Path(root).expanduser().resolve()
    key = str(root)
    if key in _SOURCE_ARMOR_SET_CACHE:
        return dict(_SOURCE_ARMOR_SET_CACHE[key])
    result: dict[str, str] = {}
    pattern = re.compile(
        r"\bnew\s+(?:[A-Za-z_$][\w.$]*\.)?ArmorSet\s*\(\s*[\"']([^\"']+)[\"']\s*,\s*"
        r"([A-Za-z_$][\w.$]*Armor)\.class",
        re.DOTALL,
    )
    for path in sorted(root.rglob("*.java")):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        for set_name, armor_class in pattern.findall(text):
            result[_compact_registry(set_name)] = _simple(armor_class)
    _SOURCE_ARMOR_SET_CACHE[key] = dict(result)
    return result
def _resolved_creative_tab(
    simple: str,
    graph: LegacyModGraph,
    source_tabs: dict[str, str],
    javap_text: str = "",
) -> str:
    """Resolve a source class's vanilla tab, following simple inheritance."""
    current = simple
    seen: set[str] = set()
    for _ in range(16):
        if not current or current in seen:
            break
        seen.add(current)
        if current in source_tabs:
            return source_tabs[current]
        source = graph.class_files.get(current) or graph.java_files.get(current)
        if source is None:
            break
        _name, current = _class_header_info_any(source)
    for field_name, tab in _LEGACY_CREATIVE_TABS.items():
        if re.search(rf"(?:CreativeTabs|CreativeModeTabs)[./]{re.escape(field_name)}\b", javap_text):
            return tab
    return ""
def _source_registered_names(root: Path) -> dict[str, set[str]]:
    """Recover class -> registry-path mappings from decompiled Java.
    This covers direct constructor registration used heavily by 1.12 mods,
    legacy GameRegistry field registrations, and DimensionType provider links.
    The result intentionally remains class based so one reusable Item/Block
    implementation can expand into every registered instance.
    """
    root = Path(root).expanduser().resolve()
    key = str(root)
    if key in _SOURCE_REGISTRY_CACHE:
        return {name: set(values) for name, values in _SOURCE_REGISTRY_CACHE[key].items()}
    result: dict[str, set[str]] = {}
    field_classes: dict[str, str] = {}
    java_files = sorted(root.rglob("*.java"))
    texts: list[str] = []
    for path in java_files:
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        texts.append(text)
        for field_name, simple, args in _source_constructor_assignments(text):
            field_classes[field_name] = simple
            resource = re.search(
                r"new\s+ResourceLocation\s*\(\s*[\"']([^\"']+)[\"']\s*,\s*[\"']([^\"']+)[\"']",
                args,
            )
            if resource:
                raw_name = resource.group(2)
            else:
                first_string = re.search(r"[\"']([^\"'\r\n]+)[\"']", args)
                raw_name = first_string.group(1) if first_string else ""
            raw_name = _registry_path(raw_name)
            if _looks_like_registry_path(raw_name):
                result.setdefault(simple, set()).add(raw_name)
    combined = "\n".join(texts)
    # Legacy explicit registration calls are more authoritative than unlocalized
    # names passed to constructors.
    explicit_by_class: dict[str, set[str]] = {}
    for _call_name, raw_args, _offset in _iter_java_calls(combined, {"registerItem", "registerBlock"}):
        arguments = _split_java_arguments(raw_args)
        if len(arguments) < 2:
            continue
        field_match = re.fullmatch(r"\s*(?:[A-Za-z_$][\w$]*\.)*([A-Za-z_$][\w$]*)\s*", arguments[0])
        raw_name = _quoted_registry_argument(arguments)
        if not field_match or not raw_name:
            continue
        field_name = field_match.group(1)
        simple = field_classes.get(field_name)
        if simple:
            clean_name = _registry_path(raw_name)
            if _looks_like_registry_path(clean_name):
                explicit_by_class.setdefault(simple, set()).add(clean_name)
    for simple, names in explicit_by_class.items():
        result[simple] = {name for name in names if name}
    # 1.12 dimension registration links a provider class to the authoritative
    # namespace:path value even though provider constructors take no registry name.
    for match in re.finditer(
        r"DimensionType\.register\s*\(\s*[\"']([^\"']+)[\"'][^;]{0,600}?"
        r"([A-Za-z_$][\w.$]*WorldProvider[A-Za-z0-9_$]*|WorldProvider[A-Za-z0-9_$]*)\.class",
        combined,
        re.DOTALL,
    ):
        raw_name, provider = match.groups()
        clean_name = _registry_path(raw_name)
        if _looks_like_registry_path(clean_name):
            result.setdefault(_simple(provider), set()).add(clean_name)
    # Modern DeferredRegister-style registrations in a shared main class.
    for _call_name, raw_args, offset in _iter_java_calls(combined, {"register"}):
        args = _split_java_arguments(raw_args)
        if len(args) < 2:
            continue
        raw_name = ""
        first = re.fullmatch(r"\s*[\"\']([^\"\']+)[\"\']\s*", args[0]) if args else None
        if first:
            raw_name = first.group(1)
        else:
            raw_name = _quoted_registry_argument(args)
        if not raw_name:
            continue
        clean_name = _registry_path(raw_name)
        if not _looks_like_registry_path(clean_name):
            continue
        prefix = combined[max(0, offset - 96):offset]
        receiver_match = re.search(r"([A-Za-z_$][A-Za-z0-9_$]*)\s*\.\s*$", prefix)
        receiver = receiver_match.group(1) if receiver_match else ""
        if receiver not in {"ITEMS", "BLOCKS", "ENTITY_TYPES", "ENTITY_TYPE", "SOUNDS", "SOUND_EVENTS", "ITEM_REGISTRY", "BLOCK_REGISTRY", "ENTITY_REGISTRY", "REGISTRY"}:
            continue
        body = args[1]
        classes = []
        classes.extend(re.findall(r"\bnew\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*\(", body))
        classes.extend(re.findall(r"\b([A-Za-z_$][A-Za-z0-9_$.]*)\s*::\s*new\b", body))
        classes.extend(re.findall(r"\.Builder\s*\.<\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*>\s*of\b", body))
        for class_name in classes:
            simple = _simple(class_name)
            if simple and simple not in {"Object", "Item", "Block", "EntityType"}:
                result.setdefault(simple, set()).add(clean_name)
    _SOURCE_REGISTRY_CACHE[key] = {name: set(values) for name, values in result.items()}
    return result
def _normalise_roots(root_or_roots: Path | list[Path] | tuple[Path, ...]) -> list[Path]:
    values = root_or_roots if isinstance(root_or_roots, (list, tuple)) else [root_or_roots]
    roots: list[Path] = []
    seen: set[str] = set()
    for value in values:
        path = Path(value).expanduser().resolve()
        key = str(path).lower()
        if key not in seen:
            seen.add(key)
            roots.append(path)
    return roots
def _load_language_names(root_or_roots: Path | list[Path] | tuple[Path, ...]) -> dict[str, str]:
    roots = _normalise_roots(root_or_roots)
    key = "|".join(str(root) for root in roots)
    if key in _LANGUAGE_NAME_CACHE:
        return dict(_LANGUAGE_NAME_CACHE[key])
    result: dict[str, str] = {}
    descriptions: dict[str, list[str]] = {}
    def add(translation_key: str, value: str) -> None:
        if not value.strip():
            return
        lowered = translation_key.strip().lower()
        clean_value = value.strip()
        result.setdefault(lowered, clean_value)
        result.setdefault(_compact_registry(lowered), clean_value)
        tokens = [token for token in re.split(r"[.:]", lowered) if token]
        is_description = any(token in {"desc", "description", "tooltip", "info"} for token in tokens)
        if tokens and tokens[-1] in {"name", "desc", "description", "tooltip"}:
            tokens.pop()
        ignored = {"item", "tile", "block", "entity", "biome", "dimension", "tooltip"}
        candidates = [token for token in tokens if token not in ignored]
        if not is_description:
            for candidate in candidates[-3:]:
                result.setdefault(candidate, clean_value)
                result.setdefault(_compact_registry(candidate), clean_value)
        else:
            # Description keys vary widely, but the final specific token is the
            # best registry-like identity available without mod-specific rules.
            for candidate in candidates[-2:]:
                compact = _compact_registry(candidate)
                if compact and clean_value not in descriptions.setdefault(compact, []):
                    descriptions[compact].append(clean_value)
    language_paths = [path for root in roots for path in root.rglob("*.lang")]
    for path in language_paths:
        if "en_us" not in path.name.lower() and "en_us" not in path.as_posix().lower() and "en_us" != path.stem.lower():
            # Legacy packs often use en_US.lang; lowercasing handles it.
            if path.stem.lower() not in {"en_us", "en_us.lang"}:
                continue
        try:
            for line in path.read_text(encoding="utf-8-sig", errors="replace").splitlines():
                if not line or line.lstrip().startswith("#") or "=" not in line:
                    continue
                name, value = line.split("=", 1)
                add(name, value)
        except OSError:
            continue
    for path in [path for root in roots for path in root.rglob("en_us.json")]:
        try:
            payload = json.loads(path.read_text(encoding="utf-8-sig", errors="replace"))
        except Exception:
            continue
        if isinstance(payload, dict):
            for name, value in payload.items():
                if isinstance(value, str):
                    add(str(name), value)
    # Some older mods (including many 1.7.10-era mods) never ship an English
    # language resource.  Instead they register display names directly from
    # Java using LanguageRegistry.addNameForObject / addStringLocalization.
    # Recover those names generically and associate object fields with their
    # authoritative GameRegistry keys when possible.
    for root in roots:
        try:
            field_registry = _source_field_registered_names(root)
        except Exception:
            field_registry = {}
        for path in root.rglob("*.java"):
            try:
                text = path.read_text(encoding="utf-8-sig", errors="replace")
            except OSError:
                continue
            # LanguageRegistry.instance().addNameForObject(MyItem, "en_US", "Big Bertha")
            for match in re.finditer(
                r"\baddNameForObject\s*\(\s*([A-Za-z_$][\w$]*)\s*,\s*[\"'](?:en_US|en_us)[\"']\s*,\s*[\"']((?:\\.|[^\"'\\])*)[\"']\s*\)",
                text,
            ):
                field_name, display = match.groups()
                try:
                    display = bytes(display, "utf-8").decode("unicode_escape")
                except Exception:
                    pass
                for registry in field_registry.get(field_name, set()):
                    add(f"item.{registry}.name", display)
                    add(f"block.{registry}.name", display)
                    add(f"entity.{registry}.name", display)
                    add(registry, display)
            # LanguageRegistry.instance().addStringLocalization("item.foo.name", "en_US", "Foo")
            for match in re.finditer(
                r"\baddStringLocalization\s*\(\s*[\"']([^\"']+)[\"']\s*,\s*[\"'](?:en_US|en_us)[\"']\s*,\s*[\"']((?:\\.|[^\"'\\])*)[\"']\s*\)",
                text,
            ):
                translation_key, display = match.groups()
                try:
                    display = bytes(display, "utf-8").decode("unicode_escape")
                except Exception:
                    pass
                add(translation_key, display)
    _LANGUAGE_NAME_CACHE[key] = dict(result)
    _LANGUAGE_DESCRIPTION_CACHE[key] = {name: list(values) for name, values in descriptions.items()}
    return result
def _language_descriptions(root_or_roots: Path | list[Path] | tuple[Path, ...], registry: str) -> list[str]:
    roots = _normalise_roots(root_or_roots)
    key = "|".join(str(root) for root in roots)
    if key not in _LANGUAGE_DESCRIPTION_CACHE:
        _load_language_names(roots)
    cache = _LANGUAGE_DESCRIPTION_CACHE.get(key, {})
    compact = _compact_registry(registry)
    variants = [compact]
    for suffix in ("ingot", "ore", "block", "item", "tool", "sword", "staff", "bow", "helmet", "chestplate", "leggings", "boots"):
        if compact.endswith(suffix) and len(compact) > len(suffix):
            variants.append(compact[: -len(suffix)])
    result: list[str] = []
    for variant in variants:
        for value in cache.get(variant, []):
            if value not in result:
                result.append(value)
    return result
_TOOLTIP_FORMATTING_CODES = {
    "BLACK": "\\u00a70",
    "DARK_BLUE": "\\u00a71",
    "DARK_GREEN": "\\u00a72",
    "DARK_AQUA": "\\u00a73",
    "DARK_RED": "\\u00a74",
    "DARK_PURPLE": "\\u00a75",
    "GOLD": "\\u00a76",
    "GRAY": "\\u00a77",
    "DARK_GRAY": "\\u00a78",
    "BLUE": "\\u00a79",
    "GREEN": "\\u00a7a",
    "AQUA": "\\u00a7b",
    "RED": "\\u00a7c",
    "LIGHT_PURPLE": "\\u00a7d",
    "YELLOW": "\\u00a7e",
    "WHITE": "\\u00a7f",
    "BOLD": "\\u00a7l",
    "STRIKETHROUGH": "\\u00a7m",
    "UNDERLINE": "\\u00a7n",
    "ITALIC": "\\u00a7o",
    "RESET": "\\u00a7r",
}
def _extract_tooltip_strings(text: str) -> list[str]:
    result: list[str] = []
    # Handles tooltip.add(TextFmt.Gold + "text") and direct string calls in
    # both MCP and obfuscated method names. Preserve formatting constants when
    # the source already provides them so MCreator can keep colored lore.
    for match in re.finditer(r"\b(?:tooltip|list|par\d+List)\s*\.\s*add\s*\((.*?)\)\s*;", text, re.DOTALL):
        expr = match.group(1)
        for name, escape in _TOOLTIP_FORMATTING_CODES.items():
            expr = re.sub(
                rf"\b(?:TextFormatting|EnumChatFormatting|ChatFormatting|Formatting)\.{name}\b",
                lambda _match, esc=escape: f'"{esc}"',
                expr,
            )
        pieces = re.findall(r'"((?:\\.|[^"\\])*)"', expr)
        if not pieces:
            continue
        value = "".join(bytes(piece, "utf-8").decode("unicode_escape") for piece in pieces).strip()
        if value and value not in result:
            result.append(value)
    return result
def _legacy_renderer_display(renderer_text: str) -> dict[str, dict[str, list[float]]]:
    """Recover old IItemRenderer GL transforms as item-model display data.
    Legacy renderers commonly dispatch EQUIPPED/EQUIPPED_FIRST_PERSON to a
    helper containing glRotatef/glScalef/glTranslatef calls.  The JSON display
    transform cannot encode an arbitrary OpenGL matrix, but preserving the
    source rotation/translation/scale is substantially more faithful than a
    generic centered-tool transform and usually keeps the handle in hand.
    """
    try:
        from model_bridge import _legacy_java_methods, eval_number, extract_balanced, split_top_level
    except Exception:
        return {}
    methods = _legacy_java_methods(renderer_text)
    render_entries = methods.get("renderItem", [])
    if not render_entries:
        return {}
    _params, render_body, _source = render_entries[0]
    def normalise_rotation(value: float) -> float:
        value = (float(value) + 180.0) % 360.0 - 180.0
        return round(value, 6)
    def transform_for_case(case_name: str) -> Optional[dict[str, list[float]]]:
        case_match = re.search(
            rf"\bcase\s+{re.escape(case_name)}\s*:(.*?)(?=\bcase\s+|\bdefault\s*:|\Z)",
            render_body,
            re.DOTALL,
        )
        if not case_match:
            return None
        case_body = case_match.group(1)
        helper_call = re.search(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\(([^;]*)\)\s*;", case_body)
        helper_body = case_body
        substitutions: dict[str, float] = {}
        if helper_call and helper_call.group(1) in methods:
            helper_name = helper_call.group(1)
            args = split_top_level(helper_call.group(2))
            overload = next((entry for entry in methods[helper_name] if len(entry[0]) == len(args)), None)
            if overload:
                helper_params, helper_body, _helper_source = overload
                for param, arg in zip(helper_params, args):
                    try:
                        substitutions[param] = float(eval_number(arg, substitutions))
                    except Exception:
                        pass
        rotations = [0.0, 0.0, 0.0]
        translation = [0.0, 0.0, 0.0]
        scale = [1.0, 1.0, 1.0]
        found = False
        for call in re.finditer(r"\bgl(Rotatef|Rotated|Translatef|Translated|Scalef|Scaled)\s*\(", helper_body):
            try:
                raw_args, _ = extract_balanced(helper_body, call.end() - 1)
                args = split_top_level(raw_args)
                values = [float(eval_number(arg, substitutions)) for arg in args]
            except Exception:
                continue
            operation = call.group(1).lower()
            if operation.startswith("rotate") and len(values) >= 4:
                angle, ax, ay, az = values[:4]
                axes = [abs(ax), abs(ay), abs(az)]
                dominant = max(range(3), key=axes.__getitem__)
                if axes[dominant] > 1e-8:
                    sign = (ax, ay, az)[dominant] / axes[dominant]
                    rotations[dominant] += angle * sign
                    found = True
            elif operation.startswith("translate") and len(values) >= 3:
                translation = values[:3]
                found = True
            elif operation.startswith("scale") and len(values) >= 3:
                scale = values[:3]
                found = True
        if not found:
            return None
        return {
            "rotation": [normalise_rotation(value) for value in rotations],
            "translation": [round(float(value), 6) for value in translation],
            "scale": [round(float(value), 6) for value in scale],
        }
    third = transform_for_case("EQUIPPED")
    first = transform_for_case("EQUIPPED_FIRST_PERSON")
    display: dict[str, dict[str, list[float]]] = {}
    if third:
        display["thirdperson_righthand"] = third
        display["thirdperson_lefthand"] = json.loads(json.dumps(third))
    if first:
        display["firstperson_righthand"] = first
        display["firstperson_lefthand"] = json.loads(json.dumps(first))
    return display
def _held_renderer_specs(roots: list[Path], compiled_registry_texts: Optional[dict[Path, str]] = None) -> dict[str, dict[str, Any]]:
    """Map registry paths to legacy IItemRenderer model/texture evidence.
    Decompiled Java is preferred because it preserves the original renderer source
    and makes GL transforms easy to recover. When only compiled classes are
    available, a lightweight bytecode fallback follows `registerItemRenderer`
    calls to the renderer class and then extracts the referenced model class and
    texture literals.
    """
    field_registries: dict[str, set[str]] = {}
    texts_by_class: dict[str, str] = {}
    combined_parts: list[str] = []
    for root in roots:
        for field, values in _source_field_registered_names(root).items():
            field_registries.setdefault(field, set()).update(values)
        for path in root.rglob("*.java"):
            try:
                text = path.read_text(encoding="utf-8-sig", errors="replace")
            except OSError:
                continue
            texts_by_class.setdefault(path.stem, text)
            combined_parts.append(text)
    def field_name_from_comment(comment: str) -> str:
        match = re.search(r"Field\s+(.+?):", comment)
        if not match:
            return ""
        return match.group(1).replace("/", ".").split(".")[-1].split("$")[0]
    # Compiled-only registry evidence. This is deliberately generic and only
    # recognizes explicit registerItem/registerBlock/register calls with a
    # static field and a string key. Source evidence remains preferred when both
    # forms are available. The likely registry-source hint is not sufficient for
    # older mods: client proxy classes frequently hold the item renderer links,
    # while the actual registration table lives in a class with an arbitrary name.
    # Therefore, when no source mapping is available, inspect only class files whose
    # constant pool contains registration tokens.
    compiled_registration_texts: dict[Path, str] = dict(compiled_registry_texts or {})
    compiled_candidates: list[Path] = []
    for root in roots:
        for path in root.rglob("*.class"):
            if path in compiled_registration_texts:
                continue
            try:
                raw = path.read_bytes()
            except OSError:
                continue
            if b"registerItem" in raw or b"registerBlock" in raw or b"GameRegistry" in raw:
                compiled_candidates.append(path)
    if compiled_candidates:
        prime_javap_cache(compiled_candidates)
        for path in compiled_candidates:
            try:
                compiled_registration_texts[path] = _run_javap(path)
            except Exception:
                continue
    for javap_text in compiled_registration_texts.values():
        try:
            for method in _parse_methods(javap_text):
                instructions = method.instructions
                for index, (_offset, operation, comment) in enumerate(instructions):
                    if not operation.startswith("invoke"):
                        continue
                    info = _method_info(comment)
                    called = info[1] if info else ""
                    if called not in {"registerItemRenderer", "registerItem", "registerBlock", "register"}:
                        continue
                    source_field = ""
                    registry_name = ""
                    for _o, op, prior_comment in reversed(instructions[max(0, index - 24):index]):
                        base = op.split()[0]
                        if not source_field and base == "getstatic":
                            source_field = field_name_from_comment(prior_comment)
                        if not registry_name and base.startswith("ldc"):
                            value = _push_constant(op, prior_comment)
                            if isinstance(value, str) and _looks_like_registry_path(value):
                                registry_name = _registry_path(value)
                        if source_field and registry_name:
                            break
                    if source_field and registry_name:
                        field_registries.setdefault(source_field, set()).add(registry_name)
        except Exception:
            continue
    combined = "\n".join(combined_parts)
    result: dict[str, dict[str, Any]] = {}
    # NeoForge/modernized legacy client handlers commonly register custom held item
    # renderers through wrapTeisrHandItem(...). Parse each call as a balanced Java
    # call so multi-line lambdas do not accidentally bind to a later ::new token.
    for _call_name, raw_args, _offset in _iter_java_calls(combined, {"wrapTeisrHandItem"}):
        try:
            args = _split_java_arguments(raw_args)
        except Exception:
            continue
        if len(args) < 2:
            continue
        item_match = re.search(r'\bitem\s*\(\s*["\']([^"\']+)["\']\s*\)', args[1])
        if not item_match:
            continue
        registry = _compact_registry(item_match.group(1))
        renderer_simple = ""
        model_class = ""
        renderer_text = ""
        renderer_ref = re.search(r'\b([A-Za-z_$][\w$]*)::new\b', args[-1])
        if renderer_ref:
            renderer_simple = _simple(renderer_ref.group(1))
            renderer_text = texts_by_class.get(renderer_simple, "")
            model_matches = re.findall(r'\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(', renderer_text)
            model_class = model_matches[0] if model_matches else ""
        else:
            model_var = re.search(r'\b([A-Za-z_$][\w$]*)::render\b', args[-1])
            if model_var:
                var_name = model_var.group(1)
                # Resolve the model variable declaration from the complete source corpus.
                decl = re.search(r'\b((?:Model|model)[A-Za-z0-9_$]+)\s+' + re.escape(var_name) + r'\s*=\s*new\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(', combined)
                if decl:
                    model_class = decl.group(2)
                    renderer_simple = "StaticBigWeaponItemStackRenderer"
                    renderer_text = texts_by_class.get(renderer_simple, "")
        if not registry or not model_class:
            continue
        texture_ref = ""
        local_text = args[-1] + "\n" + renderer_text
        tex_match = re.search(r'\btexture\s*\(\s*["\']([^"\']+\.png)["\']\s*\)', local_text, re.IGNORECASE)
        if tex_match:
            texture_ref = Path(tex_match.group(1).replace("\\", "/")).stem
        else:
            refs = _declared_texture_refs(renderer_text)
            texture_ref = refs[0] if refs else ""
        result[registry] = {
            "renderer_class": renderer_simple,
            "model_class": model_class,
            "texture_ref": texture_ref,
            "display": _legacy_renderer_display(renderer_text),
            "source": "wrapTeisrHandItem",
        }
    pattern = re.compile(
        r"(?:MinecraftForgeClient\s*\.\s*)?registerItemRenderer\s*\(\s*"
        r"(?:[A-Za-z_$][\w$]*\.)*([A-Za-z_$][\w$]*)\s*,\s*new\s+"
        r"([A-Za-z_$][\w.$]*)\s*\(",
        re.DOTALL,
    )
    for match in pattern.finditer(combined):
        field_name, renderer_name = match.groups()
        renderer_simple = _simple(renderer_name)
        renderer_text = texts_by_class.get(renderer_simple, "")
        if not renderer_text:
            continue
        model_matches = re.findall(r"\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(", renderer_text)
        model_class = model_matches[0] if model_matches else ""
        texture_refs = _declared_texture_refs(renderer_text)
        texture_ref = texture_refs[0] if texture_refs else ""
        for registry in field_registries.get(field_name, set()):
            result[_compact_registry(registry)] = {
                "renderer_class": renderer_simple,
                "model_class": model_class,
                "texture_ref": texture_ref,
                "display": _legacy_renderer_display(renderer_text),
                "source": "decompiled",
            }
    # Compiled-only fallback: inspect only class files that contain the renderer
    # registration token. This avoids invoking javap for every class in a large mod.
    compiled_files: list[Path] = []
    class_index: dict[str, Path] = {}
    for root in roots:
        for path in root.rglob("*.class"):
            class_index.setdefault(_simple(path.stem), path)
            try:
                raw = path.read_bytes()
            except OSError:
                continue
            if b"registerItemRenderer" in raw or b"MinecraftForgeClient" in raw:
                compiled_files.append(path)
    if compiled_files:
        prime_javap_cache(compiled_files)
    for registration_class in compiled_files:
        try:
            javap_text = _run_javap(registration_class)
            for method in _parse_methods(javap_text):
                instructions = method.instructions
                for index, (_offset, operation, comment) in enumerate(instructions):
                    if not operation.startswith("invoke") or (_method_info(comment) or ("", ""))[1] != "registerItemRenderer":
                        continue
                    source_field = ""
                    renderer_class = ""
                    for _o, op, prior_comment in reversed(instructions[max(0, index - 20):index]):
                        base = op.split()[0]
                        if not source_field and base == "getstatic":
                            source_field = field_name_from_comment(prior_comment)
                        if not renderer_class and base == "new":
                            renderer_class = _simple(_class_from_comment(prior_comment))
                        if source_field and renderer_class:
                            break
                    if not source_field or not renderer_class:
                        continue
                    renderer_path = class_index.get(renderer_class)
                    if renderer_path is None:
                        continue
                    renderer_text = _run_javap(renderer_path)
                    model_class = ""
                    model_match = re.search(r"new\s+#\d+\s+//\s+class\s+[^/\s]+(?:/[^/\s]+)*/((?:Model|model)[A-Za-z0-9_$]+)", renderer_text)
                    if model_match:
                        model_class = model_match.group(1)
                    if not model_class:
                        model_match = re.search(r"//\s+class\s+[^/\s]+(?:/[^/\s]+)*/((?:Model|model)[A-Za-z0-9_$]+)", renderer_text)
                        if model_match:
                            model_class = model_match.group(1)
                    texture_ref = ""
                    for string_match in re.finditer(r"//\s+String\s+([^\n]+)", renderer_text):
                        value = string_match.group(1).strip()
                        if value.lower().endswith((".png", ".jpg", ".jpeg")) or "/textures/" in value.lower():
                            texture_ref = Path(value.replace("\\", "/")).stem
                            break
                    for registry in field_registries.get(source_field, set()):
                        key = _compact_registry(registry)
                        result.setdefault(key, {
                            "renderer_class": renderer_class,
                            "model_class": model_class,
                            "texture_ref": texture_ref,
                            "display": {},
                            "source": "compiled",
                        })
        except Exception:
            continue
    return result
def _find_item_model_json(roots: list[Path], registry: str) -> Optional[Path]:
    compact = _compact_registry(registry)
    exact: list[Path] = []
    fallback: list[Path] = []
    for root in roots:
        for path in root.rglob("*.json"):
            lower = path.as_posix().lower()
            if "/models/item/" not in lower:
                continue
            if _compact_registry(path.stem) == compact:
                exact.append(path)
            elif compact and compact in _compact_registry(path.stem):
                fallback.append(path)
    candidates = sorted(exact) or sorted(fallback)
    for path in candidates:
        try:
            payload = json.loads(path.read_text(encoding="utf-8-sig", errors="replace"))
        except Exception:
            continue
        if not isinstance(payload, dict):
            continue
        # Standard generated/handheld item models are not custom 3D item models.
        # A non-standard parent, explicit elements, or explicit display transforms
        # indicates that preserving the source item model as a custom MCreator model
        # is more faithful than falling back to a 16x16 generated item.
        parent = str(payload.get("parent", "")).strip().lower()
        standard_parents = {"item/generated", "item/handheld", "builtin/generated", "builtin/handheld"}
        if payload.get("elements") or payload.get("display") or (parent and parent not in standard_parents):
            return path
    return None
def _rewrite_model_namespace(value: Any, original_modids: set[str], target_modid: str) -> Any:
    if isinstance(value, dict):
        return {key: _rewrite_model_namespace(item, original_modids, target_modid) for key, item in value.items()}
    if isinstance(value, list):
        return [_rewrite_model_namespace(item, original_modids, target_modid) for item in value]
    if isinstance(value, str) and ":" in value:
        namespace, rest = value.split(":", 1)
        if namespace.lower() in original_modids and namespace.lower() != target_modid.lower():
            return f"{target_modid}:{rest}"
    return value
def _model_texture_stem(value: str) -> str:
    """Normalize a model texture reference into MCreator's workspace texture name."""
    value = str(value or "").strip().replace("\\", "/")
    if ":" in value:
        value = value.split(":", 1)[1]
    value = value.lstrip("/")
    if value.lower().startswith("textures/"):
        value = value[9:]
    value = value.rsplit("/", 1)[-1]
    if value.lower().endswith(".png"):
        value = value[:-4]
    return value
def _model_texture_sources(source: Path, texture_ref: str) -> list[Path]:
    """Locate a texture referenced by an item/custom model JSON."""
    ref = str(texture_ref or "").strip().replace("\\", "/")
    if not ref:
        return []
    if ":" in ref:
        _namespace, ref = ref.split(":", 1)
    ref = ref.lstrip("/")
    if ref.startswith("textures/"):
        ref = ref[9:]
    if ref.lower().endswith(".png"):
        ref = ref[:-4]
    candidates: list[Path] = []
    stem_path = Path(ref)
    # Walk upward until an assets directory is found, then resolve the exact
    # resource path alongside models/. This handles both extracted jars and
    # decompiled source trees without relying on a particular mod layout.
    cursor = source.parent
    while cursor != cursor.parent:
        assets = cursor / "assets"
        if assets.is_dir():
            for namespace_dir in sorted(assets.iterdir()):
                if not namespace_dir.is_dir():
                    continue
                for base in (namespace_dir / "textures", assets):
                    candidate = base / f"{ref}.png"
                    if candidate.exists():
                        candidates.append(candidate)
            break
        cursor = cursor.parent
    # Exact filename fallback for layouts where resources are mounted outside
    # an assets directory. Keep this deliberately identity-based.
    target_name = Path(ref).name.lower() + ".png"
    for root in [source.parent, *source.parents[:3]]:
        if not root.exists():
            continue
        for candidate in root.rglob("*.png"):
            if candidate.name.lower() == target_name:
                candidates.append(candidate)
    unique: list[Path] = []
    seen: set[str] = set()
    for candidate in candidates:
        key = str(candidate.resolve())
        if key not in seen:
            seen.add(key)
            unique.append(candidate)
    return unique
def _write_custom_model_texture_mapping(
    workspace_root: Path,
    slug: str,
    texture_map: dict[str, str],
) -> str:
    """Write the MCreator .json.textures sidecar used by custom models.
    MCreator's custom-model importer uses this file to map model texture slots
    to workspace texture resources. Without it, a model may load as an untextured
    cube or silently fall back to the default 16x16 item representation.
    """
    mapping = {
        "mappings": {
            "default": {
                "map": {str(key): str(value) for key, value in texture_map.items() if value},
                "name": "default",
            }
        }
    }
    path = workspace_root / "models" / f"{slug}.json.textures"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(mapping, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return str(path)
def _copy_custom_model_texture(
    source_candidates: list[Path],
    workspace_root: Path,
    modid: str,
    texture_stem: str,
    preferred_name: str = "",
) -> tuple[str, Optional[str]]:
    """Copy the texture consumed by a custom model into the workspace texture root."""
    target_root = workspace_root / "src" / "main" / "resources" / "assets" / modid / "textures"
    target_root.mkdir(parents=True, exist_ok=True)
    source = source_candidates[0] if source_candidates else None
    stem = _model_texture_stem(preferred_name or texture_stem) or "converted_model_texture"
    target = target_root / f"{stem}.png"
    if source is not None:
        if target.exists() and target.read_bytes() != source.read_bytes():
            stem = _snake(f"{stem}_{source.stem}")
            target = target_root / f"{stem}.png"
        _copy_texture_with_metadata(source, target)
        # MCreator item JSON/custom-model imports resolve model texture slots through
        # the block texture namespace. Keep the normal workspace texture too, but
        # always mirror the exact custom-model texture into textures/block/.
        block_root = target_root / "block"
        block_root.mkdir(parents=True, exist_ok=True)
        block_target = block_root / f"{stem}.png"
        if not block_target.exists() or block_target.read_bytes() != source.read_bytes():
            _copy_texture_with_metadata(source, block_target)
        return stem, str(target)
    return stem, None
def _copy_resource_item_model_as_custom(
    source: Path,
    workspace_root: Path,
    modid: str,
    registry: str,
) -> tuple[str, list[str]]:
    payload = json.loads(source.read_text(encoding="utf-8-sig", errors="replace"))
    original_modids: set[str] = set()
    parts = source.as_posix().split("/assets/")
    if len(parts) > 1:
        original_modids.add(parts[1].split("/", 1)[0].lower())
    payload = _rewrite_model_namespace(payload, original_modids, modid)
    # MCreator's working 1.21.1 separate-transforms models use the empty texture
    # slot for their custom base model. When a source custom JSON uses a single
    # numeric/layer slot, normalize it and its face references without changing
    # multi-texture models that genuinely require several named slots.
    textures = payload.get("textures") if isinstance(payload, dict) else None
    if isinstance(textures, dict) and "" not in textures and len(textures) == 1:
        old_key, old_value = next(iter(textures.items()))
        payload["textures"] = {"": old_value}
        def _rewrite_single_slot(value):
            if isinstance(value, dict):
                return {k: _rewrite_single_slot(v) for k, v in value.items()}
            if isinstance(value, list):
                return [_rewrite_single_slot(v) for v in value]
            if isinstance(value, str) and value == f"#{old_key}":
                return "#"
            return value
        payload = _rewrite_single_slot(payload)
    slug = _snake(registry) + "_held"
    outputs: list[str] = []
    # MCreator custom models need a workspace texture mapping sidecar. Preserve
    # the model's actual slot names (layer0, particle, numeric slots, etc.) and
    # resolve each referenced PNG into the workspace texture root.
    texture_mapping: dict[str, str] = {}
    textures = payload.get("textures") if isinstance(payload, dict) else None
    if isinstance(textures, dict):
        for key, value in textures.items():
            if not isinstance(value, str) or not value:
                continue
            stem = _model_texture_stem(value)
            if not stem:
                continue
            copied_stem, _ = _copy_custom_model_texture(
                _model_texture_sources(source, value), workspace_root, modid, stem, preferred_name=stem
            )
            texture_mapping[str(key)] = copied_stem
            # Keep the model JSON namespace clean, while using the actual
            # workspace texture name in the sidecar mapping.
            if str(key).isdigit():
                textures[key] = copied_stem
    for target in (
        workspace_root / "models" / f"{slug}.json",
        workspace_root / "src" / "main" / "resources" / "assets" / modid / "models" / "custom" / f"{slug}.json",
    ):
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        outputs.append(str(target))
    outputs.append(_write_custom_model_texture_mapping(workspace_root, slug, texture_mapping))
    return slug, outputs
def _write_neoforge_separate_transform_item_model(workspace_root: Path, modid: str, registry: str, custom_slug: str, icon_texture: str, base_texture: str = "") -> str:
    """Emit the NeoForge 1.21.1 item-model wrapper used by working MCreator custom-tool examples."""
    base_tex = _model_texture_stem(base_texture or icon_texture)
    icon_tex = _model_texture_stem(icon_texture)
    payload = {
        "loader": "neoforge:separate_transforms",
        "gui_light": "front",
        "base": {"parent": f"{modid}:custom/{custom_slug}", "textures": {"": f"{modid}:block/{base_tex or icon_tex}", "particle": "minecraft:item/diamond_sword"}},
        "perspectives": {
            "gui": {"parent": "item/generated", "textures": {"layer0": f"{modid}:item/{icon_tex}" if icon_tex else "minecraft:item/diamond_sword"}},
            "fixed": {"parent": "item/generated", "textures": {"layer0": f"{modid}:item/{icon_tex}" if icon_tex else "minecraft:item/diamond_sword"}},
            "ground": {"parent": "item/generated", "textures": {"layer0": f"{modid}:item/{icon_tex}" if icon_tex else "minecraft:item/diamond_sword"}},
        },
    }
    path=workspace_root/"src"/"main"/"resources"/"assets"/modid/"models"/"item"/f"{_snake(registry)}.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
    return str(path)

def _recipe_source_files(source_roots: list[Path]) -> list[Path]:
    """Collect legacy and modern recipe JSON resources from the input roots."""
    candidates: list[Path] = []
    seen: set[str] = set()
    for root in source_roots:
        if not root.exists():
            continue
        for path in root.rglob("*.json"):
            lower = path.as_posix().lower()
            if "/recipes/" not in lower:
                continue
            if not any(segment in lower for segment in ("/assets/", "/data/")):
                continue
            key = str(path.resolve()).lower()
            if key in seen:
                continue
            seen.add(key)
            candidates.append(path)
    return sorted(candidates, key=lambda p: (len(p.parts), p.as_posix().lower()))

_VANILLA_BLOCK_NAME_HINTS = {
    "air","stone","granite","diorite","andesite","deepslate","cobbled_deepslate","netherrack","end_stone",
    "dirt","coarse_dirt","rooted_dirt","grass_block","podzol","sand","red_sand","gravel","clay","snow","snow_block",
}
_VANILLA_BLOCK_SUFFIXES = (
    "_block","_ore","_log","_wood","_planks","_slab","_stairs","_wall","_fence","_fence_gate","_door",
    "_trapdoor","_button","_pressure_plate","_sign","_hanging_sign","_leaves","_sapling","_glass","_pane",
    "_wool","_terracotta","_concrete","_concrete_powder","_bricks","_stone","_cobblestone","_dirt","_sand",
    "_gravel","_ice","_coral","_coral_block","_coral_fan","_mushroom","_stem","_nylium","_wart","_torch",
    "_lantern","_rail","_bed","_banner","_head","_skull","_anvil","_table","_crop","_roots","_flower",
)
def _looks_like_vanilla_block_path(path: str) -> bool:
    p=str(path or "").lower().replace("-","_")
    return p in _VANILLA_BLOCK_NAME_HINTS or p.endswith(_VANILLA_BLOCK_SUFFIXES)

def _recipe_ref(value: Any, registry_to_element: dict[str, str], source_modid: str, target_modid: str) -> str:
    """Convert a Minecraft item/block id into an MCreator MItemBlock token."""
    if not isinstance(value, str):
        return ""
    raw = value.strip()
    if not raw:
        return ""
    if raw.startswith("#"):
        # MCreator recipe slots use item references, not raw tag JSON. Preserve a
        # useful vanilla-ish reference for tags instead of generating an invalid
        # CUSTOM element reference. Tag-heavy recipes remain in the copied data pack.
        return raw
    if ":" in raw:
        namespace, path = raw.split(":", 1)
    else:
        namespace, path = "minecraft", raw
    compact = _compact_registry(path)
    if namespace.lower() == str(source_modid).lower():
        element_name = registry_to_element.get(compact)
        if element_name:
            return f"CUSTOM:{element_name}"
        return ""
    if namespace.lower() == "minecraft":
        # MCreator's MItemBlock mapper accepts the legacy Items.X form used by
        # existing workspaces. Keep the path exact and uppercase only the enum
        # part rather than inventing a CUSTOM reference.
        enum_name = path.upper().replace("/", "_").replace("-", "_")
        return ("Blocks." if _looks_like_vanilla_block_path(path) else "Items.") + enum_name
    return raw


def _recipe_output_ref(recipe: dict[str, Any], registry_to_element: dict[str, str], source_modid: str) -> tuple[str, int]:
    raw = recipe.get("result", {})
    count = 1
    value = raw
    if isinstance(raw, dict):
        value = raw.get("item", raw.get("id", ""))
        try:
            count = max(1, int(raw.get("count", 1)))
        except Exception:
            count = 1
    return _recipe_ref(value, registry_to_element, source_modid, source_modid), count


def _recipe_ingredient_ref(ingredient: Any, registry_to_element: dict[str, str], source_modid: str) -> str:
    if isinstance(ingredient, str):
        return _recipe_ref(ingredient, registry_to_element, source_modid, source_modid)
    if isinstance(ingredient, dict):
        if "item" in ingredient:
            return _recipe_ref(ingredient.get("item"), registry_to_element, source_modid, source_modid)
        values = ingredient.get("items")
        if isinstance(values, list):
            refs = [_recipe_ref(v, registry_to_element, source_modid, source_modid) for v in values]
            refs = [v for v in refs if v]
            return refs[0] if refs else ""
        if "tag" in ingredient:
            return "#" + str(ingredient.get("tag") or "")
    if isinstance(ingredient, list):
        refs = [_recipe_ingredient_ref(v, registry_to_element, source_modid) for v in ingredient]
        refs = [v for v in refs if v]
        return refs[0] if refs else ""
    return ""


def _recipe_mcreator_definition(payload: dict[str, Any], recipe_name: str, registry_to_element: dict[str, str], source_modid: str) -> tuple[Optional[dict[str, Any]], list[str]]:
    warnings: list[str] = []
    rtype = str(payload.get("type", "minecraft:crafting_shaped"))
    result = payload.get("result", {})
    output_ref, output_count = _recipe_output_ref(payload, registry_to_element, source_modid)
    if not output_ref:
        return None, [f"Recipe {recipe_name}: result could not be mapped to an MCreator item/block reference."]
    definition: dict[str, Any] = {
        "recipeType": "Crafting",
        "recipeRetstackSize": output_count,
        "group": str(payload.get("group", "")),
        "unlockingItems": [],
        "cookingBookCategory": "MISC",
        "xpReward": 0.0,
        "cookingTime": 200,
        "craftingBookCategory": "MISC",
        "recipeShapeless": False,
        "recipeSlots": ["" for _ in range(9)],
        "recipeReturnStack": output_ref,
        "name": recipe_name,
        "namespace": "mod",
    }
    if rtype.endswith("crafting_shaped") or rtype == "minecraft:crafting_shaped":
        pattern = payload.get("pattern", [])
        key = payload.get("key", {})
        if not isinstance(pattern, list) or len(pattern) > 3:
            return None, [f"Recipe {recipe_name}: shaped pattern larger than 3x3 is unsupported by MCreator."]
        for row_index, row in enumerate(pattern):
            if not isinstance(row, str):
                continue
            for col_index, symbol in enumerate(row[:3]):
                if row_index * 3 + col_index >= 9 or symbol == " ":
                    continue
                ref = _recipe_ingredient_ref(key.get(symbol), registry_to_element, source_modid)
                definition["recipeSlots"][row_index * 3 + col_index] = ref
        return definition, warnings
    if rtype.endswith("crafting_shapeless") or rtype == "minecraft:crafting_shapeless":
        definition["recipeShapeless"] = True
        ingredients = payload.get("ingredients", [])
        if isinstance(ingredients, list):
            for index, ingredient in enumerate(ingredients[:9]):
                definition["recipeSlots"][index] = _recipe_ingredient_ref(ingredient, registry_to_element, source_modid)
        return definition, warnings
    cooking = {
        "minecraft:smelting": "Smelting",
        "minecraft:blasting": "Blasting",
        "minecraft:smoking": "Smoking",
        "minecraft:campfire_cooking": "Campfire cooking",
        "minecraft:stonecutting": "Stone cutting",
        "minecraft:smithing": "Smithing",
        "minecraft:smithing_transform": "Smithing",
        "minecraft:smithing_trim": "Smithing",
    }
    if rtype in cooking:
        definition["recipeType"] = cooking[rtype]
        ingredient = _recipe_ingredient_ref(payload.get("ingredient"), registry_to_element, source_modid)
        if definition["recipeType"] == "Smelting":
            definition["smeltingInputStack"] = ingredient
            definition["smeltingReturnStack"] = output_ref
        elif definition["recipeType"] == "Blasting":
            definition["blastingInputStack"] = ingredient
            definition["blastingReturnStack"] = output_ref
        elif definition["recipeType"] == "Smoking":
            definition["smokingInputStack"] = ingredient
            definition["smokingReturnStack"] = output_ref
        elif definition["recipeType"] == "Campfire cooking":
            definition["campfireCookingInputStack"] = ingredient
            definition["campfireCookingReturnStack"] = output_ref
        elif definition["recipeType"] == "Stone cutting":
            definition["stoneCuttingInputStack"] = ingredient
            definition["stoneCuttingReturnStack"] = output_ref
        elif definition["recipeType"] == "Smithing":
            definition["smithingInputStack"] = _recipe_ingredient_ref(payload.get("addition"), registry_to_element, source_modid)
            definition["smithingInputAdditionStack"] = ingredient
            definition["smithingInputTemplateStack"] = _recipe_ingredient_ref(payload.get("template"), registry_to_element, source_modid)
            definition["smithingReturnStack"] = output_ref
        return definition, warnings
    return None, [f"Recipe {recipe_name}: recipe type '{rtype}' is not representable by MCreator's native recipe element."]


def _normalize_mcreator_item_ref(expr: str, source_modid: str = "") -> str:
    """Normalize Java-style item/block references to MCreator element references.

    Examples: Items.ITEMS.IRON_INGOT -> Items.IRON_INGOT;
    Blocks.BLOCKS.LAPIS_BLOCK -> Blocks.LAPIS_BLOCK;
    minecraft:block/oak_log -> minecraft:oak_log.
    """
    expr = str(expr or "").strip().strip('\"')
    if not expr:
        return ""
    if expr.startswith("minecraft:block/"):
        return "minecraft:" + expr.split("minecraft:block/", 1)[1]
    if expr.startswith("minecraft:item/"):
        return "minecraft:" + expr.split("minecraft:item/", 1)[1]
    # MCreator's Java-style recipe references are case-insensitive at the
    # parser boundary, but the serialized identifiers are not. Normalize both
    # ``Items.ITEMS.X``/``items.ITEMS.X`` and ``Blocks.BLOCKS.X`` forms to one
    # registry prefix and exactly one field namespace.
    m = re.match(r"^(items|Items)\.(.*)$", expr)
    if m:
        tail = m.group(2)
        while re.match(r"^(?:ITEMS|items)\.", tail):
            tail = tail.split(".", 1)[1]
        return "Items." + tail.upper().replace("-", "_")
    m = re.match(r"^(blocks|Blocks)\.(.*)$", expr)
    if m:
        tail = m.group(2)
        while re.match(r"^(?:BLOCKS|blocks)\.", tail):
            tail = tail.split(".", 1)[1]
        return "Blocks." + tail.upper().replace("-", "_")
    return expr


def _java_recipe_item_ref(expr: str, field_map: dict[str, set[str]], registry_to_element: dict[str, str], source_modid: str) -> str:
    expr=str(expr or '').strip()
    # Normalize common nested registry holders before field-name matching.
    expr = _normalize_mcreator_item_ref(expr, source_modid)
    # unwrap the common constructors and Ingredient wrappers
    m=re.search(r'new\s+ItemStack\s*\(\s*([^,)]+)', expr)
    if m:
        expr=m.group(1).strip()
    m=re.search(r'Ingredient\.of\s*\(\s*([^,)]+)', expr)
    if m:
        expr=m.group(1).strip()
    # Constructors/wrappers may reveal the Java registry reference only after
    # unwrapping; normalize a second time so `items.ITEMS.foo` cannot survive.
    expr = _normalize_mcreator_item_ref(expr, source_modid)
    m=re.search(r'createVanillaSpawnEgg\s*\(\s*"([^"]+)"', expr)
    if m:
        path=m.group(1).lower()
        return 'Items.' + path.upper().replace('-', '_') + '_SPAWN_EGG'
    field=re.fullmatch(r'(?:[A-Za-z_$][\w$]*\.)*([A-Za-z_$][\w$]*)', expr)
    if field:
        name=field.group(1)
        if name in {'AIR'}:
            return 'Items.AIR'
        if name in {'Items','Blocks'}:
            return ''
        values=field_map.get(name, set())
        if values:
            # Prefer a source registry that has actually become an imported element.
            for raw in values:
                key=_compact_registry(raw)
                if key in registry_to_element:
                    return f"CUSTOM:{registry_to_element[key]}"
            return f"CUSTOM:{registry_to_element.get(_compact_registry(next(iter(values))), next(iter(values)))}"
        if expr.startswith('Items.'):
            return 'Items.' + name.upper()
        if expr.startswith('Blocks.'):
            return 'Blocks.' + name.upper() + '#0'
    normalized = _normalize_mcreator_item_ref(expr, source_modid)
    if normalized.startswith('Items.') or normalized.startswith('Blocks.'):
        return normalized
    return ''

def _java_recipe_payloads(source_roots: list[Path], source_modid: str, registry_to_element: dict[str, str]) -> list[tuple[str, dict[str, Any], list[str]]]:
    """Recover recipes registered through Java helper calls when no recipe JSON exists."""
    try:
        from mcreator_content_export import _source_field_registered_names, _iter_java_calls, _split_java_arguments
    except Exception:
        return []
    field_map: dict[str,set[str]]={}
    texts=[]
    for root in source_roots:
        if not Path(root).exists():
            continue
        try:
            fmap=_source_field_registered_names(Path(root))
            for k,v in fmap.items(): field_map.setdefault(k,set()).update(v)
        except Exception:
            pass
        for path in Path(root).rglob('*.java'):
            try: texts.append(path.read_text(encoding='utf-8-sig', errors='replace'))
            except OSError: pass
    combined='\n'.join(texts)
    out=[]
    seen=set()
    for call_name, raw_args, _offset in _iter_java_calls(combined, {'addShapelessRecipe','addShapedRecipe'}):
        args=_split_java_arguments(raw_args)
        if len(args)<4: continue
        nm=re.search(r'cpId\s*\(\s*"([^"]+)"', args[0]) or re.search(r'"([^"]+)"', args[0])
        name=_snake(nm.group(1)) if nm else ''
        if not name: continue
        if name in seen: continue
        seen.add(name)
        output=_java_recipe_item_ref(args[2],field_map,registry_to_element,source_modid)
        if not output: continue
        payload={'type':'minecraft:crafting_shapeless','result':{'item':output,'count':1}}
        warnings=[]
        if call_name=='addShapelessRecipe':
            ingredients=[]
            for arg in args[3:]:
                ref=_java_recipe_item_ref(arg,field_map,registry_to_element,source_modid)
                if ref: ingredients.append({'item':ref})
            if not ingredients: continue
            payload['ingredients']=ingredients
        else:
            patterns=[]; key=[]; i=3
            while i < len(args) and re.fullmatch(r'\s*"[^"]{1,3}"\s*', args[i] or ''):
                patterns.append(args[i].strip().strip('"')); i+=1
            while i+1 < len(args):
                token=args[i].strip(); value=args[i+1].strip(); i+=2
                chmatch=re.search(r"'(.+?)'", token)
                if not chmatch: continue
                ref=_java_recipe_item_ref(value,field_map,registry_to_element,source_modid)
                if ref: key.append((chmatch.group(1), {'item':ref}))
            if not patterns or not key: continue
            payload={'type':'minecraft:crafting_shaped','pattern':patterns,'key':{k:v for k,v in key},'result':{'item':output,'count':1}}
        out.append((name,payload,warnings))
    return out

def _emit_mcreator_recipe_elements(source_roots: list[Path], output_elements: Path, workspace: dict[str, Any], source_modid: str, registry_to_element: dict[str, str], imported_fv: int = 89) -> tuple[list[MCreatorContentExport], list[str]]:
    exports: list[MCreatorContentExport] = []
    warnings: list[str] = []
    used_names, used_registries = _element_names(workspace)
    # Clean legacy sources such as Chaos Persists v0.4 register recipes in Java helper calls
    # rather than data/recipes JSON. Recover those into native MCreator recipe elements.
    for recipe_name, payload, local_warnings in _java_recipe_payloads(source_roots, source_modid, registry_to_element):
        recipe_registry = recipe_name
        element_name = _unique_name(_pascal(recipe_name) + "Recipe", used_names)
        definition, warnings2 = _recipe_mcreator_definition(payload, recipe_name, registry_to_element, source_modid)
        if not definition:
            warnings.extend(local_warnings); warnings.extend(warnings2); continue
        element={"_fv": min(89, max(0,int(imported_fv or 89))), "_type":"recipe", "definition":definition}
        output_elements.mkdir(parents=True, exist_ok=True)
        output=output_elements/f"{element_name}.mod.json"
        output.write_text(json.dumps(element,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
        _upsert_workspace_element(workspace,None,element_name,"recipe",recipe_registry,source_modid,recipe_registry,"Recipes")
        exports.append(MCreatorContentExport(element_name,recipe_registry,"recipe","Java recipe registration","created",local_warnings+warnings2))
    for source in _recipe_source_files(source_roots):
        payload = _read_json(source)
        if not payload:
            continue
        # The source recipe path supplies a stable registry-like element name.
        parts = list(source.parts)
        lower = [p.lower() for p in parts]
        try:
            recipe_index = next(i for i, part in enumerate(lower) if part == "recipes")
        except StopIteration:
            continue
        relative = Path(*parts[recipe_index + 1:])
        if not relative.name:
            continue
        recipe_registry = _snake(relative.stem)
        element_name = _unique_name(_pascal(relative.stem) + "Recipe", used_names)
        definition, local_warnings = _recipe_mcreator_definition(payload, recipe_registry, registry_to_element, source_modid)
        if not definition:
            warnings.extend(local_warnings)
            continue
        element = {"_fv": min(89, max(0, int(imported_fv or 89))), "_type": "recipe", "definition": definition}
        output_elements.mkdir(parents=True, exist_ok=True)
        output = output_elements / f"{element_name}.mod.json"
        output.write_text(json.dumps(element, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        entry = _upsert_workspace_element(workspace, None, element_name, "recipe", recipe_registry, source_modid, recipe_registry, "Recipes")
        entry["metadata"]["files"] = []
        exports.append(MCreatorContentExport(element_name, recipe_registry, "recipe", str(source), "created", local_warnings))
    return exports, warnings

def _copy_recipe_resources(source_roots: list[Path], workspace_root: Path, modid: str) -> list[str]:
    """Copy recoverable recipe JSON files into the workspace data pack tree."""
    outputs: list[str] = []
    target_root = workspace_root / "src" / "main" / "resources" / "data" / modid / "recipes"
    for source in _recipe_source_files(source_roots):
        parts = source.parts
        lower_parts = [part.lower() for part in parts]
        try:
            recipe_index = next(i for i, part in enumerate(lower_parts) if part == "recipes")
        except StopIteration:
            continue
        relative = Path(*parts[recipe_index + 1 :])
        if not relative.parts:
            continue
        target = target_root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        try:
            payload = json.loads(source.read_text(encoding="utf-8-sig", errors="replace"))
        except Exception:
            continue
        target.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        outputs.append(str(target))
    return outputs
def _box_uv_faces(cube: Any, texture_width: int, texture_height: int) -> dict[str, dict[str, Any]]:
    u, v = float(cube.uv_x), float(cube.uv_y)
    dx, dy, dz = float(cube.dx), float(cube.dy), float(cube.dz)
    sx = 16.0 / max(1, texture_width)
    sy = 16.0 / max(1, texture_height)
    def uv(x1: float, y1: float, x2: float, y2: float) -> list[float]:
        return [round(x1 * sx, 6), round(y1 * sy, 6), round(x2 * sx, 6), round(y2 * sy, 6)]
    return {
        "west": {"uv": uv(u, v + dz, u + dz, v + dz + dy), "texture": "#"},
        "north": {"uv": uv(u + dz, v + dz, u + dz + dx, v + dz + dy), "texture": "#"},
        "east": {"uv": uv(u + dz + dx, v + dz, u + 2 * dz + dx, v + dz + dy), "texture": "#"},
        "south": {"uv": uv(u + 2 * dz + dx, v + dz, u + 2 * dz + 2 * dx, v + dz + dy), "texture": "#"},
        "up": {"uv": uv(u + dz, v, u + dz + dx, v + dz), "texture": "#"},
        "down": {"uv": uv(u + dz + dx, v, u + 2 * dz + dx, v + dz), "texture": "#"},
    }
def _legacy_item_model_part_transform(part: Any) -> tuple[float, float, float, float, float, float]:
    """Return a legacy ModelRenderer pivot in Java Block/Item-model space.
    Legacy 1.7.x item renderers use the ModelBase coordinate system where Y grows
    downwards from the model origin.  The Blockbench Java Block/Item format used
    by MCreator is centered around an 16x16x16 cube, so the legacy coordinates
    must be reflected and translated.  The offsets below match the convention
    used by MCreator's working legacy OreSpawn custom models:
      source cube (0, -6, 0, 1, 12, 1)
        -> Block/Item cube [7,-16,8] .. [8,-4,9]
      source pivot (0,0,0)
        -> Block/Item pivot [7.5,8,8.5]
    """
    # ParsedModel.offset is the original ModelRenderer pivot. `origin` is a
    # Blockbench/entity-space value and must not be reused for legacy item models.
    ox, oy, oz = getattr(part, "offset", (0.0, 0.0, 0.0))
    rx, ry, rz = getattr(part, "rotation", (0.0, 0.0, 0.0))
    # The Y reflection changes the handedness of every Euler rotation axis.
    return (
        float(ox) + 7.5,
        8.0 - float(oy),
        float(oz) + 8.5,
        -math.degrees(float(rx)),
        -math.degrees(float(ry)),
        -math.degrees(float(rz)),
    )
def _parsed_model_to_tool_json(
    model: Any,
    texture_name: str,
    display: Optional[dict[str, dict[str, list[float]]]] = None,
) -> dict[str, Any]:
    """Convert a legacy ModelRenderer model into MCreator's custom item model JSON.
    This is intentionally separate from entity-model conversion. Legacy item
    renderers use a different coordinate origin and are normally consumed as a
    flat Java Block/Item model by MCreator's Tool 3D Model support.
    """
    elements: list[dict[str, Any]] = []
    for part in model.parts:
        gx, gy, gz, rx, ry, rz = _legacy_item_model_part_transform(part)
        rotations = [("x", rx), ("y", ry), ("z", rz)]
        nonzero = [item for item in rotations if abs(item[1]) > 1e-6]
        axis, angle = max(rotations, key=lambda item: abs(item[1])) if nonzero else ("z", 0.0)
        part_ox, part_oy, part_oz = (float(v) for v in getattr(part, "offset", (0.0, 0.0, 0.0)))
        for cube in part.cubes:
            # ModelRenderer cube coordinates are relative to the renderer's local
            # pivot.  The previous exporter converted the cube but forgot to apply
            # the ModelRenderer rotationPoint, which is especially visible in legacy
            # held models such as Slice and the Royal Guardian weapons.
            #
            # For parentless parts this exactly matches MCreator's known-good legacy
            # item-model convention.  Parent offsets are accumulated below when a
            # hierarchy is available; rotations remain attached to the part pivot.
            from_x = float(cube.x) + part_ox + 7.0
            to_x = from_x + float(cube.dx)
            from_y = -(float(cube.y) + float(cube.dy) + part_oy) - 10.0
            to_y = -float(cube.y) - part_oy - 10.0
            from_z = float(cube.z) + part_oz + 8.0
            to_z = from_z + float(cube.dz)
            element: dict[str, Any] = {
                "name": cube.name,
                "from": [round(from_x, 6), round(from_y, 6), round(from_z, 6)],
                "to": [round(to_x, 6), round(to_y, 6), round(to_z, 6)],
                "rotation": {
                    "angle": round(angle, 6),
                    "axis": axis,
                    "origin": [round(gx, 6), round(gy, 6), round(gz, 6)],
                },
                "faces": _box_uv_faces(cube, model.texture_width, model.texture_height),
            }
            if getattr(cube, "mirror", False):
                element["mirror_uv"] = True
            if abs(float(getattr(cube, "inflate", 0.0))) > 1e-6:
                element["inflate"] = round(float(cube.inflate), 6)
            elements.append(element)
    return {
        # This is the Blockbench/MCreator custom-model format used by the
        # working NeoForge 1.21.1 example workspace. The empty texture slot is
        # important: the neoforge:separate_transforms item wrapper binds its
        # base model to this slot.
        "format_version": "1.21.11",
        "credit": "Converted by MojModelBridge",
        "texture_size": [model.texture_width, model.texture_height],
        "textures": {"": texture_name},
        "elements": elements,
        "display": display or {
            "thirdperson_righthand": {"translation": [0, 4, 1]},
            "thirdperson_lefthand": {"translation": [0, 4, 1]},
            "firstperson_righthand": {"translation": [1, 3, -2]},
            "firstperson_lefthand": {"translation": [1, 3, -2]},
        },
    }
def _export_legacy_tool_model(
    graph: LegacyModGraph,
    model_class: str,
    texture_name: str,
    workspace_root: Path,
    modid: str,
    registry: str,
    display: Optional[dict[str, dict[str, list[float]]]] = None,
) -> tuple[Optional[str], list[str], list[str]]:
    warnings: list[str] = []
    source = graph.class_files.get(model_class) or graph.java_files.get(model_class)
    if source is None:
        return None, [], [f"Held renderer referenced model {model_class}, but its source was not found."]
    try:
        from model_bridge import Cube, Part, ParsedModel, parse_java_model, parse_legacy_java_model
        if source.suffix.lower() == ".java":
            text = source.read_text(encoding="utf-8-sig", errors="replace")
            try:
                model = parse_legacy_java_model(text, source.name)
                if model.cube_count <= 0:
                    model = parse_java_model(text, source.name)
            except Exception:
                model = parse_java_model(text, source.name)
        else:
            from legacy_bridge import parse_legacy_class
            model = parse_legacy_class(source, Cube, Part, ParsedModel, resource_roots=graph.roots)
        if model.cube_count <= 0:
            raise RuntimeError("model contains zero cubes")
        payload = _parsed_model_to_tool_json(model, texture_name, display=display)
        slug = _snake(registry) + "_held"
        outputs: list[str] = []
        # The MCreator workspace model is the authoritative custom-model source.
        # Do NOT also emit the same Blockbench model as a vanilla resource-model JSON:
        # vanilla ModelBakery only permits 0..16 cube bounds and limited item rotations,
        # while MCreator's custom-model importer supports the larger/transformed geometry.
        target = workspace_root / "models" / f"{slug}.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        outputs.append(str(target))
        # Legacy IItemRenderer texture references often point to a full-size
        # entity/item texture rather than the 16x16 inventory icon. Preserve it
        # as a workspace-level texture and connect it through the MCreator sidecar.
        texture_stem = _model_texture_stem(texture_name)
        texture_sources: list[Path] = []
        for root in graph.roots:
            if not root.exists():
                continue
            for candidate in root.rglob("*.png"):
                if _normalise_texture_stem(candidate.stem) == _normalise_texture_stem(texture_stem):
                    texture_sources.append(candidate)
        copied_stem, copied_texture = _copy_custom_model_texture(
            sorted(texture_sources, key=lambda p: str(p).lower()),
            workspace_root,
            modid,
            texture_stem,
            preferred_name=texture_stem,
        )
        if copied_texture is None:
            warnings.append(f"Held model texture '{texture_stem}.png' was not found in the supplied source/resources; the custom model mapping was still generated.")
        else:
            payload["textures"] = {"": copied_stem}
            # Generated held models use the empty Blockbench texture slot.
            # Rewrite generated face references if the pre-copy payload used #1.
            def _rewrite_default_slot(value):
                if isinstance(value, dict):
                    return {k: _rewrite_default_slot(v) for k, v in value.items()}
                if isinstance(value, list):
                    return [_rewrite_default_slot(v) for v in value]
                if isinstance(value, str) and value == "#1":
                    return "#"
                return value
            payload = _rewrite_default_slot(payload)
            # Rewrite the authoritative MCreator workspace model after resolving the
            # actual workspace texture name.
            target = workspace_root / "models" / f"{slug}.json"
            target.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
            texture_stem = copied_stem
        outputs.append(_write_custom_model_texture_mapping(workspace_root, slug, {"": texture_stem}))
        multi_axis = any(sum(abs(math.degrees(value)) > 1e-5 for value in part.rotation) > 1 for part in model.parts)
        if multi_axis:
            warnings.append("Held model contains multi-axis part rotations; MCreator custom item JSON supports one rotation axis per cube, so the dominant axis was preserved.")
        if any(part.parent is not None for part in model.parts):
            warnings.append("Held model contains nested ModelRenderer parts; legacy item-model output is flattened into custom elements while preserving each part's local pivot and dominant rotation.")
        return slug, outputs, warnings
    except Exception as exc:
        return None, [], [f"Held model extraction failed for {model_class}: {exc}"]
def _source_text_by_class(root: Path) -> dict[str, list[str]]:
    result: dict[str, list[str]] = {}
    for path in sorted(Path(root).rglob("*.java")):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        result.setdefault(path.stem, []).append(text)
    return result
def _infer_projectile_item_bindings(
    graph: LegacyModGraph,
    exports: list[MCreatorContentExport],
) -> dict[str, MCreatorContentExport]:
    """Match projectile elements to their source item/tool elements.
    The strongest signal is a registry field referenced by the projectile class
    itself (for example the item returned when an arrow is picked up).  If that
    is unavailable, use an item/tool class that directly constructs the
    projectile, which covers throwable staffs and custom bows.
    """
    item_exports = [item for item in exports if item.element_type in {"item", "tool"}]
    projectile_exports = [item for item in exports if item.element_type == "projectile"]
    if not item_exports or not projectile_exports:
        return {}
    java_texts: dict[str, list[str]] = {}
    field_registries: dict[str, set[str]] = {}
    for source_root in graph.roots:
        for name, texts in _source_text_by_class(source_root).items():
            java_texts.setdefault(name, []).extend(texts)
        for name, values in _source_field_registered_names(source_root).items():
            field_registries.setdefault(name, set()).update(values)
    items_by_compact_registry: dict[str, list[MCreatorContentExport]] = {}
    items_by_source_class: dict[str, list[MCreatorContentExport]] = {}
    for item in item_exports:
        items_by_compact_registry.setdefault(_compact_registry(item.registry_name), []).append(item)
        items_by_source_class.setdefault(Path(item.source_class).stem, []).append(item)
    bindings: dict[str, MCreatorContentExport] = {}
    for projectile in projectile_exports:
        projectile_class = Path(projectile.source_class).stem
        projectile_text = "\n".join(java_texts.get(projectile_class, []))
        scored: list[tuple[float, MCreatorContentExport]] = []
        # A projectile's own source may reference the exact item used for pickup,
        # drops, or dispenser handling. This is the most authoritative binding.
        for field_name, raw_names in field_registries.items():
            if not projectile_text or not re.search(rf"\b{re.escape(field_name)}\b", projectile_text):
                continue
            for raw_name in raw_names:
                compact = _compact_registry(raw_name)
                for item in items_by_compact_registry.get(compact, []):
                    similarity = SequenceMatcher(None, _compact_registry(projectile.registry_name), compact).ratio()
                    scored.append((200.0 + similarity, item))
        # Otherwise identify item/tool classes whose source directly constructs
        # this projectile. This is suitable for throwable items and custom bows.
        constructor_pattern = re.compile(rf"\bnew\s+{re.escape(projectile_class)}\s*\(")
        for source_class, candidates in items_by_source_class.items():
            source_text = "\n".join(java_texts.get(source_class, []))
            if not source_text or not constructor_pattern.search(source_text):
                continue
            for item in candidates:
                similarity = SequenceMatcher(
                    None,
                    _compact_registry(projectile.registry_name),
                    _compact_registry(item.registry_name),
                ).ratio()
                scored.append((100.0 + similarity, item))
        if scored:
            scored.sort(key=lambda pair: (-pair[0], pair[1].element_name.lower()))
            bindings[projectile.element_name] = scored[0][1]
    return bindings
def _translated_name(language_names: dict[str, str], registry: str, fallback: str, aliases: Optional[Iterable[str]] = None) -> str:
    candidates = [str(registry)] + [str(value) for value in (aliases or [])]
    for candidate in candidates:
        value = language_names.get(candidate.lower()) or language_names.get(_compact_registry(candidate))
        if value:
            return value
    # Last-resort fuzzy registry matching for old mods where the source registration
    # and localization key differ (e.g. class name vs display-oriented registry).
    compact_targets = [_compact_registry(candidate) for candidate in candidates if _compact_registry(candidate)]
    if compact_targets:
        best: tuple[float, str] | None = None
        from difflib import SequenceMatcher
        for key, value in language_names.items():
            compact_key = _compact_registry(key)
            if not compact_key or not value or compact_key in {"item", "entity", "block", "name"}:
                continue
            score = max(SequenceMatcher(None, target, compact_key).ratio() for target in compact_targets)
            if any(target in compact_key or compact_key in target for target in compact_targets):
                score += 0.15
            if best is None or score > best[0]:
                best = (score, value)
        if best and best[0] >= 0.65:
            return best[1]
    return fallback
def _texture_file_index(roots: list[Path]) -> list[tuple[int, Path, str, str, str]]:
    """Build a reusable texture index for a fixed set of search roots.
    Entries are ``(root_priority, path, normalized_stem, compact_stem, lower_path)``.
    The expensive recursive filesystem walk happens once per root set.
    """
    cache_key = tuple(str(Path(root).expanduser().resolve()) for root in roots)
    cached = _TEXTURE_FILE_INDEX_CACHE.get(cache_key)
    if cached is not None:
        return cached  # type: ignore[return-value]
    seen: set[str] = set()
    files: list[tuple[int, Path, str, str, str]] = []
    for priority, root in enumerate(roots):
        root = Path(root)
        if not root.exists():
            continue
        try:
            iterator = root.rglob("*.png")
        except OSError:
            continue
        for path in iterator:
            if not path.is_file():
                continue
            key = str(path.resolve()).lower()
            if key in seen:
                continue
            seen.add(key)
            norm = _normalise_texture_stem(path.stem)
            files.append((priority, path, norm, norm.replace("_", ""), path.as_posix().lower()))
    _TEXTURE_FILE_INDEX_CACHE[cache_key] = files  # type: ignore[assignment]
    return files
def _texture_candidates(roots: list[Path], stem_candidates: list[str], preferred_dirs: tuple[str, ...] = ()) -> list[Path]:
    files = _texture_file_index(roots)
    normalized_targets = {_normalise_texture_stem(s) for s in stem_candidates if s}
    normalized_targets.discard("")
    compact_targets = {value.replace("_", "") for value in normalized_targets}
    # First pass: exact and structural matches. This is the common path and avoids
    # expensive fuzzy matching across every PNG in a large mod/resource tree.
    scored: list[tuple[int, int, str, Path]] = []
    fuzzy_pool: list[tuple[int, Path, str, str, str]] = []
    for priority, path, norm, compact, lower_path in files:
        exact = norm in normalized_targets or compact in compact_targets
        score = 1000 if exact else 0
        if not exact:
            for target in normalized_targets:
                compact_target = target.replace("_", "")
                if (
                    min(len(compact), len(compact_target)) >= 6
                    and min(len(compact), len(compact_target)) / max(len(compact), len(compact_target)) >= 0.78
                    and (compact.startswith(compact_target) or compact_target.startswith(compact))
                ):
                    # Similar names with a clear prefix relationship are usually the
                    # intended texture family (e.g. crystalwoodaxe vs crystalaxe).
                    length_gap = abs(len(compact) - len(compact_target))
                    score = max(score, 520 - min(220, length_gap * 30))
                elif compact and compact_target and compact[0] == compact_target[0]:
                    # Keep a very small fuzzy shortlist for genuinely unusual naming.
                    if abs(len(compact) - len(compact_target)) <= 4 and compact[:3] == compact_target[:3]:
                        fuzzy_pool.append((priority, path, norm, compact, lower_path))
        if score > 0:
            for directory in preferred_dirs:
                if f"/{directory.strip('/')}/" in lower_path:
                    score += 40
            scored.append((score, -priority, lower_path, path))
    # Second pass: fuzzy matching only over the small shortlist, preserving the old
    # fuzzy behavior for edge cases without doing O(items × textures × string-edit-distance)
    # work for every element in a full mod.
    if fuzzy_pool:
        from difflib import SequenceMatcher
        for priority, path, norm, compact, lower_path in fuzzy_pool:
            best_ratio = 0.0
            for target in compact_targets:
                if not target or not compact:
                    continue
                ratio = SequenceMatcher(None, compact, target).ratio()
                best_ratio = max(best_ratio, ratio)
            if best_ratio >= 0.84 and min(len(compact), max((len(t) for t in compact_targets), default=0)) >= 6:
                score = int(250 * best_ratio)
                for directory in preferred_dirs:
                    if f"/{directory.strip('/')}/" in lower_path:
                        score += 40
                scored.append((score, -priority, lower_path, path))
    scored.sort(key=lambda item: (-item[0], -item[1], item[2]))
    return [item[3] for item in scored]
def _exact_texture_candidates(
    roots: list[Path],
    stem_candidates: list[str],
    preferred_dirs: tuple[str, ...] = (),
) -> list[Path]:
    """Return only identity-equivalent texture names.
    Armor inventory/layer textures are unsafe to resolve fuzzily: selecting a
    nearby helmet for boots (or an item icon for a worn layer) is worse than a
    visible placeholder.  Underscore/case differences remain accepted.
    """
    normalized_targets = {_normalise_texture_stem(value) for value in stem_candidates if value}
    normalized_targets.discard("")
    compact_targets = {value.replace("_", "") for value in normalized_targets}
    scored: list[tuple[int, int, str, Path]] = []
    for priority, path, norm, compact, lower_path in _texture_file_index(roots):
        if norm not in normalized_targets and compact not in compact_targets:
            continue
        directory_score = 0
        for directory in preferred_dirs:
            if f"/{directory.strip('/')}/" in lower_path:
                directory_score += 40
        scored.append((directory_score, -priority, lower_path, path))
    scored.sort(key=lambda item: (-item[0], -item[1], item[2]))
    return [item[3] for item in scored]
def _mcreator_texture_ref(value: str) -> str:
    value = str(value or "").strip().replace("\\", "/")
    if not value:
        return ""
    if value.lower().startswith("minecraft:block/"):
        return "minecraft:" + value.split("minecraft:block/", 1)[1]
    if value.lower().startswith("minecraft:item/"):
        return "minecraft:" + value.split("minecraft:item/", 1)[1]
    return value


def _copy_model_ref_texture_to_workspace(
    model_path: Optional[Path],
    texture_ref: str,
    texture_root: Path,
    output_subdir: str,
    registry_name: str,
) -> str:
    """Resolve a model JSON texture reference and copy it into the MCreator texture tree.

    MCreator block/plant elements reference workspace texture names, not raw
    `namespace:path` resource locations.  Raw source model JSONs therefore need
    their referenced PNGs materialized into the workspace before the element is built.
    """
    ref = str(texture_ref or "").strip().replace("\\", "/")
    if not ref:
        return ""
    # Vanilla textures must remain in the minecraft namespace. Copying them into
    # the converted mod and returning only a stem makes generated block models
    # resolve e.g. minecraft:block/diamond_ore as cndcc:block/diamond_ore, which
    # produces missing-texture warnings. MCreator accepts explicit namespaced
    # texture references for block faces.
    if ":" in ref:
        namespace, resource_path = ref.split(":", 1)
        if namespace.lower() == "minecraft":
            return f"minecraft:{resource_path.lstrip('/').removeprefix("block/").removeprefix("item/")}"
    stem = _model_texture_stem(ref)
    if model_path is None:
        return stem
    candidates = _model_texture_sources(model_path, ref)
    source = candidates[0] if candidates else None
    target_dir = texture_root / output_subdir if output_subdir else texture_root
    target_dir.mkdir(parents=True, exist_ok=True)
    if source is None:
        return stem
    target = target_dir / f"{stem}.png"
    if target.exists() and target.read_bytes() != source.read_bytes():
        target = target_dir / f"{registry_name}_{stem}.png"
        stem = target.stem
    _copy_texture_with_metadata(source, target)
    return stem


def _armor_piece_texture_stems(prefix: str, field_name: str, part: str) -> list[str]:
    base = _normalise_texture_stem(prefix)
    field = _normalise_texture_stem(field_name)
    suffixes = {
        "helmet": ("helmet", "helm", "head"),
        "body": ("body", "chest", "chestplate", "torso"),
        "leggings": ("leggings", "legs", "pants"),
        "boots": ("boots", "feet", "shoes"),
    }[part]
    candidates = [field]
    candidates.extend(f"{base}_{suffix}" for suffix in suffixes)
    # Some old mods concatenate the material and piece names without an
    # underscore; the compact comparison in _exact_texture_candidates handles
    # those while retaining exact semantic identity.
    return list(dict.fromkeys(value for value in candidates if value))
def _armor_layer_texture_stems(prefix: str, layer: int) -> list[str]:
    base = _normalise_texture_stem(prefix)
    return [
        f"{base}_layer_{layer}",
        f"{base}__layer_{layer}",
        f"{base}_{layer}",
        f"{base}layer{layer}",
    ]
def _armor_related_texture_candidates(
    source_roots: list[Path],
    stems: list[str],
    part: str,
) -> list[Path]:
    """Fallback armor icon matching for old naming conventions.

    Some legacy sets use abbreviated/shared prefixes (e.g. `peacock_boots.png`
    for a `PeacockFeather` armor class).  Only consider files whose stem ends
    in the requested armor-part semantic to avoid cross-part collisions.
    """
    exact = _exact_texture_candidates(source_roots, stems, ("item", "items"))
    if exact:
        return exact
    suffixes = {
        "helmet": ("helmet", "helm", "head"),
        "body": ("body", "chest", "chestplate", "torso"),
        "leggings": ("leggings", "legs", "pants"),
        "boots": ("boots", "boot", "feet", "shoes"),
    }[part]
    bases = [re.sub(r"[^a-z0-9]", "", str(stem).lower()) for stem in stems if stem]
    scored: list[tuple[float, Path]] = []
    for root_priority, root in enumerate(source_roots):
        for path in root.rglob("*.png"):
            low = path.as_posix().lower()
            if not any(f"/{directory}/" in low for directory in ("item", "items")):
                continue
            stem = re.sub(r"[^a-z0-9]", "", path.stem.lower())
            if not any(stem.endswith(re.sub(r"[^a-z0-9]", "", suffix)) for suffix in suffixes):
                continue
            candidate_base = stem
            matched_suffix = None
            for suffix in sorted((re.sub(r"[^a-z0-9]", "", x) for x in suffixes), key=len, reverse=True):
                if candidate_base.endswith(suffix):
                    matched_suffix = suffix
                    candidate_base = candidate_base[:-len(suffix)]
                    break
            if not candidate_base or matched_suffix is None:
                continue
            score = 0.0
            for base in bases:
                if not base:
                    continue
                if candidate_base == base:
                    score = max(score, 100.0)
                elif candidate_base.startswith(base[: max(4, min(len(base), 8))]):
                    score = max(score, 75.0 + min(len(base), 12))
                elif base.startswith(candidate_base[: max(4, min(len(candidate_base), 8))]):
                    score = max(score, 65.0 + min(len(candidate_base), 10))
            if score > 0:
                scored.append((score - root_priority * 0.01, path))
    scored.sort(key=lambda item: (-item[0], str(item[1]).lower()))
    return [path for _, path in scored]


def _item_model_texture_ref(source_roots: list[Path], registry_name: str) -> Optional[str]:
    """Read the source item's own models/item/<id>.json texture before heuristic matching."""
    candidates=[]
    target=_compact_registry(registry_name)
    for root in source_roots:
        for model_root in (root / "assets", root / "src" / "main" / "resources" / "assets"):
            if not model_root.exists(): continue
            for path in model_root.rglob("models/item/*.json"):
                if _compact_registry(path.stem)==target:
                    candidates.append(path)
    for path in sorted(candidates, key=lambda p: str(p).lower()):
        try: payload=json.loads(path.read_text(encoding="utf-8"))
        except Exception: continue
        textures=payload.get("textures") if isinstance(payload,dict) else None
        if isinstance(textures,dict):
            for key in ("layer0","layer0"):
                value=textures.get(key)
                if isinstance(value,str) and value:
                    return value
            value=next((v for v in textures.values() if isinstance(v,str) and v),None)
            if value: return value
    return None


def _copy_or_placeholder_texture(
    source_roots: list[Path],
    texture_root: Path,
    registry_name: str,
    stem_candidates: list[str],
    preferred_dirs: tuple[str, ...],
    output_subdir: str = "",
    allow_placeholder: bool = True,
) -> tuple[str, str, list[str]]:
    warnings: list[str] = []
    candidates = _texture_candidates(source_roots, stem_candidates, preferred_dirs)
    texture_name = registry_name
    source = candidates[0] if candidates else None
    if source:
        texture_name = source.stem
    elif allow_placeholder:
        warnings.append(f"No texture found for '{registry_name}'. Generated a checker placeholder.")
    else:
        warnings.append(f"No texture found for '{registry_name}'; left texture unset because this element type does not require one.")
    target_dir = texture_root / output_subdir if output_subdir else texture_root
    target = target_dir / f"{texture_name}.png"
    if source:
        if target.exists() and target.read_bytes() != source.read_bytes():
            texture_name = f"{registry_name}_{texture_name}"
            target = target_dir / f"{texture_name}.png"
        _copy_texture_with_metadata(source, target)
    elif allow_placeholder:
        if not target.exists():
            _write_placeholder_png(target)
    return texture_name if source or allow_placeholder else "", str(target) if source or allow_placeholder else "", warnings
def _element_names(workspace: dict[str, Any]) -> tuple[set[str], set[str]]:
    names = {str(item.get("name", "")).lower() for item in workspace.get("mod_elements", [])}
    registries = {str(item.get("registry_name", "")).lower() for item in workspace.get("mod_elements", [])}
    return names, registries
def _workspace_elements_by_registry(workspace: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        str(item.get("registry_name", "")).lower(): item
        for item in workspace.get("mod_elements", [])
        if str(item.get("registry_name", "")).strip()
    }
def _workspace_elements_by_compact_registry(workspace: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        _compact_registry(item.get("registry_name", "")): item
        for item in workspace.get("mod_elements", [])
        if _compact_registry(item.get("registry_name", ""))
    }
def _workspace_element_for_type(workspace: dict[str, Any], element_type: str, registry: str) -> dict[str, Any] | None:
    target = _compact_registry(registry)
    for item in workspace.get("mod_elements", []):
        if str(item.get("type", "")).lower() != str(element_type).lower():
            continue
        if _compact_registry(item.get("registry_name", "")) == target:
            return item
    return None
def _sanitize_new_import_definition(definition: dict[str, Any], element_type: str) -> dict[str, Any]:
    """Neutralize template-owned visual bindings without leaving invalid required fields.

    New imports may use a completely unrelated template element (for example a Chainsaw
    tool or an ore block). The old implementation blanked every visual field but preserved
    the template render mode, producing invalid combinations such as renderType=1 with an
    empty customModelName, or empty required block face textures. We instead reset the
    *binding* fields to safe native defaults and let the source importer fill them.
    Existing template elements are never passed through this function.
    """
    out = copy.deepcopy(definition)
    kind = str(element_type or "").lower()
    if kind in {"item", "tool"}:
        # 0 = standard generated item/tool model in the NeoForge 1.21.1 generator.
        out["renderType"] = 0
        out["customModelName"] = "Normal"
        if "blockingRenderType" in out:
            out["blockingRenderType"] = 0
        if "blockingModelName" in out:
            out["blockingModelName"] = "Normal blocking"
        # These fields are required by some schema versions and are filled from the
        # imported source texture after element construction.
        for key in ("texture", "guiTexture", "itemTexture", "particleTexture"):
            if key in out:
                out[key] = ""
    elif kind == "block":
        # Native cube-all block is a valid safe baseline. Keep the schema's expected
        # custom-model sentinel rather than leaving renderType pointing at a deleted model.
        out["renderType"] = 10
        if "customModelName" in out:
            out["customModelName"] = "Normal"
        for key in ("texture", "textureTop", "textureLeft", "textureFront", "textureRight", "textureBack", "itemTexture", "particleTexture"):
            if key in out:
                out[key] = ""
    elif kind == "plant":
        out["renderType"] = 12
        if "customModelName" in out:
            out["customModelName"] = "Normal"
        for key in ("texture", "textureBottom", "itemTexture", "particleTexture"):
            if key in out:
                out[key] = ""
    elif kind == "armor":
        for key in list(out):
            low = str(key).lower()
            if "texture" in low or "modelname" in low or "modelpart" in low or "custommodel" in low:
                if isinstance(out[key], (str, int, float, bool)) or out[key] is None:
                    out[key] = "" if not isinstance(out[key], bool) else False
    return out


def _existing_element_definition(
    workspace_root: Path,
    element_entry: dict[str, Any] | None,
    fallback_fv: int,
    fallback_definition: dict[str, Any],
) -> tuple[int, dict[str, Any], str]:
    if not element_entry:
        return fallback_fv, copy.deepcopy(fallback_definition), "created"
    element_name = str(element_entry.get("name", ""))
    path = workspace_root / "elements" / f"{element_name}.mod.json"
    existing = _read_json(path)
    if existing and isinstance(existing.get("definition"), dict):
        try:
            fv = int(existing.get("_fv", fallback_fv))
        except Exception:
            fv = fallback_fv
        return fv, copy.deepcopy(existing["definition"]), "updated"
    return fallback_fv, copy.deepcopy(fallback_definition), "updated"
def _workspace_entry_status(entry: dict[str, Any] | None) -> str:
    if not isinstance(entry, dict):
        return "created"
    if entry.get("mojmodelbridge_generated") is True or str(entry.get("path", "")).startswith("~/Converted "):
        return "resumed"
    return "existing"
def _upsert_workspace_element(
    workspace: dict[str, Any],
    entry: dict[str, Any] | None,
    element_name: str,
    element_type: str,
    registry: str,
    import_modid: str = "converted_mod",
    source_registry: str | None = None,
    folder_type: str | None = None,
) -> dict[str, Any]:
    items = workspace.setdefault("mod_elements", [])
    if entry is None:
        entry = {
            "name": element_name,
            "type": element_type,
            "compiles": True,
            "locked_code": False,
            "registry_name": registry,
            "metadata": {"files": []},
            "path": _mcreator_import_path(import_modid, folder_type or element_type),
            "mojmodelbridge_generated": True,
            "mojmodelbridge_source_modid": import_modid,
            "mojmodelbridge_source_registry": source_registry or registry,
        }
        items.append(entry)
    else:
        entry["name"] = element_name
        entry["type"] = element_type
        entry.setdefault("metadata", {}).setdefault("files", [])
        entry["registry_name"] = registry
        entry.setdefault("compiles", True)
        entry.setdefault("locked_code", False)
        if entry.get("mojmodelbridge_generated") is True:
            entry["path"] = _mcreator_import_path(import_modid, folder_type or element_type)
        entry.setdefault("mojmodelbridge_source_modid", import_modid)
        entry.setdefault("mojmodelbridge_source_registry", source_registry or registry)
    return entry
def _mcreator_import_root_name(modid: str) -> str:
    """Return the user-facing top-level folder for an imported mod.

    Preserve a simple legacy mod id such as ``orespawn`` verbatim; for compact
    names such as ``lostinfinity`` retain readable camel-case when common words
    can be inferred. This is only workspace organization metadata and never
    changes registry IDs.
    """
    raw = str(modid or "converted_mod").strip().replace("-", "_")
    if raw.lower() == "lostinfinity":
        return "LostInfinity"
    if raw.lower() == "lost_infinity":
        return "LostInfinity"
    return raw or "converted_mod"


def _mcreator_import_category(element_type: str) -> str:
    kind = str(element_type or "").lower()
    return {
        "livingentity": "Mobs",
        "item": "Items",
        "tool": "Tools",
        "weapon": "Weapons",
        "food": "Foods",
        "armor": "Armor",
        "block": "Blocks",
        "plant": "Plants",
        "projectile": "Projectiles",
        "food": "Foods",
        "dimension": "World Generation/Dimensions",
        "biome": "World Generation/Biomes",
        "tab": "Tabs",
    }.get(kind, _pascal(kind or "Other"))


def _mcreator_import_path(modid: str, element_type: str) -> str:
    return f"~/{_mcreator_import_root_name(modid)}/{_mcreator_import_category(element_type)}"


def _unique_name(base: str, used: set[str]) -> str:
    candidate = base or "ConvertedContent"
    value = candidate
    n = 2
    while value.lower() in used:
        value = f"{candidate}{n}"
        n += 1
    used.add(value.lower())
    return value
def _new_element(
    fv: int,
    definition: dict[str, Any],
    element_name: str,
    registry_name: str,
    element_type: str,
    definition_updates: dict[str, Any],
) -> dict[str, Any]:
    definition = copy.deepcopy(definition)
    definition.update(definition_updates)
    # Common MCreator definitions use the human-readable name and registry name indirectly.
    if "name" in definition:
        definition["name"] = definition_updates.get("name", element_name.replace("_", " "))
    return {"_fv": fv, "_type": element_type, "definition": definition}
def _display_name(registry_name: str) -> str:
    value = str(registry_name or "").replace("-", "_").replace(".", "_")
    value = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", value)
    value = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1 \2", value)
    value = re.sub(r"[_\s]+", " ", value).strip()
    return " ".join(word.capitalize() for word in value.split())
_ENCHANTMENT_NAME_MAP = {
    "ALL_DAMAGE_PROTECTION": "PROTECTION",
    "FIRE_PROTECTION": "FIRE_PROTECTION",
    "BLAST_PROTECTION": "BLAST_PROTECTION",
    "PROJECTILE_PROTECTION": "PROJECTILE_PROTECTION",
    "RESPIRATION": "RESPIRATION",
    "AQUA_AFFINITY": "AQUA_AFFINITY",
    "UNBREAKING": "UNBREAKING",
    "FALL_PROTECTION": "FEATHER_FALLING",
    "BLOCK_EFFICIENCY": "EFFICIENCY",
    "BLOCK_FORTUNE": "FORTUNE",
    "SILK_TOUCH": "SILK_TOUCH",
    "MENDING": "MENDING",
    "SHARPNESS": "SHARPNESS",
    "SMITE": "SMITE",
    "BANE_OF_ARTHROPODS": "BANE_OF_ARTHROPODS",
    "KNOCKBACK": "KNOCKBACK",
    "MOB_LOOTING": "LOOTING",
    "FIRE_ASPECT": "FIRE_ASPECT",
    "SWEEPING_EDGE": "SWEEPING",
    "POWER_ARROWS": "POWER",
    "PUNCH_ARROWS": "PUNCH",
    "FLAMING_ARROWS": "FLAME",
    "INFINITY_ARROWS": "INFINITY",
}


def _java_int_constants(text: str) -> dict[str, int]:
    values: dict[str, int] = {}
    # Direct declarations.
    for m in re.finditer(r"\b(?:public|private|protected)?\s*(?:static\s+)?(?:final\s+)?int\s+([A-Za-z_$][\w$]*)\s*=\s*(-?\d+)\s*;", text):
        values[m.group(1)] = int(m.group(2))
    # Config-backed integers: use the source's declared default because that is the
    # only deterministic value available to an MCreator procedure.
    for m in re.finditer(r"\b([A-Za-z_$][\w$]*)\s*=\s*config\.get\s*\(.*?,\s*[\"\'][^\"\']+[\"\'],\s*(-?\d+)\s*\)\.getInt\s*\(\s*\)\s*;", text):
        values[m.group(1)] = int(m.group(2))
    return values


def _java_numeric_literal(expr: str, constants: Optional[dict[str, int]] = None) -> Optional[int]:
    expr = re.sub(r"\b(?:int|float|double|long)\s*\b", "", str(expr or "")).strip()
    expr = re.sub(r"[fFdD]\b", "", expr)
    if constants:
        for name, value in sorted(constants.items(), key=lambda item: -len(item[0])):
            expr = re.sub(rf"\b{re.escape(name)}\b", str(value), expr)
    if not re.fullmatch(r"[-+*/()\d\s]+", expr):
        return None
    try:
        import ast
        node = ast.parse(expr, mode="eval").body
        allowed=(ast.Expression, ast.Constant, ast.UnaryOp, ast.UAdd, ast.USub, ast.BinOp, ast.Add, ast.Sub, ast.Mult, ast.Div, ast.FloorDiv, ast.Mod, ast.Pow)
        if not isinstance(node, allowed): return None
        # Evaluate only a restricted integer arithmetic tree.
        def ev(n):
            if isinstance(n, ast.Constant) and isinstance(n.value, (int,float)):
                return n.value
            if isinstance(n, ast.UnaryOp):
                v=ev(n.operand); return +v if isinstance(n.op, ast.UAdd) else -v
            if isinstance(n, ast.BinOp):
                a,b=ev(n.left),ev(n.right)
                if isinstance(n.op, ast.Add): return a+b
                if isinstance(n.op, ast.Sub): return a-b
                if isinstance(n.op, ast.Mult): return a*b
                if isinstance(n.op, (ast.Div, ast.FloorDiv)): return a//b
                if isinstance(n.op, ast.Mod): return a%b
                if isinstance(n.op, ast.Pow): return a**b
            raise ValueError
        value=ev(node)
        if isinstance(value,(int,float)) and float(value).is_integer(): return int(value)
    except Exception:
        return None
    return None


def _craft_enchantment_calls(text: str, source_roots: Optional[list[Path]] = None) -> list[tuple[str, int]]:
    """Recover stack.enchant calls reachable from an item's onCraftedBy callback.

    This follows same-class helper calls recursively because older mods commonly put
    the actual enchantment block in a private helper invoked by onCraftedBy.
    """
    from model_bridge import _legacy_java_methods
    methods = _legacy_java_methods(text)
    constants = _java_int_constants(text)
    # Fold simple local aliases such as `int m = SomeClass.SomeConfigField;`.
    for m_alias in re.finditer(r"\bint\s+([A-Za-z_$][\w$]*)\s*=\s*(?:[A-Za-z_$][\w$]*\.)?([A-Za-z_$][\w$]*)\s*;", text):
        if m_alias.group(2) in constants:
            constants[m_alias.group(1)] = constants[m_alias.group(2)]
    if source_roots:
        # Resolve qualified static constants referenced by the item class, e.g.
        # ChaosPersists.UltimateSwordMagic. Search only the named class and parse its
        # declaration/config default; no mod-specific name is assumed.
        for qclass, field in re.findall(r"\b([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)\b", text):
            if field in constants: continue
            for root in source_roots:
                for candidate in root.rglob(f"{qclass}.java"):
                    try: other=candidate.read_text(encoding="utf-8-sig", errors="replace")
                    except OSError: continue
                    other_constants=_java_int_constants(other)
                    if field in other_constants: constants[field]=other_constants[field]
                    break
                if field in constants: break
    # Resolve aliases again after loading qualified constants.
    for m_alias in re.finditer(r"\bint\s+([A-Za-z_$][\w$]*)\s*=\s*(?:[A-Za-z_$][\w$]*\.)?([A-Za-z_$][\w$]*)\s*;", text):
        if m_alias.group(2) in constants:
            constants[m_alias.group(1)] = constants[m_alias.group(2)]
    if "onCraftedBy" not in methods:
        return []
    queue = [("onCraftedBy", body) for _params, body, _source in methods.get("onCraftedBy", [])]
    seen: set[str] = set()
    results: list[tuple[str, int]] = []
    while queue:
        method_name, body = queue.pop(0)
        if method_name in seen:
            continue
        seen.add(method_name)
        for m in re.finditer(
            r"(?:[A-Za-z_$][\w$]*\.)?(?:enchant|addEnchantment)\s*\(\s*"
            r"(?:Enchantments\.)?([A-Za-z0-9_]+)\s*,\s*([^\)]+)\)",
            body,
        ):
            enchantment = _ENCHANTMENT_NAME_MAP.get(m.group(1).upper())
            level = _java_numeric_literal(m.group(2), constants)
            if enchantment and level is not None and level > 0:
                results.append((enchantment, level))
        for call in re.finditer(r"\b([A-Za-z_$][\w$]*)\s*\(", body):
            target = call.group(1)
            if target in methods and target not in seen:
                for _params, helper_body, _source in methods[target]:
                    queue.append((target, helper_body))
    # Preserve source order while de-duplicating identical enchantments.
    unique: list[tuple[str, int]] = []
    seen_pairs: set[tuple[str, int]] = set()
    for pair in results:
        if pair not in seen_pairs:
            seen_pairs.add(pair)
            unique.append(pair)
    return unique


def _enchantment_procedure_xml(enchantments: list[tuple[str, int]]) -> str:
    """Create the native MCreator on-crafted procedure shape used by the supplied 1.21.1 example."""
    if not enchantments:
        return ""
    blocks = []
    for enchantment, level in enchantments:
        blocks.append(
            f'<block type="item_add_enhancement"><field name="enhancement">{enchantment}</field>'
            f'<value name="level"><block type="math_number"><field name="NUM">{int(level)}</field></block></value>'
            f'<value name="item"><block type="itemstack_to_mcitem"></block></value></block>'
        )
    chain = blocks[-1]
    for block in reversed(blocks[:-1]):
        chain = block[:-len('</block>')] + f'<next>{chain}</next></block>'
    return (
        '<xml xmlns="https://developers.google.com/blockly/xml">'
        '<block type="event_trigger" deletable="false" x="40" y="40">'
        '<field name="trigger">no_ext_trigger</field><next>' + chain +
        '</next></block></xml>'
    )


def _emit_exact_source_worldgen_modifier(
    workspace_root: Path,
    modid: str,
    registry: str,
    generation: dict[str, Any],
) -> Optional[Path]:
    """Bridge a source-authored placed feature into NeoForge 1.21.1 biome decoration.

    The supplied NeoForge workspace uses ``neoforge:add_features`` modifiers for
    imported placed features. Reuse that contract so copied source configured/placed
    features are actually referenced by biomes rather than merely sitting unused
    in the generated resources directory.
    """
    if not generation.get("sourceResourceExact"):
        return None
    source_feature = str(generation.get("sourceFeature") or "")
    if ":" in source_feature:
        _namespace, _rest = source_feature.split(":", 1)
        feature_name = _rest.rsplit("/", 1)[-1]
        # Source bridge rewrites the namespace to the target mod id.
    else:
        feature_name = source_feature.rsplit("/", 1)[-1]
    feature_registry = feature_name[:-6] if feature_name.endswith("_placed") else feature_name
    # The source resource path already points at the placed feature's registry name;
    # prefer its explicit final stem to avoid accidentally referencing a configured feature.
    if source_feature and "/placed_feature/" in source_feature:
        feature_registry = source_feature.rsplit("/", 1)[-1]
    target = (
        Path(workspace_root)
        / "src"
        / "main"
        / "resources"
        / "data"
        / modid
        / "neoforge"
        / "biome_modifier"
        / f"{registry}_source_worldgen.json"
    )
    target.parent.mkdir(parents=True, exist_ok=True)
    step = "underground_ores" if "ore" in feature_registry.lower() else "vegetal_decoration"
    payload = {
        "type": "neoforge:add_features",
        "biomes": {"type": "neoforge:any"},
        "features": f"{modid}:{feature_registry}",
        "step": step,
    }
    target.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return target

def _add_crafted_enchantment_procedure(
    element: dict[str, Any],
    source_text: str,
    element_name: str,
    output_elements: Path,
    workspace: dict[str, Any],
    used_names: set[str],
    source_modid: str,
    source_roots: Optional[list[Path]] = None,
) -> Optional[str]:
    enchantments = _craft_enchantment_calls(source_text, source_roots)
    if not enchantments:
        return None
    procedure_name = _unique_name(f"{_pascal(element_name)}ItemIsCrafted", used_names)
    procedure = {
        "_fv": int(element.get("_fv", 89)),
        "_type": "procedure",
        "definition": {
            "procedurexml": _enchantment_procedure_xml(enchantments),
            "skipDependencyNullCheck": False,
        },
    }
    procedure_path = output_elements / f"{procedure_name}.mod.json"
    procedure_path.write_text(json.dumps(procedure, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    workspace.setdefault("mod_elements", []).append({
        "name": procedure_name,
        "type": "procedure",
        "compiles": True,
        "locked_code": False,
        "registry_name": procedure_name,
        "metadata": {
            "dependencies": [
                {"name": "itemstack", "type": "itemstack"},
                {"name": "world", "type": "world"},
            ],
            "files": [],
        },
        "path": f"~/{source_modid}/Recipes",
        "mojmodelbridge_generated": True,
    })
    definition = element.setdefault("definition", {})
    if "onCrafted" in definition or element.get("_type") in {"item", "tool"}:
        definition["onCrafted"] = procedure_name
    definition.setdefault("mojmodelbridge_crafted_enchantments", [
        {"enchantment": enchantment, "level": level} for enchantment, level in enchantments
    ])
    return procedure_name


def _ranged_behavior_updates(text: str, registry: str, template: dict[str, Any]) -> dict[str, Any]:
    """Recover modern MCreator ranged-item fields from clean-source use methods."""
    updates: dict[str, Any] = {}
    is_bow = bool(re.search(r"\bUseAnim\.BOW\b|\bgetUseAnimation\s*\([^)]*\)\s*\{[^{}]*?BOW", text, re.DOTALL))
    is_crossbow = bool(re.search(r"\bUseAnim\.CROSSBOW\b|\bgetUseAnimation\s*\([^)]*\)\s*\{[^{}]*?CROSSBOW", text, re.DOTALL))
    is_fishing = bool(re.search(r"extends\s+(?:net\.[\w.]+\.)?FishingRodItem\b|\bFishingRodItem\b", text))
    if is_bow or is_crossbow:
        updates.update({
            "enableRanged": True,
            "shootConstantly": False,
            "rangedItemChargesPower": True,
            "projectileDisableAmmoCheck": False,
            "animation": "bow" if is_bow else "crossbow",
            # MCreator 2024.x/NeoForge generates Arrow-specific Java when a
            # ranged Item points directly at a custom Projectile element. Legacy
            # Chaos Persists projectiles are arbitrary Projectiles/AbstractArrow
            # implementations and can therefore produce invalid code such as
            # `projectile.pickup` and `new ItemStack()`. Keep the native ranged
            # path on the known-good vanilla Arrow and preserve the exact source
            # constructor target for the later custom-fire bridge.
            "projectile": "Arrow",
        })
        m = re.search(r"\bgetUseDuration\s*\([^)]*\)\s*\{\s*return\s+([0-9]+)\s*;", text, re.DOTALL)
        if m and "useDuration" in template:
            updates["useDuration"] = max(0, int(m.group(1)))
        custom = re.findall(r"\bnew\s+([A-Za-z_$][\w$]*(?:Arrow|Projectile|Bolt|Shot))\s*\(", text)
        if custom:
            updates["mojmodelbridge_source_projectile_class"] = custom[0]
    return updates

def _item_element(source: Path, simple: str, superclass: str, text: str, registry: str, fv: int, template: dict[str, Any], texture: str, constructor_records: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    stack = _find_int_after(text, ("func_77625_d", "setMaxStackSize")) or 64
    max_damage = _find_int_after(text, ("func_77656_e", "setMaxDamage")) or 0
    updates = {
        "name": _display_name(registry),
        "texture": texture,
        "stackSize": max(1, min(stack, 64)),
        "damageCount": max(0, max_damage),
        "creativeTabs": ["MISC"],
    }
    if "guiTexture" in template:
        updates["guiTexture"] = texture
    if "customModelName" in template:
        updates["customModelName"] = "Normal"
    if "renderType" in template:
        updates["renderType"] = 0
    if "isFood" in template:
        updates.update(_extract_food_info(text, superclass, constructor_records))
        if updates.get("isFood"):
            updates["creativeTabs"] = ["FOOD"]
    updates.update(_ranged_behavior_updates(text, registry, template))
    return _new_element(fv, template, _element_name_from_class(simple, registry), registry, "item", updates)
def _tool_element(simple: str, text: str, registry: str, fv: int, template: dict[str, Any], texture: str, tool_type: str, material_stats: Optional[dict[str, float]] = None, material_table: Optional[dict[str, dict[str, float]]] = None) -> tuple[dict[str, Any], list[str]]:
    if material_stats is None and material_table:
        target = _compact_registry(registry)
        ranked = []
        for key, value in material_table.items():
            k = _compact_registry(key)
            if not k or len(k) < 5:
                continue
            if k in target or target in k:
                ranked.append((len(k), value))
        if ranked:
            material_stats = dict(max(ranked, key=lambda item: item[0])[1])
    usage, damage, efficiency, enchantability, warnings = _recover_tool_stats(text, tool_type, material_stats)
    # MCreator's native Bow/Fishing Rod types carry the correct vanilla interaction
    # behavior. Detect both true legacy superclasses and modern custom Items that
    # implement the corresponding use animation/behavior themselves.
    if tool_type not in {"Bow", "Fishing Rod"}:
        if re.search(r"\bUseAnim\.BOW\b|\bgetUseAnimation\s*\([^)]*\)\s*\{[^{}]*UseAnim\.BOW", text, re.DOTALL):
            tool_type = "Bow"
        elif re.search(r"extends\s+FishingRodItem\b", text) or "FishingRodItem" in text:
            tool_type = "Fishing Rod"
    attack_speed = 1.0
    # SwordItem/Tier constructors encode the final attack-speed modifier as a
    # negative literal (e.g. -2.4 => 1.6 actual attack speed). Recover it.
    m_speed = re.search(r"super\s*\([^;]*?,\s*-?(\d+(?:\.\d+)?)f?\s*,\s*new\s+Properties", text, re.DOTALL)
    if m_speed and tool_type in {"Sword", "Axe", "Pickaxe", "Shovel", "Hoe"}:
        raw = float(m_speed.group(1))
        # The common Mojmap Item constructor stores an attack-speed modifier;
        # vanilla's base is 4.0, so -2.4 becomes 1.6.
        if raw > 0:
            attack_speed = max(0.0, 4.0 - raw)
        else:
            attack_speed = 4.0 + raw
    updates = {
        "toolType": tool_type,
        "texture": texture,
        "name": _display_name(registry),
        "creativeTabs": ["COMBAT" if tool_type in {"Sword", "Bow"} else "TOOLS"],
        "usageCount": usage,
        "damageVsEntity": damage,
        "efficiency": efficiency,
        "attackSpeed": attack_speed,
        "enchantability": enchantability,
        "repairItems": [],
    }
    if "guiTexture" in template:
        updates["guiTexture"] = texture
    if "customModelName" in template:
        updates["customModelName"] = "Normal"
    if "renderType" in template:
        updates["renderType"] = 0
    if "blockingRenderType" in template:
        updates["blockingRenderType"] = 0
    if "blockingModelName" in template:
        updates["blockingModelName"] = "Normal blocking"
    warnings.append("Material-specific mining speed, repair ingredient, and special behavior were not fully reconstructed.")
    return _new_element(fv, template, _pascal(simple), registry, "tool", updates), warnings
def _group_armor_fields(text: str) -> dict[str, dict[str, str]]:
    result: dict[str, dict[str, str]] = {}
    for match in re.finditer(
        r"\b(?:[A-Za-z_$][\w$]*\.)*[A-Za-z_$][\w$]*Armor\s+"
        r"([A-Za-z0-9_]+(?:Helmet|Body|Legs|Boots|Chestplate))\s*;",
        text,
    ):
        field = match.group(1)
        for suffix, part in (("Helmet", "helmet"), ("Body", "body"), ("Chestplate", "body"), ("Legs", "leggings"), ("Boots", "boots")):
            if field.endswith(suffix):
                prefix = field[: -len(suffix)]
                result.setdefault(prefix, {})[part] = field
                break
    # A number of 1.12 mods register armor through a reusable ArmorSet
    # container rather than four top-level ItemArmor fields.  The nested set
    # still represents one complete MCreator armor element.
    for set_match in re.finditer(
        r"\b[A-Za-z0-9_.$]*ArmorSet\s+([A-Za-z0-9_]+?)(?:Set)?\s*;",
        text,
    ):
        prefix = set_match.group(1)
        result.setdefault(prefix, {}).update({
            "helmet": f"{prefix}_helmet",
            "body": f"{prefix}_chestplate",
            "leggings": f"{prefix}_leggings",
            "boots": f"{prefix}_boots",
        })
    return result
def _find_model_source(source_roots: list[Path], model_class: str) -> Optional[Path]:
    candidates: list[Path] = []
    for root in source_roots:
        java = list(root.rglob(f"{model_class}.java"))
        cls = list(root.rglob(f"{model_class}.class"))
        candidates.extend(sorted(java, key=lambda p: str(p).lower()))
        candidates.extend(sorted(cls, key=lambda p: str(p).lower()))
    if not candidates:
        return None
    # Source is preferred because inherited/field names are more readable and
    # the legacy parser can preserve the exact armor-part names expected by MCreator.
    candidates.sort(key=lambda p: (0 if p.suffix.lower() == ".java" else 1, str(p).lower()))
    return candidates[0]
def _custom_armor_model_links(source_roots: list[Path]) -> dict[str, str]:
    """Return armor-prefix -> legacy custom model class names from getArmorModel overrides."""
    result: dict[str, str] = {}
    for root in source_roots:
        for path in root.rglob("*.java"):
            try:
                text = path.read_text(encoding="utf-8-sig", errors="replace")
            except OSError:
                continue
            if "getArmorModel" not in text:
                continue
            method = re.search(r"\bgetArmorModel\s*\([^)]*\)\s*\{", text, re.DOTALL)
            if not method:
                continue
            try:
                from model_bridge import extract_balanced
                body, _ = extract_balanced(text, text.find("{", method.start()))
            except Exception:
                body = text[method.end():]
            models = re.findall(r"\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(", body)
            if not models:
                # Some decompilers use a variable constructed outside the method;
                # inspect the whole class as a conservative fallback.
                models = re.findall(r"\bnew\s+((?:Model|model)[A-Za-z0-9_$]+)\s*\(", text)
            if not models:
                continue
            model_class = _simple(models[0])
            stem = path.stem
            if stem.startswith("Item"):
                stem = stem[4:]
            stem = re.sub(r"(?i)(?:armor)$", "", stem)
            stem = re.sub(r"(?i)(?:helmet|chestplate|leggings|boots)$", "", stem)
            key = _compact_registry(stem)
            if key:
                result[key] = model_class
    return result
def _write_custom_armor_model(
    source_root_candidates: list[Path],
    model_class: str,
    workspace_root: Path,
    modid: str,
) -> tuple[bool, str, list[str]]:
    """Convert a legacy ModelBiped/ModelRenderer armor model into an MCreator Java model.
    MCreator's 1.20.1 armor generator expects a model class with a public
    ModelPart for each selected armor body group. The legacy Lost Infinity
    models already use the canonical head/body/right_arm/left_arm/right_leg/
    left_leg group names, so preserving those groups gives MCreator the same
    attachment points used by Blockbench armor workflows.
    """
    warnings: list[str] = []
    source = _find_model_source(source_root_candidates, model_class)
    if source is None:
        return False, "", [f"Custom armor model {model_class} was referenced but its source was not found."]
    try:
        from model_bridge import Cube, Part, ParsedModel, parse_java_model, parse_legacy_java_model
        if source.suffix.lower() == ".java":
            text = source.read_text(encoding="utf-8-sig", errors="replace")
            model = parse_legacy_java_model(text, source.name)
            if model.cube_count <= 0:
                model = parse_java_model(text, source.name)
        else:
            from legacy_bridge import parse_legacy_class
            model = parse_legacy_class(source, Cube, Part, ParsedModel, resource_roots=source_root_candidates)
        roots = {str(part.name).lower(): part for part in model.parts if part.parent is None}
        aliases = {
            "head": ("head", "helmet", "bipedhead", "head_part"),
            "body": ("body", "chest", "torso", "bipedbody", "body_part"),
            "right_arm": ("right_arm", "rightarm", "armright", "bipedrightarm"),
            "left_arm": ("left_arm", "leftarm", "armleft", "bipedleftarm"),
            "right_leg": ("right_leg", "rightleg", "legright", "bipedrightleg"),
            "left_leg": ("left_leg", "leftleg", "legleft", "bipedleftleg"),
        }
        missing = [target for target, names in aliases.items() if not any(name in roots for name in names)]
        if missing:
            warnings.append(f"Custom armor model {model_class} is missing canonical body-part groups: {', '.join(missing)}; the model will not be attached automatically.")
            return False, "", warnings
        from mcreator_export import model_to_mojmap_java
        java = model_to_mojmap_java(model, model_class, modid)
        # Armor.java.ftl accesses the selected ModelPart fields directly.
        java = java.replace("\tprivate final ModelPart ", "\tpublic final ModelPart ")
        target = workspace_root / "models" / "mojmap-1.20.x" / f"{model_class}.java"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(java, encoding="utf-8")
        return True, model_class, warnings
    except Exception as exc:
        return False, "", [f"Custom armor model extraction failed for {model_class}: {exc}"]
def _armor_piece_registry_aliases(registry_source: Path, pieces: dict[str, str]) -> dict[str, str]:
    """Recover exact registered item ids from field assignments.

    This handles sets where the Java field is CrystalPinkHelmet but the actual item
    registry id is simply pink_helmet.
    """
    try:
        text = registry_source.read_text(encoding="utf-8-sig", errors="replace") if registry_source.suffix.lower()==".java" else _run_javap(registry_source)
    except OSError:
        return {}
    out={}
    for part, field in pieces.items():
        if not field: continue
        pat=r"\b"+re.escape(field)+r"\s*=.*?(?:registerItem|BuiltInRegistries\.ITEM\.get).*?[\"']([a-z0-9_./-]+)[\"']"
        m=re.search(pat,text,re.IGNORECASE|re.DOTALL)
        if m:
            raw=m.group(1).split(":",1)[-1]
            if _looks_like_registry_path(raw): out[part]=raw
    return out


def _armor_elements_from_registry(
    registry_source: Path,
    fv: int,
    template: dict[str, Any],
    used_names: set[str],
    used_registries: set[str],
    texture_root: Path,
    source_roots: list[Path],
    output_elements: Path,
    workspace: dict[str, Any],
    modid: str,
    log: Optional[Callable[[str], None]],
    armor_stats: Optional[dict[str, dict[str, float]]] = None,
    language_names: Optional[dict[str, str]] = None,
    import_modid: str = "converted_mod",
) -> list[MCreatorContentExport]:
    if registry_source.suffix.lower() == ".java":
        text = registry_source.read_text(encoding="utf-8-sig", errors="replace")
    else:
        text = _run_javap(registry_source)
    sound_updates, sound_bindings = infer_sound_bindings(text, registry_source.stem, "armor", {"sound_elements": workspace.get("sound_elements", [])})
    groups = _group_armor_fields(text)
    results: list[MCreatorContentExport] = []
    existing_by_registry = _workspace_elements_by_registry(workspace)
    custom_armor_models = _custom_armor_model_links(source_roots)
    armor_model_cache: dict[str, tuple[bool, str, list[str]]] = {}
    for prefix, pieces in sorted(groups.items()):
        registry = _snake(prefix + "_armor")
        desired = _pascal(prefix + " Armor")
        existing_entry = _workspace_element_for_type(workspace, "armor", registry)
        if existing_entry:
            existing_name = str(existing_entry.get("name") or desired)
            existing_status = _workspace_entry_status(existing_entry)
            results.append(
                MCreatorContentExport(
                    existing_name,
                    str(existing_entry.get("registry_name") or registry),
                    "armor",
                    registry_source.name,
                    existing_status,
                    ([] if existing_status == "resumed" else ["Existing armor element found in the supplied MCreator workspace; source conversion intentionally skipped."]),
                )
            )
            continue
        element_name = desired if desired.lower() not in used_names else _unique_name(desired, used_names)
        status = "created"
        used_names.add(element_name.lower())
        used_registries.add(registry.lower())
        recovered_stats = (armor_stats or {}).get(_compact_registry(prefix))
        warnings: list[str] = []
        if not recovered_stats:
            warnings.append("Armor defense values, material durability, enchantability, and special effects are conservative defaults unless directly recoverable.")
        base_stem = _normalise_texture_stem(prefix)
        # Resolve each inventory icon independently. A whole-armor search for `ultimate` can
        # accidentally select `ultimate_boots.png` or an unrelated entity texture; exact piece
        # stems are authoritative when available.
        part_texture: dict[str, str] = {}
        part_outputs: list[str] = []
        registered_piece_ids = _armor_piece_registry_aliases(registry_source, pieces)
        for part in ("helmet", "body", "leggings", "boots"):
            field_name = pieces.get(part, f"{prefix}_{part}")
            stems = _armor_piece_texture_stems(prefix, field_name, part)
            if registered_piece_ids.get(part):
                rid_stem = _normalise_texture_stem(registered_piece_ids[part])
                stems = [rid_stem, *stems]
            desired_stem = stems[0] if stems else f"{base_stem}_{part}"
            candidates = _armor_related_texture_candidates(source_roots, stems, part)
            source = candidates[0] if candidates else None
            target_dir = texture_root / "item"
            target_dir.mkdir(parents=True, exist_ok=True)
            if source:
                chosen = source.stem
                target = target_dir / f"{chosen}.png"
                if source.resolve() != target.resolve():
                    if target.exists() and target.read_bytes() != source.read_bytes():
                        target = target_dir / f"{registry}_{chosen}.png"
                        chosen = target.stem
                    _copy_texture_with_metadata(source, target)
                else:
                    _copy_texture_with_metadata(source, target)
                part_texture[part] = chosen
                part_outputs.append(str(target))
            else:
                chosen = desired_stem
                target = target_dir / f"{chosen}.png"
                if not target.exists():
                    _write_placeholder_png(target)
                part_texture[part] = chosen
                part_outputs.append(str(target))
                warnings.append(f"No exact armor item texture found for {part}; generated a checker placeholder '{chosen}.png'.")
        # Recover the actual armor layer prefix from the legacy models/armor texture pair, e.g.
        # `ultimate__layer_1.png` + `ultimate__layer_2.png` -> armorTextureFile `ultimate_`.
        layer_prefix_candidates = [base_stem]
        for value in part_texture.values():
            compact = _normalise_texture_stem(value)
            compact = re.sub(r"_(?:helmet|helm|head|body|chest|chestplate|torso|leggings|legs|pants|boots|boot|feet|shoes)$", "", compact, flags=re.IGNORECASE)
            if compact: layer_prefix_candidates.append(compact)
        layer_stem_candidates = []
        for base in dict.fromkeys(layer_prefix_candidates):
            layer_stem_candidates.extend(_armor_layer_texture_stems(base, 1))
        layer_two_stem_candidates = []
        for base in dict.fromkeys(layer_prefix_candidates):
            layer_two_stem_candidates.extend(_armor_layer_texture_stems(base, 2))
        layer_one = _exact_texture_candidates(source_roots, list(dict.fromkeys(layer_stem_candidates)), ("models/armor", "armor"))
        layer_two = _exact_texture_candidates(source_roots, list(dict.fromkeys(layer_two_stem_candidates)), ("models/armor", "armor"))
        # Standard armor layers are named ``<prefix>_layer_1.png`` and MCreator
        # adds that suffix itself. A trailing underscore is only correct for
        # legacy assets that intentionally use a double underscore.
        layer_prefix = base_stem
        layer_outputs: list[str] = []
        if layer_one:
            layer_stem = layer_one[0].stem
            match = re.match(r"(.+?)(_+layer_1|_1)$", layer_stem, re.IGNORECASE)
            if match:
                # MCreator appends ``_layer_1.png`` itself. Preserve only an
                # extra separator from names such as ``ultimate__layer_1``;
                # standard ``ruby_layer_1`` must use ``ruby`` here.
                separator = match.group(2)
                layer_prefix = match.group(1) + ("_" if separator.startswith("__") else "")
            target_dir = texture_root / "models" / "armor"
            target_dir.mkdir(parents=True, exist_ok=True)
            for layer_number, source in ((1, layer_one[0]), (2, layer_two[0] if layer_two else None)):
                if source is None:
                    continue
                # MCreator's generator appends `_layer_1/2.png` to this prefix.
                # Rename legacy `<prefix>_1/2.png` assets into that convention.
                target = target_dir / f"{layer_prefix}_layer_{layer_number}.png"
                if source.resolve() != target.resolve():
                    if not target.exists() or target.read_bytes() != source.read_bytes():
                        _copy_texture_with_metadata(source, target)
                    else:
                        _copy_texture_with_metadata(source, target)
                layer_outputs.append(str(target))
                # Keep a second copy under the legacy/source path as well. Some generated
                # NeoForge item/armor renderers use the literal texture path from the
                # original ItemArmor#getArmorTexture implementation rather than the
                # MCreator convention. This makes both paths resolve to the exact pixels.
                legacy_target = texture_root / "armor" / source.name
                legacy_target.parent.mkdir(parents=True, exist_ok=True)
                _copy_texture_with_metadata(source, legacy_target)
        else:
            warnings.append(f"No legacy armor layer texture pair found for '{base_stem}'; armorTextureFile defaults to '{layer_prefix}'.")
        texture = base_stem
        texture_output = part_outputs[0] if part_outputs else ""
        updates: dict[str, Any] = {
            "creativeTabs": ["COMBAT"],
            "armorTextureFile": layer_prefix,
            "maxDamage": int((recovered_stats or {}).get("durability", 25)),
            "damageValueHelmet": int((recovered_stats or {}).get("helmet", 6)),
            "damageValueBody": int((recovered_stats or {}).get("body", 8)),
            "damageValueLeggings": int((recovered_stats or {}).get("leggings", 6)),
            "damageValueBoots": int((recovered_stats or {}).get("boots", 6)),
            "enchantability": int((recovered_stats or {}).get("enchantability", 10)),
            "toughness": 0.0,
            "rarity": "COMMON",
            "repairItems": [],
        }
        updates.update(sound_updates)
        readable_material = _translated_name(language_names or {}, prefix, _display_name(_snake(prefix)))
        part_name = {
            part: _translated_name(
                language_names or {},
                pieces.get(part, f"{prefix}_{part}"),
                f"{readable_material} " + {
                    "helmet": "Helmet",
                    "body": "Chestplate",
                    "leggings": "Leggings",
                    "boots": "Boots",
                }[part],
            )
            for part in ("helmet", "body", "leggings", "boots")
        }
        for part in ("helmet", "body", "leggings", "boots"):
            enabled = part in pieces
            updates[f"enable{part.capitalize()}"] = enabled
            if enabled:
                updates[f"texture{part.capitalize()}"] = part_texture[part]
                updates[f"{part}Name"] = part_name[part]
            else:
                updates[f"texture{part.capitalize()}"] = ""
                updates[f"{part}Name"] = ""
        # Recover the custom ModelBiped/ModelRenderer implementation used by
        # legacy armor items. MCreator's 1.20.1 armor generator binds the public
        # head/body/arm/leg ModelPart groups to the armor pieces, preserving the
        # same body-part attachment and animation behavior as the original model.
        model_key = _compact_registry(prefix)
        model_class = custom_armor_models.get(model_key)
        if model_class:
            if model_class not in armor_model_cache:
                armor_model_cache[model_class] = _write_custom_armor_model(source_roots, model_class, output_elements.parent, modid)
            attached, resolved_model, model_warnings = armor_model_cache[model_class]
            warnings.extend(model_warnings)
            if attached:
                updates.update({
                    "helmetModelName": resolved_model if "helmet" in pieces else "Default",
                    "helmetModelPart": "head",
                    "bodyModelName": resolved_model if "body" in pieces else "Default",
                    "bodyModelPart": "body",
                    "armsModelPartL": "left_arm",
                    "armsModelPartR": "right_arm",
                    "leggingsModelName": resolved_model if "leggings" in pieces else "Default",
                    "leggingsModelPartL": "left_leg",
                    "leggingsModelPartR": "right_leg",
                    "bootsModelName": resolved_model if "boots" in pieces else "Default",
                    "bootsModelPartL": "left_leg",
                    "bootsModelPartR": "right_leg",
                })
        base_fv, base_definition, _ = _existing_element_definition(workspace_root=output_elements.parent, element_entry=existing_entry, fallback_fv=fv, fallback_definition=template)
        element = _new_element(base_fv, base_definition, element_name, registry, "armor", updates)
        path = output_elements / f"{element_name}.mod.json"
        path.write_text(json.dumps(element, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        _upsert_workspace_element(workspace, existing_entry, element_name, "armor", registry, import_modid, registry)
        lang_map = workspace.setdefault("language_map", {}).setdefault("en_us", {})
        armor_display = _display_name(_snake(prefix))
        lang_map[f"item.{modid}.{registry}"] = armor_display
        for _part_key in ("helmet", "body", "leggings", "boots"):
            _part_name = part_texture.get(_part_key, "") or f"{_snake(prefix)}_{_part_key}"
            _part_label = armor_display + " " + _part_key.capitalize()
            lang_map[f"item.{modid}.{registry}_{_part_key}"] = _part_label
            lang_map[f"item.{modid}.{registry}_{_part_key}.name"] = _part_label
        results.append(MCreatorContentExport(element_name, registry, "armor", registry_source.name, status, warnings, texture, texture_output, sound_bindings))
        _log(log, f"MCreator content: armor {element_name} <- {registry_source.name}")
    return results
def _find_registry_sources(graph: LegacyModGraph) -> list[Path]:
    """Find likely content initializer/registry classes without mod-specific names.
    Both compiled classes and decompiled Java are considered.  When both are
    present, the compiled class is preferred for bytecode-only constructor
    details, while Java remains available as the authoritative source for
    registry names and metadata.
    """
    candidates: list[tuple[int, Path]] = []
    exact = {
        "main", "modmain", "registryhandler", "moditems", "modarmor", "modblocks",
        "iteminit", "blockinit", "armorinit", "entityinit", "contentinit",
    }
    paths: dict[str, list[Path]] = {}
    for path in graph.class_files.values():
        paths.setdefault(path.stem.lower().split("$")[0], []).append(path)
    for source_root in graph.roots:
        for path in source_root.rglob("*.java"):
            paths.setdefault(path.stem.lower().split("$")[0], []).append(path)
    for stem, variants in paths.items():
        score: int | None = None
        if stem in exact:
            score = 0
        elif "registry" in stem or stem.endswith("main"):
            score = 1
        elif stem.endswith("init") and any(token in stem for token in ("item", "block", "armor", "content", "entity")):
            score = 2
        elif stem.endswith(("items", "blocks", "armors")):
            score = 3
        if score is None:
            for variant in variants:
                if variant.suffix.lower() != ".java":
                    continue
                try:
                    text = variant.read_text(encoding="utf-8-sig", errors="ignore")
                except OSError:
                    continue
                if ("DeferredRegister" in text or "RegistryObject<" in text) and re.search(r"\.register\s*\(", text):
                    score = 2
                    break
        if score is None:
            continue
        # Prefer a compiled class when the same registry helper exists in both forms.
        variants.sort(key=lambda path: (0 if path.suffix.lower() == ".class" else 1, str(path).lower()))
        for path in variants:
            candidates.append((score, path))
    candidates.sort(key=lambda item: (item[0], str(item[1]).lower()))
    return [path for _, path in candidates]
def _strip_java_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return re.sub(r"//[^\n\r]*", "", text)
def _source_instantiated_classes(root: Path) -> set[str]:
    """Return concrete class names constructed by another source class.
    Some legacy projectile/effect entities are intentionally transient and are
    never added to the global entity registry.  They are still real content when
    weapons or mobs instantiate them.  Conversely, reusable throwable base
    classes are usually never instantiated directly.
    """
    result: set[str] = set()
    for path in Path(root).rglob("*.java"):
        try:
            clean = _strip_java_comments(path.read_text(encoding="utf-8-sig", errors="ignore"))
        except OSError:
            continue
        declaration = re.search(r"\bclass\s+([A-Za-z_$][\w$]*)", clean)
        owner = declaration.group(1) if declaration else path.stem
        for match in re.finditer(r"\bnew\s+([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*)\s*\(", clean):
            simple = match.group(1).split(".")[-1]
            if simple != owner:
                result.add(simple)
    return result
def _finalize_ore_generation_elements(source_roots: list[Path], output_elements: Path, source_worldgen_map: dict[str, dict[str, Any]], exports: list[MCreatorContentExport]) -> None:
    """Reassert source-authored ore/plant generation after all element writers run."""
    for export in exports:
        if export.status != "created" or export.element_type not in {"block", "plant"}:
            continue
        path = output_elements / f"{export.element_name}.mod.json"
        data = _read_json(path)
        if not isinstance(data, dict) or not isinstance(data.get("definition"), dict):
            continue
        definition = data["definition"]
        gen = source_worldgen_map.get(_compact_registry(export.registry_name))
        if export.element_type == "block" and gen:
            definition["generateFeature"] = True
            definition["restrictionBiomes"] = []
            definition["generationShape"] = str(gen.get("generationShape") or "UNIFORM")
            definition["frequencyPerChunks"] = max(1, int(gen.get("frequencyPerChunks", 1)))
            definition["frequencyOnChunk"] = max(1, int(gen.get("frequencyOnChunk", 1)))
            definition["minGenerateHeight"] = int(gen.get("minGenerateHeight", 0))
            definition["maxGenerateHeight"] = int(gen.get("maxGenerateHeight", 64))
            definition["blocksToReplace"] = list(gen.get("blocksToReplace", []))
        elif export.element_type == "plant":
            definition["generateFeature"] = True
            definition["restrictionBiomes"] = []
        path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def _finalize_exact_block_resources(source_roots: list[Path], output_elements: Path, workspace_root: Path, import_modid: str, target_modid: str, exports: list[MCreatorContentExport]) -> None:
    """Restore exact source Mojmap blockstate/model JSON after element generation."""
    seen: set[str] = set()
    for export in exports:
        if export.status != "created" or export.element_type not in {"block", "plant"}:
            continue
        key = _snake(export.registry_name)
        if key in seen:
            continue
        seen.add(key)
        try:
            _copy_source_block_model_resources(source_roots, import_modid, target_modid, key, workspace_root)
        except Exception:
            pass


def _finalize_ranged_projectiles(source_roots: list[Path], output_elements: Path, exports: list[MCreatorContentExport]) -> None:
    """Bind source custom projectile classes to native MCreator ranged items."""
    class_to_element = {str(e.source_class).split(".")[-1]: e.element_name for e in exports if e.status == "created" and e.element_type == "projectile"}
    if not class_to_element:
        return
    for export in exports:
        if export.status != "created" or export.element_type not in {"item", "tool"}:
            continue
        path = output_elements / f"{export.element_name}.mod.json"
        data = _read_json(path)
        if not isinstance(data, dict) or not isinstance(data.get("definition"), dict):
            continue
        definition = data["definition"]
        if not definition.get("enableRanged"):
            continue
        source_text = ""
        for root in source_roots:
            for java in root.rglob(f"{export.source_class.split('.')[-1]}.java"):
                try:
                    source_text = java.read_text(encoding="utf-8-sig", errors="replace")
                except OSError:
                    source_text = ""
                if source_text:
                    break
            if source_text:
                break
        if not source_text:
            continue
        for cls in re.findall(r"\bnew\s+([A-Za-z_$][\w$]*(?:Arrow|Projectile|Bolt|Shot))\s*\(", source_text):
            if cls in class_to_element:
                definition["projectile"] = f"CUSTOM:{class_to_element[cls]}"
                break
        path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

def export_mcreator_content(
    graph: LegacyModGraph,
    workspace: dict[str, Any],
    workspace_root: Path,
    output_elements: Path,
    texture_root: Path,
    texture_source_roots: list[Path] | None = None,
    modid: str = "converted_mod",
    import_modid: str = "converted_mod",
    log: Optional[Callable[[str], None]] = None,
) -> MCreatorContentResult:
    result = MCreatorContentResult()
    emitted_type_registries: set[tuple[str, str]] = set()
    # Preload compiled bytecode in bounded javap batches so a full-mod scan does not
    # spawn one JVM-inspection process per item/block/helper class.
    try:
        cached_count, fallback_count = prime_javap_cache(list(graph.class_files.values()))
        _log(log, f"MCreator content bytecode preload: {cached_count} cached, {fallback_count} fallback.")
    except Exception as exc:
        result.warnings.append(f"Bytecode preload warning: {exc}")
    output_elements.mkdir(parents=True, exist_ok=True)
    texture_root.mkdir(parents=True, exist_ok=True)
    source_roots = list(graph.roots)
    source_texture_roots = list(graph.roots)
    for root in (texture_source_roots or []):
        root = Path(root)
        if root not in source_roots:
            source_roots.append(root)
    template_defs: dict[str, tuple[int, dict[str, Any]]] = {}
    for element_type in ("item", "tool", "armor", "block", "dimension", "biome", "plant", "projectile"):
        try:
            template_defs[element_type] = _template_definition(workspace_root, element_type)
        except Exception as exc:
            result.warnings.append(f"No MCreator template schema available for {element_type}: {exc}")
    used_names, used_registries = _element_names(workspace)
    existing_by_registry = _workspace_elements_by_registry(workspace)
    existing_by_compact_registry = _workspace_elements_by_compact_registry(workspace)
    entity_classes = set(graph.entities)
    renderer_classes = {r.renderer_class for r in graph.registrations}
    model_classes = {r.model_class for r in graph.registrations if r.model_class}
    skip_simple = {"ClientProxy", "CommonProxy", "ServerProxy", "RegistryHandler"}
    # Registry sources are the source of truth for legacy item/block/armor names. Cache only
    # those files, not every model/helper class, so full-mod scans remain practical.
    # Both Java and class representations are accepted; compiled bytecode is preferred
    # when both exist, while decompiled Java supplies authoritative explicit names.
    registry_sources = _find_registry_sources(graph)
    registry_texts: dict[Path, str] = {}
    compiled_registry_sources = [path for path in registry_sources if path.suffix.lower() == ".class"]
    if compiled_registry_sources:
        prime_javap_cache(compiled_registry_sources)
    for registry_source in registry_sources:
        try:
            if registry_source.suffix.lower() == ".java":
                registry_texts[registry_source] = registry_source.read_text(encoding="utf-8-sig", errors="replace")
            else:
                registry_texts[registry_source] = _run_javap(registry_source)
        except Exception as exc:
            result.warnings.append(f"Could not inspect registry source {registry_source.name}: {exc}")
    registry_name_map: dict[str, set[str]] = {}
    constructor_records_by_class: dict[str, list[dict[str, Any]]] = {}
    source_registry_map: dict[str, set[str]] = {}
    source_field_registry_map: dict[str, set[str]] = {}
    source_instantiated_classes: set[str] = set()
    source_worldgen_map: dict[str, dict[str, Any]] = {}
    source_creative_tabs: dict[str, str] = {}
    source_registry_tabs: dict[str, str] = {}
    for source_root in graph.roots:
        for name, values in _source_registered_names(source_root).items():
            source_registry_map.setdefault(name, set()).update(values)
        for name, values in _source_field_registered_names(source_root).items():
            source_field_registry_map.setdefault(name, set()).update(values)
        source_instantiated_classes.update(_source_instantiated_classes(source_root))
        source_worldgen_map.update(_source_worldgen_by_registry(source_root))
        source_creative_tabs.update(_source_creative_tabs(source_root))
        source_registry_tabs.update(_source_registry_creative_tabs(source_root))
    language_names = _load_language_names(graph.roots)
    source_custom_tabs: dict[str, dict[str, str]] = {}
    source_armor_set_classes: dict[str, str] = {}
    source_dimension_biomes: dict[str, str] = {}
    for source_root in graph.roots:
        source_custom_tabs.update(_source_custom_tabs(source_root, language_names))
        source_armor_set_classes.update(_source_armor_set_classes(source_root))
        source_dimension_biomes.update(_source_dimension_biomes(source_root))
    dimension_exports_by_class: dict[str, MCreatorContentExport] = {}
    custom_tab_members: dict[str, list[str]] = {}
    custom_tab_plan: dict[str, dict[str, Any]] = {}
    held_renderer_specs = _held_renderer_specs(graph.roots, registry_texts)
    bytecode_tool_materials, bytecode_armor_stats, bytecode_registry_names = _bytecode_equipment_stats(registry_texts)
    source_tool_materials = bytecode_tool_materials
    source_tool_materials.update(_source_tool_materials(graph.roots))
    recovered_armor_stats = bytecode_armor_stats
    recovered_armor_stats.update(_source_armor_stats(graph.roots))
    for field_name, spec in sorted(source_custom_tabs.items()):
        tab_registry = _snake(spec.get("label_key", field_name))
        existing_tab = _workspace_element_for_type(workspace, "tab", tab_registry)
        if existing_tab:
            element_name = str(existing_tab.get("name") or (_pascal(tab_registry) + "Tab"))
        else:
            clean_label = re.sub(r"^tab[_-]?", "", spec.get("label_key", field_name), flags=re.IGNORECASE)
            element_name = _unique_name(_pascal(clean_label) + "Tab", used_names)
        custom_tab_plan[field_name] = {
            **spec,
            "registry": tab_registry,
            "element_name": element_name,
            "existing": existing_tab,
        }
    for registry_source, registry_text in registry_texts.items():
        try:
            extracted = _extract_registered_names(registry_text)
            for simple_name, names in extracted.items():
                registry_name_map.setdefault(simple_name, set()).update(names)
        except Exception:
            pass
        try:
            extracted_records = _extract_constructor_records(registry_text)
            for simple_name, records in extracted_records.items():
                constructor_records_by_class.setdefault(simple_name, []).extend(records)
            if not re.search(r"^\s*\d+:\s+", registry_text, re.MULTILINE):
                source_records = _extract_source_constructor_records(registry_text)
                for simple_name, records in source_records.items():
                    constructor_records_by_class.setdefault(simple_name, []).extend(records)
        except Exception:
            pass
    # Explicit compiled GameRegistry calls are stronger than nearby
    # unlocalized/icon strings recovered by the older per-constructor scanner.
    for simple_name, names in bytecode_registry_names.items():
        if names:
            registry_name_map[simple_name] = set(names)
    # Decompiled source is authoritative when it directly exposes constructor or
    # registration keys.  This replaces weaker bytecode guesses such as
    # unlocalized names, rather than unioning them and creating duplicate elements.
    for simple_name, names in source_registry_map.items():
        if names:
            registry_name_map[simple_name] = set(names)
    # Armor groups are best recovered from registry declarations; do this first so individual
    # ItemArmor classes don't become four separate generic items.
    for registry_source in registry_sources:
        if not template_defs.get("armor"):
            continue
        for export in _armor_elements_from_registry(
            registry_source,
            template_defs["armor"][0],
            template_defs["armor"][1],
            used_names,
            used_registries,
            texture_root,
            source_roots,
            output_elements,
            workspace,
            modid,
            log,
            recovered_armor_stats,
            language_names,
            import_modid=import_modid,
        ):
            export_key = (export.element_type.lower(), _compact_registry(export.registry_name))
            if export_key in emitted_type_registries:
                continue
            emitted_type_registries.add(export_key)
            result.exports.append(export)
            existing_by_registry[export.registry_name.lower()] = next(
                (entry for entry in workspace.get("mod_elements", []) if str(entry.get("registry_name", "")).lower() == export.registry_name.lower()),
                existing_by_registry.get(export.registry_name.lower(), {}),
            )
            if export.status == "created":
                armor_prefix = re.sub(r"armor$", "", _compact_registry(export.registry_name))
                armor_class = source_armor_set_classes.get(armor_prefix, "")
                recovered_tab = _resolved_creative_tab(armor_class, graph, source_creative_tabs) if armor_class else ""
                if recovered_tab.startswith("SOURCE_TAB:"):
                    field_name = recovered_tab.split(":", 1)[1]
                    plan = custom_tab_plan.get(field_name)
                    element_path = output_elements / f"{export.element_name}.mod.json"
                    data = _read_json(element_path)
                    if plan and data and isinstance(data.get("definition"), dict):
                        data["definition"]["creativeTabs"] = [f"CUSTOM:{plan['element_name']}"]
                        element_path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
                        custom_tab_members.setdefault(field_name, []).append(export.element_name)
    # Build the set of concrete armor-piece registries so those classes do not
    # become duplicate generic item elements alongside the single MCreator armor
    # element. The armor element owns helmet/body/leggings/boots internally.
    armor_piece_registries: set[str] = set()
    for _registry_source in registry_sources:
        try:
            _armor_text = _registry_source.read_text(encoding="utf-8-sig", errors="replace") if _registry_source.suffix.lower() == ".java" else _run_javap(_registry_source)
            for _prefix, _pieces in _group_armor_fields(_armor_text).items():
                for _part, _field_name in _pieces.items():
                    if _field_name:
                        armor_piece_registries.add(_compact_registry(_field_name))
                    armor_piece_registries.add(_compact_registry(f"{_prefix}_{_part}"))
        except Exception:
            continue

    # Build one source-of-truth view of content classes.  Compiled classes win when a
    # class exists in both representations, but Java-only mods remain fully supported.
    content_sources: dict[str, Path] = {}
    for simple, source in graph.class_files.items():
        content_sources[_simple(simple)] = source
    for source_root in graph.roots:
        for source in sorted(source_root.rglob("*.java"), key=lambda path: str(path).lower()):
            simple, _superclass, _flags, _text = _source_class_header_details(source)
            if simple and simple not in content_sources:
                content_sources[simple] = source
    for simple, source in sorted(content_sources.items(), key=lambda item: str(item[0]).lower()):
        clean_simple = _simple(simple)
        if clean_simple in skip_simple or clean_simple in entity_classes or clean_simple in renderer_classes or clean_simple in model_classes:
            continue
        if clean_simple.lower().startswith(("model", "render")):
            continue
        try:
            if source.suffix.lower() == ".java":
                _header_simple, _header_super, access_flags, _source_text = _source_class_header_details(source)
            else:
                _header_simple, _header_super, access_flags = _class_header_details(source)
            # ACC_INTERFACE | ACC_ABSTRACT. Abstract framework/base classes are
            # implementation details, not independently registered content.
            if access_flags & 0x0600:
                continue
            kind, actual_simple, text = _classify(source)
            # Modern/legacy custom bows often extend Item rather than BowItem.  Their
            # clean source still exposes UseAnim.BOW/getUseAnimation.  They must use
            # MCreator's Item schema so enableRanged/projectile/animation fields exist.
            if kind == "tool" and re.search(r"\bUseAnim\.BOW\b|\bgetUseAnimation\s*\([^)]*\)\s*\{[^{}]*UseAnim\.BOW", text, re.DOTALL):
                kind = "item"
        except Exception:
            continue
        # Bytecode is preferred for executable constants and inheritance, but
        # javap output does not reproduce Java string concatenations, tooltip
        # calls, explicit texture names, or readable registry metadata. Merge
        # the decompiled companion into the analysis text whenever available.
        java_companion = graph.java_files.get(actual_simple) or graph.java_files.get(clean_simple)
        if java_companion is not None and java_companion.resolve() != source.resolve():
            try:
                java_text = java_companion.read_text(encoding="utf-8-sig", errors="replace")
            except OSError:
                java_text = ""
            if java_text:
                text = text + "\n\n/* MojModelBridge decompiled companion */\n" + java_text
        # Biomes and dimensions are intentionally not ported. Their legacy
        # provider/worldgen/village semantics are disabled; ore generation is
        # recovered separately from exact ore feature resources.
        if kind in {"biome", "dimension"}:
            continue
        if kind == "unknown" or kind == "armor_piece":
            continue
        # Avoid exporting obvious abstract/base classes as user-facing elements.
        if re.search(r"(?i)(base|abstract|proxy|renderer|model|goal|ai|handler|proxy)$", actual_simple):
            continue
        registry_names = sorted(registry_name_map.get(actual_simple, set()))
        if kind == "projectile":
            # Throwable/arrow framework bases are common and often concrete at the
            # bytecode level, but only registered projectile entities are content.
            # Entity registry metadata is authoritative for this category.
            entity_registry = graph.registry_names.get(actual_simple, "")
            if entity_registry:
                registry_names = [entity_registry]
            elif actual_simple in source_instantiated_classes:
                # Transient projectile/effect entities can be valid without a
                # global EntityRegistry entry. Their class name is the most
                # faithful available registry identity for the target element.
                registry_names = [_snake(actual_simple)]
            elif not registry_names:
                continue
        if not registry_names:
            if kind in {"item", "tool", "block", "plant", "armor_piece"}:
                continue
            fallback_registry = _snake(re.sub(r"^(Item|Block|WorldProvider|BiomeGen)", "", actual_simple)) or _snake(actual_simple)
            registry_names = [fallback_registry]
        for raw_registry in registry_names:
            source_registry = _normalise_source_registry(actual_simple, raw_registry)
            registry = source_registry
            # Legacy spawn eggs are represented by the living-entity element's
            # spawnEggTexture in the target schema. Do not create a second generic
            # item element for egg<entity>; only skip it when a matching entity is
            # actually registered in the clean source.
            if kind in {"item", "tool"}:
                compact_reg = _compact_registry(registry)
                if compact_reg.startswith("egg") and len(compact_reg) > 3:
                    egg_target = compact_reg[3:]
                    entity_targets = set()
                    for _meta in graph.entities.values():
                        entity_targets.add(_compact_registry(getattr(_meta, "registry_name", "")))
                        entity_targets.add(_compact_registry(getattr(_meta, "class_name", "")))
                    if egg_target in entity_targets:
                        continue
            if kind in {"item", "tool"} and _compact_registry(registry) in armor_piece_registries:
                # Armor pieces are represented by the single MCreator armor element;
                # exporting them again as generic items produces duplicate inventory
                # variants and steals the intended language/registry name.
                continue
            export_key = (kind.lower(), _compact_registry(registry))
            if export_key in emitted_type_registries:
                continue
            emitted_type_registries.add(export_key)
            existing_entry = _workspace_element_for_type(workspace, kind, registry)
            if existing_entry:
                existing_status = _workspace_entry_status(existing_entry)
                same_source = str(existing_entry.get("mojmodelbridge_source_modid", "")).lower() == import_modid.lower()
                if existing_status == "resumed" or same_source:
                    existing_name = str(existing_entry.get("name") or _element_name_from_class(actual_simple, registry))
                    result.exports.append(MCreatorContentExport(existing_name, str(existing_entry.get("registry_name") or registry), kind, str(source), "resumed", []))
                    _log(log, f"MCreator content: resumed {kind} {existing_name} <- {source.name}")
                    continue
                # A pre-existing template element may use the same registry for an
                # unrelated vanilla/example object. Never let it swallow the imported
                # source content; namespace only the workspace registry on collision.
                registry = _snake(f"{import_modid}_{registry}")
                existing_entry = None
            desired_name = _element_name_from_class(actual_simple, registry)
            element_name = _unique_name(desired_name, used_names)
            used_registries.add(registry.lower())
            status = "created"
            warnings: list[str] = []
            if kind not in template_defs:
                continue
            fallback_fv, fallback_template = template_defs[kind]
            base_fv, base_template, _ = _existing_element_definition(
                workspace_root=workspace_root,
                element_entry=existing_entry,
                fallback_fv=fallback_fv,
                fallback_definition=fallback_template,
            )
            if existing_entry is None:
                base_template = _sanitize_new_import_definition(base_template, kind)
            preferred_dirs = {
                "item": ("item", "items"),
                "tool": ("item", "items"),
                "block": ("block", "blocks"),
                "plant": ("block", "blocks", "plant"),
            }.get(kind, ("item", "items"))
            png_refs = _declared_texture_refs(text)
            exact_item_model_ref = _item_model_texture_ref(source_texture_roots, registry) if kind in {"item", "tool"} else None
            source_stems = [Path(exact_item_model_ref.split(":",1)[-1]).stem] if exact_item_model_ref else [Path(ref).stem for ref in png_refs]
            stem_candidates = [*source_stems, actual_simple, source_registry, re.sub(r"^(Item|Block)", "", actual_simple)]
            if exact_item_model_ref:
                png_refs = [exact_item_model_ref, *png_refs]
            output_subdir = {"item": "item", "tool": "item", "block": "block", "plant": "block"}.get(kind, "")
            if kind == "projectile":
                # Default MCreator projectiles render their bound item. A custom
                # model texture without a custom model is ignored and creates a
                # misleading missing-resource warning.
                texture, texture_output, tex_warnings = "", "", []
            else:
                texture, texture_output, tex_warnings = _copy_or_placeholder_texture(
                    source_texture_roots,
                    texture_root,
                    registry,
                    stem_candidates,
                    preferred_dirs,
                    output_subdir=output_subdir,
                    allow_placeholder=kind in {"item", "tool", "block", "plant"},
                )
            warnings.extend(tex_warnings)
            sound_updates, sound_bindings = infer_sound_bindings(text, registry, kind, workspace)
            if kind == "item":
                element = _item_element(
                    source,
                    actual_simple,
                    _class_header_info_any(source)[1],
                    text,
                    registry,
                    base_fv,
                    base_template,
                    texture,
                    constructor_records_by_class.get(actual_simple, []),
                )
                if element.get("definition", {}).get("isFood"):
                    if "nutritionalValue" not in element.get("definition", {}):
                        warnings.append("Food was detected but exact nutrition/saturation values could not be recovered; verify the generated food properties.")
            elif kind == "tool":
                tool_type = _classify_tool(_class_header_info_any(source)[1], actual_simple) or "Sword"
                material_for_tool = source_tool_materials.get(_compact_registry(registry))
                # Do not attach unrelated material stats to custom Bow/FishingRod Items;
                # these source classes often have their own durability/enchantability and
                # are intentionally not Tier-backed. Sword/Axe/Pickaxe/etc. retain Tier data.
                tier_backed = bool(re.search(r"\bTier\b", text)) or tool_type in {"Sword", "Axe", "Pickaxe", "Shovel", "Hoe"}
                if tool_type in {"Bow", "Fishing Rod"} and not tier_backed:
                    material_for_tool = None
                element, tool_warnings = _tool_element(
                    actual_simple,
                    text,
                    registry,
                    base_fv,
                    base_template,
                    texture,
                    tool_type,
                    material_for_tool,
                    source_tool_materials if tier_backed else None,
                )
                warnings.extend(tool_warnings)
            # Source callbacks such as onCraftedBy(ItemStack, ...) are executable
            # behavior, not merely metadata. Convert their statically recoverable
            # enchant calls into the native MCreator on-crafted procedure format.
            if kind in {"item", "tool"}:
                try:
                    procedure_name = _add_crafted_enchantment_procedure(
                        element, text, element_name, output_elements, workspace, used_names, import_modid, source_roots
                    )
                    if procedure_name:
                        warnings.append(f"Recovered crafted enchantments into procedure '{procedure_name}'.")
                except Exception as exc:
                    warnings.append(f"Could not export crafted enchantments: {exc}")
            elif kind == "block":
                generation = source_worldgen_map.get(_compact_registry(source_registry), {})
                model_info = _mcreator_block_model_info(source_roots, import_modid, source_registry)
                model_path = Path(model_info.get("modelPath")) if model_info.get("modelPath") else None
                raw_block_texture = str(model_info.get("texture") or "")
                raw_top = str(model_info.get("textureTop") or raw_block_texture)
                raw_bottom = str(model_info.get("textureBottom") or raw_block_texture)
                raw_left = str(model_info.get("textureLeft") or raw_block_texture)
                raw_front = str(model_info.get("textureFront") or raw_block_texture)
                raw_right = str(model_info.get("textureRight") or raw_block_texture)
                raw_back = str(model_info.get("textureBack") or raw_block_texture)
                block_texture = _copy_model_ref_texture_to_workspace(model_path, raw_block_texture, texture_root, "block", registry) or texture
                block_top = _copy_model_ref_texture_to_workspace(model_path, raw_top, texture_root, "block", registry) or block_texture
                block_bottom = _copy_model_ref_texture_to_workspace(model_path, raw_bottom, texture_root, "block", registry) or block_texture
                block_left = _copy_model_ref_texture_to_workspace(model_path, raw_left, texture_root, "block", registry) or block_texture
                block_front = _copy_model_ref_texture_to_workspace(model_path, raw_front, texture_root, "block", registry) or block_texture
                block_right = _copy_model_ref_texture_to_workspace(model_path, raw_right, texture_root, "block", registry) or block_texture
                block_back = _copy_model_ref_texture_to_workspace(model_path, raw_back, texture_root, "block", registry) or block_texture
                render_type = int(model_info.get("renderType") or 11)
                custom_model_name = str(model_info.get("customModelName") or ("Single texture" if render_type == 11 else "Normal"))
                if render_type == 11:
                    updates_textures = {
                        "texture": block_texture,
                        "textureTop": "", "textureBottom": "", "textureLeft": "",
                        "textureFront": "", "textureRight": "", "textureBack": "",
                        "itemTexture": "", "particleTexture": "",
                    }
                else:
                    updates_textures = {
                        "texture": block_texture,
                        "textureTop": block_top,
                        "textureBottom": block_bottom,
                        "textureLeft": block_left,
                        "textureFront": block_front,
                        "textureRight": block_right,
                        "textureBack": block_back,
                        "itemTexture": "",
                        "particleTexture": "",
                    }
                # Preserve exact modern model/state resources. Vanilla Mojmap JSON is already
                # compatible with the 1.20.1/1.21.1 resource format.
                try:
                    _copy_source_block_model_resources(source_roots, str(import_modid), str(modid), registry, workspace_root)
                except Exception as exc:
                    warnings.append(f"Could not preserve exact source blockstate/model resources: {exc}")
                exact_source_worldgen = bool(generation.get("sourceResourceExact"))
                generation_enabled = bool(generation.get("generateFeature")) and (
                    exact_source_worldgen or bool(generation)
                )
                updates = {**updates_textures,
                    "name": _display_name(registry),
                    "creativeTabs": ["BUILDING_BLOCKS"],
                    "hardness": 1.0,
                    "resistance": 10.0,
                    "hasBlockItem": True,
                    "generateFeature": generation_enabled,
                    "restrictionBiomes": list(generation.get("restrictionBiomes", [])),
                    "generationShape": generation.get("generationShape", "UNIFORM"),
                    "frequencyPerChunks": int(generation.get("frequencyPerChunks", 1)),
                    "frequencyOnChunk": int(generation.get("frequencyOnChunk", 1)),
                    "minGenerateHeight": int(generation.get("minGenerateHeight", 0)),
                    "maxGenerateHeight": int(generation.get("maxGenerateHeight", 64)),
                    "blocksToReplace": list(generation.get("blocksToReplace", [])),
                }
                updates.update(sound_updates)
                updates.update(_block_custom_drop_updates(text, source_field_registry_map))
                updates["renderType"] = render_type
                if "customModelName" in base_template:
                    updates["customModelName"] = custom_model_name
                element = _new_element(base_fv, base_template, element_name, registry, "block", updates)
                warnings.append("Custom block shapes, metadata-dependent collision boxes, and custom drops were not fully reconstructed.")
                if generation.get("approximation"):
                    warnings.append(str(generation["approximation"]))
                if generation.get("sourceResourceExact"):
                    warnings.append(f"Preserved source vein size metadata: {generation.get('veinSize', 0)}")
            elif kind == "plant":
                model_info = _normalize_block_model_info_textures(_mcreator_block_model_info(source_roots, import_modid, registry))
                model_path = Path(model_info.get("modelPath")) if model_info.get("modelPath") else None
                plant_raw = str(model_info.get("texture") or "")
                plant_texture = _copy_model_ref_texture_to_workspace(model_path, plant_raw, texture_root, "block", registry) or texture
                plant_updates = {
                    "renderType": 12,
                    "texture": plant_texture,
                    "customModelName": "Cross model",
                    "textureBottom": "",
                    "itemTexture": "",
                    "particleTexture": "",
                    "name": _display_name(registry),
                    "creativeTabs": ["DECORATIONS"],
                    "hardness": 0.0,
                    "resistance": 0.0,
                    "generationType": "custom",
                    "generateFeature": True,
                    "frequencyOnChunks": 1,
                    "generateAtAnyHeight": True,
                    "restrictionBiomes": [],
                }
                plant_updates.update(sound_updates)
                plant_updates.update(_block_custom_drop_updates(text, source_field_registry_map))
                element = _new_element(base_fv, base_template, element_name, registry, "plant", plant_updates)
                warnings.append("Growth behavior and custom placement rules were not fully reconstructed.")
            elif kind == "dimension":
                texture = ""
                texture_output = ""
                element = _new_element(
                    base_fv, base_template, element_name, registry, "dimension",
                    {
                        "name": _display_name(registry),
                        # Empty biome source lists produce an invalid multi-noise
                        # biome source in the 1.20.1 MCreator generator.  Plains is
                        # a safe, explicit fallback until source biome mapping is recovered.
                        "biomesInDimension": ["plains"],
                        "biomesInDimensionCaves": [],
                        "useCustomEffects": False,
                        "defaultEffects": "overworld",
                        "worldGenType": "Normal world gen",
                        "mainFillerBlock": "Blocks.STONE#0",
                        "fluidBlock": "Blocks.AIR",
                        "creativeTabs": ["TOOLS"],
                        "texture": "",
                    },
                )
                warnings.append("Dimension provider was detected, but chunk generation, biome selection, dimension ID, and custom effects require manual reconstruction.")
            elif kind == "biome":
                texture = ""
                texture_output = ""
                element = _new_element(
                    base_fv, base_template, element_name, registry, "biome",
                    {
                        "name": _display_name(registry),
                        "groundBlock": "Blocks.GRASS#0",
                        "undergroundBlock": "Blocks.STONE#0",
                        "spawnEntries": [],
                    },
                )
                warnings.append("Biome spawn lists and terrain generation settings were not fully reconstructed.")
            elif kind == "projectile":
                projectile_updates = {
                    "projectileItem": "",
                    "customModelTexture": "",
                    "modelWidth": 0.5,
                    "modelHeight": 0.5,
                }
                projectile_updates.update(sound_updates)
                element = _new_element(
                    base_fv, base_template, element_name, registry, "projectile", projectile_updates,
                )
                warnings.append("Projectile behavior and custom renderer logic require manual reconstruction.")
            else:
                continue
            display_name = _translated_name(language_names, registry, _display_name(registry))
            if isinstance(element.get("definition"), dict):
                if "name" in element["definition"]:
                    element["definition"]["name"] = display_name
                if "specialInformation" in element["definition"]:
                    tooltip_lines = _extract_tooltip_strings(text)
                    for line in _language_descriptions(graph.roots, registry):
                        if line not in tooltip_lines:
                            tooltip_lines.append(line)
                    if tooltip_lines:
                        element["definition"]["specialInformation"] = {"fixedValue": tooltip_lines}
                if kind in {"item", "tool", "block", "plant"} and "creativeTabs" in element["definition"]:
                    source_tab_field = source_registry_tabs.get(_compact_registry(registry), "")
                    recovered_tab = (
                        f"SOURCE_TAB:{source_tab_field}"
                        if source_tab_field
                        else _resolved_creative_tab(actual_simple, graph, source_creative_tabs, text)
                    )
                    if recovered_tab.startswith("SOURCE_TAB:"):
                        field_name = recovered_tab.split(":", 1)[1]
                        plan = custom_tab_plan.get(field_name)
                        if plan:
                            element["definition"]["creativeTabs"] = [f"CUSTOM:{plan['element_name']}"]
                            custom_tab_members.setdefault(field_name, []).append(element_name)
                    elif recovered_tab:
                        element["definition"]["creativeTabs"] = [recovered_tab]
                if kind in {"item", "tool"} and "renderType" in element["definition"]:
                    held_slug: Optional[str] = None
                    held_warnings: list[str] = []
                    item_model = _find_item_model_json(source_roots, registry)
                    if item_model is not None:
                        try:
                            held_slug, _held_outputs = _copy_resource_item_model_as_custom(
                                item_model, workspace_root, modid, registry
                            )
                        except Exception as exc:
                            held_warnings.append(f"Could not preserve item-model display transforms from {item_model.name}: {exc}")
                    if held_slug is None:
                        held_spec = held_renderer_specs.get(_compact_registry(registry))
                        if held_spec and held_spec.get("model_class"):
                            model_texture = Path(held_spec.get("texture_ref") or texture).stem or texture
                            held_slug, _held_outputs, extracted_warnings = _export_legacy_tool_model(
                                graph,
                                held_spec["model_class"],
                                model_texture,
                                workspace_root,
                                modid,
                                registry,
                                display=held_spec.get("display") or None,
                            )
                            held_warnings.extend(extracted_warnings)
                    if held_slug:
                        # MCreator's NeoForge 1.21.1 working custom-model format uses
                        # neoforge:separate_transforms to keep GUI/ground icons separate
                        # from the 3D held model. Generate that wrapper explicitly.
                        base_model_texture = ""
                        try:
                            custom_json = workspace_root / "src" / "main" / "resources" / "assets" / modid / "models" / "custom" / f"{held_slug}.json"
                            payload = json.loads(custom_json.read_text(encoding="utf-8"))
                            texmap = payload.get("textures", {}) if isinstance(payload, dict) else {}
                            if isinstance(texmap, dict):
                                base_model_texture = str(texmap.get("1") or next(iter(texmap.values()), ""))
                        except Exception:
                            base_model_texture = ""
                        _write_neoforge_separate_transform_item_model(workspace_root, modid, registry, held_slug, texture, base_model_texture)
                        element["definition"]["renderType"] = 1
                        element["definition"]["customModelName"] = f"{held_slug}:default"
                        # MCreator's custom item model generator uses guiTexture to keep the
                        # small inventory icon separate from the full held/tool model. The
                        # field is not present in every legacy schema template, but it is a
                        # valid definition property and is essential for 3D tools such as
                        # Big Bertha, Slice, Royal Guardian Sword, and Blades of Duality.
                        element["definition"]["guiTexture"] = texture
                    warnings.extend(held_warnings)
            # Newly created content is added to the workspace. Existing elements are skipped
            # earlier in the source-first check so the supplied workspace remains untouched.
            element_path = output_elements / f"{element_name}.mod.json"
            element_path.write_text(json.dumps(element, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
            folder_kind = None
            if kind == "item" and bool(element.get("definition", {}).get("isFood")):
                folder_kind = "food"
            elif kind == "tool":
                tool_type = str(element.get("definition", {}).get("toolType", "")).lower()
                if any(token in tool_type for token in ("sword", "bow", "crossbow", "trident", "mace", "weapon")):
                    folder_kind = "weapon"
            _upsert_workspace_element(workspace, existing_entry, element_name, kind, registry, import_modid, raw_registry, folder_kind)
            workspace.setdefault("language_map", {}).setdefault("en_us", {})[f"{('item' if kind in {'item','tool','armor'} else kind)}.{modid}.{registry}"] = display_name
            created_export = MCreatorContentExport(element_name, registry, kind, str(source), status, warnings, texture, texture_output, sound_bindings)
            result.exports.append(created_export)
            if kind == "dimension":
                dimension_exports_by_class[actual_simple] = created_export
            _log(log, f"MCreator content: {status} {kind} {element_name} <- {source.name}")
    # Final crafted-procedure attachment pass. This intentionally runs after every element
    # has been written so existing/resumed elements cannot lose the onCrafted binding.
    procedure_by_enchantments = {}
    for proc_path in output_elements.glob("*.mod.json"):
        try:
            payload = json.loads(proc_path.read_text(encoding="utf-8"))
        except Exception:
            continue
        if payload.get("_type") != "procedure":
            continue
        proc_def = payload.get("definition", {}) or {}
        ench = proc_def.get("mojmodelbridge_crafted_enchantments")
        if ench:
            procedure_by_enchantments[json.dumps(ench, sort_keys=True)] = proc_path.stem
    for element_path in output_elements.glob("*.mod.json"):
        try:
            payload = json.loads(element_path.read_text(encoding="utf-8"))
        except Exception:
            continue
        if payload.get("_type") not in {"item", "tool"}:
            continue
        definition = payload.get("definition", {}) or {}
        if definition.get("onCrafted"):
            continue
        ench = definition.get("mojmodelbridge_crafted_enchantments")
        if ench:
            proc = procedure_by_enchantments.get(json.dumps(ench, sort_keys=True))
            if proc:
                definition["onCrafted"] = proc
                payload["definition"] = definition
                element_path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    # Resource-only biomes/dimensions are intentionally disabled. Keep the helper
    # functions for regression/reference use, but never materialize those elements
    # during a conversion.
    # Dimension/biome linkage is intentionally disabled.
    # Materialize source-defined custom creative tabs only when at least one newly
    # converted element uses them. Existing template tabs remain untouched.
    exports_by_compact_registry = {
        _compact_registry(export.registry_name): export
        for export in result.exports
        if _compact_registry(export.registry_name)
    }
    for field_name, members in sorted(custom_tab_members.items()):
        if not members:
            continue
        plan = custom_tab_plan.get(field_name)
        if not plan:
            continue
        existing_tab = plan.get("existing")
        if existing_tab:
            result.exports.append(MCreatorContentExport(
                str(plan["element_name"]), str(plan["registry"]), "tab", field_name, _workspace_entry_status(existing_tab),
                ["Existing MCreator creative tab found; source conversion intentionally left it untouched."],
            ))
            continue
        icon_export = exports_by_compact_registry.get(_compact_registry(plan.get("icon_registry", "")))
        if icon_export is None:
            member_name = members[0]
            icon_export = next((item for item in result.exports if item.element_name == member_name), None)
        if icon_export is None:
            continue
        icon = f"CUSTOM:{icon_export.element_name}"
        if icon_export.element_type == "armor":
            icon += ".helmet"
        tab_element = {
            "_fv": 89,
            "_type": "tab",
            "definition": {
                "name": str(plan.get("display_name") or _display_name(plan["registry"])),
                "icon": icon,
                "showSearch": len(members) >= 24,
            },
        }
        tab_path = output_elements / f"{plan['element_name']}.mod.json"
        tab_path.write_text(json.dumps(tab_element, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        _upsert_workspace_element(workspace, None, str(plan["element_name"]), "tab", str(plan["registry"]), import_modid, str(plan["registry"]))
        result.exports.append(MCreatorContentExport(
            str(plan["element_name"]), str(plan["registry"]), "tab", field_name, "created", [],
        ))
        _log(log, f"MCreator content: created tab {plan['element_name']} <- {field_name}")
    # Resolve item-registry collisions caused by automatic BlockItems.  A custom
    # item and block may share the same path because Minecraft's block and item
    # registries are separate; the explicit item must own the item-registry path.
    explicit_item_paths = {
        export.registry_name.lower()
        for export in result.exports
        if export.status == "created" and export.element_type in {"item", "tool"}
    }
    for export in result.exports:
        if export.status != "created" or export.element_type not in {"block", "plant"}:
            continue
        if export.registry_name.lower() not in explicit_item_paths:
            continue
        element_path = output_elements / f"{export.element_name}.mod.json"
        data = _read_json(element_path)
        if not data or not isinstance(data.get("definition"), dict):
            continue
        data["definition"]["hasBlockItem"] = False
        element_path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        warning = (
            f"Automatic BlockItem disabled because explicit item '{export.registry_name}' "
            "owns the same item-registry path."
        )
        export.warnings.append(warning)
        result.warnings.append(f"{export.element_name}: {warning}")
    # Bind default projectile rendering/pickup items using source references.
    # This does not claim to reproduce custom hit logic, but it makes the
    # generated projectile usable by MCreator launch procedures and renderers.
    projectile_bindings = _infer_projectile_item_bindings(graph, result.exports)
    for projectile in [item for item in result.exports if item.status == "created" and item.element_type == "projectile"]:
        bound_item = projectile_bindings.get(projectile.element_name)
        if not bound_item:
            warning = "No source item/tool binding could be recovered for this projectile."
            projectile.warnings.append(warning)
            result.warnings.append(f"{projectile.element_name}: {warning}")
            continue
        element_path = output_elements / f"{projectile.element_name}.mod.json"
        data = _read_json(element_path)
        if not data or not isinstance(data.get("definition"), dict):
            continue
        data["definition"]["projectileItem"] = f"CUSTOM:{bound_item.element_name}"
        data["definition"]["customModelTexture"] = ""
        element_path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        projectile.warnings = [warning for warning in projectile.warnings if "item binding" not in warning.lower()]
        projectile.warnings.append(
            f"Projectile item bound from source references to '{bound_item.element_name}'. "
            "Verify launch trigger and custom impact behavior in MCreator."
        )
    # Recipes must exist as actual MCreator elements as well as raw data resources.
    # Build a source-registry -> imported-element map from the descriptors created above.
    registry_to_element: dict[str, str] = {}
    for descriptor in workspace.get("mod_elements", []):
        if str(descriptor.get("mojmodelbridge_source_modid", "")).lower() != str(import_modid).lower():
            continue
        source_registry = descriptor.get("mojmodelbridge_source_registry") or descriptor.get("registry_name")
        if source_registry:
            registry_to_element[_compact_registry(source_registry)] = str(descriptor.get("name", ""))
        registry_to_element.setdefault(_compact_registry(descriptor.get("registry_name", "")), str(descriptor.get("name", "")))
    recipe_exports, recipe_warnings = _emit_mcreator_recipe_elements(
        source_roots, output_elements, workspace, import_modid, registry_to_element, imported_fv=89
    )
    result.exports.extend(recipe_exports)
    result.warnings.extend(recipe_warnings)
    copied_recipes = _copy_recipe_resources(source_roots, workspace_root, modid)
    if copied_recipes:
        result.warnings.append(f"Copied {len(copied_recipes)} recoverable recipe JSON resource(s) into the workspace data tree.")
    else:
        result.warnings.append("No recoverable recipe JSON resources were found in the input roots.")
    try:
        _finalize_ore_generation_elements(source_roots, output_elements, source_worldgen_map, result.exports)
        _finalize_exact_block_resources(source_roots, output_elements, workspace_root, str(import_modid), str(modid), result.exports)
        _finalize_ranged_projectiles(source_roots, output_elements, result.exports)
    except Exception as exc:
        result.warnings.append(f"Final source-fidelity pass encountered an error: {exc}")
    result.warnings.extend([
        "Recipe JSON patterns unsupported by native MCreator elements remain in the data pack and are reported.",
        "Biome/dimension/village worldgen is intentionally disabled; ore/plant generation uses native MCreator fields where recoverable.",
    ])
    return result