from __future__ import annotations

import json
import math
import os
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Optional

from legacy_bridge import (
    _method_info,
    _parse_constant,
    _parse_methods,
    _run_javap,
    prime_javap_cache,
)


@dataclass
class ConstructedObject:
    class_name: str
    args: list[Any] = field(default_factory=list)


@dataclass
class RendererRegistration:
    entity_class: str
    renderer_class: str
    model_class: str = ""
    model_args: list[Any] = field(default_factory=list)
    renderer_args: list[Any] = field(default_factory=list)
    shadow_size: float = 0.5
    visual_scale: float = 1.0
    texture_refs: list[str] = field(default_factory=list)
    source_class: str = ""
    bytecode_offset: int = 0


@dataclass
class EntityMetadata:
    class_name: str
    display_name: str
    registry_name: str
    superclass: str = ""
    width: float = 0.6
    height: float = 1.8
    health: float = 10.0
    movement_speed: float = 0.3
    attack_damage: float = 3.0
    armor: float = 0.0
    follow_range: float = 16.0
    xp: int = 0
    fire_immune: bool = False
    water_mob: bool = False
    flying_mob: bool = False
    ai_classes: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


@dataclass
class LegacyModGraph:
    root: Path
    source_roots: list[Path] = field(default_factory=list)
    registrations: list[RendererRegistration] = field(default_factory=list)
    entities: dict[str, EntityMetadata] = field(default_factory=dict)
    registry_names: dict[str, str] = field(default_factory=dict)
    class_files: dict[str, Path] = field(default_factory=dict)
    java_files: dict[str, Path] = field(default_factory=dict)
    warnings: list[str] = field(default_factory=list)

    @property
    def roots(self) -> list[Path]:
        roots = [Path(self.root)] + [Path(path) for path in self.source_roots]
        seen: set[str] = set()
        result: list[Path] = []
        for path in roots:
            resolved = path.expanduser().resolve()
            key = str(resolved).lower()
            if key not in seen:
                seen.add(key)
                result.append(resolved)
        return result

    @property
    def model_constructor_args(self) -> dict[str, list[Any]]:
        result: dict[str, list[Any]] = {}
        for reg in self.registrations:
            if reg.model_class and reg.model_args and reg.model_class not in result:
                result[reg.model_class] = list(reg.model_args)
        return result

    @property
    def registrations_by_model(self) -> dict[str, list[RendererRegistration]]:
        result: dict[str, list[RendererRegistration]] = {}
        for reg in self.registrations:
            if reg.model_class:
                result.setdefault(reg.model_class, []).append(reg)
        return result

    def to_dict(self) -> dict[str, Any]:
        return {
            "root": str(self.root),
            "source_roots": [str(path) for path in self.roots],
            "class_count": len(self.class_files),
            "class_files": {name: str(path) for name, path in self.class_files.items()},
            "java_count": len(self.java_files),
            "java_files": {name: str(path) for name, path in self.java_files.items()},
            "renderer_registration_count": len(self.registrations),
            "model_constructor_args": self.model_constructor_args,
            "registrations": [
                {
                    "entity_class": r.entity_class,
                    "renderer_class": r.renderer_class,
                    "model_class": r.model_class,
                    "model_args": r.model_args,
                    "renderer_args": _jsonable(r.renderer_args),
                    "shadow_size": r.shadow_size,
                    "visual_scale": r.visual_scale,
                    "texture_refs": r.texture_refs,
                    "source_class": r.source_class,
                    "bytecode_offset": r.bytecode_offset,
                }
                for r in self.registrations
            ],
            "entities": {name: vars(meta) for name, meta in self.entities.items()},
            "registry_names": dict(self.registry_names),
            "warnings": list(self.warnings),
        }


def _jsonable(value: Any) -> Any:
    if isinstance(value, ConstructedObject):
        return {"class": value.class_name, "args": [_jsonable(v) for v in value.args]}
    if isinstance(value, list):
        return [_jsonable(v) for v in value]
    if isinstance(value, tuple):
        return [_jsonable(v) for v in value]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    return repr(value)


def _simple(name: str) -> str:
    return name.replace("/", ".").split(".")[-1].split("$")[0]


def _class_from_comment(comment: str) -> str:
    match = re.search(r"\bclass\s+([A-Za-z0-9_/$]+)", comment)
    return match.group(1).replace("/", ".") if match else ""


def _string_from_comment(comment: str) -> Optional[str]:
    match = re.search(r"\bString\s+(.*)$", comment)
    return match.group(1) if match else None


def _descriptor_args(desc: str) -> list[str]:
    if "(" not in desc or ")" not in desc:
        return []
    raw = desc.split("(", 1)[1].split(")", 1)[0]
    args: list[str] = []
    i = 0
    while i < len(raw):
        start = i
        while i < len(raw) and raw[i] == "[":
            i += 1
        if i >= len(raw):
            break
        if raw[i] == "L":
            end = raw.find(";", i)
            if end < 0:
                break
            args.append(raw[start : end + 1])
            i = end + 1
        else:
            args.append(raw[start : i + 1])
            i += 1
    return args


