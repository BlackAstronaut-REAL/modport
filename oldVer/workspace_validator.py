from __future__ import annotations

import json
import re
from collections import Counter, defaultdict
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Any, Iterable


@dataclass
class ValidationIssue:
    severity: str
    code: str
    message: str
    element: str = ""
    registry_name: str = ""
    path: str = ""


@dataclass
class WorkspaceValidationReport:
    workspace: str
    errors: list[ValidationIssue] = field(default_factory=list)
    warnings: list[ValidationIssue] = field(default_factory=list)
    info: list[ValidationIssue] = field(default_factory=list)
    stats: dict[str, Any] = field(default_factory=dict)

    @property
    def ok(self) -> bool:
        return not self.errors

    def add(self, severity: str, code: str, message: str, **context: str) -> None:
        issue = ValidationIssue(severity, code, message, **context)
        getattr(self, severity + "s").append(issue)

    def to_dict(self) -> dict[str, Any]:
        return {
            "workspace": self.workspace,
            "ok": self.ok,
            "stats": self.stats,
            "errors": [asdict(issue) for issue in self.errors],
            "warnings": [asdict(issue) for issue in self.warnings],
            "info": [asdict(issue) for issue in self.info],
        }


def _read_json(path: Path) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8-sig"))
    except Exception:
        return None
    return value if isinstance(value, dict) else None


def _workspace_file(root: Path) -> Path | None:
    candidates: list[tuple[int, int, Path]] = []
    for path in root.glob("*.mcreator"):
        data = _read_json(path)
        if data and isinstance(data.get("workspaceSettings"), dict):
            candidates.append((int(data.get("mcreatorVersion", 0) or 0), len(data.get("mod_elements", [])), path))
    return max(candidates, default=(0, 0, None), key=lambda row: (row[0], row[1]))[2]


def _resource_ref_candidates(root: Path, modid: str, definition: dict[str, Any]) -> Iterable[tuple[str, Path]]:
    textures = root / "src" / "main" / "resources" / "assets" / modid / "textures"
    keys = {
        "texture": (textures / "item", textures / "block"),
        "textureHelmet": (textures / "item",),
        "textureBody": (textures / "item",),
        "textureLeggings": (textures / "item",),
        "textureBoots": (textures / "item",),
        "customModelTexture": (textures / "projectile", textures / "entity", textures / "entities", textures / "item"),
        "mobModelTexture": (textures / "entity", textures / "entities"),
    }
    for key, directories in keys.items():
        value = definition.get(key)
        if not isinstance(value, str) or not value.strip() or value in {"Normal", "Default"}:
            continue
        filename = value if value.lower().endswith(".png") else value + ".png"
        yield key, directories[0] / filename
        # Extra alternatives are represented by a sentinel path separated with |.
        for directory in directories[1:]:
            yield key + "|alternative", directory / filename


