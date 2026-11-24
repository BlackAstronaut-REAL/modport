from __future__ import annotations

import math
import re
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

from legacy_bridge import _method_info, _parse_constant, _parse_methods, _run_javap


@dataclass
class _ModernObj:
    kind: str
    name: str = ""
    parent: Optional["_ModernObj"] = None
    cubes: list[Any] = field(default_factory=list)
    uv_x: int = 0
    uv_y: int = 0
    mirror: bool = False
    inflate: float = 0.0
    pose: tuple[float, float, float, float, float, float] = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
    delta: float = 0.0
    runtime_part: Any = None


def _simple(name: str) -> str:
    return name.replace("/", ".").split(".")[-1].split("$")[0]


def _class_from_comment(comment: str) -> str:
    match = re.search(r"\bclass\s+([A-Za-z0-9_/$]+)", comment)
    return match.group(1).replace("/", ".") if match else ""


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
            args.append(raw[start:end + 1])
            i = end + 1
        else:
            args.append(raw[start:i + 1])
            i += 1
    return args


def _parse_header_name(header: str) -> str:
    match = re.search(r"\b([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*;", header)
    if match:
        return match.group(1)
    match = re.search(r"\b([A-Za-z_$][\w$]*)\s*\([^)]*\)", header)
    return match.group(1) if match else ""


def _is_static_method(header: str) -> bool:
    return bool(re.search(r"\bstatic\b", header))


_VERBOSE_RECIPE_CACHE: dict[str, dict[int, str]] = {}