def _push_constant(op: str, comment: str) -> Any:
    value = _parse_constant(op, comment)
    if value is not None:
        return value
    string = _string_from_comment(comment)
    if string is not None:
        return string
    if comment.startswith("class "):
        return ("class", _class_from_comment(comment))
    return None


def _parse_renderer_registration_method(method: Any, source_name: str) -> list[RendererRegistration]:
    stack: list[Any] = []
    registrations: list[RendererRegistration] = []

    def pop() -> Any:
        return stack.pop() if stack else None

    for offset, op, comment in method.instructions:
        base = op.split()[0]
        if base == "new":
            class_name = _class_from_comment(comment)
            stack.append(ConstructedObject(class_name))
            continue
        if base == "dup":
            if stack:
                stack.append(stack[-1])
            continue
        if base in {"pop", "pop2"}:
            pop()
            continue
        if base.startswith("ldc") or base in {
            "fconst_0", "fconst_1", "fconst_2", "dconst_0", "dconst_1",
            "iconst_m1", "iconst_0", "iconst_1", "iconst_2", "iconst_3",
            "iconst_4", "iconst_5", "bipush", "sipush",
        }:
            value = _push_constant(op, comment)
            if value is not None:
                stack.append(value)
            continue
        if base == "aconst_null":
            stack.append(None)
            continue
        if not base.startswith("invoke"):
            continue

        info = _method_info(comment)
        if not info:
            continue
        owner, name, desc = info
        argc = len(_descriptor_args(desc))

        if name == "<init>":
            args = [pop() for _ in range(argc)][::-1]
            receiver = pop()
            class_name = owner.replace("/", ".")
            instance = ConstructedObject(class_name, args)
            # "new; dup; ...; invokespecial" leaves the original duplicated
            # reference on the stack. Replace it with the initialized object.
            if stack and isinstance(stack[-1], ConstructedObject):
                if _simple(stack[-1].class_name) == _simple(class_name):
                    stack[-1] = instance
                else:
                    stack.append(instance)
            else:
                stack.append(instance)
            continue

        if name == "registerEntityRenderingHandler":
            args = [pop() for _ in range(argc)][::-1]
            if len(args) < 2:
                continue
            entity_ref, renderer = args[0], args[1]
            if not (isinstance(entity_ref, tuple) and entity_ref and entity_ref[0] == "class"):
                continue
            if not isinstance(renderer, ConstructedObject):
                continue
            model = next(
                (
                    arg
                    for arg in renderer.args
                    if isinstance(arg, ConstructedObject)
                    and _simple(arg.class_name).startswith("Model")
                ),
                None,
            )
            numeric = [float(v) for v in renderer.args if isinstance(v, (int, float))]
            shadow = numeric[0] if numeric else 0.5
            visual_scale = numeric[-1] if len(numeric) >= 2 else 1.0
            registrations.append(
                RendererRegistration(
                    entity_class=_simple(entity_ref[1]),
                    renderer_class=_simple(renderer.class_name),
                    model_class=_simple(model.class_name) if model else "",
                    model_args=list(model.args) if model else [],
                    renderer_args=list(renderer.args),
                    shadow_size=shadow,
                    visual_scale=visual_scale,
                    source_class=source_name,
                    bytecode_offset=offset,
                )
            )
            continue

        # Keep the stack conservative for unrelated calls.
        for _ in range(argc):
            pop()
        if base not in {"invokestatic", "invokedynamic"}:
            pop()

    return registrations


def _png_refs_from_class(path: Path) -> list[str]:
    try:
        data = path.read_bytes()
    except OSError:
        return []
    text = data.decode("latin-1", errors="ignore")
    refs = set()
    for match in re.finditer(r"(?i)[A-Za-z0-9_./\\-]{1,240}\.png", text):
        value = match.group(0).replace("\\", "/").strip("/ ")
        # Constant-pool parsing may include a class descriptor prefix. Keep the
        # printable PNG tail, which is what ResourceLocation uses.
        value = re.sub(r"^[^A-Za-z0-9]+", "", value)
        refs.add(value)
    return sorted(refs)


def _parse_registry_names(class_files: dict[str, Path]) -> dict[str, str]:
    result: dict[str, str] = {}
    candidates = []
    for name, path in class_files.items():
        try:
            data = path.read_bytes()
        except OSError:
            continue
        if b"registerModEntity" in data or b"registerGlobalEntityID" in data:
            candidates.append(path)
    for path in candidates:
        try:
            methods = _parse_methods(_run_javap(path))
        except Exception:
            continue
        for method in methods:
            recent_class = ""
            recent_string = ""
            for _offset, op, comment in method.instructions:
                if comment.startswith("class "):
                    recent_class = _simple(_class_from_comment(comment))
                string = _string_from_comment(comment)
                if string is not None:
                    recent_string = string
                info = _method_info(comment) if op.split()[0].startswith("invoke") else None
                if info and info[1] in {"registerModEntity", "registerGlobalEntityID"}:
                    if recent_class and recent_string:
                        result.setdefault(recent_class, recent_string)
    return result


