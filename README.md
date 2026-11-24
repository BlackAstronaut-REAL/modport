# MojModelBridge

**Project roadmap:** [ROADMAP.md](ROADMAP.md) — Rust engine parity, the standalone desktop application, and the integrated model/animation viewer.

This repository currently contains the transitional Python/Tkinter desktop shell and the independent native Rust rewrite under [rust/](rust/). The Rust CLI and library are being developed as the future primary engine; the target desktop application will call that same Rust library directly and will not require the Python engine.

## Preservation rule

The files under [oldVer/](oldVer/) are the known-working MojModelBridge 3.4.0 Beta 12 conversion baseline and remain the behavioral reference while Rust parity is established. The current transitional desktop shell still imports and calls `oldVer/model_bridge.py`; it has not yet been migrated to the Rust engine. Keep the legacy engine unchanged unless a separately reviewed reference fix is required. The Rust implementation and promotion gates are tracked in [ROADMAP.md](ROADMAP.md) and [rust/docs/ENGINE_MIGRATION.md](rust/docs/ENGINE_MIGRATION.md).

## Current application pages

- **Porter:** functional front end to the existing --full-port workflow. Accepts a raw JAR/ZIP, one combined input root, or separate compiled/decompiled roots; writes Blockbench/ and MCreatorWorkspace/ under the chosen output folder.
- **Mod Editor:** hand-off to the generated editable MCreator workspace. The in-app unified element editor is a later phase.
- **Code Studio:** opens generated project and engine source folders for editing in the user's preferred editor.
- **Compatibility:** currently exposes the generator targets the existing engine already supports: NeoForge 1.21.1 and Forge 1.20.1.
- **Build & Test:** opens the exported workspace and can run its Gradle wrapper when present.
- **Projects:** remembers recent project/output folders in the user's application-data directory.

The long-term plan for complex logic is to retain recovered source/bytecode evidence, represent behavior independently of the target loader, and translate or rebuild it using MCreator's procedure/text representation where appropriate. Unsupported sections must remain attached to the project with provenance and conversion warnings rather than silently disappearing. Logic rewriting is not part of this baseline.

## Native Rust desktop prototype

A separate native Rust desktop application is now under [rust/desktop/](rust/desktop/). It keeps the existing far-left navigation as a collapsible rail and adds a categorized content browser, textured model/animation preview, and basic name/lore/component editor with a live Minecraft tooltip preview.

Run it from the repository root:

```powershell
cargo run --manifest-path rust/desktop/Cargo.toml
```

The desktop app uses the native Rust engine library and does not load oldVer/. Its current browser indexes extracted mod directories; item/block entries are texture-derived, and editor fields are preview drafts rather than workspace write-back. See [rust/desktop/README.md](rust/desktop/README.md) for build instructions and prototype boundaries. The established Python/Tkinter application below remains available while the Rust app is developed.

## Run from source

Requirements: Python 3.10+ and a JDK with javap available on PATH for compiled .class analysis.

```bat
py -3 app.py
```

The app expects the oldVer/ folder beside app.py.

## Build the Windows EXE

```bat
build_windows.bat
```

Or:

```bat
py -3 -m pip install pyinstaller
py -3 -m PyInstaller --noconfirm --clean MojModelBridge.spec
```

The executable is created at dist\MojModelBridge.exe. The original engine Python modules and MCreator schemas are bundled as data inside the one-file EXE. A JDK is still required at runtime when converting compiled Java classes; the EXE does not bundle a Java runtime.

A GitHub Actions workflow runs the legacy regression suite as a report-only check, then builds a Windows x64 executable on pushes affecting the application/engine and uploads it as the MojModelBridge-Windows-x64 artifact. Some historical tests depend on sample folders outside the repository; the current CI report also identifies two animation-loop endpoint assertions that remain unchanged in the preserved engine.

## Current baseline limits

- Raw JAR/ZIP files are safely extracted to a per-user cache before the original folder-based engine scans them.
- In-app element creation/editing, procedure graph editing, universal source/target adapters, and automated game launch tests are future work.
- Conversion output and reports are produced by the unchanged legacy engine; runtime parity should still be checked in the target Minecraft/MCreator environment.