def validate_workspace(
    workspace_root: Path,
    workspace: dict[str, Any] | None = None,
    write_reports: bool = True,
) -> WorkspaceValidationReport:
    root = Path(workspace_root).expanduser().resolve()
    workspace_path = _workspace_file(root)
    if workspace is None:
        if workspace_path is None:
            report = WorkspaceValidationReport(str(root))
            report.add("error", "workspace_missing", "No readable .mcreator workspace definition was found.", path=str(root))
            return report
        workspace = _read_json(workspace_path)
    if not isinstance(workspace, dict):
        report = WorkspaceValidationReport(str(workspace_path or root))
        report.add("error", "workspace_invalid", "Workspace JSON is not an object.")
        return report

    report = WorkspaceValidationReport(str(workspace_path or root))
    settings = workspace.get("workspaceSettings", {}) if isinstance(workspace.get("workspaceSettings"), dict) else {}
    modid = str(settings.get("modid") or "converted_mod")
    descriptors = [item for item in workspace.get("mod_elements", []) if isinstance(item, dict)]
    report.stats["descriptor_count"] = len(descriptors)
    report.stats["element_type_counts"] = dict(Counter(str(item.get("type", "unknown")) for item in descriptors))

    descriptor_keys: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    name_map: dict[str, list[dict[str, Any]]] = defaultdict(list)
    element_by_name = {str(item.get("name", "")): item for item in descriptors if str(item.get("name", ""))}
    custom_biomes = {
        str(item.get("name", ""))
        for item in descriptors
        if item.get("type") == "biome" and str(item.get("name", ""))
    }
    for item in descriptors:
        name = str(item.get("name", ""))
        registry = str(item.get("registry_name", ""))
        kind = str(item.get("type", ""))
        descriptor_keys[(kind.lower(), registry.lower())].append(item)
        name_map[name.lower()].append(item)
        if not name:
            report.add("error", "element_name_empty", "Workspace element descriptor has no name.", registry_name=registry)
        if registry and registry != registry.lower():
            report.add("error", "registry_uppercase", "Modern registry paths must be lowercase.", element=name, registry_name=registry)
        if registry and not re.fullmatch(r"[a-z0-9_.-]+", registry):
            report.add("error", "registry_invalid", "Registry path contains invalid characters.", element=name, registry_name=registry)

    for (kind, registry), values in descriptor_keys.items():
        if registry and len(values) > 1:
            report.add(
                "error", "duplicate_same_type_registry",
                f"{len(values)} {kind} descriptors use registry path '{registry}'.",
                registry_name=registry,
            )
    for name, values in name_map.items():
        if name and len(values) > 1:
            report.add("error", "duplicate_element_name", f"{len(values)} descriptors use element name '{name}'.", element=name)

    loaded_elements: dict[str, dict[str, Any]] = {}
    explicit_item_paths: set[str] = set()
    automatic_block_item_paths: set[str] = set()
    missing_element_files = 0
    empty_creative_tabs = 0
    animated_textures = 0
    for descriptor in descriptors:
        name = str(descriptor.get("name", ""))
        kind = str(descriptor.get("type", ""))
        registry = str(descriptor.get("registry_name", ""))
        path = root / "elements" / f"{name}.mod.json"
        data = _read_json(path)
        if data is None:
            missing_element_files += 1
            report.add("error", "element_file_missing", "Descriptor has no readable .mod.json file.", element=name, registry_name=registry, path=str(path))
            continue
        loaded_elements[name] = data
        file_kind = str(data.get("_type", ""))
        if file_kind and file_kind != kind:
            report.add("error", "element_type_mismatch", f"Descriptor type '{kind}' differs from file type '{file_kind}'.", element=name, registry_name=registry, path=str(path))
        definition = data.get("definition", {}) if isinstance(data.get("definition"), dict) else {}
        if not definition:
            report.add("error", "definition_missing", "Element JSON has no non-empty definition object; MCreator will report a missing mod definition.", element=name, registry_name=registry, path=str(path))
        if str(data.get("_type", "")).strip() and str(data.get("_type", "")).lower() != kind.lower():
            report.add("error", "element_type_mismatch", f"Element file _type '{data.get('_type')}' does not match descriptor type '{kind}'.", element=name, registry_name=registry, path=str(path))
        try:
            fv = int(data.get("_fv", 0))
            if fv > 89:
                report.add("error", "element_format_too_new", f"Element format FV{fv} is newer than the supported MCreator format FV89.", element=name, registry_name=registry, path=str(path))
        except Exception:
            report.add("error", "element_format_invalid", "Element _fv is not an integer.", element=name, registry_name=registry, path=str(path))
        # Native item/tool/block models must not carry an empty custom model sentinel.
        if kind in {"item", "tool"} and int(definition.get("renderType", 0) or 0) == 0 and not str(definition.get("customModelName", "")):
            report.add("error", "native_item_model_missing", "Native item/tool renderType=0 requires customModelName='Normal'.", element=name, registry_name=registry, path=str(path))
        if kind in {"block", "plant"} and not str(definition.get("customModelName", "")):
            report.add("warning", "block_model_missing", "Block/plant customModelName is empty; native render modes may fail to resolve a model.", element=name, registry_name=registry, path=str(path))
        tabs = definition.get("creativeTabs")
        if kind in {"item", "tool", "armor", "block", "plant", "livingentity"} and isinstance(tabs, list) and not tabs:
            empty_creative_tabs += 1
            report.add("warning", "creative_tabs_empty", "Content is not assigned to a creative tab.", element=name, registry_name=registry)
        if kind in {"item", "tool"} and registry:
            explicit_item_paths.add(registry.lower())
        if kind in {"block", "plant"} and registry and definition.get("hasBlockItem", True):
            automatic_block_item_paths.add(registry.lower())
        if kind == "dimension":
            biomes = definition.get("biomesInDimension")
            if not isinstance(biomes, list) or not biomes:
                report.add("error", "dimension_biomes_empty", "Dimension has no surface biome; generated multi-noise biome sources cannot be empty.", element=name, registry_name=registry)
            else:
                for biome in biomes:
                    if isinstance(biome, str) and biome.startswith("CUSTOM:") and biome.split(":", 1)[1] not in custom_biomes:
                        report.add("error", "dimension_custom_biome_missing", f"Dimension references missing custom biome '{biome}'.", element=name, registry_name=registry)
        if kind == "projectile" and not str(definition.get("projectileItem", "")).strip():
            report.add("warning", "projectile_item_unbound", "Projectile has no bound item; it cannot be selected by normal ranged-item generation.", element=name, registry_name=registry)

        # Metadata paths are workspace-relative and should all exist after export.
        metadata = descriptor.get("metadata", {}) if isinstance(descriptor.get("metadata"), dict) else {}
        for rel in metadata.get("files", []) if isinstance(metadata.get("files"), list) else []:
            target = root / str(rel)
            if not target.exists():
                report.add("error", "metadata_file_missing", "Workspace metadata references a missing file.", element=name, registry_name=registry, path=str(rel))

        # Texture checks are intentionally conservative: only report missing if none
        # of the known directories contains the filename.
        grouped: dict[str, list[Path]] = defaultdict(list)
        for key, candidate in _resource_ref_candidates(root, modid, definition):
            grouped[key.split("|", 1)[0]].append(candidate)
        for key, candidates in grouped.items():
            if not any(candidate.exists() for candidate in candidates):
                report.add("warning", "texture_reference_missing", f"Texture field '{key}' does not resolve to a copied PNG.", element=name, registry_name=registry, path=" | ".join(str(p) for p in candidates))
            else:
                for candidate in candidates:
                    if candidate.exists() and candidate.with_name(candidate.name + ".mcmeta").exists():
                        animated_textures += 1
                        break

    for registry in sorted(explicit_item_paths & automatic_block_item_paths):
        report.add(
            "error", "block_item_registry_collision",
            "An explicit item and an automatic BlockItem would register the same item path. Disable hasBlockItem on the block/plant.",
            registry_name=registry,
        )

    report.stats.update({
        "loaded_element_files": len(loaded_elements),
        "missing_element_files": missing_element_files,
        "empty_creative_tabs": empty_creative_tabs,
        "animated_texture_references": animated_textures,
        "error_count": len(report.errors),
        "warning_count": len(report.warnings),
    })

    if write_reports:
        json_path = root / "MOJMODELBRIDGE_WORKSPACE_VALIDATION.json"
        txt_path = root / "MOJMODELBRIDGE_WORKSPACE_VALIDATION.txt"
        json_path.write_text(json.dumps(report.to_dict(), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        lines = [
            f"Workspace: {report.workspace}",
            f"Result: {'PASS' if report.ok else 'FAIL'}",
            f"Errors: {len(report.errors)}",
            f"Warnings: {len(report.warnings)}",
            "",
        ]
        for title, issues in (("ERRORS", report.errors), ("WARNINGS", report.warnings), ("INFO", report.info)):
            lines.append(title)
            lines.append("-" * len(title))
            if not issues:
                lines.append("None")
            for issue in issues:
                context = ", ".join(value for value in (issue.element, issue.registry_name, issue.path) if value)
                lines.append(f"[{issue.code}] {issue.message}" + (f" ({context})" if context else ""))
            lines.append("")
        txt_path.write_text("\n".join(lines), encoding="utf-8")
    return report