def _display_name(class_name: str) -> str:
    name = re.sub(r"^(Entity|Mob)", "", class_name)
    name = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", name)
    name = re.sub(r"[_$]+", " ", name)
    return re.sub(r"\s+", " ", name).strip() or class_name


def _registry_name(value: str) -> str:
    value = re.sub(r"^(Entity|Mob)", "", value)
    value = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", value)
    value = re.sub(r"[^A-Za-z0-9]+", "_", value).strip("_")
    return value.lower() or "converted_entity"


_ATTR_FIELDS = {
    "field_111267_a": "health",
    "maxHealth": "health",
    "field_111263_d": "movement_speed",
    "movementSpeed": "movement_speed",
    "field_111264_e": "attack_damage",
    "attackDamage": "attack_damage",
    "field_111265_b": "follow_range",
    "followRange": "follow_range",
    "field_111266_c": "knockback_resistance",
}


def _parse_entity_metadata(path: Path, registry_hint: str = "") -> EntityMetadata:
    text = _run_javap(path)
    class_match = re.search(r"(?m)^public (?:abstract )?class\s+([^\s]+)\s+extends\s+([^\s{]+)", text)
    if not class_match:
        class_match = re.search(r"(?m)^public (?:abstract )?class\s+([^\s{]+)", text)
    class_name = _simple(class_match.group(1)) if class_match else path.stem
    superclass = _simple(class_match.group(2)) if class_match and class_match.lastindex and class_match.lastindex >= 2 else ""
    meta = EntityMetadata(
        class_name=class_name,
        display_name=_display_name(registry_hint or class_name),
        registry_name=_registry_name(registry_hint or class_name),
        superclass=superclass,
    )
    lower_super = superclass.lower()
    meta.water_mob = any(token in lower_super for token in ("watermob", "squid", "fish"))
    meta.flying_mob = any(token in lower_super for token in ("flying", "ambient", "bat"))
    meta.fire_immune = "field_70178_ae" in text

    methods = _parse_methods(text)
    # Size: use the largest branch as the default for giant PlayNicely-style mobs.
    size_values: list[tuple[float, float]] = []
    ai_classes: list[str] = []
    found_attributes: set[str] = set()
    for method in methods:
        stack: list[Any] = []
        current_attr: Optional[str] = None

        def pop() -> Any:
            return stack.pop() if stack else None

        for _offset, op, comment in method.instructions:
            base = op.split()[0]
            if base.startswith("ldc") or base in {
                "fconst_0", "fconst_1", "fconst_2", "dconst_0", "dconst_1",
                "iconst_m1", "iconst_0", "iconst_1", "iconst_2", "iconst_3",
                "iconst_4", "iconst_5", "bipush", "sipush",
            }:
                value = _parse_constant(op, comment)
                if value is not None:
                    stack.append(value)
                continue
            if base.startswith(("aload", "fload", "dload", "iload")):
                stack.append(None)
                continue
            if base == "new":
                cls = _simple(_class_from_comment(comment))
                if cls.startswith("EntityAI"):
                    ai_classes.append(cls)
                stack.append(ConstructedObject(cls))
                continue
            if base == "getstatic":
                field = re.search(r"Field\s+.+?\.?([A-Za-z0-9_$]+):", comment)
                if field:
                    current_attr = _ATTR_FIELDS.get(field.group(1), current_attr)
                stack.append(None)
                continue
            if not base.startswith("invoke"):
                continue
            info = _method_info(comment)
            if not info:
                continue
            owner, name, desc = info
            argc = len(_descriptor_args(desc))
            args = [pop() for _ in range(argc)][::-1]
            if base not in {"invokestatic", "invokedynamic"}:
                pop()
            if name in {"func_70105_a", "setSize"} and len(args) >= 2:
                if all(isinstance(v, (int, float)) for v in args[-2:]):
                    size_values.append((float(args[-2]), float(args[-1])))
            elif name in {"func_111128_a", "setBaseValue"} and args:
                value = args[-1]
                if current_attr and isinstance(value, (int, float)):
                    setattr(meta, current_attr, float(value))
                    found_attributes.add(current_attr)
                current_attr = None

    if size_values:
        meta.width, meta.height = max(size_values, key=lambda pair: pair[0] * pair[1])
    meta.ai_classes = sorted(set(ai_classes))
    for attribute, label in (
        ("health", "maximum health"),
        ("attack_damage", "attack damage"),
        ("movement_speed", "movement speed"),
    ):
        if attribute not in found_attributes:
            meta.warnings.append(
                f"{label.capitalize()} was not assigned directly in this entity class; the MCreator value is a safe fallback and may come from a superclass or custom stats helper."
            )

    # XP reward is frequently assigned directly to field_70728_aV.
    # Read the preceding constant instruction without a whole-file wildcard regex.
    for method in methods:
        previous_value: Any = None
        for _offset, op, comment in method.instructions:
            base = op.split()[0]
            if base.startswith("ldc") or base in {
                "iconst_m1", "iconst_0", "iconst_1", "iconst_2", "iconst_3",
                "iconst_4", "iconst_5", "bipush", "sipush",
            }:
                value = _parse_constant(op, comment)
                if isinstance(value, (int, float)):
                    previous_value = value
            elif base == "putfield" and "field_70728_aV:I" in comment and isinstance(previous_value, (int, float)):
                meta.xp = int(previous_value)
    return meta



