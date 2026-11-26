# PyInstaller one-file Windows executable spec.
from pathlib import Path

root = Path(SPECPATH)
engine = root / "oldVer"

# Keep the original engine as Python source data so the app imports those same
# modules from the extracted one-file payload at runtime.
datas = []
for file in engine.glob("*.py"):
    datas.append((str(file), "oldVer"))
for file in (engine / "mcreator_templates").rglob("*"):
    if file.is_file():
        destination = Path("oldVer") / "mcreator_templates" / file.relative_to(engine / "mcreator_templates").parent
        datas.append((str(file), str(destination)))

a = Analysis(
    [str(root / "app.py")],
    pathex=[str(root)],
    binaries=[],
    datas=datas,
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=["pytest"],
    noarchive=False,
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="MojModelBridge",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    console=False,
    disable_windowed_traceback=False,
)
