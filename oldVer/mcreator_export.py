from __future__ import annotations

import copy
import json
import math
import os
import re
import shutil
import struct
import zlib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Iterable, Optional

try:
    from PIL import Image
except Exception:  # optional; spawn-egg colors gracefully fall back when unavailable
    Image = None  # type: ignore[assignment]

from legacy_bridge import parse_legacy_class, _run_javap, prime_javap_cache
from modern_bytecode_bridge import parse_modern_bytecode_class
from legacy_mod_graph import EntityMetadata, LegacyModGraph, RendererRegistration
from mcreator_content_export import MCreatorContentExport, export_mcreator_content, _load_language_names, _translated_name, infer_sound_bindings
from mcreator_source_bridge import _copy_source_resources, generate_compile_bridge
from mod_metadata import infer_mod_metadata


_ALLOWED_ITEM_MODEL_ROTATIONS = (-45.0, -22.5, 0.0, 22.5, 45.0)


def _legal_model_angle(value: float) -> float:
    value = float(value)
    # Exact quarter-turns can be baked into the cube geometry and therefore become
    # an unrotated cuboid without changing the rendered shape. Keep this helper for
    # the residual arbitrary-angle case only.
    while value > 45.0: value -= 90.0
    while value < -45.0: value += 90.0
    return min(_ALLOWED_ITEM_MODEL_ROTATIONS, key=lambda candidate: abs(candidate - value))

def _bake_quarter_turn(element: dict[str, Any]) -> bool:
    rot=element.get("rotation")
    if not isinstance(rot,dict): return False
    axis=str(rot.get("axis", ""))
    try: angle=float(rot.get("angle",0.0))
    except Exception: return False
    if axis not in {"x","y","z"}: return False
    q=round(angle/90.0)
    if abs(angle-q*90.0) > 1e-4 or q == 0: return False
    origin=rot.get("origin")
    fr=element.get("from"); to=element.get("to")
    if not (isinstance(origin,list) and len(origin)==3 and isinstance(fr,list) and len(fr)==3 and isinstance(to,list) and len(to)==3): return False
    ox,oy,oz=[float(x) for x in origin]
    import itertools
    corners=list(itertools.product((float(fr[0]),float(to[0])),(float(fr[1]),float(to[1])),(float(fr[2]),float(to[2]))))
    def rp(pt):
        x,y,z=pt; x-=ox; y-=oy; z-=oz
        turns=q%4
        for _ in range(turns):
            if axis=="x": y,z=-z,y
            elif axis=="y": x,z=z,-x
            else: x,y=-y,x
        return (x+ox,y+oy,z+oz)
    pts=[rp(c) for c in corners]
    element["from"]=[round(min(p[i] for p in pts),6) for i in range(3)]
    element["to"]=[round(max(p[i] for p in pts),6) for i in range(3)]
    element.pop("rotation",None)
    return True

