# MojModelBridge Roadmap

**Direction:** make the Rust rewrite the primary MojModelBridge engine and ship it as a standalone desktop application, while retaining a command-line interface for batch work and automation.

**Current implementation branch:** `rust-rewrite`  
**Behavioral reference:** `oldVer/` (MojModelBridge 3.4.0 Beta 12; preserve as the regression oracle until the Rust replacement is accepted).

This is the product and engineering roadmap. The lower-level module-by-module port map remains in [rust/docs/ENGINE_MIGRATION.md](rust/docs/ENGINE_MIGRATION.md).

---

## 1. Product goals

1. **One native Rust engine.** The CLI and desktop application must call the same Rust library. The desktop app must not import, spawn, or bundle the Python engine.
2. **Standalone desktop workflow.** Users can open a mod, inspect what was discovered, configure a port, run conversions, inspect warnings, validate/build the result, and reopen recent projects without living in a terminal.
3. **Integrated model and animation viewer.** Users can browse models recovered from a mod, inspect part hierarchies and textures, and preview their animations inside MojModelBridge. This is a read-only viewer first; it does not need to become a full Blockbench replacement.
4. **Parity before replacement.** Rust becomes the main implementation only after the legacy regression corpus and real workspace builds meet the acceptance gates in section 7.
5. **Preserve evidence.** Unsupported behavior, source/bytecode provenance, confidence, warnings, and original assets must remain inspectable. A successful export must not imply that unsupported behavior was faithfully ported.

### Product boundaries

- Keep the **CLI**. It is useful for automation, CI, batch conversion, and debugging even after the desktop app ships.
- Keep the **viewer** separate from the parser and exporters. It consumes the engine's model/animation representation; it must not grow a second model parser.
- Treat the **Mod Editor** as a later, separate feature. First make generated workspace content inspectable and buildable; then add focused native element editing. Procedure-graph editing is a much larger follow-on.
- Preserve `oldVer/` as a read-only reference during the migration. Do not delete it as part of the first Rust desktop release.

---

## 2. Target architecture

The intended data flow is:

```text
                  ┌──────────────────────────┐
                  │     Desktop application  │
                  │ projects · porter ·      │
                  │ viewer · build · reports  │
                  └────────────┬─────────────┘
                               │
┌───────────────────┐          │          ┌────────────────────┐
│ CLI / automation  │──────────┼──────────│ Future integrations│
└───────────────────┘          │          └────────────────────┘
                               ▼
                  ┌──────────────────────────┐
                  │     Rust engine library  │
                  │ scan · parse · IR ·      │
                  │ convert · export · build │
                  └────────────┬─────────────┘
                               │
                 ┌─────────────┴──────────────┐
                 ▼                            ▼
       ┌──────────────────┐         ┌─────────────────────┐
       │ Model/animation  │         │ MCreator workspace  │
       │ viewer scene IR  │         │ elements/resources  │
       └────────┬─────────┘         └─────────────────────┘
                ▼
       ┌──────────────────┐
       │ GPU viewport     │
       │ meshes · UVs ·   │
       │ hierarchy · pose │
       └──────────────────┘
```

### Rust workspace layout (target)

Keep the existing `mojmodelbridge` library and CLI as the core. Add the desktop app as a separate binary/package in the Rust workspace rather than mixing GUI dependencies into the conversion engine.

- `rust/src/`: engine library and CLI; no GUI or GPU types in core conversion modules.
- `rust/src/viewer/`: engine-side scene construction, mesh/UV generation, hierarchy transforms, and animation sampling adapters. This stays CPU-testable and independent of the graphics API.
- `rust/desktop/`: native desktop shell, project/session management, background jobs, and panels.
- `rust/desktop/viewer/` (or a small renderer crate): GPU viewport implementation and graphics resources.
- `rust/docs/`: detailed port map, design decisions, and test strategy.

**Initial UI/rendering candidate:** egui/eframe for the application shell and panels, with a wgpu-backed 3D viewport. Validate this with a small prototype before committing to the dependency stack. The viewer's scene builder must remain renderer-agnostic so the UI or GPU backend can change without touching parsing/export logic.

### Shared data contracts