def _split_java_arguments(raw: str) -> list[str]:
    args: list[str] = []
    start = 0
    depth = 0
    quote = ""
    escaped = False
    for index, char in enumerate(raw):
        if quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = ""
            continue
        if char in {'"', "'"}:
            quote = char
        elif char in "([{<":
            depth += 1
        elif char in ")]}>":
            depth = max(0, depth - 1)
        elif char == "," and depth == 0:
            args.append(raw[start:index].strip())
            start = index + 1
    args.append(raw[start:].strip())
    return args


def _iter_java_calls(text: str, names: set[str] | None = None) -> Iterable[tuple[str, str, int]]:
    """Yield balanced Java call expressions as ``(name, arguments, offset)``.

    The parser is deliberately small but handles nested constructors/lambdas and
    quoted strings, which is enough for registry and renderer bootstrap code in
    1.8-1.12 mods.  It does not attempt to parse arbitrary Java syntax.
    """
    pattern = re.compile(r"\b([A-Za-z_$][A-Za-z0-9_$]*)\s*\(")
    for match in pattern.finditer(text):
        name = match.group(1)
        if names is not None and name not in names:
            continue
        open_index = text.find("(", match.start())
        depth = 0
        quote = ""
        escaped = False
        for index in range(open_index, len(text)):
            char = text[index]
            if quote:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == quote:
                    quote = ""
                continue
            if char in {'"', "'"}:
                quote = char
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
                if depth == 0:
                    yield name, text[open_index + 1:index], match.start()
                    break


def _java_scalar(value: str) -> Any:
    value = value.strip()
    if re.fullmatch(r"[-+]?\d+", value):
        try:
            return int(value)
        except ValueError:
            return value
    if re.fullmatch(r"[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?[fFdD]?", value):
        try:
            return float(value.rstrip("fFdD"))
        except ValueError:
            return value
    if value in {"true", "false"}:
        return value == "true"
    string = re.fullmatch(r'"((?:\\.|[^"\\])*)"', value, re.DOTALL)
    if string:
        try:
            return json.loads(value)
        except Exception:
            return string.group(1)
    return value


def _source_java_index(root: Path) -> dict[str, list[Path]]:
    result: dict[str, list[Path]] = {}
    for path in root.rglob("*.java"):
        result.setdefault(_simple(path.stem), []).append(path)
    return result


def _source_modern_entity_registry_names(java_files: Iterable[Path]) -> dict[str, str]:
    """Recover 1.20.x RegistryObject<EntityType<...>> -> registry names."""
    result: dict[str, str] = {}
    for path in java_files:
        try:
            text = path.read_text(encoding="utf-8-sig", errors="ignore")
        except OSError:
            continue
        for match in re.finditer(
            r"RegistryObject\s*<\s*EntityType\s*<\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*>\s*>\s*"
            r"[A-Za-z_$][A-Za-z0-9_$]*\s*=\s*[^;]{0,1200}?\.register\s*\(\s*\"([^\"]+)\"",
            text,
            re.DOTALL,
        ):
            entity, registry = _simple(match.group(1)), match.group(2)
            clean = _registry_name(registry)
            if clean:
                result.setdefault(entity, clean)
    return result