def _load_string_concat_recipes(source: Path) -> dict[int, str]:
    """Recover StringConcatFactory recipes from javap -v when modern model builders
    use invokedynamic to generate child-part names."""
    key = str(source.resolve()) if source.exists() else str(source)
    if key in _VERBOSE_RECIPE_CACHE:
        return _VERBOSE_RECIPE_CACHE[key]
    try:
        proc = subprocess.run(
            ["javap", "-v", "-p", str(source)],
            capture_output=True,
            text=True,
            errors="replace",
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        _VERBOSE_RECIPE_CACHE[key] = {}
        return {}
    text = proc.stdout or ""
    recipes: dict[int, str] = {}
    current: Optional[int] = None
    in_args = False
    for line in text.splitlines():
        m = re.match(r"\s*(\d+):.*StringConcatFactory\.makeConcatWithConstants", line)
        if m:
            current = int(m.group(1))
            in_args = False
            continue
        if current is not None and "Method arguments:" in line:
            in_args = True
            continue
        if current is not None and in_args:
            m = re.match(r"\s*#\d+\s+(.+)$", line)
            if m:
                recipe = m.group(1).replace("\\u0001", "{0}")
                recipes[current] = recipe
                current = None
                in_args = False
                continue
            if line.strip() and not line.startswith(" " * 4):
                current = None
                in_args = False
    _VERBOSE_RECIPE_CACHE[key] = recipes
    return recipes


def _is_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _as_float(value: Any, default: float = 0.0) -> float:
    return float(value) if _is_number(value) else default


def _constant(op: str, comment: str) -> Any:
    value = _parse_constant(op, comment)
    if value is not None:
        return value
    if comment.startswith("String "):
        return comment[7:]
    if "// class " in comment:
        return ("class", _class_from_comment(comment))
    return None


def _method_return_kind(desc: str) -> str:
    if ")" not in desc:
        return ""
    ret = desc.split(")", 1)[1]
    if ret.startswith("L") and ret.endswith(";"):
        return _simple(ret[1:-1])
    return ret


def _numeric_bin(op: str, a: Any, b: Any) -> Any:
    if not (_is_number(a) and _is_number(b)):
        return None
    if op in {"fadd", "dadd", "iadd", "ladd"}:
        return a + b
    if op in {"fsub", "dsub", "isub", "lsub"}:
        return a - b
    if op in {"fmul", "dmul", "imul", "lmul"}:
        return a * b
    if op in {"fdiv", "ddiv", "idiv", "ldiv"}:
        try:
            return a / b
        except ZeroDivisionError:
            return None
    return None


class _ModernBytecodeInterpreter:
    def __init__(self, class_name: str, methods: list[Any], cube_cls: Any, part_cls: Any) -> None:
        self.class_name = class_name
        self.methods = methods
        self.method_map: dict[str, list[Any]] = {}
        for method in methods:
            self.method_map.setdefault(_parse_header_name(method.header), []).append(method)
        self.cube_cls = cube_cls
        self.part_cls = part_cls
        self.parts: list[Any] = []
        self.root: Optional[_ModernObj] = None
        self.texture_width = 64
        self.texture_height = 32
        self.index = 0
        self.call_stack: list[str] = []
        self.warnings: list[str] = []
        self.concat_recipes: dict[int, str] = {}

    def _new_part(self, parent: _ModernObj, name: str, builder: Optional[_ModernObj], pose: Optional[_ModernObj]) -> _ModernObj:
        pose_values = pose.pose if pose else (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        part = self.part_cls(
            name=name,
            variable=name,
            parent_variable=parent.name if parent.name else "root",
            offset=(pose_values[0], pose_values[1], pose_values[2]),
            rotation=(pose_values[3], pose_values[4], pose_values[5]),
            cubes=[],
            source_index=self.index,
        )
        self.index += 1
        if builder is not None and builder.kind == "builder":
            part.cubes.extend(builder.cubes)
        # `_ModernObj` is the interpreter-side handle; the parsed model uses the real
        # `Part` instance. Keep the runtime parent relationship there so helper methods
        # can pass PartDefinition handles around without requiring the proxy itself to
        # expose a `children` collection.
        if parent.kind == "part" and parent.runtime_part is not None:
            part.parent = parent.runtime_part
        self.parts.append(part)
        return _ModernObj("part", name=name, parent=parent, runtime_part=part)

    def _invoke_helper(self, method_name: str, args: list[Any]) -> Any:
        candidates = self.method_map.get(method_name, [])
        if not candidates or method_name in self.call_stack:
            return None
        # Prefer methods with matching parameter count.
        candidate = None
        for method in candidates:
            if "(" in method.header:
                # Parameter count is approximate from header; bytecode descriptor is more reliable
                # and is handled by the caller when available. For same-name overloads, first is fine.
                candidate = method
                break
        if candidate is None:
            return None
        self.call_stack.append(method_name)
        try:
            return self._execute_method(candidate, args)
        finally:
            self.call_stack.pop()

    def _execute_method(self, method: Any, incoming_args: list[Any]) -> Any:
        stack: list[Any] = []
        local_base = 0 if _is_static_method(method.header) else 1
        locals_map: dict[int, Any] = {i + local_base: value for i, value in enumerate(incoming_args)}

        def pop(default: Any = None) -> Any:
            return stack.pop() if stack else default

        def push(value: Any) -> None:
            stack.append(value)

        for offset, op, comment in method.instructions:
            base = op.split()[0]
            if base == "nop":
                continue
            if base == "new":
                cls = _simple(_class_from_comment(comment))
                if cls == "MeshDefinition":
                    push(_ModernObj("mesh"))
                elif cls == "CubeListBuilder":
                    push(_ModernObj("builder"))
                elif cls == "CubeDeformation":
                    push(_ModernObj("deformation"))
                elif cls == "PartPose":
                    push(_ModernObj("pose"))
                else:
                    push(_ModernObj("other", name=cls))
                continue
            if base == "dup":
                if stack:
                    push(stack[-1])
                continue
            if base in {"swap"} and len(stack) >= 2:
                stack[-1], stack[-2] = stack[-2], stack[-1]
                continue
            if base in {"pop", "pop2"}:
                pop()
                continue
            if base.startswith(("aload_", "iload_", "fload_", "dload_", "lload_")):
                try:
                    idx = int(base.split("_")[-1])
                except ValueError:
                    idx = 0
                push(locals_map.get(idx))
                continue
            if base in {"aload", "iload", "fload", "dload", "lload"}:
                try:
                    idx = int(op.split()[-1])
                except ValueError:
                    idx = 0
                push(locals_map.get(idx))
                continue
            if base.startswith(("astore_", "istore_", "fstore_", "dstore_", "lstore_")):
                try:
                    idx = int(base.split("_")[-1])
                except ValueError:
                    idx = 0
                locals_map[idx] = pop()
                continue
            if base in {"astore", "istore", "fstore", "dstore", "lstore"}:
                try:
                    idx = int(op.split()[-1])
                except ValueError:
                    idx = 0
                locals_map[idx] = pop()
                continue
            if base.startswith(("iconst_", "fconst_", "dconst_", "lconst_")) or base in {"bipush", "sipush", "ldc", "ldc_w", "ldc2_w"}:
                value = _constant(op, comment)
                if value is not None:
                    push(value)
                continue
            if base == "aconst_null":
                push(None)
                continue
            if base in {"fneg", "dneg", "ineg", "lneg"}:
                value = pop()
                push(-value if _is_number(value) else None)
                continue
            if base in {"fadd", "fsub", "fmul", "fdiv", "dadd", "dsub", "dmul", "ddiv", "iadd", "isub", "imul", "idiv", "ladd", "lsub", "lmul", "ldiv"}:
                b = pop(); a = pop(); push(_numeric_bin(base, a, b)); continue
            if base in {"i2f", "i2d", "f2d", "d2f", "i2l", "l2f", "l2d", "f2i", "d2i", "l2i"}:
                value = pop(); push(float(value) if _is_number(value) else value); continue
            if base.startswith("if") or base in {"goto", "goto_w", "tableswitch", "lookupswitch"}:
                # Full control-flow is intentionally not emulated. Geometry builders
                # generated by Blockbench use straight-line builder code; if a helper
                # contains a branch we preserve the method but report it.
                continue
            if base == "return":
                return None
            if base in {"areturn", "ireturn", "freturn", "dreturn", "lreturn"}:
                return pop()
            if base == "invokedynamic":
                indy_match = re.search(r"InvokeDynamic\s+#(\d+)", comment, re.IGNORECASE)
                bootstrap_index = int(indy_match.group(1)) if indy_match else 0
                desc_match = re.search(r"makeConcatWithConstants:(\([^)]*\).*)$", comment)
                desc = desc_match.group(1) if desc_match else "()Ljava/lang/String;"
                arg_count = len(_descriptor_args(desc))
                args = [pop() for _ in range(arg_count)][::-1]
                recipe = self.concat_recipes.get(bootstrap_index)
                if recipe is not None:
                    value = recipe
                    for idx, arg in enumerate(args):
                        value = value.replace("{" + str(idx) + "}", str(arg))
                    push(value)
                else:
                    value = str(args[0]) if args else str(bootstrap_index)
                    push(f"part_{value}")
                continue
            if base == "getstatic":
                field_match = re.search(r"Field\s+.+?([A-Za-z0-9_$]+):L([^;]+);", comment)
                if field_match:
                    field_name = field_match.group(1)
                    owner_name = _simple(field_match.group(2))
                    if owner_name == "PartPose" and field_name in {"ZERO", "f_171404_"}:
                        push(_ModernObj("pose"))
                        continue
                    if owner_name == "CubeDeformation" and field_name in {"ZERO", "f_171458_"}:
                        push(_ModernObj("deformation", delta=0.0))
                        continue
                push(None)
                continue
            if not base.startswith("invoke"):
                continue

            info = _method_info(comment)
            if not info:
                continue
            owner, name, desc = info
            arg_count = len(_descriptor_args(desc))
            args = [pop() for _ in range(arg_count)][::-1]
            receiver = None if base in {"invokestatic", "invokedynamic"} else pop()
            owner_simple = _simple(owner)
            return_kind = _method_return_kind(desc)

            # Constructors: initialize the object left behind by new/dup.
            if name == "<init>":
                obj = receiver if isinstance(receiver, _ModernObj) else None
                if obj is not None:
                    cls = _simple(owner)
                    if cls == "CubeDeformation":
                        obj.kind = "deformation"
                        obj.delta = _as_float(args[-1] if args else 0.0)
                    elif cls == "PartPose":
                        obj.kind = "pose"
                    elif cls == "MeshDefinition":
                        obj.kind = "mesh"
                    elif cls == "CubeListBuilder":
                        obj.kind = "builder"
                # The duplicate reference remains on the stack in normal new/dup/init bytecode.
                if stack and stack[-1] is receiver:
                    continue
                push(receiver)
                continue

            # Static factories/constants.
            if owner_simple == "CubeListBuilder" and name in {"create", "m_171558_"}:
                push(_ModernObj("builder")); continue
            if owner_simple == "PartPose" and name in {"offset", "m_171419_"}:
                vals = [_as_float(v) for v in args[-3:]]
                push(_ModernObj("pose", pose=(vals[0], vals[1], vals[2], 0.0, 0.0, 0.0))); continue
            if owner_simple == "PartPose" and name in {"offsetAndRotation", "m_171423_"}:
                vals = [_as_float(v) for v in args[-6:]]
                push(_ModernObj("pose", pose=tuple(vals))); continue
            if owner_simple == "PartPose" and name in {"rotation", "m_171422_", "m_171420_"}:
                vals = [_as_float(v) for v in args[-3:]]
                push(_ModernObj("pose", pose=(0.0, 0.0, 0.0, vals[0], vals[1], vals[2]))); continue
            if owner_simple == "MeshDefinition" and name in {"getRoot", "m_171576_"}:
                if self.root is None:
                    self.root = _ModernObj("part", name="root")
                push(self.root); continue
            if owner_simple == "HumanoidModel" and return_kind in {"MeshDefinition", "MeshDefinition;"}:
                if self.root is None:
                    self.root = _ModernObj("part", name="root")
                push(_ModernObj("mesh")); continue
            if owner_simple == "CubeDeformation" and name in {"extend", "extendUniform"}:
                base = receiver if isinstance(receiver, _ModernObj) else _ModernObj("deformation")
                amount = _as_float(args[-1] if args else 0.0)
                push(_ModernObj("deformation", delta=base.delta + amount)); continue
            if owner_simple == "CubeListBuilder" and name in {"texOffs", "m_171514_"}:
                builder = receiver if isinstance(receiver, _ModernObj) else _ModernObj("builder")
                if args and len(args) >= 2:
                    builder.uv_x = int(_as_float(args[-2])); builder.uv_y = int(_as_float(args[-1]))
                push(builder); continue
            if owner_simple == "CubeListBuilder" and name in {"mirror", "m_171480_"}:
                builder = receiver if isinstance(receiver, _ModernObj) else _ModernObj("builder")
                builder.mirror = True
                push(builder); continue
            if owner_simple == "CubeListBuilder" and name in {"addBox", "m_171481_", "m_171482_", "m_171483_", "m_171484_", "m_171488_", "m_171489_"}:
                builder = receiver if isinstance(receiver, _ModernObj) else _ModernObj("builder")
                cube_args = list(args)
                if cube_args and isinstance(cube_args[0], str):
                    cube_args = cube_args[1:]
                inflate = 0.0
                if cube_args and isinstance(cube_args[-1], _ModernObj) and cube_args[-1].kind == "deformation":
                    inflate = cube_args[-1].delta
                    cube_args = cube_args[:-1]
                if len(cube_args) >= 6 and all(_is_number(v) for v in cube_args[:6]):
                    vals = [_as_float(v) for v in cube_args[:6]]
                    cube = self.cube_cls(
                        x=vals[0], y=vals[1], z=vals[2], dx=vals[3], dy=vals[4], dz=vals[5],
                        uv_x=builder.uv_x, uv_y=builder.uv_y, mirror=builder.mirror,
                        inflate=inflate, name=f"cube_{len(builder.cubes)}",
                    )
                    builder.cubes.append(cube)
                else:
                    self.warnings.append(f"Unsupported CubeListBuilder.addBox signature in {self.class_name} at bytecode offset {offset}.")
                push(builder); continue
            if owner_simple == "PartDefinition" and name in {"addOrReplaceChild", "addChild", "m_171599_", "m_171591_"}:
                parent = receiver if isinstance(receiver, _ModernObj) else self.root
                builder = args[1] if len(args) >= 2 and isinstance(args[1], _ModernObj) else None
                pose = args[2] if len(args) >= 3 and isinstance(args[2], _ModernObj) else None
                name_value = str(args[0]) if args else f"part_{self.index}"
                if parent is None:
                    parent = self.root or _ModernObj("part", name="root")
                    self.root = parent
                child = self._new_part(parent, name_value, builder, pose)
                push(child); continue
            if owner_simple == "LayerDefinition" and name in {"create", "m_171565_"}:
                if len(args) >= 3 and _is_number(args[-2]) and _is_number(args[-1]):
                    self.texture_width = int(_as_float(args[-2], 64))
                    self.texture_height = int(_as_float(args[-1], 32))
                push(_ModernObj("layer")); continue

            # User-defined helper method in the same class.
            if (not owner or _simple(owner) == self.class_name) and name in self.method_map:
                result = self._invoke_helper(name, args)
                if result is not None:
                    push(result)
                    continue

            # Preserve an unknown return value if the signature returns something.
            if return_kind not in {"V", ""}:
                push(None)

        return stack[-1] if stack else None

    def _default_method_args(self, method: Any) -> list[Any]:
        """Seed common model-builder parameters for static helper methods.

        Modern armor/model factories frequently expose ``setup(CubeDeformation)``
        instead of a zero-argument ``createBodyLayer`` method.  Javap gives us the
        readable parameter types in the method header, so a neutral deformation and
        scalar defaults are safer than executing the method with missing locals.
        """
        match = re.search(r"\(([^)]*)\)", method.header)
        if not match or not match.group(1).strip():
            return []
        args: list[Any] = []
        for raw in match.group(1).split(","):
            name = _simple(raw.strip().split()[-1])
            if name == "CubeDeformation":
                args.append(_ModernObj("deformation", delta=0.0))
            elif name == "MeshDefinition":
                args.append(_ModernObj("mesh"))
            elif name == "CubeListBuilder":
                args.append(_ModernObj("builder"))
            elif name == "PartPose":
                args.append(_ModernObj("pose"))
            elif name == "PartDefinition":
                root = self.root or _ModernObj("part", name="root")
                self.root = root
                args.append(root)
            elif name in {"float", "double"}:
                args.append(0.0)
            elif name in {"byte", "short", "int", "long"}:
                args.append(0)
            elif name == "boolean":
                args.append(False)
            else:
                args.append(None)
        return args

    def execute(self) -> tuple[list[Any], int, int, list[str]]:
        candidates: list[Any] = []
        priorities = ("createBodyLayer", "create", "createMesh", "createLayerDefinition", "getTexturedModelData", "getModelData", "createLayer")
        for name in priorities:
            candidates.extend(self.method_map.get(name, []))
        if not candidates:
            for method in self.methods:
                instruction_text = "\n".join(comment for _offset, _op, comment in method.instructions)
                if (
                    "CubeListBuilder" in method.header
                    or "LayerDefinition" in method.header
                    or "MeshDefinition" in method.header
                    or "CubeListBuilder" in instruction_text
                    or "PartDefinition" in instruction_text
                    or "LayerDefinition" in instruction_text
                ):
                    candidates.append(method)
        for method in candidates:
            self.parts.clear(); self.index = 0; self.root = None; self.warnings.clear()
            try:
                self._execute_method(method, self._default_method_args(method))
            except Exception as exc:
                self.warnings.append(f"Modern bytecode interpreter failed in {_parse_header_name(method.header)}: {exc}")
            if self.parts and any(p.cubes for p in self.parts):
                break
        if not self.parts:
            raise RuntimeError("No PartDefinition/CubeListBuilder geometry could be reconstructed from modern compiled bytecode.")
        return self.parts, self.texture_width, self.texture_height, self.warnings


def parse_modern_bytecode_class(
    source: Path,
    cube_cls: Any,
    part_cls: Any,
    parsed_model_cls: Any,
    constructor_args: Optional[list[Any]] = None,
) -> Any:
    text = _run_javap(source)
    class_match = re.search(r"(?m)^(?:public\s+)?(?:final\s+)?class\s+([A-Za-z0-9_.$]+)", text)
    class_name = _simple(class_match.group(1)) if class_match else source.stem
    methods = _parse_methods(text)
    interpreter = _ModernBytecodeInterpreter(class_name, methods, cube_cls, part_cls)
    if any(ins[1].split()[0] == "invokedynamic" for method in methods for ins in method.instructions):
        interpreter.concat_recipes = _load_string_concat_recipes(source)
    parts, texture_width, texture_height, warnings = interpreter.execute()
    model = parsed_model_cls(class_name, texture_width, texture_height, parts, warnings)
    model.source_format = "modern_mojmap_bytecode"
    model.constructor_args = list(constructor_args or [])
    model.animation_analysis = {
        "source_format": "modern_mojmap_bytecode",
        "mode": "preserve_java",
        "note": "Compiled modern model geometry reconstructed from CubeListBuilder/PartDefinition/PartPose bytecode. Runtime animation methods are preserved as Java analysis where available.",
    }
    return model
