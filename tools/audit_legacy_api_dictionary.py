#!/usr/bin/env python3
"""Compare ModPort's legacy API dictionary with the published Forge JavaDoc indexes.

This audit counts documented owner/method pairs, not just method names. It catches
stale class paths, wrong release scopes, duplicate entries, and undocumented mappings
before the dictionary is expanded further.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import Counter, defaultdict
from html.parser import HTMLParser
from pathlib import Path
from typing import Any
from urllib.parse import unquote, urljoin, urlparse
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
VERSIONS = {
    "1.7.10": {
        "url": "https://skmedix.github.io/ForgeJavaDocs/javadoc/forge/1.7.10-10.13.4.1614/index-all.html",
    },
    "1.8.9": {
        "url": "https://skmedix.github.io/ForgeJavaDocs/javadoc/forge/1.8.9-11.15.1.2318/index-all.html",
    },
    "1.12.2": {
        "url": "https://skmedix.github.io/ForgeJavaDocs/javadoc/forge/1.12.2-14.23.5.2859/index-all.html",
    },
}
API_PACKAGE_PREFIXES = (
    "net/minecraft/",
    "net/minecraftforge/",
    "cpw/mods/fml/",
)
METHOD_KIND = re.compile(r"\b(?:(?:Static|Default)\s+)?Method in (?:class|interface|enum|annotation type)\b", re.IGNORECASE)


class JavaDocIndexParser(HTMLParser):
    """Extract method owner paths from old and new Javadoc index-all pages."""

    def __init__(self, index_url: str) -> None:
        super().__init__(convert_charrefs=True)
        self.index_url = index_url
        self.in_dt = False
        self.current_link: dict[str, str] | None = None
        self.links: list[dict[str, str]] = []
        self.entry_text: list[str] = []
        self.methods: set[tuple[str, str]] = set()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attrs_map = dict(attrs)
        if tag.lower() == "dt":
            self.in_dt = True
            self.current_link = None
            self.links = []
            self.entry_text = []
        elif self.in_dt and tag.lower() == "a":
            self.current_link = {"href": attrs_map.get("href") or "", "text": ""}

    def handle_data(self, data: str) -> None:
        if not self.in_dt:
            return
        self.entry_text.append(data)
        if self.current_link is not None:
            self.current_link["text"] += data

    def handle_endtag(self, tag: str) -> None:
        if tag.lower() == "a" and self.current_link is not None:
            self.links.append(self.current_link)
            self.current_link = None
        elif tag.lower() == "dt" and self.in_dt:
            self._finish_entry()
            self.in_dt = False
            self.current_link = None

    def _finish_entry(self) -> None:
        text = re.sub(r"\\s+", " ", "".join(self.entry_text)).strip()
        kind_match = METHOD_KIND.search(text)
        if kind_match is None:
            return

        # Javadoc sometimes links member names and sometimes emits the member
        # signature as plain text. Parse the text in both cases; requiring a
        # member anchor silently discarded large groups of documented methods.
        member_link = next(
            (
                item for item in self.links
                if item["href"] and "#" in item["href"] and item["text"].strip()
            ),
            None,
        )
        member_display = (
            member_link["text"].strip()
            if member_link is not None
            else text[:kind_match.start()].strip()
        )
        member_display = re.sub(r"^(?:(?:public|protected|static|default)\\s+)+", "", member_display)
        method_name = member_display.split("(", 1)[0].strip().split()[-1] if member_display else ""
        if not re.match(r"^[A-Za-z_$][A-Za-z0-9_$]*$", method_name):
            return

        owner_link = next(
            (
                item for item in reversed(self.links)
                if item["href"]
                and ".html" in item["href"].lower()
                and "#" not in item["href"]
            ),
            None,
        )
        if owner_link is not None:
            owner_url = urljoin(self.index_url, owner_link["href"])
        elif member_link is not None:
            owner_url = urljoin(self.index_url, member_link["href"].split("#", 1)[0])
        else:
            # A few generated index rows have no class anchor. Recover the
            # fully qualified owner from the descriptive text without guessing
            # nested-class package separators.
            owner_match = re.search(
                r"\\bMethod in (?:class|interface|enum|annotation type)\\s+([A-Za-z_$][A-Za-z0-9_$.]*)",
                text,
                re.IGNORECASE,
            )
            if owner_match is None:
                return
            owner_name = owner_match.group(1)
            package_owner = owner_name.replace(".", "/") + ".html"
            self._add_method_path(package_owner, method_name, None)
            return

        index_path = urlparse(self.index_url).path
        base_path = index_path.rsplit("/", 1)[0].rstrip("/") + "/"
        owner_path = unquote(urlparse(owner_url).path)
        if not owner_path.startswith(base_path):
            return
        relative_path = owner_path[len(base_path):].lstrip("/")
        self._add_method_path(relative_path, method_name, base_path)

    def _add_method_path(
        self,
        relative_path: str,
        method_name: str,
        base_path: str | None,
    ) -> None:
        if base_path is None:
            if not relative_path.startswith(API_PACKAGE_PREFIXES):
                return
            self.methods.add((relative_path, method_name))
            return
        if relative_path.startswith(API_PACKAGE_PREFIXES):
            self.methods.add((relative_path, method_name))


def parse_javadoc_index(html: str, index_url: str) -> set[tuple[str, str]]:
    parser = JavaDocIndexParser(index_url)
    parser.feed(html)
    parser.close()
    return parser.methods


def load_index(version: str, cache_dir: Path, offline: bool) -> set[tuple[str, str]]:
    url = VERSIONS[version]["url"]
    cache_dir.mkdir(parents=True, exist_ok=True)
    cache_file = cache_dir / f"forge-{version}-index-all.html"
    if cache_file.is_file():
        html = cache_file.read_text(encoding="utf-8", errors="replace")
    else:
        if offline:
            raise RuntimeError(f"missing cached JavaDoc index: {cache_file}")
        request = Request(url, headers={"User-Agent": "ModPort-JavaDoc-Audit/1.0"})
        with urlopen(request, timeout=90) as response:
            html = response.read().decode("utf-8", errors="replace")
        cache_file.write_text(html, encoding="utf-8")
    methods = parse_javadoc_index(html, url)
    if not methods:
        raise RuntimeError(f"no Forge/Minecraft methods parsed from {url}")
    return methods


def load_dictionary(dictionary_json: Path | None) -> dict[str, list[dict[str, Any]]]:
    if dictionary_json is not None:
        data = json.loads(dictionary_json.read_text(encoding="utf-8"))
    else:
        command = [
            "cargo", "run", "--quiet",
            "--manifest-path", str(ROOT / "rust" / "Cargo.toml"),
            "--example", "dump_legacy_api_dictionary",
        ]
        completed = subprocess.run(
            command, cwd=ROOT, check=True, capture_output=True, text=True
        )
        data = json.loads(completed.stdout)
    if not isinstance(data, dict):
        raise ValueError("dictionary export must be a JSON object keyed by version")
    return data


def docs_page_path(url: str) -> str | None:
    path = unquote(urlparse(url).path)
    for prefix in API_PACKAGE_PREFIXES:
        position = path.find(prefix)
        if position >= 0:
            candidate = path[position:]
            if candidate.endswith(".html"):
                return candidate
    return None


def audit_version(
    version: str,
    documented: set[tuple[str, str]],
    mappings: list[dict[str, Any]],
    documented_by_version: dict[str, set[tuple[str, str]]],
) -> dict[str, Any]:
    # Keep the full JavaDoc class-page path in the coverage key. Class simple
    # names are not globally unique, so package collisions cannot inflate coverage.
    docs_by_owner: dict[str, set[str]] = defaultdict(set)
    methods_by_page: dict[str, set[str]] = defaultdict(set)
    for page, method in documented:
        methods_by_page[page].add(method)
        docs_by_owner[Path(page).stem].add(page)

    duplicate_counter: Counter[tuple[str, str]] = Counter()
    mapped_pages: set[tuple[str, str]] = set()
    stale: list[dict[str, Any]] = []
    doc_link_mismatches: list[dict[str, str]] = []
    ambiguous: list[dict[str, str]] = []

    for item in mappings:
        owner = str(item.get("source_owner", ""))
        method = str(item.get("source_method", ""))
        if not owner or not method:
            continue
        duplicate_counter[(owner, method)] += 1

        urls = [
            str(url) for url in item.get("documentation_urls", [])
            if isinstance(url, str)
        ]
        expected_pages = {
            page for page in (docs_page_path(url) for url in urls) if page
        }
        exact_pages = sorted(
            page for page in expected_pages
            if Path(page).stem == owner
            and method in methods_by_page.get(page, set())
        )

        if exact_pages:
            mapped_pages.update((page, method) for page in exact_pages)
            continue

        # Recover a bad/missing URL only when owner + method identifies one
        # unique class page. Never guess between same-named classes in packages.
        fallback_pages = sorted(
            page for page in docs_by_owner.get(owner, set())
            if method in methods_by_page.get(page, set())
        )
        if len(fallback_pages) == 1:
            page = fallback_pages[0]
            mapped_pages.add((page, method))
            doc_link_mismatches.append({
                "owner": owner,
                "method": method,
                "expected_pages": "; ".join(sorted(expected_pages)),
                "actual_pages": page,
            })
        elif len(fallback_pages) > 1:
            ambiguous.append({
                "owner": owner,
                "method": method,
                "expected_pages": "; ".join(sorted(expected_pages)),
                "candidate_pages": "; ".join(fallback_pages),
            })
        else:
            other_release_matches: dict[str, list[str]] = {}
            for other_version, other_documented in documented_by_version.items():
                if other_version == version:
                    continue
                pages = sorted(
                    page for page, candidate_method in other_documented
                    if candidate_method == method and Path(page).stem == owner
                )
                if pages:
                    other_release_matches[other_version] = pages
            stale.append({
                "owner": owner,
                "method": method,
                "documentation_urls": "; ".join(urls),
                "documented_in_other_versions": other_release_matches,
            })

    duplicate_pairs = [
        {"owner": owner, "method": method, "count": count}
        for (owner, method), count in sorted(duplicate_counter.items())
        if count > 1
    ]
    unmapped = sorted(
        documented - mapped_pages,
        key=lambda pair: (pair[0].casefold(), pair[1].casefold()),
    )
    scope_candidates = [
        item for item in stale if item["documented_in_other_versions"]
    ]
    return {
        "version": version,
        "documented_method_owner_pairs": len(documented),
        "documented_class_pages": len(methods_by_page),
        "dictionary_records": len(mappings),
        "unique_dictionary_owner_method_pairs": len(duplicate_counter),
        "validated_dictionary_pairs": len(mapped_pages),
        "coverage_percent": round(
            100.0 * len(mapped_pages) / len(documented), 2
        ) if documented else 0.0,
        "stale_or_undocumented_mappings": stale,
        "version_scope_candidates": scope_candidates,
        "documentation_link_mismatches": doc_link_mismatches,
        "ambiguous_owner_method_mappings": ambiguous,
        "duplicate_dictionary_pairs": duplicate_pairs,
        # Complete release-specific JavaDoc catalogue. This makes the audit
        # output the reproducible source for adding still-unmapped API methods.
        "documented_methods": [
            {"class_page": page, "owner": Path(page).stem, "method": method}
            for page, method in sorted(documented, key=lambda pair: (pair[0].casefold(), pair[1].casefold()))
        ],
        "unmapped_methods": [
            {"class_page": page, "owner": Path(page).stem, "method": method}
            for page, method in unmapped
        ],
    }


def build_scope_recommendations(
    documented_by_version: dict[str, set[tuple[str, str]]],
    dictionary: dict[str, list[dict[str, Any]]],
) -> list[dict[str, Any]]:
    """Recommend exact release masks from owner-page/method presence in JavaDocs.

    Recommendations are emitted only when the owner is unambiguous in every
    release. They are evidence-backed scope fixes, not guessed API equivalences.
    """
    methods_by_page: dict[str, dict[str, set[str]]] = {}
    pages_by_owner: dict[str, dict[str, set[str]]] = {}
    for version, documented in documented_by_version.items():
        methods_by_page[version] = defaultdict(set)
        pages_by_owner[version] = defaultdict(set)
        for page, method in documented:
            methods_by_page[version][page].add(method)
            pages_by_owner[version][Path(page).stem].add(page)

    current_versions: dict[tuple[str, str], set[str]] = defaultdict(set)
    url_pages: dict[tuple[str, str], set[str]] = defaultdict(set)
    for version, mappings in dictionary.items():
        for item in mappings:
            owner = str(item.get("source_owner", ""))
            method = str(item.get("source_method", ""))
            if not owner or not method:
                continue
            pair = (owner, method)
            current_versions[pair].add(version)
            for url in item.get("documentation_urls", []):
                if isinstance(url, str):
                    page = docs_page_path(url)
                    if page:
                        url_pages[pair].add(page)

    recommendations: list[dict[str, Any]] = []
    for (owner, method), current in sorted(current_versions.items()):
        documented_versions: set[str] = set()
        ambiguous: dict[str, list[str]] = {}
        for version in documented_by_version:
            page_matches = {
                page for page in url_pages[(owner, method)]
                if method in methods_by_page[version].get(page, set())
                and Path(page).stem == owner
            }
            if len(page_matches) == 1:
                documented_versions.add(version)
                continue
            # When a class page path has moved between releases or a row has no
            # URL, recover only if owner + method identifies exactly one page.
            fallback = sorted(
                page for page in pages_by_owner[version].get(owner, set())
                if method in methods_by_page[version].get(page, set())
            )
            if len(fallback) == 1:
                documented_versions.add(version)
            elif len(fallback) > 1:
                ambiguous[version] = fallback

        if ambiguous or documented_versions == current:
            continue
        if not documented_versions:
            continue
        recommendations.append({
            "owner": owner,
            "method": method,
            "current_versions": sorted(current),
            "documented_versions": sorted(documented_versions),
            "remove_versions": sorted(current - documented_versions),
            "add_versions": sorted(documented_versions - current),
            "reason": "JavaDoc owner-page/method presence differs from the current dictionary release mask.",
        })
    return recommendations


def render_markdown(reports: list[dict[str, Any]]) -> str:
    lines = [
        "# Legacy Forge API Dictionary Audit",
        "",
        "Generated by tools/audit_legacy_api_dictionary.py against the linked Forge JavaDocs.",
        "",
        "Coverage is the percentage of unique documented API owner/method pairs in the",
        "net.minecraft, net.minecraftforge, and cpw.mods.fml packages that have a",
        "dictionary entry for that release. This is not a claim that every behavior is",
        "automatically portable; conversion status and semantic adapters remain separate.",
        "",
        "| Forge version | Documented API methods | Dictionary records | Validated mappings | Coverage | Stale mappings | Link mismatches | Ambiguous | Duplicate pairs |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for report in reports:
        lines.append(
            f"| {report['version']} "
            f"| {report['documented_method_owner_pairs']} "
            f"| {report['dictionary_records']} "
            f"| {report['validated_dictionary_pairs']} "
            f"| {report['coverage_percent']:.2f}% "
            f"| {len(report['stale_or_undocumented_mappings'])} "
            f"| {len(report['documentation_link_mismatches'])} "
            f"| {len(report['ambiguous_owner_method_mappings'])} "
            f"| {len(report['duplicate_dictionary_pairs'])} |"
        )

    for report in reports:
        lines.extend([
            "",
            f"## {report['version']}",
            "",
            f"- Documented method/owner pairs: **{report['documented_method_owner_pairs']}**",
            f"- Dictionary records: **{report['dictionary_records']}**",
            f"- Validated mappings: **{report['validated_dictionary_pairs']}**",
            f"- Coverage: **{report['coverage_percent']:.2f}%**",
            f"- Stale or undocumented mappings: **{len(report['stale_or_undocumented_mappings'])}**",
            f"- Stale mappings also documented in another release: **{len(report['version_scope_candidates'])}**",
            f"- Documentation-link mismatches: **{len(report['documentation_link_mismatches'])}**",
            f"- Ambiguous owner/method mappings: **{len(report['ambiguous_owner_method_mappings'])}**",
            f"- Duplicate owner/method pairs: **{len(report['duplicate_dictionary_pairs'])}**",
        ])
        if report["stale_or_undocumented_mappings"]:
            lines.extend(["", "### Mappings not found in this release's JavaDocs", ""])
            for item in report["stale_or_undocumented_mappings"][:40]:
                line = f"- {item['owner']}#{item['method']}"
                other = item.get("documented_in_other_versions", {})
                if other:
                    line += f" (documented in: {', '.join(sorted(other))}; review version mask)"
                lines.append(line)
            if len(report["stale_or_undocumented_mappings"]) > 40:
                lines.append(
                    f"- ...and {len(report['stale_or_undocumented_mappings']) - 40} more (see JSON output)"
                )
        if report["documentation_link_mismatches"]:
            lines.extend(["", "### Documentation URL mismatches", ""])
            for item in report["documentation_link_mismatches"][:30]:
                lines.append(
                    f"- {item['owner']}#{item['method']}: expected "
                    f"{item['expected_pages']}, found {item['actual_pages']}"
                )
        if report["ambiguous_owner_method_mappings"]:
            lines.extend(["", "### Ambiguous same-name owners", ""])
            for item in report["ambiguous_owner_method_mappings"][:30]:
                lines.append(
                    f"- {item['owner']}#{item['method']}: "
                    f"{item['candidate_pages']}"
                )
        lines.extend(["", "### First 60 unmapped documented methods", ""])
        for item in report["unmapped_methods"][:60]:
            lines.append(
                f"- {item['class_page']}#{item['method']}"
            )
        if len(report["unmapped_methods"]) > 60:
            lines.append(
                f"- ...and {len(report['unmapped_methods']) - 60} more (see JSON output)"
            )
    lines.append("")
    return "\n".join(lines)


def self_test() -> None:
    url = "https://example.invalid/javadoc/forge/1.12.2/index-all.html"
    html = """
    <dl>
      <dt><span class="memberNameLink"><a href="net/minecraft/block/Block.html#addDestroyEffects()">addDestroyEffects()</a></span> - Method in class <a href="net/minecraft/block/Block.html">net.minecraft.block.Block</a></dt>
      <dt><span class="memberNameLink"><a href="net/minecraft/world/IWorld.html#getHeight()">getHeight()</a></span> - Default method in interface <a href="net/minecraft/world/IWorld.html">net.minecraft.world.IWorld</a></dt>
      <dt>isFullCube(IBlockState) - Method in class <a href="net/minecraft/block/Block.html">net.minecraft.block.Block</a></dt>
      <dt><a href="net/minecraft/util/MathHelper.html#floor(double)">floor(double)</a> - Static method in class <a href="net/minecraft/util/MathHelper.html">net.minecraft.util.MathHelper</a></dt>
      <dt><span class="memberNameLink"><a href="net/minecraft/block/Block.html#Block()">Block()</a></span> - Constructor for class <a href="net/minecraft/block/Block.html">net.minecraft.block.Block</a></dt>
      <dt><span class="memberNameLink"><a href="java/lang/Object.html#toString()">toString()</a></span> - Method in class <a href="java/lang/Object.html">java.lang.Object</a></dt>
    </dl>
    """
    parsed = parse_javadoc_index(html, url)
    expected = {
        ("net/minecraft/block/Block.html", "addDestroyEffects"),
        ("net/minecraft/world/IWorld.html", "getHeight"),
        ("net/minecraft/block/Block.html", "isFullCube"),
        ("net/minecraft/util/MathHelper.html", "floor"),
    }
    if parsed != expected:
        raise AssertionError(f"JavaDoc index parser self-test failed: {parsed!r}")
    print("JavaDoc index parser self-test passed.")

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--dictionary-json", type=Path,
        help="Use an exported dictionary JSON file instead of compiling the Rust example.",
    )
    parser.add_argument(
        "--cache-dir", type=Path, default=ROOT / ".cache" / "forge-javadocs",
        help="Directory for downloaded index-all.html files.",
    )
    parser.add_argument(
        "--offline", action="store_true",
        help="Use only JavaDoc index files already present in --cache-dir.",
    )
    parser.add_argument("--write-markdown", type=Path, help="Write the Markdown report to this path.")
    parser.add_argument("--write-json", type=Path, help="Write complete audit findings as JSON.")
    parser.add_argument("--write-scope-suggestions", type=Path, help="Write JavaDoc-derived release-mask correction suggestions as JSON.")
    parser.add_argument("--self-test", action="store_true", help="Run the parser self-test and exit.")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0

    try:
        dictionary = load_dictionary(args.dictionary_json)
        documented_by_version = {
            version: load_index(version, args.cache_dir, args.offline)
            for version in VERSIONS
        }
        scope_suggestions = build_scope_recommendations(
            documented_by_version,
            dictionary,
        )
        reports = []
        for version in VERSIONS:
            mappings = dictionary.get(version)
            if not isinstance(mappings, list):
                raise ValueError(f"dictionary export has no mapping list for {version}")
            reports.append(
                audit_version(
                    version,
                    documented_by_version[version],
                    mappings,
                    documented_by_version,
                )
            )
    except Exception as error:  # noqa: BLE001 - one actionable CLI error
        print(f"Audit failed: {error}", file=sys.stderr)
        return 2

    markdown = render_markdown(reports)
    print(markdown)
    if args.write_markdown:
        args.write_markdown.parent.mkdir(parents=True, exist_ok=True)
        args.write_markdown.write_text(markdown, encoding="utf-8")
    if args.write_json:
        args.write_json.parent.mkdir(parents=True, exist_ok=True)
        args.write_json.write_text(
            json.dumps(reports, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    if args.write_scope_suggestions:
        args.write_scope_suggestions.parent.mkdir(parents=True, exist_ok=True)
        args.write_scope_suggestions.write_text(
            json.dumps(scope_suggestions, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    print(f"JavaDoc release-scope correction candidates: {len(scope_suggestions)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