def _source_entity_registry_names(java_files: Iterable[Path]) -> dict[str, str]:
    """Recover registry paths from common 1.8-1.12 helper-based registrations.

    Registration arrays can contain thousands of calls on a single source line.
    Two linear regex passes are intentionally used instead of recursively walking
    every nested call expression.
    """
    result: dict[str, str] = {}
    class_first = re.compile(
        r"\b([A-Za-z_$][A-Za-z0-9_$]*(?:Entry|Entity|Projectile|Mob|Eggless|Register)[A-Za-z0-9_$]*)\s*"
        r"\(\s*([A-Za-z_$][A-Za-z0-9_$.]*)\.class\s*,\s*\"([^\"]+)\"",
        re.IGNORECASE,
    )
    string_first = re.compile(
        r"\b([A-Za-z_$][A-Za-z0-9_$]*(?:Entry|Entity|Projectile|Mob|Eggless|Register)[A-Za-z0-9_$]*)\s*"
        r"\(\s*\"([^\"]+)\"\s*,\s*([A-Za-z_$][A-Za-z0-9_$.]*)\.class",
        re.IGNORECASE,
    )
    for path in java_files:
        try:
            text = path.read_text(encoding="utf-8-sig", errors="ignore")
        except OSError:
            continue
        if ".class" not in text or not (
            "EntityEntryBuilder" in text
            or "registerModEntity" in text
            or "registerGlobalEntityID" in text
        ):
            continue
        for match in class_first.finditer(text):
            entity, registry = _simple(match.group(2)), match.group(3)
            if registry and not registry.lower().endswith((".png", ".json")) and "/" not in registry:
                result.setdefault(entity, registry)
        for match in string_first.finditer(text):
            registry, entity = match.group(2), _simple(match.group(3))
            if registry and not registry.lower().endswith((".png", ".json")) and "/" not in registry:
                result.setdefault(entity, registry)
    return result

def _renderer_model_from_source(renderer: str, java_index: dict[str, list[Path]]) -> tuple[str, list[Any], float, float, list[str]]:
    paths = java_index.get(renderer, [])
    if not paths:
        return "", [], 0.5, 1.0, []
    # Prefer files under render/renderer packages over unrelated same-name files.
    path = sorted(paths, key=lambda p: (0 if "render" in p.as_posix().lower() else 1, len(p.parts), str(p)))[0]
    try:
        text = path.read_text(encoding="utf-8-sig", errors="ignore")
    except OSError:
        return "", [], 0.5, 1.0, []
    texture_refs = sorted(set(re.findall(r'(?i)[A-Za-z0-9_./\\-]{1,240}\.png', text)))
    model_match = re.search(r"\bnew\s+([A-Za-z_$][A-Za-z0-9_$]*Model[A-Za-z0-9_$]*)\s*\((.*?)\)", text, re.DOTALL)
    if not model_match:
        # Most model names begin with Model, but some mods use CreatureModel.
        model_match = re.search(r"\bnew\s+(Model[A-Za-z0-9_$]+)\s*\((.*?)\)", text, re.DOTALL)
    model_class = _simple(model_match.group(1)) if model_match else ""
    model_args = [_java_scalar(value) for value in _split_java_arguments(model_match.group(2))] if model_match else []
    shadow = 0.5
    if model_match:
        tail = text[model_match.end(): model_match.end() + 160]
        number = re.search(r",\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+))[fFdD]?\s*\)", tail)
        if number:
            shadow = float(number.group(1))
    visual_scale = 1.0
    scale_match = re.search(r"(?:GlStateManager\.[A-Za-z0-9_$]+|glScalef|scale)\s*\(\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+))[fFdD]?\s*,\s*\1[fFdD]?\s*,\s*\1[fFdD]?", text)
    if scale_match:
        visual_scale = float(scale_match.group(1))
    return model_class, model_args, shadow, visual_scale, texture_refs


def _entity_source_texture_refs(entity: str, java_index: dict[str, list[Path]]) -> list[str]:
    """Recover entity-specific texture ResourceLocations, including inherited variant selectors."""
    all_sources=[]
    direct_texts=[]
    for paths in java_index.values():
        for path in paths:
            if path.suffix.lower() != '.java': continue
            try: txt=path.read_text(encoding='utf-8-sig', errors='ignore')
            except OSError: continue
            all_sources.append(txt)
            if path.stem == entity: direct_texts.append(txt)
    def const_map(text):
        consts={}
        for m in re.finditer(r'ResourceLocation\s+([A-Za-z0-9_$]+)\s*=\s*ResourceLocation\.(?:fromNamespaceAndPath|tryBuild)\s*\(\s*"[^"]+"\s*,\s*"([^"]+)"\s*\)', text, re.S):
            consts[m.group(1)] = m.group(2)
        for m in re.finditer(r'ResourceLocation\s+([A-Za-z0-9_$]+)\s*=\s*new\s+ResourceLocation\s*\(\s*"([^"]+)"\s*\)', text, re.S):
            consts[m.group(1)] = m.group(2)
        return consts
    selected=set()
    # Exact variant selector, e.g. if ("EntityRedAnt".equals(n)) return TEXTURE2.
    for text in all_sources:
        consts=const_map(text)
        for m in re.finditer(r'if\s*\(\s*"' + re.escape(entity) + r'"\s*\.equals\([^)]*\)\s*\)\s*\{?\s*return\s+([A-Za-z0-9_$]+)', text, re.S):
            if m.group(1) in consts: selected.add(consts[m.group(1)])
    if selected:
        return sorted(selected)
    # Entity's own texture constants/direct resource references.
    direct=set()
    for text in direct_texts:
        consts=const_map(text)
        # Default/fallback return inside getTexture method.
        m=re.search(r'getTexture\s*\([^)]*\)\s*\{(.*?)\n\s*\}', text, re.S)
        if m:
            body=m.group(1)
            returns=re.findall(r'\breturn\s+([A-Za-z0-9_$]+)\s*;',body)
            if returns:
                value=consts.get(returns[-1])
                if value: direct.add(value)
        direct.update(re.findall(r'(?i)textures/(?:entity|entities)/[^"\s]+?\.png', text))
    return sorted(direct)

