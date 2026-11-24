
from __future__ import annotations

import math
import re
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional


class LegacyParseError(RuntimeError):
    pass


@dataclass
class _LegacyPart:
    name: str
    tex_u: int = 0
    tex_v: int = 0
    offset: list[float] = field(default_factory=lambda: [0.0, 0.0, 0.0])
    rotation: list[float] = field(default_factory=lambda: [0.0, 0.0, 0.0])
    mirror: bool = False
    texture_prefix: str = ""
    cubes: list[tuple[float, float, float, float, float, float, int, int, float, bool]] = field(default_factory=list)
    parent: Optional[str] = None
    source_index: int = 0


@dataclass
class _ObjRef:
    part: _LegacyPart


@dataclass
class _CubeRef:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    dx: float = 0.0
    dy: float = 0.0
    dz: float = 0.0
    tex_u: int = 0
    tex_v: int = 0
    delta: float = 0.0
    mirror: bool = False
    owner: Optional[_LegacyPart] = None


@dataclass
class _CubeListRef:
    owner: _LegacyPart


@dataclass
class _Expr:
    text: str


@dataclass
class _Method:
    header: str
    instructions: list[tuple[int, str, str]]


_ROT_FIELDS = {
    "field_78795_f": ("rotation", "x"),  # rotationAngleX
    "field_78796_g": ("rotation", "y"),  # rotationAngleY
    "field_78808_h": ("rotation", "z"),  # rotationAngleZ
}

_POS_FIELDS = {
    "field_78798_e": ("offset", "x"),  # rotationPointX
    "field_78797_d": ("offset", "y"),  # rotationPointY
    "field_78800_c": ("offset", "z"),  # rotationPointZ
}

_PARAM_NAMES = {
    1: "limbSwing",
    2: "limbSwingAmount",
    3: "ageInTicks",
    4: "netHeadYaw",
    5: "headPitch",
    6: "scaleFactor",
    7: "entity",
}


_JAVAP_TEXT_CACHE: dict[str, str] = {}


def _source_cache_key(source: Path) -> str:
    try:
        return str(source.expanduser().resolve())
    except OSError:
        return str(source)


def _split_javap_blocks(text: str) -> list[str]:
    """Split multi-class javap output while preserving command argument order."""
    starts = [match.start() for match in re.finditer(r'(?m)^Compiled from \"', text)]
    if not starts:
        return [text] if text.strip() else []
    starts.append(len(text))
    return [text[starts[i]:starts[i + 1]].strip() + "\n" for i in range(len(starts) - 1)]


def prime_javap_cache(sources: list[Path]) -> tuple[int, int]:
    """Preload javap output in batches. Returns ``(cached, fallback)`` counts.

    Windows has a relatively small command-line length limit, so chunks are
    bounded by both file count and rendered command length. Any chunk that does
    not split cleanly is simply left for the normal per-file fallback.
    """
    pending: list[Path] = []
    for source in sources:
        if source.suffix.lower() != ".class":
            continue
        if _source_cache_key(source) not in _JAVAP_TEXT_CACHE:
            pending.append(source)
    if not pending:
        return 0, 0

    chunks: list[list[Path]] = []
    current: list[Path] = []
    current_chars = len("javap -c -p ")
    # Keep this below the Windows CreateProcess practical command-line ceiling.
    max_chars = 7000
    for source in pending:
        rendered = str(source)
        if current and (len(current) >= 50 or current_chars + len(rendered) + 3 > max_chars):
            chunks.append(current)
            current = []
            current_chars = len("javap -c -p ")
        current.append(source)
        current_chars += len(rendered) + 3
    if current:
        chunks.append(current)

    cached = 0
    fallback = 0
    for chunk in chunks:
        try:
            result = subprocess.run(
                ["javap", "-c", "-p", *[str(path) for path in chunk]],
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                check=False,
                timeout=max(60, min(300, len(chunk) * 8)),
            )
        except (FileNotFoundError, subprocess.TimeoutExpired):
            fallback += len(chunk)
            continue
        if result.returncode != 0:
            fallback += len(chunk)
            continue
        blocks = _split_javap_blocks(result.stdout)
        if len(blocks) != len(chunk):
            fallback += len(chunk)
            continue
        for source, block in zip(chunk, blocks):
            _JAVAP_TEXT_CACHE[_source_cache_key(source)] = block
            cached += 1
    return cached, fallback


