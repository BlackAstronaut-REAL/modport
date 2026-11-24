# MojModelBridge Desktop

This is the first desktop application shell around the working MojModelBridge 3.4.0 Beta 12 engine preserved in [oldVer/](oldVer/).

## Preservation rule

The files under oldVer/ are the known-working conversion baseline. The desktop shell imports and calls oldVer/model_bridge.py directly, including its existing full-port pipeline and exporters. It does not replace the model parser, mob geometry handling, animation recovery, texture resolution, MCreator element generation, or report generation. Engine changes are intentionally deferred.

## Current application pages

- **Porter:** functional front end to the existing --full-port workflow. Accepts a raw JAR/ZIP, one combined input root, or separate compiled/decompiled roots; writes Blockbench/ and MCreatorWorkspace/ under the chosen output folder.
- **Mod Editor:** hand-off to the generated editable MCreator workspace. The in-app unified element editor is a later phase.
- **Code Studio:** opens generated project and engine source folders for editing in the user's preferred editor.
- **Compatibility:** currently exposes the generator targets the existing engine already supports: NeoForge 1.21.1 and Forge 1.20.1.
- **Build & Test:** opens the exported workspace and can run its Gradle wrapper when present.
- **Projects:** remembers recent project/output folders in the user's application-data directory.

The long-term plan for rewritten complex logic is to retain recoverable logic and translate/rebuild it using MCreator's procedure/text representation where appropriate. That work is not part of this baseline; no procedure rewriting is performed by the desktop shell.

## Run from source

Requirements: Python 3.10+ and a JDK with javap available on PATH for compiled .class analysis.

\`\`\`bat
py -3 app.py
\`\`\`

The app expects the oldVer/ folder beside app.py.

## Build the Windows EXE

\`\`\`bat
build_windows.bat
\`\`\`

Or:

\`\`\`bat
py -3 -m pip install pyinstaller
py -3 -m PyInstaller --noconfirm --clean MojModelBridge.spec
\`\`\`

The executable is created at dist\MojModelBridge.exe. The original engine Python modules and MCreator schemas are bundled as data inside the one-file EXE. A JDK is still required at runtime when converting compiled Java classes; the EXE does not bundle a Java runtime.

A GitHub Actions workflow builds a Windows x64 executable on pushes affecting the application/engine and uploads it as the MojModelBridge-Windows-x64 artifact.

## Current baseline limits

- Raw JAR/ZIP files are safely extracted to a per-user cache before the original folder-based engine scans them.
- In-app element creation/editing, procedure graph editing, universal source/target adapters, and automated game launch tests are future work.
- Conversion output and reports are produced by the unchanged legacy engine; runtime parity should still be checked in the target Minecraft/MCreator environment.