def _source_renderer_registrations(root: Path, java_index: dict[str, list[Path]]) -> list[RendererRegistration]:
    results: list[RendererRegistration] = []
    renderer_info_cache: dict[str, tuple[str, list[Any], float, float, list[str]]] = {}
    pattern = re.compile(
        r"\b(?:registerEntityRenderingHandler|registerRender|registerRenderer|registerEntityRenderer)\s*\(\s*"
        r"(?:(?P<classref>[A-Za-z_$][A-Za-z0-9_$.]*)\.class|(?P<fieldref>[A-Za-z_$][A-Za-z0-9_$.]*)\.get\(\))\s*,\s*"
        r"(?:(?:\([^)]*\)|[A-Za-z_$][A-Za-z0-9_$]*)\s*->\s*new\s+(?P<lambda>[A-Za-z_$][A-Za-z0-9_$.]*)\s*\("
        r"|(?P<methodref>[A-Za-z_$][A-Za-z0-9_$]*)\s*::\s*new)",
        re.DOTALL,
    )
    field_to_entity: dict[str, str] = {}
    all_java_paths = [path for paths in java_index.values() for path in paths]
    for path in all_java_paths:
        try:
            text = path.read_text(encoding="utf-8-sig", errors="ignore")
        except OSError:
            continue
        for match in re.finditer(
            r"RegistryObject\s*<\s*EntityType\s*<\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*>\s*>\s*"
            r"([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*[^;]{0,1200}?\.register\s*\(",
            text,
            re.DOTALL,
        ):
            field_to_entity[match.group(2)] = _simple(match.group(1))
        for match in re.finditer(
            r"(?:static\s+final\s+)?EntityType\s*<\s*([A-Za-z_$][A-Za-z0-9_$.]*)\s*>\s*"
            r"([A-Za-z_$][A-Za-z0-9_$]*)\s*=",
            text,
        ):
            field_to_entity.setdefault(match.group(2), _simple(match.group(1)))

    accepted_tokens = ("registerEntityRenderingHandler", "registerRender", "registerRenderer", "registerEntityRenderer")
    for paths in java_index.values():
        for path in paths:
            try:
                text = path.read_text(encoding="utf-8-sig", errors="ignore")
            except OSError:
                continue
            if not any(token in text for token in accepted_tokens):
                continue
            for match in pattern.finditer(text):
                classref = match.group("classref") or ""
                fieldref = match.group("fieldref") or ""
                entity = _simple(classref) if classref else field_to_entity.get(_simple(fieldref), "")
                renderer = _simple(match.group("lambda") or match.group("methodref") or "")
                if not entity or not renderer:
                    continue
                info = renderer_info_cache.get(renderer)
                if info is None:
                    info = _renderer_model_from_source(renderer, java_index)
                    renderer_info_cache[renderer] = info
                model_class, model_args, shadow, visual_scale, texture_refs = info
                source_specific_refs = _entity_source_texture_refs(entity, java_index)
                texture_refs = sorted(set(texture_refs + source_specific_refs))
                statement_end = text.find(";", match.end())
                # Clean Mojmap registrations commonly pass shadow and renderer scale as
                # the final numeric arguments: new RenderAnt(ctx, new ModelAnt(), 0.1f, 0.25f).
                # The renderer class itself stores the scale field, so recover these constants
                # from the registration expression rather than defaulting to 1.0.
                tail_expr = text[match.start(): statement_end if statement_end >= 0 else match.end() + 400]
                ctor = re.search(r'new\s+' + re.escape(renderer) + r'\s*\(.*?,\s*(-?(?:\d+(?:\.\d*)?|\.\d+))[fFdD]?\s*,\s*(-?(?:\d+(?:\.\d*)?|\.\d+))[fFdD]?\s*\)', tail_expr, re.S)
                if ctor:
                    try: shadow=float(ctor.group(1)); visual_scale=float(ctor.group(2))
                    except ValueError: pass
                expression = text[match.start(): statement_end if statement_end >= 0 else match.end() + 300]
                direct_refs = re.findall(r'(?i)[A-Za-z0-9_./\-]{1,240}\.png', expression)
                if not model_class:
                    inline_model = re.search(r"\bnew\s+(Model[A-Za-z0-9_$]+)\s*\(", expression)
                    if inline_model:
                        model_class = _simple(inline_model.group(1))
                        inline_args = re.search(r"\bnew\s+" + re.escape(inline_model.group(1)) + r"\s*\(([^)]*)\)", expression, re.DOTALL)
                        if inline_args:
                            model_args = [_java_scalar(value) for value in _split_java_arguments(inline_args.group(1))]
                results.append(RendererRegistration(
                    entity_class=entity,
                    renderer_class=renderer,
                    model_class=model_class,
                    model_args=model_args,
                    renderer_args=[expression.strip()],
                    shadow_size=shadow,
                    visual_scale=visual_scale,
                    texture_refs=sorted(set(texture_refs + direct_refs)),
                    source_class=path.name,
                    bytecode_offset=match.start(),
                ))
    return results

