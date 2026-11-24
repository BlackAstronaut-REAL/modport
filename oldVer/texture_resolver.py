from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Optional


def _tlog(message: str, level: str = "INFO") -> None:
    print(f"[TEXTURE:{level}] {message}", flush=True)


_REF_RE = re.compile(r"(?i)(?:[a-z0-9_.-]+:)?(?:assets/)?(?:textures/)?[a-z0-9_./\\-]+\.png")
_TEXTURE_PATH_RE = re.compile(r"(?i)(?:[a-z0-9_.-]+:)?textures/[a-z0-9_./\\-]+\.png")
_PNG_NAME_RE = re.compile(r"(?i)[a-z0-9_./\\-]+\.png")
_PRINTABLE_PNG_RE = re.compile(rb"[ -~]{1,300}\.png")
_MODEL_TOKEN_RE = re.compile(r"\bModel[A-Za-z0-9_$]+\b")
_AUXILIARY_WORDS = {
    "eye", "eyes", "glow", "glowing", "emissive", "overlay", "layer",
    "armor", "armour", "saddle", "mask", "beam", "laser", "outline",
}


@dataclass(frozen=True)
class TextureReference:
    value: str
    source: Path


@dataclass(frozen=True)
class SourceRecord:
    path: Path
    texture_refs: tuple[str, ...] = ()
    model_mentions: tuple[str, ...] = ()


@dataclass(frozen=True)
class TextureResolution:
    path: Optional[Path]
    score: float = 0.0
    evidence: tuple[str, ...] = ()


@dataclass(frozen=True)
class TextureSetResolution:
    primary: Optional[Path]
    variants: tuple[Path, ...] = ()
    auxiliaries: tuple[Path, ...] = ()
    evidence: tuple[str, ...] = ()
    candidate_details: tuple[tuple[str, float, str], ...] = ()


@dataclass
class ModLayout:
    root: Path
    model_roots: list[Path] = field(default_factory=list)
    render_roots: list[Path] = field(default_factory=list)
    texture_roots: list[Path] = field(default_factory=list)
    animation_roots: list[Path] = field(default_factory=list)
    resource_roots: list[Path] = field(default_factory=list)

    def describe(self) -> dict[str, object]:
        return {
            "root": str(self.root),
            "model_roots": [str(p) for p in self.model_roots],
            "render_roots": [str(p) for p in self.render_roots],
            "texture_roots": [str(p) for p in self.texture_roots],
            "animation_roots": [str(p) for p in self.animation_roots],
            "resource_roots": [str(p) for p in self.resource_roots],
        }


def _unique_existing(paths: Iterable[Path]) -> list[Path]:
    seen: set[str] = set()
    result: list[Path] = []
    for raw in paths:
        try:
            path = Path(raw).expanduser().resolve()
        except OSError:
            continue
        key = os.path.normcase(str(path))
        if key in seen or not path.exists():
            continue
        seen.add(key)
        result.append(path)
    return result


def discover_mod_layout(mod_root: Path) -> ModLayout:
    """Discover common decompiled-mod and Gradle project layouts from one root.

    This intentionally supports split compiled/decompiled mod layouts such as:
      <root>/xol/lostinfinity/mob/model
      <root>/xol/lostinfinity/mob/render
      <root>/assets/lostinfinity/textures

    It also understands normal src/main/java + src/main/resources projects.
    """
    root = Path(mod_root).expanduser().resolve()
    if not root.is_dir():
        raise ValueError(f"Mod root is not a directory: {root}")

    model_roots: list[Path] = []
    render_roots: list[Path] = []
    texture_roots: list[Path] = []
    animation_roots: list[Path] = []
    resource_roots: list[Path] = []

    # Fast-path conventional roots.
    for candidate in (
        root / "src" / "main" / "java",
        root / "src" / "main" / "resources",
        root / "assets",
        root / "resources",
    ):
        if candidate.exists():
            resource_roots.append(candidate)

    # Named package/resource folders. Keep all sorted subfolders beneath them.
    for path in root.rglob("*"):
        if not path.is_dir():
            continue
        name = path.name.lower()
        if name in {"model", "models"}:
            # Only classify it as a model root when model files actually exist.
            try:
                if any(path.rglob("*.class")) or any(path.rglob("*.java")):
                    model_roots.append(path)
            except OSError:
                pass
        elif name in {"render", "renderer", "renderers"}:
            try:
                if any(path.rglob("*.class")) or any(path.rglob("*.java")):
                    render_roots.append(path)
            except OSError:
                pass
        elif name == "textures":
            # Prefer actual asset/resource texture trees over incidental folders.
            lowered = {part.lower() for part in path.parts}
            if "assets" in lowered or "resources" in lowered:
                texture_roots.append(path)
        elif name in {"animation", "animations"}:
            try:
                if any(path.rglob("*.json")):
                    animation_roots.append(path)
            except OSError:
                pass

    # Fallback for decompiled dumps that keep models in flat package folders
    # instead of a directory literally named model/models (common in 1.7.10).
    # Add the nearest package folder rather than the entire mod root so scans do
    # not attempt every registry/entity/helper class in the archive.
    if not model_roots:
        candidate_parents: dict[Path, int] = {}
        try:
            candidates = list(root.rglob("Model*.class")) + list(root.rglob("Model*.java"))
        except OSError:
            candidates = []
        for candidate in candidates:
            stem = candidate.stem.lower()
            if stem.endswith(("animation", "animations")) or candidate.parent.name.lower() in {"animation", "animations"}:
                continue
            candidate_parents[candidate.parent] = candidate_parents.get(candidate.parent, 0) + 1
        # Prefer folders with several model classes, but still accept a package
        # containing one isolated model when that is all the corpus supplies.
        for parent, _count in sorted(candidate_parents.items(), key=lambda item: (-item[1], str(item[0]).lower())):
            model_roots.append(parent)

    resource_roots.extend(texture_roots)
    resource_roots.extend(animation_roots)
    return ModLayout(
        root=root,
        model_roots=_unique_existing(model_roots),
        render_roots=_unique_existing(render_roots),
        texture_roots=_unique_existing(texture_roots),
        animation_roots=_unique_existing(animation_roots),
        resource_roots=_unique_existing(resource_roots),
    )


