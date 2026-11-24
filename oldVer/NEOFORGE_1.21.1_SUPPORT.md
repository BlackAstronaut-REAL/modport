# NeoForge 1.21.1 MCreator support

MojModelBridge now emits standalone MCreator workspaces targeting `neoforge-1.21.1`.

Validated against the public MCreator generator structure for NeoForge 1.21.1: the generator uses the `mojmap-1.21.x` Java-model source key and current MCreator 2026.2 uses NeoForge 21.1.228 for the 1.21.1 target.

The standalone Gradle scaffold uses Java 21, ModDevGradle 2.0.107, and NeoForge 21.1.228. MCreator remains responsible for regenerating its own generator-managed Gradle files when the workspace is opened.

CLI:

`python model_bridge.py --full-port --mod-root <MODROOT> --mcreator-output <WORKSPACE>`

Optional legacy target:

`python model_bridge.py --full-port --mcreator-generator forge-1.20.1 ...`