def load_legacy_mod_graph(path: Path, expected_root: Optional[Path] = None) -> LegacyModGraph:
    """Load a previously scanned graph for resumable large-mod conversions.

    Older cache files did not contain ``class_files``; those are reconstructed
    with a cheap filesystem walk while preserving the expensive renderer/entity
    source analysis from the cache.
    """
    data = json.loads(Path(path).read_text(encoding="utf-8-sig"))
    if not isinstance(data, dict):
        raise ValueError("Legacy graph cache is not a JSON object")
    root = Path(str(data.get("root") or expected_root or "")).expanduser().resolve()
    if expected_root is not None and root != Path(expected_root).expanduser().resolve():
        raise ValueError(f"Legacy graph cache root mismatch: {root} != {Path(expected_root).resolve()}")

    class_files: dict[str, Path] = {}
    cached_files = data.get("class_files")
    if isinstance(cached_files, dict):
        for name, raw in cached_files.items():
            candidate = Path(str(raw))
            if candidate.exists():
                class_files[str(name)] = candidate.resolve()
    if not class_files and root.exists():
        for candidate in sorted(root.rglob("*.class")):
            simple = _simple(candidate.stem)
            current = class_files.get(simple)
            if current is None or ("$" in current.stem and "$" not in candidate.stem):
                class_files[simple] = candidate.resolve()

    source_roots: list[Path] = []
    for raw_root in data.get("source_roots", []) or []:
        candidate = Path(str(raw_root)).expanduser()
        if candidate.exists():
            source_roots.append(candidate.resolve())
    if not source_roots:
        source_roots = [root]

    java_files: dict[str, Path] = {}
    cached_java = data.get("java_files")
    if isinstance(cached_java, dict):
        for name, raw in cached_java.items():
            candidate = Path(str(raw))
            if candidate.exists():
                java_files[str(name)] = candidate.resolve()
    if not java_files:
        for source_root in source_roots:
            for candidate in sorted(source_root.rglob("*.java")):
                simple = _simple(candidate.stem)
                java_files.setdefault(simple, candidate.resolve())

    graph = LegacyModGraph(root=root, source_roots=source_roots, class_files=class_files, java_files=java_files)
    for raw in data.get("registrations", []):
        if not isinstance(raw, dict):
            continue
        graph.registrations.append(RendererRegistration(
            entity_class=str(raw.get("entity_class", "")),
            renderer_class=str(raw.get("renderer_class", "")),
            model_class=str(raw.get("model_class", "")),
            model_args=list(raw.get("model_args", []) or []),
            renderer_args=list(raw.get("renderer_args", []) or []),
            shadow_size=float(raw.get("shadow_size", 0.5) or 0.5),
            visual_scale=float(raw.get("visual_scale", 1.0) or 1.0),
            texture_refs=list(raw.get("texture_refs", []) or []),
            source_class=str(raw.get("source_class", "")),
            bytecode_offset=int(raw.get("bytecode_offset", 0) or 0),
        ))
    entities = data.get("entities", {})
    if isinstance(entities, dict):
        for name, raw in entities.items():
            if not isinstance(raw, dict):
                continue
            fields = {key: value for key, value in raw.items() if key in EntityMetadata.__dataclass_fields__}
            fields.setdefault("class_name", str(name))
            fields.setdefault("display_name", str(name))
            fields.setdefault("registry_name", _simple(str(name)).lower())
            graph.entities[str(name)] = EntityMetadata(**fields)
    registry_names = data.get("registry_names", {})
    if isinstance(registry_names, dict):
        graph.registry_names = {str(k): str(v) for k, v in registry_names.items()}
    graph.warnings = [str(value) for value in data.get("warnings", []) or []]
    return graph