class TextureIndex:
    """Indexes textures and links renderer/entity companions to model classes.

    A renderer that mentions ``ModelDeviantPiglin`` and references a PNG is a
    direct graph edge between that model and texture. Multiple renderers can
    therefore expose different creature/color variants for one shared model.
    """

    def __init__(self, roots: Iterable[Path], shallow_roots: Iterable[Path] = ()) -> None:
        self.roots = tuple(_unique_existing(Path(r) for r in roots))
        self.shallow_roots = tuple(_unique_existing(Path(r) for r in shallow_roots))
        self.png_files: list[Path] = []
        self.references: list[TextureReference] = []
        self.records: list[SourceRecord] = []
        _tlog(f"Building texture index: roots={[str(p) for p in self.roots]}, shallow_roots={[str(p) for p in self.shallow_roots]}")
        self._build()
        _tlog(f"Index ready: png_files={len(self.png_files)}, references={len(self.references)}, source_records={len(self.records)}")

    @staticmethod
    def _extract_text_references(text: str) -> set[str]:
        refs: set[str] = set()
        for regex in (_REF_RE, _TEXTURE_PATH_RE):
            for match in regex.finditer(text):
                value = match.group(0).replace("\\", "/").strip("\"' ;,()[]{}")
                if value.lower().endswith(".png"):
                    refs.add(value)
        return refs

    @staticmethod
    def _extract_binary_references(data: bytes) -> set[str]:
        # latin-1 preserves every byte and lets regex recover constant-pool ASCII
        # without treating malformed/decompiler glyphs as meaningful characters.
        text = data.decode("latin-1", errors="ignore")
        refs = TextureIndex._extract_text_references(text)
        for match in _PRINTABLE_PNG_RE.finditer(data):
            raw = match.group(0).decode("latin-1", errors="ignore")
            for ref in _PNG_NAME_RE.findall(raw):
                refs.add(ref.replace("\\", "/").strip("\"' ;,()[]{}"))
        return refs

    @staticmethod
    def _extract_model_mentions(text: str) -> set[str]:
        return {token.split("$")[0] for token in _MODEL_TOKEN_RE.findall(text)}

    @staticmethod
    def _extract_binary_model_mentions(data: bytes) -> set[str]:
        return TextureIndex._extract_model_mentions(data.decode("latin-1", errors="ignore"))

    def _iter_files(self, suffixes: tuple[str, ...], limit: int = 200000) -> Iterable[Path]:
        count = 0
        seen: set[str] = set()
        for root in self.roots:
            if root.is_file():
                candidates = [root]
            else:
                candidates = []
                for suffix in suffixes:
                    candidates.extend(root.rglob(f"*{suffix}"))
            for path in candidates:
                key = os.path.normcase(str(path.resolve()))
                if key in seen:
                    continue
                seen.add(key)
                if count >= limit:
                    return
                count += 1
                yield path

    def _iter_shallow_files(self, suffixes: tuple[str, ...]) -> Iterable[Path]:
        seen: set[str] = set()
        for root in self.shallow_roots:
            if not root.is_dir():
                continue
            try:
                candidates = list(root.iterdir())
            except OSError:
                continue
            for path in candidates:
                if not path.is_file() or path.suffix.lower() not in suffixes:
                    continue
                key = os.path.normcase(str(path.resolve()))
                if key in seen:
                    continue
                seen.add(key)
                yield path

    def _build(self) -> None:
        png_seen: set[str] = set()
        for path in list(self._iter_files((".png",))) + list(self._iter_shallow_files((".png",))):
            key = os.path.normcase(str(path.resolve()))
            if key not in png_seen:
                png_seen.add(key)
                self.png_files.append(path.resolve())
        self.png_files.sort(key=lambda p: str(p).lower())

        files = list(self._iter_files((".java", ".class"))) + list(self._iter_shallow_files((".java", ".class")))
        seen: set[str] = set()
        for path in files:
            key = os.path.normcase(str(path.resolve()))
            if key in seen:
                continue
            seen.add(key)
            try:
                if path.suffix.lower() == ".java":
                    text = path.read_text(encoding="utf-8-sig", errors="ignore")
                    refs = self._extract_text_references(text)
                    mentions = self._extract_model_mentions(text)
                else:
                    data = path.read_bytes()
                    refs = self._extract_binary_references(data)
                    mentions = self._extract_binary_model_mentions(data)
            except OSError:
                continue
            record = SourceRecord(path.resolve(), tuple(sorted(refs)), tuple(sorted(mentions)))
            self.records.append(record)
            for ref in record.texture_refs:
                self.references.append(TextureReference(ref, record.path))
                _tlog(f"Reference: {path.name} -> {ref}; models={list(record.model_mentions)}")

    @staticmethod
    def _canonical(value: str) -> str:
        value = re.sub(r"(?i)^(model|renderer|render|entity)", "", value)
        return re.sub(r"[^a-z0-9]+", "", value.lower())

    @staticmethod
    def _normalise_ref(value: str) -> str:
        value = value.strip().replace("\\", "/").removeprefix("/")
        if ":" in value and not value.startswith(("http:", "https:")):
            namespace, path = value.split(":", 1)
            if namespace:
                value = path
        if value.startswith("assets/"):
            value = value[7:]
        return value.lower()

    @staticmethod
    def _texture_relative_candidates(path: Path) -> set[str]:
        parts = [p.lower() for p in path.parts]
        result = {path.name.lower()}
        for marker in ("textures", "assets"):
            if marker in parts:
                idx = parts.index(marker)
                result.add("/".join(parts[idx:]))
                if marker == "assets" and idx + 1 < len(parts):
                    result.add("/".join(parts[idx + 1:]))
                    if idx + 2 < len(parts):
                        result.add("/".join(parts[idx + 2:]))
        return result

    @staticmethod
    def _is_auxiliary_texture(path: Path) -> bool:
        tokens = {t for t in re.split(r"[^a-z0-9]+", path.stem.lower()) if t}
        # Also catch joined suffixes such as deviantspidereye.
        joined = "".join(tokens)
        return bool(tokens & _AUXILIARY_WORDS) or any(joined.endswith(word) for word in _AUXILIARY_WORDS)

    def _associated_records(self, source: Path, model_name: str) -> list[tuple[SourceRecord, float, str]]:
        source = source.resolve()
        model_canon = self._canonical(model_name)
        result: list[tuple[SourceRecord, float, str]] = []
        for record in self.records:
            score = 0.0
            reason = ""
            mention_canons = {self._canonical(m) for m in record.model_mentions}
            stem_canon = self._canonical(record.path.stem)
            if record.path == source:
                score, reason = 100000.0, "texture reference in model source"
            elif model_canon and model_canon in mention_canons:
                score, reason = 60000.0, f"companion {record.path.name} references exact model class"
            elif stem_canon == model_canon:
                score, reason = 30000.0, f"companion name exactly matches model"
            elif model_canon and stem_canon.endswith(model_canon) and record.path.stem.lower().startswith(("render", "renderer")):
                score, reason = 20000.0, f"renderer name matches model"
            if score:
                result.append((record, score, reason))
        result.sort(key=lambda item: (-item[1], str(item[0].path).lower()))
        return result

    def _match_ref(self, ref: str) -> list[tuple[Path, float, str]]:
        norm = self._normalise_ref(ref)
        if not norm.endswith(".png"):
            return []
        name = Path(norm).name.lower()
        matches: list[tuple[Path, float, str]] = []
        for png in self.png_files:
            keys = self._texture_relative_candidates(png)
            if norm in keys:
                matches.append((png, 100000.0, "exact resource path"))
            elif png.name.lower() == name:
                matches.append((png, 50000.0, "exact resource basename"))
        return matches

    def resolve_set(self, source: Path, model_name: str, explicit: Optional[Path] = None, max_variants: int = 32) -> TextureSetResolution:
        source = source.expanduser().resolve()
        _tlog(f"Resolve-set request: source={source.name}, model_name={model_name}, explicit={explicit or '<none>'}")

        if explicit:
            explicit = explicit.expanduser().resolve()
            if explicit.is_file():
                return TextureSetResolution(explicit, (explicit,), (), ("explicit texture file",))
            if explicit.is_dir():
                scoped = TextureIndex([explicit], shallow_roots=[source.parent])
                return scoped.resolve_set(source, model_name, None, max_variants)
            return TextureSetResolution(None, (), (), (f"texture path does not exist: {explicit}",))

        model_canon = self._canonical(model_name)
        candidates: dict[Path, tuple[float, str]] = {}
        aliases = {model_canon}
        associated = self._associated_records(source, model_name)
        _tlog(f"Associated source records: {len(associated)}")
        for record, relation_score, relation_reason in associated:
            aliases.add(self._canonical(record.path.stem))
            for ref in record.texture_refs:
                for path, ref_score, ref_reason in self._match_ref(ref):
                    score = relation_score + ref_score
                    texture_canon = self._canonical(path.stem)
                    if texture_canon == model_canon:
                        score += 30000.0
                    if self._is_auxiliary_texture(path):
                        score -= 90000.0
                    evidence = f"{relation_reason}; {ref_reason}: {ref}"
                    previous = candidates.get(path)
                    if previous is None or score > previous[0]:
                        candidates[path] = (score, evidence)

        # Conservative family fallback for color suffixes and renderer/entity aliases.
        # It never accepts arbitrary substring matches, so 'fooeye' will not become
        # a body variant unless it was explicitly referenced by an associated class.
        aliases = {a for a in aliases if a}
        for png in self.png_files:
            stem = png.stem.lower()
            stem_canon = self._canonical(stem)
            family = False
            for alias in aliases:
                if stem_canon == alias:
                    family = True
                    break
                # Delimited variant names: name_red, name-blue, name.3.
                if re.match(rf"(?i)^(?:model|render|renderer|entity)?{re.escape(alias)}[_\-.].+", stem):
                    family = True
                    break
            if family and png not in candidates:
                score = 1000.0 + (5000.0 if stem_canon == model_canon else 0.0)
                if self._is_auxiliary_texture(png):
                    score -= 90000.0
                candidates[png] = (score, "conservative model/renderer texture-family match")

        if not candidates:
            _tlog("No texture match found.", "WARN")
            return TextureSetResolution(None)

        ordered = sorted(candidates.items(), key=lambda item: (-item[1][0], len(item[0].parts), str(item[0]).lower()))
        auxiliaries = tuple(path for path, (score, _ev) in ordered if self._is_auxiliary_texture(path))
        body = [(path, score, ev) for path, (score, ev) in ordered if not self._is_auxiliary_texture(path)]
        if not body:
            # Do not silently choose an eye/glow texture as the model's main skin.
            _tlog(f"Only auxiliary textures matched: {[str(p) for p in auxiliaries]}", "WARN")
            return TextureSetResolution(None, (), auxiliaries, ("only auxiliary/overlay textures matched",))

        primary, primary_score, primary_ev = body[0]
        variants = tuple(path for path, _score, _ev in body[:max_variants])
        details = tuple((str(path), score, ev) for path, score, ev in body[:max_variants])
        _tlog(f"Primary texture: {primary} score={primary_score:.3f} ({primary_ev})")
        for path, score, ev in body[:max_variants]:
            _tlog(f"  body/variant score={score:.3f}: {path} ({ev})")
        for path in auxiliaries:
            _tlog(f"  skipped auxiliary texture: {path}")
        return TextureSetResolution(primary, variants, auxiliaries, (primary_ev,), details)

    def resolve(self, source: Path, model_name: str, explicit: Optional[Path] = None) -> TextureResolution:
        result = self.resolve_set(source, model_name, explicit)
        score = 0.0
        if result.primary:
            for path, candidate_score, _ev in result.candidate_details:
                if Path(path) == result.primary:
                    score = candidate_score
                    break
        return TextureResolution(result.primary, score, result.evidence)


def infer_search_roots(source: Path, extra_roots: Iterable[Path] = ()) -> list[Path]:
    source = source.expanduser().resolve()
    roots: list[Path] = [Path(root) for root in extra_roots]
    current = source.parent if source.is_file() else source
    ancestors = [current, *list(current.parents)[:7]]
    for ancestor in ancestors:
        for candidate in (
            ancestor / "src" / "main" / "resources",
            ancestor / "src" / "main" / "java",
            ancestor / "src",
            ancestor / "resources",
            ancestor / "assets",
        ):
            if candidate.exists():
                roots.append(candidate)
    return _unique_existing(roots)
