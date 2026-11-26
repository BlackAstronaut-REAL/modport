from __future__ import annotations

import hashlib
import importlib
import json
import os
import queue
import shutil
import subprocess
import sys
import threading
import traceback
import zipfile
from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime
from pathlib import Path
from tkinter import filedialog, messagebox, ttk
import tkinter as tk

APP_NAME = "MojModelBridge"
APP_VERSION = "0.1.0"


def app_root() -> Path:
    if getattr(sys, "frozen", False):
        return Path(getattr(sys, "_MEIPASS", Path(sys.executable).parent))
    return Path(__file__).resolve().parent


def user_data_dir() -> Path:
    if os.name == "nt":
        base = Path(os.environ.get("APPDATA", Path.home() / "AppData" / "Roaming"))
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    path = base / APP_NAME
    path.mkdir(parents=True, exist_ok=True)
    return path


class QueueWriter:
    def __init__(self, target: queue.Queue[str]):
        self.target = target

    def write(self, value: str) -> int:
        if value:
            self.target.put(value)
        return len(value)

    def flush(self) -> None:
        pass


class MojModelBridgeApp(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title(f"{APP_NAME} · Desktop {APP_VERSION}")
        self.geometry("1280x820")
        self.minsize(1040, 700)
        self.configure(bg="#10141d")

        self.root_dir = app_root()
        self.engine_dir = self.root_dir / "oldVer"
        self.config_path = user_data_dir() / "desktop.json"
        self.recent_projects = self._load_recent_projects()
        self.log_queue: queue.Queue[str] = queue.Queue()
        self.worker: threading.Thread | None = None
        self.port_running = False
        self.engine = None
        self._load_engine()
        self._configure_styles()
        self._build_shell()
        self._show_page("Dashboard")
        self.after(100, self._drain_log_queue)

    def _load_engine(self) -> None:
        if not (self.engine_dir / "model_bridge.py").is_file():
            return
        sys.path.insert(0, str(self.engine_dir))
        try:
            self.engine = importlib.import_module("model_bridge")
        except Exception:
            self.engine = None
            self.engine_load_error = traceback.format_exc()

    def _load_recent_projects(self) -> list[dict[str, str]]:
        try:
            data = json.loads(self.config_path.read_text(encoding="utf-8"))
            return [v for v in data.get("recent_projects", []) if isinstance(v, dict) and v.get("path")]
        except (OSError, ValueError, AttributeError):
            return []

    def _save_recent_projects(self) -> None:
        self.config_path.write_text(json.dumps({"recent_projects": self.recent_projects[:20]}, indent=2), encoding="utf-8")

    def _configure_styles(self) -> None:
        style = ttk.Style(self)
        try:
            style.theme_use("clam")
        except tk.TclError:
            pass
        style.configure("TFrame", background="#151b27")
        style.configure("Panel.TFrame", background="#1b2433")
        style.configure("TLabel", background="#151b27", foreground="#e8edf5", font=("Segoe UI", 10))
        style.configure("Muted.TLabel", background="#151b27", foreground="#9aa8bc", font=("Segoe UI", 9))
        style.configure("Panel.TLabel", background="#1b2433", foreground="#e8edf5", font=("Segoe UI", 10))
        style.configure("PanelMuted.TLabel", background="#1b2433", foreground="#9aa8bc", font=("Segoe UI", 9))
        style.configure("Title.TLabel", background="#151b27", foreground="#f4f7fb", font=("Segoe UI Semibold", 22))
        style.configure("Heading.TLabel", background="#1b2433", foreground="#f4f7fb", font=("Segoe UI Semibold", 12))
        style.configure("TButton", font=("Segoe UI Semibold", 9), padding=(12, 8), background="#273449", foreground="#f4f7fb")
        style.map("TButton", background=[("active", "#354761"), ("disabled", "#222a36")])
        style.configure("Accent.TButton", background="#3978d6", foreground="#ffffff")
        style.map("Accent.TButton", background=[("active", "#4b8be9"), ("disabled", "#294c7c")])
        style.configure("Nav.TButton", anchor="w", padding=(14, 11), background="#111722", foreground="#c5d0df")
        style.map("Nav.TButton", background=[("active", "#24334a")], foreground=[("active", "#ffffff")])
        style.configure("TEntry", fieldbackground="#101722", foreground="#f4f7fb", insertcolor="#ffffff", padding=7)
        style.configure("TCombobox", fieldbackground="#101722", foreground="#f4f7fb", padding=6)
        style.configure("TCheckbutton", background="#1b2433", foreground="#dce5f0")
        style.configure("Treeview", background="#101722", fieldbackground="#101722", foreground="#e8edf5", rowheight=28)
        style.configure("Treeview.Heading", background="#273449", foreground="#ffffff", font=("Segoe UI Semibold", 9))

    def _build_shell(self) -> None:
        shell = tk.Frame(self, bg="#10141d")
        shell.pack(fill="both", expand=True)
        sidebar = tk.Frame(shell, bg="#111722", width=222)
        sidebar.pack(side="left", fill="y")
        sidebar.pack_propagate(False)

        brand = tk.Frame(sidebar, bg="#111722", padx=18, pady=22)
        brand.pack(fill="x")
        tk.Label(brand, text="MOJMODEL", bg="#111722", fg="#ffffff", font=("Segoe UI Semibold", 17)).pack(anchor="w")
        tk.Label(brand, text="BRIDGE", bg="#111722", fg="#62a2ff", font=("Segoe UI Semibold", 17)).pack(anchor="w")
        tk.Label(brand, text=f"DESKTOP  {APP_VERSION}", bg="#111722", fg="#7e8da4", font=("Segoe UI", 8)).pack(anchor="w", pady=(8, 0))

        nav = tk.Frame(sidebar, bg="#111722")
        nav.pack(fill="x", padx=9, pady=(18, 0))
        self.nav_buttons = {}
        for label in ("Dashboard", "Porter", "Mod Editor", "Code Studio", "Compatibility", "Build & Test", "Projects"):
            btn = ttk.Button(nav, text=label, style="Nav.TButton", command=lambda page=label: self._show_page(page))
            btn.pack(fill="x", pady=2)
            self.nav_buttons[label] = btn

        bottom = tk.Frame(sidebar, bg="#111722", padx=18, pady=16)
        bottom.pack(side="bottom", fill="x")
        status = "Engine loaded" if self.engine else "Engine not found"
        color = "#72d49a" if self.engine else "#f0b46d"
        tk.Label(bottom, text="●  " + status, bg="#111722", fg=color, font=("Segoe UI", 9)).pack(anchor="w")
        tk.Label(bottom, text="Legacy engine is kept intact", bg="#111722", fg="#7e8da4", font=("Segoe UI", 8)).pack(anchor="w", pady=(5, 0))

        self.content = tk.Frame(shell, bg="#151b27")
        self.content.pack(side="left", fill="both", expand=True)
        self.pages = {}
        builders = (
            ("Dashboard", self._build_dashboard), ("Porter", self._build_porter),
            ("Mod Editor", self._build_mod_editor), ("Code Studio", self._build_code_studio),
            ("Compatibility", self._build_compatibility), ("Build & Test", self._build_build_test),
            ("Projects", self._build_projects),
        )
        for name, builder in builders:
            frame = tk.Frame(self.content, bg="#151b27")
            self.pages[name] = frame
            builder(frame)

    def _page_header(self, parent: tk.Widget, title: str, subtitle: str) -> None:
        header = tk.Frame(parent, bg="#151b27")
        header.pack(fill="x", padx=30, pady=(28, 20))
        ttk.Label(header, text=title, style="Title.TLabel").pack(anchor="w")
        ttk.Label(header, text=subtitle, style="Muted.TLabel").pack(anchor="w", pady=(5, 0))

    def _panel(self, parent: tk.Widget, title: str, subtitle: str | None = None) -> tk.Frame:
        outer = tk.Frame(parent, bg="#1b2433", padx=18, pady=16, highlightthickness=1, highlightbackground="#29364a")
        if title:
            ttk.Label(outer, text=title, style="Heading.TLabel").pack(anchor="w")
        if subtitle:
            ttk.Label(outer, text=subtitle, style="PanelMuted.TLabel", wraplength=850).pack(anchor="w", pady=(5, 12))
        return outer

    def _build_dashboard(self, page: tk.Frame) -> None:
        self._page_header(page, "Desktop workspace", "A desktop front end for the proven MojModelBridge 3.4 engine.")
        body = tk.Frame(page, bg="#151b27")
        body.pack(fill="both", expand=True, padx=30, pady=(0, 26))
        hero = self._panel(body, "Port first. Improve the engine later.", "This baseline wraps the existing conversion pipeline. The original model, mob geometry, animation, texture, and MCreator conversion code under oldVer is not rewritten by this desktop shell.")
        hero.pack(fill="x", pady=(0, 14))
        ttk.Button(hero, text="Open Porter", style="Accent.TButton", command=lambda: self._show_page("Porter")).pack(anchor="w", pady=(4, 0))
        cards = tk.Frame(body, bg="#151b27")
        cards.pack(fill="x")
        entries = (
            ("01", "Porter", "Import a raw JAR/ZIP, combined source, or compiled and decompiled roots."),
            ("02", "Editable output", "Generate Blockbench models and an MCreator workspace with the original exporter."),
            ("03", "Next layers", "Add editing, compatibility adapters, and automated testing without replacing the engine."),
        )
        for i, (number, title, desc) in enumerate(entries):
            card = self._panel(cards, title, desc)
            card.grid(row=0, column=i, sticky="nsew", padx=(0 if i == 0 else 8, 0))
            tk.Label(card, text=number, bg="#1b2433", fg="#62a2ff", font=("Segoe UI Semibold", 18)).pack(anchor="w", pady=(10, 0))
            cards.grid_columnconfigure(i, weight=1)
        if not self.engine:
            warning = self._panel(body, "Engine files not found", "The app expects oldVer/ beside the source or bundled inside the EXE. Build from the repository root so PyInstaller includes it.")
            warning.pack(fill="x", pady=(16, 0))

    def _field(self, parent: tk.Widget, label: str, variable: tk.StringVar, browse_kind: str, row: int) -> None:
        ttk.Label(parent, text=label, style="Panel.TLabel").grid(row=row, column=0, sticky="w", pady=5)
        ttk.Entry(parent, textvariable=variable).grid(row=row, column=1, sticky="ew", padx=(12, 8), pady=5)
        ttk.Button(parent, text="Browse…", command=lambda: self._browse(variable, browse_kind)).grid(row=row, column=2, sticky="ew", pady=5)

    def _browse(self, variable: tk.StringVar, kind: str) -> None:
        if kind == "archive":
            value = filedialog.askopenfilename(title="Select a mod JAR or ZIP", filetypes=[("Mod archives", "*.jar *.zip"), ("All files", "*.*")])
        else:
            value = filedialog.askdirectory(title="Select " + kind)
        if value:
            variable.set(value)

    def _extract_archive(self, archive_path: Path) -> Path:
        """Safely unpack a JAR/ZIP to the per-user cache for the original folder-based engine."""
        archive_path = archive_path.expanduser().resolve()
        if not archive_path.is_file():
            raise FileNotFoundError(archive_path)
        stat = archive_path.stat()
        identity = f"{archive_path.name}|{stat.st_size}|{stat.st_mtime_ns}"
        digest = hashlib.sha256(identity.encode("utf-8")).hexdigest()[:20]
        cache_root = user_data_dir() / "cache" / "archives"
        destination = cache_root / digest
        if (destination / ".extracted-ok").is_file():
            return destination

        cache_root.mkdir(parents=True, exist_ok=True)
        staging = cache_root / (digest + ".partial")
        if staging.exists():
            shutil.rmtree(staging)
        staging.mkdir(parents=True)
        total_size = 0
        try:
            with zipfile.ZipFile(archive_path) as archive:
                for member in archive.infolist():
                    target = (staging / member.filename).resolve()
                    if target != staging.resolve() and staging.resolve() not in target.parents:
                        raise ValueError(f"Archive contains an unsafe path: {member.filename}")
                    total_size += member.file_size
                    if total_size > 8 * 1024 * 1024 * 1024:
                        raise ValueError("Archive expands beyond the 8 GiB safety limit.")
                archive.extractall(staging)
            (staging / ".extracted-ok").write_text(str(archive_path), encoding="utf-8")
            if destination.exists():
                shutil.rmtree(destination)
            staging.replace(destination)
            return destination
        except Exception:
            shutil.rmtree(staging, ignore_errors=True)
            raise

    def _build_porter(self, page: tk.Frame) -> None:
        self._page_header(page, "Porter", "Run the existing full conversion pipeline. Porting internals remain unchanged in this baseline.")
        body = tk.Frame(page, bg="#151b27")
        body.pack(fill="both", expand=True, padx=30, pady=(0, 24))
        self.archive_var = tk.StringVar()
        self.mod_root_var = tk.StringVar()
        self.compiled_root_var = tk.StringVar()
        self.decompiled_root_var = tk.StringVar()
        self.output_root_var = tk.StringVar(value=str(Path.home() / "MojModelBridge Projects" / "New Port"))
        self.template_var = tk.StringVar()
        self.author_var = tk.StringVar()
        self.generator_var = tk.StringVar(value="neoforge-1.21.1")

        sources = self._panel(body, "Source inputs", "Use one raw archive or a combined folder, or provide separate compiled and decompiled folders. The old engine merges representations and avoids duplicate discovery.")
        sources.pack(fill="x", pady=(0, 12))
        source_form = tk.Frame(sources, bg="#1b2433")
        source_form.pack(fill="x", pady=(2, 0))
        source_form.columnconfigure(1, weight=1)
        self._field(source_form, "Mod JAR / ZIP archive", self.archive_var, "archive", 0)
        self._field(source_form, "Combined mod root", self.mod_root_var, "combined mod root", 1)
        self._field(source_form, "Compiled classes root", self.compiled_root_var, "compiled classes root", 2)
        self._field(source_form, "Decompiled Java/resources root", self.decompiled_root_var, "decompiled source root", 3)
        ttk.Label(source_form, text="Archives are safely extracted to the app cache before conversion. Use either an archive or a combined folder, plus optional compiled/decompiled roots.", style="PanelMuted.TLabel", wraplength=880).grid(row=4, column=0, columnspan=3, sticky="w", pady=(5, 0))

        outputs = self._panel(body, "Output and generator", "The output folder will contain Blockbench/ and MCreatorWorkspace/ subfolders.")
        outputs.pack(fill="x", pady=(0, 12))
        output_form = tk.Frame(outputs, bg="#1b2433")
        output_form.pack(fill="x", pady=(2, 0))
        output_form.columnconfigure(1, weight=1)
        self._field(output_form, "Output project folder", self.output_root_var, "output folder", 0)
        self._field(output_form, "Optional MCreator template", self.template_var, "MCreator template workspace", 1)
        ttk.Label(output_form, text="Generator", style="Panel.TLabel").grid(row=2, column=0, sticky="w", pady=5)
        ttk.Combobox(output_form, textvariable=self.generator_var, values=("neoforge-1.21.1", "forge-1.20.1"), state="readonly", width=24).grid(row=2, column=1, sticky="w", padx=(12, 8), pady=5)
        ttk.Label(output_form, text="Author override (optional)", style="Panel.TLabel").grid(row=3, column=0, sticky="w", pady=5)
        ttk.Entry(output_form, textvariable=self.author_var).grid(row=3, column=1, sticky="ew", padx=(12, 8), pady=5)

        actions = tk.Frame(body, bg="#151b27")
        actions.pack(fill="x", pady=(0, 10))
        self.port_button = ttk.Button(actions, text="Start full port", style="Accent.TButton", command=self._start_port)
        self.port_button.pack(side="left")
        ttk.Button(actions, text="Open output folder", command=lambda: self._open_path(self.output_root_var.get())).pack(side="left", padx=8)
        self.port_status = ttk.Label(actions, text="Ready", style="Muted.TLabel")
        self.port_status.pack(side="right")

        log_panel = self._panel(body, "Engine log", "Live output from the original conversion engine.")
        log_panel.pack(fill="both", expand=True)
        log_holder = tk.Frame(log_panel, bg="#101722")
        log_holder.pack(fill="both", expand=True, pady=(8, 0))
        self.port_log = tk.Text(log_holder, bg="#0d131d", fg="#cbd6e4", insertbackground="#ffffff", relief="flat", wrap="word", font=("Consolas", 9), padx=10, pady=10)
        scroll = ttk.Scrollbar(log_holder, orient="vertical", command=self.port_log.yview)
        self.port_log.configure(yscrollcommand=scroll.set)
        self.port_log.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")
        if not self.engine:
            self._append_log("ERROR: oldVer/model_bridge.py could not be loaded. The engine folder is required.\n")

    def _start_port(self) -> None:
        if self.port_running:
            return
        if not self.engine:
            messagebox.showerror("Engine unavailable", "Could not load oldVer/model_bridge.py. Rebuild from the repository root or restore oldVer.")
            return
        archive = self.archive_var.get().strip()
        combined = self.mod_root_var.get().strip()
        compiled = self.compiled_root_var.get().strip()
        decompiled = self.decompiled_root_var.get().strip()
        if archive and combined:
            messagebox.showwarning("Choose one combined source", "Use either a JAR/ZIP archive or a combined mod root, not both.")
            return
        if not any((archive, combined, compiled, decompiled)):
            messagebox.showwarning("Missing source", "Choose an archive, combined root, compiled root, or decompiled root.")
            return
        if not self.output_root_var.get().strip():
            messagebox.showwarning("Missing output", "Choose an output folder.")
            return
        out_root = Path(self.output_root_var.get()).expanduser()
        if archive:
            try:
                combined = str(self._extract_archive(Path(archive)))
                self._append_log(f"Archive extracted to cache: {combined}\n")
            except Exception as exc:
                messagebox.showerror("Archive extraction failed", str(exc))
                return

        args = ["--full-port", "--output", str(out_root / "Blockbench"), "--mcreator-output", str(out_root / "MCreatorWorkspace"), "--model-type", "auto", "--mcreator-generator", self.generator_var.get()]
        if combined:
            args.extend(["--mod-root", combined])
        if compiled:
            args.extend(["--compiled-root", compiled])
        if decompiled:
            args.extend(["--decompiled-root", decompiled])
        if self.template_var.get().strip():
            args.extend(["--mcreator-template", self.template_var.get().strip()])
        if self.author_var.get().strip():
            args.extend(["--mod-author", self.author_var.get().strip()])

        self.port_running = True
        self.last_output_root = out_root
        self.port_button.configure(state="disabled")
        self.port_status.configure(text="Port running…")
        self._append_log(f"\n=== MojModelBridge Desktop {APP_VERSION} ===\n")
        self._append_log("Existing engine call: model_bridge.main(" + " ".join(args) + ")\n")
        self.worker = threading.Thread(target=self._run_engine, args=(args,), daemon=True)
        self.worker.start()

    def _run_engine(self, args: list[str]) -> None:
        status = 0
        try:
            writer = QueueWriter(self.log_queue)
            with redirect_stdout(writer), redirect_stderr(writer):
                status = int(self.engine.main(args) or 0)
        except BaseException:
            self.log_queue.put("\n" + traceback.format_exc() + "\n")
            status = 1
        self.log_queue.put(f"\n__MOJMODELBRIDGE_DONE__:{status}\n")

    def _append_log(self, value: str) -> None:
        for name in ("port_log", "build_log"):
            widget = getattr(self, name, None)
            if widget is not None:
                widget.insert("end", value)
                widget.see("end")

    def _drain_log_queue(self) -> None:
        try:
            while True:
                value = self.log_queue.get_nowait()
                gradle_marker = "__GRADLE_DONE__:"
                if gradle_marker in value:
                    try:
                        build_code = int(value.split(gradle_marker, 1)[1].strip().splitlines()[0])
                    except (ValueError, IndexError):
                        build_code = 1
                    if hasattr(self, "build_status"):
                        self.build_status.configure(text="Gradle build succeeded." if build_code == 0 else f"Gradle build failed (exit code {build_code}).")
                    self._append_log(value.replace(gradle_marker + str(build_code), "Gradle exit code: " + str(build_code)))
                else:
                    marker = "__MOJMODELBRIDGE_DONE__:"
                    if marker in value:
                        try:
                            code = int(value.split(marker, 1)[1].strip().splitlines()[0])
                        except (ValueError, IndexError):
                            code = 1
                        self.port_running = False
                        if hasattr(self, "port_button"):
                            self.port_button.configure(state="normal")
                        if hasattr(self, "port_status"):
                            self.port_status.configure(text="Finished" if code == 0 else f"Finished with exit code {code}")
                        self._append_log("\nPort process finished. Review the generated reports and test the workspace in its target environment.\n")
                        if code == 0 and hasattr(self, "last_output_root") and self.last_output_root.exists():
                            project = {"path": str(self.last_output_root), "updated": datetime.now().strftime("%Y-%m-%d %H:%M")}
                            self.recent_projects = [p for p in self.recent_projects if p.get("path") != project["path"]]
                            self.recent_projects.insert(0, project)
                            self._save_recent_projects()
                            self._refresh_projects()
                    else:
                        self._append_log(value)
        except queue.Empty:
            pass
        self.after(100, self._drain_log_queue)

    def _build_mod_editor(self, page: tk.Frame) -> None:
        self._page_header(page, "Mod Editor", "The next layer: edit imported content and create new elements in the same project model.")
        body = tk.Frame(page, bg="#151b27")
        body.pack(fill="both", expand=True, padx=30, pady=(0, 26))
        panel = self._panel(body, "Editor foundation", "This baseline does not alter the conversion engine or invent an incomplete editor schema. Existing MCreator workspace output remains editable in MCreator. The in-app editor can be added against a normalized project representation in the next phase.")
        panel.pack(fill="x")
        ttk.Button(panel, text="Open latest MCreator workspace", command=self._open_latest_workspace).pack(anchor="w", pady=(5, 0))

    def _build_code_studio(self, page: tk.Frame) -> None:
        self._page_header(page, "Code Studio", "Direct access to generated Java, resources, and MCreator procedure files.")
        body = tk.Frame(page, bg="#151b27")
        body.pack(fill="both", expand=True, padx=30, pady=(0, 26))
        panel = self._panel(body, "Generated source", "Open generated project and engine source folders in your preferred editor. The original backend under oldVer is treated as the stable engine baseline.")
        panel.pack(fill="x")
        ttk.Button(panel, text="Open latest project output", command=self._open_latest_project).pack(anchor="w", pady=(5, 0))
        ttk.Button(panel, text="Open engine source folder", command=lambda: self._open_path(self.engine_dir)).pack(anchor="w", pady=(8, 0))

    def _build_compatibility(self, page: tk.Frame) -> None:
        self._page_header(page, "Compatibility", "Target selection currently mirrors the generators supported by the legacy exporter.")
        body = tk.Frame(page, bg="#151b27")
        body.pack(fill="both", expand=True, padx=30, pady=(0, 26))
        panel = self._panel(body, "Available output targets", "This baseline is not yet a universal cross-loader translator. Source and target adapters can be expanded later without changing oldVer.")
        panel.pack(fill="x")
        for name, detail in (("NeoForge 1.21.1", "Default standalone MCreator export target."), ("Forge 1.20.1", "Legacy MCreator generator option.")):
            row = tk.Frame(panel, bg="#1b2433")
            row.pack(fill="x", pady=8)
            tk.Label(row, text="●", bg="#1b2433", fg="#72d49a", font=("Segoe UI", 12)).pack(side="left", padx=(0, 10))
            tk.Label(row, text=name, bg="#1b2433", fg="#f4f7fb", font=("Segoe UI Semibold", 10)).pack(side="left")
            tk.Label(row, text=detail, bg="#1b2433", fg="#9aa8bc", font=("Segoe UI", 9)).pack(side="left", padx=12)

    def _build_build_test(self, page: tk.Frame) -> None:
        self._page_header(page, "Build & Test", "Inspect generated outputs and run a Gradle build when the exported workspace includes its wrapper.")
        body = tk.Frame(page, bg="#151b27")
        body.pack(fill="both", expand=True, padx=30, pady=(0, 26))
        panel = self._panel(body, "Workspace build", "A successful conversion is not a substitute for compiling and launching the generated mod. This is the first build hand-off; automated runtime tests will be expanded later.")
        panel.pack(fill="x")
        ttk.Button(panel, text="Open latest MCreator workspace", command=self._open_latest_workspace).pack(anchor="w", pady=(5, 0))
        ttk.Button(panel, text="Run Gradle build", command=self._build_latest_workspace).pack(anchor="w", pady=(8, 0))
        self.build_status = ttk.Label(panel, text="No build started.", style="PanelMuted.TLabel")
        self.build_status.pack(anchor="w", pady=(12, 0))
        log_panel = self._panel(body, "Build output", "Live Gradle output from the generated workspace.")
        log_panel.pack(fill="both", expand=True, pady=(12, 0))
        log_holder = tk.Frame(log_panel, bg="#101722")
        log_holder.pack(fill="both", expand=True, pady=(8, 0))
        self.build_log = tk.Text(log_holder, bg="#0d131d", fg="#cbd6e4", insertbackground="#ffffff", relief="flat", wrap="word", font=("Consolas", 9), padx=10, pady=10)
        scroll = ttk.Scrollbar(log_holder, orient="vertical", command=self.build_log.yview)
        self.build_log.configure(yscrollcommand=scroll.set)
        self.build_log.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")

    def _build_projects(self, page: tk.Frame) -> None:
        self._page_header(page, "Projects", "Keep output folders organized and reopen previous ports.")
        body = tk.Frame(page, bg="#151b27")
        body.pack(fill="both", expand=True, padx=30, pady=(0, 26))
        panel = self._panel(body, "Recent projects", "Projects are folders on disk; recent paths are stored in your user application-data directory.")
        panel.pack(fill="both", expand=True)
        toolbar = tk.Frame(panel, bg="#1b2433")
        toolbar.pack(fill="x", pady=(10, 8))
        ttk.Button(toolbar, text="Add existing folder", command=self._add_project).pack(side="left")
        ttk.Button(toolbar, text="Open selected", command=self._open_selected_project).pack(side="left", padx=8)
        self.projects_tree = ttk.Treeview(panel, columns=("path", "updated"), show="headings", height=12)
        self.projects_tree.heading("path", text="Project folder")
        self.projects_tree.heading("updated", text="Last opened")
        self.projects_tree.column("path", width=650)
        self.projects_tree.column("updated", width=180)
        self.projects_tree.pack(fill="both", expand=True)
        self._refresh_projects()

    def _show_page(self, name: str) -> None:
        for frame in self.pages.values():
            frame.pack_forget()
        self.pages[name].pack(fill="both", expand=True)
        for label, button in self.nav_buttons.items():
            button.configure(style="Accent.TButton" if label == name else "Nav.TButton")

    def _recent_path(self) -> Path | None:
        for item in self.recent_projects:
            path = Path(item.get("path", ""))
            if path.exists():
                return path
        return None

    def _open_latest_project(self) -> None:
        path = self._recent_path()
        if path is None:
            messagebox.showinfo("No project", "Add an existing project folder or run a port first.")
        else:
            self._open_path(path)

    def _open_latest_workspace(self) -> None:
        path = self._recent_path()
        workspace = path / "MCreatorWorkspace" if path else None
        if workspace is None or not workspace.exists():
            messagebox.showinfo("No workspace", "No recent MCreatorWorkspace folder was found.")
        else:
            self._open_path(workspace)

    def _open_path(self, value: str | Path) -> None:
        path = Path(value).expanduser()
        if not path.exists():
            messagebox.showwarning("Folder not found", f"This path does not exist:\n{path}")
            return
        try:
            if os.name == "nt":
                os.startfile(str(path))  # type: ignore[attr-defined]
            elif sys.platform == "darwin":
                subprocess.Popen(["open", str(path)])
            else:
                subprocess.Popen(["xdg-open", str(path)])
        except Exception as exc:
            messagebox.showerror("Could not open folder", str(exc))

    def _add_project(self) -> None:
        value = filedialog.askdirectory(title="Select an existing MojModelBridge output/project folder")
        if not value:
            return
        path = Path(value)
        record = {"path": str(path), "updated": datetime.now().strftime("%Y-%m-%d %H:%M")}
        self.recent_projects = [p for p in self.recent_projects if p.get("path") != str(path)]
        self.recent_projects.insert(0, record)
        self._save_recent_projects()
        self._refresh_projects()

    def _refresh_projects(self) -> None:
        if not hasattr(self, "projects_tree"):
            return
        for item in self.projects_tree.get_children():
            self.projects_tree.delete(item)
        for project in self.recent_projects:
            self.projects_tree.insert("", "end", values=(project.get("path", ""), project.get("updated", "")))

    def _open_selected_project(self) -> None:
        selection = self.projects_tree.selection()
        if not selection:
            messagebox.showinfo("Select project", "Select a project folder first.")
            return
        self._open_path(self.projects_tree.item(selection[0], "values")[0])

    def _build_latest_workspace(self) -> None:
        path = self._recent_path()
        workspace = path / "MCreatorWorkspace" if path else None
        if workspace is None or not workspace.exists():
            messagebox.showwarning("No workspace", "No recent MCreatorWorkspace folder was found.")
            return
        wrapper = workspace / ("gradlew.bat" if os.name == "nt" else "gradlew")
        if not wrapper.is_file():
            messagebox.showinfo("Gradle wrapper missing", "This workspace does not contain gradlew/gradlew.bat. Open it in MCreator to let the generator create its build files first.")
            return
        if self.worker and self.worker.is_alive():
            messagebox.showwarning("Busy", "Another task is already running.")
            return
        self.build_status.configure(text="Gradle build running…")
        self.worker = threading.Thread(target=self._run_gradle, args=(workspace, wrapper), daemon=True)
        self.worker.start()

    def _run_gradle(self, workspace: Path, wrapper: Path) -> None:
        try:
            command = [str(wrapper), "build"]
            if os.name == "nt":
                command = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/s", "/c", f'"{wrapper}" build']
            proc = subprocess.Popen(command, cwd=workspace, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
            assert proc.stdout is not None
            for line in proc.stdout:
                self.log_queue.put(line)
            code = proc.wait()
            self.log_queue.put(f"\n__GRADLE_DONE__:{code}\n")
        except Exception:
            self.log_queue.put(traceback.format_exc())
            self.log_queue.put("\n__GRADLE_DONE__:1\n")

    def run(self) -> None:
        self.mainloop()


if __name__ == "__main__":
    MojModelBridgeApp().run()