def scan_legacy_mod(root: Path, log=None, source_roots: Optional[Iterable[Path]] = None) -> LegacyModGraph:
    root = Path(root).expanduser().resolve()
    roots: list[Path] = []
    for candidate in [root, *(source_roots or [])]:
        resolved = Path(candidate).expanduser().resolve()
        if resolved not in roots:
            roots.append(resolved)
    class_paths = sorted({path.resolve() for source_root in roots for path in source_root.rglob("*.class")})
    # Prefer outer classes over compiler-generated inner/anonymous classes when two files
    # collapse to the same simple name (e.g. RenderBattleAxe and RenderBattleAxe$1).
    class_files: dict[str, Path] = {}
    for path in class_paths:
        simple = _simple(path.stem)
        current = class_files.get(simple)
        if current is None:
            class_files[simple] = path
            continue
        current_inner = "$" in current.stem
        candidate_inner = "$" in path.stem
        if current_inner and not candidate_inner:
            class_files[simple] = path
        elif current_inner == candidate_inner and len(path.stem) < len(current.stem):
            class_files[simple] = path
    java_files: dict[str, Path] = {}
    for source_root in roots:
        for path in sorted(source_root.rglob("*.java")):
            simple = _simple(path.stem)
            java_files.setdefault(simple, path.resolve())
    graph = LegacyModGraph(root=root, source_roots=roots, class_files=class_files, java_files=java_files)
    if log:
        log(f"Legacy mod graph: found {len(class_paths)} class files")

    proxy_candidates: list[Path] = []
    for path in class_paths:
        try:
            data = path.read_bytes()
        except OSError:
            continue
        if b"registerEntityRenderingHandler" in data:
            proxy_candidates.append(path)

    prime_javap_cache(proxy_candidates)
    for proxy in proxy_candidates:
        try:
            methods = _parse_methods(_run_javap(proxy))
        except Exception as exc:
            graph.warnings.append(f"Could not inspect renderer registry {proxy.name}: {exc}")
            continue
        for method in methods:
            regs = _parse_renderer_registration_method(method, proxy.name)
            graph.registrations.extend(regs)

    # 1.8-1.12 commonly moves registrations into RegistryEvent helpers and uses
    # lambda/method-reference renderer factories.  Decompiled source is the most
    # reliable representation of those relationships; merge it with bytecode
    # results when available rather than requiring either representation alone.
    java_index: dict[str, list[Path]] = {}
    for source_root in roots:
        for name, paths in _source_java_index(source_root).items():
            java_index.setdefault(name, []).extend(path for path in paths if path not in java_index.get(name, []))
    if java_index:
        try:
            source_regs: list[RendererRegistration] = []
            for source_root in roots:
                source_regs.extend(_source_renderer_registrations(source_root, java_index))
            graph.registrations.extend(source_regs)
            if log and source_regs:
                log(f"Legacy mod graph: recovered {len(source_regs)} renderer links from Java registry sources")
        except Exception as exc:
            graph.warnings.append(f"Java renderer registry scan failed: {exc}")
        try:
            all_java_paths = [path for paths in java_index.values() for path in paths]
            source_registry_names = _source_entity_registry_names(all_java_paths)
            source_registry_names.update(_source_modern_entity_registry_names(all_java_paths))
        except Exception as exc:
            source_registry_names = {}
            graph.warnings.append(f"Java entity registry scan failed: {exc}")
    else:
        source_registry_names = {}

    # Batch-inspect only the classes needed by the recovered graph.
    relevant_paths: list[Path] = []
    for reg in graph.registrations:
        for class_name in (reg.entity_class, reg.renderer_class, reg.model_class):
            path = class_files.get(class_name)
            if path is not None:
                relevant_paths.append(path)
    prime_javap_cache(relevant_paths)

    # Attach exact PNG names from each renderer class without discarding
    # source-level ResourceLocations already recovered from lambda factories.
    for reg in graph.registrations:
        renderer_path = class_files.get(reg.renderer_class)
        if renderer_path:
            reg.texture_refs = sorted(set(reg.texture_refs + _png_refs_from_class(renderer_path)))

    graph.registry_names = _parse_registry_names(class_files)
    graph.registry_names.update(source_registry_names)
    for reg in graph.registrations:
        entity_path = class_files.get(reg.entity_class)
        if not entity_path:
            continue
        try:
            meta = _parse_entity_metadata(entity_path, graph.registry_names.get(reg.entity_class, ""))
        except Exception as exc:
            meta = EntityMetadata(
                class_name=reg.entity_class,
                display_name=_display_name(reg.entity_class),
                registry_name=_registry_name(graph.registry_names.get(reg.entity_class, reg.entity_class)),
                warnings=[f"Entity metadata parser failed: {exc}"],
            )
        graph.entities[reg.entity_class] = meta

    # De-duplicate exact registrations that bridge methods may expose twice.
    unique: dict[tuple[str, str, str, str], RendererRegistration] = {}
    for reg in graph.registrations:
        key = (
            reg.entity_class,
            reg.renderer_class,
            reg.model_class,
            json.dumps(_jsonable(reg.renderer_args), sort_keys=True),
        )
        unique.setdefault(key, reg)
    graph.registrations = list(unique.values())
    graph.registrations.sort(key=lambda r: (r.entity_class.lower(), r.renderer_class.lower()))

    if log:
        log(
            f"Legacy mod graph complete: {len(graph.registrations)} renderer links, "
            f"{len(graph.registrations_by_model)} model classes, {len(graph.entities)} entity metadata records"
        )
    return graph
