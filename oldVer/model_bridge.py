#!/usr/bin/env python3
"""
MojModelBridge

Converts modern Mojang-mapped Java model source and compiled legacy
ModelBase/ModelRenderer bytecode into editable Blockbench .bbmodel projects.
The shared output pipeline preserves geometry, hierarchy, textures, variants,
and recoverable procedural animation without third-party Python packages.
"""
from __future__ import annotations

import argparse
import base64
import mimetypes
import ast
import json
import math
import html
import os
import re
import sys
import traceback
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Optional

from texture_resolver import TextureIndex, TextureSetResolution, ModLayout, discover_mod_layout, infer_search_roots

VERSION = "3.4.0-beta12"
UUID_NAMESPACE = uuid.UUID("30f30b00-8b6d-4d33-85d2-a55bc1f487a9")


def _log(message: str, callback=None, level: str = "INFO") -> None:
    """Always mirror detailed conversion diagnostics to the hosting CMD window.

    The GUI may also provide a callback, but console output is intentionally
    unconditional so batch debugging remains possible when the GUI is used.
    """
    line = f"[{level}] {message}"
    print(line, flush=True)
    if callback is not None:
        try:
            callback(line)
        except Exception:
            pass


class ConversionError(RuntimeError):
    pass


@dataclass
class Cube:
    x: float
    y: float
    z: float
    dx: float
    dy: float
    dz: float
    uv_x: int = 0
    uv_y: int = 0
    mirror: bool = False
    inflate: float = 0.0
    name: str = "cube"


@dataclass
class Part:
    name: str
    variable: str
    parent_variable: str
    offset: tuple[float, float, float] = (0.0, 0.0, 0.0)
    rotation: tuple[float, float, float] = (0.0, 0.0, 0.0)
    cubes: list[Cube] = field(default_factory=list)
    parent: Optional["Part"] = None
    children: list["Part"] = field(default_factory=list)
    source_index: int = 0
    source_name: str = ""
    render_instance_index: int = 0
    render_instance_spec: dict[str, object] = field(default_factory=dict)
    origin: tuple[float, float, float] = (0.0, 24.0, 0.0)
    group_uuid: str = ""


@dataclass
class ParsedModel:
    class_name: str
    texture_width: int
    texture_height: int
    parts: list[Part]
    warnings: list[str] = field(default_factory=list)
    animation_analysis: dict[str, object] = field(default_factory=dict)
    animation_logic: str = ""
    baked_animation_json: dict[str, object] = field(default_factory=dict)
    blockbench_animations: list[dict[str, object]] = field(default_factory=list)
    animation_source: str = ""
    animation_resource: str = ""
    animation_resource_path: str = ""
    source_format: str = "modern_mojmap_java"
    primary_texture: str = ""
    texture_variants: list[str] = field(default_factory=list)
    auxiliary_textures: list[str] = field(default_factory=list)
    texture_evidence: list[str] = field(default_factory=list)
    variant_model_outputs: list[str] = field(default_factory=list)
    constructor_args: list[object] = field(default_factory=list)
    scalar_fields: dict[str, float] = field(default_factory=dict)
    model_bounds: dict[str, object] = field(default_factory=dict)
    linked_entities: list[str] = field(default_factory=list)
    linked_renderers: list[str] = field(default_factory=list)
    cached_cube_count: Optional[int] = None
    render_instances: dict[str, list[dict[str, object]]] = field(default_factory=dict)
    default_render_parts: set[str] = field(default_factory=set)
    render_runtime_rotations: list[dict[str, object]] = field(default_factory=list)

    @property
    def cube_count(self) -> int:
        if self.cached_cube_count is not None:
            return int(self.cached_cube_count)
        if self.render_instances:
            total = 0
            for part in self.parts:
                instance_count = max(1, len(self.render_instances.get(part.name, [])))
                total += len(part.cubes) * instance_count
            return total
        return sum(len(p.cubes) for p in self.parts)


# ----------------------------- Java text helpers -----------------------------

def strip_java_comments(text: str) -> str:
    """Remove // and /* */ comments while preserving strings and newlines."""
    out: list[str] = []
    i = 0
    state = "code"
    quote = ""
    while i < len(text):
        ch = text[i]
        nxt = text[i + 1] if i + 1 < len(text) else ""
        if state == "code":
            if ch in ('"', "'"):
                state = "string"
                quote = ch
                out.append(ch)
            elif ch == "/" and nxt == "/":
                state = "line_comment"
                out.extend("  ")
                i += 1
            elif ch == "/" and nxt == "*":
                state = "block_comment"
                out.extend("  ")
                i += 1
            else:
                out.append(ch)
        elif state == "string":
            out.append(ch)
            if ch == "\\" and i + 1 < len(text):
                out.append(text[i + 1])
                i += 1
            elif ch == quote:
                state = "code"
        elif state == "line_comment":
            if ch == "\n":
                out.append("\n")
                state = "code"
            else:
                out.append(" ")
        elif state == "block_comment":
            if ch == "*" and nxt == "/":
                out.extend("  ")
                i += 1
                state = "code"
            elif ch == "\n":
                out.append("\n")
            else:
                out.append(" ")
        i += 1
    return "".join(out)


def extract_balanced(text: str, open_index: int, opener: str = "(", closer: str = ")") -> tuple[str, int]:
    if open_index >= len(text) or text[open_index] != opener:
        raise ConversionError(f"Expected '{opener}' at character {open_index}")
    depth = 0
    state = "code"
    quote = ""
    i = open_index
    while i < len(text):
        ch = text[i]
        if state == "code":
            if ch in ('"', "'"):
                state = "string"
                quote = ch
            elif ch == opener:
                depth += 1
            elif ch == closer:
                depth -= 1
                if depth == 0:
                    return text[open_index + 1 : i], i + 1
        else:
            if ch == "\\" and i + 1 < len(text):
                i += 1
            elif ch == quote:
                state = "code"
        i += 1
    raise ConversionError(f"Unclosed '{opener}' beginning at character {open_index}")


def split_top_level(text: str, delimiter: str = ",") -> list[str]:
    result: list[str] = []
    start = 0
    stack: list[str] = []
    state = "code"
    quote = ""
    pairs = {"(": ")", "[": "]", "{": "}", "<": ">"}
    closing = set(pairs.values())
    i = 0
    while i < len(text):
        ch = text[i]
        if state == "code":
            if ch in ('"', "'"):
                state = "string"
                quote = ch
            elif ch in pairs:
                # Avoid treating comparison/shift operators as generic brackets.
                if ch != "<" or re.match(r"[A-Za-z_$?]", text[i + 1 : i + 2]):
                    stack.append(pairs[ch])
            elif ch in closing and stack and ch == stack[-1]:
                stack.pop()
            elif ch == delimiter and not stack:
                result.append(text[start:i].strip())
                start = i + 1
        else:
            if ch == "\\" and i + 1 < len(text):
                i += 1
            elif ch == quote:
                state = "code"
        i += 1
    result.append(text[start:].strip())
    return result


def unquote_java_string(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] == '"':
        try:
            return json.loads(value)
        except json.JSONDecodeError:
            return value[1:-1]
    return value


# ----------------------------- numeric evaluator -----------------------------

_ALLOWED_BINOPS = {
    ast.Add: lambda a, b: a + b,
    ast.Sub: lambda a, b: a - b,
    ast.Mult: lambda a, b: a * b,
    ast.Div: lambda a, b: a / b,
    ast.Mod: lambda a, b: a % b,
    ast.Pow: lambda a, b: a**b,
}
_ALLOWED_UNARY = {ast.UAdd: lambda a: a, ast.USub: lambda a: -a}


def _normalise_java_number(expr: str) -> str:
    expr = expr.strip()
    expr = re.sub(r"\((?:float|double|int|long|short|byte)\)\s*", "", expr)
    expr = re.sub(r"(?<=\d)[fFdDlL]\b", "", expr)
    replacements = {
        "Math.PI": repr(math.pi),
        "StrictMath.PI": repr(math.pi),
        "Mth.PI": repr(math.pi),
        "Mth.HALF_PI": repr(math.pi / 2),
        "Mth.TWO_PI": repr(math.pi * 2),
        "Mth.DEG_TO_RAD": repr(math.pi / 180),
        "Mth.RAD_TO_DEG": repr(180 / math.pi),
    }
    for old, new in replacements.items():
        expr = expr.replace(old, new)
    return expr


def eval_number(expr: str, constants: Optional[dict[str, float]] = None) -> float:
    expr = _normalise_java_number(expr)
    constants = constants or {}
    try:
        tree = ast.parse(expr, mode="eval")
    except SyntaxError as exc:
        raise ConversionError(f"Unsupported numeric expression: {expr!r}") from exc

    def visit(node: ast.AST) -> float:
        if isinstance(node, ast.Expression):
            return visit(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, float)):
            return float(node.value)
        if isinstance(node, ast.Name) and node.id in constants:
            return float(constants[node.id])
        if isinstance(node, ast.UnaryOp) and type(node.op) in _ALLOWED_UNARY:
            return _ALLOWED_UNARY[type(node.op)](visit(node.operand))
        if isinstance(node, ast.BinOp) and type(node.op) in _ALLOWED_BINOPS:
            return _ALLOWED_BINOPS[type(node.op)](visit(node.left), visit(node.right))
        raise ConversionError(f"Unsupported numeric expression: {expr!r}")

    value = visit(tree)
    if not math.isfinite(value):
        raise ConversionError(f"Non-finite numeric result: {expr!r}")
    return value


def eval_int(expr: str, constants: Optional[dict[str, float]] = None) -> int:
    return int(round(eval_number(expr, constants)))


def find_numeric_constants(text: str) -> dict[str, float]:
    constants: dict[str, float] = {}
    pattern = re.compile(
        r"(?:public|protected|private)?\s*(?:static\s+)?(?:final\s+)?"
        r"(?:float|double|int|long|short|byte)\s+([A-Za-z_$][\w$]*)\s*=\s*([^;]+);"
    )
    # Repeated passes allow constants to refer to earlier constants.
    pending = list(pattern.finditer(text))
    for _ in range(4):
        remaining = []
        changed = False
        for match in pending:
            try:
                constants[match.group(1)] = eval_number(match.group(2), constants)
                changed = True
            except ConversionError:
                remaining.append(match)
        pending = remaining
        if not changed:
            break
    return constants


# ------------------------------- model parser -------------------------------

_BIPED_PARTS: dict[str, tuple[str, tuple[float, float, float], tuple[float, float, float, float, float, float, int, int, float, bool]]] = {
    # variable: (display name, rotation point, cube x/y/z/dx/dy/dz/u/v/inflate/mirror)
    "field_78116_c": ("head", (0.0, 0.0, 0.0), (-4.0, -8.0, -4.0, 8.0, 8.0, 8.0, 0, 0, 0.0, False)),
    "bipedHead": ("head", (0.0, 0.0, 0.0), (-4.0, -8.0, -4.0, 8.0, 8.0, 8.0, 0, 0, 0.0, False)),
    "field_178720_f": ("hat", (0.0, 0.0, 0.0), (-4.0, -8.0, -4.0, 8.0, 8.0, 8.0, 32, 0, 0.5, False)),
    "bipedHeadwear": ("hat", (0.0, 0.0, 0.0), (-4.0, -8.0, -4.0, 8.0, 8.0, 8.0, 32, 0, 0.5, False)),
    "field_78115_e": ("body", (0.0, 0.0, 0.0), (-4.0, 0.0, -2.0, 8.0, 12.0, 4.0, 16, 16, 0.0, False)),
    "bipedBody": ("body", (0.0, 0.0, 0.0), (-4.0, 0.0, -2.0, 8.0, 12.0, 4.0, 16, 16, 0.0, False)),
    "field_178724_i": ("right_arm", (-5.0, 2.0, 0.0), (-3.0, -2.0, -2.0, 4.0, 12.0, 4.0, 40, 16, 0.0, False)),
    "bipedRightArm": ("right_arm", (-5.0, 2.0, 0.0), (-3.0, -2.0, -2.0, 4.0, 12.0, 4.0, 40, 16, 0.0, False)),
    "field_178723_h": ("left_arm", (5.0, 2.0, 0.0), (-1.0, -2.0, -2.0, 4.0, 12.0, 4.0, 32, 48, 0.0, True)),
    "bipedLeftArm": ("left_arm", (5.0, 2.0, 0.0), (-1.0, -2.0, -2.0, 4.0, 12.0, 4.0, 32, 48, 0.0, True)),
    "field_178722_k": ("right_leg", (-1.9, 12.0, 0.0), (-2.0, 0.0, -2.0, 4.0, 12.0, 4.0, 0, 16, 0.0, False)),
    "bipedRightLeg": ("right_leg", (-1.9, 12.0, 0.0), (-2.0, 0.0, -2.0, 4.0, 12.0, 4.0, 0, 16, 0.0, False)),
    "field_178721_j": ("left_leg", (1.9, 12.0, 0.0), (-2.0, 0.0, -2.0, 4.0, 12.0, 4.0, 16, 48, 0.0, True)),
    "bipedLeftLeg": ("left_leg", (1.9, 12.0, 0.0), (-2.0, 0.0, -2.0, 4.0, 12.0, 4.0, 16, 48, 0.0, True)),
}


def _looks_like_legacy_java_model(clean: str, class_name: str) -> bool:
    if re.search(r"\binterface\s+" + re.escape(class_name) + r"\b", clean):
        return False
    if "PartDefinition" in clean or "CubeListBuilder" in clean or "ModelPartData" in clean:
        return False
    declaration = re.search(
        r"\bclass\s+" + re.escape(class_name) + r"(?:\s+extends\s+([A-Za-z_$][\w$\.]*))?",
        clean,
    )
    superclass = declaration.group(1).split(".")[-1] if declaration and declaration.group(1) else ""
    geometry = any(token in clean for token in (
        "new ModelBox(", ".addBox(", ".func_78789_a(", ".func_78790_a(",
        ".cubeList.add(", ".field_78804_l.add(",
    ))
    model_super = superclass in {"ModelBase", "ModelBiped", "ModelPlayer"} or superclass.startswith("Model")
    return (geometry and (model_super or class_name.startswith("Model"))) or superclass in {"ModelBiped", "ModelPlayer"}


def _constructor_body(
    clean: str,
    class_name: str,
    constructor_args: Optional[list[object]] = None,
) -> tuple[str, dict[str, float]]:
    pattern = re.compile(
        r"\b(?:public|protected|private)?\s*" + re.escape(class_name) + r"\s*\(([^)]*)\)\s*\{"
    )
    candidates: list[tuple[int, str, list[str]]] = []
    for match in pattern.finditer(clean):
        try:
            body, _ = extract_balanced(clean, match.end() - 1, "{", "}")
        except ConversionError:
            continue
        param_names: list[str] = []
        raw_params = split_top_level(match.group(1)) if match.group(1).strip() else []
        for raw in raw_params:
            names = re.findall(r"[A-Za-z_$][\w$]*", raw)
            if names:
                param_names.append(names[-1])
        candidates.append((len(raw_params), body, param_names))
    if not candidates:
        return clean, {}

    supplied = list(constructor_args or [])
    if supplied:
        # Renderer registrations are the authoritative constructor invocation.
        # Prefer an overload with the same arity; if a decompiler omitted or
        # synthesized an overload, fall back to the nearest arity rather than
        # silently selecting the zero-argument constructor.
        candidates.sort(key=lambda row: (0 if row[0] == len(supplied) else 1, abs(row[0] - len(supplied)), row[0]))
    else:
        candidates.sort(key=lambda row: row[0])
    _arity, body, param_names = candidates[0]
    params: dict[str, float] = {name: 0.0 for name in param_names}
    for index, value in enumerate(supplied[:len(param_names)]):
        if isinstance(value, bool):
            params[param_names[index]] = 1.0 if value else 0.0
        elif isinstance(value, (int, float)):
            params[param_names[index]] = float(value)
    return body, params