def _run_javap(source: Path) -> str:
    cache_key = _source_cache_key(source)
    cached = _JAVAP_TEXT_CACHE.get(cache_key)
    if cached is not None:
        return cached
    try:
        result = subprocess.run(
            ["javap", "-c", "-p", str(source)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
            timeout=60,
        )
    except FileNotFoundError as exc:
        raise LegacyParseError(
            "javap was not found. A JDK is required to inspect compiled legacy .class models."
        ) from exc
    if result.returncode != 0:
        raise LegacyParseError(result.stderr.strip() or f"javap failed with exit code {result.returncode}")
    _JAVAP_TEXT_CACHE[cache_key] = result.stdout
    return result.stdout


def _parse_methods(text: str) -> list[_Method]:
    lines = text.splitlines()
    headers: list[tuple[int, str]] = []
    for i, line in enumerate(lines):
        if re.match(
            r"^\s{2}(?:public|private|protected|static|final|native|synchronized|abstract).*\(.*\);$",
            line,
        ):
            headers.append((i, line.strip()))

    methods: list[_Method] = []
    for idx, (start, header) in enumerate(headers):
        end = headers[idx + 1][0] if idx + 1 < len(headers) else len(lines)
        block = lines[start:end]
        instructions: list[tuple[int, str, str]] = []
        for line in block:
            match = re.match(r"^\s*(\d+):\s+(.+?)(?:\s+//\s+(.*))?$", line)
            if match:
                instructions.append(
                    (int(match.group(1)), match.group(2).strip(), (match.group(3) or "").strip())
                )
        if instructions:
            methods.append(_Method(header, instructions))
    return methods


def _simple_class_name(value: str) -> str:
    return str(value or "").replace("/", ".").split(".")[-1].split("$")[0]


def _field_name(comment: str) -> Optional[str]:
    match = re.search(r"Field\s+(.+?):", comment)
    if not match:
        return None
    token = match.group(1).split(".")[-1].split("/")[-1]
    return token


def _method_info(comment: str) -> Optional[tuple[str, str, str]]:
    match = re.search(r"(?:Method|InterfaceMethod)\s+(.+?)\.(.+?):(\(.*\).*)", comment)
    if match:
        owner, name, desc = match.groups()
        return owner, name.strip('"'), desc
    match = re.search(r"(?:Method|InterfaceMethod)\s+([^:]+):(\(.*\).*)", comment)
    if match:
        name, desc = match.groups()
        return "", name.strip('"'), desc
    return None


def _descriptor_arg_count(desc: str) -> int:
    """Count JVM descriptor parameters (long/double are one operand-stack value)."""
    if "(" not in desc or ")" not in desc:
        return 0
    raw = desc.split("(", 1)[1].split(")", 1)[0]
    count = 0
    i = 0
    while i < len(raw):
        while i < len(raw) and raw[i] == "[":
            i += 1
        if i >= len(raw):
            break
        if raw[i] == "L":
            end = raw.find(";", i)
            if end < 0:
                break
            i = end + 1
        else:
            i += 1
        count += 1
    return count


def _descriptor_returns_value(desc: str) -> bool:
    return ")" in desc and desc.split(")", 1)[1][:1] not in {"", "V"}


def _parse_constant(op: str, comment: str) -> Optional[float | int]:
    op = op.strip()
    if op == "fconst_0":
        return 0.0
    if op == "fconst_1":
        return 1.0
    if op == "fconst_2":
        return 2.0
    if op == "dconst_0":
        return 0.0
    if op == "dconst_1":
        return 1.0
    if op == "iconst_m1":
        return -1
    if op.startswith("iconst_"):
        return int(op[-1])
    match = re.match(r"(?:bipush|sipush)\s+(-?\d+)", op)
    if match:
        return int(match.group(1))
    if op.startswith("ldc") or op.startswith("ldc2_w"):
        match = re.search(
            r"(?:float|double|int|long)\s+([+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)(?:[fFdDlL])?",
            comment,
        )
        if match:
            value = float(match.group(1))
            if re.search(r"\b(?:int|long)\b", comment) and value.is_integer():
                return int(value)
            return value
    return None


def _parse_class_name(text: str) -> str:
    match = re.search(r"^\s*public (?:final )?class ([\w.$]+)", text, re.M)
    if match:
        return match.group(1).split(".")[-1]
    match = re.search(r"^\s*(?:public )?class ([\w.$]+)", text, re.M)
    if match:
        return match.group(1).split(".")[-1]
    return "LegacyModel"


def _find_model_renderer_constructor(text: str) -> bool:
    normalized = text.replace("/", ".")
    if "extends net.minecraft.client.model.ModelBase" in normalized:
        return True
    if "extends net.minecraft.client.model.ModelBiped" in normalized:
        return True

    # Armor and custom animation frameworks commonly inherit ModelBase/ModelBiped
    # indirectly.  The bytecode in the concrete subclass is still self-contained
    # enough to recover when it declares and constructs ModelRenderer parts.
    has_renderer_field = bool(re.search(r"ModelRenderer\s+[A-Za-z_$][\w$]*;", normalized))
    has_renderer_ctor = 'ModelRenderer."<init>"' in normalized or "ModelRenderer.<init>" in normalized
    has_custom_renderer_ctor = bool(re.search(r"(?:model|Model)Renderer.*?<init>", normalized, re.S))
    return bool(has_renderer_field and (has_renderer_ctor or has_custom_renderer_ctor))


def _extract_external_animation_resource(text: str) -> tuple[str, str]:
    """Recover a ResourceLocation used by a custom external animation system."""
    # Typical javap output around a ResourceLocation constructor has the namespace
    # and JSON path as adjacent String constants.
    pattern = re.compile(
        r'new .*ResourceLocation.*?String\s+([a-z0-9_.-]+)\s+.*?String\s+([^\s]+\.json)',
        re.IGNORECASE | re.DOTALL,
    )
    match = pattern.search(text)
    if match:
        namespace, path = match.groups()
        return f"{namespace}:{path}", path

    strings = re.findall(r'// String\s+([^\s]+)', text)
    json_paths = [value for value in strings if value.lower().endswith('.json')]
    for path in json_paths:
        idx = strings.index(path)
        for namespace in reversed(strings[:idx]):
            if re.fullmatch(r'[a-z0-9_.-]+', namespace, re.IGNORECASE):
                return f"{namespace}:{path}", path
        return path, path
    return "", ""


def _resolve_resource_path(resource: str, path_hint: str, roots: list[Path]) -> Optional[Path]:
    if not resource and not path_hint:
        return None
    namespace = ""
    rel = path_hint or resource
    if ":" in resource:
        namespace, rel = resource.split(":", 1)
    rel = rel.replace("\\", "/").lstrip("/")
    candidates: list[Path] = []
    for root in roots:
        root = root.expanduser().resolve()
        candidates.extend([
            root / "assets" / namespace / rel if namespace else root / rel,
            root / namespace / rel if namespace else root / rel,
            root / "assets" / rel,
            root / rel,
        ])
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    return None


def _format_number(value: float | int) -> str:
    if isinstance(value, int):
        return str(value)
    if abs(float(value) - round(float(value))) < 1e-10:
        return str(int(round(float(value))))
    return f"{float(value):.8g}"


def _expr_text(value: Any) -> str:
    if isinstance(value, _Expr):
        return value.text
    if isinstance(value, (int, float)):
        return _format_number(value)
    return str(value)


def _java_float(value: float) -> str:
    value = float(value)
    if abs(value - math.pi) < 1e-5:
        return "(float)Math.PI"
    if abs(value - round(value)) < 1e-10:
        return f"{int(round(value))}.0F"
    return f"{value:.8g}F"


def _java_expr(text: str) -> str:
    # Symbolic expressions emitted by the bytecode interpreter are intentionally
    # conservative. These replacements target the common legacy MathHelper calls.
    text = re.sub(r"\bsin\(", "Mth.sin(", text)
    text = re.sub(r"\bcos\(", "Mth.cos(", text)
    text = re.sub(r"\babs\(", "Math.abs(", text)

    # Add float suffixes to bare decimal literals without touching identifiers.
    def repl(match: re.Match[str]) -> str:
        token = match.group(0)
        if token.endswith(("F", "f")):
            return token
        return token + "F"

    text = re.sub(r"(?<![\w.])(?:\d+\.\d+|\.\d+)(?![\w.])", repl, text)
    return text


def _source_rotation_to_blockbench_degrees(x: float, y: float, z: float) -> tuple[float, float, float]:
    """Map legacy ModelRenderer rotation values to the converted ModelPart pose space.

    MojModelBridge writes legacy rotations directly into modern ``PartPose``/``ModelPart``
    fields. The animation bake therefore uses the same axis convention: no second sign
    inversion is applied at the animation stage. Earlier versions transformed animation
    deltas a second time, which caused animation-only 90-degree turns and position drift.
    """
    return (float(x), float(y), float(z))


def _source_axis_sign(axis: str) -> float:
    """Legacy source axes map 1:1 to the converted ModelPart axes."""
    return 1.0


def _source_position_axis_sign(axis: str) -> float:
    """Legacy position keyframes use Blockbench's inverted Y axis for model-space motion."""
    if axis == "y":
        return -1.0
    return 1.0


def _safe_eval_expr(text: str, values: dict[str, float]) -> float:
    expr = text.replace("Mth.sin", "sin").replace("Mth.cos", "cos")
    expr = expr.replace("Math.abs", "abs")
    # The evaluator is only used on expressions produced by our own bytecode
    # interpreter, which contains arithmetic and the functions below.
    env = {
        "sin": math.sin,
        "cos": math.cos,
        "abs": abs,
        **values,
    }
    return float(eval(expr, {"__builtins__": {}}, env))


def _select_animation_methods(methods: list[_Method]) -> list[_Method]:
    """Return likely animation-bearing methods in priority order.

    Many 1.7.10 bosses animate inside render/func_78088_a instead of
    setRotationAngles. Keep both candidates and later choose the method that
    actually reconstructs the most assignments.
    """
    ordered: list[_Method] = []
    priorities = (
        ("func_78087_a", "setRotationAngles"),
        ("func_78088_a", "render"),
    )
    for names in priorities:
        for method in methods:
            if any(re.search(rf"\b{re.escape(name)}\s*\(", method.header) for name in names):
                if method not in ordered:
                    ordered.append(method)
    return ordered


def _animation_parameter_map(header: str) -> dict[int, str]:
    # Legacy setRotationAngles(float,float,float,float,float,float,Entity)
    if re.search(r"\b(?:func_78087_a|setRotationAngles)\s*\(", header):
        return {
            1: "limbSwing",
            2: "limbSwingAmount",
            3: "ageInTicks",
            4: "netHeadYaw",
            5: "headPitch",
            6: "scaleFactor",
            7: "entity",
        }
    # Legacy render(Entity,float,float,float,float,float,float)
    if re.search(r"\b(?:func_78088_a|render)\s*\(", header):
        return {
            1: "entity",
            2: "limbSwing",
            3: "limbSwingAmount",
            4: "ageInTicks",
            5: "netHeadYaw",
            6: "headPitch",
            7: "scaleFactor",
        }
    return dict(_PARAM_NAMES)


def _constructor_local_defaults(header: str, provided_args: Optional[list[Any]] = None) -> dict[int, Any]:
    """Return JVM local values for a constructor signature.

    When a renderer/client-proxy registration provides the actual constructor
    arguments, use them. This matters for old models where a constructor float
    controls animation speed or geometry inflation. Otherwise fall back to
    zero/false values so parameterized Techne/Tabula models remain recoverable.
    """
    match = re.search(r"\((.*)\);$", header.strip())
    if not match:
        return {}
    params_text = match.group(1).strip()
    if not params_text:
        return {}
    params = [part.strip() for part in params_text.split(",") if part.strip()]
    values: dict[int, Any] = {}
    slot = 1
    for index, param in enumerate(params):
        base = param.replace("[]", "").strip()
        supplied = provided_args[index] if provided_args is not None and index < len(provided_args) else None
        if supplied is not None:
            values[slot] = supplied
        elif base in {"boolean"}:
            values[slot] = False
        elif base in {"byte", "short", "char", "int", "long", "float", "double"}:
            values[slot] = 0.0 if base in {"float", "double"} else 0
        else:
            values[slot] = f"local{slot}"
        slot += 2 if base in {"long", "double"} else 1
    return values


def _load_slot(base: str, op: str) -> int:
    match = re.search(r"_(\d+)$", base)
    if match:
        return int(match.group(1))
    pieces = op.split()
    return int(pieces[1]) if len(pieces) > 1 else 0


def _parse_constructor(
    method: _Method,
    constructor_args: Optional[list[Any]] = None,
) -> tuple[dict[str, _LegacyPart], dict[str, int], dict[str, float], list[str]]:
    """Recover legacy ModelBase geometry from common 1.7.10-1.12.2 bytecode patterns.

    Supports both:
      * ModelRenderer(ModelBase)
      * ModelRenderer(ModelBase, int textureU, int textureV)
      * ModelRenderer.addBox(...)
      * direct new ModelBox(...) + ModelRenderer.field_78804_l.add(...)

    The direct ModelBox form is especially common in models exported by
    Tabula/Techne-style generators and was the reason some otherwise valid
    classes were rejected by v1.3.
    """
    stack: list[Any] = []
    local_defaults = _constructor_local_defaults(method.header, constructor_args)
    local_values = dict(local_defaults)
    texture_offsets: dict[str, tuple[int, int]] = {}
    temp_objects: list[_LegacyPart] = []
    field_objects: dict[str, _LegacyPart] = {}
    texture = {"w": 64, "h": 32}
    model_texture_explicit = {"w": False, "h": False}
    scalar_fields: dict[str, float] = {}
    warnings: list[str] = []

    def field_ref(name: Optional[str]) -> Any:
        if name and name in field_objects:
            return _ObjRef(field_objects[name])
        return name

    def popn(count: int) -> list[Any]:
        if count <= 0:
            return []
        if len(stack) < count:
            result = [None] * (count - len(stack)) + stack[:]
            stack.clear()
            return result
        result = stack[-count:]
        del stack[-count:]
        return result

    for _, op, comment in method.instructions:
        base = op.split()[0]

        if base.startswith("aload"):
            slot = _load_slot(base, op)
            stack.append("this" if slot == 0 else local_values.get(slot, f"local{slot}"))
            continue

        if base.startswith(("fload", "dload", "iload", "lload")):
            slot = _load_slot(base, op)
            stack.append(local_values.get(slot, 0.0))
            continue

        if base.startswith(("fstore", "dstore", "istore", "lstore", "astore")):
            slot = _load_slot(base, op)
            local_values[slot] = stack.pop() if stack else 0.0
            continue

        if base in {"fadd", "dadd", "iadd", "ladd", "fsub", "dsub", "isub", "lsub",
                    "fmul", "dmul", "imul", "lmul", "fdiv", "ddiv", "idiv", "ldiv"}:
            right = stack.pop() if stack else 0.0
            left = stack.pop() if stack else 0.0
            if isinstance(left, (int, float)) and isinstance(right, (int, float)):
                if base.endswith("add"):
                    stack.append(left + right)
                elif base.endswith("sub"):
                    stack.append(left - right)
                elif base.endswith("mul"):
                    stack.append(left * right)
                else:
                    stack.append(left / right if right != 0 else 0.0)
            else:
                stack.append(0.0)
            continue

        if base in {"fneg", "dneg", "ineg", "lneg"}:
            value = stack.pop() if stack else 0.0
            stack.append(-value if isinstance(value, (int, float)) else 0.0)
            continue

        if base in {"i2f", "i2d", "i2l", "f2d", "f2i", "f2l", "d2f", "d2i", "d2l", "l2f", "l2d", "l2i"}:
            # Numeric conversion does not change the recovered scalar value.
            continue

        if base == "new" and "ModelRenderer" in comment:
            obj = _LegacyPart(name=f"__tmp_{len(temp_objects)}", source_index=len(temp_objects))
            temp_objects.append(obj)
            stack.append(obj)
            continue

        if base == "new" and "ModelBox" in comment:
            stack.append(_CubeRef())
            continue

        if base == "dup":
            if stack:
                stack.append(stack[-1])
            continue

        if base in {
            "fconst_0", "fconst_1", "fconst_2", "dconst_0", "dconst_1",
            "iconst_m1", "iconst_0", "iconst_1", "iconst_2", "iconst_3",
            "iconst_4", "iconst_5", "bipush", "sipush", "ldc", "ldc_w", "ldc2_w",
        }:
            value = _parse_constant(op, comment)
            if value is not None:
                stack.append(value)
            elif "String " in comment:
                stack.append(("string", comment.split("String ", 1)[1]))
            continue

        if base == "getfield":
            name = _field_name(comment)
            owner = stack.pop() if stack else None
            if isinstance(owner, _ObjRef) and name == "field_78804_l":
                # ModelRenderer.field_78804_l = cubeList. Remember which
                # ModelRenderer owns the list so a later List.add(ModelBox)
                # can attach the recovered cube to the right part.
                stack.append(_CubeListRef(owner.part))
            elif name == "field_78090_t" and owner == "this":
                stack.append("textureWidth")
            elif name == "field_78089_u" and owner == "this":
                stack.append("textureHeight")
            elif isinstance(owner, _ObjRef) and name in _ROT_FIELDS:
                axis = {"x": 0, "y": 1, "z": 2}[_ROT_FIELDS[name][1]]
                stack.append(float(owner.part.rotation[axis]))
            elif isinstance(owner, _ObjRef) and name in _POS_FIELDS:
                axis = {"x": 0, "y": 1, "z": 2}[_POS_FIELDS[name][1]]
                stack.append(float(owner.part.offset[axis]))
            else:
                stack.append(field_ref(name))
            continue

        if base == "putfield":
            name = _field_name(comment)
            value = stack.pop() if stack else None
            owner = stack.pop() if stack else None

            if name == "field_78090_t" and isinstance(value, (int, float)):
                texture["w"] = int(value)
                model_texture_explicit["w"] = True
            elif name == "field_78089_u" and isinstance(value, (int, float)):
                texture["h"] = int(value)
                model_texture_explicit["h"] = True
            elif owner == "this" and name and isinstance(value, (int, float)):
                scalar_fields[name] = float(value)
            elif name == "field_78809_i" and isinstance(owner, _ObjRef):
                owner.part.mirror = bool(value)
            elif isinstance(value, _LegacyPart):
                value.name = name or value.name
                field_objects[name or value.name] = value
            elif isinstance(owner, _ObjRef) and name in _ROT_FIELDS and isinstance(value, (int, float)):
                axis = _ROT_FIELDS[name][1]
                owner.part.rotation[{"x": 0, "y": 1, "z": 2}[axis]] = float(value)
            elif isinstance(owner, _ObjRef) and name in _POS_FIELDS and isinstance(value, (int, float)):
                axis = _POS_FIELDS[name][1]
                owner.part.offset[{"x": 0, "y": 1, "z": 2}[axis]] = float(value)
            continue

        if not base.startswith("invoke"):
            # Keep bytecode stack simulation conservative. Unhandled opcodes
            # are skipped rather than turning a valid model into a hard failure.
            continue

        info = _method_info(comment)
        if not info:
            continue
        owner, name, desc = info

        if name == "<init>" and "ModelRenderer" in owner:
            # ModelRenderer(ModelBase)
            if "ModelBase;)V" in desc and len(stack) >= 3:
                _this = stack.pop()   # ModelBase instance
                obj2 = stack.pop()    # duplicated ModelRenderer ref (constructor receiver)
                obj1 = stack.pop()    # original duplicated ref retained after <init>
                if isinstance(obj1, _LegacyPart):
                    # Keep the original ref on the stack because JVM bytecode
                    # leaves it in place after invokespecial's receiver is consumed.
                    stack.append(obj1)
                continue

            # ModelRenderer(ModelBase, int textureU, int textureV)
            if "ModelBase;II)V" in desc and len(stack) >= 5:
                v = stack.pop()
                u = stack.pop()
                _this = stack.pop()
                obj2 = stack.pop()
                obj1 = stack.pop()
                if isinstance(obj1, _LegacyPart):
                    if isinstance(u, (int, float)):
                        obj1.tex_u = int(u)
                    if isinstance(v, (int, float)):
                        obj1.tex_v = int(v)
                    stack.append(obj1)
                continue

            # ModelRenderer(ModelBase, String texturePrefix) is used by older
            # vanilla-dragon-derived and Draconic Evolution models. Named cubes
            # then resolve UVs through ModelBase.setTextureOffset(prefix.name).
            if "ModelBase;Ljava/lang/String;)V" in desc and len(stack) >= 4:
                prefix = stack.pop()
                _this = stack.pop()
                _receiver = stack.pop()
                original = stack.pop()
                if isinstance(original, _LegacyPart):
                    if isinstance(prefix, tuple) and prefix and prefix[0] == "string":
                        original.texture_prefix = str(prefix[1])
                    stack.append(original)
                continue

            # Unknown ModelRenderer constructor overload. Leave the surviving
            # duplicated object reference in place when possible.
            warnings.append(f"Unsupported ModelRenderer constructor {desc}; model may be partially reconstructed.")
            continue

        if name == "<init>" and "ModelBox" in owner:
            # Common direct ModelBox constructor:
            # (ModelRenderer, int, int, float, float, float,
            #  int, int, int, float, boolean)V
            if "ModelRenderer;IIFFFIIIFZ)V" in desc and len(stack) >= 14:
                mirror = stack.pop()
                delta = stack.pop()
                dz = stack.pop()
                dy = stack.pop()
                dx = stack.pop()
                z = stack.pop()
                y = stack.pop()
                x = stack.pop()
                tex_v = stack.pop()
                tex_u = stack.pop()
                model_renderer = stack.pop()
                receiver = stack.pop()
                original = stack[-1] if stack else None
                if isinstance(original, _CubeRef):
                    original.x = float(x)
                    original.y = float(y)
                    original.z = float(z)
                    original.dx = float(dx)
                    original.dy = float(dy)
                    original.dz = float(dz)
                    original.tex_u = int(tex_u)
                    original.tex_v = int(tex_v)
                    original.delta = float(delta)
                    original.mirror = bool(mirror)
                    original.owner = model_renderer.part if isinstance(model_renderer, _ObjRef) else None
                continue

            warnings.append(f"Unsupported ModelBox constructor {desc}; cube may be missing.")
            continue

        if name in ("func_78793_a", "setRotationPoint") and len(stack) >= 4:
            z = stack.pop()
            y = stack.pop()
            x = stack.pop()
            obj = stack.pop()
            if isinstance(obj, _ObjRef) and all(isinstance(v, (int, float)) for v in (x, y, z)):
                obj.part.offset = [float(x), float(y), float(z)]
            continue

        if name in ("func_78790_a", "addBox", "func_78789_a", "func_78786_a"):
            # Legacy ModelRenderer has several cube overloads:
            #   addBox(x,y,z,dx,dy,dz)
            #   addBox(x,y,z,dx,dy,dz,inflate)
            #   addBox(name,x,y,z,dx,dy,dz[,inflate])
            # 1.7.10 Techne exports overwhelmingly use func_78789_a(FFFIII).
            has_name = "Ljava/lang/String;" in desc
            has_inflate = "FFFIIIF" in desc
            numeric_count = 7 if has_inflate else 6
            needed = 1 + numeric_count + (1 if has_name else 0)
            if len(stack) >= needed:
                raw = [stack.pop() for _ in range(numeric_count)][::-1]
                box_name = None
                if has_name:
                    raw_name = stack.pop()
                    if isinstance(raw_name, tuple) and raw_name and raw_name[0] == "string":
                        box_name = str(raw_name[1])
                obj = stack.pop()
                if isinstance(obj, _ObjRef) and all(isinstance(v, (int, float)) for v in raw):
                    x, y, z, dx, dy, dz = raw[:6]
                    delta = raw[6] if has_inflate else 0.0
                    uv_u, uv_v = obj.part.tex_u, obj.part.tex_v
                    if box_name:
                        full_name = f"{obj.part.texture_prefix}.{box_name}" if obj.part.texture_prefix else box_name
                        uv_u, uv_v = texture_offsets.get(full_name, texture_offsets.get(box_name, (uv_u, uv_v)))
                    obj.part.cubes.append(
                        (
                            float(x), float(y), float(z),
                            float(dx), float(dy), float(dz),
                            int(uv_u), int(uv_v), float(delta), bool(obj.part.mirror),
                        )
                    )
            continue

        if name in ("func_78787_b", "setTextureSize") and len(stack) >= 3:
            h = stack.pop()
            w = stack.pop()
            _obj = stack.pop()
            if isinstance(w, (int, float)) and isinstance(h, (int, float)):
                # This is per-ModelRenderer state. Never let a small part-level
                # value overwrite a large model-wide texture declaration.
                if not model_texture_explicit["w"]:
                    texture["w"] = max(texture["w"], int(w))
                if not model_texture_explicit["h"]:
                    texture["h"] = max(texture["h"], int(h))
            continue

        if (name == "func_78085_a" or (name == "setTextureOffset" and "ModelBase" in owner)) and len(stack) >= 4:
            v = stack.pop()
            u = stack.pop()
            key = stack.pop()
            _model = stack.pop()
            if isinstance(key, tuple) and key and key[0] == "string" and isinstance(u, (int, float)) and isinstance(v, (int, float)):
                texture_offsets[str(key[1])] = (int(u), int(v))
            continue

        if name in ("func_78784_a", "setTextureOffset") and len(stack) >= 3:
            v = stack.pop()
            u = stack.pop()
            obj = stack.pop()
            if isinstance(obj, _ObjRef):
                obj.part.tex_u = int(u)
                obj.part.tex_v = int(v)
                stack.append(obj)
            continue

        if name in ("func_78792_a", "addChild") and len(stack) >= 2:
            child = stack.pop()
            parent = stack.pop()
            if isinstance(child, _ObjRef) and isinstance(parent, _ObjRef):
                child.part.parent = parent.part.name
            continue

        if name in ("sin", "cos") and ("java/lang/Math" in owner or owner.endswith(".Math")):
            a = pop()
            stack.append(_Expr(f"{name}({_expr_text(a)})"))
            continue

        # Preserve simple entity/model getter expressions so state-dependent
        # assignments remain visible in reports and generated Java.
        if name.startswith(("get", "is", "has")):
            receiver = pop()
            stack.append(_Expr(f"{_expr_text(receiver)}.{name}()"))
            warnings.append(f"State getter {owner}.{name} was preserved symbolically; branch selection is approximate.")
            continue

        if name in ("setRotateAngle", "setRotationAngle", "setRotation", "setAngles") and len(stack) >= 5:
            rz = stack.pop()
            ry = stack.pop()
            rx = stack.pop()
            obj = stack.pop()
            _this = stack.pop()
            if isinstance(obj, _ObjRef) and all(isinstance(v, (int, float)) for v in (rx, ry, rz)):
                obj.part.rotation = [float(rx), float(ry), float(rz)]
            continue

        if name == "add" and ("java.util.List" in owner or "java/util/List" in owner) and len(stack) >= 2:
            cube = stack.pop()
            cube_list = stack.pop()
            if isinstance(cube_list, _CubeListRef) and isinstance(cube, _CubeRef):
                if cube.owner is None:
                    cube.owner = cube_list.owner
                target = cube.owner or cube_list.owner
                target.cubes.append(
                    (
                        cube.x, cube.y, cube.z,
                        cube.dx, cube.dy, cube.dz,
                        cube.tex_u, cube.tex_v, cube.delta, cube.mirror,
                    )
                )
            continue

        # Every JVM invocation consumes its descriptor arguments plus the
        # receiver for non-static calls. Older versions skipped unknown calls
        # entirely, leaving superclass constructors and harmless helpers on the
        # simulated operand stack. Those stale values later shifted legitimate
        # ModelRenderer operands and corrupted rotation points in otherwise
        # valid compiled models. Consume unsupported calls conservatively and
        # retain only a symbolic return value when the descriptor requires one.
        argc = _descriptor_arg_count(desc)
        _args = popn(argc)
        receiver = None if base == "invokestatic" else (stack.pop() if stack else None)
        if _descriptor_returns_value(desc):
            if receiver is not None:
                stack.append(_Expr(f"{_expr_text(receiver)}.{name}()"))
            else:
                stack.append(_Expr(f"{name}()"))

    if not field_objects:
        warnings.append("No ModelRenderer fields were reconstructed from the constructor.")
    elif not any(part.cubes for part in field_objects.values()):
        warnings.append(
            "ModelRenderer parts were found, but no cubes were reconstructed. "
            "The class may use an uncommon ModelBox/addBox overload."
        )

    return field_objects, texture, scalar_fields, warnings


def _method_param_count_from_header(header: str) -> int:
    match = re.search(r"\((.*)\)", header)
    if not match:
        return 0
    body = match.group(1).strip()
    if not body:
        return 0
    # Legacy model helpers overwhelmingly use primitive scalar arguments. For the
    # uncommon object-parameter case, counting comma-separated parameters is still
    # a better helper overload discriminator than treating every method as unique.
    return len([part for part in body.split(",") if part.strip()])


def _parse_animation(
    method: Optional[_Method],
    parts: dict[str, _LegacyPart],
    parameter_names: Optional[dict[int, str]] = None,
    scalar_fields: Optional[dict[str, float]] = None,
    helper_methods: Optional[list[_Method]] = None,
    current_class: str = "",
    helper_depth: int = 0,
) -> tuple[dict[str, Any], str, dict[str, Any], list[str]]:
    if method is None:
        return {}, "", {}, []

    field_objects = {name: _ObjRef(part) for name, part in parts.items()}
    parameter_names = dict(parameter_names or _PARAM_NAMES)
    scalar_fields = dict(scalar_fields or {})
    state: dict[str, dict[str, _Expr]] = {}
    for name, part in parts.items():
        state[name] = {
            "x": _Expr(_format_number(part.rotation[0])),
            "y": _Expr(_format_number(part.rotation[1])),
            "z": _Expr(_format_number(part.rotation[2])),
        }

    for name, part in parts.items():
        state[name].update(
            {
                "px": _Expr(_format_number(part.offset[0])),
                "py": _Expr(_format_number(part.offset[1])),
                "pz": _Expr(_format_number(part.offset[2])),
            }
        )

    stack: list[Any] = []
    locals_expr: dict[int, Any] = {}
    formulas: list[dict[str, Any]] = []
    warnings: list[str] = []
    helper_calls: list[tuple[str, list[Any], int]] = []
    preserved_branch_locals: set[int] = set()

    def pop() -> Any:
        return stack.pop() if stack else _Expr("<?>")

    runtime_names = set(parameter_names.values()) | {
        "limbSwing", "limbSwingAmount", "ageInTicks", "netHeadYaw", "headPitch",
    }

    def dynamic_expr(value: Any) -> bool:
        text = _expr_text(value)
        return any(re.search(rf"\b{re.escape(name)}\b", text) for name in runtime_names) or any(
            token in text for token in ("sin(", "cos(", ".get", ".is", ".has")
        )

    def store_local(slot: int, value: Any) -> None:
        previous = locals_expr.get(slot)
        # A linear bytecode walk sees both sides of an if/else. Decompilers often
        # emit a dynamic assignment in one branch and a constant reset in the
        # other; blindly accepting the later reset collapses the entire animation
        # to zero. Preserve the dynamic branch as the best clip-recovery signal.
        if previous is not None and dynamic_expr(previous) and not dynamic_expr(value):
            if slot not in preserved_branch_locals:
                warnings.append(
                    f"Local {slot} had a dynamic branch followed by a static fallback; "
                    "the dynamic expression was preserved for animation recovery."
                )
                preserved_branch_locals.add(slot)
            return
        locals_expr[slot] = value

    for offset, op, comment in method.instructions:
        base = op.split()[0]

        if base.startswith("aload"):
            n = _load_slot(base, op)
            stack.append("this" if n == 0 else locals_expr.get(n, parameter_names.get(n, f"local{n}")))
            continue

        if base.startswith("fload") or base.startswith("dload") or base.startswith("iload"):
            if base.endswith("_0"):
                n = 0
            elif "_" in base:
                n = int(base.rsplit("_", 1)[1])
            else:
                n = int(op.split()[1])
            stack.append(locals_expr.get(n, _Expr(parameter_names.get(n, f"local{n}"))))
            continue

        if base.startswith("fstore") or base.startswith("dstore") or base.startswith("istore"):
            if base.endswith("_0"):
                n = 0
            elif "_" in base:
                n = int(base.rsplit("_", 1)[1])
            else:
                n = int(op.split()[1])
            store_local(n, pop())
            continue

        if base.startswith("astore"):
            n = _load_slot(base, op)
            store_local(n, pop())
            continue

        if base in {
            "fconst_0", "fconst_1", "fconst_2", "dconst_0", "dconst_1",
            "iconst_m1", "iconst_0", "iconst_1", "iconst_2", "iconst_3",
            "iconst_4", "iconst_5", "bipush", "sipush", "ldc", "ldc_w", "ldc2_w",
        }:
            value = _parse_constant(op, comment)
            if value is not None:
                stack.append(_Expr(_format_number(value)))
            continue

        if base == "dup":
            if stack:
                stack.append(stack[-1])
            continue
        if base == "dup_x1":
            if len(stack) >= 2:
                value1 = stack.pop()
                value2 = stack.pop()
                stack.extend([value1, value2, value1])
            continue
        if base == "dup2":
            if len(stack) >= 2:
                stack.extend(stack[-2:])
            elif stack:
                stack.append(stack[-1])
            continue
        if base in {"checkcast", "nop"}:
            continue
        if base in {"pop", "pop2"}:
            if stack:
                stack.pop()
            continue

        if base.startswith("if_icmp") or base.startswith("if_acmp"):
            pop(); pop()
            continue
        if base.startswith("if"):
            pop()
            continue
        if base in {"goto", "goto_w"}:
            continue

        if base == "getstatic":
            name = _field_name(comment)
            stack.append(_Expr(name or "<?>"))
            continue

        if base == "getfield":
            name = _field_name(comment)
            obj = pop()
            if obj == "this" and name in field_objects:
                stack.append(field_objects[name])
            elif obj == "this" and name in scalar_fields:
                stack.append(_Expr(_format_number(scalar_fields[name])))
            elif isinstance(obj, _ObjRef):
                if name in _ROT_FIELDS:
                    axis = _ROT_FIELDS[name][1]
                    stack.append(state[obj.part.name][axis])
                elif name in _POS_FIELDS:
                    axis = _POS_FIELDS[name][1]
                    stack.append(state[obj.part.name]["p" + axis])
                else:
                    stack.append(obj)
            else:
                stack.append(_Expr(f"{obj}.{name}"))
            continue

        if base == "putfield":
            name = _field_name(comment)
            value = pop()
            owner = pop()
            if isinstance(owner, _ObjRef) and isinstance(value, (_Expr, int, float)):
                target = None
                if name in _ROT_FIELDS:
                    target = ("rotation", _ROT_FIELDS[name][1])
                elif name in _POS_FIELDS:
                    target = ("offset", _POS_FIELDS[name][1])
                if target:
                    family, axis = target
                    key = axis if family == "rotation" else "p" + axis
                    expr = value if isinstance(value, _Expr) else _Expr(_format_number(value))
                    state[owner.part.name][key] = expr
                    formulas.append(
                        {
                            "bone": owner.part.name,
                            "channel": family,
                            "axis": axis,
                            "expression": expr.text,
                            "bytecode_offset": offset,
                        }
                    )
            continue

        if base in ("fadd", "iadd", "dadd"):
            b, a = pop(), pop()
            stack.append(_Expr(f"({_expr_text(a)} + {_expr_text(b)})"))
            continue
        if base in ("fsub", "isub", "dsub"):
            b, a = pop(), pop()
            stack.append(_Expr(f"({_expr_text(a)} - {_expr_text(b)})"))
            continue
        if base in ("fmul", "imul", "dmul"):
            b, a = pop(), pop()
            stack.append(_Expr(f"({_expr_text(a)} * {_expr_text(b)})"))
            continue
        if base in ("fdiv", "idiv", "ddiv"):
            b, a = pop(), pop()
            stack.append(_Expr(f"({_expr_text(a)} / {_expr_text(b)})"))
            continue
        if base in ("fneg", "ineg", "dneg"):
            a = pop()
            stack.append(_Expr(f"-({_expr_text(a)})"))
            continue
        if base in ("f2d", "d2f", "i2f", "i2d"):
            continue

        if not base.startswith("invoke"):
            continue
        info = _method_info(comment)
        if not info:
            continue
        owner, name, desc = info
        argc = _descriptor_arg_count(desc)

        if base == "invokestatic":
            if name in ("func_76126_a", "sin") and ("MathHelper" in owner or owner.endswith("/Math") or owner.endswith(".Math")):
                a = pop()
                stack.append(_Expr(f"sin({_expr_text(a)})"))
            elif name in ("func_76134_b", "cos") and ("MathHelper" in owner or owner.endswith("/Math") or owner.endswith(".Math")):
                a = pop()
                stack.append(_Expr(f"cos({_expr_text(a)})"))
            elif name == "abs":
                a = pop()
                stack.append(_Expr(f"abs({_expr_text(a)})"))
            else:
                args = [pop() for _ in range(argc)][::-1]
                # Rendering/GL calls do not influence the model pose; consume
                # their operands quietly so they cannot contaminate later math.
                harmless = owner.startswith("org/lwjgl/opengl/") or name.startswith("gl")
                if not harmless:
                    warnings.append(f"Unsupported static call {owner}.{name}; expression may be incomplete.")
                if _descriptor_returns_value(desc):
                    stack.append(_Expr(f"{name}({', '.join(_expr_text(a) for a in args)})"))
            continue

        if name in ("setRotateAngle", "setRotationAngle", "setRotation", "setAngles") and len(stack) >= 5:
            rz = pop()
            ry = pop()
            rx = pop()
            obj = pop()
            _this = pop()
            if isinstance(obj, _ObjRef):
                for axis, value in (("x", rx), ("y", ry), ("z", rz)):
                    expr = value if isinstance(value, _Expr) else _Expr(_expr_text(value))
                    state[obj.part.name][axis] = expr
                    formulas.append({
                        "bone": obj.part.name,
                        "channel": "rotation",
                        "axis": axis,
                        "expression": expr.text,
                        "bytecode_offset": offset,
                    })
            continue

        args = [pop() for _ in range(argc)][::-1]
        receiver = pop()
        harmless = (
            "ModelBase" in owner and name in {"func_78088_a", "render", "func_78087_a", "setRotationAngles"}
        ) or (
            "ModelRenderer" in owner and name in {"func_78785_a", "render", "func_78791_b", "postRender"}
        )

        # Legacy models frequently move complex animation logic into helper methods
        # such as ModelTheKing.moveLeftHead(...). Those helpers mutate the same
        # ModelRenderer fields as the main render/setRotationAngles method. Inline
        # their recovered assignments with the call-site expressions so the baked
        # animation and generated Java both retain those channels.
        owner_simple = _simple_class_name(owner) if owner else current_class
        helper = None
        if (
            helper_methods
            and helper_depth < 3
            and receiver == "this"
            and (not owner or not current_class or owner_simple == current_class)
            and not harmless
            and not _descriptor_returns_value(desc)
        ):
            matches = [
                candidate
                for candidate in helper_methods
                if candidate is not method
                and re.search(rf"\b{re.escape(name)}\s*\(", candidate.header)
                and _method_param_count_from_header(candidate.header) == argc
            ]
            if matches:
                helper = matches[0]

        if helper is not None:
            helper_calls.append((name, args, offset))
        elif not harmless:
            warnings.append(f"Unsupported instance call {owner}.{name}; expression may be incomplete.")
        if _descriptor_returns_value(desc):
            receiver_text = _expr_text(receiver)
            stack.append(_Expr(f"{receiver_text}.{name}({', '.join(_expr_text(a) for a in args)})"))

    # Parse same-class helper methods after the main method so all helper assignments
    # are merged into one formula stream and classified per axis with the caller's
    # runtime expressions substituted for helper parameters.
    if helper_calls and helper_methods:
        seen_helper_keys: set[tuple[str, str]] = set()
        for helper_name, helper_args, call_offset in helper_calls:
            matches = [
                candidate
                for candidate in helper_methods
                if re.search(rf"\b{re.escape(helper_name)}\s*\(", candidate.header)
                and _method_param_count_from_header(candidate.header) == len(helper_args)
            ]
            if not matches:
                continue
            helper = matches[0]
            key = (helper_name, ",".join(_expr_text(arg) for arg in helper_args))
            if key in seen_helper_keys:
                continue
            seen_helper_keys.add(key)
            helper_parameter_names = {index: _expr_text(value) for index, value in enumerate(helper_args, start=1)}
            helper_analysis, _helper_logic, _helper_baked, helper_warnings = _parse_animation(
                helper,
                parts,
                parameter_names=helper_parameter_names,
                scalar_fields=scalar_fields,
                helper_methods=helper_methods if helper_depth < 2 else None,
                current_class=current_class,
                helper_depth=helper_depth + 1,
            )
            helper_formulas = helper_analysis.get("formulas", []) if isinstance(helper_analysis, dict) else []
            for formula in helper_formulas:
                formula = dict(formula)
                formula["helper_call_offset"] = call_offset
                formulas.append(formula)
            warnings.extend(helper_warnings)

    by_bone: dict[str, dict[str, list[str]]] = {}
    for formula in formulas:
        by_bone.setdefault(formula["bone"], {}).setdefault(formula["channel"], []).append(
            formula["expression"]
        )

    # Animation classification must happen per formula/axis, not once per bone.
    # A single bone may legitimately have X driven by walking while Y/Z are driven
    # by idle motion or head tracking.  Classifying only by bone causes one channel
    # to hide the others during baking.
    formula_classification: dict[int, str] = {}
    bone_animation_classes: dict[str, set[str]] = {}
    for index, formula in enumerate(formulas):
        expr = formula["expression"]
        has_head = any(name in expr for name in ("netHeadYaw", "headPitch"))
        has_walk = "limbSwing" in expr or "limbSwingAmount" in expr
        has_idle = "ageInTicks" in expr
        if has_head and (has_walk or has_idle):
            category = "combined"
        elif has_head:
            category = "head_tracking"
        elif has_walk and has_idle:
            category = "combined"
        elif has_walk:
            category = "walk_cycle"
        elif has_idle:
            category = "idle"
        else:
            category = "static"
        formula["animation_class"] = category
        formula_classification[index] = category
        if category not in ("static",):
            bone_animation_classes.setdefault(formula["bone"], set()).add(category)

    # Keep the legacy single-value classification field for compatibility, but make
    # mixed-animation bones explicit instead of silently choosing the first class.
    classification: dict[str, Any] = {}
    for bone, classes in bone_animation_classes.items():
        ordered = sorted(classes)
        classification[bone] = ordered[0] if len(ordered) == 1 else ordered

    analysis = {
        "source_method": method.header,
        "parameter_mapping": {
            name: {
                "slot": slot,
                "meaning": {
                    "limbSwing": "model movement phase",
                    "limbSwingAmount": "movement amplitude",
                    "ageInTicks": "render age / idle time",
                    "netHeadYaw": "head yaw in degrees",
                    "headPitch": "head pitch in degrees",
                    "scaleFactor": "legacy render scale parameter",
                    "entity": "rendered entity",
                }.get(name, "method parameter"),
            }
            for slot, name in sorted(parameter_names.items())
        },
        "formula_count": len(formulas),
        "bones": by_bone,
        "classification": classification,
        "formula_classification": {str(k): v for k, v in formula_classification.items()},
        "formulas": formulas,
        "warnings": warnings,
    }

    logic_lines = [
        "// Generated by MojModelBridge from recovered legacy ModelBase animation bytecode.",
        "// This is designed for a modern Mojmap/NeoForge-style ModelPart model.",
        "// Bind-pose rotations remain in the model definition; this method applies runtime animation.",
        "",
        "public void setupAnim(Entity entity, float limbSwing, float limbSwingAmount, float ageInTicks,",
        "                      float netHeadYaw, float headPitch) {",
    ]

    if any("scaleFactor" in f["expression"] for f in formulas):
        logic_lines.extend(
            [
                "    // Legacy setRotationAngles had a scaleFactor parameter; modern setupAnim does not.",
                "    final float scaleFactor = 1.0F;",
            ]
        )

    seen_lines: set[str] = set()
    for formula in formulas:
        expr = _java_expr(formula["expression"])
        bone = formula["bone"]
        channel = formula["axis"]
        if channel in ("x", "y", "z"):
            if _source_axis_sign(channel) < 0:
                expr = f"-({expr})"
            line = f"    this.{bone}.{channel}Rot = {expr};"
        else:
            line = f"    this.{bone}.{channel} = {expr};"
        if line not in seen_lines:
            logic_lines.append(line)
            seen_lines.add(line)

    logic_lines.append("}")
    logic = "\n".join(logic_lines) + "\n"

    baked = _build_baked_preview(analysis, parts)
    return analysis, logic, baked, warnings


def _build_baked_preview(
    analysis: dict[str, Any],
    parts: dict[str, _LegacyPart],
) -> dict[str, Any]:
    """Bake recoverable procedural animation formulas into Blockbench-native deltas.

    The static converted model is treated as the authoritative neutral pose. Legacy
    ModelRenderer code often writes absolute rotationPoint/rotateAngle values in a
    render method, and those values can be far from the constructor bind pose at
    ageInTicks/limbSwing == 0. To avoid the animation starting with a malformed pose,
    each animated axis is rebased to its own first sampled target:

        animation_delta(t) = target(t) - target(t=0)

    This keeps the static model intact at the first animation frame while preserving
    the source motion shape over the rest of the clip.
    """

    formulas = analysis.get("formulas", [])
    if not isinstance(formulas, list):
        formulas = []
    source_has_open_closed = bool(analysis.get("source_has_open_closed", False))
    source_has_activity = bool(analysis.get("source_has_activity", False))

    axis_index = {"x": 0, "y": 1, "z": 2}

    def bind_rotation_deg(bone: str) -> list[float]:
        part = parts.get(bone)
        if part is None:
            return [0.0, 0.0, 0.0]
        source = tuple(math.degrees(float(part.rotation[i])) for i in range(3))
        transformed = _source_rotation_to_blockbench_degrees(*source)
        return [float(v) for v in transformed]

    def bind_offset(bone: str) -> list[float]:
        part = parts.get(bone)
        if part is None:
            return [0.0, 0.0, 0.0]
        return [float(v) for v in part.offset]

    neutral_values: dict[tuple[str, str, str], float] = {}
    for part in parts.values():
        variable_name = getattr(part, "variable", part.name)
        for axis, value in zip(("x", "y", "z"), part.rotation):
            neutral_values[(part.name, "rotation", axis)] = float(value)
            neutral_values[(variable_name, "rotation", axis)] = float(value)
        for axis, value in zip(("x", "y", "z"), part.offset):
            neutral_values[(part.name, "position", axis)] = float(value)
            neutral_values[(variable_name, "position", axis)] = float(value)

    def _rotate_point(point: tuple[float, float, float], rotation: tuple[float, float, float]) -> tuple[float, float, float]:
        x, y, z = point
        rx, ry, rz = rotation
        cx, sx = math.cos(rx), math.sin(rx)
        cy, sy = math.cos(ry), math.sin(ry)
        cz, sz = math.cos(rz), math.sin(rz)
        y, z = y * cx - z * sx, y * sx + z * cx
        x, z = x * cy + z * sy, -x * sy + z * cy
        x, y = x * cz - y * sz, x * sz + y * cz
        return (x, y, z)

    def _inverse_rotate_point(point: tuple[float, float, float], rotation: tuple[float, float, float]) -> tuple[float, float, float]:
        rx, ry, rz = rotation
        x, y, z = point
        # Inverse of the forward X -> Y -> Z application above.
        x, y = x * math.cos(rz) + y * math.sin(rz), -x * math.sin(rz) + y * math.cos(rz)
        x, z = x * math.cos(ry) - z * math.sin(ry), x * math.sin(ry) + z * math.cos(ry)
        y, z = y * math.cos(rx) + z * math.sin(rx), -y * math.sin(rx) + z * math.cos(rx)
        return (x, y, z)

    def _part_joint_local(part: Optional[_LegacyPart], direction_world: tuple[float, float, float], want_far: bool) -> tuple[float, float, float]:
        if part is None or not part.cubes:
            return (0.0, 0.0, 0.0)
        bind_dir_local = _inverse_rotate_point(direction_world, tuple(float(v) for v in part.rotation))
        length = math.sqrt(sum(v * v for v in bind_dir_local)) or 1.0
        unit = tuple(v / length for v in bind_dir_local)
        corners: list[tuple[float, float, float]] = []
        for cube in part.cubes:
            x, y, z, dx, dy, dz, _u, _v, inflate, _mirror = cube
            inf = float(inflate)
            min_x, max_x = x - inf, x + dx + inf
            min_y, max_y = y - inf, y + dy + inf
            min_z, max_z = z - inf, z + dz + inf
            corners.extend((cx, cy, cz) for cx in (min_x, max_x) for cy in (min_y, max_y) for cz in (min_z, max_z))
        scorer = lambda c: c[0] * unit[0] + c[1] * unit[1] + c[2] * unit[2]
        return (max if want_far else min)(corners, key=scorer)

    def _infer_joint_relations(state_formulas: list[dict[str, Any]]) -> dict[str, str]:
        # Joint continuity is deliberately selective. It works extremely well for
        # segmented wings whose source explicitly places the next segment from the
        # previous wing segment, but applying the same geometric correction to tails,
        # necks, heads, arms, or legs can fight the source-authored rotation-point
        # system and produce worse disjointing. Only wing/fan/feather-like part chains
        # opt in automatically; other body parts retain their exact source positions.
        votes: dict[tuple[str, str], set[str]] = {}
        ref_re = re.compile(r"\b([A-Za-z_$][\w$]*)\.(?:x|y|z)\b")
        def wingish(name: str) -> bool:
            n = re.sub(r"[^a-z0-9]", "", str(name).lower())
            # User-facing rule: geometric joint continuity is ONLY for wing chains.
            # Fins/feathers/membranes/tails/limbs can have intentionally authored
            # pivots and must keep their exact source positions.
            return "wing" in n
        for formula in state_formulas:
            if str(formula.get("channel", "")) not in {"position", "offset"}:
                continue
            child = str(formula.get("bone", ""))
            axis = str(formula.get("axis", ""))
            if not wingish(child):
                continue
            for parent in set(ref_re.findall(str(formula.get("expression", "")))):
                if parent == child or parent not in parts or child not in parts:
                    continue
                if not wingish(parent):
                    continue
                votes.setdefault((child, parent), set()).add(axis)
        result: dict[str, str] = {}
        for (child, parent), axes in sorted(votes.items(), key=lambda item: (-len(item[1]), item[0])):
            if len(axes) >= 2:
                result.setdefault(child, parent)
        return result

    def eval_animation_formula_set(
        matching: list[dict[str, Any]],
        times: list[tuple[float, dict[str, float]]],
    ) -> dict[str, Any]:
        anim_bones: dict[str, dict[str, dict[str, list[float]]]] = {}

        # Simulate the source method's mutable ModelRenderer state for each sample.
        # This mirrors the actual legacy execution model more closely than recursively
        # expanding expressions: assignments happen in source order, and later
        # segments immediately observe the current values of earlier segments.
        channel_fields = {
            "field_78795_f": ("rotation", "x"), "rotateAngleX": ("rotation", "x"), "xRot": ("rotation", "x"),
            "field_78796_g": ("rotation", "y"), "rotateAngleY": ("rotation", "y"), "yRot": ("rotation", "y"),
            "field_78808_h": ("rotation", "z"), "rotateAngleZ": ("rotation", "z"), "zRot": ("rotation", "z"),
            "field_78800_c": ("position", "x"), "rotationPointX": ("position", "x"), "x": ("position", "x"),
            "field_78797_d": ("position", "y"), "rotationPointY": ("position", "y"), "y": ("position", "y"),
            "field_78798_e": ("position", "z"), "rotationPointZ": ("position", "z"), "z": ("position", "z"),
        }
        ref_pattern = re.compile(r"\b([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)\b")

        def canonical_channel(channel: str) -> str:
            return "rotation" if channel == "rotation" else "position"

        def prepare_state_formulas(state_formulas: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], bool]:
            prepared: list[dict[str, Any]] = []
            formula_positions = {
                (str(f.get("bone", "")), canonical_channel(str(f.get("channel", ""))), str(f.get("axis", ""))): index
                for index, f in enumerate(state_formulas)
            }
            needs_second_pass = False
            for index, formula in enumerate(state_formulas):
                channel = canonical_channel(str(formula.get("channel", "")))
                axis = str(formula.get("axis", ""))
                bone = str(formula.get("bone", ""))
                if channel not in {"rotation", "position"} or axis not in axis_index:
                    continue
                expression = str(formula.get("expression", "0.0"))
                refs: list[tuple[str, str, str]] = []

                def replace_ref(match: re.Match[str]) -> str:
                    ref_bone, field_name = match.groups()
                    mapped = channel_fields.get(field_name)
                    if mapped is None:
                        return match.group(0)
                    ref_channel, ref_axis = mapped
                    ref_key = (ref_bone, canonical_channel(ref_channel), ref_axis)
                    alias = f"__ref_{len(refs)}"
                    refs.append(ref_key)
                    ref_index = formula_positions.get(ref_key)
                    if ref_index is not None and ref_index > index:
                        nonlocal needs_second_pass
                        needs_second_pass = True
                    return alias

                expression = ref_pattern.sub(replace_ref, expression)
                try:
                    if re.search(r"/\s*\(?entity_state\)?", expression):
                        expression = re.sub(r"/\s*\(?entity_state\)?", "/ entity_scalar", expression)
                    code = compile(expression, "<mojmodelbridge-animation>", "eval")
                except Exception:
                    code = None
                prepared.append({
                    "bone": bone,
                    "channel": channel,
                    "axis": axis,
                    "refs": refs,
                    "same_channel_refs": [
                        ref for ref in refs
                        if ref[1] == channel and ref[2] == axis
                    ],
                    "code": code,
                    "expression": expression,
                })
            return prepared, needs_second_pass

        SAFE_EVAL_GLOBALS = {"__builtins__": {}, "sin": math.sin, "cos": math.cos, "abs": abs, "radians": math.radians, "degrees": math.degrees, "pow": pow, "attacking": 0.0, "sprinting": 0.0, "baby": 0.0, "entity_state": 0.0, "entity_scalar": 1.0}

        def evaluate_state(
            values: dict[str, float],
            prepared_formulas: list[dict[str, Any]],
            needs_second_pass: bool,
            animation_kind: str,
        ) -> dict[tuple[str, str, str], float]:
            state = dict(neutral_values)
            pass_count = 2 if needs_second_pass else 1
            for _ in range(pass_count):
                changed = False
                for formula in prepared_formulas:
                    code = formula["code"]
                    if code is None:
                        continue
                    env = dict(values)
                    # For baked previews the neutral runtime state should be
                    # non-attacking/non-special unless a dedicated attack clip is
                    # recovered. Previously this was hard-coded to 1.0, which
                    # forced ternaries like getAttacking()==0 onto their attack
                    # branch and distorted idle animations.
                    env["entity_scalar"] = 1.0
                    bone_text = re.sub(r"[^a-z0-9]", "", str(formula.get("bone", "")).lower())
                    # When a legacy model gates wing motion on an activity/attack
                    # getter, bake the animated wing branch for the native clip while
                    # leaving jaws/body state getters at their neutral value. This is
                    # source-pattern driven rather than a mod-specific exception.
                    # Default baked clips represent the neutral runtime state.
                    # Do NOT force wing parts onto an alternate activity/attack
                    # ternary branch: that made models such as PitchBlack bake the
                    # attack wing pose into their idle clip while jaws/body stayed
                    # neutral. Dedicated attack clips can opt in explicitly.
                    if source_has_open_closed:
                        # Native target entities do not expose the legacy open/closed
                        # sync field. The source's active/open state is the only state
                        # that produces the moving Triffid structure, so bake that as
                        # the base animation state rather than locking it shut.
                        env["entity_state"] = 1.0
                    elif source_has_activity and ("wing" in bone_text or "mem" in bone_text):
                        # Flying models such as PitchBlack gate wing motion on
                        # getActivity(); keep body/jaw formulas neutral while using
                        # the actual wing-flapping branch for the baked native clip.
                        env["entity_state"] = 1.0
                    else:
                        env["entity_state"] = 1.0 if str(animation_kind).lower() in {"attack", "attacking", "attack_cycle"} else 0.0
                    for index, ref_key in enumerate(formula["refs"]):
                        env[f"__ref_{index}"] = float(state.get(ref_key, 0.0))
                    try:
                        result = float(eval(code, SAFE_EVAL_GLOBALS, env))
                    except Exception:
                        continue
                    key = (formula["bone"], formula["channel"], formula["axis"])
                    previous = float(state.get(key, 0.0))
                    if abs(result - previous) > 1e-9:
                        state[key] = result
                        changed = True
                if not changed:
                    break
            return state

        # Group all source assignments by bone and axis. Direct ModelRenderer
        # rotation assignments are absolute targets. For a baked clip, the last
        # assignment belonging to that clip wins; if the source method explicitly
        # writes a static/reset value to the axis, that value becomes the fallback;
        # only otherwise do we retain the constructor bind pose.
        grouped: dict[tuple[str, str, str], list[dict[str, Any]]] = {}
        for f in formulas:
            if f.get("channel") not in {"rotation", "offset", "position"} or f.get("axis") not in axis_index:
                continue
            grouped.setdefault((str(f["bone"]), str(f["channel"]), str(f["axis"])), []).append(f)

        allowed_classes = {"static", "combined", kind}
        state_formulas = [f for f in formulas if str(f.get("animation_class", "static")) in allowed_classes]
        prepared_state_formulas, needs_second_pass = prepare_state_formulas(state_formulas)
        joint_relations = _infer_joint_relations(state_formulas)

        # The static converted model is the authoritative bind pose. Animated
        # source fields are absolute ModelPart/ModelRenderer values, so the native
        # Blockbench channel must contain `source_target - bind_pose`, not
        # `source_target - first_sample`. Rebasing to the first sample makes a source
        # expression such as `1.22 + cos(t)*0.15` become a -0.30..0 swing instead of
        # the correct -0.15..+0.15 swing around the authored bind rotation. It also
        # breaks chained positions because every segment gets a different arbitrary
        # zero point.
        baseline_targets: dict[tuple[str, str, str], float] = {}
        for (bone, source_channel, axis), fs in grouped.items():
            part = parts.get(bone)
            try:
                if source_channel == "rotation":
                    bind_value = float(part.rotation[axis_index[axis]]) if part is not None else 0.0
                    baseline_targets[(bone, source_channel, axis)] = math.degrees(bind_value) * _source_axis_sign(axis)
                else:
                    bind_value = float(part.offset[axis_index[axis]]) if part is not None else 0.0
                    baseline_targets[(bone, source_channel, axis)] = bind_value * _source_position_axis_sign(axis)
            except Exception:
                baseline_targets[(bone, source_channel, axis)] = 0.0

        for time, values in times:
            per_bone: dict[str, dict[str, list[float]]] = {}
            state = evaluate_state(values, prepared_state_formulas, needs_second_pass, kind)

            joint_position_targets: dict[str, tuple[float, float, float]] = {}
            for child_name, parent_name in joint_relations.items():
                child_part = parts.get(child_name)
                parent_part = parts.get(parent_name)
                if child_part is None or parent_part is None:
                    continue
                parent_bind = tuple(float(v) for v in parent_part.offset)
                child_bind = tuple(float(v) for v in child_part.offset)
                direction = tuple(child_bind[i] - parent_bind[i] for i in range(3))
                if math.sqrt(sum(v * v for v in direction)) < 1e-6:
                    continue
                parent_rot = tuple(float(state.get((parent_name, "rotation", axis), parent_part.rotation[axis_index[axis]])) for axis in ("x", "y", "z"))
                child_rot = tuple(float(state.get((child_name, "rotation", axis), child_part.rotation[axis_index[axis]])) for axis in ("x", "y", "z"))
                parent_pos = tuple(float(state.get((parent_name, "position", axis), parent_part.offset[axis_index[axis]])) for axis in ("x", "y", "z"))
                parent_joint_local = _part_joint_local(parent_part, direction, True)
                child_joint_local = _part_joint_local(child_part, direction, False)
                rotated_parent_joint = _rotate_point(parent_joint_local, parent_rot)
                rotated_child_joint = _rotate_point(child_joint_local, child_rot)
                joint_position_targets[child_name] = tuple(parent_pos[i] + rotated_parent_joint[i] - rotated_child_joint[i] for i in range(3))

            for (bone, source_channel, axis), fs in grouped.items():
                relevant = [
                    f for f in fs
                    if f.get("animation_class") == kind
                    or f.get("animation_class") == "combined"
                ]
                if not relevant:
                    continue
                f = relevant[-1]
                output_channel = "rotation" if source_channel == "rotation" else "position"
                delta = per_bone.setdefault(bone, {}).setdefault(output_channel, [0.0, 0.0, 0.0])
                try:
                    if source_channel == "position" and bone in joint_position_targets:
                        source_value = float(joint_position_targets[bone][axis_index[axis]])
                    else:
                        source_value = float(state.get((bone, canonical_channel(source_channel), axis), 0.0))
                    transformed_target = (
                        math.degrees(source_value) * _source_axis_sign(axis)
                        if source_channel == "rotation"
                        else source_value * _source_position_axis_sign(axis)
                    )
                    baseline = baseline_targets.get((bone, source_channel, axis), 0.0)
                    delta[axis_index[axis]] = transformed_target - baseline
                except Exception:
                    pass

            for bone, bone_channels in per_bone.items():
                for output_channel, vec in bone_channels.items():
                    channel = anim_bones.setdefault(bone, {}).setdefault(output_channel, {})
                    channel[f"{time:.6f}"] = [round(float(vec[0]), 6), round(float(vec[1]), 6), round(float(vec[2]), 6)]

        # Exact source aliases such as `jaw1.xRot = jaw5.xRot` and `tooth.x =
        # jaw.x` are not new animation formulas. They are hard links to the source
        # part's current channel. Copy the referenced channel after evaluation so
        # the alias cannot acquire a separate bind-pose offset or sampling error.
        alias_re = re.compile(r"^\s*([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)\s*$")
        field_to_channel = {
            "field_78795_f": ("rotation", "x"), "rotateAngleX": ("rotation", "x"), "xRot": ("rotation", "x"),
            "field_78796_g": ("rotation", "y"), "rotateAngleY": ("rotation", "y"), "yRot": ("rotation", "y"),
            "field_78808_h": ("rotation", "z"), "rotateAngleZ": ("rotation", "z"), "zRot": ("rotation", "z"),
            "field_78800_c": ("position", "x"), "rotationPointX": ("position", "x"), "x": ("position", "x"),
            "field_78797_d": ("position", "y"), "rotationPointY": ("position", "y"), "y": ("position", "y"),
            "field_78798_e": ("position", "z"), "rotationPointZ": ("position", "z"), "z": ("position", "z"),
        }
        formulas_now = [f for f in matching if f.get("animation_class") in {kind, "combined"}]
        for formula in formulas_now:
            target_bone = str(formula.get("bone", ""))
            expr = str(formula.get("expression", ""))
            m_alias = alias_re.fullmatch(expr)
            if not m_alias:
                continue
            ref_bone, ref_field = m_alias.groups()
            mapped = field_to_channel.get(ref_field)
            if mapped is None:
                continue
            channel_name, _axis = mapped
            target_channels = anim_bones.get(target_bone, {})
            ref_channels = anim_bones.get(ref_bone, {})
            if channel_name in ref_channels:
                target_channels[channel_name] = {k: list(v) for k, v in ref_channels[channel_name].items()}
                anim_bones[target_bone] = target_channels

        # Mirrored anatomy fallback: when one side of a clearly paired left/right
        # limb has recoverable motion and the counterpart is effectively static,
        # mirror the recovered source motion. This is only used for a missing/zero
        # counterpart, never to overwrite two independently animated sides.
        def mirror_name(name: str) -> Optional[str]:
            raw = str(name)
            variants = []
            if re.search(r"(?i)left", raw): variants.append(re.sub(r"(?i)left", "right", raw, count=1))
            if re.search(r"(?i)right", raw): variants.append(re.sub(r"(?i)right", "left", raw, count=1))
            if raw.startswith("L") and len(raw) > 1: variants.append("R" + raw[1:])
            if raw.startswith("R") and len(raw) > 1: variants.append("L" + raw[1:])
            if raw.startswith("l") and len(raw) > 1: variants.append("r" + raw[1:])
            if raw.startswith("r") and len(raw) > 1: variants.append("l" + raw[1:])
            return next((candidate for candidate in variants if candidate in anim_bones or candidate in parts), None)

        def motion_mag(channel_data: dict[str, list[float]]) -> float:
            return max((max(abs(float(v)) for v in vec) for vec in channel_data.values()), default=0.0)

        for source_name in list(anim_bones):
            counterpart = mirror_name(source_name)
            if not counterpart or counterpart == source_name:
                continue
            source_channels = anim_bones.get(source_name, {})
            target_channels = anim_bones.setdefault(counterpart, {})
            for channel_name, keyframes in source_channels.items():
                if not isinstance(keyframes, dict):
                    continue
                existing = target_channels.get(channel_name, {})
                if existing and motion_mag(existing) > 1e-3:
                    continue
                mirrored: dict[str, list[float]] = {}
                for time_key, vec in keyframes.items():
                    out = [float(vec[0]), float(vec[1]), float(vec[2])]
                    if channel_name == "position":
                        out[0] = -out[0]
                    elif channel_name == "rotation":
                        out[1] = -out[1]
                        out[2] = -out[2]
                    mirrored[time_key] = out
                if motion_mag(mirrored) > 1e-3:
                    target_channels[channel_name] = mirrored

        return anim_bones

    def _linear_coefficients_in_trig(expr: str, variable: str) -> list[float]:
        """Find linear ``variable`` coefficients inside sin/cos arguments.

        This is intentionally conservative: it only walks arithmetic expressions that
        remain linear in the sampled animation parameter. Nested trig calls are scanned
        recursively, which handles formulas like ``cos(cos(ageInTicks * 0.35 * 0.65) * ...)``.
        """
        import ast

        normalized = str(expr).replace("Mth.sin", "sin").replace("Mth.cos", "cos")
        normalized = normalized.replace("Math.sin", "sin").replace("Math.cos", "cos")
        normalized = normalized.replace("Math.abs", "abs")

        def linear(node: ast.AST) -> Optional[float]:
            if isinstance(node, ast.Name) and node.id == variable:
                return 1.0
            if isinstance(node, ast.Constant):
                return 0.0
            if isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.UAdd, ast.USub)):
                value = linear(node.operand)
                return None if value is None else (-value if isinstance(node.op, ast.USub) else value)
            if isinstance(node, ast.BinOp):
                left = linear(node.left)
                right = linear(node.right)
                if isinstance(node.op, (ast.Add, ast.Sub)) and left is not None and right is not None:
                    return left + right if isinstance(node.op, ast.Add) else left - right
                if isinstance(node.op, (ast.Mult, ast.Div)):
                    # One side must be constant and the other linear.
                    if left is not None and right == 0.0:
                        try:
                            const = float(ast.literal_eval(node.right))
                            return left * const if isinstance(node.op, ast.Mult) else left / const
                        except Exception:
                            pass
                    if right is not None and left == 0.0 and isinstance(node.op, ast.Mult):
                        try:
                            const = float(ast.literal_eval(node.left))
                            return right * const
                        except Exception:
                            pass
                return None
            return None

        coefficients: list[float] = []
        # Java animation sources commonly contain several assignments separated by
        # semicolons/newlines. Parse each expression independently so one unrelated
        # construct cannot make the whole clock analysis fail.
        for statement in re.split(r"[;\n]+", normalized):
            statement = statement.strip()
            if not statement:
                continue
            candidate = statement.split("=", 1)[1].strip() if "=" in statement else statement
            try:
                tree = ast.parse(candidate, mode="eval")
            except Exception:
                continue
            for node in ast.walk(tree):
                if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id in {"sin", "cos"} and node.args:
                    coeff = linear(node.args[0])
                    if coeff is not None and abs(coeff) > 1e-8:
                        coefficients.append(abs(float(coeff)))
        # Linear modulo clocks such as `ageInTicks * 0.7 % (2*pi)` also define
        # an explicit repeating period even though they are not inside sin/cos.
        modulo = re.compile(
            rf"\b{re.escape(variable)}\s*\*\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)\s*%"
        )
        for match in modulo.finditer(normalized):
            try:
                coeff = abs(float(match.group(1)))
                if coeff > 1e-8:
                    coefficients.append(coeff)
            except ValueError:
                pass
        return coefficients

    def _periods_seconds_for(variable: str) -> list[float]:
        coefficients = _linear_coefficients_in_trig("\n".join(str(f.get("expression", "")) for f in formulas), variable)
        periods: list[float] = []
        for coeff in coefficients:
            period = (2.0 * math.pi) / (abs(coeff) * 20.0)
            if 0.08 <= period <= 12.0:
                periods.append(period)
        return sorted(periods)

    def _robust_period(periods: list[float], default: float) -> float:
        if not periods:
            return default
        # Prefer the shortest strong source period. Using a percentile of all
        # incidental frequencies was stretching some clips to 5-15 seconds.
        periods = sorted(p for p in periods if 0.08 <= p <= 6.0)
        if not periods:
            return default
        return min(periods)

    def _best_loop_period(kind: str, matching: list[dict[str, Any]]) -> float:
        variable = "limbSwing" if kind == "walk_cycle" else "ageInTicks"
        raw_periods = _periods_seconds_for(variable)
        if not raw_periods:
            return 1.0
        from fractions import Fraction
        # Find a compact rational common period for all source angular frequencies.
        # This avoids picking the fastest component and forcibly snapping unrelated
        # slower oscillators at the end of the clip.
        frequencies = [1.0 / p for p in raw_periods if p > 1e-8]
        if not frequencies:
            return 1.0
        base = min(frequencies)
        ratios = [Fraction(f / base).limit_denominator(128) for f in frequencies]
        lcm = 1
        for frac in ratios:
            lcm = math.lcm(lcm, int(frac.denominator))
        candidate = lcm / base
        if 0.08 <= candidate <= 12.0:
            return float(candidate)
        # Fall back to a sampled candidate search if the mathematically common
        # period is impractically long.
        candidates = sorted({round(p * m, 6) for p in raw_periods for m in (1,2,3,4,6,8,12) if 0.08 <= p*m <= 12.0})
        if not candidates:
            return 1.0
        best, best_error = candidates[0], float("inf")
        for cand in candidates:
            try:
                preview = eval_animation_formula_set(matching, make_times(kind, cand, sample_count=64))
                error = _endpoint_error(preview)
            except Exception:
                continue
            if error < best_error:
                best, best_error = cand, error
            if error < 0.05:
                break
        return best

    def _idle_period_seconds(matching: list[dict[str, Any]] | None = None) -> float:
        return _best_loop_period("idle", matching or formulas)

    def _walk_period_seconds(matching: list[dict[str, Any]] | None = None) -> float:
        return _best_loop_period("walk_cycle", matching or formulas)

    def make_times(kind: str, length_override: float | None = None, sample_count: int = 128) -> list[tuple[float, dict[str, float]]]:
        sample_count = max(2, int(sample_count))
        if kind == "walk_cycle":
            length = float(length_override if length_override is not None else _walk_period_seconds())
            return [
                (
                    length * i / sample_count,
                    {
                        "limbSwing": 2.0 * math.pi * i / sample_count,
                        "limbSwingAmount": 1.0,
                        "ageInTicks": 20.0 * (length * i / sample_count),
                        "netHeadYaw": 0.0,
                        "headPitch": 0.0,
                    },
                )
                for i in range(sample_count + 1)
            ]
        if kind == "idle":
            length = float(length_override if length_override is not None else _idle_period_seconds())
            return [
                (
                    length * i / sample_count,
                    {
                        "limbSwing": 0.0,
                        "limbSwingAmount": 0.0,
                        "ageInTicks": 20.0 * (length * i / sample_count),
                        "netHeadYaw": 0.0,
                        "headPitch": 0.0,
                    },
                )
                for i in range(sample_count + 1)
            ]
        return []

    def _endpoint_error(anim_bones: dict[str, Any]) -> float:
        max_error = 0.0
        if not anim_bones:
            return float("inf")
        for bone_channels in anim_bones.values():
            for channel_keys in bone_channels.values():
                times = sorted(channel_keys, key=float)
                if not times:
                    continue
                first = channel_keys[times[0]]
                last = channel_keys[times[-1]]
                max_error = max(max_error, max(abs(float(a) - float(b)) for a, b in zip(first, last)))
        return max_error

    def _bake_closed_window(kind: str, matching: list[dict[str, Any]]) -> tuple[dict[str, Any], list[tuple[float, dict[str, float]]], bool]:
        # Bake one full source period but start at the first moving sample.
        # This preserves bind pose outside the animation and gives the loop equal
        # moving endpoints instead of snapping back to the initial pose.
        base_length = _best_loop_period(kind, matching)
        raw = make_times(kind, base_length * 129.0 / 128.0, sample_count=129)
        if len(raw) < 3:
            return eval_animation_formula_set(matching, raw), raw, True
        start_time = raw[1][0]
        final_times = [(float(t - start_time), dict(values)) for t, values in raw[1:]]
        final_times[-1] = (base_length, dict(final_times[0][1]))
        final_bones = eval_animation_formula_set(matching, final_times)
        # The clip begins on the first moving source sample (keyframe 2 of the
        # original model timeline). Close the seam to that same moving state, not
        # to the static bind pose.
        if final_bones and len(final_times) >= 2:
            for bone_channels in final_bones.values():
                for channel_keys in bone_channels.values():
                    ordered_keys = sorted(channel_keys, key=lambda value: float(value))
                    if len(ordered_keys) >= 2:
                        channel_keys[ordered_keys[-1]] = list(channel_keys[ordered_keys[0]])
        return final_bones, final_times, True

    animations: dict[str, Any] = {}
    for kind, name in (
        ("walk_cycle", "animation.model.walk"),
        ("idle", "animation.model.idle"),
    ):
        if kind == "walk_cycle":
            matching = [
                f for f in formulas
                if f.get("animation_class") == "walk_cycle"
                or f.get("animation_class") == "combined"
            ]
        else:
            matching = [
                f for f in formulas
                if f.get("animation_class") == "idle"
                or f.get("animation_class") == "combined"
            ]
        if not matching:
            continue
        anim_bones, times, loopable = _bake_closed_window(kind, matching)
        # If every channel is zero for the whole clip (common for movement-amplitude
        # formulas evaluated with limbSwingAmount=0 in the idle clip), omit the empty
        # animation rather than attaching a useless runtime animation.
        has_motion = any(
            any(any(abs(float(component)) > 1e-6 for component in vector) for vector in channel_keys.values())
            for bone_channels in anim_bones.values()
            for channel_keys in bone_channels.values()
        )
        if anim_bones and has_motion:
            length = times[-1][0] if times else 0.0
            animations[name] = {
                "loop": True,
                "bones": anim_bones,
                "moj_model_bridge": {
                    "source_class": kind,
                    "multi_axis_safe": True,
                    "bind_pose_preserved": True,
                    "source_animated_axes_are_absolute": True,
                    "blockbench_keyframes_are_additive_deltas": True,
                    "animation_is_rebased_to_static_bind_pose": True,
                    "rotation_transform": "source_xyz_to_converted_modelpart_xyz = (x, y, z)",
                    "position_transform": "source_rotation_point_delta_to_converted_modelpart_position = (x, y, z)",
                },
            }

    # If the source exposes animation formulas but no conventional walk/idle clip
    # can be recovered, do not discard the motion. Build one or more generic clips
    # from the remaining classified formula groups and name them anim1, anim2, ... .
    # This is deliberately a fallback: named AnimationDefinition/idle/walk clips
    # keep their normal names when they are recoverable.
    fallback_kinds_needed = sorted({
        str(f.get("animation_class", "static"))
        for f in formulas
        if str(f.get("animation_class", "static")) not in {"static", "walk_cycle", "idle", "combined"}
    })
    if not animations or fallback_kinds_needed:
        fallback_kinds = fallback_kinds_needed or sorted({
            str(f.get("animation_class", "static"))
            for f in formulas
            if str(f.get("animation_class", "static")) != "static"
        })

        def make_fallback_times() -> list[tuple[float, dict[str, float]]]:
            length = 2.0
            samples = 48
            return [
                (
                    length * i / samples,
                    {
                        "limbSwing": 2.0 * math.pi * i / samples,
                        "limbSwingAmount": 1.0,
                        "ageInTicks": 20.0 * length * i / samples,
                        "netHeadYaw": 35.0 * math.sin(2.0 * math.pi * i / samples),
                        "headPitch": 15.0 * math.sin(2.0 * math.pi * i / samples + math.pi / 2.0),
                    },
                )
                for i in range(samples + 1)
            ]

        fallback_times = make_fallback_times()
        for fallback_index, kind in enumerate(fallback_kinds, 1):
            fallback_formulas = [
                f for f in formulas
                if str(f.get("animation_class", "static")) == kind
                or str(f.get("animation_class", "static")) == "combined"
            ]
            if not fallback_formulas:
                continue
            fallback_bones = eval_animation_formula_set(fallback_formulas, fallback_times)
            has_motion = any(
                any(any(abs(float(component)) > 1e-6 for component in vector) for vector in channel_keys.values())
                for bone_channels in fallback_bones.values()
                for channel_keys in bone_channels.values()
            )
            if not fallback_bones or not has_motion:
                continue
            first_key = f"{fallback_times[0][0]:.6f}"
            final_key = f"{fallback_times[-1][0]:.6f}"
            max_endpoint_delta = 0.0
            for bone_channels in fallback_bones.values():
                for channel_keys in bone_channels.values():
                    first_value = channel_keys.get(first_key)
                    final_value = channel_keys.get(final_key)
                    if first_value is None or final_value is None:
                        continue
                    max_endpoint_delta = max(
                        max_endpoint_delta,
                        max(abs(float(a) - float(b)) for a, b in zip(first_value, final_value)),
                    )
            loopable = True
            for bone_channels in fallback_bones.values():
                for channel_keys in bone_channels.values():
                    ordered_keys = sorted(channel_keys, key=lambda value: float(value))
                    if len(ordered_keys) >= 2:
                        channel_keys[ordered_keys[-1]] = list(channel_keys[ordered_keys[0]])
            animations[f"anim{fallback_index}"] = {
                "loop": loopable,
                "bones": fallback_bones,
                "moj_model_bridge": {
                    "source_class": "fallback_procedural",
                    "source_animation_class": kind,
                    "bind_pose_preserved": True,
                    "blockbench_keyframes_are_additive_deltas": True,
                    "animation_is_rebased_to_static_bind_pose": True,
                },
            }

    return {
        "format_version": "1.8.0",
        "animations": animations,
        "moj_model_bridge": {
            "note": "Baked preview uses per-axis formula classification. Native Blockbench rotation keyframes are emitted as additive deltas from the converted bind pose; runtime-accurate entity-driven animation is in *_animation.java.",
            "source": "legacy ModelBase procedural animation",
            "supports_multiple_axes_per_bone": True,
            "supports_multiple_animation_classes_per_bone": True,
            "source_animated_axes_are_absolute": True,
            "blockbench_keyframes_are_additive_deltas": True,
            "animation_is_rebased_to_static_start_pose": True,
            "rotation_transform": "source_xyz_to_converted_modelpart_xyz = (x, y, z)",
        },
    }