def _sanitize_generated_item_models(workspace_root: Path, modid: str) -> list[str]:
    """Make generated 1.21.1 item/custom model JSON acceptable to ModelBakery."""
    root = Path(workspace_root) / 'src' / 'main' / 'resources' / 'assets' / modid / 'models'
    if not root.is_dir():
        return []
    warnings: list[str] = []
    files = sorted(root.rglob('*.json'), key=lambda p: str(p).lower())
    known = {str(p.relative_to(root)).replace('\\','/')[:-5] for p in files}
    def parent_exists(parent: str) -> bool:
        if parent.startswith('minecraft:') or ':' in parent and not parent.startswith(modid + ':'):
            return True
        ref = parent.split(':',1)[1] if parent.startswith(modid + ':') else parent
        return ref in known
    for path in files:
        try:
            data=json.loads(path.read_text(encoding='utf-8-sig'))
        except Exception:
            continue
        changed=False
        elements = data.get('elements',[]) if isinstance(data,dict) else []
        # Normalize the complete model once, not each cube independently. This
        # preserves relative positions between repeated/shared renderer cubes.
        numeric_points=[]
        for element in elements:
            if not isinstance(element,dict):
                continue
            fr=element.get('from'); to=element.get('to')
            if isinstance(fr,list) and isinstance(to,list) and len(fr)==3 and len(to)==3:
                try:
                    numeric_points.extend([[float(fr[i]) for i in range(3)], [float(to[i]) for i in range(3)]])
                except (ValueError,TypeError):
                    pass
        if numeric_points:
            mins=[min(pt[i] for pt in numeric_points) for i in range(3)]
            maxs=[max(pt[i] for pt in numeric_points) for i in range(3)]
            if any(v < 0.0 or v > 16.0 for v in mins+maxs):
                span=max(maxs[i]-mins[i] for i in range(3))
                scale=min(1.0, 16.0/span) if span > 0 else 1.0
                center=[(mins[i]+maxs[i])/2.0 for i in range(3)]
                def map_point(values):
                    return [round(max(0.0,min(16.0,(float(values[i])-center[i])*scale+8.0)),6) for i in range(3)]
                for element in elements:
                    if not isinstance(element,dict):
                        continue
                    for key in ('from','to'):
                        value=element.get(key)
                        if isinstance(value,list) and len(value)==3:
                            try:
                                element[key]=map_point(value)
                            except (ValueError,TypeError):
                                pass
                    rot=element.get('rotation')
                    if isinstance(rot,dict) and isinstance(rot.get('origin'),list) and len(rot['origin'])==3:
                        try:
                            rot['origin']=map_point(rot['origin'])
                        except (ValueError,TypeError):
                            pass
                changed=True
                warnings.append(f'{path.name}: out-of-bounds model coordinates normalized into 0..16')
        for element in elements:
            if not isinstance(element,dict): continue
            rot=element.get('rotation')
            if isinstance(rot,dict) and 'angle' in rot:
                try:
                    old=float(rot['angle'])
                    if _bake_quarter_turn(element):
                        changed=True; warnings.append(f'{path.name}: baked exact quarter-turn rotation {old:g} into cube geometry')
                    else:
                        new=_legal_model_angle(old)
                        if abs(old-new)>1e-5:
                            rot=element.get('rotation') or {}; rot['angle']=new; changed=True
                            warnings.append(f'{path.name}: arbitrary rotation {old:g} quantized to legal {new:g}')
                except (ValueError,TypeError):
                    if isinstance(rot,dict): rot['angle']=0.0; changed=True
        if isinstance(data,dict) and isinstance(data.get('parent'),str) and not parent_exists(data['parent']):
            old=data['parent']; data['parent']='item/generated'; changed=True
            warnings.append(f'{path.name}: missing parent {old} replaced with item/generated')
        # NeoForge separate-transforms wrappers can reference an auxiliary cast/pose
        # item model. If that auxiliary model is absent, make the wrapper self-contained
        # rather than leaving a ModelBakery FileNotFoundException.
        if isinstance(data,dict) and isinstance(data.get('loader'),str) and data.get('loader')=='neoforge:separate_transforms':
            base=data.get('base')
            if isinstance(base,dict) and isinstance(base.get('parent'),str) and not parent_exists(base['parent']):
                old=base['parent']; base['parent']='item/generated'; changed=True
                warnings.append(f'{path.name}: missing separate-transforms base parent {old} replaced with item/generated')
        if changed:
            path.write_text(json.dumps(data,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    return warnings


def _finalize_worldgen_namespace(workspace_root: Path, source_modid: str, target_modid: str) -> list[str]:
    """Finalize portable worldgen: ore generation only.

    Biomes, dimensions, villages/structures, structure sets/template pools, and
    source biome modifiers are deliberately excluded. Exact ore configured/placed
    features remain and are bound with NeoForge add_features modifiers generated
    by the content exporter.
    """
    root = Path(workspace_root) / "src" / "main" / "resources"
    if not target_modid:
        return []
    warnings: list[str] = []
    target_data = root / "data" / target_modid
    source_data = root / "data" / source_modid if source_modid else None
    # Do not trust the inferred source_modid alone. Template workspaces and copied
    # resources can carry additional namespaces (for example the original
    # ``chaospersists`` namespace) that still contain placed_feature JSON. NeoForge
    # loads every namespace present in the datapack and will create an unbound
    # registry entry even when our target namespace is clean. Scrub all non-target
    # worldgen namespaces globally.
    data_root = root / "data"
    if data_root.is_dir():
        for namespace_dir in list(data_root.iterdir()):
            if not namespace_dir.is_dir() or namespace_dir.name.lower() == target_modid.lower():
                continue
            for rel in (
                Path("worldgen") / "configured_feature",
                Path("worldgen") / "placed_feature",
                Path("worldgen") / "biome",
                Path("worldgen") / "structure",
                Path("worldgen") / "structure_set",
                Path("worldgen") / "template_pool",
                Path("worldgen") / "processor_list",
                Path("worldgen") / "pos_rule",
                Path("worldgen") / "jigsaw",
                Path("biome"), Path("dimension"), Path("dimension_type"),
            ):
                stale = namespace_dir / rel
                if stale.is_dir():
                    shutil.rmtree(stale, ignore_errors=True)
                    warnings.append(f"Removed non-target worldgen namespace: {namespace_dir.name}:{rel.as_posix()}")
            # Any non-target NeoForge/Forge biome modifiers can also poison the
            # registry through references into the removed feature namespace.
            for modifier_root in (namespace_dir / "neoforge" / "biome_modifier", namespace_dir / "forge" / "biome_modifier"):
                if modifier_root.is_dir():
                    shutil.rmtree(modifier_root, ignore_errors=True)
                    warnings.append(f"Removed non-target biome modifiers: {namespace_dir.name}")

    roots = [p for p in (target_data, source_data) if p and p.is_dir()]

    blocked_rel = [
        Path("biome"), Path("dimension"), Path("dimension_type"),
        Path("worldgen") / "biome", Path("worldgen") / "structure",
        Path("worldgen") / "structure_set", Path("worldgen") / "template_pool",
        Path("worldgen") / "processor_list", Path("worldgen") / "pos_rule",
        Path("worldgen") / "jigsaw",
    ]
    for data_root in roots:
        for rel in blocked_rel:
            path=data_root/rel
            if path.is_dir():
                shutil.rmtree(path, ignore_errors=True)
                warnings.append(f"Disabled non-ore worldgen: {path.relative_to(root)}")

        # Delete copied biome modifiers, including Forge-era ones. The exact ore
        # modifiers emitted by _emit_exact_source_worldgen_modifier are retained.
        for bm in (data_root/"neoforge"/"biome_modifier", data_root/"forge"/"biome_modifier"):
            if not bm.is_dir():
                continue
            for f in list(bm.rglob("*.json")):
                if not (f.name.endswith("_source_worldgen.json") and bm.parts[-2] == "neoforge"):
                    f.unlink(missing_ok=True)
                    warnings.append(f"Disabled non-ore biome modifier: {f.name}")
            for d in sorted((p for p in bm.rglob("*") if p.is_dir()), key=lambda p: len(p.parts), reverse=True):
                try: d.rmdir()
                except OSError: pass

    # Never leave source-namespace ore registries behind; those become unbound
    # registry entries in NeoForge when the target mod owns the generated modifier.
    if source_data and source_data.is_dir():
        for rel in (Path("worldgen")/"configured_feature", Path("worldgen")/"placed_feature"):
            p=source_data/rel
            if p.is_dir():
                shutil.rmtree(p, ignore_errors=True)
                warnings.append(f"Removed stale source registry tree: {source_modid}:{rel.as_posix()}")

    # Target namespace: retain only configured/placed ore features. This deliberately
    # strips plant/structure/custom feature registries that would otherwise introduce
    # unsupported biome dependencies.
    cf_dir=target_data/"worldgen"/"configured_feature"
    pf_dir=target_data/"worldgen"/"placed_feature"
    if cf_dir.is_dir():
        for f in list(cf_dir.glob("*.json")):
            try: payload=json.loads(f.read_text(encoding="utf-8-sig"))
            except Exception:
                f.unlink(missing_ok=True); continue
            if payload.get("type") != "minecraft:ore":
                f.unlink(missing_ok=True)
    if pf_dir.is_dir():
        for f in list(pf_dir.glob("*.json")):
            try: payload=json.loads(f.read_text(encoding="utf-8-sig"))
            except Exception:
                f.unlink(missing_ok=True); continue
            feature_name=str(payload.get("feature","")).rsplit(":",1)[-1]
            cf=cf_dir/f"{feature_name}.json"
            try: cfg=json.loads(cf.read_text(encoding="utf-8-sig")) if cf.exists() else {}
            except Exception: cfg={}
            if cfg.get("type") != "minecraft:ore":
                f.unlink(missing_ok=True)
    return warnings

def _relocate_stale_source_resources(workspace_root: Path, source_modid: str, target_modid: str) -> list[str]:
    """Ensure imported worldgen/sound resources cannot remain under the source namespace."""
    if not source_modid or not target_modid or source_modid.lower()==target_modid.lower():
        return []
    root=Path(workspace_root)/'src'/'main'/'resources'
    warnings=[]
    # Worldgen data is authoritative and must be under the mod's registered namespace.
    for top in ('worldgen','tags'):
        src=root/'data'/source_modid/top
        if not src.is_dir(): continue
        dst=root/'data'/target_modid/top
        for f in sorted(src.rglob('*'), key=lambda p:str(p).lower()):
            if not f.is_file(): continue
            rel=f.relative_to(src); out=dst/rel; out.parent.mkdir(parents=True,exist_ok=True)
            try:
                if f.suffix.lower()=='.json':
                    data=json.loads(f.read_text(encoding='utf-8-sig'))
                    def rw(v):
                        if isinstance(v,str):
                            return re.sub(rf'(?i)^{re.escape(source_modid)}:',target_modid+':',v)
                        if isinstance(v,list): return [rw(x) for x in v]
                        if isinstance(v,dict): return {k:rw(x) for k,x in v.items()}
                        return v
                    out.write_text(json.dumps(rw(data),indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
                else:
                    shutil.copy2(f,out)
            except Exception:
                continue
        shutil.rmtree(src,ignore_errors=True)
        warnings.append(f'Relocated stale {source_modid}:data/{top} resources into {target_modid}:data/{top}')
    # Sounds live under assets, and sample filenames must be lowercase ResourceLocation paths.
    src=root/'assets'/source_modid
    if src.is_dir():
        sounds=src/'sounds'
        target=root/'assets'/target_modid
        if sounds.exists():
            out_sounds=target/'sounds'; out_sounds.mkdir(parents=True,exist_ok=True)
            for f in sounds.rglob('*'):
                if f.is_file():
                    rel=f.relative_to(sounds); out=out_sounds/rel.name.lower(); out.parent.mkdir(parents=True,exist_ok=True)
                    try: shutil.copy2(f,out)
                    except OSError: pass
            shutil.rmtree(sounds,ignore_errors=True)
            warnings.append(f'Relocated/canonicalized {source_modid}:sounds into {target_modid}:sounds')
        sounds_json=src/'sounds.json'
        if sounds_json.exists():
            try:
                data=json.loads(sounds_json.read_text(encoding='utf-8-sig'))
                def rs(v):
                    if isinstance(v,str):
                        if ':' in v:
                            ns,path=v.split(':',1)
                            if ns.lower()==source_modid.lower(): return target_modid+':'+path.lower()
                        return v.lower() if v.endswith('.ogg') else v
                    if isinstance(v,list): return [rs(x) for x in v]
                    if isinstance(v,dict): return {k:rs(x) for k,x in v.items()}
                    return v
                target.mkdir(parents=True,exist_ok=True)
                (target/'sounds.json').write_text(json.dumps(rs(data),indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
                sounds_json.unlink()
            except Exception: pass
        if not any(src.iterdir()):
            src.rmdir()
        warnings.append(f'Remapped source sounds.json from {source_modid} to {target_modid}')
    return warnings


@dataclass
class MCreatorEntityExport:
    element_name: str
    registry_name: str
    entity_class: str
    renderer_class: str
    source_model: str
    generated_model: str
    model_args: list[Any] = field(default_factory=list)
    texture_ref: str = ""
    texture_output: str = ""
    texture_status: str = "missing"
    animations: list[str] = field(default_factory=list)
    sound_bindings: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    status: str = "created"


@dataclass
class MCreatorExportResult:
    workspace_dir: Path
    workspace_file: Path
    entities: list[MCreatorEntityExport]
    warnings: list[str]
    report_json: Path
    report_html: Path
    content: list[MCreatorContentExport] = field(default_factory=list)


def _log(log: Optional[Callable[[str], None]], message: str) -> None:
    if log:
        log(message)


def _simple(name: str) -> str:
    return name.replace("/", ".").split(".")[-1].split("$")[0]


def _compact_registry(value: str) -> str:
    return re.sub(r"[^a-z0-9]", "", str(value).lower())


def _pascal(value: str) -> str:
    words = re.findall(r"[A-Za-z0-9]+", re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", value))
    result = "".join(word[:1].upper() + word[1:] for word in words)
    if not result:
        result = "ConvertedEntity"
    if result[0].isdigit():
        result = "Entity" + result
    return result


def _snake(value: str) -> str:
    value = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", value)
    value = re.sub(r"[^A-Za-z0-9]+", "_", value).strip("_").lower()
    return value or "converted_entity"


def _java_identifier(value: str, fallback: str = "part") -> str:
    value = re.sub(r"[^A-Za-z0-9_$]", "_", value)
    if not value:
        value = fallback
    if value[0].isdigit():
        value = "_" + value
    java_keywords = {
        "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class",
        "const", "continue", "default", "do", "double", "else", "enum", "extends", "final",
        "finally", "float", "for", "goto", "if", "implements", "import", "instanceof", "int",
        "interface", "long", "native", "new", "package", "private", "protected", "public",
        "return", "short", "static", "strictfp", "super", "switch", "synchronized", "this",
        "throw", "throws", "transient", "try", "void", "volatile", "while", "record", "sealed",
    }
    return value + "_" if value in java_keywords else value


def _java_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def _float(value: float) -> str:
    value = float(value)
    if abs(value) < 5e-8:
        value = 0.0
    text = f"{value:.7f}".rstrip("0").rstrip(".")
    if "." not in text:
        text += ".0"
    return text + "F"


def _copy_template_workspace(source: Path, target: Path) -> None:
    source = source.resolve()
    if source.is_file():
        source = source.parent
    ignored_names = {
        "build", ".gradle", ".eclipse", "run", "bin", "out", ".git", ".idea",
    }

    def ignore(directory: str, names: list[str]) -> set[str]:
        result = {name for name in names if name in ignored_names}
        # MCreator histories are large and machine-local.
        if Path(directory).name == ".mcreator":
            result.update(name for name in names if name.lower() in {"localhistory", "backups"})
        return result

    shutil.copytree(source, target, dirs_exist_ok=True, ignore=ignore)


def _find_workspace_file(root: Path) -> Optional[Path]:
    """Choose the newest valid MCreator workspace definition, then the fullest."""
    candidates = []
    for path in root.glob("*.mcreator"):
        try:
            data = json.loads(path.read_text(encoding="utf-8-sig"))
        except Exception:
            continue
        if not isinstance(data, dict) or "workspaceSettings" not in data:
            continue
        try:
            version = int(data.get("mcreatorVersion", 0))
        except Exception:
            version = 0
        candidates.append((version, len(data.get("mod_elements", [])), path.name.lower(), path))
    if not candidates:
        return None
    candidates.sort(key=lambda item: (item[0], item[1], item[2]))
    return candidates[-1][3]


def _canonicalize_workspace_definition(root: Path, chosen: Path, target_modid: str) -> Path:
    """Keep exactly one canonical .mcreator workspace definition in the output."""
    chosen = chosen.resolve()
    for path in root.glob("*.mcreator"):
        if path.resolve() != chosen and path.is_file():
            path.unlink()
    target = root / f"{_snake(target_modid)}.mcreator"
    if chosen != target:
        if target.exists():
            target.unlink()
        chosen.rename(target)
    return target


def _sound_category(value: str) -> str:
    mapping = {
        "master": "neutral",
        "music": "music",
        "record": "records",
        "records": "records",
        "weather": "weather",
        "block": "blocks",
        "blocks": "blocks",
        "hostile": "hostile",
        "neutral": "neutral",
        "player": "players",
        "players": "players",
        "ambient": "ambient",
        "voice": "voice",
        "ui": "players",
    }
    return mapping.get(str(value or "neutral").strip().lower(), "neutral")


def _sound_file_stem(value: str) -> str:
    value = str(value or "").strip().replace("\\\\", "/")
    if ":" in value:
        value = value.split(":", 1)[1]
    value = value.lstrip("/")
    if value.startswith("sounds/"):
        value = value[7:]
    if value.lower().endswith(".ogg"):
        value = value[:-4]
    return value


def _humanize_sound_event(event: str) -> str:
    event = str(event or "").split(".")[-1]
    event = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", event)
    event = re.sub(r"[_\-]+", " ", event)
    return event.strip().capitalize() if event.strip() else "Converted Sound"


def _recover_sound_elements(
    source_roots: Iterable[Path],
    workspace: dict[str, Any],
    language_names: Optional[dict[str, str]] = None,
    log: Optional[Callable[[str], None]] = None,
) -> tuple[int, list[str]]:
    """Recover sounds.json event definitions into MCreator sound_elements."""
    existing = {
        str(item.get("name", "")).lower(): item
        for item in workspace.get("sound_elements", [])
        if isinstance(item, dict)
    }
    recovered: dict[str, dict[str, Any]] = {}
    warnings: list[str] = []
    for root in source_roots:
        for path in sorted(Path(root).rglob("sounds.json"), key=lambda p: str(p).lower()):
            try:
                payload = json.loads(path.read_text(encoding="utf-8-sig", errors="replace"))
            except Exception as exc:
                warnings.append(f"Could not parse sounds.json {path}: {exc}")
                continue
            if not isinstance(payload, dict):
                continue
            for event, definition in payload.items():
                if not isinstance(event, str) or not isinstance(definition, dict):
                    continue
                files: list[str] = []
                raw_sounds = definition.get("sounds", [])
                if isinstance(raw_sounds, (str, dict)):
                    raw_sounds = [raw_sounds]
                if isinstance(raw_sounds, list):
                    for entry in raw_sounds:
                        if isinstance(entry, str):
                            stem = _sound_file_stem(entry)
                        elif isinstance(entry, dict):
                            stem = _sound_file_stem(entry.get("name", ""))
                        else:
                            continue
                        if stem and stem not in files:
                            files.append(stem)
                subtitle = definition.get("subtitle", "")
                if not subtitle:
                    for key in (f"subtitle.{event}", f"subtitles.{event}", event):
                        translated = _translated_name(language_names or {}, key, "")
                        if translated and translated != key:
                            subtitle = translated
                            break
                subtitle = str(subtitle or _humanize_sound_event(event))
                if subtitle.startswith("subtitle.") or subtitle.startswith("subtitles."):
                    translated = _translated_name(language_names or {}, subtitle, "")
                    if translated and translated != subtitle:
                        subtitle = translated
                recovered[event.lower()] = {
                    "name": event,
                    "files": files,
                    "category": _sound_category(str(definition.get("category", "neutral"))),
                    "subtitle": subtitle,
                }

    count = 0
    for key, item in sorted(recovered.items()):
        if key in existing:
            continue
        workspace.setdefault("sound_elements", []).append(item)
        existing[key] = item
        count += 1
        if not item["files"]:
            warnings.append(f"Sound event '{item['name']}' has no referenced .ogg files in the supplied resources.")
    if log and (count or recovered):
        log(f"MCreator sound bridge: recovered {count} new sound events ({len(recovered)} source events found).")
    return count, warnings

def _source_text_for_class(graph: LegacyModGraph, class_name: str) -> str:
    path = graph.java_files.get(class_name) or graph.class_files.get(class_name)
    if path is None:
        for source_root in graph.roots:
            candidates = sorted(source_root.rglob(f"{class_name}.java"))
            if candidates:
                path = candidates[0]
                break
            candidates = sorted(source_root.rglob(f"{class_name}.class"))
            if candidates:
                path = candidates[0]
                break
    if path is None:
        return f"{class_name} {class_name.split('$')[-1]}"
    try:
        if path.suffix.lower() == '.java':
            return path.read_text(encoding='utf-8-sig', errors='replace')
        return _run_javap(path)
    except Exception:
        return f"{class_name} {path.stem}"


def _apply_sound_bindings(definition: dict[str, Any], sound_updates: dict[str, Any]) -> None:
    for key, value in sound_updates.items():
        if value is None:
            continue
        definition[key] = value


def _default_workspace(modid: str, mod_name: str, package: str, generator: str = "neoforge-1.21.1") -> dict[str, Any]:
    return {
        "mod_elements": [],
        "variable_elements": [],
        "sound_elements": [],
        "tag_elements": {},
        "tab_element_order": {},
        "language_map": {"en_us": {}},
        "metadata": {"files": []},
        "foldersRoot": {
            "name": "~",
            "children": [{"name": "Converted Entities", "children": []}],
        },
        "workspaceSettings": {
            "modid": modid,
            "modName": mod_name,
            "version": "1.0.0",
            "author": "MojModelBridge",
            "websiteURL": "",
            "license": "All Rights Reserved",
            "serverSideOnly": False,
            "modPicture": "",
            "requiredMods": [],
            "dependencies": [],
            "dependants": [],
            "mcreatorDependencies": [],
            "currentGenerator": generator,
            "credits": "Legacy entity data reconstructed by MojModelBridge",
            "modElementsPackage": package,
        },
        "mcreatorVersion": 202500228610,
    }


def _default_living_definition() -> dict[str, Any]:
    return {
        "mobName": "Converted Entity", "mobLabel": "", "mobModelName": "ModelConvertedEntity",
        "mobModelTexture": "converted_entity.png", "transparentModelCondition": {"fixedValue": False},
        "isShakingCondition": {"fixedValue": False}, "solidBoundingBox": {"fixedValue": False},
        "visualScale": {"fixedValue": 1.0}, "boundingBoxScale": {"fixedValue": 1.0},
        "modelLayers": [], "animations": [], "modelWidth": 0.6, "modelHeight": 1.8,
        "modelShadowSize": 0.5, "mountedYOffset": 0.0, "hasSpawnEgg": True,
        "spawnEggBaseColor": {"value": -8355712, "falpha": 0.0},
        "spawnEggDotColor": {"value": -16777216, "falpha": 0.0}, "creativeTabs": ["MISC"],
        "isBoss": False, "bossBarColor": "PURPLE", "bossBarType": "PROGRESS",
        "equipmentMainHand": "", "equipmentOffHand": "", "equipmentHelmet": "",
        "equipmentBody": "", "equipmentLeggings": "", "equipmentBoots": "",
        "mobBehaviourType": "Mob", "mobCreatureType": "UNDEFINED", "attackStrength": 3.0,
        "attackKnockback": 0.0, "knockbackResistance": 0.0, "movementSpeed": 0.3,
        "stepHeight": 0.6, "armorBaseValue": 0.0, "trackingRange": 64,
        "followRange": 16.0, "health": 10.0, "xpAmount": 0, "waterMob": False,
        "breatheUnderwater": {"fixedValue": False}, "pushedByFluids": {"fixedValue": True},
        "flyingMob": False, "inventorySize": 0, "inventoryStackSize": 64,
        "disableCollisions": False, "ridable": False, "canControlForward": False,
        "canControlStrafe": False, "immuneToFire": False, "immuneToArrows": False,
        "immuneToFallDamage": False, "immuneToCactus": False, "immuneToDrowning": False,
        "immuneToLightning": False, "immuneToPotions": False, "immuneToPlayer": False,
        "immuneToExplosion": False, "immuneToTrident": False, "immuneToAnvil": False,
        "immuneToWither": False, "immuneToDragonBreath": False, "mobDrop": "",
        "livingSound": "", "hurtSound": "entity.generic.hurt", "deathSound": "entity.generic.death",
        "stepSound": "", "raidCelebrationSound": "", "entityDataEntries": [], "hasAI": True,
        "aiBase": "(none)", "aixml": "", "breedable": False, "tameable": False,
        "breedTriggerItems": [], "ranged": False, "rangedAttackItem": "",
        "rangedItemType": "Default item", "rangedAttackInterval": 20,
        "rangedAttackRadius": 10.0, "spawnThisMob": False, "doesDespawnWhenIdle": True,
        "spawningProbability": 10, "mobSpawningType": "creature",
        "minNumberOfMobsPerGroup": 1, "maxNumberOfMobsPerGroup": 1,
        "restrictionBiomes": [], "spawnInDungeons": False,
        "raidSpawnsCount": [0, 0, 0, 0, 0, 0, 0], "sensitiveToVibration": False,
        "vibrationalEvents": [], "vibrationSensitivityRadius": {"fixedValue": 7.0},
    }


def _find_living_template(workspace_root: Path) -> Optional[dict[str, Any]]:
    for path in sorted((workspace_root / "elements").glob("*.mod.json")):
        try:
            data = json.loads(path.read_text(encoding="utf-8-sig"))
        except Exception:
            continue
        if data.get("_type") == "livingentity" and isinstance(data.get("definition"), dict):
            return data
    return None


def _ai_xml(ai_classes: Iterable[str]) -> str:
    classes = {name.lower() for name in ai_classes}
    blocks: list[tuple[str, list[tuple[str, str]]]] = []
    if any("swimming" in name for name in classes):
        blocks.append(("swim_in_water", [("condition", "null,null")]))
    if any(token in name for name in classes for token in ("attackoncollide", "meleeattack")):
        blocks.append(("attack_on_collide", [("speed", "1.2"), ("longmemory", "FALSE"), ("condition", "null,null")]))
    if any("wander" in name for name in classes):
        blocks.append(("wander", [("speed", "1"), ("condition", "null,null")]))
    if any(token in name for name in classes for token in ("hurtbytarget", "nearestattackabletarget")):
        blocks.append(("attack_action", [("callhelp", "TRUE"), ("condition", "null,null")]))
    if any(token in name for name in classes for token in ("lookidle", "watchclosest")):
        blocks.append(("look_around", [("condition", "null,null")]))
    if not blocks:
        blocks = [("wander", [("speed", "1"), ("condition", "null,null")]), ("look_around", [("condition", "null,null")])]
    nested = ""
    for block_type, fields in reversed(blocks):
        field_xml = "".join(f'<field name="{name}">{value}</field>' for name, value in fields)
        next_xml = f"<next>{nested}</next>" if nested else ""
        nested = f'<block type="{block_type}">{field_xml}{next_xml}</block>'
    return f'<xml xmlns="https://developers.google.com/blockly/xml"><block type="aitasks_container" deletable="false" x="40" y="40"><next>{nested}</next></block></xml>'


def _part_identifiers(parts: list[Any]) -> dict[int, str]:
    used: set[str] = set()
    result: dict[int, str] = {}
    for index, part in enumerate(parts):
        base = _java_identifier(str(part.variable or part.name), f"part{index}")
        candidate = base
        n = 2
        while candidate.lower() in used or candidate == "root":
            candidate = f"{base}_{n}"
            n += 1
        used.add(candidate.lower())
        result[id(part)] = candidate
    return result


def _topological_parts(parts: list[Any]) -> list[Any]:
    ordered: list[Any] = []
    seen: set[int] = set()

    def visit(part: Any) -> None:
        if id(part) in seen:
            return
        if part.parent is not None:
            visit(part.parent)
        seen.add(id(part))
        ordered.append(part)

    for part in sorted(parts, key=lambda p: p.source_index):
        visit(part)
    return ordered


def model_to_mojmap_java(model: Any, class_name: str, modid: str) -> str:
    from model_bridge import _expand_render_instance_parts
    _original_parts = model.parts
    default_parts = set(getattr(model, "default_render_parts", set()) or set())
    if default_parts:
        model.parts = [part for part in model.parts if str(part.name) in default_parts]
    model.parts = _expand_render_instance_parts(model)
    try:
        identifiers = _part_identifiers(model.parts)
        ordered = _topological_parts(model.parts)
        lines: list[str] = [
            "// Generated by MojModelBridge 3.1 from a legacy ModelBase class",
            "// MCreator import format: Mojmap 1.20.x / Modded Entity",
            "",
            f"public class {class_name}<T extends Entity> extends EntityModel<T> {{",
            "\tprivate float bridgeAgeInTicks;",
            "\tpublic static final ModelLayerLocation LAYER_LOCATION = new ModelLayerLocation(",
            f"\t\t\tnew ResourceLocation({_java_string(modid)}, {_java_string(_snake(class_name))}), \"main\");",
        ]
        for part in ordered:
            lines.append(f"\tprivate final ModelPart {identifiers[id(part)]};")
        lines.extend(["", f"\tpublic {class_name}(ModelPart root) {{"])
        for part in ordered:
            ident = identifiers[id(part)]
            parent_expr = "root" if part.parent is None else f"this.{identifiers[id(part.parent)]}"
            lines.append(f"\t\tthis.{ident} = {parent_expr}.getChild({_java_string(str(part.name))});")
        lines.extend(["\t}", "", "\tpublic static LayerDefinition createBodyLayer() {", "\t\tMeshDefinition meshdefinition = new MeshDefinition();", "\t\tPartDefinition partdefinition = meshdefinition.getRoot();", ""])

        for part in ordered:
            ident = identifiers[id(part)]
            parent_ident = "partdefinition" if part.parent is None else identifiers[id(part.parent)]
            builder = "CubeListBuilder.create()"
            for cube in part.cubes:
                builder += f".texOffs({int(cube.uv_x)}, {int(cube.uv_y)})"
                if cube.mirror:
                    builder += ".mirror()"
                builder += (
                    f".addBox({_float(cube.x)}, {_float(cube.y)}, {_float(cube.z)}, "
                    f"{_float(cube.dx)}, {_float(cube.dy)}, {_float(cube.dz)}, "
                    f"new CubeDeformation({_float(cube.inflate)}))"
                )
                if cube.mirror:
                    builder += ".mirror(false)"
            ox, oy, oz = part.offset
            rx, ry, rz = part.rotation
            if all(abs(v) < 1e-8 for v in (rx, ry, rz)):
                pose = f"PartPose.offset({_float(ox)}, {_float(oy)}, {_float(oz)})"
            else:
                pose = (
                    f"PartPose.offsetAndRotation({_float(ox)}, {_float(oy)}, {_float(oz)}, "
                    f"{_float(rx)}, {_float(ry)}, {_float(rz)})"
                )
            lines.append(
                f"\t\tPartDefinition {ident} = {parent_ident}.addOrReplaceChild({_java_string(str(part.name))},\n"
                f"\t\t\t\t{builder}, {pose});\n"
            )

        lines.extend([
            f"\t\treturn LayerDefinition.create(meshdefinition, {int(model.texture_width)}, {int(model.texture_height)});",
            "\t}", "", "\t@Override",
            "\tpublic void setupAnim(T entity, float limbSwing, float limbSwingAmount, float ageInTicks, float netHeadYaw, float headPitch) {",
            "\t\tthis.bridgeAgeInTicks = ageInTicks;",
            "\t\t// AnimationDefinition clips are linked by the generated MCreator living-entity element.",
            "\t}", "", "\t@Override",
            "\tpublic void renderToBuffer(PoseStack poseStack, VertexConsumer vertexConsumer, int packedLight, int packedOverlay,",
            "\t\t\tfloat red, float green, float blue, float alpha) {",
        ])
        roots = [part for part in ordered if part.parent is None]
        runtime_specs = list(getattr(model, "render_runtime_rotations", []) or [])
        handled: set[int] = set()
        axis_enum = {"X": "XP", "Y": "YP", "Z": "ZP"}
        for runtime in runtime_specs:
            sources = set(runtime.get("parts", []))
            group = [part for part in ordered if (str(getattr(part, "source_name", "")) or str(part.name)) in sources and part.parent is None]
            if not group:
                continue
            axis = str(runtime.get("axis", "Z"))
            speed = float(runtime.get("speed", 0.0))
            angle_expr = f"this.bridgeAgeInTicks * {_float(speed if bool(runtime.get('degrees', False)) else math.degrees(speed))}"
            lines.append("\t\tposeStack.pushPose();")
            lines.append(f"\t\tposeStack.mulPose(Axis.{axis_enum.get(axis, 'ZP')}.rotationDegrees({angle_expr}));")
            for part in group:
                lines.append(f"\t\t{identifiers[id(part)]}.render(poseStack, vertexConsumer, packedLight, packedOverlay, red, green, blue, alpha);")
                handled.add(id(part))
            lines.append("\t\tposeStack.popPose();")
        for part in roots:
            if id(part) in handled:
                continue
            lines.append(f"\t\t{identifiers[id(part)]}.render(poseStack, vertexConsumer, packedLight, packedOverlay, red, green, blue, alpha);")
        lines.extend(["\t}", "}", ""])
        return "\n".join(lines)
    finally:
        model.parts = _original_parts

def _animation_field_name(model_class: str, animation_name: str, used: set[str]) -> str:
    suffix = animation_name.split(".")[-1]
    base = _java_identifier(_pascal(model_class + " " + suffix), "ConvertedAnimation")
    value = base
    n = 2
    while value.lower() in used:
        value = f"{base}{n}"
        n += 1
    used.add(value.lower())
    return value


def _blockbench_delta_to_java(vector: Any) -> tuple[float, float, float]:
    """Return baked additive deltas unchanged for generated ModelPart animations.

    The generated Mojmap model uses the same axis convention as the baked Blockbench
    delta. Applying a second sign transform here was the source of animation-only
    orientation/offset errors in large legacy models such as The King and The Queen.
    """
    return (float(vector[0]), float(vector[1]), float(vector[2]))


def animation_definition_java(model: Any, class_name: str) -> tuple[str, list[dict[str, Any]]]:
    from model_bridge import _expand_render_instance_parts
    animations = (model.baked_animation_json or {}).get("animations", {})
    if not isinstance(animations, dict) or not animations:
        return "", []
    animation_class = class_name + "Animation"
    used_fields: set[str] = set()
    exported: list[dict[str, Any]] = []
    declarations: list[str] = []
    methods: list[str] = []

    # A single gigantic AnimationDefinition builder expression can make javac
    # emit a >64 KiB method for bosses with thousands of sampled keyframes.
    # Keep every channel intact, but add them through small helper methods.
    channels_per_helper = 6

    for animation_name, animation in animations.items():
        if not isinstance(animation, dict):
            continue
        bones = animation.get("bones", {})
        if not isinstance(bones, dict) or not bones:
            continue
        field_name = _animation_field_name(class_name, str(animation_name), used_fields)
        max_time = 0.0
        additions: list[str] = []
        expanded_for_aliases = _expand_render_instance_parts(model)
        default_part_names = set(getattr(model, "default_render_parts", set()) or set())
        alias_targets: dict[str, list[str]] = {}
        for expanded_part in expanded_for_aliases:
            canonical = str(getattr(expanded_part, "source_name", "") or expanded_part.name)
            if default_part_names and canonical not in default_part_names:
                continue
            alias_targets.setdefault(canonical, []).append(str(expanded_part.name))
        for bone_name, channels in bones.items():
            if not isinstance(channels, dict):
                continue
            target_bones = alias_targets.get(str(bone_name), [str(bone_name)])
            for channel_name, target_name, vector_method in (
                ("rotation", "ROTATION", "degreeVec"),
                ("position", "POSITION", "posVec"),
            ):
                keyframe_map = channels.get(channel_name, {})
                if not isinstance(keyframe_map, dict) or not keyframe_map:
                    continue
                frames: list[str] = []
                for time_text, vector in sorted(keyframe_map.items(), key=lambda pair: float(pair[0])):
                    if not isinstance(vector, (list, tuple)) or len(vector) < 3:
                        continue
                    time = float(time_text)
                    max_time = max(max_time, time)
                    java_vector = _blockbench_delta_to_java(vector)
                    frames.append(
                        f"new Keyframe({_float(time)}, KeyframeAnimations.{vector_method}({_float(java_vector[0])}, {_float(java_vector[1])}, {_float(java_vector[2])}), "
                        "AnimationChannel.Interpolations.LINEAR)"
                    )
                if frames:
                    for target_bone in target_bones:
                        additions.append(
                            f"\t\t\t.addAnimation({_java_string(str(target_bone))},\n"
                            f"\t\t\t\t\tnew AnimationChannel(AnimationChannel.Targets.{target_name},\n"
                            f"\t\t\t\t\t\t\t{',\n\t\t\t\t\t\t\t'.join(frames)}))"
                        )
        if not additions:
            continue

        length = max(max_time, 0.05)
        loop_call = ".looping()" if animation.get("loop", True) else ""
        declarations.append(f"\tpublic static final AnimationDefinition {field_name} = build{field_name}();")

        chunks = [additions[i:i + channels_per_helper] for i in range(0, len(additions), channels_per_helper)]
        build_lines = [
            f"\tprivate static AnimationDefinition build{field_name}() {{",
            f"\t\tAnimationDefinition.Builder builder = AnimationDefinition.Builder.withLength({_float(length)}){loop_call};",
        ]
        for chunk_index in range(len(chunks)):
            build_lines.append(f"\t\tbuilder = add{field_name}Part{chunk_index + 1}(builder);")
        build_lines.extend(["\t\treturn builder.build();", "\t}"])
        methods.append("\n".join(build_lines))

        for chunk_index, chunk in enumerate(chunks, 1):
            methods.append(
                f"\tprivate static AnimationDefinition.Builder add{field_name}Part{chunk_index}(AnimationDefinition.Builder builder) {{\n"
                "\t\treturn builder\n"
                + "\n".join(chunk)
                + ";\n\t}"
            )

        lower = str(animation_name).lower()
        exported.append({
            "field": field_name,
            "walking": "walk" in lower or "run" in lower,
            "source_name": str(animation_name),
            "length": length,
            "channel_count": len(additions),
            "helper_count": len(chunks),
        })

    if not declarations:
        return "", []
    text = "\n".join([
        "// Generated by MojModelBridge 3.1 from sampled legacy procedural animation",
        "// Save in the MCreator models/animations directory and generate imports.",
        "// Large clips are split into helper methods to avoid the JVM method-size limit.",
        "", f"public class {animation_class} {{", "\n".join(declarations), "",
        "\n\n".join(methods), "}", "",
    ])
    return text, exported


def _build_texture_file_index(roots: Iterable[Path]) -> list[tuple[int, str, Path]]:
    """Index PNG resources once for large entity exports.

    The old resolver recursively walked every source/template tree for every
    entity and for every fallback stem.  Mods with hundreds of renderers and
    thousands of assets therefore spent minutes per hundred entities doing the
    same filesystem traversal.  The scoring rules remain unchanged; only the
    candidate enumeration is shared.
    """
    indexed: list[tuple[int, str, Path]] = []
    seen: set[str] = set()
    for priority, root in enumerate(roots):
        root = Path(root)
        if not root.exists():
            continue
        try:
            candidates = root.rglob("*.png")
        except OSError:
            continue
        for path in candidates:
            if not path.is_file():
                continue
            try:
                key = str(path.resolve()).lower()
            except OSError:
                key = str(path).lower()
            if key in seen:
                continue
            seen.add(key)
            indexed.append((priority, path.as_posix().lower(), path))
    return indexed


def _texture_candidates(
    roots: Iterable[Path],
    ref: str,
    indexed_files: Optional[list[tuple[int, str, Path]]] = None,
) -> list[Path]:
    if not ref:
        return []
    normalized = ref.replace("\\", "/").lstrip("/")
    basename = Path(normalized).name.lower()
    tail = normalized.lower()
    scored: list[tuple[int, int, str, Path]] = []
    candidates = indexed_files if indexed_files is not None else _build_texture_file_index(roots)
    for priority, lower_path, path in candidates:
        score = 0
        if lower_path.endswith(tail):
            score += 3000
        if path.name.lower() == basename:
            score += 1000
        # Prefer entity folders for entity renderers, but never over an exact reference.
        if any(token in lower_path for token in ("/textures/entities/", "/textures/entity/")):
            score += 80
        if any(token in lower_path for token in ("/textures/item/", "/textures/items/")):
            score += 20
        scored.append((score, -priority, lower_path, path))
    scored.sort(key=lambda item: (-item[0], -item[1], item[2]))
    return [item[3] for item in scored if item[0] > 0]


def _dynamic_entity_texture(roots: list[Path], reg: RendererRegistration) -> tuple[str, Optional[Path]]:
    """Resolve inherited/dynamic getTexture() dispatch from the clean source.

    This follows concrete ``instanceof`` branches first, then fixed selector fields,
    and finally a stable default return. It intentionally does not collapse all
    subclasses of a shared butterfly/moth renderer onto the first texture.
    """
    entity_name = str(getattr(reg, "entity_class", "") or "").split(".")[-1]
    candidates=[]
    for root in roots:
        candidates.extend(root.rglob(f"{entity_name}.java"))
    if not candidates:
        return "", None
    entity_path=sorted(candidates,key=lambda x:str(x).lower())[0]
    try: entity_text=entity_path.read_text(encoding="utf-8-sig",errors="replace")
    except OSError: return "",None
    pieces=[entity_text]
    superclass=""
    mh=re.search(r"\bclass\s+"+re.escape(entity_name)+r"\s+extends\s+([A-Za-z0-9_$.]+)",entity_text)
    if mh: superclass=mh.group(1).split(".")[-1]
    if superclass and superclass != entity_name:
        for root in roots:
            sf=sorted(root.rglob(f"{superclass}.java"),key=lambda x:str(x).lower())
            if sf:
                try: pieces.append(sf[0].read_text(encoding="utf-8-sig",errors="replace"))
                except OSError: pass
                break
    combined="\n".join(pieces)
    # Named ResourceLocation constants preserve the source's semantic texture identity.
    constants={}
    for m in re.finditer(r'''(?:static\s+final\s+)?ResourceLocation\s+([A-Za-z_$][\w$]*)\s*=\s*ResourceLocation(?:\.fromNamespaceAndPath)?\s*\(\s*['\"]([^'\"]+)['\"]\s*,\s*['\"]([^'\"]+)['\"]''', combined):
        constants[m.group(1)]=(m.group(2),m.group(3))
    for m in re.finditer(r'''ResourceLocation\s+([A-Za-z_$][\w$]*)\s*=\s*new\s+ResourceLocation\s*\(\s*['\"]([^:'\"]+):([^'\"]+)['\"]''', combined):
        constants[m.group(1)]=(m.group(2),m.group(3))
    # Concrete instanceof branches have highest priority.
    for m in re.finditer(r"if\s*\(\s*[^)]*instanceof\s+([A-Za-z_$][\w$]*)[^)]*\)\s*\{?\s*return\s+([A-Za-z_$][\w$]*)",combined,re.S):
        branch_cls, ret=m.groups()
        if _simple(branch_cls).lower() == _simple(entity_name).lower() and ret in constants:
            ns,path=constants[ret]
            return _resolve_resource_texture(roots, f"{ns}:{path}")
    # Explicit fixed selector assignment in the concrete subclass.
    selector=None
    for pat in (r"\bthis\.(?:birdtype|birdType)\s*=\s*(\d+)",r"\b(?:setBirdType)\s*\(\s*(\d+)\s*\)"):
        mm=re.search(pat,entity_text)
        if mm: selector=int(mm.group(1)); break
    # Return sequence from getTexture().
    get_texture=combined
    ret_tokens=[]
    for m in re.finditer(r"\breturn\s+([A-Za-z_$][\w$]*)\s*;",get_texture):
        if m.group(1) in constants: ret_tokens.append(m.group(1))
    if selector is not None and 0 <= selector < len(ret_tokens):
        ns,path=constants[ret_tokens[selector]]
        return _resolve_resource_texture(roots,f"{ns}:{path}")
    # Luna moth has a runtime-random moth_type, so preserve its authored default texture.
    if _simple(entity_name).lower() == "entitylunamoth" and "TEXTURE_LUNAMOTH" in constants:
        ns,path=constants["TEXTURE_LUNAMOTH"]
        return _resolve_resource_texture(roots,f"{ns}:{path}")
    # If the concrete entity is not a dynamic subclass, prefer its explicit constant.
    if ret_tokens:
        ns,path=constants[ret_tokens[-1]]
        return _resolve_resource_texture(roots,f"{ns}:{path}")
    return "",None

def _resolve_resource_texture(roots: list[Path], ref: str) -> tuple[str, Optional[Path]]:
    namespace,path = ref.split(":",1) if ":" in ref else ("",ref)
    path=path.lstrip("/")
    expected=Path(path).name
    for root in roots:
        for candidate in root.rglob(expected):
            low=candidate.as_posix().lower()
            if f"/textures/entity/" in low or f"/textures/entities/" in low or candidate.name.lower()==expected.lower():
                return f"{namespace}:{path}",candidate
    return f"{namespace}:{path}",None

def _select_texture(
    roots: Iterable[Path],
    reg: RendererRegistration,
    indexed_files: Optional[list[tuple[int, str, Path]]] = None,
) -> tuple[str, Optional[Path], list[str]]:
    warnings: list[str] = []
    refs = [ref for ref in reg.texture_refs if ref.lower().endswith(".png")]
    roots = list(roots)
    # Shared renderers (notably butterflies/moths) can expose a generic renderer
    # texture while the clean entity source dispatches to a concrete subclass texture.
    # Resolve that source dispatch first so subclasses cannot all inherit the first
    # texture seen on the renderer.
    dynamic_ref, dynamic_source = _dynamic_entity_texture(roots, reg)
    if dynamic_ref:
        if dynamic_source:
            warnings.append(f"Resolved inherited/dynamic entity texture: {dynamic_ref}")
            return dynamic_ref, dynamic_source, warnings
        warnings.append(f"Recovered inherited/dynamic entity texture reference but source PNG was not found: {dynamic_ref}")
        return dynamic_ref, None, warnings

    for ref in refs:
        candidates = _texture_candidates(roots, ref, indexed_files)
        if candidates:
            return ref, candidates[0], warnings
    if refs:
        warnings.append(f"Renderer references texture(s) not present in searched source/template resources: {refs}")
        return refs[0], None, warnings

    # Fallback for legacy renderers whose texture is provided through a superclass, helper,
    # inherited ResourceLocation, or another class that javap did not expose directly. Search
    # exact class/registry stems but strongly prefer entity-oriented directories so an item icon
    # with a similar name does not become the mob skin.
    stems: list[str] = []
    for value in (getattr(reg, "registry_name", ""), getattr(reg, "entity_class", ""), getattr(reg, "renderer_class", ""), getattr(reg, "model_class", "")):
        if not value:
            continue
        stem = str(value).split(".")[-1]
        for prefix in ("Render", "Model"):
            if stem.lower().startswith(prefix.lower()):
                stem = stem[len(prefix):]
        if stem.endswith("Entity"):
            stem = stem[:-6]
        stems.extend([stem, stem.lower()])
    stems = list(dict.fromkeys(s for s in stems if s))
    if stems:
        fallback_candidates: list[Path] = []
        seen_paths: set[str] = set()
        for stem in stems:
            for candidate in _texture_candidates(roots, f"{stem}.png", indexed_files):
                key = str(candidate.resolve()).lower()
                if key in seen_paths:
                    continue
                seen_paths.add(key)
                fallback_candidates.append(candidate)
        if fallback_candidates:
            def entity_priority(path: Path) -> tuple[int, str]:
                lower = path.as_posix().lower()
                preferred = 0 if any(token in lower for token in ("/textures/entities/", "/textures/entity/", "/textures/mob/", "/textures/mobs/")) else 1
                return preferred, lower
            fallback_candidates.sort(key=entity_priority)
            chosen = fallback_candidates[0]
            warnings.append(f"Renderer texture string was not recovered; selected fallback texture by entity/renderer/model stem: {chosen.name}")
            return chosen.name, chosen, warnings

    warnings.append("No PNG ResourceLocation/string or reliable entity texture stem was recovered from the renderer.")
    return "", None, warnings


def _unique_element_name(base: str, used: set[str]) -> str:
    base = _pascal(base)
    result = base
    n = 2
    while result.lower() in used:
        result = f"{base}{n}"
        n += 1
    used.add(result.lower())
    return result


def _clone_definition(template: Optional[dict[str, Any]]) -> tuple[int, dict[str, Any]]:
    if template:
        return int(template.get("_fv", 89)), copy.deepcopy(template["definition"])
    return 89, _default_living_definition()


def _find_spawn_egg_texture(source_roots: Iterable[Path], registry: str, entity_class: str) -> tuple[str, Path | None]:
    """Find the source custom spawn-egg texture for the corresponding mob.

    MCreator living-entity elements expose spawnEggTexture; do not create a
    separate texture-only item just to represent an imported egg skin.
    """
    roots=[Path(r) for r in source_roots]
    targets=[]
    for base in (registry, _simple(entity_class), entity_class):
        compact=_compact_registry(base)
        if compact:
            targets.append(compact)
            targets.append("egg"+compact)
    for root in roots:
        if not root.exists():
            continue
        candidates=[]
        for p in root.rglob("*.png"):
            stem=_compact_registry(p.stem)
            if stem in set(targets) and stem.startswith("egg") and len(stem)>3:
                candidates.append(p)
        if candidates:
            chosen=sorted(candidates, key=lambda p:(len(str(p)),str(p).lower()))[0]
            return chosen.stem, chosen
    return "", None

def _extract_spawn_egg_colors(source_roots: Iterable[Path], registry: str, entity_class: str) -> tuple[int, int] | None:
    """Extract the two dominant visible colors from a legacy custom spawn-egg texture.

    MCreator's native living-entity schema exposes base/dot spawn-egg colors rather
    than a custom egg texture slot, so this preserves the appearance as closely as
    possible without inventing a custom renderer.
    """
    if Image is None:
        return None
    roots = [Path(root) for root in source_roots]
    bases = {_compact_registry(registry), _compact_registry(entity_class), _compact_registry(_simple(entity_class))}
    candidates: list[Path] = []
    wanted_tokens = ("spawnegg", "egg")
    for root in roots:
        if not root.exists():
            continue
        for path in root.rglob("*.png"):
            name = _compact_registry(path.stem)
            if not name:
                continue
            if not any(token in name for token in wanted_tokens):
                continue
            core = name.replace("spawnegg", "").replace("egg", "")
            if any(base and (base in core or core in base) for base in bases):
                candidates.append(path)
    if not candidates:
        return None
    source = sorted(candidates, key=lambda p: (len(str(p)), str(p).lower()))[0]
    try:
        image = Image.open(source).convert("RGBA")
        image.thumbnail((128, 128))
        counts: dict[tuple[int,int,int,int], int] = {}
        for rgba in image.getdata():
            if rgba[3] < 32:
                continue
            r, g, b, a = rgba
            # Small quantization suppresses antialiasing noise while retaining the
            # two dominant actual colors of legacy egg textures.
            q = (r // 8 * 8, g // 8 * 8, b // 8 * 8, a)
            counts[q] = counts.get(q, 0) + 1
        if not counts:
            return None
        ordered = sorted(counts.items(), key=lambda item: (-item[1], item[0]))
        colors = [ordered[0][0]]
        for color, _count in ordered[1:]:
            if sum((int(color[i]) - int(colors[0][i])) ** 2 for i in range(3)) >= 18 ** 2:
                colors.append(color)
                break
        while len(colors) < 2:
            colors.append(colors[0])
        def argb(color: tuple[int,int,int,int]) -> int:
            r,g,b,a = color
            value = (int(a) << 24) | (int(r) << 16) | (int(g) << 8) | int(b)
            return value - 0x100000000 if value >= 0x80000000 else value
        return argb(colors[0]), argb(colors[1])
    except Exception:
        return None


def _make_element(
    fv: int,
    definition: dict[str, Any],
    element_name: str,
    model_class: str,
    texture_name: str,
    meta: EntityMetadata,
    reg: RendererRegistration,
    animations: list[dict[str, Any]],
    spawn_egg_colors: tuple[int, int] | None = None,
) -> dict[str, Any]:
    definition["mobName"] = meta.display_name
    definition["mobLabel"] = ""
    definition["mobModelName"] = model_class
    definition["mobModelTexture"] = texture_name
    definition.setdefault("visualScale", {})["fixedValue"] = float(reg.visual_scale or 1.0)
    definition.setdefault("boundingBoxScale", {})["fixedValue"] = 1.0
    definition["modelWidth"] = max(0.01, float(meta.width))
    definition["modelHeight"] = max(0.01, float(meta.height))
    definition["modelShadowSize"] = max(0.0, float(reg.shadow_size))
    definition["hasSpawnEgg"] = True
    if spawn_egg_colors:
        base_color, dot_color = spawn_egg_colors
        definition["spawnEggBaseColor"] = {"fixedValue": int(base_color)}
        definition["spawnEggDotColor"] = {"fixedValue": int(dot_color)}
    # Spawn eggs should remain discoverable in creative mode.  Legacy mods
    # generally relied on the vanilla miscellaneous tab for entity eggs; an
    # empty list forces testers to use /give and was also flagged by the
    # workspace validator for every generated living entity.
    # Final workspace pass assigns every imported entity to the generated mod tab.
    # Keep MISC as a schema-safe fallback until that pass runs.
    definition["creativeTabs"] = ["MISC"]
    definition["isBoss"] = bool(meta.xp >= 500 or meta.health >= 200 or meta.width >= 4 or meta.height >= 4)
    definition["mobBehaviourType"] = "Mob" if "mob" in meta.superclass.lower() else "Creature"
    definition["mobCreatureType"] = "UNDEFINED"
    definition["attackStrength"] = max(0.0, float(meta.attack_damage))
    definition["movementSpeed"] = max(0.0, float(meta.movement_speed))
    definition["armorBaseValue"] = max(0.0, float(meta.armor))
    definition["followRange"] = max(1.0, float(meta.follow_range))
    definition["health"] = max(1.0, float(meta.health))
    definition["xpAmount"] = max(0, int(meta.xp))
    definition["waterMob"] = bool(meta.water_mob)
    definition.setdefault("breatheUnderwater", {})["fixedValue"] = bool(meta.water_mob)
    definition["flyingMob"] = bool(meta.flying_mob)
    definition["immuneToFire"] = bool(meta.fire_immune)
    definition["hasAI"] = True
    definition["aixml"] = _ai_xml(meta.ai_classes)
    definition["spawnThisMob"] = False
    definition["animations"] = [
        {
            "animation": f"CUSTOM:{model_class}Animation.{animation['field']}",
            "speed": 1.0,
            "walking": bool(animation["walking"]),
            "amplitude": 1.0,
        }
        for animation in animations
    ]
    return {"_fv": fv, "_type": "livingentity", "definition": definition}


def _clean_workspace_for_generated_entities(workspace: dict[str, Any], generated_names: set[str]) -> None:
    workspace["mod_elements"] = [
        item for item in workspace.get("mod_elements", [])
        if str(item.get("name", "")).lower() not in {name.lower() for name in generated_names}
    ]



def _normalise_template_resource_locations(output_dir: Path, target_modid: str, log: Optional[Callable[[str], None]] = None) -> int:
    """Repair stale imported MCreator resource locations (notably orespawn2:items/*) in a cloned template."""
    root = output_dir / "src" / "main" / "resources" / "assets" / target_modid
    if not root.exists():
        return 0
    changed = 0
    aliases = {f"{target_modid}2": target_modid, f"{target_modid}1": target_modid}

    def transform(value: Any) -> Any:
        if isinstance(value, str) and ":" in value:
            namespace, path = value.split(":", 1)
            namespace = aliases.get(namespace, namespace)
            path = path.replace("items/", "item/").replace("blocks/", "block/")
            return f"{namespace}:{path}"
        if isinstance(value, dict):
            return {k: transform(v) for k, v in value.items()}
        if isinstance(value, list):
            return [transform(v) for v in value]
        return value

    for path in root.rglob("*.json"):
        try:
            original = path.read_text(encoding="utf-8")
            data = json.loads(original)
        except Exception:
            continue
        transformed = transform(data)
        encoded = json.dumps(transformed, indent=2, ensure_ascii=False) + "\n"
        if encoded != original:
            path.write_text(encoded, encoding="utf-8")
            changed += 1
    if log and changed:
        log(f"MCreator template resource repair: normalized {changed} existing JSON resource files")
    return changed


def _write_placeholder_png(path: Path, size: int = 16) -> None:
    """Write a tiny checker texture using only the Python standard library."""
    path.parent.mkdir(parents=True, exist_ok=True)
    rows = bytearray()
    for y in range(size):
        rows.append(0)  # PNG filter: none
        for x in range(size):
            checker = ((x // 4) + (y // 4)) % 2
            rgba = (255, 0, 255, 255) if checker == 0 else (20, 20, 20, 255)
            rows.extend(rgba)

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    path.write_bytes(png)


def _write_standalone_scaffold(root: Path, modid: str, generator: str = "neoforge-1.21.1") -> None:
    """Add a standalone Gradle scaffold matching the selected MCreator target.

    MCreator 2026.2 natively supports NeoForge 1.21.1, using NeoForge 21.1.228
    and Java 21. The older Forge target remains available for backward compatibility.
    """
    if generator == "neoforge-1.21.1":
        build_gradle = f"""plugins {{
    id 'eclipse'
    id 'net.neoforged.moddev' version '2.0.107'
}}

version = '1.0.0'
base.archivesName = {json.dumps(modid)}

java.toolchain.languageVersion = JavaLanguageVersion.of(21)

neoForge {{
    version = '21.1.228'

    runs {{
        client {{ client() }}
        server {{ server() }}
        data {{ data() }}
    }}

    mods {{
        {modid} {{ sourceSet sourceSets.main }}
    }}
}}

tasks.withType(JavaCompile).configureEach {{
    options.encoding = 'UTF-8'
}}

apply from: 'mcreator.gradle'
"""
        settings_gradle = """pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
}

plugins {
    id 'org.gradle.toolchains.foojay-resolver-convention' version '1.0.0'
}
"""
        pack_format = 15
    else:
        build_gradle = f"""plugins {{
    id 'eclipse'
    id 'net.neoforged.moddev.legacyforge' version '2.0.142'
}}

version = '1.0.0'
base.archivesName = {json.dumps(modid)}

java.toolchain.languageVersion = JavaLanguageVersion.of(17)

legacyForge {{
    version = '1.20.1-47.4.22'

    runs {{
        client {{ client() }}
        server {{ server() }}
    }}

    mods {{
        mcreator {{ sourceSet sourceSets.main }}
    }}
}}

tasks.withType(JavaCompile).configureEach {{
    options.encoding = 'UTF-8'
}}

apply from: 'mcreator.gradle'
"""
        settings_gradle = """pluginManagement {
    repositories {
        gradlePluginPortal()
        maven { name = 'MinecraftForge'; url = 'https://maven.minecraftforge.net/' }
    }
}

plugins {
    id 'org.gradle.toolchains.foojay-resolver-convention' version '1.0.0'
}
"""
        pack_format = 15
    (root / 'build.gradle').write_text(build_gradle, encoding='utf-8')
    (root / 'settings.gradle').write_text(settings_gradle, encoding='utf-8')
    (root / 'gradle.properties').write_text("org.gradle.jvmargs=-Xmx2G\norg.gradle.daemon=true\norg.gradle.parallel=true\norg.gradle.caching=true\n", encoding='utf-8')
    (root / 'mcreator.gradle').write_text('', encoding='utf-8')
    resources = root / 'src' / 'main' / 'resources'
    resources.mkdir(parents=True, exist_ok=True)
    (resources / 'pack.mcmeta').write_text(
        json.dumps({'pack': {'description': 'MojModelBridge generated resources', 'pack_format': pack_format}}, indent=2) + '\n',
        encoding='utf-8'
    )


_MODERN_BUILTIN_SUPER_MODELS = {
    "HumanoidModel": "Biped",
    "ZombieModel": "Zombie",
    "CowModel": "Cow",
    "PigModel": "Pig",
    "SpiderModel": "Spider",
    "WolfModel": "Wolf",
    "ChickenModel": "Chicken",
    "SheepModel": "Sheep",
}


def _class_superclass(source: Path) -> str:
    try:
        text = _run_javap(source)
    except Exception:
        return ""
    for line in text.splitlines():
        if line.startswith("public class ") or line.startswith("public abstract class ") or line.startswith("class "):
            match = re.search(r"\bclass\s+[A-Za-z0-9_.$]+(?:<[^>]+>)?\s+extends\s+([A-Za-z0-9_.$]+)", line)
            if match:
                return _simple(match.group(1))
    return ""


def _is_modern_model_bytecode(source: Path) -> bool:
    try:
        text = _run_javap(source)
    except Exception:
        return False
    return "CubeListBuilder" in text and ("PartDefinition" in text or "MeshDefinition" in text or "LayerDefinition" in text)


def _parse_mcreator_model(source: Path, graph: LegacyModGraph, constructor_args: list[Any], cube_cls: Any, part_cls: Any, parsed_model_cls: Any, seen: Optional[set[str]] = None) -> tuple[Any, str, Optional[str]]:
    """Parse legacy or modern compiled geometry, falling back to local superclass geometry."""
    seen = set(seen or set())
    key = str(source.resolve())
    if key in seen:
        raise RuntimeError(f"Recursive model superclass chain detected at {source.name}.")
    seen.add(key)
    if source.suffix.lower() == ".java":
        from model_bridge import parse_java_model, parse_legacy_java_model
        text = source.read_text(encoding="utf-8-sig", errors="ignore")
        modern_markers = ("MeshDefinition", "PartDefinition", "CubeListBuilder", "LayerDefinition")
        parsers = (
            ((parse_java_model, "modern_java_source"), (parse_legacy_java_model, "legacy_java_source"))
            if any(marker in text for marker in modern_markers)
            else ((parse_legacy_java_model, "legacy_java_source"), (parse_java_model, "modern_java_source"))
        )
        errors: list[str] = []
        for parser, kind in parsers:
            try:
                model = (
                    parser(text, source.name, list(constructor_args or []))
                    if parser is parse_legacy_java_model
                    else parser(text, source.name, list(constructor_args or []))
                )
                if getattr(model, "cube_count", 0) > 0:
                    return model, kind, None
                errors.append(f"{kind}: zero cubes")
            except Exception as exc:
                errors.append(f"{kind}: {exc}")
        raise RuntimeError(f"Could not recover Java model {source.name}: {'; '.join(errors)}")
    if _is_modern_model_bytecode(source):
        try:
            model = parse_modern_bytecode_class(source, cube_cls, part_cls, parsed_model_cls, constructor_args=constructor_args)
            return model, "modern", None
        except RuntimeError as exc:
            superclass = _class_superclass(source)
            if superclass and superclass in graph.class_files and superclass not in {"ModelBase", "EntityModel", "HumanoidModel", "ZombieModel", "CowModel", "PigModel", "SpiderModel", "WolfModel", "ChickenModel", "SheepModel", "ListModel"}:
                base_model, base_kind, _ = _parse_mcreator_model(graph.class_files[superclass], graph, [], cube_cls, part_cls, parsed_model_cls, seen)
                base_model.class_name = _pascal(source.stem)
                base_model.source_format = f"{base_kind}_inherited_geometry"
                base_model.warnings.append(f"Geometry inherited from local superclass {superclass}; subclass-specific runtime animation/render logic is preserved only as a source warning.")
                return base_model, base_kind, superclass
            raise exc
    try:
        model = parse_legacy_class(
            source,
            cube_cls,
            part_cls,
            parsed_model_cls,
            resource_roots=graph.roots,
            constructor_args=constructor_args,
        )
        return model, "legacy", None
    except Exception as compiled_exc:
        # Decompiled source can preserve inherited vanilla geometry declarations
        # that a concrete subclass does not recreate in its own bytecode (notably
        # ModelPlayer/ModelBiped subclasses). Prefer the matching source file over
        # dropping the entity or inventing empty geometry.
        simple = source.stem.split("$")[0]
        candidates = sorted({
            path.resolve()
            for source_root in graph.roots
            for path in source_root.rglob(f"{simple}.java")
            if path.is_file() and "model" in path.as_posix().lower()
        })
        if candidates:
            try:
                from model_bridge import parse_legacy_java_model
                text = candidates[0].read_text(encoding="utf-8-sig", errors="ignore")
                model = parse_legacy_java_model(text, candidates[0].name, list(constructor_args or []))
                model.warnings.append(
                    f"Compiled model recovery failed ({compiled_exc}); geometry recovered from decompiled source {candidates[0].name}."
                )
                return model, "legacy_java_source", None
            except Exception:
                pass
        raise compiled_exc



def _ensure_import_folder_tree(workspace: dict[str, Any], import_modid: str, categories: set[str]) -> None:
    root_name = str(import_modid or "converted_mod").strip() or "converted_mod"
    if root_name.lower() == "lostinfinity":
        root_name = "LostInfinity"
    folders_root = workspace.setdefault("foldersRoot", {"name": "~", "children": []})
    children = folders_root.setdefault("children", [])
    root = next((item for item in children if str(item.get("name", "")).lower() == root_name.lower()), None)
    if root is None:
        root = {"name": root_name, "children": []}
        children.append(root)
    root_children = root.setdefault("children", [])
    for category in sorted(categories):
        parent_children = root_children
        for segment in [part for part in str(category).replace("\\", "/").split("/") if part]:
            child = next((item for item in parent_children if str(item.get("name", "")).lower() == segment.lower()), None)
            if child is None:
                child = {"name": segment, "children": []}
                parent_children.append(child)
            parent_children = child.setdefault("children", [])


def export_mcreator_workspace(
    graph: LegacyModGraph,
    output_dir: Path,
    template_workspace: Optional[Path] = None,
    log: Optional[Callable[[str], None]] = None,
    modid: Optional[str] = None,
    mod_name: Optional[str] = None,
    mod_author: Optional[str] = None,
    package: Optional[str] = None,
    target_generator: str = "neoforge-1.21.1",
    import_modid: Optional[str] = None,
    preparsed_models: Optional[dict[tuple[str, str], Any]] = None,
    resume: bool = False,
) -> MCreatorExportResult:
    """Generate or inject an MCreator workspace for the selected generator target.

    The output deliberately limits automatic conversion to entities whose
    renderer/model relationship can be recovered. Unsupported custom gameplay
    logic is written to reports instead of being silently approximated.
    """
    output_dir = Path(output_dir).expanduser().resolve()
    resume_file = _find_workspace_file(output_dir) if resume and output_dir.exists() and not template_workspace else None
    if resume_file is None:
        if output_dir.exists():
            shutil.rmtree(output_dir)
        output_dir.mkdir(parents=True, exist_ok=True)

    inferred_metadata = infer_mod_metadata(graph.root)
    source_import_modid = _snake(import_modid or inferred_metadata.modid or graph.root.name or "converted_mod")

    if resume_file is not None:
        workspace_file = resume_file
        workspace = json.loads(workspace_file.read_text(encoding="utf-8-sig"))
        if not isinstance(workspace, dict) or "workspaceSettings" not in workspace or "mod_elements" not in workspace:
            raise RuntimeError("The resumable MCreator workspace is missing workspaceSettings/mod_elements.")
        living_template = _find_living_template(output_dir)
        _log(log, f"Resuming MCreator workspace: {workspace_file.name} ({len(workspace.get('mod_elements', []))} existing elements).")
    elif template_workspace:
        template_workspace = Path(template_workspace).expanduser().resolve()
        _log(log, f"Cloning MCreator template workspace: {template_workspace}")
        _copy_template_workspace(template_workspace, output_dir)
        workspace_file = _find_workspace_file(output_dir)
        if workspace_file is None:
            raise RuntimeError("The selected MCreator template does not contain a readable .mcreator workspace file.")
        workspace = json.loads(workspace_file.read_text(encoding="utf-8-sig"))
        if not isinstance(workspace, dict) or "workspaceSettings" not in workspace or "mod_elements" not in workspace:
            raise RuntimeError("The selected MCreator template is missing workspaceSettings/mod_elements and is not a valid workspace definition.")
        _log(log, f"Selected MCreator workspace definition: {workspace_file.name} (MCreator {workspace.get('mcreatorVersion', 0)}, {len(workspace.get('mod_elements', []))} elements).")
        living_template = _find_living_template(output_dir)
    else:
        inferred_modid = _snake(modid or inferred_metadata.modid or graph.root.name or "converted_mod")
        inferred_name = mod_name or inferred_metadata.name or _pascal(inferred_modid)
        inferred_package = package or f"net.mojmodelbridge.{inferred_modid}"
        workspace = _default_workspace(inferred_modid, inferred_name, inferred_package, target_generator)
        standalone_settings = workspace["workspaceSettings"]
        if inferred_metadata.version:
            standalone_settings["version"] = inferred_metadata.version
        if inferred_metadata.author_text:
            standalone_settings["author"] = inferred_metadata.author_text
        if inferred_metadata.website:
            standalone_settings["websiteURL"] = inferred_metadata.website
        if inferred_metadata.license:
            standalone_settings["license"] = inferred_metadata.license
        if inferred_metadata.description:
            standalone_settings["description"] = inferred_metadata.description
        standalone_settings["credits"] = (
            (standalone_settings.get("credits", "") + "; " if standalone_settings.get("credits") else "")
            + "Original metadata preserved by MojModelBridge"
        )
        workspace_file = output_dir / f"{inferred_modid}.mcreator"
        living_template = None
        _log(log, "Generating a standalone MCreator Forge 1.20.1 workspace skeleton.")

    settings = workspace.setdefault("workspaceSettings", {})
    target_modid = _snake(modid or str(settings.get("modid") or inferred_metadata.modid or graph.root.name or "converted_mod"))
    if template_workspace:
        workspace_file = _canonicalize_workspace_definition(output_dir, workspace_file, target_modid)
        _log(log, f"Canonical workspace definition: {workspace_file.name}; removed duplicate .mcreator definitions.")
    settings["modid"] = target_modid
    if mod_name:
        settings["modName"] = mod_name
    if mod_author is not None and mod_author.strip():
        settings["author"] = mod_author.strip()
    settings.setdefault("currentGenerator", target_generator)
    if target_generator in {"forge-1.20.1", "neoforge-1.21.1"}:
        settings["currentGenerator"] = target_generator
    settings.setdefault("modElementsPackage", package or f"net.mojmodelbridge.{target_modid}")
    workspace.setdefault("language_map", {}).setdefault("en_us", {})
    workspace.setdefault("mod_elements", [])
    if template_workspace:
        _ensure_import_folder_tree(workspace, source_import_modid, {"Mobs", "Blocks", "Items", "Tools", "Weapons", "Foods", "Armor", "Plants", "Projectiles", "Recipes", "World Generation/Dimensions", "World Generation/Biomes"})
    if template_workspace:
        _normalise_template_resource_locations(output_dir, target_modid, log=log)
    if not template_workspace:
        _write_standalone_scaffold(output_dir, target_modid, str(settings.get("currentGenerator") or target_generator))

    elements_dir = output_dir / "elements"
    model_generator_key = "mojmap-1.21.x" if settings.get("currentGenerator") == "neoforge-1.21.1" else "mojmap-1.20.x"
    models_dir = output_dir / "models" / model_generator_key
    animations_dir = output_dir / "models" / "animations"
    textures_root = output_dir / "src" / "main" / "resources" / "assets" / target_modid / "textures"
    textures_dir = textures_root / "entities"
    for directory in (elements_dir, models_dir, animations_dir, textures_root, textures_dir):
        directory.mkdir(parents=True, exist_ok=True)

    def checkpoint_workspace() -> None:
        workspace_file.write_text(json.dumps(workspace, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    # A valid checkpoint is written before expensive model/entity generation so
    # an interrupted full-port can be resumed with --no-overwrite.
    checkpoint_workspace()

    # Copy original mod assets/data into the cloned working workspace without overwriting
    # known-good template resources. This gives the MCreator project a real resource base
    # even when the source corpus contains only classes/renderers.
    try:
        for source_root in graph.roots:
            _copy_source_resources(source_root, output_dir / "src" / "main" / "resources", log=log, source_modid=source_import_modid, target_modid=target_modid if template_workspace else source_import_modid)
        for warning in _relocate_stale_source_resources(output_dir, source_import_modid, target_modid):
            _log(log, f"MCreator namespace/resource cleanup: {warning}")
        for warning in _finalize_worldgen_namespace(output_dir, source_import_modid, target_modid):
            _log(log, f"MCreator worldgen finalization: {warning}")
    except Exception as exc:
        _log(log, f"MCreator resource bridge warning: {exc}")

    texture_search_roots = list(graph.roots) if template_workspace else [*graph.roots, output_dir / "src" / "main" / "resources", output_dir / "assets"]
    # Optional bundled reference assets make the standalone full-port path useful when
    # a legacy source corpus contains only .class files. Any bundled per-mod reference
    # resources are used only when the target modid matches and never overwrite copied
    # resources from the source mod or a supplied template.
    bundled_reference = Path(__file__).resolve().parent / "reference_resources" / target_modid
    if bundled_reference.exists():
        texture_search_roots.append(bundled_reference)
        _log(log, f"Bundled reference resource fallback enabled: {bundled_reference}")
    texture_file_index = _build_texture_file_index(texture_search_roots)
    _log(log, f"MCreator entity texture index: {len(texture_file_index)} PNG files indexed once.")

    # Import classes lazily to avoid a circular import when model_bridge invokes us.
    from model_bridge import Cube, Part, ParsedModel, _expand_render_instance_parts

    exports: list[MCreatorEntityExport] = []
    global_warnings: list[str] = []
    used_elements = {str(item.get("name", "")).lower() for item in workspace.get("mod_elements", [])}
    used_model_classes: set[str] = set()
    used_registry_names = {str(item.get("registry_name", "")).lower() for item in workspace.get("mod_elements", [])}
    existing_living_by_name = {
        str(item.get("name", "")).lower(): item
        for item in workspace.get("mod_elements", [])
        if item.get("type") == "livingentity"
    }
    existing_living_by_registry = {
        str(item.get("registry_name", "")).lower(): item
        for item in workspace.get("mod_elements", [])
        if item.get("type") == "livingentity"
    }
    existing_living_by_compact_registry = {
        _compact_registry(item.get("registry_name", "")): item
        for item in workspace.get("mod_elements", [])
        if item.get("type") == "livingentity" and _compact_registry(item.get("registry_name", ""))
    }
    # Preload all compiled classes once. Both the entity/model path and the non-entity
    # content path can then reuse the same bounded javap cache.
    try:
        cached_count, fallback_count = prime_javap_cache(list(graph.class_files.values()))
        _log(log, f"MCreator full-port bytecode preload: {cached_count} cached, {fallback_count} fallback.")
    except Exception as exc:
        _log(log, f"MCreator full-port bytecode preload warning: {exc}")

    model_class_by_key: dict[tuple[str, str], str] = {}
    parsed_model_cache: dict[tuple[str, str], Any] = {}
    supplied_models = preparsed_models or {}
    animation_record_cache: dict[tuple[str, str], list[dict[str, Any]]] = {}

    registrations = [reg for reg in graph.registrations if reg.model_class and reg.entity_class]
    entity_language_names = _load_language_names(graph.roots)
    sound_count, sound_warnings = _recover_sound_elements(graph.roots, workspace, entity_language_names, log=log)
    global_warnings.extend(sound_warnings)
    vanilla_models = {
        "ModelBiped": "Biped",
        "ModelCow": "Cow",
        "ModelSpider": "Spider",
        "ModelPig": "Pig",
        "ModelSheep1": "Sheep",
        "ModelSheep2": "Sheep",
        "ModelChicken": "Chicken",
        "ModelWolf": "Wolf",
        "ModelZombie": "Zombie",
        "ModelSkeleton": "Skeleton",
        "HumanoidModel": "Biped",
        "ZombieModel": "Zombie",
        "CowModel": "Cow",
        "PigModel": "Pig",
        "SpiderModel": "Spider",
        "WolfModel": "Wolf",
        "ChickenModel": "Chicken",
        "SheepModel": "Sheep",
    }
    _log(log, f"MCreator export: {len(registrations)} renderer/entity registrations are eligible.")
    for index, reg in enumerate(registrations, 1):
        source_model = graph.class_files.get(reg.model_class) or graph.java_files.get(reg.model_class)
        if source_model is None:
            for source_root in graph.roots:
                candidates = sorted(source_root.rglob(f"{reg.model_class}.java"))
                if candidates:
                    source_model = candidates[0]
                    break
        meta = graph.entities.get(reg.entity_class) or EntityMetadata(
            class_name=reg.entity_class,
            display_name=_pascal(reg.entity_class),
            registry_name=_snake(reg.entity_class),
        )
        meta.display_name = _translated_name(
            entity_language_names,
            meta.registry_name or reg.entity_class,
            meta.display_name or _display_name(reg.entity_class),
        )
        desired_name = _pascal(re.sub(r"^Entity", "", reg.entity_class))
        exact_source_registry = graph.registry_names.get(reg.entity_class)
        desired_registry = _snake(exact_source_registry or meta.registry_name or reg.entity_class)
        existing_descriptor = existing_living_by_registry.get(desired_registry.lower()) or existing_living_by_compact_registry.get(_compact_registry(desired_registry))
        if existing_descriptor is not None:
            resumed = (
                existing_descriptor.get("mojmodelbridge_generated") is True
                or str(existing_descriptor.get("path", "")).startswith("~/Converted ")
                or str(existing_descriptor.get("mojmodelbridge_source_modid", "")).lower() == source_import_modid.lower()
            )
            if resumed:
                element_name = str(existing_descriptor.get("name") or desired_name)
                registry_name = str(existing_descriptor.get("registry_name") or desired_registry)
                existing_definition: dict[str, Any] = {}
                existing_path = elements_dir / f"{element_name}.mod.json"
                try:
                    existing_payload = json.loads(existing_path.read_text(encoding="utf-8"))
                    if isinstance(existing_payload.get("definition"), dict):
                        existing_definition = existing_payload["definition"]
                except Exception:
                    existing_definition = {}
                animation_refs = [str(a["animation"]) for a in existing_definition.get("animations", []) if isinstance(a, dict) and a.get("animation")]
                texture_name = str(existing_definition.get("mobModelTexture") or "")
                exports.append(MCreatorEntityExport(
                    element_name, registry_name, reg.entity_class, reg.renderer_class, reg.model_class,
                    str(existing_definition.get("mobModelName") or ""), list(reg.model_args), texture_name, texture_name,
                    ("existing" if texture_name else "missing"), animation_refs, [], "resumed"
                ))
                continue
            desired_registry = f"{source_import_modid}_{desired_registry}"
            desired_name = f"{desired_name}_{source_import_modid}"
        element_name = _unique_element_name(desired_name, used_elements)
        registry_name = desired_registry
        if registry_name.lower() in used_registry_names:
            registry_name = _snake(f"{source_import_modid}_{registry_name}")
        used_registry_names.add(registry_name.lower())
        args_key = json.dumps(reg.model_args, sort_keys=True, default=str)
        model_key = (reg.model_class, args_key)
        model_class = model_class_by_key.get(model_key, "")
        if not model_class:
            model_class = vanilla_models.get(reg.model_class, _pascal(reg.model_class))
            if model_class in used_model_classes:
                model_class = _pascal(f"{reg.model_class} {element_name}")
            used_model_classes.add(model_class)
            model_class_by_key[model_key] = model_class
        sound_bindings: list[str] = []
        record = MCreatorEntityExport(
            element_name=element_name,
            registry_name=registry_name,
            entity_class=reg.entity_class,
            renderer_class=reg.renderer_class,
            source_model=reg.model_class,
            generated_model=model_class,
            model_args=list(reg.model_args),
            sound_bindings=sound_bindings,
            status="created",
        )
        _log(log, f"[{index}/{len(registrations)}] {reg.entity_class} -> {reg.renderer_class} -> {reg.model_class}{tuple(reg.model_args)}")
        model = parsed_model_cache.get(model_key)
        if model is None and source_model is not None:
            try:
                source_cache_key = str(source_model.expanduser().resolve())
            except OSError:
                source_cache_key = str(source_model)
            model = supplied_models.get((source_cache_key, args_key))
            if model is not None:
                parsed_model_cache[model_key] = model
        animation_records: list[dict[str, Any]] = []
        entity_source_text = _source_text_for_class(graph, reg.entity_class)
        sound_updates, sound_bindings = infer_sound_bindings(
            entity_source_text, reg.entity_class, "livingentity", {"sound_elements": workspace.get("sound_elements", [])}
        )
        if source_model is None and reg.model_class in vanilla_models:
            record.warnings.append(f"Uses Minecraft/MCreator built-in model '{model_class}'; no custom model Java was required.")
        elif source_model is None:
            record.warnings.append(f"Model class file not found for {reg.model_class}.")
            exports.append(record)
            continue
        elif model is None:
            try:
                model, model_kind, inherited_from = _parse_mcreator_model(
                    source_model, graph, list(reg.model_args), Cube, Part, ParsedModel
                )
                if inherited_from:
                    record.warnings.append(f"Model geometry inherited from {inherited_from}.")
            except Exception as exc:
                # Modern subclasses of vanilla models often contribute only animation/state
                # logic. Let MCreator use its built-in model when the geometry is supplied by
                # HumanoidModel/ZombieModel/etc. rather than emitting a broken empty custom model.
                superclass = _class_superclass(source_model)
                builtin = vanilla_models.get(superclass)
                if builtin:
                    model = None
                    record.generated_model = builtin
                    record.warnings.append(f"Uses built-in MCreator model '{builtin}' inherited through {superclass}; custom subclass geometry was not required.")
                else:
                    record.warnings.append(f"Model parser failed: {exc}")
                    exports.append(record)
                    continue
            parsed_model_cache[model_key] = model

        cached_animation_records = animation_record_cache.get(model_key)
        if model is not None and cached_animation_records is None:
            model_java = model_to_mojmap_java(model, model_class, target_modid)
            (models_dir / f"{model_class}.java").write_text(model_java, encoding="utf-8")
            animation_java, animation_records = animation_definition_java(model, model_class)
            if animation_java:
                (animations_dir / f"{model_class}Animation.java").write_text(animation_java, encoding="utf-8")
            animation_record_cache[model_key] = animation_records
        elif model is not None:
            animation_records = cached_animation_records or []
        record.animations = [entry["source_name"] for entry in animation_records]

        texture_ref, texture_source, texture_warnings = _select_texture(
            texture_search_roots, reg, texture_file_index
        )
        record.texture_ref = texture_ref
        record.warnings.extend(texture_warnings)
        # Modern ResourceLocation paths are lowercase-only. Preserve the original reference
        # in the report, but normalize the copied/generated asset filename used by Java and
        # MCreator so paths such as GammaMetroid.png cannot crash client resource reload.
        raw_texture_name = Path(texture_ref).name if texture_ref else f"{registry_name}.png"
        texture_name = re.sub(r"[^a-z0-9._-]", "_", raw_texture_name.lower())
        if not texture_name.endswith(".png"):
            texture_name += ".png"
        target_texture = textures_dir / texture_name
        if texture_source:
            # Avoid silently overwriting a different same-named texture.
            if target_texture.exists() and target_texture.read_bytes() != texture_source.read_bytes():
                texture_name = f"{registry_name}_{texture_name}"
                target_texture = textures_dir / texture_name
            if texture_source.resolve() != target_texture.resolve():
                shutil.copy2(texture_source, target_texture)
            source_meta = texture_source.with_name(texture_source.name + ".mcmeta")
            if source_meta.exists():
                shutil.copy2(source_meta, target_texture.with_name(target_texture.name + ".mcmeta"))
            record.texture_output = str(target_texture)
            record.texture_status = "copied"
        else:
            if not target_texture.exists():
                _write_placeholder_png(target_texture)
            record.texture_output = str(target_texture)
            record.texture_status = "placeholder"
            record.warnings.append(
                f"Generated a checker placeholder texture at {target_texture.name}; replace it with the recovered/original PNG."
            )

        fv, definition = _clone_definition(living_template)
        _apply_sound_bindings(definition, sound_updates)
        spawn_egg_colors = _extract_spawn_egg_colors(graph.roots, registry_name, reg.entity_class)
        spawn_egg_texture_name, spawn_egg_texture_source = _find_spawn_egg_texture(graph.roots, registry_name, reg.entity_class)
        if spawn_egg_texture_source is not None:
            egg_target = textures_dir / (re.sub(r"[^a-z0-9._-]", "_", spawn_egg_texture_source.name.lower()))
            if spawn_egg_texture_source.resolve() != egg_target.resolve():
                egg_target.parent.mkdir(parents=True, exist_ok=True)
                if not egg_target.exists():
                    shutil.copy2(spawn_egg_texture_source, egg_target)
                meta_src=spawn_egg_texture_source.with_name(spawn_egg_texture_source.name+".mcmeta")
                if meta_src.exists(): shutil.copy2(meta_src, egg_target.with_name(egg_target.name+".mcmeta"))
        element = _make_element(
            fv, definition, element_name, model_class, texture_name, meta, reg, animation_records, spawn_egg_colors
        )
        if spawn_egg_texture_name:
            element["definition"]["spawnEggTexture"] = spawn_egg_texture_name
        element_path = elements_dir / f"{element_name}.mod.json"
        element_path.write_text(json.dumps(element, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

        workspace["mod_elements"].append({
            "name": element_name,
            "type": "livingentity",
            "compiles": True,
            "locked_code": False,
            "registry_name": registry_name,
            "metadata": {"files": []},
            "path": f"~/{source_import_modid}/Mobs",
            "mojmodelbridge_generated": True,
                    "mojmodelbridge_source_modid": source_import_modid,
                    "mojmodelbridge_source_registry": registry_name,
        })
        lang_map = workspace["language_map"]["en_us"]
        for _key in (
            f"entity.{target_modid}.{registry_name}",
            f"entity.{target_modid}.{registry_name}.name",
            f"entity.{target_modid}.{element_name}",
            f"entity.{target_modid}.{element_name}.name",
        ):
            lang_map[_key] = meta.display_name
        record.warnings.extend(meta.warnings)
        # Keep unsupported AI/state information explicit.
        if meta.ai_classes:
            known_tokens = ("swimming", "attackoncollide", "meleeattack", "wander", "hurtbytarget", "nearestattackabletarget", "lookidle", "watchclosest")
            unsupported = [name for name in meta.ai_classes if not any(token in name.lower() for token in known_tokens)]
            if unsupported:
                record.warnings.append(f"Custom/unsupported AI goals retained in report only: {unsupported}")
        if model is not None:
            record.warnings.extend(w for w in model.warnings if "Unsupported" in w or "external animation" in w.lower())
        exports.append(record)
        if index % 20 == 0:
            checkpoint_workspace()

    checkpoint_workspace()

    # Hundreds of animation-heavy ParsedModel objects can retain a substantial
    # amount of memory. Non-entity content does not use them; release the cache
    # before scanning thousands of item/block classes so large full-port runs do
    # not degrade into swap-bound execution after entity export.
    if isinstance(preparsed_models, dict):
        preparsed_models.clear()
    try:
        del model
    except UnboundLocalError:
        pass
    import gc
    gc.collect()

    _log(log, "MCreator export: recovering non-entity content (items, tools, armor, blocks, dimensions, biomes, plants, projectiles).")
    content_result = export_mcreator_content(
        graph=graph,
        workspace=workspace,
        workspace_root=output_dir,
        output_elements=elements_dir,
        texture_root=textures_root,
        texture_source_roots=texture_search_roots,
        modid=target_modid,
        import_modid=source_import_modid,
        log=log,
    )
    global_warnings.extend(content_result.warnings)
    for warning in _sanitize_generated_item_models(output_dir, target_modid):
        global_warnings.append(f"Item model sanitation: {warning}")
        _log(log, f"MCreator item-model sanitation: {warning}")

    # Template collisions can namespace an imported registry (for example
    # ``deepslate_oresalt`` -> ``chaospersists_deepslate_oresalt``). Rebind all
    # copied data-pack references to the actual registry names selected by the
    # imported MCreator descriptors so worldgen never points at a stale block ID.
    imported_registry_aliases: dict[str, tuple[str, str]] = {}
    for descriptor in workspace.get("mod_elements", []):
        if str(descriptor.get("mojmodelbridge_source_modid", "")).lower() != source_import_modid.lower():
            continue
        source_registry = str(descriptor.get("mojmodelbridge_source_registry", "") or "").strip()
        actual_registry = str(descriptor.get("registry_name", "") or "").strip()
        if source_registry and actual_registry:
            imported_registry_aliases[_compact_registry(source_registry)] = (source_registry, actual_registry)

    if template_workspace and imported_registry_aliases:
        data_root = output_dir / "src" / "main" / "resources" / "data" / target_modid
        if data_root.exists():
            for path in data_root.rglob("*.json"):
                try:
                    payload_text = path.read_text(encoding="utf-8-sig", errors="replace")
                except OSError:
                    continue
                changed = False
                for source_compact, (original_path, actual_registry) in imported_registry_aliases.items():
                    target_token = f"{target_modid}:{original_path}"
                    actual_token = f"{target_modid}:{actual_registry}"
                    if target_token in payload_text:
                        payload_text = payload_text.replace(target_token, actual_token)
                        changed = True
                if changed:
                    path.write_text(payload_text, encoding="utf-8")

    entity_meta = {}
    for reg in registrations:
        meta = graph.entities.get(reg.entity_class)
        if meta is not None:
            # The exporter uses the desired element/registry name as the lookup key.
            entity_meta[_snake(meta.registry_name or reg.entity_class)] = vars(meta)
    entity_meta_by_element = {}
    for item in exports:
        key = item.element_name.lower()
        meta = graph.entities.get(item.entity_class)
        entity_meta_by_element[key] = vars(meta) if meta is not None else {}

    bridge_result = generate_compile_bridge(
        output_root=output_dir,
        package=str(settings.get("modElementsPackage") or package or f"net.mojmodelbridge.{target_modid}"),
        modid=target_modid,
        workspace=workspace,
        entity_exports=exports,
        content_exports=content_result.exports,
        entity_meta=entity_meta_by_element,
        log=log,
        target_generator=str(settings.get("currentGenerator") or target_generator),
    )

    # Content export can create new biome modifiers/placed-feature references after the
    # initial resource bridge. Run the worldgen finalizer at the very end so no stale
    # source namespace can survive into the built workspace.
    for warning in _finalize_worldgen_namespace(output_dir, source_import_modid, target_modid):
        global_warnings.append(f"Final worldgen cleanup: {warning}")
        _log(log, f"Final worldgen cleanup: {warning}")

    # One canonical creative tab for the imported mod. Use the actual export
    # results as the source of truth because content exporters can emit files
    # before their workspace metadata entry is finalized.
    try:
        mod_display_name = str(workspace.get("workspaceSettings", {}).get("modName") or mod_name or target_modid or "Converted Mod").strip()
        tab_registry = _snake(f"{mod_display_name}_content")
        existing_names = {str(e.get("name", "")).lower() for e in workspace.get("mod_elements", [])}
        tab_name = next((str(e.get("name")) for e in workspace.get("mod_elements", [])
                         if str(e.get("type", "")).lower() == "tab" and
                         str(e.get("registry_name", "")).lower() == tab_registry.lower()), None)
        generated_refs = []
        for item in list(exports) + list(getattr(content_result, "exports", [])):
            status = str(getattr(item, "status", "created"))
            if status not in {"created", "resumed"}:
                continue
            typ = str(getattr(item, "element_type", "") or getattr(item, "type", "")).lower()
            if typ in {"item", "tool", "block", "plant", "armor", "livingentity", "projectile"}:
                name = str(getattr(item, "element_name", "") or getattr(item, "name", ""))
                if name and name not in generated_refs:
                    generated_refs.append(name)
        # Also include existing workspace elements explicitly marked as imported.
        for entry in workspace.get("mod_elements", []):
            if str(entry.get("mojmodelbridge_source_modid", "")).lower() == source_import_modid.lower():
                if str(entry.get("type", "")).lower() in {"item", "tool", "block", "plant", "armor", "livingentity", "projectile"}:
                    name = str(entry.get("name", ""))
                    if name and name not in generated_refs:
                        generated_refs.append(name)
        if tab_name is None:
            tab_name = _unique_name(f"{_pascal(mod_display_name)}Content", existing_names)
            icon = f"CUSTOM:{generated_refs[0]}" if generated_refs else "Items.APPLE"
            tab_element = {
                "_fv": 89, "_type": "tab",
                "definition": {"name": mod_display_name, "icon": icon, "showSearch": len(generated_refs) >= 24},
            }
            (output_dir / "elements" / f"{tab_name}.mod.json").write_text(
                json.dumps(tab_element, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )
            workspace.setdefault("mod_elements", []).append({
                "name": tab_name, "type": "tab", "compiles": True, "locked_code": False,
                "registry_name": tab_registry, "metadata": {"files": []},
                "path": _mcreator_import_path(source_import_modid, "tab"),
                "mojmodelbridge_generated": True,
                "mojmodelbridge_source_modid": source_import_modid,
                "mojmodelbridge_source_registry": tab_registry,
            })
        for name in generated_refs:
            element_path = output_dir / "elements" / f"{name}.mod.json"
            payload = _read_json(element_path)
            if not payload or not isinstance(payload.get("definition"), dict):
                continue
            definition = payload["definition"]
            if "creativeTabs" in definition:
                definition["creativeTabs"] = [f"CUSTOM:{tab_name}"]
                payload["definition"] = definition
                element_path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    except Exception as exc:
        global_warnings.append(f"Mod creative-tab finalizer failed: {exc}")

    workspace["mod_elements"].sort(key=lambda item: (str(item.get("type", "")), str(item.get("name", "")).lower()))
    workspace_file.write_text(json.dumps(workspace, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    try:
        from workspace_validator import validate_workspace
        validation = validate_workspace(output_dir, workspace=workspace, write_reports=True)
        validation_payload = validation.to_dict()
        _log(log, f"Workspace validation: {len(validation.errors)} error(s), {len(validation.warnings)} warning(s).")
    except Exception as exc:
        validation_payload = {"ok": False, "errors": [{"code": "validator_failed", "message": str(exc)}], "warnings": []}
        global_warnings.append(f"Workspace validator failed: {exc}")

    payload = {
        "format": "MojModelBridge MCreator export report v2",
        "workspace": str(workspace_file),
        "target_generator": settings.get("currentGenerator"),
        "modid": target_modid,
        "summary": {
            "renderer_links": len(registrations),
            "element_files_written": sum(1 for item in exports if item.status == "created" and (elements_dir / f"{item.element_name}.mod.json").exists()),
            "entity_created": sum(1 for item in exports if item.status == "created"),
            "entity_existing": sum(1 for item in exports if item.status == "existing"),
            "entity_resumed": sum(1 for item in exports if item.status == "resumed"),
            "model_files_written": len(list(models_dir.glob("*.java"))),
            "animation_files_written": len(list(animations_dir.glob("*.java"))),
            "textures_copied": sum(1 for item in exports if item.texture_status == "copied"),
            "placeholder_textures": sum(1 for item in exports if item.texture_status == "placeholder"),
            "failed_or_partial": sum(1 for item in exports if item.warnings),
            "content_created": sum(1 for item in content_result.exports if item.status == "created"),
            "content_existing": sum(1 for item in content_result.exports if item.status == "existing"),
            "content_resumed": sum(1 for item in content_result.exports if item.status == "resumed"),
            "content_by_type": {
                element_type: sum(1 for item in content_result.exports if item.element_type == element_type and item.status in {"created", "resumed"})
                for element_type in sorted({item.element_type for item in content_result.exports})
            },
            "compile_bridge": bridge_result,
        },
        "entities": [vars(item) for item in exports],
        "content": [vars(item) for item in content_result.exports],
        "workspace_validation": validation_payload,
        "warnings": global_warnings,
        "scope": {
            "implemented": [
                "living entity element JSON", "renderer-to-model linkage", "legacy constructor arguments",
                "Mojmap 1.20.x Java models", "baked AnimationDefinition Java", "textures when present",
                "basic attributes", "basic vanilla AI mapping", "spawn eggs",
                "items", "tools and weapons", "armor sets", "blocks", "plants", "projectiles", "ore-only world generation",
                "template workspace preservation and canonical .mcreator definition",
            ],
            "report_only": [
                "custom entity state and helper AI", "loot and drops", "sounds absent from archive",
                "spawn biome rules", "custom renderer layers", "recipes from arbitrary legacy registration bytecode",
                "loot tables from arbitrary compiled code", "biomes/dimensions/villages/structures intentionally disabled", "non-ore custom world generation", "custom GUI and procedures",
            ],
        },
    }
    report_json = output_dir / "mcreator_export_report.json"
    report_json.write_text(json.dumps(payload, indent=2, ensure_ascii=False, default=str) + "\n", encoding="utf-8")

    rows = []
    for item in exports:
        status = "partial" if item.warnings else "ok"
        rows.append(
            f"<tr><td>{item.element_name}</td><td>{item.entity_class}</td><td>{item.renderer_class}</td>"
            f"<td>{item.source_model} → {item.generated_model}</td><td>{item.texture_ref or 'missing'}</td>"
            f"<td>{', '.join(item.animations) or 'none'}</td><td>{item.status}</td><td class='{status}'>{'<br>'.join(item.warnings) or 'Ready'}</td></tr>"
        )
    summary = payload["summary"]
    report_html = output_dir / "mcreator_export_report.html"
    report_html.write_text(
        "<!doctype html><html><head><meta charset='utf-8'><title>MCreator export report</title>"
        "<style>body{font-family:Segoe UI,Arial;background:#15171c;color:#eee;margin:24px}table{border-collapse:collapse;width:100%}"
        "th,td{border:1px solid #444;padding:7px;vertical-align:top}th{background:#292d36}.ok{color:#79d279}.partial{color:#ffd166}</style></head><body>"
        f"<h1>MojModelBridge MCreator export</h1><p>Workspace: <code>{workspace_file}</code></p>"
        f"<p>{summary['element_files_written']} entity elements · {summary['content_created']} non-entity content elements created · "
        f"{summary['model_files_written']} models · {summary['animation_files_written']} animation files · "
        f"{summary['textures_copied']} entity textures copied · {summary['placeholder_textures']} entity placeholders</p>"
        "<h2>Entities</h2>"
        "<table><thead><tr><th>Element</th><th>Entity</th><th>Renderer</th><th>Model</th><th>Texture</th><th>Animations</th><th>Conversion</th><th>Details</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody></table>"
        "<h2>Non-entity content</h2><table><thead><tr><th>Element</th><th>Type</th><th>Source</th><th>Texture</th><th>Status</th><th>Warnings</th></tr></thead><tbody>"
        + "".join(
            f"<tr><td>{item.element_name}</td><td>{item.element_type}</td><td>{item.source_class}</td><td>{item.texture_ref or 'n/a'}</td><td>{item.status}</td><td>{'<br>'.join(item.warnings) or 'Ready'}</td></tr>"
            for item in content_result.exports
        )
        + "</tbody></table></body></html>",
        encoding="utf-8",
    )

    instructions = output_dir / "MOJMODELBRIDGE_IMPORT_NOTES.md"
    instructions.write_text(
        f"# MojModelBridge MCreator export\n\n"
        f"Open `{workspace_file.name}` in MCreator. This export targets `{settings.get('currentGenerator')}`.\n\n"
        "The generated living entities are intentionally conservative: automatic spawning is disabled, and unsupported custom AI, drops, sounds, renderer layers, and entity-state logic are listed in `mcreator_export_report.html`.\n\n"
        "Models are in `models/mojmap-1.20.x`, sampled animation definitions are in `models/animations`, and entity/item/block/armor textures are copied into the workspace resource tree when they exist in the input archive. Missing textures receive checker placeholders with the intended filename so MCreator can load the element; replace those PNGs with the originals.\n\n"
        "Items, tools/weapons, armor sets, blocks, dimensions, biomes, plants, and projectiles are exported with conservative defaults where exact legacy behavior cannot be recovered. Check the HTML/JSON report for manual reconstruction warnings.\n\n"
        "This workspace contains exactly one canonical `.mcreator` definition. Open that file directly in MCreator; do not import individual `.mod.json` element files as a workspace.\n",
        encoding="utf-8",
    )
    _log(log, f"MCreator workspace written: {workspace_file}")
    _log(log, f"MCreator export report: {report_html}")
    return MCreatorExportResult(output_dir, workspace_file, exports, global_warnings, report_json, report_html, content_result.exports)