- `ParsedModel` remains the canonical parsed-model input: hierarchy, parts, cubes, pivots/offsets, rotations, texture dimensions, source metadata, and attached animation data.
- Introduce a small renderer-neutral `ViewerScene` representation derived from `ParsedModel`. It should contain named nodes/bones, cube meshes or mesh inputs, material/texture references, bind transforms, and animation clips.
- Reuse the animation data already produced by the Rust animation pipeline. The viewer should sample/interpolate those clips at runtime; it should not re-parse Java or bytecode.
- Keep Blockbench serialization as an export format, not the viewer's internal runtime format.
- Keep GPU objects, window handles, and UI state out of serialized engine IR.

---

## 3. Roadmap phases

### Phase A — Engine parity and production foundation

**Goal:** make the Rust library the trustworthy replacement for the legacy engine.

- [x] Native Rust CLI and shared library exist.
- [x] Java and supported compiled-model parsing paths exist.
- [x] Blockbench model serialization and initial animation export/preview paths exist.
- [x] Core MCreator exporters and dependency-aware `export-all` orchestration exist.
- [x] Native workspace validation and an initial Gradle compile bridge exist.
- [x] Shared compiled registration scanning works on loose class files and JAR/ZIP class entries.
- [ ] Complete source/compiled discovery parity, including unusual registry patterns, indirect factory references, and registration control flow.
- [ ] Complete compiled-only property/behavior recovery: item/tool stats and repair materials, armor properties/models, block states/drops, plant growth/placement, projectile impact behavior, and entity attributes/AI.
- [ ] Complete resource parity for raw archives: textures, animation metadata, sounds, models, blockstates, recipes, and data resources should be discoverable without requiring a manually extracted copy.
- [ ] Finish advanced world-generation conversion (configured/placed features, ores, biome generation rules, and associated MCreator procedures).
- [ ] Continue animation/texture differential testing for source and compiled inputs; preserve timing, interpolation, loop endpoints, helper calls, repeated render instances, variants, and auxiliary textures.
- [ ] Finish batch/resume, deterministic caching, durable reports, cancellation, and useful diagnostics for large mods.
- [ ] Make the Gradle bridge report actionable compiler errors and support the actual MCreator regeneration/build sequence for each supported generator.

**Rule:** do not replace missing source logic with guessed behavior. Preserve evidence and emit an explicit unsupported/partial-conversion diagnostic.

### Phase B — Viewer scene and rendering prototype

**Goal:** display an already-converted model correctly before building the full desktop shell around it.

This phase is intentionally scoped as a renderer, not a new parser.

- [x] Add `ParsedModel -> ViewerScene` conversion in a renderer-independent Rust module.
- [x] Generate initial cube faces from Minecraft/Blockbench pivot coordinates, parent transforms, inflate, mirror, and per-face UV metadata; reference-model UV verification remains open.
- [x] Resolve primary textures and texture variants using the existing texture index. Missing-texture placeholders and alpha-cutout verification remain open.
- [x] Render a textured hierarchical cube model in the initial desktop viewport (CPU rasterization prototype).
- [x] Add orbit, pan, zoom, reset-camera/initial fit-to-model, and basic face lighting. Grid, axes, and a final GPU mesh renderer remain open.
- [ ] Add a model tree with part selection, visibility toggles, and pivot display.
- [x] Add clip selection, play/pause, looping and timeline scrubbing. Playback speed and a dedicated bind-pose reset remain open.
- [x] Sample the selected clip and apply animated node transforms to the scene hierarchy each frame; static GPU mesh caching remains open.
- [ ] Verify the same model's proportions, UVs, hierarchy, and animation poses against its Blockbench export and reference screenshots.

**Viewer MVP acceptance:** load a model from the existing Rust conversion pipeline, render it with its correct texture, select/hide parts, and play/scrub at least one parsed animation clip. This must work without launching Blockbench or requiring a separate model conversion step.

### Phase C — Native desktop application shell

**Goal:** replace the current Python/Tkinter shell with a Rust desktop app that calls the Rust library directly.

- [x] Add a separate Cargo package for the desktop application while keeping the engine/CLI headless.
- [x] Build the initial application shell with persistent/collapsible navigation, content browser, viewer, and editor surfaces. Project management, settings, logs, and task progress remain open.
- [ ] Port the useful existing pages: Dashboard, Porter, Projects, Compatibility, Build & Test, and Code/Output inspection.
- [ ] Make the Porter workflow support raw JAR/ZIP, compiled class roots, decompiled source roots, and combined inputs.
- [ ] Run long conversions and Gradle builds as background jobs; expose progress, cancellation, logs, and per-stage results without freezing the UI.
- [ ] Create a versioned project/session manifest containing source roots, output workspace, selected target, recent artifacts, and project settings. Keep large original mod files in place rather than copying them into the project by default.
- [ ] Integrate the Phase B model/animation viewer as a first-class page/panel.
- [ ] Package a standalone Windows x64 app with no Python runtime or JDK requirement for parsing/conversion. A JDK may still be required by a user's selected Gradle build or generator.
- [ ] Add clean-machine install/launch tests and crash-safe project/report writes.