def parse_legacy_class(
    source: Path,
    cube_cls: Any,
    part_cls: Any,
    parsed_model_cls: Any,
    resource_roots: Optional[list[Path]] = None,
    constructor_args: Optional[list[Any]] = None,
) -> Any:
    text = _run_javap(source)
    if not _find_model_renderer_constructor(text):
        raise LegacyParseError(
            "The .class does not appear to use a supported Minecraft 1.7.10-1.12.2 ModelBase hierarchy; "
            "expected a ModelBase/ModelRenderer entity model."
        )

    class_name = _parse_class_name(text)
    methods = _parse_methods(text)
    constructors = [m for m in methods if re.search(rf"\b{re.escape(class_name)}\s*\(", m.header)]
    if not constructors:
        raise LegacyParseError("Could not locate the model constructor.")

    parsed_constructors: list[tuple[dict[str, _LegacyPart], dict[str, int], dict[str, float], list[str], _Method]] = []
    for candidate in constructors:
        candidate_parts, candidate_texture, candidate_scalars, candidate_warnings = _parse_constructor(
            candidate, constructor_args=constructor_args
        )
        parsed_constructors.append(
            (candidate_parts, candidate_texture, candidate_scalars, candidate_warnings, candidate)
        )

    # Prefer the constructor that actually builds the largest geometry set.
    # This handles no-arg constructors delegating to float/boolean constructors,
    # and old models that expose only a parameterized constructor.
    raw_parts, texture, scalar_fields, warnings, constructor = max(
        parsed_constructors,
        key=lambda entry: (sum(len(part.cubes) for part in entry[0].values()), len(entry[0])),
    )
    if not constructor.header.endswith("();"):
        if constructor_args:
            warnings.append(
                f"Reconstructed parameterized constructor '{constructor.header}' using renderer-supplied arguments {constructor_args!r}."
            )
        else:
            warnings.append(
                f"Reconstructed parameterized constructor '{constructor.header}' using zero/false defaults for primitive arguments."
            )
    if not raw_parts:
        raise LegacyParseError(
            "No ModelRenderer fields were reconstructed. This class may use unsupported constructor patterns."
        )

    legacy_parts = list(raw_parts.values())
    name_to_part = {p.name: p for p in legacy_parts}

    parts: list[Any] = []
    legacy_to_modern: dict[str, Any] = {}

    for index, legacy in enumerate(sorted(legacy_parts, key=lambda p: p.source_index)):
        cubes = [
            cube_cls(
                x=x,
                y=y,
                z=z,
                dx=dx,
                dy=dy,
                dz=dz,
                uv_x=uv_x,
                uv_y=uv_y,
                mirror=cube_mirror,
                inflate=delta,
                name=f"{legacy.name}_cube" if i == 0 else f"{legacy.name}_cube_{i+1}",
            )
            for i, (x, y, z, dx, dy, dz, uv_x, uv_y, delta, cube_mirror) in enumerate(legacy.cubes)
        ]
        parent_name = legacy.parent
        part = part_cls(
            name=legacy.name,
            variable=legacy.name,
            parent_variable=parent_name or "root",
            offset=tuple(legacy.offset),
            rotation=tuple(legacy.rotation),
            cubes=cubes,
            source_index=index,
        )
        parts.append(part)
        legacy_to_modern[legacy.name] = part

    for legacy in legacy_parts:
        if legacy.parent and legacy.parent in legacy_to_modern:
            child = legacy_to_modern[legacy.name]
            parent = legacy_to_modern[legacy.parent]
            child.parent = parent
            parent.children.append(child)

    animation_candidates: list[tuple[int, dict[str, Any], str, dict[str, Any], list[str], _Method]] = []
    for candidate_method in _select_animation_methods(methods):
        try:
            candidate_analysis, candidate_logic, candidate_baked, candidate_warnings = _parse_animation(
                candidate_method,
                raw_parts,
                parameter_names=_animation_parameter_map(candidate_method.header),
                scalar_fields=scalar_fields,
                helper_methods=methods,
                current_class=class_name,
            )
        except Exception as exc:
            warnings.append(f"Animation method '{candidate_method.header}' could not be analyzed: {exc}")
            continue
        animation_candidates.append(
            (
                int(candidate_analysis.get("formula_count", 0)),
                candidate_analysis,
                candidate_logic,
                candidate_baked,
                candidate_warnings,
                candidate_method,
            )
        )

    if animation_candidates:
        _score, analysis, logic, baked, animation_warnings, animation_method = max(
            animation_candidates, key=lambda item: item[0]
        )
        warnings.extend(animation_warnings)
        if "func_78088_a" in animation_method.header or re.search(r"\brender\s*\(", animation_method.header):
            warnings.append(
                "Animation assignments were recovered from the legacy render method because setRotationAngles contained less usable logic."
            )
        other_methods = [
            {
                "method": item[5].header,
                "formula_count": item[0],
            }
            for item in animation_candidates
            if item[5] is not animation_method and item[0] > 0
        ]
        if other_methods:
            analysis["additional_animation_methods"] = other_methods
    else:
        analysis, logic, baked, animation_warnings, animation_method = {}, "", {}, [], None

    animation_resource, animation_resource_hint = _extract_external_animation_resource(text)
    resolved_animation_resource = _resolve_resource_path(
        animation_resource,
        animation_resource_hint,
        list(resource_roots or []),
    )
    if animation_resource:
        if resolved_animation_resource:
            warnings.append(
                f"External animation JSON detected: '{animation_resource}' -> '{resolved_animation_resource}'. Geometry was converted; external animation baking is not yet applied automatically."
            )
        else:
            warnings.append(
                f"External animation JSON referenced by model: '{animation_resource}'. The resource was not found in the configured search roots."
            )

    # De-duplicate warnings without hiding the original order.
    seen: set[str] = set()
    warnings = [w for w in warnings if not (w in seen or seen.add(w))]

    model = parsed_model_cls(
        class_name,
        texture["w"],
        texture["h"],
        parts,
        warnings,
    )
    if animation_resource:
        if not analysis:
            analysis = {
                "source_method": "external animation resource",
                "formula_count": 0,
                "bones": {},
                "classification": {},
                "formulas": [],
                "warnings": warnings[:],
                "external_animation_resource": animation_resource,
                "external_animation_resource_path": str(resolved_animation_resource) if resolved_animation_resource else "",
            }
        else:
            analysis["external_animation_resource"] = animation_resource
            analysis["external_animation_resource_path"] = str(resolved_animation_resource) if resolved_animation_resource else ""
    model.animation_analysis = analysis
    model.animation_logic = logic
    model.baked_animation_json = baked
    model.animation_source = animation_method.header if animation_method else ("external JSON" if animation_resource else "")
    model.constructor_args = list(constructor_args or [])
    model.scalar_fields = dict(scalar_fields)
    model.animation_resource = animation_resource
    model.animation_resource_path = str(resolved_animation_resource) if resolved_animation_resource else ""
    try:
        header = source.read_bytes()[:8]
        major = int.from_bytes(header[6:8], "big") if header[:4] == b"\xca\xfe\xba\xbe" else 0
    except OSError:
        major = 0
    if major and major <= 50:
        model.source_format = "minecraft_1.7_1.8_modelbase_class"
    elif major and major <= 52:
        model.source_format = "minecraft_1.9_1.12_modelbase_class"
    else:
        model.source_format = "legacy_modelbase_class"
    return model
