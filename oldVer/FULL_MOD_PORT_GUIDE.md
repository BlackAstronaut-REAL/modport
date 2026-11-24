# Full Mod → MCreator Port Guide (v3.4 Beta 2)

## Best input: separate compiled and decompiled trees

```bat
full_port_combined.bat "D:\Mods\MyMod\Compiled" "D:\Mods\MyMod\Decompiled" "D:\Converted\MyMod"
```

With an existing workspace used as a container/reference:

```bat
full_port_combined.bat "D:\Mods\MyMod\Compiled" "D:\Mods\MyMod\Decompiled" "D:\MCreator\Template" "D:\Converted\MyMod"
```

The two inputs are merged into one conversion. Bytecode and source are complementary; they are not exported as separate mods.

## Combined folder

```bat
full_port.bat "D:\Mods\MyMod" "D:\Converted\MyMod"
```

Or:

```bat
full_port.bat "D:\Mods\MyMod" "D:\MCreator\Template" "D:\Converted\MyMod"
```

Existing template elements are left untouched. Missing source elements are created. The template is never treated as the source of truth.

## Direct CLI

```bat
python model_bridge.py --full-port ^
  --compiled-root "D:\Mods\MyMod\Compiled" ^
  --decompiled-root "D:\Mods\MyMod\Decompiled" ^
  --output "D:\Converted\MyMod\Blockbench" ^
  --mcreator-output "D:\Converted\MyMod\MCreatorWorkspace" ^
  --model-type auto
```

## Resume

```bat
python model_bridge.py ... --no-overwrite
```

Completed `.bbmodel` files and MCreator checkpoints are reused.

## Source modes

```text
auto   Pair equivalent compiled/decompiled models and emit each once.
both   Analyze both representations deliberately.
java   Decompiled/source input only.
class  Compiled bytecode only.
```

The content pipeline still uses all configured roots for registry/resource metadata; the source mode primarily controls model conversion.

## What to review before building

1. `MOJMODELBRIDGE_WORKSPACE_VALIDATION.json`
2. `mcreator_export_report.html`
3. Missing/placeholder texture warnings
4. Inferred/default equipment values
5. Unresolved animation/state warnings
6. Tool 3D Model assignments and custom item model files

See `TESTING_GUIDE_V3.4_BETA2.md` for the desktop test sequence.
