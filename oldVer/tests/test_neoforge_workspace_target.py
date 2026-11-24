import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import json
import tempfile

from mcreator_export import _default_workspace, _write_standalone_scaffold


def test_default_target_is_neoforge_1211():
    ws = _default_workspace("examplemod", "Example Mod", "com.example.examplemod")
    assert ws["workspaceSettings"]["currentGenerator"] == "neoforge-1.21.1"


def test_neoforge_scaffold_uses_java21_and_neo_211228():
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        _write_standalone_scaffold(root, "examplemod", "neoforge-1.21.1")
        build = (root / "build.gradle").read_text()
        assert "net.neoforged.moddev" in build
        assert "21.1.228" in build
        assert "JavaLanguageVersion.of(21)" in build
        props = (root / "gradle.properties").read_text()
        assert "org.gradle.jvmargs" in props


def test_legacy_forge_scaffold_still_available():
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        _write_standalone_scaffold(root, "examplemod", "forge-1.20.1")
        build = (root / "build.gradle").read_text()
        assert "legacyForge" in build
        assert "1.20.1-47.4.22" in build
