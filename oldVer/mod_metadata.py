from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class ModMetadata:
    modid: str = ""
    name: str = ""
    version: str = ""
    authors: list[str] = field(default_factory=list)
    description: str = ""
    website: str = ""
    license: str = ""
    source: str = ""

    @property
    def author_text(self) -> str:
        return ", ".join(value for value in self.authors if value)


def _first(value: Any) -> str:
    if isinstance(value, str):
        return value.strip()
    if isinstance(value, (int, float)):
        return str(value)
    return ""


def _authors(value: Any) -> list[str]:
    if isinstance(value, str):
        return [part.strip() for part in re.split(r"[,;]", value) if part.strip()]
    if isinstance(value, list):
        result: list[str] = []
        for entry in value:
            if isinstance(entry, str) and entry.strip():
                result.append(entry.strip())
            elif isinstance(entry, dict):
                name = _first(entry.get("name"))
                if name:
                    result.append(name)
        return result
    return []


def _candidate_files(root: Path, names: set[str]) -> list[Path]:
    result: list[Path] = []
    seen: set[str] = set()
    for path in root.rglob("*"):
        if not path.is_file() or path.name.lower() not in names:
            continue
        key = str(path.resolve()).lower()
        if key not in seen:
            seen.add(key)
            result.append(path)
    return sorted(result, key=lambda p: (len(p.parts), str(p).lower()))


def _from_mcmod(path: Path) -> ModMetadata | None:
    try:
        payload = json.loads(path.read_text(encoding="utf-8-sig", errors="replace"))
    except Exception:
        return None
    if isinstance(payload, list):
        payload = next((entry for entry in payload if isinstance(entry, dict)), None)
    if not isinstance(payload, dict):
        return None
    return ModMetadata(
        modid=_first(payload.get("modid") or payload.get("id")),
        name=_first(payload.get("name")),
        version=_first(payload.get("version")),
        authors=_authors(payload.get("authors") or payload.get("authorList") or payload.get("author")),
        description=_first(payload.get("description")),
        website=_first(payload.get("url") or payload.get("website")),
        license=_first(payload.get("license")),
        source=str(path),
    )


def _from_fabric(path: Path) -> ModMetadata | None:
    try:
        payload = json.loads(path.read_text(encoding="utf-8-sig", errors="replace"))
    except Exception:
        return None
    if not isinstance(payload, dict):
        return None
    contact = payload.get("contact") if isinstance(payload.get("contact"), dict) else {}
    return ModMetadata(
        modid=_first(payload.get("id")),
        name=_first(payload.get("name")),
        version=_first(payload.get("version")),
        authors=_authors(payload.get("authors") or payload.get("contributors")),
        description=_first(payload.get("description")),
        website=_first(contact.get("homepage") if isinstance(contact, dict) else ""),
        license=_first(payload.get("license")),
        source=str(path),
    )


def _toml_string(text: str, key: str) -> str:
    match = re.search(rf"(?m)^\s*{re.escape(key)}\s*=\s*[\"']([^\"']*)[\"']", text)
    return match.group(1).strip() if match else ""


def _from_mods_toml(path: Path) -> ModMetadata | None:
    try:
        text = path.read_text(encoding="utf-8-sig", errors="replace")
    except Exception:
        return None
    modid = _toml_string(text, "modId")
    if not modid:
        return None
    authors = _authors(_toml_string(text, "authors"))
    return ModMetadata(
        modid=modid,
        name=_toml_string(text, "displayName"),
        version=_toml_string(text, "version"),
        authors=authors,
        description=_toml_string(text, "description"),
        website=_toml_string(text, "displayURL"),
        license=_toml_string(text, "license"),
        source=str(path),
    )


def _from_java(root: Path) -> ModMetadata | None:
    best: ModMetadata | None = None
    mod_pattern = re.compile(r"@Mod\s*\((.*?)\)", re.DOTALL)
    for path in root.rglob("*.java"):
        try:
            text = path.read_text(encoding="utf-8-sig", errors="replace")
        except OSError:
            continue
        for block in mod_pattern.findall(text):
            def value(key: str) -> str:
                match = re.search(rf"\b{re.escape(key)}\s*=\s*[\"']([^\"']+)[\"']", block)
                return match.group(1).strip() if match else ""
            modid = value("modid") or value("value")
            if not modid:
                continue
            candidate = ModMetadata(
                modid=modid,
                name=value("name"),
                version=value("version"),
                source=str(path),
            )
            if best is None or (candidate.name and not best.name):
                best = candidate
    return best


def _merge(primary: ModMetadata, fallback: ModMetadata | None) -> ModMetadata:
    if fallback is None:
        return primary
    if not primary.modid:
        primary.modid = fallback.modid
    if not primary.name:
        primary.name = fallback.name
    if not primary.version:
        primary.version = fallback.version
    if not primary.authors:
        primary.authors = list(fallback.authors)
    if not primary.description:
        primary.description = fallback.description
    if not primary.website:
        primary.website = fallback.website
    if not primary.license:
        primary.license = fallback.license
    if not primary.source:
        primary.source = fallback.source
    return primary


def infer_mod_metadata(root: Path) -> ModMetadata:
    """Infer stable mod metadata from packaged resources before using folder names.

    Metadata files are authoritative, Java ``@Mod`` annotations are a fallback, and
    an assets namespace is used only when neither supplies a mod ID.
    """
    root = Path(root).expanduser().resolve()
    result = ModMetadata()
    for path in _candidate_files(root, {"mcmod.info"}):
        result = _merge(result, _from_mcmod(path))
        if result.modid and result.name:
            break
    if not result.modid:
        for path in _candidate_files(root, {"fabric.mod.json"}):
            result = _merge(result, _from_fabric(path))
            if result.modid:
                break
    if not result.modid:
        for path in _candidate_files(root, {"mods.toml", "neoforge.mods.toml"}):
            result = _merge(result, _from_mods_toml(path))
            if result.modid:
                break
    result = _merge(result, _from_java(root))
    if not result.modid:
        namespaces: list[str] = []
        for assets in root.rglob("assets"):
            if not assets.is_dir():
                continue
            for child in assets.iterdir():
                if child.is_dir() and re.fullmatch(r"[a-z0-9_.-]+", child.name):
                    namespaces.append(child.name)
        if namespaces:
            result.modid = sorted(namespaces, key=lambda value: (value in {"minecraft", "forge"}, len(value), value))[0]
    return result