def parse_legacy_java_model(
    text: str,
    source_name: str = "model.java",
    constructor_args: Optional[list[object]] = None,
) -> ParsedModel:
    """Parse decompiled 1.7.10-1.12.2 ModelRenderer/ModelBox source.

    This is deliberately a source interpreter for constructor geometry rather
    than a Java compiler.  It handles the common Techne/Tabula/decompiler forms,
    obfuscated SRG names, direct ModelBox construction, inherited ModelBiped
    parts, helper-generated rotations, and nested child hierarchies.
    """
    clean = strip_java_comments(text)
    warnings: list[str] = []
    class_match = re.search(r"\b(?:public\s+)?(?:final\s+)?class\s+([A-Za-z_$][\w$]*)", clean)
    class_name = class_match.group(1) if class_match else Path(source_name).stem
    constants = find_numeric_constants(clean)
    body, parameter_defaults = _constructor_body(clean, class_name, constructor_args)
    constants.update(parameter_defaults)

    texture_width, texture_height = 64, 32
    for field_name, target in (("field_78090_t", "width"), ("textureWidth", "width"), ("field_78089_u", "height"), ("textureHeight", "height")):
        match = re.search(r"(?:this\.)?" + re.escape(field_name) + r"\s*=\s*([^;]+);", body)
        if match:
            try:
                value = eval_int(match.group(1), constants)
                if target == "width":
                    texture_width = value
                else:
                    texture_height = value
            except ConversionError:
                pass
    super_call = re.search(r"\bsuper\s*\(([^;]*)\)\s*;", body)
    biped_inflate = 0.0
    if super_call:
        args = split_top_level(super_call.group(1))
        if args:
            try:
                biped_inflate = eval_number(args[0], constants)
            except ConversionError:
                pass
        if len(args) >= 4:
            try:
                texture_width = eval_int(args[-2], constants)
                texture_height = eval_int(args[-1], constants)
            except ConversionError:
                pass

    parts: list[Part] = []
    by_variable: dict[str, Part] = {}
    uv_by_variable: dict[str, tuple[int, int]] = {}
    mirror_by_variable: dict[str, bool] = {}
    aliases: dict[str, str] = {}

    def ensure_part(variable: str, name: Optional[str] = None) -> Part:
        variable = variable.removeprefix("this.")
        existing = by_variable.get(variable)
        if existing is not None:
            return existing
        biped = _BIPED_PARTS.get(variable)
        display_name = name or (biped[0] if biped else variable)
        part = Part(display_name, variable, "", source_index=len(parts))
        if biped:
            part.offset = biped[1]
            x, y, z, dx, dy, dz, u, v, inflate, mirror = biped[2]
            part.cubes.append(Cube(x, y, z, dx, dy, dz, u, v, mirror, inflate + biped_inflate, f"{display_name}_base"))
        parts.append(part)
        by_variable[variable] = part
        return part

    # Only instantiate inherited biped parts when the source actually extends
    # or references the biped model family.  This is vanilla API knowledge, not
    # a mod-specific fallback.
    declaration = re.search(r"\bclass\s+" + re.escape(class_name) + r"(?:\s+extends\s+([A-Za-z_$][\w$\.]*))?", clean)
    superclass = declaration.group(1).split(".")[-1] if declaration and declaration.group(1) else ""
    biped_source = superclass in {"ModelBiped", "ModelPlayer"} or any(variable in body for variable in _BIPED_PARTS)
    if biped_source:
        seen_biped_names: set[str] = set()
        for variable, (display_name, _offset, _cube) in _BIPED_PARTS.items():
            if display_name in seen_biped_names:
                continue
            if variable in body or superclass in {"ModelBiped", "ModelPlayer"} or superclass.startswith("Model"):
                part = ensure_part(variable)
                seen_biped_names.add(display_name)
                # Map unobfuscated and SRG aliases to one shared part.
                for alias, (alias_name, _aoffset, _acube) in _BIPED_PARTS.items():
                    if alias_name == display_name:
                        by_variable[alias] = part

    events: list[tuple[int, str, object]] = []

    renderer_pattern = re.compile(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*=\s*new\s+ModelRenderer\s*\(")
    for match in renderer_pattern.finditer(body):
        try:
            raw_args, end = extract_balanced(body, match.end() - 1)
        except ConversionError:
            continue
        events.append((match.start(), "create", (match.group(1), raw_args)))

    call_specs = {
        "set_uv": ("func_78784_a", "setTextureOffset"),
        # 1.7.10 decompilers commonly expose either SRG overload name:
        # func_78789_a(x,y,z,dx,dy,dz) or func_78790_a(...,inflate).
        "add_box": ("func_78789_a", "func_78790_a", "addBox"),
        "offset": ("func_78793_a", "setRotationPoint"),
        "add_child": ("func_78792_a", "addChild"),
    }
    for kind, methods in call_specs.items():
        method_alt = "|".join(re.escape(method) for method in methods)
        pattern = re.compile(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*(?:" + method_alt + r")\s*\(")
        for match in pattern.finditer(body):
            try:
                raw_args, _end = extract_balanced(body, match.end() - 1)
            except ConversionError:
                continue
            events.append((match.start(), kind, (match.group(1), raw_args)))

    rotation_helper = re.compile(
        r"(?:this\.)?[A-Za-z_$][\w$]*(?:Rotation|Rotate|rotation|rotate)[A-Za-z_$\w]*\s*\("
    )
    for match in rotation_helper.finditer(body):
        try:
            raw_args, _end = extract_balanced(body, match.end() - 1)
        except ConversionError:
            continue
        args = split_top_level(raw_args)
        if len(args) >= 4:
            variable = args[0].strip().removeprefix("this.")
            if re.fullmatch(r"[A-Za-z_$][\w$]*", variable):
                events.append((match.start(), "rotation", (variable, ",".join(args[1:4]))))

    assignment_specs = {
        "rotation_x": ("field_78795_f", "rotateAngleX", "xRot"),
        "rotation_y": ("field_78796_g", "rotateAngleY", "yRot"),
        "rotation_z": ("field_78808_h", "rotateAngleZ", "zRot"),
        "mirror": ("field_78809_i", "mirror"),
    }
    for kind, fields in assignment_specs.items():
        field_alt = "|".join(re.escape(field) for field in fields)
        pattern = re.compile(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*(?:" + field_alt + r")\s*=\s*([^;]+);")
        for match in pattern.finditer(body):
            events.append((match.start(), kind, (match.group(1), match.group(2))))

    # Some decompilers lower repeated part-position adjustments through a
    # temporary alias, for example:
    #   ModelRenderer var10000 = this.head;
    #   var10000.field_78797_d += 6.0F;
    # Preserve those constructor-time offsets/rotations instead of silently
    # discarding them or treating the alias as a separate model part.
    alias_pattern = re.compile(
        r"(?:\bModelRenderer\s+)?([A-Za-z_$][\w$]*)\s*=\s*(?:this\.)?([A-Za-z_$][\w$]*)\s*;"
    )
    for match in alias_pattern.finditer(body):
        lhs, rhs = match.groups()
        if lhs.startswith("var") or "ModelRenderer" in match.group(0):
            events.append((match.start(), "alias", (lhs, rhs)))

    field_delta_specs = {
        "rotation_delta_x": ("field_78795_f", "rotateAngleX", "xRot"),
        "rotation_delta_y": ("field_78796_g", "rotateAngleY", "yRot"),
        "rotation_delta_z": ("field_78808_h", "rotateAngleZ", "zRot"),
        "position_delta_x": ("field_78798_e", "rotationPointX", "x"),
        "position_delta_y": ("field_78797_d", "rotationPointY", "y"),
        "position_delta_z": ("field_78800_c", "rotationPointZ", "z"),
    }
    for kind, fields in field_delta_specs.items():
        field_alt = "|".join(re.escape(field) for field in fields)
        pattern = re.compile(
            r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*(?:" + field_alt + r")\s*([+-])=\s*([^;]+);"
        )
        for match in pattern.finditer(body):
            events.append((match.start(), kind, (match.group(1), match.group(2) + match.group(3))))

    model_box_pattern = re.compile(
        r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*(?:field_78804_l|cubeList)\s*\.\s*add\s*\(\s*new\s+ModelBox\s*\("
    )
    for match in model_box_pattern.finditer(body):
        try:
            raw_args, _end = extract_balanced(body, match.end() - 1)
        except ConversionError:
            continue
        events.append((match.start(), "model_box", (match.group(1), raw_args)))

    for _position, kind, payload in sorted(events, key=lambda row: (row[0], row[1])):
        variable, raw_args = payload  # type: ignore[misc]
        variable = str(variable).removeprefix("this.")
        if kind == "alias":
            aliases[variable] = aliases.get(str(raw_args).removeprefix("this."), str(raw_args).removeprefix("this."))
            continue
        variable = aliases.get(variable, variable)
        part = ensure_part(variable)
        args = split_top_level(str(raw_args))
        try:
            if kind == "create":
                if len(args) >= 3:
                    uv_by_variable[variable] = (eval_int(args[-2], constants), eval_int(args[-1], constants))
            elif kind == "set_uv" and len(args) >= 2:
                uv_by_variable[variable] = (eval_int(args[0], constants), eval_int(args[1], constants))
            elif kind == "offset" and len(args) >= 3:
                part.offset = tuple(eval_number(value, constants) for value in args[:3])  # type: ignore[assignment]
            elif kind == "add_child" and args:
                child_var = args[0].strip().removeprefix("this.")
                child = ensure_part(child_var)
                if child.parent is not part:
                    if child.parent is not None and child in child.parent.children:
                        child.parent.children.remove(child)
                    child.parent = part
                    child.parent_variable = part.variable
                    if child not in part.children:
                        part.children.append(child)
            elif kind == "rotation" and len(args) >= 3:
                part.rotation = tuple(eval_number(value, constants) for value in args[:3])  # type: ignore[assignment]
            elif kind in {"rotation_x", "rotation_y", "rotation_z"}:
                value = eval_number(str(raw_args), constants)
                rotation = list(part.rotation)
                rotation[{"rotation_x": 0, "rotation_y": 1, "rotation_z": 2}[kind]] = value
                part.rotation = tuple(rotation)  # type: ignore[assignment]
            elif kind.startswith("rotation_delta_"):
                raw = str(raw_args).strip()
                sign = -1.0 if raw.startswith("-") else 1.0
                value = eval_number(raw[1:] if raw[:1] in "+-" else raw, constants) * sign
                rotation = list(part.rotation)
                rotation[{"rotation_delta_x": 0, "rotation_delta_y": 1, "rotation_delta_z": 2}[kind]] += value
                part.rotation = tuple(rotation)  # type: ignore[assignment]
            elif kind.startswith("position_delta_"):
                raw = str(raw_args).strip()
                sign = -1.0 if raw.startswith("-") else 1.0
                value = eval_number(raw[1:] if raw[:1] in "+-" else raw, constants) * sign
                offset = list(part.offset)
                offset[{"position_delta_x": 0, "position_delta_y": 1, "position_delta_z": 2}[kind]] += value
                part.offset = tuple(offset)  # type: ignore[assignment]
            elif kind == "mirror":
                mirror_by_variable[variable] = str(raw_args).strip().lower() == "true"
            elif kind == "add_box":
                cube_name = f"{part.name}_cube{len(part.cubes) + 1}"
                if args and args[0].strip().startswith('"'):
                    cube_name = unquote_java_string(args.pop(0))
                if len(args) >= 6:
                    x, y, z = (eval_number(value, constants) for value in args[:3])
                    dx, dy, dz = (eval_number(value, constants) for value in args[3:6])
                    inflate = eval_number(args[6], constants) if len(args) >= 7 else 0.0
                    u, v = uv_by_variable.get(variable, (0, 0))
                    part.cubes.append(Cube(x, y, z, dx, dy, dz, u, v, mirror_by_variable.get(variable, False), inflate, cube_name))
            elif kind == "model_box":
                # ModelBox(owner, u, v, x, y, z, dx, dy, dz, inflate, mirror)
                if len(args) >= 9:
                    offset = 1 if not re.fullmatch(r"[-+]?\d+(?:\.\d+)?[fFdD]?", args[0].strip()) else 0
                    values = args[offset:]
                    if len(values) >= 8:
                        u, v = eval_int(values[0], constants), eval_int(values[1], constants)
                        x, y, z = (eval_number(value, constants) for value in values[2:5])
                        dx, dy, dz = (eval_number(value, constants) for value in values[5:8])
                        inflate = eval_number(values[8], constants) if len(values) >= 9 else 0.0
                        mirror = values[9].strip().lower() == "true" if len(values) >= 10 else mirror_by_variable.get(variable, False)
                        part.cubes.append(Cube(x, y, z, dx, dy, dz, u, v, mirror, inflate, f"{part.name}_cube{len(part.cubes) + 1}"))
        except ConversionError as exc:
            warnings.append(f"Skipped {kind} operation for {variable}: {exc}")

    # Remove untouched duplicate biped aliases and empty incidental references,
    # while retaining hierarchy parents even when they have no cubes.
    unique_parts: list[Part] = []
    seen_ids: set[int] = set()
    for part in parts:
        if id(part) in seen_ids:
            continue
        seen_ids.add(id(part))
        if part.cubes or part.children or part.parent is not None:
            part.source_index = len(unique_parts)
            unique_parts.append(part)
    if not unique_parts or sum(len(part.cubes) for part in unique_parts) == 0:
        raise ConversionError("No legacy ModelRenderer/ModelBox geometry was recovered from Java source.")

    model = ParsedModel(class_name, texture_width, texture_height, unique_parts, warnings)
    model.source_format = "legacy_java_source"
    model.constructor_args = list(constructor_args or [])
    try:
        analysis, logic, baked, animation_warnings = _extract_legacy_java_animation(clean, class_name, unique_parts)
        model.animation_analysis = analysis
        model.animation_logic = logic
        model.baked_animation_json = baked
        model.warnings.extend(animation_warnings)
        # Native Blockbench animators are generated in model_to_bbmodel only
        # after deterministic group UUIDs have been assigned. Baking here used
        # empty UUID keys and caused every animated bone to overwrite the prior
        # one, leaving only the final limb/tail channel in source-only exports.
    except Exception as exc:
        model.warnings.append(f"Decompiled Java animation recovery failed: {exc}")
    return model


def _legacy_java_methods(text: str) -> dict[str, list[tuple[list[str], str, str]]]:
    methods: dict[str, list[tuple[list[str], str, str]]] = {}
    pattern = re.compile(
        r"(?m)^[ \t]*(?:(?:public|protected|private|static|final|synchronized)\s+)*"
        r"[A-Za-z_$][\w$<>\[\].?, ]*\s+([A-Za-z_$][\w$]*)\s*\(([^;{}]*)\)\s*\{"
    )
    for match in pattern.finditer(text):
        brace = text.find("{", match.start(), match.end())
        try:
            body, end = extract_balanced(text, brace, "{", "}")
        except ConversionError:
            continue
        params: list[str] = []
        for raw in split_top_level(match.group(2)):
            cleaned = re.sub(r"@\w+(?:\([^)]*\))?\s*", "", raw).strip()
            token = re.search(r"([A-Za-z_$][\w$]*)\s*(?:\[\])?\s*$", cleaned)
            if token:
                params.append(token.group(1))
        methods.setdefault(match.group(1), []).append((params, body, text[match.start():end]))
    return methods


def _convert_java_ternaries(expr: str) -> str:
    text = str(expr).strip()
    text = re.sub(r"\(*\s*(?:entity|e|this\.animEntity)\s*\)*\s*\.getAttacking\s*\(\s*\)", "attacking", text)
    text = re.sub(r"\(*\s*(?:entity|e|this\.animEntity)\s*\)*\s*\.isSprinting\s*\(\s*\)", "sprinting", text)
    text = re.sub(r"\(*\s*(?:entity|e|this\.animEntity)\s*\)*\s*\.isBaby\s*\(\s*\)", "baby", text)
    text = re.sub(r"[A-Za-z_$][\w$]*\s*\.getAttacking\s*\(\s*\)", "attacking", text)
    text = re.sub(r"[A-Za-z_$][\w$]*\s*\.isSprinting\s*\(\s*\)", "sprinting", text)
    text = re.sub(r"[A-Za-z_$][\w$]*\s*\.isBaby\s*\(\s*\)", "baby", text)
    # Preserve getter semantics: scalar-valued getters (scale/speed/size/factor)
    # must remain numeric inputs, while activity/flag getters represent entity state.
    def _generic_getter_repl(match: re.Match[str]) -> str:
        name = match.group(1)
        low = name.lower()
        if any(token in low for token in ("scale", "speed", "size", "factor", "multiplier", "amount", "value")):
            return "entity_scalar"
        return "entity_state"
    text = re.sub(r"(?:[A-Za-z_$][\w$]*|\([^)]*\))\s*\.get([A-Za-z_$][\w$]*)\s*\(\s*\)", _generic_getter_repl, text)

    def top_level_char(src: str, wanted: str, start_at: int = 0) -> int:
        depth = 0; quote = None; i = start_at
        while i < len(src):
            ch = src[i]
            if quote:
                if ch == quote and (i == 0 or src[i-1] != '\\'): quote = None
            elif ch in ('"', "'"): quote = ch
            elif ch in '([{': depth += 1
            elif ch in ')]}': depth = max(0, depth - 1)
            elif depth == 0 and ch == wanted: return i
            i += 1
        return -1

    q = top_level_char(text, '?')
    if q < 0:
        return text
    depth = 0; quote = None; nested = 0; colon = -1; i = q + 1
    while i < len(text):
        ch = text[i]
        if quote:
            if ch == quote and (i == 0 or text[i-1] != '\\'): quote = None
        elif ch in ('"', "'"): quote = ch
        elif ch in '([{': depth += 1
        elif ch in ')]}': depth = max(0, depth - 1)
        elif depth == 0 and ch == '?': nested += 1
        elif depth == 0 and ch == ':':
            if nested: nested -= 1
            else: colon = i; break
        i += 1
    if colon < 0: return text
    cond = _convert_java_ternaries(text[:q].strip())
    yes = _convert_java_ternaries(text[q+1:colon].strip())
    no = _convert_java_ternaries(text[colon+1:].strip())
    return f"(({yes}) if ({cond}) else ({no}))"


def _normalise_legacy_java_expr(expr: str, substitutions: dict[str, str]) -> str:
    result = str(expr).strip()
    result = re.sub(r"\((?:float|double|int|long)\)\s*", "", result)
    # Reference casts use uppercase class/type names; lowercase parentheses are
    # often ordinary function arguments such as radians(h1).
    result = re.sub(r"\(([A-Z][A-Za-z0-9_$]*(?:\.[A-Za-z_$][\w$]*)*(?:<[^>]+>)?)\)\s*", "", result)
    replacements = {
        "MathHelper.func_76134_b": "cos", "MathHelper.cos": "cos",
        "MathHelper.func_76126_a": "sin", "MathHelper.sin": "sin",
        "MathHelper.func_76135_e": "abs", "MathHelper.abs": "abs",
        "Math.cos": "cos", "Math.sin": "sin", "Math.abs": "abs",
        "Math.toRadians": "radians", "Math.toDegrees": "degrees", "Math.pow": "pow",
        "Mth.cos": "cos", "Mth.sin": "sin", "Mth.abs": "abs", "Math.PI": str(math.pi),
    }
    for old, new in replacements.items(): result = result.replace(old, new)
    result = re.sub(r"(?<=\d)[fFdD]\b", "", result)
    result = result.replace("this.", "")
    for name, value in sorted(substitutions.items(), key=lambda item: -len(item[0])):
        result = re.sub(rf"\b{re.escape(name)}\b", f"({value})", result)
    result = re.sub(r"^\([^()=]+\)\s*=\s*(.+)$", r"\1", result)
    return _convert_java_ternaries(result)


def _extract_legacy_java_animation(
    text: str,
    class_name: str,
    parts: list[Part],
) -> tuple[dict[str, object], str, dict[str, object], list[str]]:
    """Recover common procedural animation formulas from decompiled Java.

    Bytecode remains the preferred source when available, but this keeps
    decompiled-only conversions functional and understands same-class helper
    methods plus both MCP and SRG ModelRenderer field names.
    """
    methods = _legacy_java_methods(text)
    candidates: list[tuple[str, list[str], str, str]] = []
    for name in ("func_78087_a", "setRotationAngles", "setupAnim", "func_78088_a", "render", "renderToBuffer"):
        for params, body, source in methods.get(name, []):
            candidates.append((name, params, body, source))
    if not candidates:
        return {}, "", {}, []

    part_names = {part.variable for part in parts} | {part.name for part in parts}
    canonical_to_part: dict[str, str] = {}
    def _canonical_part_name(value: str) -> str:
        return re.sub(r"[^a-z0-9]", "", str(value).lower())
    for _part in parts:
        for _name in {_part.variable, _part.name}:
            canonical_to_part.setdefault(_canonical_part_name(_name), _part.variable)
    def resolve_bone(value: str) -> str:
        raw = str(value).removeprefix("this.")
        if raw in part_names: return raw
        return canonical_to_part.get(_canonical_part_name(raw), raw)
    channel_fields = {
        "field_78795_f": ("rotation", "x"), "rotateAngleX": ("rotation", "x"), "xRot": ("rotation", "x"),
        "field_78796_g": ("rotation", "y"), "rotateAngleY": ("rotation", "y"), "yRot": ("rotation", "y"),
        "field_78808_h": ("rotation", "z"), "rotateAngleZ": ("rotation", "z"), "zRot": ("rotation", "z"),
        "field_78800_c": ("position", "x"), "rotationPointX": ("position", "x"), "x": ("position", "x"),
        "field_78797_d": ("position", "y"), "rotationPointY": ("position", "y"), "y": ("position", "y"),
        "field_78798_e": ("position", "z"), "rotationPointZ": ("position", "z"), "z": ("position", "z"),
    }
    semantic_order = ["limbSwing", "limbSwingAmount", "ageInTicks", "netHeadYaw", "headPitch", "scaleFactor"]
    scalar_constants = {
        name: repr(float(value))
        for name, value in find_numeric_constants(text).items()
        if math.isfinite(float(value))
    }
    # Modern model classes often keep animation tunables in fields initialized
    # from constructor parameters (for example `wingspeed`). When the source
    # model is being converted without an entity instance, use a neutral 1.0
    # parameter value so the recovered waveform is still exported instead of
    # becoming an unevaluable expression. Literal field initializers still win.
    declared_float_fields = re.findall(
        r"\b(?:private|protected|public)?\s*(?:final\s+)?(?:float|double)\s+([A-Za-z_$][\w$]*)\s*(?:;|=)",
        text,
    )
    for field_name in declared_float_fields:
        scalar_constants.setdefault(field_name, "1.0")

    # Renderer/entity state arrays are commonly written in setupAnim and read
    # later by renderToBuffer. Normalize indexed fields into scalar aliases so
    # the recovered render formulas can consume the source-authored values.
    shared_state_exprs: dict[str, str] = {}
    array_assignment = re.compile(
        r"(?:this\.)?([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)\s*\[\s*(\d+)\s*\]\s*=\s*([^;]+);"
    )
    for match in array_assignment.finditer(text):
        state_key = f"{match.group(1)}_{match.group(2)}_{match.group(3)}"
        shared_state_exprs[state_key] = _normalise_legacy_java_expr(match.group(4), scalar_constants)

    neutral_values: dict[tuple[str, str, str], float] = {}
    for part in parts:
        for axis, value in zip(("x", "y", "z"), part.rotation):
            neutral_values[(part.variable, "rotation", axis)] = float(value)
        for axis, value in zip(("x", "y", "z"), part.offset):
            neutral_values[(part.variable, "position", axis)] = float(value)

    def recover(candidate: tuple[str, list[str], str, str]) -> tuple[list[dict[str, str]], str]:
        method_name, params, body, source = candidate
        if method_name in {"func_78088_a", "render", "renderToBuffer"}:
            # Modern Mojmap models frequently put the actual animation in
            # renderToBuffer and cache setupAnim arguments into fields.  In that
            # form the render method's Java parameters are pose/render plumbing,
            # not animation clocks.  Preserve the canonical clock names so local
            # aliases such as `float ageInTicks = this.animAgeInTicks;` remain
            # bakeable.
            float_params = []
        else:
            float_params = params[:6]
        substitutions = dict(scalar_constants)
        substitutions.update(shared_state_exprs)
        substitutions.update({
            semantic: semantic
            for semantic in semantic_order
        })
        substitutions.update({
            param: semantic_order[index]
            for index, param in enumerate(float_params)
            if index < len(semantic_order)
        })
        if method_name == "setupAnim" and len(params) >= 6:
            substitutions.update({p: semantic_order[i] for i, p in enumerate(params[1:7]) if i < len(semantic_order)})
            substitutions[params[0]] = "entity"
            for cached_field, cached_param in re.findall(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*=\s*(?:this\.)?([A-Za-z_$][\w$]*)\s*;", text):
                if cached_param in semantic_order:
                    substitutions[cached_field] = cached_param
        elif method_name in {"func_78088_a", "render", "renderToBuffer"} and params:
            substitutions[params[0]] = "entity"
            for cached_field, cached_param in re.findall(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*=\s*(?:this\.)?([A-Za-z_$][\w$]*)\s*;", text):
                if cached_param in semantic_order:
                    substitutions[cached_field] = cached_param
        formulas_by_key: dict[tuple[str, str, str], dict[str, str]] = {}

        def process(method_body: str, local_subs: dict[str, str], depth: int = 0) -> None:
            if depth > 4:
                return
            local_values = dict(local_subs)

            # IMPORTANT: legacy/modern decompiled model methods frequently reuse the
            # same temporary variable (`newangle`, `angle`, `offset`, etc.) several
            # times in one method. The value visible to a later assignment is the
            # value from the most recent earlier assignment, not the first/most
            # dynamic assignment in the whole method. Build that state in source
            # order so expressions are bound exactly where the source executes.
            local_pattern = re.compile(
                r"(?:\b(?:final\s+)?(?:float|double|int|boolean)\s+)?\b([A-Za-z_$][\w$]*)\s*(=|\+=|-=)\s*([^;]+);"
            )

            def replace_part_refs(expression: str) -> str:
                rp = re.compile(r"\b([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)\b")
                def repl(match):
                    bone, field_name = match.groups()
                    if field_name in channel_fields:
                        resolved = resolve_bone(bone)
                        if resolved in part_names: return f"{resolved}.{field_name}"
                    return match.group(0)
                return rp.sub(repl, expression)


            # Recover statements in source order. This is intentionally a small
            # execution model rather than several independent regex passes: it keeps
            # temporary locals, chained assignments, part assignments, and helper
            # calls in the same order as the original Java method.
            chain_pattern = re.compile(
                r"((?:(?:this\.)?[A-Za-z_$][\w$]*\s*\.\s*[A-Za-z_$][\w$]*\s*=\s*){2,})([^;]+);"
            )
            assignment_pattern = re.compile(
                r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*([A-Za-z_$][\w$]*)\s*(=|\+=|-=)\s*([^;]+);"
            )
            local_matches = list(local_pattern.finditer(method_body))
            chain_matches = list(chain_pattern.finditer(method_body))
            assignment_matches = list(assignment_pattern.finditer(method_body))
            consumed_ranges = [(m.start(), m.end()) for m in chain_matches]

            events: list[tuple[int, int, str, re.Match[str]]] = []
            for m in local_matches:
                # Skip the left side of part assignments (`part.xRot = ...`).
                previous = method_body[m.start() - 1] if m.start() else ""
                name = m.group(1)
                raw_value = m.group(3)
                if previous == "." or name in part_names or name in channel_fields:
                    continue
                if any(a <= m.start() < b for a, b in consumed_ranges):
                    continue
                events.append((m.start(), 0, "local", m))
            for m in chain_matches:
                events.append((m.start(), 1, "chain", m))
            for m in assignment_matches:
                if any(a <= m.start() < b for a, b in consumed_ranges):
                    continue
                events.append((m.start(), 2, "assignment", m))

            # Helper calls are also order-sensitive because a helper may mutate a
            # part after a caller has already assigned one of its fields.
            call_pattern = re.compile(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\(")
            for call in call_pattern.finditer(method_body):
                helper_name = call.group(1)
                if helper_name in {"sin", "cos", "abs", "super"} or helper_name not in methods:
                    continue
                events.append((call.start(), 3, "helper", call))

            for _position, _priority, event_kind, event in sorted(events, key=lambda row: (row[0], row[1])):
                if event_kind == "local":
                    name, operator, raw_value = event.groups()
                    value = _normalise_legacy_java_expr(raw_value, local_values)
                    if "instanceof" in value:
                        continue
                    if operator != "=" and name in local_values:
                        value = f"({local_values[name]}) {'+' if operator == '+=' else '-'} ({value})"
                    # Decompiled if/else blocks are flattened into source order.
                    # A dynamic branch is followed by a static reset in many model
                    # classes; that reset is only a fallback for an unknown runtime
                    # condition and must not erase the recoverable waveform. Keep a
                    # later dynamic assignment, but ignore a later static replacement
                    # when the current value already carries an animation clock.
                    dynamic_score = sum(token in value for token in semantic_order)
                    current = local_values.get(name)
                    current_score = sum(token in current for token in semantic_order) if current else 0
                    if current is not None and current_score > 0 and dynamic_score == 0 and operator == "=":
                        continue
                    local_values[name] = value
                    continue

                if event_kind == "chain":
                    targets = re.findall(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.\s*([A-Za-z_$][\w$]*)\s*=\s*", event.group(1))
                    expression = replace_part_refs(_normalise_legacy_java_expr(event.group(2), local_values))
                    for bone, field_name in reversed(targets):
                        bone = resolve_bone(bone)
                        if bone not in part_names or field_name not in channel_fields:
                            continue
                        channel, axis = channel_fields[field_name]
                        formulas_by_key[(bone, channel, axis)] = {
                            "bone": bone, "channel": channel, "axis": axis, "expression": expression,
                        }
                    continue

                if event_kind == "assignment":
                    bone, field_name, operator, raw_expr = event.groups()
                    bone = resolve_bone(bone)
                    if bone not in part_names or field_name not in channel_fields:
                        continue
                    expression = replace_part_refs(_normalise_legacy_java_expr(raw_expr, local_values))
                    if re.search(r"\b(?:new|instanceof|null)\b", expression):
                        continue
                    channel, axis = channel_fields[field_name]
                    key = (bone, channel, axis)
                    if operator != "=" and key in formulas_by_key:
                        prior = formulas_by_key[key]["expression"]
                        expression = f"({prior}) {'+' if operator == '+=' else '-'} ({expression})"
                    # Last source assignment wins for the sampled execution path.
                    formulas_by_key[key] = {
                        "bone": bone,
                        "channel": channel,
                        "axis": axis,
                        "expression": expression,
                    }
                    continue

                if event_kind == "helper":
                    helper_name = event.group(1)
                    try:
                        raw_args, _ = extract_balanced(method_body, event.end() - 1)
                    except ConversionError:
                        continue
                    args = split_top_level(raw_args)
                    overload = next((entry for entry in methods[helper_name] if len(entry[0]) == len(args)), None)
                    if not overload:
                        continue
                    helper_params, helper_body, _helper_source = overload
                    helper_subs = dict(local_values)
                    for param, arg in zip(helper_params, args):
                        helper_subs[param] = replace_part_refs(_normalise_legacy_java_expr(arg, local_values))
                    helper_body_resolved = helper_body
                    for param, resolved_arg in zip(helper_params, [helper_subs[p] for p in helper_params]):
                        bare = resolved_arg.removeprefix("this.")
                        helper_body_resolved = re.sub(rf"\b{re.escape(param)}\b", bare, helper_body_resolved)
                    process(helper_body_resolved, helper_subs, depth + 1)

            # Common generated helper calls directly express three rotation axes.
            for call in re.finditer(r"(?:this\.)?[A-Za-z_$][\w$]*(?:Rotation|Rotate|rotation|rotate)[A-Za-z_$\w]*\s*\(", method_body):
                try:
                    raw_args, _ = extract_balanced(method_body, call.end() - 1)
                except ConversionError:
                    continue
                args = split_top_level(raw_args)
                if len(args) < 4:
                    continue
                bone = args[0].strip().removeprefix("this.")
                if bone not in part_names:
                    continue
                for axis, arg in zip(("x", "y", "z"), args[1:4]):
                    expr = replace_part_refs(_normalise_legacy_java_expr(arg, local_values))
                    formulas_by_key[(bone, "rotation", axis)] = {
                        "bone": bone, "channel": "rotation", "axis": axis, "expression": expr,
                    }

        process(body, substitutions)
        return list(formulas_by_key.values()), source

    recovered: list[tuple[int, list[dict[str, str]], str]] = []
    merged_formulas: dict[tuple[str, str, str, str], dict[str, str]] = {}
    source_names: list[str] = []
    for candidate in candidates:
        candidate_formulas, source = recover(candidate)
        recovered.append((len(candidate_formulas), candidate_formulas, source))
        if source:
            source_names.append(source.split("{", 1)[0].strip())
        for formula in candidate_formulas:
            key = (str(formula.get("bone", "")), str(formula.get("channel", "")), str(formula.get("axis", "")), str(formula.get("expression", "")))
            merged_formulas.setdefault(key, formula)
    formulas = list(merged_formulas.values())
    source_method = "\n\n".join(source_names)
    if not formulas:
        return {}, "", {}, []

    by_bone: dict[str, dict[str, list[str]]] = {}
    for formula in formulas:
        expression = formula["expression"]
        has_walk = "limbSwing" in expression or "limbSwingAmount" in expression
        has_idle = "ageInTicks" in expression
        has_head = "netHeadYaw" in expression or "headPitch" in expression
        category = "combined" if sum((has_walk, has_idle, has_head)) > 1 else (
            "walk_cycle" if has_walk else "idle" if has_idle else "head_tracking" if has_head else "static"
        )
        formula["animation_class"] = category
        by_bone.setdefault(formula["bone"], {}).setdefault(formula["channel"], []).append(expression)

    # Propagate animation classes through ModelRenderer dependencies. In segmented
    # legacy models, a body segment often derives its position from the previous
    # segment's animated rotation/position, so the dependent formula itself may not
    # mention limbSwing or ageInTicks. Without propagation those segments are
    # classified static and disappear from the baked clip.
    field_to_channel = {
        "field_78795_f": ("rotation", "x"), "rotateAngleX": ("rotation", "x"), "xRot": ("rotation", "x"),
        "field_78796_g": ("rotation", "y"), "rotateAngleY": ("rotation", "y"), "yRot": ("rotation", "y"),
        "field_78808_h": ("rotation", "z"), "rotateAngleZ": ("rotation", "z"), "zRot": ("rotation", "z"),
        "field_78800_c": ("position", "x"), "rotationPointX": ("position", "x"), "x": ("position", "x"),
        "field_78797_d": ("position", "y"), "rotationPointY": ("position", "y"), "y": ("position", "y"),
        "field_78798_e": ("position", "z"), "rotationPointZ": ("position", "z"), "z": ("position", "z"),
    }
    formula_by_key = {
        (str(formula["bone"]), str(formula["channel"]), str(formula["axis"])): formula
        for formula in formulas
    }
    ref_pattern = re.compile(r"\b([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)\b")

    def merge_classes(classes: set[str]) -> str:
        dynamic = {value for value in classes if value != "static"}
        if not dynamic:
            return "static"
        if len(dynamic) == 1:
            return next(iter(dynamic))
        return "combined"

    for _ in range(16):
        changed = False
        for formula in formulas:
            inherited = {str(formula.get("animation_class", "static"))}
            for ref in ref_pattern.finditer(str(formula.get("expression", ""))):
                ref_bone, field_name = ref.groups()
                mapped = field_to_channel.get(field_name)
                if mapped is None:
                    continue
                ref_channel, ref_axis = mapped
                referenced = formula_by_key.get((ref_bone, ref_channel, ref_axis))
                if referenced is not None:
                    inherited.add(str(referenced.get("animation_class", "static")))
            resolved = merge_classes(inherited)
            if resolved != formula.get("animation_class"):
                formula["animation_class"] = resolved
                changed = True
        if not changed:
            break

    classification: dict[str, set[str]] = {}
    for formula in formulas:
        category = str(formula.get("animation_class", "static"))
        if category != "static":
            classification.setdefault(formula["bone"], set()).add(category)

    analysis: dict[str, object] = {
        "source_method": source_method.split("{", 1)[0].strip(),
        "source_format": "legacy_java_source",
        "formula_count": len(formulas),
        "bones": by_bone,
        "classification": {key: sorted(values) for key, values in classification.items()},
        "formulas": formulas,
        "warnings": [],
    }

    from legacy_bridge import _LegacyPart, _build_baked_preview
    legacy_parts: dict[str, _LegacyPart] = {}
    for index, part in enumerate(parts):
        legacy_part = _LegacyPart(
            name=part.name,
            offset=list(part.offset),
            rotation=list(part.rotation),
            source_index=index,
            parent=(part.parent.name if part.parent is not None else None),
            cubes=[
                (
                    float(cube.x), float(cube.y), float(cube.z),
                    float(cube.dx), float(cube.dy), float(cube.dz),
                    int(cube.uv_x), int(cube.uv_y), float(cube.inflate), bool(cube.mirror),
                )
                for cube in part.cubes
            ],
        )
        # Animation formulas are recovered against either the source variable
        # (`__part_7`, `this.jaw`) or the authored ModelPart name (`jaw`). The
        # bind-pose evaluator must accept both identities; otherwise it silently
        # falls back to zero and turns absolute source rotations/positions into
        # wildly exaggerated animation deltas.
        legacy_parts[part.variable] = legacy_part
        if part.name and part.name != part.variable:
            legacy_parts[part.name] = legacy_part
    analysis["source_has_open_closed"] = bool(re.search(r"\bgetOpenClosed\s*\(", text))
    analysis["source_has_activity"] = bool(re.search(r"\bgetActivity\s*\(", text))
    baked = _build_baked_preview(analysis, legacy_parts)
    logic = (
        f"// Extracted by MojModelBridge {VERSION} from decompiled legacy Java.\n"
        "// Preserve/adapt this source method for runtime behavior not representable as baked keyframes.\n\n"
        + source_method.strip()
        + "\n"
    )
    return analysis, logic, baked, []

def iter_method_calls(chain: str) -> Iterable[tuple[str, str]]:
    i = 0
    while i < len(chain):
        dot = chain.find(".", i)
        if dot < 0:
            return
        match = re.match(r"\.\s*([A-Za-z_$][\w$]*)\s*\(", chain[dot:])
        if not match:
            i = dot + 1
            continue
        name = match.group(1)
        open_index = dot + match.end() - 1
        try:
            args, end = extract_balanced(chain, open_index)
        except ConversionError:
            return
        yield name, args
        i = end


def parse_inflate(expr: str, constants: dict[str, float], warnings: list[str]) -> float:
    expr = expr.strip()
    if not expr or expr.endswith("CubeDeformation.NONE") or expr == "CubeDeformation.NONE":
        return 0.0
    match = re.search(r"(?:new\s+)?CubeDeformation\s*\((.*)\)", expr, re.S)
    if match:
        args = split_top_level(match.group(1))
        values = [eval_number(arg, constants) for arg in args]
        if len(values) == 1:
            return values[0]
        if len(values) >= 3:
            if max(values[:3]) - min(values[:3]) > 1e-7:
                warnings.append(
                    "A per-axis CubeDeformation was reduced to one Blockbench inflate value "
                    f"({values[0]}, {values[1]}, {values[2]} -> average)."
                )
            return sum(values[:3]) / 3.0
    return eval_number(expr, constants)


def parse_builder(builder: str, constants: dict[str, float], warnings: list[str], part_name: str) -> list[Cube]:
    cubes: list[Cube] = []
    uv_x = 0
    uv_y = 0
    mirror = False
    cube_number = 0
    for method, raw_args in iter_method_calls(builder):
        args = split_top_level(raw_args)
        if method in ("texOffs", "uv") and len(args) >= 2:
            uv_x = eval_int(args[0], constants)
            uv_y = eval_int(args[1], constants)
        elif method in ("mirror", "mirrored"):
            mirror = True if not args or not args[0] else args[0].strip().lower() != "false"
        elif method in ("addBox", "cuboid"):
            cube_number += 1
            cube_name = f"{part_name}_cube" if cube_number == 1 else f"{part_name}_cube_{cube_number}"
            if args and args[0].strip().startswith('"'):
                cube_name = unquote_java_string(args.pop(0)) or cube_name
            if len(args) < 6:
                warnings.append(f"Skipped an addBox in {part_name}: expected at least 6 dimensions.")
                continue
            try:
                x, y, z, dx, dy, dz = (eval_number(arg, constants) for arg in args[:6])
            except ConversionError as exc:
                warnings.append(f"Skipped an addBox in {part_name}: {exc}")
                continue
            inflate = 0.0
            cube_mirror = mirror
            # Common overloads add CubeDeformation/dilation and sometimes a boolean.
            for extra in args[6:]:
                token = extra.strip()
                if token.lower() in ("true", "false"):
                    cube_mirror = token.lower() == "true"
                elif "CubeDeformation" in token or "Dilation" in token:
                    token = token.replace("Dilation", "CubeDeformation")
                    try:
                        inflate = parse_inflate(token, constants, warnings)
                    except ConversionError as exc:
                        warnings.append(f"Could not parse inflation for {part_name}: {exc}")
                else:
                    try:
                        inflate = eval_number(token, constants)
                    except ConversionError:
                        pass
            cubes.append(
                Cube(
                    x=x,
                    y=y,
                    z=z,
                    dx=dx,
                    dy=dy,
                    dz=dz,
                    uv_x=uv_x,
                    uv_y=uv_y,
                    mirror=cube_mirror,
                    inflate=inflate,
                    name=cube_name,
                )
            )
    return cubes


def parse_pose(pose: str, constants: dict[str, float], warnings: list[str], part_name: str) -> tuple[tuple[float, float, float], tuple[float, float, float]]:
    pose = pose.strip()
    if pose in ("PartPose.ZERO", "ModelTransform.NONE", "ModelTransform.NONE;"):
        return (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)
    match = re.search(r"(?:PartPose|ModelTransform)\s*\.\s*(offsetAndRotation|offset|rotation|of|pivot)\s*\(", pose)
    if not match:
        warnings.append(f"Unknown PartPose for {part_name}; used zero transform: {pose[:100]}")
        return (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)
    args_text, _ = extract_balanced(pose, match.end() - 1)
    args = split_top_level(args_text)
    try:
        nums = [eval_number(arg, constants) for arg in args]
    except ConversionError as exc:
        warnings.append(f"Could not parse PartPose for {part_name}; used zero transform: {exc}")
        return (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)
    method = match.group(1)
    if method in ("offsetAndRotation", "of") and len(nums) >= 6:
        return tuple(nums[:3]), tuple(nums[3:6])
    if method in ("offset", "pivot") and len(nums) >= 3:
        return tuple(nums[:3]), (0.0, 0.0, 0.0)
    if method == "rotation" and len(nums) >= 3:
        return (0.0, 0.0, 0.0), tuple(nums[:3])
    warnings.append(f"Incomplete PartPose for {part_name}; used zero transform: {pose[:100]}")
    return (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)


def find_builder_variables(text: str) -> dict[str, str]:
    builders: dict[str, str] = {}
    pattern = re.compile(r"CubeListBuilder\s+([A-Za-z_$][\w$]*)\s*=", re.M)
    for match in pattern.finditer(text):
        start = match.end()
        end = text.find(";", start)
        if end > start:
            builders[match.group(1)] = text[start:end].strip()
    return builders


def extract_java_method(text: str, method_names: tuple[str, ...]) -> tuple[str, str] | tuple[None, None]:
    """Extract one complete Java method, including its signature and body."""
    for name in method_names:
        pattern = re.compile(
            rf"(?m)^[ \t]*(?:(?:public|protected|private|static|final|synchronized|abstract|native)\s+)*"
            rf"[\w<>\[\],.? ]+\b{name}\s*\([^;{{]*\)\s*(?:throws\s+[^{{]+)?\{{"
        )
        match = pattern.search(text)
        if not match:
            continue
        brace_index = text.find("{", match.start(), match.end())
        try:
            _, end = extract_balanced(text, brace_index, "{", "}")
        except ConversionError:
            continue
        return text[match.start():end].strip(), name
    return None, None


def extract_modern_animation(text: str, class_name: str) -> tuple[dict[str, object], str, str, list[str]]:
    """Preserve modern setupAnim/setAngles and renderToBuffer Java as a sidecar.

    Modern Mojmap entity models often split their runtime behavior across a
    declarative mesh builder and procedural animation/render methods. We do not
    attempt to symbolize or bake arbitrary code into Blockbench keyframes yet, but
    we do preserve the complete Java methods so the animation can be recovered in
    the target workspace instead of being discarded.
    """
    method_specs = (("setupAnim", "setAngles"), ("renderToBuffer", "render"))
    captured: list[tuple[str, str]] = []
    for names in method_specs:
        method, method_name = extract_java_method(text, names)
        if method and method_name:
            captured.append((method_name, method))

    if not captured:
        return {}, "", "", []

    logic_lines = [
        f"// Extracted by MojModelBridge {VERSION} from {class_name}.",
        "// This preserves the source model's procedural animation/render logic.",
        "// Paste/adapt these methods into the target Blockbench/MCreator Java model.",
        "",
    ]
    for method_name, method in captured:
        logic_lines.append(f"// --- {method_name} ---")
        logic_lines.append(method)
        logic_lines.append("")
    logic = "\n".join(logic_lines).rstrip() + "\n"

    method_names = ", ".join(name for name, _ in captured)
    analysis = {
        "source_format": "modern_mojmap_java",
        "method": captured[0][0],
        "methods": [name for name, _ in captured],
        "mode": "preserve_java",
        "note": (
            "Procedural animation was extracted as Java (setupAnim/renderToBuffer). "
            "Arbitrary runtime expressions are not automatically baked into native Blockbench keyframes yet."
        ),
    }
    return analysis, logic, method_names, []


def extract_modern_render_instances(
    text: str,
    class_name: str,
    parts: list[Part],
    constants: dict[str, float],
) -> tuple[dict[str, list[dict[str, object]]], list[str]]:
    """Recover explicit render-loop multiplicity from modern Mojmap source.

    This is intentionally conservative: only loops that both (a) render a known
    part and (b) increment a local angle variable used in the render transform are
    expanded. The repeated render calls become real model instances, while any
    outer pose-stack animation remains available to the normal animation sampler.
    """
    method_text, method_name = extract_java_method(text, ("renderToBuffer", "render"))
    if not method_text:
        return {}, []

    brace_index = method_text.find("{")
    if brace_index < 0:
        return {}, []
    try:
        body, _ = extract_balanced(method_text, brace_index, "{", "}")
    except ConversionError:
        return {}, []

    part_names = {part.name for part in parts} | {part.variable for part in parts}
    canonical_parts: dict[str, str] = {}
    for part in parts:
        for alias in (part.name, part.variable):
            canonical_parts.setdefault(re.sub(r"[^a-z0-9]", "", alias.lower()), part.name)

    def resolve_render_part(name: str) -> Optional[str]:
        if name in part_names:
            return name
        return canonical_parts.get(re.sub(r"[^a-z0-9]", "", str(name).lower()))
    loop_pattern = re.compile(r"for\s*\((.*?)\)\s*\{", re.S)
    render_instances: dict[str, list[dict[str, object]]] = {}
    warnings: list[str] = []
    pos = 0
    while True:
        match = loop_pattern.search(body, pos)
        if not match:
            break
        open_index = body.find("{", match.start(), match.end())
        if open_index < 0:
            break
        try:
            loop_body, end = extract_balanced(body, open_index, "{", "}")
        except ConversionError:
            pos = match.end()
            continue
        pos = end

        header = match.group(1)
        header_parts = [piece.strip() for piece in header.split(";")]
        count: Optional[int] = None
        if len(header_parts) >= 2:
            condition = header_parts[1]
            m_le = re.search(r"<=\s*([^;]+)$", condition)
            m_lt = re.search(r"<\s*([^;]+)$", condition)
            try:
                if m_le:
                    count = eval_int(m_le.group(1), constants) + 1
                elif m_lt:
                    count = eval_int(m_lt.group(1), constants)
            except ConversionError:
                count = None
        if count is None or count < 2:
            continue

        rendered_parts = sorted({
            resolved for raw_name in re.findall(r"(?:this\.)?([A-Za-z_$][\w$]*)\.render\s*\(", loop_body)
            for resolved in ([resolve_render_part(raw_name)] if resolve_render_part(raw_name) else [])
        })
        if not rendered_parts:
            continue

        increments = {
            name: expr.strip()
            for name, expr in re.findall(r"\b([A-Za-z_$][\w$]*)\s*\+=\s*([^;]+);", loop_body)
        }
        rotation_calls = list(re.finditer(r"Axis\.([XYZ])P\.rotation(Degrees)?\(\s*([A-Za-z_$][\w$]*)\s*\)", loop_body))
        direct_rotations = list(re.finditer(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.(xRot|yRot|zRot)\s*=\s*([^;]+);", loop_body))

        chosen_axis: Optional[str] = None
        chosen_var: Optional[str] = None
        chosen_degrees = False
        chosen_step: Optional[float] = None
        # PoseStack loops such as Rotator use Axis.*.rotation(newangle). Other
        # models (PurplePower) rotate the repeated part directly via `part.zRot =
        # newangle`. Treat both as explicit render multiplicity.
        for call in rotation_calls:
            axis, degrees_token, angle_var = call.groups()
            if angle_var not in increments:
                continue
            try:
                step = eval_number(increments[angle_var], constants)
            except ConversionError:
                continue
            chosen_axis = axis
            chosen_var = angle_var
            chosen_degrees = bool(degrees_token)
            chosen_step = math.radians(step) if chosen_degrees else step
            break
        if chosen_axis is None:
            for call in direct_rotations:
                _part_name, field_name, expression = call.groups()
                for angle_var, increment_expr in increments.items():
                    normalized = re.sub(r"\b(?:this\.)?%s\b" % re.escape(angle_var), angle_var, expression.strip())
                    if re.fullmatch(re.escape(angle_var), normalized):
                        axis = {"xRot": "X", "yRot": "Y", "zRot": "Z"}[field_name]
                        try:
                            step = eval_number(increment_expr, constants)
                        except ConversionError:
                            continue
                        chosen_axis = axis
                        chosen_var = angle_var
                        chosen_degrees = False
                        chosen_step = step
                        break
                if chosen_axis is not None:
                    break

        axis_index = {"X": 0, "Y": 1, "Z": 2}.get(chosen_axis or "Z", 2)
        step_value = float(chosen_step) if chosen_step is not None else 0.0
        for part_name in rendered_parts:
            if part_name not in part_names:
                continue
            instances = render_instances.setdefault(part_name, [])
            if instances and len(instances) != count:
                warnings.append(
                    f"Render multiplicity for '{part_name}' in {class_name} was encountered more than once; "
                    "instance copies were appended in source order."
                )
            for i in range(count):
                extra = [0.0, 0.0, 0.0]
                if chosen_axis is not None and chosen_step is not None:
                    extra[axis_index] = float(i) * step_value
                runtime_transforms = [
                    {"part": m.group(1), "axis": m.group(2), "expression": m.group(3).strip()}
                    for m in direct_rotations if m.group(1) in rendered_parts
                ]
                part_assignments = [
                    {"part": m.group(1), "axis": m.group(2), "expression": m.group(3).strip()}
                    for m in direct_rotations
                    if m.group(1) in rendered_parts
                ]
                position_assignments = [
                    {"part": m.group(1), "axis": m.group(2), "expression": m.group(3).strip()}
                    for m in re.finditer(
                        r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\.(x|y|z)\s*=\s*([^;]+);", loop_body
                    )
                    if m.group(1) in rendered_parts
                ]
                instances.append({
                    "rotation": tuple(extra),
                    "source_method": method_name or "renderToBuffer",
                    "instance_index": i,
                    "instance_count": count,
                    "runtime_transforms": runtime_transforms,
                    "part_assignments": part_assignments,
                    "position_assignments": position_assignments,
                })

    return render_instances, warnings


def extract_modern_helper_render_instances(
    text: str,
    parts: list[Part],
    constants: dict[str, float],
) -> dict[str, list[dict[str, object]]]:
    """Recover repeated renders performed through local helper methods.

    Clean source models such as ModelCrab render a three-part leg through a helper
    and call that helper eight times with different absolute x/z positions. Those
    calls are real geometry instances, but they are invisible to the simpler
    render-loop detector because the loop lives in a helper method.
    """
    part_names = {part.name for part in parts} | {part.variable for part in parts}
    defs: dict[str, tuple[list[str], str]] = {}
    method_pat = re.compile(
        r"(?:public|protected|private)\s+(?:static\s+)?(?:final\s+)?[A-Za-z_$][\w.$<>?,\[\] ]*\s+([A-Za-z_$][\w$]*)\s*\(([^)]*)\)\s*\{"
    )
    pos = 0
    while True:
        m = method_pat.search(text, pos)
        if not m:
            break
        try:
            body, end = extract_balanced(text, text.find("{", m.start(), m.end()), "{", "}")
        except Exception:
            pos = m.end(); continue
        params=[]
        for raw in split_top_level(m.group(2)):
            pm=re.search(r"([A-Za-z_$][\w$]*)\s*$", raw.strip())
            params.append(pm.group(1) if pm else "")
        defs[m.group(1)] = (params, body)
        pos=end
    if not defs:
        return {}
    out: dict[str,list[dict[str,object]]] = {}
    known={p.name for p in parts}
    for outer_name,(outer_params,outer_body) in defs.items():
        calls=list(re.finditer(r"(?:this\.)?([A-Za-z_$][\w$]*)\s*\(([^;{}]*)\);", outer_body))
        grouped: dict[str,list[re.Match]]={}
        for call in calls:
            callee=call.group(1)
            if callee in defs and callee != outer_name and callee in defs:
                nested_params,nested_body=defs[callee]
                if re.search(r"(?:this\.)?[A-Za-z_$][\w$]*\.render\s*\(", nested_body):
                    grouped.setdefault(callee,[]).append(call)
        for callee, group in grouped.items():
            if len(group)<2:
                continue
            params,nested_body=defs[callee]
            rendered={m.group(1) for m in re.finditer(r"(?:this\.)?([A-Za-z_$][\w$]*)\.render\s*\(", nested_body) if m.group(1) in known}
            if not rendered:
                continue
            for idx,call in enumerate(group):
                args=split_top_level(call.group(2))
                mapping={params[i]:args[i].strip() for i in range(min(len(params),len(args)))}
                assignments: dict[str,dict[str,float]]={}
                for m in re.finditer(r"(?:this\.)?([A-Za-z_$][\w$]*)\.(x|y|z)\s*=\s*([^;]+);", nested_body):
                    pn,axis,expr=m.groups()
                    if pn not in rendered:
                        continue
                    substituted=expr
                    for name,val in sorted(mapping.items(), key=lambda kv:-len(kv[0])):
                        substituted=re.sub(rf"\b{re.escape(name)}\b", val, substituted)
                    try:
                        value=eval_number(substituted, constants)
                    except Exception:
                        continue
                    assignments.setdefault(pn,{})[axis]=float(value)
                if not assignments:
                    continue
                for pn in rendered:
                    spec={
                        "instance_index": idx,
                        "instance_count": len(group),
                        "source_method": outer_name,
                        "helper_method": callee,
                    }
                    posv=assignments.get(pn)
                    if posv:
                        spec["position"]=(posv.get("x"),posv.get("y"),posv.get("z"))
                    out.setdefault(pn,[]).append(spec)
    return out


def extract_modern_render_runtime_rotations(
    text: str,
    render_instances: dict[str,list[dict[str,object]]],
    constants: dict[str,float],
) -> list[dict[str,object]]:
    """Recover global PoseStack rotations that drive repeated render groups."""
    method_text,_=extract_java_method(text,("renderToBuffer","render"))
    if not method_text: return []
    bi=method_text.find("{")
    try: body,_=extract_balanced(method_text,bi,"{","}")
    except Exception: return []
    increments={n:e.strip() for n,e in re.findall(r"\b([A-Za-z_$][\w$]*)\s*\+=\s*([^;]+);",body)}
    if not increments: return []
    result=[]
    for m in re.finditer(r"Axis\.([XYZ])P\.rotation(Degrees)?\(\s*([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*)\s*\)",body):
        axis,degrees,var=m.groups(); var=var.split('.')[-1]
        if var not in increments: continue
        try: step=eval_number(increments[var],constants)
        except Exception: continue
        after=body[m.end():]
        loop=re.search(r"for\s*\([^)]*\)\s*\{",after,re.DOTALL)
        if not loop: continue
        l_open=after.find('{',loop.start(),loop.end())
        try: loop_body,l_end=extract_balanced(after,l_open,'{','}')
        except Exception: continue
        # Only a counter update after the render loop is a persistent animation
        # state (Rotator.rf1 += 2). Inner-loop counters such as newangle += 45°
        # describe static instance spacing and must not become runtime rotation.
        post_loop=after[l_end:]
        if len(re.findall(rf"\b{re.escape(var)}\s*\+=", body)) != 1:
            continue
        if not re.search(rf"\b{re.escape(var)}\s*\+=",post_loop):
            continue
        rendered={x for x in re.findall(r"(?:this\.)?([A-Za-z_$][\w$]*)\.render\s*\(",loop_body) if x in render_instances}
        if rendered:
            result.append({"axis":axis,"degrees":bool(degrees),"speed":float(step),"parts":sorted(rendered),"variable":var})
    return result

def parse_java_model(text: str, source_name: str = "model.java", constructor_args: list[object] | None = None) -> ParsedModel:
    clean = strip_java_comments(text)
    warnings: list[str] = []
    class_match = re.search(r"\b(?:public\s+)?(?:final\s+)?class\s+([A-Za-z_$][\w$]*)", clean)
    class_name = class_match.group(1) if class_match else Path(source_name).stem
    if _looks_like_legacy_java_model(clean, class_name):
        return parse_legacy_java_model(text, source_name)
    constants = find_numeric_constants(clean)
    # Bind renderer-supplied constructor arguments to source parameter names.
    # Modern models commonly carry animation scalars such as wingspeed through
    # their constructor, while the decompiled source only stores the parameter
    # name in the field assignment.
    ctor_args = list(constructor_args or [])
    if ctor_args:
        ctor_match = re.search(r"\bpublic\s+" + re.escape(class_name) + r"\s*\(([^)]*)\)", clean)
        if ctor_match:
            params = []
            for raw_param in split_top_level(ctor_match.group(1)):
                m = re.search(r"([A-Za-z_$][\w$]*)\s*$", raw_param.strip())
                params.append(m.group(1) if m else "")
            for idx, value in enumerate(ctor_args):
                if idx >= len(params) or not params[idx]:
                    continue
                try:
                    constants[params[idx]] = eval_number(str(value), constants)
                except Exception:
                    try:
                        constants[params[idx]] = float(value)
                    except Exception:
                        continue

    texture_width, texture_height = 64, 32
    layer_matches = list(re.finditer(r"LayerDefinition\s*\.\s*create\s*\(", clean))
    if layer_matches:
        for match in layer_matches:
            try:
                raw, _ = extract_balanced(clean, match.end() - 1)
                args = split_top_level(raw)
                if len(args) >= 3:
                    texture_width = eval_int(args[-2], constants)
                    texture_height = eval_int(args[-1], constants)
                    break
            except ConversionError:
                continue
    else:
        tex_w = re.search(r"\btexWidth\s*=\s*([^;]+);", clean)
        tex_h = re.search(r"\btexHeight\s*=\s*([^;]+);", clean)
        if tex_w and tex_h:
            try:
                texture_width = eval_int(tex_w.group(1), constants)
                texture_height = eval_int(tex_h.group(1), constants)
            except ConversionError:
                pass
        else:
            warnings.append("Texture resolution was not found; defaulted to 64 x 32.")

    root_variables = set(re.findall(r"PartDefinition\s+([A-Za-z_$][\w$]*)\s*=\s*[A-Za-z_$][\w$]*\s*\.\s*getRoot\s*\(\s*\)", clean))
    root_variables.update(re.findall(r"ModelPartData\s+([A-Za-z_$][\w$]*)\s*=.*?getRoot\s*\(\s*\)", clean))
    root_variables.update({"root", "partdefinition", "modelPartData"})
    builder_vars = find_builder_variables(clean)

    call_pattern = re.compile(r"([A-Za-z_$][\w$]*)\s*\.\s*(?:addOrReplaceChild|addChild)\s*\(")
    parts: list[Part] = []
    variable_to_part: dict[str, Part] = {}
    pos = 0
    index = 0
    while True:
        match = call_pattern.search(clean, pos)
        if not match:
            break
        parent_variable = match.group(1)
        open_index = match.end() - 1
        try:
            raw_args, end = extract_balanced(clean, open_index)
        except ConversionError as exc:
            warnings.append(str(exc))
            break
        pos = end
        args = split_top_level(raw_args)
        if len(args) < 3:
            warnings.append(f"Skipped child call near character {match.start()}: expected 3 arguments.")
            continue

        # Detect the variable receiving addOrReplaceChild's return value.
        statement_start = max(clean.rfind(";", 0, match.start()), clean.rfind("{", 0, match.start()), clean.rfind("}", 0, match.start())) + 1
        prefix = clean[statement_start : match.start()]
        assignment = re.search(r"(?:PartDefinition|ModelPartData)?\s*([A-Za-z_$][\w$]*)\s*=\s*$", prefix)
        variable = assignment.group(1) if assignment else f"__part_{index}"

        name = unquote_java_string(args[0])
        builder = args[1].strip()
        if builder in builder_vars:
            builder = builder_vars[builder]
        offset, rotation = parse_pose(args[2], constants, warnings, name)
        cubes = parse_builder(builder, constants, warnings, name)
        part = Part(
            name=name,
            variable=variable,
            parent_variable=parent_variable,
            offset=offset,
            rotation=rotation,
            cubes=cubes,
            source_index=index,
        )
        index += 1
        parts.append(part)
        variable_to_part[variable] = part

    if not parts:
        raise ConversionError(
            "No modern model parts were found. Expected PartDefinition.addOrReplaceChild(...) "
            "or ModelPartData.addChild(...) geometry."
        )

    for part in parts:
        parent = variable_to_part.get(part.parent_variable)
        if parent is not None and parent is not part:
            part.parent = parent
            parent.children.append(part)
        elif part.parent_variable not in root_variables and not part.parent_variable.lower().endswith("root"):
            warnings.append(
                f"Parent variable '{part.parent_variable}' for '{part.name}' was not resolved; treated as a root part."
            )

    if sum(len(p.cubes) for p in parts) == 0:
        warnings.append("Parts were found, but no CubeListBuilder.addBox calls were converted.")

    animation_analysis, animation_logic, animation_source, animation_warnings = extract_modern_animation(text, class_name)
    warnings.extend(animation_warnings)

    # Modern Mojmap source uses the same mutable ModelPart fields as the legacy
    # ModelRenderer path. Reuse the proven formula sampler so setupAnim/renderToBuffer
    # assignments become actual Blockbench keyframes rather than only a Java sidecar.
    baked_analysis, baked_logic, baked_preview, baked_warnings = _extract_legacy_java_animation(
        text, class_name, parts
    )
    if baked_analysis:
        baked_analysis["source_has_open_closed"] = bool(re.search(r"\bgetOpenClosed\s*\(", clean))
        baked_analysis["source_has_activity"] = bool(re.search(r"\bgetActivity\s*\(", clean))
    warnings.extend(baked_warnings)
    if baked_analysis:
        # Keep the modern-method inventory for diagnostics, while storing the
        # sampled procedural analysis separately so callers can still see that
        # both setupAnim and renderToBuffer were found.
        animation_analysis = dict(animation_analysis)
        animation_analysis["baked_formula_analysis"] = baked_analysis
        animation_analysis["baked_formula_count"] = baked_analysis.get("formula_count", 0)
        animation_logic = animation_logic or baked_logic
        animation_source = animation_source or "setupAnim/renderToBuffer"
        baked_animation_json = baked_preview
    else:
        baked_animation_json = {}

    render_instances, render_warnings = extract_modern_render_instances(text, class_name, parts, constants)
    helper_instances = extract_modern_helper_render_instances(text, parts, constants)
    for name, specs in helper_instances.items():
        render_instances[name] = specs
    warnings.extend(render_warnings)
    render_runtime_rotations = extract_modern_render_runtime_rotations(text, render_instances, constants)

    # Many clean source models contain mutually-exclusive render branches (for
    # example an adult form vs. a flower form selected by getActivity()). The
    # MCreator model renderer renders every PartDefinition unconditionally, so
    # retaining both branches stacks them on top of each other. For a zero/default
    # activity branch, preserve that branch as the static model; the alternate
    # branch remains source evidence but is not duplicated into the target model.
    default_render_parts: set[str] = set()
    try:
        method_text, _method_name = extract_java_method(text, ("renderToBuffer", "render"))
        if method_text:
            bidx = method_text.find("{")
            body, _ = extract_balanced(method_text, bidx, "{", "}")
            for im in re.finditer(r"\bif\s*\(", body):
                op = body.find("(", im.start())
                try:
                    cond, cond_end = extract_balanced(body, op, "(", ")")
                except Exception:
                    continue
                if not re.search(r"getActivity\s*\(\s*\)\s*==\s*0", cond):
                    continue
                brace = body.find("{", cond_end)
                if brace < 0: continue
                try:
                    then_body, then_end = extract_balanced(body, brace, "{", "}")
                except Exception: continue
                em = re.match(r"\s*else\s*\{", body[then_end:])
                if not em: continue
                ebrace = then_end + em.end() - 1
                try:
                    else_body, _ = extract_balanced(body, ebrace, "{", "}")
                except Exception: continue
                rendered_names = lambda chunk: {m.group(1) for m in re.finditer(r"(?:this\.)?([A-Za-z_$][\w$]*)\.render\s*\(", chunk)}
                then_names, else_names = rendered_names(then_body), rendered_names(else_body)
                if then_names and else_names and not (then_names & else_names):
                    default_render_parts = then_names
                    break
    except Exception:
        default_render_parts = set()
    model = ParsedModel(class_name, texture_width, texture_height, parts, warnings)
    model.animation_analysis = animation_analysis
    model.animation_logic = animation_logic
    model.baked_animation_json = baked_animation_json
    model.animation_source = animation_source
    model.render_instances = render_instances
    model.default_render_parts = default_render_parts
    model.render_runtime_rotations = render_runtime_rotations
    model.constructor_args = list(constructor_args or [])
    model.source_format = "modern_mojmap_java"
    return model


# ------------------------------ BBModel writer ------------------------------

def clean_number(value: float) -> float | int:
    if abs(value) < 1e-10:
        return 0
    rounded = round(value)
    if abs(value - rounded) < 1e-9:
        return int(rounded)
    return round(value, 8)


def vec(values: Iterable[float]) -> list[float | int]:
    return [clean_number(v) for v in values]


def deterministic_uuid(model_name: str, kind: str, index: int, name: str) -> str:
    return str(uuid.uuid5(UUID_NAMESPACE, f"{model_name}:{kind}:{index}:{name}"))


def assign_origins(model: ParsedModel) -> None:
    visiting: set[int] = set()

    def visit(part: Part) -> None:
        key = id(part)
        if key in visiting:
            raise ConversionError(f"Cyclic model hierarchy involving '{part.name}'")
        visiting.add(key)
        ox, oy, oz = part.offset
        if part.parent is None:
            part.origin = (-ox, 24.0 - oy, oz)
        else:
            visit(part.parent)
            px, py, pz = part.parent.origin
            part.origin = (px - ox, py - oy, pz + oz)
        visiting.remove(key)

    for part in model.parts:
        visit(part)


def compute_model_bounds(model: ParsedModel) -> dict[str, object]:
    """Return a conservative Blockbench-space AABB and useful preview values.

    The legacy boss models can be hundreds of pixels wide.  Blockbench accepts
    that geometry, but a 1x1 visible box makes the project open at an unusable
    scale and can make the model look like a single dense block.  This bound is
    intentionally based on the same coordinate conversion used by the cubes.
    Group rotations are not expanded into the AABB, so the values are a preview
    hint rather than collision metadata.
    """
    assign_origins(model)
    mins = [math.inf, math.inf, math.inf]
    maxs = [-math.inf, -math.inf, -math.inf]
    for part in model.parts:
        gx, gy, gz = part.origin
        for cube in part.cubes:
            to_x = gx - cube.x
            from_x = to_x - cube.dx
            to_y = gy - cube.y
            from_y = to_y - cube.dy
            from_z = gz + cube.z
            to_z = from_z + cube.dz
            for axis, pair in enumerate(((from_x, to_x), (from_y, to_y), (from_z, to_z))):
                mins[axis] = min(mins[axis], *pair)
                maxs[axis] = max(maxs[axis], *pair)
    if not all(math.isfinite(v) for v in [*mins, *maxs]):
        mins = [-8.0, 0.0, -8.0]
        maxs = [8.0, 16.0, 8.0]
    sizes = [maxs[i] - mins[i] for i in range(3)]
    center = [(mins[i] + maxs[i]) / 2.0 for i in range(3)]
    width_blocks = max(1.0, sizes[0] / 16.0, sizes[2] / 16.0)
    height_blocks = max(1.0, sizes[1] / 16.0)
    # Blockbench's third visible_box value is a vertical preview offset.
    y_offset_blocks = (center[1] - 24.0) / 16.0
    result = {
        "min": [clean_number(v) for v in mins],
        "max": [clean_number(v) for v in maxs],
        "size": [clean_number(v) for v in sizes],
        "center": [clean_number(v) for v in center],
        "visible_box": [
            clean_number(width_blocks),
            clean_number(height_blocks),
            clean_number(y_offset_blocks),
        ],
    }
    model.model_bounds = result
    return result


def bb_faces(texture_index: Optional[int] = None) -> dict[str, dict[str, object]]:
    return {
        side: {"uv": [0, 0, 0, 0], "texture": texture_index}
        for side in ("north", "east", "south", "west", "up", "down")
    }


def _embedded_texture(texture_path: Optional[Path], texture_uuid: str, display_name: Optional[str] = None, selected: bool = False) -> Optional[dict[str, object]]:
    if texture_path is None:
        return None
    texture_path = texture_path.expanduser().resolve()
    if not texture_path.exists() or not texture_path.is_file():
        raise ConversionError(f"Texture file does not exist: {texture_path}")
    mime = mimetypes.guess_type(texture_path.name)[0] or "image/png"
    encoded = base64.b64encode(texture_path.read_bytes()).decode("ascii")
    name = display_name or texture_path.name
    return {
        "name": name,
        "folder": "",
        "namespace": "",
        "id": texture_uuid,
        "uuid": texture_uuid,
        "particle": False,
        "render_mode": "normal",
        "internal": True,
        "saved": True,
        "selected": selected,
        "path": texture_path.name,
        "source": f"data:{mime};base64,{encoded}",
        "moj_model_bridge_source": str(texture_path),
    }


def _prepare_embedded_textures(model: ParsedModel, texture_paths: Optional[Iterable[Path] | Path]) -> list[dict[str, object]]:
    if texture_paths is None:
        return []
    if isinstance(texture_paths, Path):
        paths = [texture_paths]
    else:
        paths = [Path(p) for p in texture_paths]
    unique: list[Path] = []
    seen: set[str] = set()
    for path in paths:
        resolved = path.expanduser().resolve()
        key = os.path.normcase(str(resolved))
        if key not in seen and resolved.is_file():
            seen.add(key)
            unique.append(resolved)
    names: dict[str, int] = {}
    embedded: list[dict[str, object]] = []
    for index, path in enumerate(unique):
        base = path.name
        names[base.lower()] = names.get(base.lower(), 0) + 1
    used: set[str] = set()
    for index, path in enumerate(unique):
        display = path.name
        if names[path.name.lower()] > 1:
            display = f"{path.parent.name}_{path.name}"
        original = display
        counter = 2
        while display.lower() in used:
            display = f"{Path(original).stem}_{counter}{Path(original).suffix}"
            counter += 1
        used.add(display.lower())
        texture_uuid = deterministic_uuid(model.class_name, "texture", index, str(path))
        item = _embedded_texture(path, texture_uuid, display_name=display, selected=index == 0)
        if item is not None:
            embedded.append(item)
    return embedded

def _blockbench_keyframe_uuid(
    model: ParsedModel,
    animation_name: str,
    bone: str,
    channel: str,
    time: float,
    part_uuid: str = "",
) -> str:
    return deterministic_uuid(
        model.class_name,
        f"keyframe:{animation_name}:{bone}:{channel}:{part_uuid}",
        int(round(time * 1000000)),
        f"{bone}:{channel}:{time:.8f}:{part_uuid}",
    )


def _expand_render_instance_parts(model: ParsedModel) -> list[Part]:
    """Duplicate parts that are explicitly rendered multiple times in source."""
    render_instances = model.render_instances or {}
    if not render_instances:
        return list(model.parts)

    expanded: list[Part] = []
    for part in model.parts:
        part.source_name = part.source_name or part.name
        specs = list(render_instances.get(part.name, []))
        if not specs:
            expanded.append(part)
            continue

        # The first source render is authoritative when helper methods overwrite
        # ModelPart.x/y/z (for example the eight Crab leg passes).
        first_spec = specs[0] if specs else {}
        if isinstance(first_spec, dict) and first_spec.get("position"):
            try:
                part.offset = tuple(float(v) for v in first_spec["position"])
            except Exception:
                pass
        if isinstance(first_spec, dict) and first_spec.get("rotation"):
            try:
                frx, fry, frz = first_spec["rotation"]
                part.rotation = (part.rotation[0] + float(frx), part.rotation[1] + float(fry), part.rotation[2] + float(frz))
            except Exception:
                pass
        expanded.append(part)
        for instance_index, spec in enumerate(specs[1:], start=1):
            rotation = spec.get("rotation", (0.0, 0.0, 0.0))
            try:
                rx, ry, rz = rotation
            except Exception:
                rx, ry, rz = (0.0, 0.0, 0.0)
            position = spec.get("position", part.offset)
            try:
                position = tuple(float(v) for v in position)
            except Exception:
                position = part.offset
            clone = Part(
                name=f"{part.name}_{instance_index + 1}",
                variable=f"{part.variable}_{instance_index + 1}",
                parent_variable=part.parent_variable,
                offset=position,
                rotation=(part.rotation[0] + float(rx), part.rotation[1] + float(ry), part.rotation[2] + float(rz)),
                cubes=[Cube(**cube.__dict__) for cube in part.cubes],
                parent=part.parent,
                children=list(part.children),
                source_index=part.source_index,
                source_name=part.source_name,
                render_instance_index=instance_index,
                render_instance_spec=dict(spec),
            )
            expanded.append(clone)
    return expanded


def bake_preview_into_blockbench(model: ParsedModel) -> list[dict[str, object]]:
    """Convert the portable preview bake into Blockbench-native animation JSON."""
    if not model.baked_animation_json:
        return []

    part_uuids_by_source: dict[str, list[str]] = {}
    part_name_by_uuid: dict[str, str] = {}
    for part in model.parts:
        aliases = {part.source_name or "", part.name or "", part.variable or ""}
        for alias in aliases:
            if alias:
                part_uuids_by_source.setdefault(alias, []).append(part.group_uuid)
        part_name_by_uuid[part.group_uuid] = part.name
    animations: list[dict[str, object]] = []
    raw_animations = model.baked_animation_json.get("animations", {})
    if not isinstance(raw_animations, dict):
        return []

    for animation_index, (name, raw_animation) in enumerate(raw_animations.items()):
        if not isinstance(raw_animation, dict):
            continue
        raw_bones = raw_animation.get("bones", {})
        if not isinstance(raw_bones, dict):
            continue
        animators: dict[str, object] = {}
        max_time = 0.0
        missing_bones: list[str] = []

        for bone_name, bone_data in raw_bones.items():
            matching_uuids = part_uuids_by_source.get(str(bone_name), [])
            if not matching_uuids or not isinstance(bone_data, dict):
                missing_bones.append(str(bone_name))
                continue
            bone_keyframes: list[dict[str, object]] = []
            for channel_name in ("rotation", "position"):
                channels = bone_data.get(channel_name, {})
                if not isinstance(channels, dict):
                    continue
                for time_key, values in sorted(channels.items(), key=lambda item: float(item[0])):
                    try:
                        time = float(time_key)
                        values_list = list(values)
                        vector = [float(values_list[i]) if i < len(values_list) else 0.0 for i in range(3)]
                    except (TypeError, ValueError):
                        continue
                    max_time = max(max_time, time)
                    bone_keyframes.append({
                        "channel": channel_name,
                        "data_points": [{
                            "x": clean_number(vector[0]),
                            "y": clean_number(vector[1]),
                            "z": clean_number(vector[2]),
                        }],
                        "time": clean_number(time),
                        "color": -1,
                        "interpolation": "linear",
                        "bezier_linked": True,
                        "bezier_left_time": [-0.1, -0.1, -0.1],
                        "bezier_left_value": [0, 0, 0],
                        "bezier_right_time": [0.1, 0.1, 0.1],
                        "bezier_right_value": [0, 0, 0],
                    })
            if bone_keyframes:
                for part_uuid in matching_uuids:
                    animator_keyframes = []
                    for keyframe in bone_keyframes:
                        animator_keyframes.append({
                            **keyframe,
                            "uuid": _blockbench_keyframe_uuid(
                                model,
                                name,
                                str(bone_name),
                                str(keyframe["channel"]),
                                float(keyframe["time"]),
                                part_uuid,
                            ),
                        })
                    animators[part_uuid] = {
                        "name": part_name_by_uuid.get(part_uuid, str(bone_name)),
                        "type": "bone",
                        "keyframes": animator_keyframes,
                    }

        if not animators:
            continue

        animation = {
            "name": str(name),
            "loop": "loop" if raw_animation.get("loop") else "once",
            "override": False,
            "length": clean_number(max_time),
            "snapping": 24,
            "selected": animation_index == 0,
            "anim_time_update": "",
            "blend_weight": "",
            "start_delay": 0,
            "loop_delay": 0,
            "animators": animators,
        }
        if missing_bones:
            animation["moj_model_bridge_missing_bones"] = missing_bones
        animations.append(animation)

    return animations


def model_to_bbmodel(
    model: ParsedModel,
    source_file: Optional[str] = None,
    texture_paths: Optional[Iterable[Path] | Path] = None,
) -> dict[str, object]:
    original_parts = model.parts
    model.parts = _expand_render_instance_parts(model)
    try:
        assign_origins(model)
        bounds = compute_model_bounds(model)
        elements: list[dict[str, object]] = []
        groups: list[dict[str, object]] = []
        children_by_part: dict[int, list[object]] = {id(p): [] for p in model.parts}
        embedded_textures = _prepare_embedded_textures(model, texture_paths)

        for i, part in enumerate(model.parts):
            part.group_uuid = deterministic_uuid(model.class_name, "group", i, part.name)
            rx, ry, rz = part.rotation
            rotation_degrees = (-math.degrees(rx), -math.degrees(ry), math.degrees(rz))
            group: dict[str, object] = {
                "uuid": part.group_uuid,
                "export": True,
                "locked": False,
                "origin": vec(part.origin),
                "rotation": vec(rotation_degrees),
                "color": i % 8,
                "name": part.name,
                "children": [],
                "reset": False,
                "shade": True,
                "mirror_uv": False,
                "selected": False,
                "visibility": True,
                "autouv": 0,
                "isOpen": True,
                "primary_selected": False,
            }
            groups.append(group)

            gx, gy, gz = part.origin
            for cube_index, cube in enumerate(part.cubes):
                # Exact inverse of Blockbench's flip-Y Mojmaps exporter.
                to_x = gx - cube.x
                from_x = to_x - cube.dx
                to_y = gy - cube.y
                from_y = to_y - cube.dy
                from_z = gz + cube.z
                to_z = from_z + cube.dz
                cube_uuid = deterministic_uuid(model.class_name, f"cube:{i}", cube_index, cube.name)
                element: dict[str, object] = {
                    "name": cube.name,
                    "box_uv": True,
                    "rescale": False,
                    "locked": False,
                    "light_emission": 0,
                    "render_order": "default",
                    "allow_mirror_modeling": True,
                    "from": vec((from_x, from_y, from_z)),
                    "to": vec((to_x, to_y, to_z)),
                    "autouv": 0,
                    "color": i % 8,
                    "origin": vec(part.origin),
                    "uv_offset": [cube.uv_x, cube.uv_y],
                    "mirror_uv": cube.mirror,
                    "inflate": clean_number(cube.inflate),
                    "faces": bb_faces(0 if embedded_textures else None),
                    "type": "cube",
                    "uuid": cube_uuid,
                }
                elements.append(element)
                children_by_part[id(part)].append(cube_uuid)

        def outliner_node(part: Part) -> dict[str, object]:
            children: list[object] = list(children_by_part[id(part)])
            children.extend(outliner_node(child) for child in sorted(part.children, key=lambda p: p.source_index))
            return {"uuid": part.group_uuid, "isOpen": True, "children": children}

        roots = [p for p in model.parts if p.parent is None]
        outliner = [outliner_node(p) for p in sorted(roots, key=lambda p: p.source_index)]
        metadata: dict[str, object] = {
            "format_version": "5.0",
            "model_format": "modded_entity",
            "box_uv": True,
        }
        native_animations = model.blockbench_animations or bake_preview_into_blockbench(model)
        result: dict[str, object] = {
            "meta": metadata,
            "name": model.class_name,
            "model_identifier": model.class_name,
            "geometry_name": model.class_name,
            "modded_entity_entity_class": "Entity",
            "modded_entity_flip_y": True,
            "visible_box": bounds["visible_box"],
            "variable_placeholders": "",
            "variable_placeholder_buttons": [],
            "timeline_setups": [],
            "unhandled_root_fields": {},
            "resolution": {"width": model.texture_width, "height": model.texture_height},
            "elements": elements,
            "groups": groups,
            "outliner": outliner,
            "textures": embedded_textures,
            "animations": native_animations,
        }
        bridge_fields: dict[str, object] = {
            "version": VERSION,
            "render_instance_sources": {
                part.name: dict(getattr(part, "render_instance_spec", {}) or {})
                for part in model.parts
                if getattr(part, "render_instance_index", 0)
                and getattr(part, "render_instance_spec", {})
            },
            "source_file": source_file,
            "source_format": model.source_format,
            "primary_texture": model.primary_texture or None,
            "texture_variants": list(model.texture_variants),
            "auxiliary_textures_skipped": list(model.auxiliary_textures),
            "variant_model_outputs": list(model.variant_model_outputs),
            "constructor_args": list(model.constructor_args),
            "scalar_fields": dict(model.scalar_fields),
            "model_bounds": bounds,
            "linked_entities": list(model.linked_entities),
            "linked_renderers": list(model.linked_renderers),
        }
        if model.animation_analysis:
            bridge_fields.update({
                "animation_source": model.animation_source,
                "animation_artifacts": {
                    "analysis": f"{model.class_name}_animation_analysis.json",
                    "runtime_logic": f"{model.class_name}_animation.java",
                    "baked_preview": f"{model.class_name}_baked_animations.json",
                },
                "notes": (
                    "Runtime animation logic is exported as Java. Preview animations are embedded into the native "
                    "Blockbench animation list when they can be baked. Modern source position keyframes are rebased "
                    "to the converted bind pose, and explicit render-loop multiplicity is materialized as additional "
                    "model instances when the source renders the same part multiple times."
                ),
            })
        else:
            bridge_fields["notes"] = "Geometry converted from model source; no procedural animation was embedded."
        if model.animation_resource:
            bridge_fields["external_animation_resource"] = model.animation_resource
            bridge_fields["external_animation_resource_path"] = model.animation_resource_path or None
            bridge_fields["notes_external_animation"] = "This model delegates animation to an external JSON resource through a custom animation framework. The resource reference is preserved; full external-animation baking requires the JSON resource to be available and supported by the importer."
        result["unhandled_root_fields"] = {"moj_model_bridge": bridge_fields}
        return result
    finally:
        model.parts = original_parts


def load_cached_bbmodel(path: Path) -> ParsedModel:
    """Reconstruct a :class:`ParsedModel` from MojModelBridge's .bbmodel output.

    Full-port runs can spend most of their time recovering large procedural
    legacy models.  A completed ``.bbmodel`` already contains the converted
    geometry, hierarchy, pivots, bind rotations, native animations, and bridge
    metadata, while the adjacent sidecars preserve the sampled animation data
    used by the MCreator backend.  Loading that output makes interrupted runs
    resumable instead of parsing the same bytecode again.

    This intentionally accepts only the subset emitted by ``model_to_bbmodel``;
    arbitrary hand-edited Blockbench projects may still load, but are not
    guaranteed to reconstruct source-space offsets losslessly.
    """
    path = Path(path)
    try:
        data = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ConversionError(f"Could not load cached Blockbench model {path}: {exc}") from exc
    if not isinstance(data, dict):
        raise ConversionError(f"Cached Blockbench model is not a JSON object: {path}")

    def number(value: object, default: float = 0.0) -> float:
        try:
            return float(value)
        except (TypeError, ValueError):
            return default

    def vector(value: object, size: int = 3, default: float = 0.0) -> tuple[float, ...]:
        values = list(value) if isinstance(value, (list, tuple)) else []
        return tuple(number(values[i], default) if i < len(values) else default for i in range(size))

    resolution = data.get("resolution", {})
    if not isinstance(resolution, dict):
        resolution = {}
    class_name = str(data.get("model_identifier") or data.get("name") or path.stem)
    texture_width = max(1, int(number(resolution.get("width"), 64)))
    texture_height = max(1, int(number(resolution.get("height"), 32)))

    group_rows = data.get("groups", [])
    if not isinstance(group_rows, list):
        group_rows = []
    parts: list[Part] = []
    parts_by_uuid: dict[str, Part] = {}
    used_variables: set[str] = set()
    for index, row in enumerate(group_rows):
        if not isinstance(row, dict):
            continue
        group_uuid = str(row.get("uuid") or deterministic_uuid(class_name, "cached-group", index, str(row.get("name") or "part")))
        name = str(row.get("name") or f"part{index + 1}")
        variable = re.sub(r"[^A-Za-z0-9_$]", "_", name) or f"part{index + 1}"
        if variable[0].isdigit():
            variable = "part_" + variable
        base_variable = variable
        suffix = 2
        while variable.lower() in used_variables:
            variable = f"{base_variable}_{suffix}"
            suffix += 1
        used_variables.add(variable.lower())
        origin = vector(row.get("origin"))
        bb_rotation = vector(row.get("rotation"))
        part = Part(
            name=name,
            variable=variable,
            parent_variable="",
            rotation=(
                -math.radians(bb_rotation[0]),
                -math.radians(bb_rotation[1]),
                math.radians(bb_rotation[2]),
            ),
            source_index=index,
            origin=(origin[0], origin[1], origin[2]),
            group_uuid=group_uuid,
        )
        parts.append(part)
        parts_by_uuid[group_uuid] = part

    element_rows = data.get("elements", [])
    if not isinstance(element_rows, list):
        element_rows = []
    elements_by_uuid = {
        str(row.get("uuid")): row
        for row in element_rows
        if isinstance(row, dict) and row.get("uuid")
    }
    assigned_elements: set[str] = set()
    ordered_parts: list[Part] = []

    def add_cube(part: Part, element_uuid: str) -> None:
        row = elements_by_uuid.get(element_uuid)
        if row is None or element_uuid in assigned_elements:
            return
        assigned_elements.add(element_uuid)
        from_xyz = vector(row.get("from"))
        to_xyz = vector(row.get("to"))
        gx, gy, gz = part.origin
        uv = vector(row.get("uv_offset"), size=2)
        part.cubes.append(Cube(
            x=gx - to_xyz[0],
            y=gy - to_xyz[1],
            z=from_xyz[2] - gz,
            dx=to_xyz[0] - from_xyz[0],
            dy=to_xyz[1] - from_xyz[1],
            dz=to_xyz[2] - from_xyz[2],
            uv_x=int(round(uv[0])),
            uv_y=int(round(uv[1])),
            mirror=bool(row.get("mirror_uv", False)),
            inflate=number(row.get("inflate")),
            name=str(row.get("name") or f"cube{len(part.cubes) + 1}"),
        ))

    visiting: set[str] = set()

    def visit_node(node: object, parent: Optional[Part] = None) -> None:
        if not isinstance(node, dict):
            return
        group_uuid = str(node.get("uuid") or "")
        part = parts_by_uuid.get(group_uuid)
        if part is None or group_uuid in visiting:
            return
        visiting.add(group_uuid)
        if part not in ordered_parts:
            part.source_index = len(ordered_parts)
            ordered_parts.append(part)
        if parent is not None and part.parent is None and parent is not part:
            part.parent = parent
            part.parent_variable = parent.variable
            if part not in parent.children:
                parent.children.append(part)
        children = node.get("children", [])
        if isinstance(children, list):
            for child in children:
                if isinstance(child, str):
                    add_cube(part, child)
                elif isinstance(child, dict):
                    visit_node(child, part)
        visiting.remove(group_uuid)

    outliner = data.get("outliner", [])
    if isinstance(outliner, list):
        for node in outliner:
            visit_node(node)

    # Keep malformed/edited projects useful: preserve groups omitted from the
    # outliner and associate orphan cubes by their recorded group origin.
    for part in parts:
        if part not in ordered_parts:
            part.source_index = len(ordered_parts)
            ordered_parts.append(part)
    for element_uuid, row in elements_by_uuid.items():
        if element_uuid in assigned_elements or not parts:
            continue
        element_origin = vector(row.get("origin"))
        part = min(
            parts,
            key=lambda candidate: sum((candidate.origin[i] - element_origin[i]) ** 2 for i in range(3)),
        )
        add_cube(part, element_uuid)

    # Invert assign_origins() to recover the source model's rotation-point
    # offsets.  These offsets are what model_to_mojmap_java emits later.
    for part in ordered_parts:
        ox, oy, oz = part.origin
        if part.parent is None:
            part.offset = (-ox, 24.0 - oy, oz)
        else:
            px, py, pz = part.parent.origin
            part.offset = (px - ox, py - oy, oz - pz)

    model = ParsedModel(class_name, texture_width, texture_height, ordered_parts)
    model.blockbench_animations = list(data.get("animations", [])) if isinstance(data.get("animations"), list) else []

    bridge_root = data.get("unhandled_root_fields", {})
    bridge = bridge_root.get("moj_model_bridge", {}) if isinstance(bridge_root, dict) else {}
    if not isinstance(bridge, dict):
        bridge = {}
    model.source_format = str(bridge.get("source_format") or "cached_bbmodel")
    model.primary_texture = str(bridge.get("primary_texture") or "")
    model.texture_variants = [str(value) for value in bridge.get("texture_variants", [])] if isinstance(bridge.get("texture_variants"), list) else []
    model.auxiliary_textures = [str(value) for value in bridge.get("auxiliary_textures_skipped", [])] if isinstance(bridge.get("auxiliary_textures_skipped"), list) else []
    model.variant_model_outputs = [str(value) for value in bridge.get("variant_model_outputs", [])] if isinstance(bridge.get("variant_model_outputs"), list) else []
    model.constructor_args = list(bridge.get("constructor_args", [])) if isinstance(bridge.get("constructor_args"), list) else []
    model.scalar_fields = {
        str(key): number(value)
        for key, value in bridge.get("scalar_fields", {}).items()
    } if isinstance(bridge.get("scalar_fields"), dict) else {}
    model.model_bounds = dict(bridge.get("model_bounds", {})) if isinstance(bridge.get("model_bounds"), dict) else {}
    model.linked_entities = [str(value) for value in bridge.get("linked_entities", [])] if isinstance(bridge.get("linked_entities"), list) else []
    model.linked_renderers = [str(value) for value in bridge.get("linked_renderers", [])] if isinstance(bridge.get("linked_renderers"), list) else []
    model.animation_source = str(bridge.get("animation_source") or "")
    model.animation_resource = str(bridge.get("external_animation_resource") or "")
    model.animation_resource_path = str(bridge.get("external_animation_resource_path") or "")

    stem = path.with_suffix("")
    analysis_path = stem.with_name(stem.name + "_animation_analysis.json")
    logic_path = stem.with_name(stem.name + "_animation.java")
    baked_path = stem.with_name(stem.name + "_baked_animations.json")
    try:
        if analysis_path.exists():
            analysis = json.loads(analysis_path.read_text(encoding="utf-8-sig"))
            if isinstance(analysis, dict):
                model.animation_analysis = analysis
    except (OSError, json.JSONDecodeError) as exc:
        model.warnings.append(f"Cached animation analysis could not be loaded: {exc}")
    try:
        if logic_path.exists():
            model.animation_logic = logic_path.read_text(encoding="utf-8-sig")
    except OSError as exc:
        model.warnings.append(f"Cached runtime animation Java could not be loaded: {exc}")
    try:
        if baked_path.exists():
            baked = json.loads(baked_path.read_text(encoding="utf-8-sig"))
            if isinstance(baked, dict):
                model.baked_animation_json = baked
    except (OSError, json.JSONDecodeError) as exc:
        model.warnings.append(f"Cached baked animation data could not be loaded: {exc}")
    model.warnings.append(f"Resumed from cached Blockbench conversion: {path.name}")
    return model


def load_cached_bbmodel_summary(path: Path) -> ParsedModel:
    """Load a lightweight report-only summary of a cached ``.bbmodel``.

    Large procedural models can contain megabytes of sampled keyframes. During a
    resumed full-port, models whose living-entity elements already exist do not
    need to be reconstructed in memory again. This loader keeps counts and bridge
    metadata while deliberately omitting geometry/keyframe payloads. It must not
    be supplied to a code generator that needs cubes; callers only use it for
    conversion reports and CLI summaries.
    """
    path = Path(path)
    try:
        data = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ConversionError(f"Could not load cached Blockbench summary {path}: {exc}") from exc
    if not isinstance(data, dict):
        raise ConversionError(f"Cached Blockbench model is not a JSON object: {path}")

    resolution = data.get("resolution", {}) if isinstance(data.get("resolution"), dict) else {}
    try:
        texture_width = max(1, int(float(resolution.get("width", 64))))
    except (TypeError, ValueError):
        texture_width = 64
    try:
        texture_height = max(1, int(float(resolution.get("height", 32))))
    except (TypeError, ValueError):
        texture_height = 32
    class_name = str(data.get("model_identifier") or data.get("name") or path.stem)

    groups = data.get("groups", []) if isinstance(data.get("groups"), list) else []
    parts = [
        Part(
            name=str(row.get("name") or f"part{index + 1}"),
            variable=f"cached_part_{index + 1}",
            parent_variable="",
            source_index=index,
        )
        for index, row in enumerate(groups)
        if isinstance(row, dict)
    ]
    elements = data.get("elements", []) if isinstance(data.get("elements"), list) else []
    model = ParsedModel(class_name, texture_width, texture_height, parts, cached_cube_count=len(elements))

    animations = data.get("animations", []) if isinstance(data.get("animations"), list) else []
    model.blockbench_animations = [
        {"name": str(row.get("name") or f"animation{index + 1}")}
        for index, row in enumerate(animations)
        if isinstance(row, dict)
    ]
    bridge_root = data.get("unhandled_root_fields", {})
    bridge = bridge_root.get("moj_model_bridge", {}) if isinstance(bridge_root, dict) else {}
    if not isinstance(bridge, dict):
        bridge = {}
    model.source_format = str(bridge.get("source_format") or "cached_bbmodel_summary")
    model.primary_texture = str(bridge.get("primary_texture") or "")
    model.texture_variants = [str(value) for value in bridge.get("texture_variants", [])] if isinstance(bridge.get("texture_variants"), list) else []
    model.auxiliary_textures = [str(value) for value in bridge.get("auxiliary_textures_skipped", [])] if isinstance(bridge.get("auxiliary_textures_skipped"), list) else []
    model.variant_model_outputs = [str(value) for value in bridge.get("variant_model_outputs", [])] if isinstance(bridge.get("variant_model_outputs"), list) else []
    model.constructor_args = list(bridge.get("constructor_args", [])) if isinstance(bridge.get("constructor_args"), list) else []
    model.model_bounds = dict(bridge.get("model_bounds", {})) if isinstance(bridge.get("model_bounds"), dict) else {}
    model.linked_entities = [str(value) for value in bridge.get("linked_entities", [])] if isinstance(bridge.get("linked_entities"), list) else []
    model.linked_renderers = [str(value) for value in bridge.get("linked_renderers", [])] if isinstance(bridge.get("linked_renderers"), list) else []
    model.animation_resource = str(bridge.get("external_animation_resource") or "")
    model.animation_resource_path = str(bridge.get("external_animation_resource_path") or "")
    model.warnings.append(f"Resumed lightweight cached summary: {path.name}")
    return model


def _source_type_for(source: Path, model_type: str) -> str:
    model_type = model_type.lower()
    suffix = source.suffix.lower()
    if model_type == "java":
        if suffix != ".java":
            raise ConversionError(f"Model type is set to Java source, but input is {source.name}")
        return "java"
    if model_type == "class":
        if suffix != ".class":
            raise ConversionError(f"Model type is set to compiled class, but input is {source.name}")
        return "class"
    if suffix == ".java":
        return "java"
    if suffix == ".class":
        return "class"
    raise ConversionError(f"Unsupported model source type: {source.suffix}")


def _compiled_model_flavor(source: Path) -> str:
    """Identify the compiled model family before selecting a bytecode adapter."""
    try:
        data = source.read_bytes()
    except OSError:
        return "legacy"
    if b"net/minecraft/client/model/geom/builders/CubeListBuilder" in data or b"CubeListBuilder" in data:
        return "modern"
    if b"software/bernie/geckolib" in data or b"GeoModel" in data:
        return "geckolib"
    return "legacy"


def _log_model_details(model: ParsedModel, source: Path, callback=None) -> None:
    _log(f"Parsed model: class={model.class_name}, format={model.source_format}, texture_resolution={model.texture_width}x{model.texture_height}", callback)
    _log(f"Geometry totals: parts={len(model.parts)}, cubes={model.cube_count}", callback)
    for index, part in enumerate(sorted(model.parts, key=lambda p: p.source_index)):
        rx, ry, rz = part.rotation
        bb_rx, bb_ry, bb_rz = (-math.degrees(rx), -math.degrees(ry), math.degrees(rz))
        _log(
            f"Part[{index}] {part.name!r}: variable={part.variable!r}, parent={part.parent.name if part.parent else '<root>'!r}, "
            f"offset=({part.offset[0]:.6g}, {part.offset[1]:.6g}, {part.offset[2]:.6g}), "
            f"source_rot_rad=({rx:.6g}, {ry:.6g}, {rz:.6g}), "
            f"blockbench_rot_deg=({bb_rx:.6g}, {bb_ry:.6g}, {bb_rz:.6g}), cubes={len(part.cubes)}, children={len(part.children)}",
            callback,
        )
        for cube_index, cube in enumerate(part.cubes):
            _log(
                f"  Cube[{cube_index}] {cube.name!r}: pos=({cube.x:.6g}, {cube.y:.6g}, {cube.z:.6g}), "
                f"size=({cube.dx:.6g}, {cube.dy:.6g}, {cube.dz:.6g}), uv=({cube.uv_x}, {cube.uv_y}), "
                f"mirror={cube.mirror}, inflate={cube.inflate:.6g}",
                callback,
            )

    if model.animation_analysis:
        _log(f"Animation analysis keys: {', '.join(sorted(str(k) for k in model.animation_analysis.keys()))}", callback)
        for key, value in model.animation_analysis.items():
            if key in {"source_format", "method", "mode", "note"}:
                _log(f"  Animation analysis: {key}={value}", callback)
    else:
        _log("Animation analysis: none", callback)

    if model.animation_source:
        _log(f"Animation source: {model.animation_source}", callback)
    if model.animation_resource:
        _log(f"External animation resource: {model.animation_resource} -> {model.animation_resource_path or '<not resolved>'}", callback)

    if model.baked_animation_json:
        raw = model.baked_animation_json.get("animations", {})
        if isinstance(raw, dict):
            _log(f"Baked animation clips: {len(raw)}", callback)
            for name, anim in raw.items():
                bones = anim.get("bones", {}) if isinstance(anim, dict) else {}
                bone_count = len(bones) if isinstance(bones, dict) else 0
                keyframe_count = 0
                channels: set[str] = set()
                if isinstance(bones, dict):
                    for bone_data in bones.values():
                        if isinstance(bone_data, dict):
                            for channel, values in bone_data.items():
                                if isinstance(values, dict):
                                    channels.add(str(channel))
                                    keyframe_count += len(values)
                _log(f"  Clip {name!r}: bones={bone_count}, keyframes={keyframe_count}, channels={sorted(channels)}", callback)
    if model.blockbench_animations:
        _log(f"Native Blockbench animations already present: {len(model.blockbench_animations)}", callback)

    if model.warnings:
        _log(f"Warnings recorded: {len(model.warnings)}", callback, "WARN")
        for warning in model.warnings:
            _log(f"  {warning}", callback, "WARN")


def _safe_file_slug(value: str) -> str:
    slug = re.sub(r"[^A-Za-z0-9._-]+", "_", value).strip("._-")
    return slug or "texture"


def convert_file(
    source: Path,
    output: Path,
    overwrite: bool = True,
    texture_path: Optional[Path] = None,
    texture_index: Optional[TextureIndex] = None,
    texture_search_roots: Iterable[Path] = (),
    model_type: str = "auto",
    generate_variant_models: bool = True,
    constructor_args: Optional[list[object]] = None,
    linked_entities: Iterable[str] = (),
    linked_renderers: Iterable[str] = (),
    log_callback=None,
) -> ParsedModel:
    if output.exists() and not overwrite:
        raise ConversionError(f"Output already exists: {output}")

    source_kind = _source_type_for(source, model_type)
    _log(f"--- Converting {source} ---", log_callback)
    _log(f"Source kind: {source_kind}; requested model filter: {model_type}", log_callback)
    _log(f"Output target: {output}", log_callback)
    if source_kind == "class":
        flavor = _compiled_model_flavor(source)
        _log(f"Compiled model flavor: {flavor}", log_callback)
        if flavor == "modern":
            from modern_bytecode_bridge import parse_modern_bytecode_class
            model = parse_modern_bytecode_class(
                source, Cube, Part, ParsedModel,
                constructor_args=list(constructor_args or []),
            )
            # Modern compiled models can contain a standard createBodyLayer/create method.
            # Their procedural animation is handled by the associated renderer/entity class;
            # preserve a clear report marker rather than pretending the class itself contains it.
        elif flavor == "geckolib":
            from legacy_bridge import parse_legacy_class
            model = parse_legacy_class(
                source, Cube, Part, ParsedModel,
                resource_roots=[Path(p) for p in texture_search_roots],
                constructor_args=list(constructor_args or []),
            )
            model.warnings.append("GeckoLib model class detected. Geometry may be external JSON/geo data; the compiled class alone is not authoritative for full geometry extraction.")
        else:
            from legacy_bridge import parse_legacy_class
            model = parse_legacy_class(
                source, Cube, Part, ParsedModel,
                resource_roots=[Path(p) for p in texture_search_roots],
                constructor_args=list(constructor_args or []),
            )
    else:
        text = source.read_text(encoding="utf-8-sig")
        modern_markers = ("MeshDefinition", "PartDefinition", "CubeListBuilder", "LayerDefinition", "ModelPartData")
        parser_order = (
            (parse_java_model, parse_legacy_java_model)
            if any(marker in text for marker in modern_markers)
            else (parse_legacy_java_model, parse_java_model)
        )
        errors: list[str] = []
        model = None
        for parser in parser_order:
            try:
                candidate = (
                    parser(text, source.name, list(constructor_args or []))
                    if parser is parse_legacy_java_model
                    else parser(text, source.name)
                )
                if candidate.cube_count > 0:
                    model = candidate
                    break
                errors.append(f"{parser.__name__}: zero cubes")
            except Exception as exc:
                errors.append(f"{parser.__name__}: {exc}")
        if model is None:
            raise ConversionError(f"Could not recover Java model geometry: {'; '.join(errors)}")

    model.constructor_args = list(getattr(model, "constructor_args", constructor_args or []))
    model.scalar_fields = dict(getattr(model, "scalar_fields", {}))
    model.linked_entities = sorted(set(str(v) for v in linked_entities if v))
    model.linked_renderers = sorted(set(str(v) for v in linked_renderers if v))

    _log_model_details(model, source, log_callback)

    if texture_index is None:
        roots = infer_search_roots(source, texture_search_roots)
        texture_index = TextureIndex(roots, shallow_roots=[source.parent])
    _log(
        f"Texture resolution: explicit={texture_path or '<none>'}; index_pngs={len(texture_index.png_files)}; "
        f"indexed_references={len(texture_index.references)}; source_records={len(getattr(texture_index, 'records', []))}",
        log_callback,
    )
    texture_set = texture_index.resolve_set(source, model.class_name, texture_path)
    resolved_texture = texture_set.primary
    texture_paths = list(texture_set.variants)
    if resolved_texture is not None and resolved_texture not in texture_paths:
        texture_paths.insert(0, resolved_texture)

    model.primary_texture = str(resolved_texture) if resolved_texture else ""
    model.texture_variants = [str(path) for path in texture_paths]
    model.auxiliary_textures = [str(path) for path in texture_set.auxiliaries]
    model.texture_evidence = list(texture_set.evidence)

    if resolved_texture is not None:
        _log(
            f"Primary texture selected: {resolved_texture}; body/color variants={len(texture_paths)}; "
            f"auxiliary overlays skipped={len(texture_set.auxiliaries)}",
            log_callback,
        )
        model.warnings.append(
            f"Primary texture resolved to '{resolved_texture}' ({'; '.join(texture_set.evidence) or 'automatic discovery'})."
        )
        if len(texture_paths) > 1:
            model.warnings.append(f"Embedded {len(texture_paths)} compatible body/color textures and generated alternate model projects.")
    else:
        _log("Texture selected: NONE", log_callback, "WARN")
        model.warnings.append(
            "No body texture was automatically resolved. Auxiliary eye/glow/overlay textures are never selected as the main skin."
        )

    # Pre-compute alternate project names so the main .bbmodel metadata/report
    # can list them before it is serialized.
    variant_jobs: list[tuple[Path, Path]] = []
    if generate_variant_models and len(texture_paths) > 1:
        variant_dir = output.with_suffix("").with_name(output.stem + "_variants")
        used_names: set[str] = set()
        for variant in texture_paths[1:]:
            slug = _safe_file_slug(variant.stem)
            candidate = slug
            counter = 2
            while candidate.lower() in used_names:
                candidate = f"{slug}_{counter}"
                counter += 1
            used_names.add(candidate.lower())
            variant_output = variant_dir / f"{output.stem}__{candidate}.bbmodel"
            variant_jobs.append((variant, variant_output))
        model.variant_model_outputs = [str(path) for _texture, path in variant_jobs]

    data = model_to_bbmodel(model, str(source), texture_paths)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    _log(f"Wrote Blockbench model: {output} (bytes={output.stat().st_size})", log_callback)
    _log(f"Embedded textures: {len(data.get('textures', []))}; native Blockbench animations: {len(data.get('animations', []))}", log_callback)

    for variant_texture, variant_output in variant_jobs:
        if variant_output.exists() and not overwrite:
            _log(f"Skipped existing variant model: {variant_output}", log_callback, "WARN")
            continue
        variant_data = model_to_bbmodel(model, str(source), [variant_texture])
        bridge = variant_data.get("unhandled_root_fields", {}).get("moj_model_bridge", {})
        if isinstance(bridge, dict):
            bridge["variant_texture"] = str(variant_texture)
            bridge["variant_of"] = str(output)
        variant_output.parent.mkdir(parents=True, exist_ok=True)
        variant_output.write_text(json.dumps(variant_data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        _log(f"Wrote texture-variant Blockbench model: {variant_output} -> {variant_texture}", log_callback)

    if model.animation_analysis:
        stem = output.with_suffix("")
        analysis_path = stem.with_name(stem.name + "_animation_analysis.json")
        logic_path = stem.with_name(stem.name + "_animation.java")
        baked_path = stem.with_name(stem.name + "_baked_animations.json")
        analysis_path.write_text(json.dumps(model.animation_analysis, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        logic_path.write_text(model.animation_logic, encoding="utf-8")
        if model.baked_animation_json:
            baked_path.write_text(json.dumps(model.baked_animation_json, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        _log(f"Wrote animation analysis: {analysis_path}", log_callback)
        _log(f"Wrote runtime animation Java: {logic_path}", log_callback)
        if model.baked_animation_json:
            _log(f"Wrote baked animation JSON: {baked_path}", log_callback)
    else:
        _log("No animation sidecar files were generated.", log_callback)

    _log(f"SUCCESS: {source.name}", log_callback)
    return model


def _class_header_details(source: Path) -> tuple[str, str, int]:
    """Read the class name, direct superclass, and JVM access flags cheaply.

    Model discovery runs across very large mod archives, so this deliberately
    avoids spawning ``javap`` for every candidate.  It is sufficient to reject
    registries, renderer layers, entity classes, abstract model bases, and
    anonymous helper classes that merely reference Minecraft model APIs.
    """
    try:
        data = source.read_bytes()
    except OSError:
        return source.stem, "", 0
    if len(data) < 10 or data[:4] != b"\xCA\xFE\xBA\xBE":
        return source.stem, "", 0
    pos = 8
    try:
        cp_count = int.from_bytes(data[pos:pos + 2], "big")
        pos += 2
        cp: list[object] = [None] * cp_count
        index = 1
        while index < cp_count:
            tag = data[pos]
            pos += 1
            if tag == 1:
                length = int.from_bytes(data[pos:pos + 2], "big")
                pos += 2
                cp[index] = data[pos:pos + length].decode("utf-8", errors="replace")
                pos += length
            elif tag in (3, 4):
                pos += 4
            elif tag in (5, 6):
                pos += 8
                index += 1
            elif tag in (7, 8, 16, 19, 20):
                cp[index] = int.from_bytes(data[pos:pos + 2], "big")
                pos += 2
            elif tag in (9, 10, 11, 12, 17, 18):
                pos += 4
            elif tag == 15:
                pos += 3
            else:
                return source.stem, "", 0
            index += 1
        access_flags = int.from_bytes(data[pos:pos + 2], "big")
        pos += 2
        this_idx = int.from_bytes(data[pos:pos + 2], "big")
        pos += 2
        super_idx = int.from_bytes(data[pos:pos + 2], "big")

        def resolve_class(cp_index: int) -> str:
            if not cp_index or cp_index >= len(cp):
                return ""
            name_index = cp[cp_index]
            if not isinstance(name_index, int) or name_index >= len(cp):
                return ""
            value = cp[name_index]
            return value.replace("/", ".").split(".")[-1] if isinstance(value, str) else ""

        return resolve_class(this_idx) or source.stem, resolve_class(super_idx), access_flags
    except (IndexError, ValueError):
        return source.stem, "", 0


def collect_model_files(inputs: Iterable[Path], recursive: bool = True, model_type: str = "auto") -> list[Path]:
    """Collect candidate model inputs with deterministic source/class pairing.

    ``auto`` selects one representation per logical class.  Parallel archive trees
    such as ``Compiled/com/example/Model.class`` and
    ``Decompiled/com/example/Model.java`` are paired by their package-relative
    path rather than their immediate parent directory.  ``both`` deliberately
    returns both representations for diagnostics or bytecode/source comparison.
    """
    suffixes = {".java", ".class"}
    if model_type == "java":
        suffixes = {".java"}
    elif model_type == "class":
        suffixes = {".class"}
    elif model_type not in {"auto", "both"}:
        raise ValueError(f"Unknown model_type: {model_type}")

    files: list[Path] = []

    def is_likely_model_source(path: Path) -> bool:
        """Avoid treating every class in old flat decompiled packages as a model.

        Names are only a first hint; byte/source markers keep unconventional
        names such as ``ToreterrorModel`` and custom ModelRenderer subclasses.
        Explicit files are still subjected to this check, but known model APIs
        make the result deterministic rather than relying only on naming.
        """
        stem = path.stem.lower()
        if stem.startswith(("render", "renderer", "clientproxy", "commonproxy")):
            return False
        name_hint = stem.startswith("model") or stem.endswith("model")
        try:
            if path.suffix.lower() == ".java":
                text = path.read_text(encoding="utf-8-sig", errors="ignore")
                clean = strip_java_comments(text)
                if re.search(r"\b(?:interface|enum|record)\s+" + re.escape(path.stem) + r"\b", clean):
                    return False
                class_match = re.search(
                    r"\bclass\s+([A-Za-z_$][\w$]*)(?:\s+extends\s+([A-Za-z_$][\w$\.]*))?",
                    clean,
                )
                class_name = class_match.group(1) if class_match else path.stem
                superclass = class_match.group(2).split(".")[-1] if class_match and class_match.group(2) else ""
                modern_geometry = bool(re.search(
                    r"\b(?:PartDefinition|ModelPartData|CubeListBuilder|MeshDefinition)\b|\.\s*addOrReplaceChild\s*\(",
                    clean,
                ))
                gecko_model = superclass.endswith("GeoModel") or "extends GeoModel" in clean
                legacy_model = _looks_like_legacy_java_model(clean, class_name)
                known_super = superclass in {
                    "ModelBase", "ModelBiped", "ModelPlayer", "EntityModel", "HierarchicalModel", "AgeableListModel",
                }
                # An import or field mentioning ModelRenderer does not make an
                # item/entity/registry class a model. Require model geometry or
                # an actual model inheritance declaration.
                # A ModelBase subclass containing only an empty ModelRenderer
                # is often an intentional invisible/effect model. It should not
                # be reported as a failed geometry conversion. Inherited Biped
                # models remain valid even without local addBox calls.
                empty_effect_model = (
                    superclass == "ModelBase"
                    and "new ModelRenderer" in clean
                    and not any(token in clean for token in (
                        "new ModelBox(", ".addBox(", ".func_78789_a(",
                        ".func_78790_a(", ".field_78804_l.add(", ".cubeList.add(",
                    ))
                    and any(token in clean for token in (".func_78785_a(", ".render("))
                )
                return modern_geometry or gecko_model or legacy_model or (name_hint and known_super and not empty_effect_model)
            data = path.read_bytes()
            simple, superclass, access_flags = _class_header_details(path)
            # Keep compatibility with explicitly supplied/synthetic class
            # fixtures whose class header cannot be decoded. Real JVM classes
            # always have a resolvable direct superclass (usually Object).
            if not superclass and not access_flags:
                return name_hint or any(marker in data for marker in (b"ModelRenderer", b"CubeListBuilder"))
            # ACC_INTERFACE | ACC_ABSTRACT: framework bases are useful to their
            # concrete subclasses but are not independently exportable models.
            if access_flags & 0x0600:
                return False
            # Anonymous compiler helpers and enum payloads are never renderer
            # model registrations on their own. Named nested model classes remain
            # eligible because some real mods register them directly.
            if re.search(r"\$\d+$", simple) or superclass in {"Enum", "Record"}:
                return False
            model_super = (
                superclass in {
                    "ModelBase", "ModelBiped", "ModelPlayer", "EntityModel",
                    "HierarchicalModel", "AgeableListModel", "GeoModel",
                }
                or superclass.startswith("Model")
                or superclass.endswith("Model")
            )
            geometry_markers = (
                b'net/minecraft/client/model/ModelRenderer."<init>"',
                b"net/minecraft/client/model/ModelRenderer.<init>",
                b"net/minecraft/client/model/geom/builders/CubeListBuilder",
                b"addOrReplaceChild",
                b"software/bernie/geckolib/model/GeoModel",
            )
            # Do not accept a class merely because it imports/references a model
            # type. Registries, client events, entities, and render layers do that
            # frequently. Require model inheritance or local geometry creation.
            return model_super or any(marker in data for marker in geometry_markers)
        except OSError:
            return name_hint

    def is_generated_artifact(path: Path) -> bool:
        stem = path.stem.lower()
        if stem.endswith(("_animation", "_animation_logic", "_animation_analysis")):
            return True
        # MCreator/Blockbench Java keyframe definition classes are animation
        # companions, not model geometry. They are indexed separately instead
        # of being reported as failed model conversions.
        if stem.endswith("animation") and path.parent.name.lower() in {"animation", "animations"}:
            return True
        return False

    for item in inputs:
        item = item.expanduser().resolve()
        if (
            item.is_file()
            and item.suffix.lower() in suffixes
            and not is_generated_artifact(item)
            and is_likely_model_source(item)
        ):
            files.append(item)
        elif item.is_dir():
            for suffix in suffixes:
                iterator = item.rglob(f"*{suffix}") if recursive else item.glob(f"*{suffix}")
                files.extend(
                    path for path in iterator
                    if not is_generated_artifact(path) and is_likely_model_source(path)
                )
    resolved = sorted(set(path.resolve() for path in files))
    if model_type == "auto":
        def logical_pair_key(path: Path) -> tuple[str, str]:
            # Decompiled/Compiled are common sibling roots in supplied mod corpora.
            # Remove only those representation markers, preserving the package path
            # so unrelated classes with the same simple name do not collide.
            parts = list(path.parent.parts)
            marker_index = next(
                (i for i, part in enumerate(parts) if part.lower() in {"compiled", "decompiled", "classes", "sources"}),
                None,
            )
            if marker_index is not None:
                parent_key = Path(*parts[:marker_index], *parts[marker_index + 1 :])
            else:
                parent_key = path.parent
            return os.path.normcase(str(parent_key)), path.stem.lower()

        grouped: dict[tuple[str, str], list[Path]] = {}
        for path in resolved:
            grouped.setdefault(logical_pair_key(path), []).append(path)
        selected: list[Path] = []
        for candidates in grouped.values():
            if len(candidates) == 1:
                selected.extend(candidates)
                continue
            java = next((p for p in candidates if p.suffix.lower() == ".java"), None)
            compiled = next((p for p in candidates if p.suffix.lower() == ".class"), None)
            if java is not None and compiled is not None:
                try:
                    text = java.read_text(encoding="utf-8-sig", errors="ignore")
                except OSError:
                    text = ""
                # Modern source is substantially easier and more complete than
                # production-obfuscated modern bytecode. Legacy decompiled source
                # varies heavily by decompiler, so prefer its compiled class.
                if (
                    ("CubeListBuilder" in text and ("MeshDefinition" in text or "LayerDefinition" in text))
                    or re.search(r"\bextends\s+(?:[\w.]+\.)?ModelPlayer\b", text)
                ):
                    selected.append(java)
                else:
                    selected.append(compiled)
            else:
                selected.extend(candidates)
        resolved = sorted(set(selected))
    return resolved


def collect_java_files(inputs: Iterable[Path], recursive: bool = True) -> list[Path]:
    # Backwards-compatible alias used by older scripts.
    return collect_model_files(inputs, recursive, "auto")


def _normalised_output_stem(source: Path) -> str:
    # Browser/download duplicate suffixes such as "ModelGuardian(3).class" are
    # not part of the actual class name and should not leak into MCreator names.
    stem = re.sub(r"\s*\(\d+\)$", "", source.stem).strip()
    return _safe_file_slug(stem)


def _write_conversion_report(
    output_dir: Path,
    successes: list[tuple[Path, Path, ParsedModel]],
    failures: list[tuple[Path, str]],
    layout: Optional[ModLayout],
    log_callback=None,
) -> tuple[Path, Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    payload: dict[str, object] = {
        "moj_model_bridge_version": VERSION,
        "mod_layout": layout.describe() if layout else None,
        "summary": {"converted": len(successes), "failed": len(failures)},
        "models": [],
        "failures": [{"source": str(source), "error": error} for source, error in failures],
    }
    models = payload["models"]
    assert isinstance(models, list)
    for source, target, model in successes:
        models.append({
            "source": str(source),
            "output": str(target),
            "class_name": model.class_name,
            "source_format": model.source_format,
            "parts": len(model.parts),
            "cubes": model.cube_count,
            "texture_resolution": [model.texture_width, model.texture_height],
            "model_bounds": dict(model.model_bounds or compute_model_bounds(model)),
            "constructor_args": list(model.constructor_args),
            "linked_entities": list(model.linked_entities),
            "linked_renderers": list(model.linked_renderers),
            "primary_texture": model.primary_texture or None,
            "texture_variants": list(model.texture_variants),
            "auxiliary_textures_skipped": list(model.auxiliary_textures),
            "variant_model_outputs": list(model.variant_model_outputs),
            "animations": [a.get("name") for a in (model.blockbench_animations or bake_preview_into_blockbench(model)) if isinstance(a, dict)],
            "external_animation_resource": model.animation_resource or None,
            "warnings": list(model.warnings),
        })

    json_path = output_dir / "conversion_report.json"
    json_path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    rows: list[str] = []
    for item in models:
        assert isinstance(item, dict)
        textures = item.get("texture_variants") or []
        variant_outputs = item.get("variant_model_outputs") or []
        animations = item.get("animations") or []
        warnings = item.get("warnings") or []
        rows.append(
            "<tr>"
            f"<td><code>{html.escape(str(item.get('class_name')))}</code><br><small>{html.escape(str(item.get('source')))}</small></td>"
            f"<td>{item.get('parts')} parts<br>{item.get('cubes')} cubes<br><small>{html.escape(str(item.get('source_format')))}</small></td>"
            f"<td><b>{html.escape(str(item.get('primary_texture') or 'None'))}</b>"
            f"<br>{len(textures)} body/color texture(s)<br>{len(item.get('auxiliary_textures_skipped') or [])} overlay(s) skipped"
            + ("<details><summary>Texture list</summary><pre>" + html.escape("\n".join(str(v) for v in textures)) + "</pre></details>" if textures else "")
            + "</td>"
            f"<td>{html.escape(', '.join(str(a) for a in animations) or 'None')}</td>"
            f"<td>{len(variant_outputs)} alternate .bbmodel(s)"
            + ("<details><summary>Variant outputs</summary><pre>" + html.escape("\n".join(str(v) for v in variant_outputs)) + "</pre></details>" if variant_outputs else "")
            + "</td>"
            f"<td>{'<br>'.join(html.escape(str(w)) for w in warnings) or 'None'}</td>"
            "</tr>"
        )
    failure_html = "".join(
        f"<li><code>{html.escape(str(source))}</code>: {html.escape(error)}</li>" for source, error in failures
    ) or "<li>None</li>"
    layout_html = "<pre>Not supplied</pre>"
    if layout:
        layout_html = "<pre>" + html.escape(json.dumps(layout.describe(), indent=2)) + "</pre>"
    html_text = f"""<!doctype html>
<html><head><meta charset="utf-8"><title>MojModelBridge conversion report</title>
<style>
body{{font-family:Segoe UI,Arial,sans-serif;margin:24px;background:#16181d;color:#e7e9ee}}h1,h2{{color:#fff}}
table{{border-collapse:collapse;width:100%;background:#20232a}}th,td{{border:1px solid #3a3f49;padding:9px;vertical-align:top;text-align:left}}
th{{background:#292d36}}code,pre{{font-family:Consolas,monospace}}small{{color:#aeb4c0}}details{{margin-top:6px}}a{{color:#8ab4ff}}
.ok{{color:#76d275}}.bad{{color:#ff8585}}
</style></head><body>
<h1>MojModelBridge {html.escape(VERSION)}</h1>
<p><span class="ok">Converted: {len(successes)}</span> &nbsp; <span class="bad">Failed: {len(failures)}</span></p>
<h2>Detected mod layout</h2>{layout_html}
<h2>Models</h2>
<table><thead><tr><th>Model</th><th>Geometry</th><th>Textures</th><th>Animations</th><th>Variant projects</th><th>Warnings</th></tr></thead>
<tbody>{''.join(rows) or '<tr><td colspan="6">No models converted.</td></tr>'}</tbody></table>
<h2>Failures</h2><ul>{failure_html}</ul>
</body></html>"""
    html_path = output_dir / "conversion_report.html"
    html_path.write_text(html_text, encoding="utf-8")
    _log(f"Wrote conversion reports: {html_path} and {json_path}", log_callback)
    return html_path, json_path



def _missing_resume_model_classes(graph: object, mcreator_output: Optional[Path]) -> set[str]:
    """Return model classes still needed by a resumable MCreator workspace."""
    if graph is None or mcreator_output is None:
        return set()
    root = Path(mcreator_output).expanduser()
    if not root.exists():
        return set()
    workspace_files = sorted(root.glob("*.mcreator"))
    if not workspace_files:
        return set()
    try:
        workspace = json.loads(workspace_files[0].read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError):
        return set()
    descriptors = workspace.get("mod_elements", []) if isinstance(workspace, dict) else []
    living = [item for item in descriptors if isinstance(item, dict) and item.get("type") == "livingentity"]
    existing_names = {str(item.get("name", "")).lower() for item in living}
    compact = lambda value: re.sub(r"[^a-z0-9]", "", str(value).lower())
    existing_registries = {compact(item.get("registry_name", "")) for item in living}
    missing: set[str] = set()
    for reg in getattr(graph, "registrations", []):
        if not getattr(reg, "model_class", "") or not getattr(reg, "entity_class", ""):
            continue
        meta = getattr(graph, "entities", {}).get(reg.entity_class)
        raw_registry = getattr(meta, "registry_name", "") if meta is not None else reg.entity_class
        desired_registry = _safe_registry_name(raw_registry or reg.entity_class)
        desired_name = re.sub(r"[^A-Za-z0-9]+", " ", re.sub(r"^Entity", "", reg.entity_class)).title().replace(" ", "")
        if desired_name.lower() not in existing_names and compact(desired_registry) not in existing_registries:
            missing.add(reg.model_class)
    return missing


def _safe_registry_name(value: object) -> str:
    text = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", str(value or ""))
    return re.sub(r"[^a-z0-9_.-]+", "_", text.lower()).strip("_") or "converted_entity"

def batch_convert(
    inputs: list[Path],
    output_dir: Path,
    recursive: bool = True,
    overwrite: bool = True,
    texture_path: Optional[Path] = None,
    texture_search_root: Optional[Path] = None,
    model_type: str = "auto",
    mod_root: Optional[Path] = None,
    compiled_root: Optional[Path] = None,
    decompiled_root: Optional[Path] = None,
    generate_variant_models: bool = True,
    mcreator_output: Optional[Path] = None,
    mcreator_template: Optional[Path] = None,
    mcreator_author: Optional[str] = None,
    mcreator_generator: str = "neoforge-1.21.1",
    log_callback=None,
) -> tuple[list[tuple[Path, Path, ParsedModel]], list[tuple[Path, str]]]:
    layout: Optional[ModLayout] = None
    legacy_graph = None
    supplied_roots: list[Path] = []
    for candidate in (mod_root, decompiled_root, compiled_root):
        if candidate is not None:
            resolved = Path(candidate).expanduser().resolve()
            if resolved not in supplied_roots:
                supplied_roots.append(resolved)
    primary_root = Path(mod_root).expanduser().resolve() if mod_root else (Path(decompiled_root).expanduser().resolve() if decompiled_root else (Path(compiled_root).expanduser().resolve() if compiled_root else None))
    if primary_root:
        layouts = [discover_mod_layout(root) for root in supplied_roots]
        layout = layouts[0]
        for extra in layouts[1:]:
            for attr in ("model_roots", "render_roots", "texture_roots", "animation_roots", "resource_roots"):
                target = getattr(layout, attr)
                for path in getattr(extra, attr):
                    if path not in target:
                        target.append(path)
        _log(f"Mod root: {layout.root}", log_callback)
        if len(supplied_roots) > 1:
            _log(f"Combined source roots: {[str(path) for path in supplied_roots]}", log_callback)
        _log(f"Detected model roots: {[str(p) for p in layout.model_roots]}", log_callback)
        _log(f"Detected renderer roots: {[str(p) for p in layout.render_roots]}", log_callback)
        _log(f"Detected texture roots: {[str(p) for p in layout.texture_roots]}", log_callback)
        _log(f"Detected animation roots: {[str(p) for p in layout.animation_roots]}", log_callback)
        try:
            from legacy_mod_graph import load_legacy_mod_graph, scan_legacy_mod
            graph_path = output_dir / "legacy_mod_graph.json"
            if not overwrite and graph_path.exists():
                try:
                    legacy_graph = load_legacy_mod_graph(graph_path, expected_root=layout.root)
                    _log(
                        f"Loaded cached legacy entity/render graph: {graph_path} "
                        f"({len(legacy_graph.registrations)} registrations).",
                        log_callback,
                    )
                except Exception as cache_exc:
                    _log(f"Legacy graph cache unavailable ({cache_exc}); rescanning source mod.", log_callback, "WARNING")
                    legacy_graph = scan_legacy_mod(layout.root, log=lambda msg: _log(msg, log_callback), source_roots=supplied_roots)
            else:
                legacy_graph = scan_legacy_mod(layout.root, log=lambda msg: _log(msg, log_callback), source_roots=supplied_roots)
            graph_path.parent.mkdir(parents=True, exist_ok=True)
            graph_path.write_text(json.dumps(legacy_graph.to_dict(), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
            _log(f"Wrote recovered legacy entity/render graph: {graph_path}", log_callback)
        except Exception as exc:
            _log(f"Legacy entity/render graph unavailable: {exc}", log_callback, "WARNING")

    resume_missing_model_classes = (
        _missing_resume_model_classes(legacy_graph, mcreator_output)
        if not overwrite and mcreator_output is not None
        else set()
    )
    resume_skip_model_pass = False
    if resume_missing_model_classes:
        _log(
            f"MCreator resume needs {len(resume_missing_model_classes)} model class(es); "
            "other cached models will load as lightweight summaries.",
            log_callback,
        )
    elif not overwrite and mcreator_output is not None and Path(mcreator_output).exists():
        resume_skip_model_pass = True
        _log(
            "MCreator resume living entities are complete; skipping model discovery, "
            "texture indexing, and cache loading before non-entity continuation.",
            log_callback,
        )

    effective_inputs = [Path(p) for p in inputs]
    if not effective_inputs and layout:
        effective_inputs = list(layout.model_roots)
    _log(
        f"Batch start: inputs={len(effective_inputs)}, recursive={recursive}, overwrite={overwrite}, "
        f"model_type={model_type}, generate_variant_models={generate_variant_models}",
        log_callback,
    )
    _log(f"Input paths: {[str(p) for p in effective_inputs]}", log_callback)
    files = [] if resume_skip_model_pass else collect_model_files(effective_inputs, recursive, model_type)
    _log(f"Discovered model files: {len(files)}", log_callback)
    class_files = [path for path in files if path.suffix.lower() == ".class"]
    if class_files:
        try:
            from legacy_bridge import prime_javap_cache
            cached_count, fallback_count = prime_javap_cache(class_files)
            _log(
                f"Batched javap preload: cached={cached_count}, per-file fallback={fallback_count}",
                log_callback,
            )
        except Exception as exc:
            _log(f"Batched javap preload unavailable; using per-file inspection: {exc}", log_callback, "WARNING")
    for path in files:
        _log(f"  discovered: {path}", log_callback)
    try:
        output_abs = output_dir.expanduser().resolve()
        files = [path for path in files if output_abs not in path.parents and path != output_abs]
    except OSError:
        pass

    successes: list[tuple[Path, Path, ParsedModel]] = []
    failures: list[tuple[Path, str]] = []
    # Reuse models already parsed during Blockbench conversion when the MCreator
    # backend asks for the same source + constructor arguments.  Full-port used
    # to parse every large legacy model twice, which made animation-heavy mods
    # with many legacy models needlessly expensive.
    preparsed_models: dict[tuple[str, str], ParsedModel] = {}
    common_root: Optional[Path] = None
    dirs = [p if p.is_dir() else p.parent for p in effective_inputs]
    if dirs:
        try:
            common_root = Path(os.path.commonpath([str(p.resolve()) for p in dirs]))
        except ValueError:
            common_root = None

    search_roots: list[Path] = []
    if texture_search_root:
        search_roots.append(texture_search_root)
    search_roots.extend([p for p in effective_inputs if p.is_dir()])
    if layout:
        search_roots.extend(layout.model_roots)
        search_roots.extend(layout.render_roots)
        search_roots.extend(layout.texture_roots)
        search_roots.extend(layout.animation_roots)
        search_roots.extend(layout.resource_roots)
    for source in files:
        search_roots.extend(infer_search_roots(source, ()))
    unique_roots: list[Path] = []
    seen_roots: set[str] = set()
    for root in search_roots:
        try:
            root = Path(root).expanduser().resolve()
        except OSError:
            continue
        key = os.path.normcase(str(root))
        if key not in seen_roots and root.exists():
            seen_roots.add(key)
            unique_roots.append(root)

    shallow_roots = [source.parent for source in files]
    if layout:
        shallow_roots.extend(layout.render_roots)
    if resume_skip_model_pass:
        texture_index = TextureIndex([])
    elif texture_path and texture_path.exists() and texture_path.is_dir():
        texture_index = TextureIndex([texture_path, *([layout.root] if layout else [])])
    else:
        texture_index = TextureIndex(unique_roots, shallow_roots=shallow_roots)
    _log(f"Texture/resource search roots: {[str(p) for p in unique_roots]}", log_callback)
    _log(f"Texture index totals: png_files={len(texture_index.png_files)}, references={len(texture_index.references)}, source_records={len(texture_index.records)}", log_callback)

    for source in files:
        try:
            relative_parent = Path()
            if common_root:
                try:
                    relative_parent = source.parent.relative_to(common_root)
                except ValueError:
                    pass
            target = output_dir / relative_parent / f"{_normalised_output_stem(source)}.bbmodel"
            constructor_args: list[object] = []
            linked_entities: list[str] = []
            linked_renderers: list[str] = []
            if legacy_graph is not None:
                registrations = legacy_graph.registrations_by_model.get(source.stem, [])
                if registrations:
                    constructor_args = list(registrations[0].model_args)
                    linked_entities = [reg.entity_class for reg in registrations]
                    linked_renderers = [reg.renderer_class for reg in registrations]
                    _log(
                        f"Renderer graph for {source.stem}: constructor_args={constructor_args!r}, "
                        f"entities={linked_entities}, renderers={linked_renderers}",
                        log_callback,
                    )
            if target.exists() and not overwrite:
                needs_full_cache = source.stem in resume_missing_model_classes
                model = load_cached_bbmodel(target) if needs_full_cache else load_cached_bbmodel_summary(target)
                # Registration metadata belongs to the current source graph and
                # may be newer than the cached project metadata.
                model.constructor_args = list(constructor_args or model.constructor_args)
                model.linked_entities = sorted(set(str(v) for v in linked_entities if v)) or model.linked_entities
                model.linked_renderers = sorted(set(str(v) for v in linked_renderers if v)) or model.linked_renderers
                _log(
                    f"Resumed cached conversion: {target} "
                    f"(parts={len(model.parts)}, cubes={model.cube_count}, animations={len(model.blockbench_animations)})",
                    log_callback,
                )
            else:
                model = convert_file(
                    source,
                    target,
                    overwrite,
                    texture_path,
                    texture_index=texture_index,
                    texture_search_roots=unique_roots,
                    model_type=model_type,
                    generate_variant_models=generate_variant_models,
                    constructor_args=constructor_args,
                    linked_entities=linked_entities,
                    linked_renderers=linked_renderers,
                    log_callback=log_callback,
                )
            successes.append((source, target, model))
            try:
                source_key = str(source.expanduser().resolve())
            except OSError:
                source_key = str(source)
            args_key = json.dumps(constructor_args, sort_keys=True, default=str)
            if model.cached_cube_count is None:
                preparsed_models[(source_key, args_key)] = model
        except Exception as exc:
            _log(f"FAILURE: {source}: {exc}", log_callback, "ERROR")
            failures.append((source, str(exc)))
    _log(f"Batch complete: successes={len(successes)}, failures={len(failures)}", log_callback)
    _write_conversion_report(output_dir, successes, failures, layout, log_callback)
    if mcreator_output is not None:
        if legacy_graph is None:
            _log("MCreator export skipped: no legacy entity/render graph was recovered.", log_callback, "WARNING")
        else:
            try:
                from mcreator_export import export_mcreator_workspace
                export_mcreator_workspace(
                    legacy_graph,
                    Path(mcreator_output),
                    template_workspace=Path(mcreator_template) if mcreator_template else None,
                    log=lambda msg: _log(msg, log_callback),
                    preparsed_models=preparsed_models,
                    resume=not overwrite,
                    mod_author=mcreator_author,
                    target_generator=mcreator_generator,
                )
            except Exception as exc:
                _log(f"MCreator export failed: {exc}", log_callback, "ERROR")
    return successes, failures


# ----------------------------------- GUI -----------------------------------

def launch_gui() -> int:
    try:
        import tkinter as tk
        from tkinter import filedialog, messagebox, ttk
    except ImportError:
        print("Tkinter is not available in this Python installation.", file=sys.stderr)
        return 2

    class App(tk.Tk):
        def __init__(self) -> None:
            super().__init__()
            self.title(f"MojModelBridge {VERSION}")
            self.geometry("980x940")
            self.minsize(760, 580)
            self.selected: list[Path] = []
            self.output_var = tk.StringVar(value=str(Path.cwd() / "converted_models"))
            self.texture_file_var = tk.StringVar(value="")
            self.texture_root_var = tk.StringVar(value="")
            self.mod_root_var = tk.StringVar(value="")
            self.compiled_root_var = tk.StringVar(value="")
            self.decompiled_root_var = tk.StringVar(value="")
            self.mcreator_output_var = tk.StringVar(value="")
            self.mcreator_template_var = tk.StringVar(value="")
            self.mcreator_author_var = tk.StringVar(value="")
            self.mcreator_generator_var = tk.StringVar(value="neoforge-1.21.1")
            self.model_type_var = tk.StringVar(value="auto")
            self.recursive_var = tk.BooleanVar(value=True)
            self.overwrite_var = tk.BooleanVar(value=True)
            self.variant_models_var = tk.BooleanVar(value=True)
            self.status_var = tk.StringVar(value="Select model files or a model source folder.")
            self._build(ttk, tk, filedialog, messagebox)

        def _build(self, ttk, tk, filedialog, messagebox) -> None:
            outer = ttk.Frame(self, padding=12)
            outer.pack(fill="both", expand=True)

            ttk.Label(outer, text="Minecraft Model → Blockbench .bbmodel", font=("Segoe UI", 14, "bold")).pack(anchor="w")
            ttk.Label(
                outer,
                text=(
                    "Converts modern Mojmap Java and compiled legacy 1.7.10-1.12.2 ModelBase classes. "
                    "Animation logic is exported as Java, and recoverable preview animations are baked into the native Blockbench animation list. "
                    "Textures can be found automatically from the model, renderer, or companion classes."
                ),
                wraplength=860,
            ).pack(anchor="w", pady=(2, 10))

            mod_frame = ttk.LabelFrame(outer, text="Mod sources (compiled, decompiled, or both)", padding=8)
            mod_frame.pack(fill="x", pady=(0, 8))
            ttk.Label(mod_frame, text="Combined/root:").grid(row=0, column=0, sticky="w")
            ttk.Entry(mod_frame, textvariable=self.mod_root_var).grid(row=0, column=1, sticky="ew", padx=6)
            ttk.Button(mod_frame, text="Browse…", command=lambda: self.choose_mod_root(filedialog, self.mod_root_var, "Select combined mod root")).grid(row=0, column=2)
            ttk.Label(mod_frame, text="Compiled:").grid(row=1, column=0, sticky="w", pady=(5, 0))
            ttk.Entry(mod_frame, textvariable=self.compiled_root_var).grid(row=1, column=1, sticky="ew", padx=6, pady=(5, 0))
            ttk.Button(mod_frame, text="Browse…", command=lambda: self.choose_mod_root(filedialog, self.compiled_root_var, "Select compiled mod/classes root")).grid(row=1, column=2, pady=(5, 0))
            ttk.Label(mod_frame, text="Decompiled:").grid(row=2, column=0, sticky="w", pady=(5, 0))
            ttk.Entry(mod_frame, textvariable=self.decompiled_root_var).grid(row=2, column=1, sticky="ew", padx=6, pady=(5, 0))
            ttk.Button(mod_frame, text="Browse…", command=lambda: self.choose_mod_root(filedialog, self.decompiled_root_var, "Select decompiled Java/resources root")).grid(row=2, column=2, pady=(5, 0))
            ttk.Label(
                mod_frame,
                text="Use Combined/root for one folder, or select separate Compiled and Decompiled folders. Both are analyzed into one output mod.",
                wraplength=900,
            ).grid(row=3, column=0, columnspan=3, sticky="w", pady=(6, 0))
            mod_frame.columnconfigure(1, weight=1)

            buttons = ttk.Frame(outer)
            buttons.pack(fill="x")
            ttk.Button(buttons, text="Add Model Files…", command=lambda: self.add_files(filedialog)).pack(side="left")
            ttk.Button(buttons, text="Add Model Folder…", command=lambda: self.add_folder(filedialog)).pack(side="left", padx=6)
            ttk.Button(buttons, text="Clear", command=self.clear_files).pack(side="left")

            source_opts = ttk.LabelFrame(outer, text="Model source filter", padding=8)
            source_opts.pack(fill="x", pady=(8, 0))
            ttk.Label(source_opts, text="Convert:").pack(side="left")
            ttk.Combobox(
                source_opts,
                textvariable=self.model_type_var,
                values=("auto", "both", "java", "class"),
                state="readonly",
                width=18,
            ).pack(side="left", padx=6)
            ttk.Label(source_opts, text="Auto = choose the best paired representation; Both = convert both copies; Java = source only; Class = compiled only.").pack(side="left")

            list_frame = ttk.Frame(outer)
            list_frame.pack(fill="both", expand=False, pady=8)
            self.file_list = tk.Listbox(list_frame, height=8, selectmode="extended")
            scrollbar = ttk.Scrollbar(list_frame, orient="vertical", command=self.file_list.yview)
            self.file_list.configure(yscrollcommand=scrollbar.set)
            self.file_list.pack(side="left", fill="both", expand=True)
            scrollbar.pack(side="right", fill="y")

            out_frame = ttk.LabelFrame(outer, text="Output", padding=8)
            out_frame.pack(fill="x")
            ttk.Entry(out_frame, textvariable=self.output_var).pack(side="left", fill="x", expand=True)
            ttk.Button(out_frame, text="Browse…", command=lambda: self.choose_output(filedialog)).pack(side="left", padx=(6, 0))

            tex_frame = ttk.LabelFrame(outer, text="Texture discovery", padding=8)
            tex_frame.pack(fill="x", pady=(6, 0))

            ttk.Label(tex_frame, text="Texture file:").grid(row=0, column=0, sticky="w")
            ttk.Entry(tex_frame, textvariable=self.texture_file_var).grid(row=0, column=1, sticky="ew", padx=6)
            ttk.Button(tex_frame, text="Browse…", command=lambda: self.choose_texture(filedialog)).grid(row=0, column=2)

            ttk.Label(tex_frame, text="Search folder:").grid(row=1, column=0, sticky="w", pady=(6, 0))
            ttk.Entry(tex_frame, textvariable=self.texture_root_var).grid(row=1, column=1, sticky="ew", padx=6, pady=(6, 0))
            ttk.Button(tex_frame, text="Browse…", command=lambda: self.choose_texture_root(filedialog)).grid(row=1, column=2, pady=(6, 0))

            ttk.Label(
                tex_frame,
                text="Leave both blank for automatic discovery. Legacy models may resolve textures from Render*.java/.class companion files.",
                wraplength=820,
            ).grid(row=2, column=0, columnspan=3, sticky="w", pady=(6, 0))
            tex_frame.columnconfigure(1, weight=1)

            mc_frame = ttk.LabelFrame(outer, text="MCreator workspace export (optional)", padding=8)
            mc_frame.pack(fill="x", pady=(6, 0))
            ttk.Label(mc_frame, text="Workspace output:").grid(row=0, column=0, sticky="w")
            ttk.Entry(mc_frame, textvariable=self.mcreator_output_var).grid(row=0, column=1, sticky="ew", padx=6)
            ttk.Button(mc_frame, text="Browse…", command=lambda: self.choose_mcreator_output(filedialog)).grid(row=0, column=2)
            ttk.Label(mc_frame, text="Template workspace:").grid(row=1, column=0, sticky="w", pady=(6, 0))
            ttk.Entry(mc_frame, textvariable=self.mcreator_template_var).grid(row=1, column=1, sticky="ew", padx=6, pady=(6, 0))
            ttk.Button(mc_frame, text="Browse…", command=lambda: self.choose_mcreator_template(filedialog)).grid(row=1, column=2, pady=(6, 0))
            ttk.Label(mc_frame, text="Author override:").grid(row=2, column=0, sticky="w", pady=(6, 0))
            ttk.Entry(mc_frame, textvariable=self.mcreator_author_var).grid(row=2, column=1, sticky="ew", padx=6, pady=(6, 0))
            ttk.Label(mc_frame, text="Blank = preserve author(s) recovered from the original mod metadata.").grid(row=2, column=2, sticky="w", pady=(6, 0))
            ttk.Label(mc_frame, text="Generator:").grid(row=3, column=0, sticky="w", pady=(6, 0))
            ttk.Combobox(mc_frame, textvariable=self.mcreator_generator_var, values=("neoforge-1.21.1", "forge-1.20.1"), state="readonly", width=22).grid(row=3, column=1, sticky="w", padx=6, pady=(6, 0))
            ttk.Label(
                mc_frame,
                text=(
                    "Leave workspace output blank to skip. Standalone exports default to MCreator 2026.2 NeoForge 1.21.1; "
                    "a template workspace is cloned and the selected generator is preserved/updated."
                ),
                wraplength=820,
            ).grid(row=4, column=0, columnspan=3, sticky="w", pady=(6, 0))
            mc_frame.columnconfigure(1, weight=1)

            options = ttk.Frame(outer)
            options.pack(fill="x", pady=8)
            ttk.Checkbutton(options, text="Include subfolders", variable=self.recursive_var).pack(side="left")
            ttk.Checkbutton(options, text="Overwrite existing files", variable=self.overwrite_var).pack(side="left", padx=14)
            ttk.Checkbutton(options, text="Generate per-texture variant .bbmodels", variable=self.variant_models_var).pack(side="left")
            ttk.Button(options, text="Convert", command=lambda: self.convert(messagebox)).pack(side="right")

            log_frame = ttk.LabelFrame(outer, text="Conversion Log", padding=6)
            log_frame.pack(fill="both", expand=True)
            self.log = tk.Text(log_frame, wrap="word", height=12, state="disabled")
            log_scroll = ttk.Scrollbar(log_frame, orient="vertical", command=self.log.yview)
            self.log.configure(yscrollcommand=log_scroll.set)
            self.log.pack(side="left", fill="both", expand=True)
            log_scroll.pack(side="right", fill="y")
            ttk.Label(outer, textvariable=self.status_var).pack(anchor="w", pady=(6, 0))

        def append_log(self, message: str) -> None:
            self.log.configure(state="normal")
            self.log.insert("end", message.rstrip() + "\n")
            self.log.see("end")
            self.log.configure(state="disabled")
            self.update_idletasks()

        def refresh_list(self) -> None:
            self.file_list.delete(0, "end")
            for path in self.selected:
                self.file_list.insert("end", str(path))
            self.status_var.set(f"{len(self.selected)} input path(s) selected.")

        def add_files(self, filedialog) -> None:
            paths = filedialog.askopenfilenames(
                title="Select model files",
                filetypes=[
                    ("Java/Class models", "*.java *.class"),
                    ("Java source", "*.java"),
                    ("Compiled classes", "*.class"),
                    ("All files", "*.*"),
                ],
            )
            for value in paths:
                path = Path(value)
                if path not in self.selected:
                    self.selected.append(path)
            self.refresh_list()

        def add_folder(self, filedialog) -> None:
            value = filedialog.askdirectory(title="Select model source folder")
            if value:
                path = Path(value)
                if path not in self.selected:
                    self.selected.append(path)
                self.refresh_list()

        def clear_files(self) -> None:
            self.selected.clear()
            self.refresh_list()

        def choose_mod_root(self, filedialog, target_var=None, title="Select unpacked/decompiled mod root") -> None:
            value = filedialog.askdirectory(title=title)
            if value:
                (target_var or self.mod_root_var).set(value)
                try:
                    layout = discover_mod_layout(Path(value))
                    self.append_log(f"Detected model roots: {[str(p) for p in layout.model_roots]}")
                    self.append_log(f"Detected renderer roots: {[str(p) for p in layout.render_roots]}")
                    self.append_log(f"Detected texture roots: {[str(p) for p in layout.texture_roots]}")
                except Exception as exc:
                    self.append_log(f"Mod-root scan warning: {exc}")

        def choose_output(self, filedialog) -> None:
            value = filedialog.askdirectory(title="Select output folder")
            if value:
                self.output_var.set(value)

        def choose_texture(self, filedialog) -> None:
            value = filedialog.askopenfilename(
                title="Select texture PNG",
                filetypes=[("PNG texture", "*.png"), ("All files", "*.*")],
            )
            if value:
                self.texture_file_var.set(value)

        def choose_texture_root(self, filedialog) -> None:
            value = filedialog.askdirectory(title="Select texture/resource search folder")
            if value:
                self.texture_root_var.set(value)

        def choose_mcreator_output(self, filedialog) -> None:
            value = filedialog.askdirectory(title="Select MCreator workspace output folder")
            if value:
                self.mcreator_output_var.set(value)

        def choose_mcreator_template(self, filedialog) -> None:
            value = filedialog.askdirectory(title="Select an existing MCreator workspace template")
            if value:
                self.mcreator_template_var.set(value)

        def convert(self, messagebox) -> None:
            mod_root = Path(self.mod_root_var.get()).expanduser() if self.mod_root_var.get().strip() else None
            compiled_root = Path(self.compiled_root_var.get()).expanduser() if self.compiled_root_var.get().strip() else None
            decompiled_root = Path(self.decompiled_root_var.get()).expanduser() if self.decompiled_root_var.get().strip() else None
            if not self.selected and mod_root is None and compiled_root is None and decompiled_root is None:
                messagebox.showwarning("No input", "Select a mod root, model file, or model folder.")
                return
            output = Path(self.output_var.get()).expanduser()
            texture_file = Path(self.texture_file_var.get()).expanduser() if self.texture_file_var.get().strip() else None
            texture_root = Path(self.texture_root_var.get()).expanduser() if self.texture_root_var.get().strip() else None
            mcreator_output = Path(self.mcreator_output_var.get()).expanduser() if self.mcreator_output_var.get().strip() else None
            mcreator_template = Path(self.mcreator_template_var.get()).expanduser() if self.mcreator_template_var.get().strip() else None
            mcreator_author = self.mcreator_author_var.get().strip() or None
            mcreator_generator = self.mcreator_generator_var.get().strip() or "neoforge-1.21.1"
            self.append_log(f"\nMojModelBridge {VERSION}")
            self.append_log(f"Model filter: {self.model_type_var.get()}")
            self.append_log(f"Output: {output}")
            successes, failures = batch_convert(
                self.selected,
                output,
                recursive=self.recursive_var.get(),
                overwrite=self.overwrite_var.get(),
                texture_path=texture_file,
                texture_search_root=texture_root,
                model_type=self.model_type_var.get(),
                mod_root=mod_root,
                compiled_root=compiled_root,
                decompiled_root=decompiled_root,
                generate_variant_models=self.variant_models_var.get(),
                mcreator_output=mcreator_output,
                mcreator_template=mcreator_template,
                mcreator_author=mcreator_author,
                mcreator_generator=mcreator_generator,
                log_callback=self.append_log,
            )
            for source, target, model in successes:
                self.append_log(f"OK  {source.name} -> {target} ({len(model.parts)} parts, {model.cube_count} cubes)")
                for warning in model.warnings:
                    self.append_log(f"    {warning}")
            for source, error in failures:
                self.append_log(f"FAIL {source}: {error}")
            if not successes and not failures:
                self.append_log("No matching model files were found in the selected inputs.")
            self.status_var.set(f"Finished: {len(successes)} converted, {len(failures)} failed.")
            if failures:
                messagebox.showwarning("Conversion finished", f"Converted {len(successes)} file(s); {len(failures)} failed. See the log.")
            else:
                messagebox.showinfo("Conversion finished", f"Converted {len(successes)} file(s) into:\n{output}")

    app = App()
    app.mainloop()
    return 0


# ----------------------------------- CLI -----------------------------------

def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Convert Minecraft model sources/classes into editable Blockbench .bbmodel files, including compiled legacy 1.7.10-1.12.2 ModelBase classes and procedural Java animation recovery."
    )
    parser.add_argument("inputs", nargs="*", type=Path, help="Java files, class files, and/or source folders")
    parser.add_argument("-o", "--output", type=Path, default=Path("converted_models"), help="Output directory")
    parser.add_argument("--model-type", choices=("auto", "both", "java", "class"), default="auto", help="Which model sources to scan: auto-paired, both representations, Java source only, or compiled .class only")
    parser.add_argument("--no-recursive", action="store_true", help="Do not scan input folders recursively")
    parser.add_argument("--no-overwrite", action="store_true", help="Do not overwrite existing .bbmodel files")
    parser.add_argument("--gui", action="store_true", help="Open the graphical interface")
    parser.add_argument("--texture", type=Path, help="Optional explicit texture file or texture search folder (backwards-compatible option)")
    parser.add_argument("--texture-root", type=Path, help="Folder to search for PNGs and companion Java/class texture references")
    parser.add_argument("--mod-root", type=Path, help="Unpacked/decompiled mod base; automatically discovers model, renderer, texture, and animation subfolders")
    parser.add_argument("--compiled-root", type=Path, help="Optional compiled mod/classes root to merge with --decompiled-root or --mod-root")
    parser.add_argument("--decompiled-root", type=Path, help="Optional decompiled Java/resources root to merge with --compiled-root or --mod-root")
    parser.add_argument("--no-variant-models", action="store_true", help="Embed discovered texture variants but do not generate one alternate .bbmodel per texture")
    parser.add_argument("--mcreator-output", type=Path, help="Also generate an MCreator workspace at this path")
    parser.add_argument("--mcreator-generator", choices=("neoforge-1.21.1", "forge-1.20.1"), default="neoforge-1.21.1", help="MCreator generator target; defaults to NeoForge 1.21.1")
    parser.add_argument("--mcreator-template", type=Path, help="Optional existing MCreator workspace to clone before injecting converted entities")
    parser.add_argument("--mod-author", help="Override the MCreator workspace author; blank/default preserves original mod metadata")
    parser.add_argument("--full-port", action="store_true", help="Run the complete mod-root migration pipeline: models, textures, entities, items, tools, armor, blocks, dimensions, biomes, plants, projectiles, MCreator source bridge, and reports")
    parser.add_argument("--version", action="version", version=f"MojModelBridge {VERSION}")
    return parser


def main(argv: Optional[list[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.gui or (not args.inputs and not args.mod_root and not args.compiled_root and not args.decompiled_root):
        return launch_gui()
    if args.full_port:
        if args.mod_root is None and args.compiled_root is None and args.decompiled_root is None:
            parser.error("--full-port requires --mod-root, --compiled-root, and/or --decompiled-root")
        if args.mcreator_output is None:
            args.mcreator_output = args.output / "MCreatorWorkspace"
        if not args.inputs:
            args.inputs = [path for path in (args.decompiled_root, args.compiled_root, args.mod_root) if path is not None]
        print(f"MojModelBridge {VERSION} FULL MOD PORT")
        print(f"Combined/mod root: {args.mod_root or '<not supplied>'}")
        print(f"Compiled root: {args.compiled_root or '<not supplied>'}")
        print(f"Decompiled root: {args.decompiled_root or '<not supplied>'}")
        print(f"Blockbench output: {args.output}")
        print(f"MCreator output: {args.mcreator_output}")
        print(f"MCreator generator: {args.mcreator_generator}")
        print(f"MCreator template: {args.mcreator_template or '<standalone skeleton>'}")
    successes, failures = batch_convert(
        args.inputs,
        args.output,
        recursive=not args.no_recursive,
        overwrite=not args.no_overwrite,
        texture_path=args.texture,
        texture_search_root=args.texture_root,
        model_type=args.model_type,
        mod_root=args.mod_root,
        compiled_root=args.compiled_root,
        decompiled_root=args.decompiled_root,
        generate_variant_models=not args.no_variant_models,
        mcreator_output=args.mcreator_output,
        mcreator_template=args.mcreator_template,
        mcreator_author=args.mod_author,
        mcreator_generator=args.mcreator_generator,
    )
    for source, target, model in successes:
        print(f"OK: {source} -> {target} ({len(model.parts)} parts, {model.cube_count} cubes)")
        for warning in model.warnings:
            print(f"  {warning}")
    for source, error in failures:
        print(f"ERROR: {source}: {error}", file=sys.stderr)
    if not successes and not failures:
        if args.mcreator_output is not None:
            workspace_root = Path(args.mcreator_output).expanduser()
            if workspace_root.exists() and any(workspace_root.glob("*.mcreator")):
                print(f"MCreator workspace resume completed: {workspace_root}")
                return 0
        print("No matching model files found.", file=sys.stderr)
        return 1
    return 1 if failures else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
    except Exception:
        traceback.print_exc()
        raise SystemExit(1)