**Compatibility approach:** the current `app.py` / Tkinter shell remains a temporary working frontend until the Rust desktop app covers its daily workflows. Do not remove the existing shell or its packaging path before the replacement passes the acceptance gate.

### Phase D — Mod browser and integrated inspection

**Goal:** make the viewer useful on whole mods, not only on a manually selected model file.

- [ ] Browse discovered entities, renderers, models, textures, animations, items, blocks, and other ported content from a project.
- [ ] Follow the recovered entity → renderer → model links from the mod graph.
- [ ] Show model and animation provenance: source class/file, compiled class/JAR, texture candidates, selected texture, warnings, and parser confidence.
- [ ] Provide quick preview thumbnails and a searchable model/animation list.
- [ ] Compare bind pose and animation clips; support side-by-side or overlay comparison when useful.
- [ ] Allow opening the original source, generated Blockbench file, generated Java, and relevant MCreator element from the selected asset.
- [ ] Keep inspection read-only initially; previewing must never modify source files or generated workspace elements.

### Phase E — Native editing and advanced workflows

**Goal:** grow the desktop app beyond a front end without delaying the Rust engine replacement.

- [ ] Add focused editing for imported element metadata and supported MCreator fields, with schema validation and undoable changes.
- [ ] Add a source/target compatibility editor for mappings, overrides, and unsupported behavior decisions.
- [ ] Add controlled model/animation adjustments only after the viewer's scene and animation contracts are stable; preserve a clear distinction between source-derived and user-authored changes.
- [ ] Add procedure editing only as a separate milestone with its own graph model, validation, serialization, and parity strategy. Do not treat it as a small extension of the model viewer.
- [ ] Add optional launch/test automation for generated Minecraft workspaces where a reliable test environment is available.

---

## 4. Model and animation viewer — design notes

The first viewer should be a **previewer**, not a model editor.

### Main layout

The current desktop prototype follows this arrangement:

- **Far-left app rail:** retain the global Dashboard, Porter, Mod Editor, Code Studio, Compatibility, Build & Test, and Projects navigation. It remains fixed at the far left and can collapse to an icon rail.
- **Content browser:** immediately to its right, show category choices (mobs, blocks, weapons, tools, armor, items, food, models, and later discovered content). Selecting a category fills the adjacent item list.
- **Asset rows:** show a compact item/block texture thumbnail or mob spawn-egg icon, with the content name beside it and a search filter above.
- **Right detail column:** keep the model/animation viewer above the basic editor. The viewer can expand to occupy the detail column; animation selection and play/pause stay at the top of the viewport.
- **Basic editor:** below the viewer, expose texture choice, display name, lore, and Minecraft component JSON. Preview the tooltip using Minecraft formatting colors/styles before adding actual workspace write-back.
- **Later viewer panels:** hierarchical model parts/bones with visibility toggles, selected-part highlight, pivot inspector, axes/grid, source provenance and related assets.

### Runtime behavior

1. Parse/convert using the existing engine path.
2. Build a `ViewerScene` once from the canonical model representation.
3. The current prototype rasterizes the scene on the CPU and displays the resulting image in the native eframe viewport. The target renderer will cache static GPU geometry and textures.
4. On each frame, sample the selected animation at the current playback time, apply its transforms to the node hierarchy, and draw.
5. Keep animation evaluation and hierarchy math testable on the CPU. The final GPU backend should receive final transforms and draw data only.

Start with the most common cube-based entity models and their parsed animation clips. Add special cases (translucent layers, emissive/eye overlays, multiple texture variants, unusual pivots, and non-cube geometry if encountered) as explicit renderer capabilities rather than hiding them in ad hoc UI code.

### Viewer correctness checks

- Cube dimensions, face winding, normals, UV orientation, and texture resolution.
- Parent/child transform order and pivot conventions.
- Rotation units and coordinate handedness.
- Inflate/mirror behavior and repeated-render part instances.
- Alpha cutout/transparency and texture variant selection.
- Animation time units, keyframe interpolation, loop boundaries, and bind-pose restoration.
- Large model performance and GPU resource disposal when switching projects.

