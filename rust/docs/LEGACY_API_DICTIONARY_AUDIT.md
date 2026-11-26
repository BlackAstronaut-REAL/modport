# Legacy Forge API dictionary audit

The source API catalogue is audited before bulk expansion. The audit compares
the Rust dictionaries with the actual index-all.html method indexes for Forge
1.7.10, 1.8.9, and 1.12.2.

## Run

From the repository root, with Rust/Cargo and Python 3.10+ available:

    py -3 tools/audit_legacy_api_dictionary.py

The first run downloads and caches each JavaDoc index under
.cache/forge-javadocs/. Later runs reuse the cache. Use --offline to require
cached indexes, --write-markdown PATH to save the summary, and --write-json PATH
to save all findings. The dictionary exporter can also be run directly:

    cargo run --manifest-path rust/Cargo.toml --example dump_legacy_api_dictionary

Run the offline parser test with:

    py -3 tools/audit_legacy_api_dictionary.py --self-test

## What is measured

- Documented methods: unique class-page path + method-name pairs indexed in the
  Minecraft and Forge/FML namespaces for that exact release. Constructors and
  fields are excluded.
- Coverage: documented class-page/method pairs matched by a dictionary record
  for that release. Full class-page paths are used so identically named classes
  in different packages cannot inflate the percentage.
- Stale mappings: dictionary entries whose owner/method cannot be found in the
  release index. These may indicate bad names, owner types, or version masks.
- Documentation-link mismatches: the method exists under the owner, but its
  stored class-page URL points elsewhere. A unique fallback owner is reported
  but remains visible as a mismatch.
- Ambiguous owners: a stale URL has multiple same-named owner candidates. These
  are not guessed or counted as coverage.
- Duplicates: repeated owner/method entries in one release.
- Unmapped methods: documented methods without a dictionary entry; these form
  the evidence-backed backlog for expansion.

A mapping only establishes that ModPort recognizes a source API. Its conversion
status, target contract, and required semantic adapter are separate measures.
Dictionary coverage is not automatic conversion coverage or proof of 1:1 behavior.

## Source indexes

- Forge 1.7.10: https://skmedix.github.io/ForgeJavaDocs/javadoc/forge/1.7.10-10.13.4.1614/index-all.html
- Forge 1.8.9: https://skmedix.github.io/ForgeJavaDocs/javadoc/forge/1.8.9-11.15.1.2318/index-all.html
- Forge 1.12.2: https://skmedix.github.io/ForgeJavaDocs/javadoc/forge/1.12.2-14.23.5.2859/index-all.html

The totals describe public and protected methods indexed by these JavaDocs, not
private implementation details. The linked JavaDoc indexes are the audit source
of truth; avoid replacing them with hand-maintained API lists.
