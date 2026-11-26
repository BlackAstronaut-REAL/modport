# MojModelBridge Native Desktop

This is the standalone Rust desktop application. It depends on the shared mojmodelbridge library and runs without a Python engine.

## Build and run

From the repository root:

~~~powershell
cargo run --manifest-path rust/desktop/Cargo.toml
~~~

Build a release executable:

~~~powershell
cargo build --release --manifest-path rust/desktop/Cargo.toml
~~~

The Windows executable is written to `rust/desktop/target/release/mojmodelbridge-desktop.exe`.

## Integrated workflows

- **Porter:** choose multiple directories, JAR/ZIP archives, Java source files, and compiled class files; set target metadata and output location; run the native Rust NeoForge conversion; optionally compile the result immediately.
- **Build & Test:** select an existing generated Gradle project, choose tasks, toggle offline mode, run the build, and inspect diagnostics plus retained Gradle logs/reports.
- **Project Converter / Projects:** import and save source roots for browsing and reuse.
- **Content Browser / Viewer:** inspect recovered registered assets, textures, models, and animations.

Porting and Gradle operations run on a background thread and report their progress/results in the UI. The desktop calls the same `port_to_neoforge` and `compile_workspace` Rust library functions as the CLI; it does not invoke a Python engine or shell out to the CLI executable.

The converter is intentionally transparent about unsupported behavior. Custom AI, event handlers, networking, capabilities, custom rendering, and many special abilities are still pending adapters and appear in the conversion analysis/warnings.