---

## 5. Regression and test strategy

Use the legacy implementation as an oracle, but make tests deterministic and fixture-based.

- **Model corpus:** legacy ModelBase models, modern Mojmap models, and compiled class fixtures.
- **Animation corpus:** idle/walk/head/attack and other recovered channels, including loop endpoints, helper methods, repeated renders, and compiled animation classes.
- **Texture corpus:** body variants, eye/glow overlays, shared basenames, animated PNG metadata, and missing/ambiguous references.
- **Whole-mod corpus:** OreSpawn, Lost Infinity, Twilight Forest, and Chaos Persists.
- **Workspace corpus:** representative Forge and NeoForge MCreator projects, including at least one clean Gradle build per supported generator.

Add tests at three levels:
1. Unit tests for parser/IR/transform/mesh/animation logic.
2. Integration tests for mod discovery, export-all, workspace validation, and Gradle diagnostics.
3. Differential tests against `oldVer/` output, classifying each mismatch as a parity bug, intentional difference, or unsupported behavior.

For the viewer, add CPU tests for mesh vertices/UVs, transform composition, and sampled animation poses; add a small set of deterministic screenshot/golden tests only after the rendering backend is stable.

---

## 6. Release milestones

| Milestone | Deliverable | Exit condition |
|---|---|---|
| **R1 — Rust core parity** | Rust CLI and exporters | Regression corpus passes; known unsupported cases are explicit; workspaces validate and build |
| **R2 — Viewer prototype** | Textured model viewport and animation playback | Viewer MVP acceptance in Phase B passes |
| **R3 — Desktop alpha** | Standalone Rust shell with Porter, Projects, Build & Test, and Viewer | Common workflows run through the Rust library without Python |
| **R4 — Desktop beta** | Mod browser, provenance inspector, robust background jobs, packaging | Clean-machine tests and whole-mod inspection pass |
| **R5 — Rust becomes main** | Rust app/CLI are the maintained product | All gates in section 7 pass; legacy engine remains only as a reference/test oracle |
| **R6 — Native editor** | Focused element/model editing, then procedure tooling | Separate editing milestones meet data-integrity and undo/redo requirements |

These are capability gates, not calendar promises. Viewer work can proceed alongside parity work once the shared model/animation contracts are stable.

---

## 7. Gate for making Rust the main implementation

Do not switch the default product path simply because the Rust executable builds. All of the following must be true:

- [ ] The Rust engine handles the supported input formats and target generators without invoking Python.
- [ ] The agreed whole-mod regression corpus passes the documented structural and behavior checks.
- [ ] Model geometry, animation timing/poses, texture selection, and MCreator element/resource outputs meet the agreed parity thresholds.
- [ ] Unsupported or partially converted logic is surfaced with provenance and actionable warnings.
- [ ] Exported workspaces pass native validation and clean Gradle builds for the supported generator matrix.
- [ ] Batch conversion, resume/cache, logs, and reports are reliable on large mods.
- [ ] The Rust desktop app can perform the normal Porter workflow and open the integrated viewer on a clean machine.
- [ ] Windows packaging is self-contained for parsing/conversion; any external requirement such as a JDK for Gradle is clearly surfaced.
- [ ] The CLI remains available and uses the same core library as the desktop app.
- [ ] The legacy engine is retained as a read-only oracle until the first stable Rust release has been validated.

**Promotion rule:** once these gates pass, make the Rust implementation the main/default path and move the legacy desktop shell out of the primary workflow. Keep `oldVer/` available for regression comparisons until there is a deliberate, separately reviewed archival decision.

---

## 8. Immediate next work

1. Keep closing the current compiled-registration/JAR and Gradle integration gaps; ensure the latest Windows CI run is green on the final branch head.
2. Add the renderer-neutral `ViewerScene` module and CPU tests for hierarchical transforms, cube-face UV generation, and animation sampling.
3. Prototype one textured model viewport using the selected Rust GUI/GPU stack. Prove static rendering first, then animation playback.
4. Create the desktop Cargo package around the existing CLI/library without moving conversion logic out of the core.

The key constraint is simple: **one engine, multiple front ends.** The viewer and desktop app should expose the capabilities of the Rust core, not fork or duplicate them.
