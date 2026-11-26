use crate::browser::{self, BrowserEntry, BrowserIndex, IconKind};
use crate::renderer::{self, Camera};
use crate::tooltip;
use crate::project::{self, ProjectSources};
use eframe::egui::{self, Color32, FontFamily, RichText, TextureHandle, TextureOptions};
use image::RgbaImage;
use mojmodelbridge::pipeline::load_model_preview;
use mojmodelbridge::viewer::ViewerScene;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, mpsc::Receiver, Arc};
use std::thread;
use std::time::{Duration, Instant};

use mojmodelbridge::gradle_bridge::{compile_workspace, GradleCompileReport};
use mojmodelbridge::neoforge_port::{port_to_neoforge, NeoForgePortReport};
use mojmodelbridge::neoforge_project::DEFAULT_NEOFORGE_VERSION;

const BACKGROUND: Color32 = Color32::from_rgb(16, 20, 29);
const SURFACE: Color32 = Color32::from_rgb(25, 32, 45);
const SURFACE_LIGHT: Color32 = Color32::from_rgb(34, 44, 61);
const TEXT: Color32 = Color32::from_rgb(232, 237, 245);
const MUTED: Color32 = Color32::from_rgb(145, 158, 179);
const ACCENT: Color32 = Color32::from_rgb(84, 151, 242);

#[derive(Debug, Clone)]
struct AnimatedTextureFrame {
    index: u32,
    duration_seconds: f32,
}

#[derive(Debug, Clone)]
struct AnimatedTexture {
    frame_width: u32,
    frame_height: u32,
    columns: u32,
    frames: Vec<AnimatedTextureFrame>,
    elapsed_seconds: f32,
    current_frame: usize,
}

impl AnimatedTexture {
    fn frame_at(&self, elapsed: f32) -> usize {
        let total: f32 = self.frames.iter().map(|frame| frame.duration_seconds).sum();
        if total <= f32::EPSILON { return 0; }
        let mut time = elapsed.rem_euclid(total);
        for (index, frame) in self.frames.iter().enumerate() {
            if time < frame.duration_seconds { return index; }
            time -= frame.duration_seconds;
        }
        0
    }

    fn crop(&self, image: &RgbaImage, frame: usize) -> RgbaImage {
        let index = self.frames.get(frame).map(|entry| entry.index).unwrap_or(0);
        let x = (index % self.columns.max(1)) * self.frame_width;
        let y = (index / self.columns.max(1)) * self.frame_height;
        image::imageops::crop_imm(
            image, x, y, self.frame_width, self.frame_height,
        ).to_image()
    }
}

#[derive(Debug, Clone)]
struct PortUiSummary {
    output_root: String,
    mod_id: String,
    display_name: String,
    conversion_state: String,
    item_count: usize,
    block_count: usize,
    entity_count: usize,
    armor_set_count: usize,
    projectile_count: usize,
    recipe_count: usize,
    resource_count: usize,
    warning_count: usize,
    analysis_file: String,
}

#[derive(Debug, Clone)]
struct BuildUiSummary {
    success: bool,
    exit_code: Option<i32>,
    duration_seconds: f64,
    log_file: String,
    report_file: String,
    diagnostics: Vec<String>,
    output_tail: String,
}

enum BackgroundMessage {
    Progress(String),
    PortCompleted(PortUiSummary),
    BuildCompleted(BuildUiSummary),
    Failed(String),
    Finished,
}

fn build_ui_summary(report: GradleCompileReport) -> BuildUiSummary {
    let combined = if report.stderr.trim().is_empty() {
        report.stdout.clone()
    } else {
        format!("{}\n{}", report.stdout, report.stderr)
    };
    let lines = combined.lines().collect::<Vec<_>>();
    let output_tail = lines.iter().rev().take(36).copied().collect::<Vec<_>>()
        .into_iter().rev().collect::<Vec<_>>().join("\n");
    BuildUiSummary {
        success: report.success,
        exit_code: report.exit_code,
        duration_seconds: report.duration_seconds,
        log_file: report.log_file.to_string_lossy().to_string(),
        report_file: report.report_file.to_string_lossy().to_string(),
        diagnostics: report.diagnostics.iter()
            .map(|item| format!("{}: {}", item.severity, item.message))
            .collect(),
        output_tail,
    }
}

fn port_ui_summary(report: &NeoForgePortReport) -> PortUiSummary {
    PortUiSummary {
        output_root: report.target_project.output_root.to_string_lossy().to_string(),
        mod_id: report.target_mod_id.clone(),
        display_name: report.target_project.mod_name.clone(),
        conversion_state: report.conversion_state.clone(),
        item_count: report.items.len(),
        block_count: report.blocks.len(),
        entity_count: report.entities.len(),
        armor_set_count: report.armor_sets.len(),
        projectile_count: report.native_projectiles.len(),
        recipe_count: report.native_recipes.generated.len(),
        resource_count: report.resource_files_copied,
        warning_count: report.warnings.len(),
        analysis_file: report.analysis_file.to_string_lossy().to_string(),
    }
}

pub struct MojModelBridgeApp {
    active_page: String,
    sidebar_collapsed: bool,
    root_path: String,
    project_sources: ProjectSources,
    active_project_manifest: Option<PathBuf>,
    active_project_name: String,
    scan_status: String,
    browser: BrowserIndex,
    category: String,
    item_search: String,
    selected_entry: Option<usize>,
    selected_error: String,

    scene: Option<ViewerScene>,
    sprite_3d_preview: bool,
    texture_paths: Vec<PathBuf>,
    selected_texture: usize,
    texture_images: Vec<RgbaImage>,
    preview_image: Option<RgbaImage>,
    texture_animation: Option<AnimatedTexture>,
    preview_texture: Option<TextureHandle>,
    icon_textures: HashMap<PathBuf, TextureHandle>,
    camera: Camera,
    selected_animation: usize,
    animation_time: f32,
    playing: bool,
    expanded_viewer: bool,
    last_frame: Instant,

    draft_name: String,
    draft_lore: String,
    components_json: String,
    metadata_json: String,
    minecraftia_available: bool,

    port_sources: Vec<String>,
    port_output: String,
    port_mod_id: String,
    port_mod_name: String,
    port_package: String,
    port_neoforge_version: String,
    port_overwrite: bool,
    port_build_after_conversion: bool,
    build_project_path: String,
    build_tasks: String,
    build_offline: bool,
    background_rx: Option<Receiver<BackgroundMessage>>,
    background_busy: bool,
    background_progress: String,
    background_error: String,
    background_log: Vec<String>,
    port_result: Option<PortUiSummary>,
    build_result: Option<BuildUiSummary>,
}

fn parse_animated_texture(
    image_dimensions: (u32, u32),
    metadata: &serde_json::Value,
) -> Option<AnimatedTexture> {
    let animation = metadata.get("animation")?.as_object()?;
    let (image_width, image_height) = image_dimensions;
    let width = animation.get("width").and_then(serde_json::Value::as_u64)
        .map(|value| value as u32).unwrap_or(image_width);
    let height = animation.get("height").and_then(serde_json::Value::as_u64)
        .map(|value| value as u32).unwrap_or(width);
    if width == 0 || height == 0 || width > image_width || height > image_height {
        return None;
    }
    let columns = image_width / width;
    let rows = image_height / height;
    if columns == 0 || rows == 0 { return None; }
    let frame_count = columns.saturating_mul(rows);
    let default_duration = animation.get("frametime").and_then(serde_json::Value::as_u64)
        .unwrap_or(1).max(1) as f32 / 20.0;
    let frames = if let Some(raw_frames) = animation.get("frames").and_then(serde_json::Value::as_array) {
        raw_frames.iter().filter_map(|raw| {
            let (index, duration) = if let Some(index) = raw.as_u64() {
                (index as u32, default_duration)
            } else {
                let object = raw.as_object()?;
                let index = object.get("index")?.as_u64()? as u32;
                let ticks = object.get("time").and_then(serde_json::Value::as_u64)
                    .unwrap_or((default_duration * 20.0) as u64).max(1);
                (index, ticks as f32 / 20.0)
            };
            (index < frame_count).then_some(AnimatedTextureFrame {
                index,
                duration_seconds: duration,
            })
        }).collect::<Vec<_>>()
    } else {
        (0..frame_count).map(|index| AnimatedTextureFrame {
            index,
            duration_seconds: default_duration,
        }).collect()
    };
    if frames.is_empty() { return None; }
    Some(AnimatedTexture {
        frame_width: width,
        frame_height: height,
        columns,
        frames,
        elapsed_seconds: 0.0,
        current_frame: 0,
    })
}

fn preferred_texture_index(entry: &BrowserEntry, textures: &[PathBuf]) -> usize {
    // When a model references multiple images, show an animated block/item
    // texture by default instead of silently choosing a static first texture.
    if entry.category != "Armor" && textures.len() > 1 {
        if let Some(index) = textures.iter().position(|path| {
            let metadata_path = PathBuf::from(format!("{}.mcmeta", path.display()));
            fs::read(&metadata_path).ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|metadata| metadata.get("animation").cloned())
                .is_some()
        }) {
            return index;
        }
    }
    if entry.category != "Armor" || textures.len() < 2 {
        return 0;
    }
    let detail = entry.detail.to_ascii_lowercase();
    if detail.contains("leggings") || detail.contains("leg_armor") || detail.contains("pants") {
        return textures.iter().position(|path| {
            path.file_name().and_then(|value| value.to_str())
                .is_some_and(|name| {
                    let name = name.to_ascii_lowercase();
                    name.contains("layer_2") || name.contains("layer2")
                })
        }).unwrap_or(0);
    }
    textures.iter().position(|path| {
        path.file_name().and_then(|value| value.to_str())
            .is_some_and(|name| {
                let name = name.to_ascii_lowercase();
                name.contains("layer_1") || name.contains("layer1")
            })
    }).unwrap_or(0)
}

impl MojModelBridgeApp {
    pub fn new(context: &eframe::CreationContext<'_>) -> Self {
        let minecraftia_available = load_minecraftia_font(&context.egui_ctx);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BACKGROUND;
        visuals.window_fill = SURFACE;
        visuals.extreme_bg_color = Color32::from_rgb(12, 15, 22);
        visuals.faint_bg_color = Color32::from_rgb(22, 28, 39);
        visuals.widgets.noninteractive.bg_fill = SURFACE;
        visuals.widgets.inactive.bg_fill = SURFACE_LIGHT;
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 64, 88);
        visuals.widgets.active.bg_fill = Color32::from_rgb(52, 111, 190);
        visuals.selection.bg_fill = Color32::from_rgb(52, 111, 190);
        visuals.override_text_color = Some(TEXT);
        context.egui_ctx.set_visuals(visuals);
        context.egui_ctx.global_style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 7.0);
            style.spacing.button_padding = egui::vec2(10.0, 7.0);
            style.visuals.window_corner_radius = egui::CornerRadius::same(8);
        });

        let recent_project = project::load_recent_project();
        let initial_port_sources = recent_project.as_ref()
            .map(|saved| saved.sources.paths().into_iter()
                .map(|path| path.to_string_lossy().to_string()).collect::<Vec<_>>())
            .unwrap_or_default();
        let default_port_output = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("modport-output")
            .to_string_lossy().to_string();
        Self {
            active_page: "Project Converter".to_owned(),
            sidebar_collapsed: false,
            root_path: String::new(),
            project_sources: recent_project.as_ref().map(|saved| saved.sources.clone()).unwrap_or_default(),
            active_project_manifest: recent_project.as_ref().map(|saved| saved.manifest_path.clone()),
            active_project_name: recent_project.as_ref().map(|saved| saved.name.clone()).unwrap_or_default(),
            scan_status: if recent_project.is_some() { "Saved project found · choose Update to rescan".to_owned() } else { "No project loaded".to_owned() },
            browser: BrowserIndex::default(),
            category: "Mobs".to_owned(),
            item_search: String::new(),
            selected_entry: None,
            selected_error: String::new(),
            scene: None,
            sprite_3d_preview: false,
            texture_paths: Vec::new(),
            selected_texture: 0,
            texture_images: Vec::new(),
            preview_image: None,
            texture_animation: None,
            preview_texture: None,
            icon_textures: HashMap::new(),
            camera: Camera::default(),
            selected_animation: 0,
            animation_time: 0.0,
            playing: false,
            expanded_viewer: false,
            last_frame: Instant::now(),
            draft_name: String::new(),
            draft_lore: String::new(),
            components_json: "{}".to_owned(),
            metadata_json: "{}".to_owned(),
            minecraftia_available,
            port_sources: initial_port_sources,
            port_output: default_port_output.clone(),
            port_mod_id: String::new(),
            port_mod_name: String::new(),
            port_package: String::new(),
            port_neoforge_version: DEFAULT_NEOFORGE_VERSION.to_owned(),
            port_overwrite: false,
            port_build_after_conversion: true,
            build_project_path: default_port_output,
            build_tasks: "build".to_owned(),
            build_offline: false,
            background_rx: None,
            background_busy: false,
            background_progress: "Ready to convert.".to_owned(),
            background_error: String::new(),
            background_log: Vec::new(),
            port_result: None,
            build_result: None,
        }
    }

    fn apply_browser_index(&mut self, context: &egui::Context, index: BrowserIndex) {
        let display = index.display_name.clone();
        let count = index.entries.len();
        self.browser = index;
        self.active_page = "Content Browser".to_owned();
        self.scan_status = format!("Indexed {count} assets · {display}");
        self.selected_entry = None;
        self.scene = None;
        self.sprite_3d_preview = false;
        self.texture_paths.clear();
        self.texture_images.clear();
        self.preview_image = None;
        self.preview_texture = None;
        self.selected_error.clear();
        self.item_search.clear();
        self.category = browser::CATEGORIES.iter()
            .copied()
            .find(|category| !self.browser.in_category(category).is_empty())
            .unwrap_or("Mobs").to_owned();
        if let Some(first) = self.browser.in_category(&self.category).first().copied() {
            self.activate_entry(context, first);
        }
    }

    fn scan(&mut self, context: &egui::Context) {
        let root = PathBuf::from(self.root_path.trim());
        match browser::scan_mod(&root) {
            Ok(index) => self.apply_browser_index(context, index),
            Err(error) => {
                self.scan_status = "Scan failed".to_owned();
                self.selected_error = format!("{error:#}");
            }
        }
    }

    fn scan_project_sources(&mut self, context: &egui::Context) {
        let roots = self.project_sources.paths();
        if roots.is_empty() {
            self.selected_error = "Select at least one Compiled, Decompiled, or Source folder.".to_owned();
            return;
        }
        match browser::scan_mod_roots(&roots) {
            Ok(index) => {
                let display = index.display_name.clone();
                self.apply_browser_index(context, index);
                match project::save_project(&display, &self.project_sources) {
                    Ok(manifest) => {
                        self.active_project_name = display.clone();
                        self.active_project_manifest = Some(manifest);
                        self.scan_status = format!("Project parsed · {} assets · {display}", self.browser.entries.len());
                    }
                    Err(error) => {
                        self.scan_status = format!("Parsed {} assets · project could not be saved", self.browser.entries.len());
                        self.selected_error = format!("Project manifest could not be saved: {error:#}");
                    }
                }
            }
            Err(error) => {
                self.scan_status = "Project scan failed".to_owned();
                self.selected_error = format!("{error:#}");
            }
        }
    }

    fn update_current_project(&mut self, context: &egui::Context) {
        let Some(manifest) = self.active_project_manifest.clone() else {
            self.scan_project_sources(context);
            return;
        };
        match project::load_project(&manifest) {
            Ok(saved) => {
                self.project_sources = saved.sources;
                self.active_project_name = saved.name;
                self.scan_project_sources(context);
            }
            Err(error) => {
                self.selected_error = format!("Could not load saved project sources: {error:#}");
            }
        }
    }

    fn rebuild_sprite_preview(&mut self) {
        let Some(path) = self.texture_paths.get(self.selected_texture).cloned() else { return; };
        let Some(image) = self.texture_images.get(self.selected_texture).cloned() else { return; };
        self.scene = Some(crate::resource_model::sprite_preview_scene(
            &self.draft_name,
            &path,
            &image,
        ));
        self.preview_texture = None;
        self.playing = false;
        self.selected_animation = 0;
        self.animation_time = 0.0;
    }

    fn load_legacy_model_preview(&mut self, entry: &BrowserEntry, source: &Path) -> anyhow::Result<()> {
        let asset = load_model_preview(source, &self.browser.texture_roots)?;
        let scene = ViewerScene::from_parsed_model(&asset.model)?;
        self.texture_paths = entry.texture_paths.clone();
        for path in &asset.texture_paths {
            if !self.texture_paths.contains(path) {
                self.texture_paths.push(path.clone());
            }
        }
        if self.texture_paths.is_empty() {
            self.texture_paths = asset.texture_paths;
        }
        self.selected_texture = if !entry.texture_paths.is_empty() {
            preferred_texture_index(entry, &self.texture_paths)
        } else {
            self.texture_paths.iter()
                .position(|path| path.to_string_lossy() == asset.model.primary_texture)
                .unwrap_or(0)
        };
        self.scene = Some(scene);
        self.refresh_preview_image();
        Ok(())
    }

    fn activate_entry(&mut self, context: &egui::Context, index: usize) {
        let Some(entry) = self.browser.entries.get(index).cloned() else { return; };
        self.selected_entry = Some(index);
        self.draft_name = entry.name.clone();
        self.draft_lore = entry.lore.join("\n");
        self.components_json = entry.components_json.clone();
        self.metadata_json = entry.metadata_json.clone();
        self.scene = None;
        self.sprite_3d_preview = false;
        self.texture_paths.clear();
        self.texture_images.clear();
        self.selected_texture = 0;
        self.preview_image = None;
        self.texture_animation = None;
        self.camera.reset();
        self.selected_animation = 0;
        self.animation_time = 0.0;
        self.playing = false;
        self.selected_error.clear();

        if entry.category == "Blocks"
            && entry.resource_model_source.is_none()
            && entry.model_source.is_none()
            && matches!(entry.render_shape.as_deref(), Some(
                "cube" | "plant" | "floor" | "torch" | "wall_torch"
                    | "fence" | "fence_post" | "fence_side" | "fence_gate"
                    | "wall" | "wall_post" | "wall_side"
                    | "pane" | "pane_post" | "pane_side"
            ))
        {
            let shape = entry.render_shape.as_deref().unwrap_or("cube");
            match crate::resource_model::synthetic_block_preview(
                &entry.name,
                shape,
                entry.icon_path.clone(),
            ) {
                Ok((scene, textures)) => {
                    self.scene = Some(scene);
                    self.texture_paths = textures;
                    self.selected_texture = 0;
                    self.refresh_preview_image();
                }
                Err(error) => self.selected_error = format!("Legacy block preview could not be created: {error:#}"),
            }
        } else if entry.category == "Armor" {
            if let Some(source) = entry.model_source.as_ref() {
                if let Err(error) = self.load_legacy_model_preview(&entry, source) {
                    self.selected_error = format!("Armor model preview could not be loaded: {error:#}");
                }
            } else if let Some(source) = entry.resource_model_source.as_ref() {
                match crate::resource_model::load_item_model(source, &self.browser.texture_roots) {
                    Ok((scene, textures)) => {
                        self.scene = Some(scene);
                        self.texture_paths = if !entry.texture_paths.is_empty() { entry.texture_paths.clone() } else { textures };
                        self.selected_texture = preferred_texture_index(&entry, &self.texture_paths);
                        self.refresh_preview_image();
                    }
                    Err(error) => self.selected_error = format!("Armor model preview could not be loaded: {error:#}"),
                }
            } else if let Some(path) = entry.icon_path.as_ref() {
                self.texture_paths.push(path.clone());
                self.refresh_preview_image();
            }
        } else if let Some(source) = entry.resource_model_source.as_ref() {
            let loaded = if entry.category == "Blocks" {
                let mut model_sources = vec![source.clone()];
                model_sources.extend(entry.additional_resource_model_sources.iter().cloned());
                let mut model_rotations = vec![entry.resource_model_rotation];
                model_rotations.extend(entry.additional_resource_model_rotations.iter().copied());
                crate::resource_model::load_block_models(
                    &model_sources,
                    &model_rotations,
                    &self.browser.texture_roots,
                )
            } else {
                crate::resource_model::load_item_model(source, &self.browser.texture_roots)
            };
            match loaded {
                Ok((scene, textures)) => {
                    self.scene = Some(scene);
                    self.texture_paths = if !textures.is_empty() {
                        textures
                    } else {
                        entry.texture_paths.clone()
                    };
                    self.selected_texture = preferred_texture_index(&entry, &self.texture_paths);
                    self.refresh_preview_image();
                }
                Err(error) => {
                    let resource_error = format!("{error:#}");
                    if let Some(source) = entry.model_source.as_ref() {
                        match self.load_legacy_model_preview(&entry, source) {
                            Ok(()) => {
                                self.selected_error.clear();
                            }
                            Err(legacy_error) => {
                                self.selected_error = format!(
                                    "Resource model preview failed ({resource_error}); legacy model preview failed ({legacy_error:#})."
                                );
                            }
                        }
                    } else {
                        self.selected_error = format!("Resource model preview could not be loaded: {resource_error}");
                    }
                    if self.scene.is_none() {
                        if let Some(path) = entry.icon_path.as_ref() {
                            self.texture_paths.push(path.clone());
                            self.refresh_preview_image();
                        }
                    }
                }
            }
        } else if let Some(source) = entry.model_source.as_ref() {
            if let Err(error) = self.load_legacy_model_preview(&entry, source) {
                self.selected_error = format!("Model preview could not be loaded: {error:#}");
            }
        } else if let Some(path) = entry.icon_path.as_ref().or_else(|| entry.texture_paths.first()) {
            self.texture_paths = if entry.texture_paths.is_empty() {
                vec![path.clone()]
            } else {
                entry.texture_paths.clone()
            };
            self.refresh_preview_image();
        }
        context.request_repaint();
    }

    fn refresh_preview_image(&mut self) {
        self.texture_animation = None;
        self.texture_images = self.texture_paths.iter().map(|path| {
            image::open(path).map(|image| image.to_rgba8())
                .unwrap_or_else(|_| RgbaImage::new(1, 1))
        }).collect();
        self.preview_image = self.texture_images.get(self.selected_texture).cloned();

        let Some(path) = self.texture_paths.get(self.selected_texture) else { return; };
        let Some(image) = self.texture_images.get(self.selected_texture) else { return; };
        let metadata_path = PathBuf::from(format!("{}.mcmeta", path.display()));
        let Ok(bytes) = fs::read(&metadata_path) else { return; };
        let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return; };
        let Some(animated) = parse_animated_texture((image.width(), image.height()), &metadata) else { return; };
        let frame = animated.crop(image, 0);
        self.preview_image = Some(frame.clone());
        if let Some(texture) = self.texture_images.get_mut(self.selected_texture) {
            *texture = frame;
        }
        self.texture_animation = Some(animated);
    }

    fn image_texture(&mut self, context: &egui::Context, path: &Path) -> Option<TextureHandle> {
        if let Some(texture) = self.icon_textures.get(path) {
            return Some(texture.clone());
        }
        let image = image::open(path).ok()?.to_rgba8();
        let dimensions = [image.width() as usize, image.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(dimensions, image.as_raw());
        let handle = context.load_texture(
            path.to_string_lossy().to_string(),
            color,
            TextureOptions::NEAREST,
        );
        self.icon_textures.insert(path.to_path_buf(), handle.clone());
        Some(handle)
    }

    fn show_sidebar(&mut self, root_ui: &mut egui::Ui) {
        let width = if self.sidebar_collapsed { 68.0 } else { 196.0 };
        egui::Panel::left("main_navigation")
            .exact_size(width)
            .resizable(false)
            .frame(egui::Frame::new().fill(Color32::from_rgb(14, 19, 28)).inner_margin(10.0))
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(if self.sidebar_collapsed { "»" } else { "«" }).clicked() {
                        self.sidebar_collapsed = !self.sidebar_collapsed;
                    }
                    if !self.sidebar_collapsed {
                        ui.label(RichText::new("MOJMODEL").strong().size(15.0));
                    }
                });
                if !self.sidebar_collapsed {
                    ui.label(RichText::new("BRIDGE  ·  RUST").size(10.0).color(ACCENT));
                    ui.add_space(18.0);
                } else {
                    ui.add_space(13.0);
                }

                let navigation = [
                    ("Dashboard", "⌂"),
                    ("Project Converter", "⇄"),
                    ("Porter", "⇢"),
                    ("Content Browser", "▦"),
                    ("Mod Editor", "✎"),
                    ("Code Studio", "</>"),
                    ("Compatibility", "◇"),
                    ("Build & Test", "⚒"),
                    ("Projects", "▣"),
                ];
                for (name, glyph) in navigation {
                    let label = if self.sidebar_collapsed {
                        glyph.to_owned()
                    } else {
                        format!("{glyph}   {name}")
                    };
                    if ui.selectable_label(self.active_page == name, label).clicked() {
                        self.active_page = name.to_owned();
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if !self.sidebar_collapsed {
                        ui.separator();
                        ui.label(RichText::new("● Native Rust engine").size(10.0).color(Color32::from_rgb(105, 207, 151)));
                        ui.label(RichText::new("Native Rust conversion engine").size(9.0).color(MUTED));
                    } else {
                        ui.label(RichText::new("●").color(Color32::from_rgb(105, 207, 151)));
                    }
                });
            });
    }

    fn show_header(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(self.active_page.as_str());
            if self.active_page == "Content Browser" {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Rescan directory").clicked() {
                        self.scan(context);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.root_path)
                            .desired_width(300.0)
                            .hint_text("Single extracted directory"),
                    );
                });
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.scan_status).size(11.0).color(MUTED));
            if !self.selected_error.is_empty() {
                ui.label(RichText::new(&self.selected_error).size(10.0).color(Color32::from_rgb(243, 137, 127)));
            }
        });
    }

    fn show_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.group(|ui| {
            ui.heading("Native mod workspace");
            ui.label("Open an extracted mod directory to browse recovered mobs, blocks, item textures, and model sources.");
            ui.add_space(10.0);
            ui.label(RichText::new("Rust engine · model and animation viewer · content browser").color(MUTED));
            ui.add_space(8.0);
            ui.label("The content browser is the first integrated editor surface. Porting and build workflows remain available from the navigation rail.");
        });
    }

    fn show_project_converter(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.heading("Import a mod as a project");
        ui.label(RichText::new("Select any combination of compiled classes, decompiled Java, and original source/resources. The app merges them into one parsed project and remembers the original folder locations.").color(MUTED));
        ui.add_space(12.0);

        show_source_folder_row(ui, "Compiled", &mut self.project_sources.compiled);
        show_source_folder_row(ui, "Decompiled", &mut self.project_sources.decompiled);
        show_source_folder_row(ui, "Source", &mut self.project_sources.source);

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let enabled = !self.project_sources.paths().is_empty();
            if ui.add_enabled(enabled, egui::Button::new("Create project and parse")).clicked() {
                self.scan_project_sources(context);
            }
            if ui.add_enabled(self.active_project_manifest.is_some(), egui::Button::new("Update saved project")).clicked() {
                self.update_current_project(context);
            }
        });
        ui.add_space(12.0);
        if !self.active_project_name.is_empty() {
            ui.group(|ui| {
                ui.label(RichText::new(format!("Current project: {}", self.active_project_name)).strong());
                if let Some(manifest) = &self.active_project_manifest {
                    ui.label(RichText::new(manifest.to_string_lossy()).size(9.0).color(MUTED));
                }
                ui.label("Update rescans the saved locations and replaces the current browser index; it does not import a second copy.");
            });
        } else {
            ui.label(RichText::new("A project manifest is created locally after the first successful parse.").color(MUTED));
        }
    }

    fn begin_port_conversion(&mut self) {
        if self.background_busy {
            return;
        }
        let roots = self.port_sources.iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if roots.is_empty() {
            self.background_error = "Add at least one mod folder, JAR, ZIP, Java source, or class file.".to_owned();
            return;
        }
        let missing = roots.iter().filter(|path| !path.exists())
            .map(|path| path.display().to_string()).collect::<Vec<_>>();
        if !missing.is_empty() {
            self.background_error = format!("These input paths do not exist: {}", missing.join(", "));
            return;
        }
        if self.port_output.trim().is_empty() {
            self.background_error = "Choose an output folder for the converted NeoForge project.".to_owned();
            return;
        }
        let output = PathBuf::from(self.port_output.trim());
        let mod_id = (!self.port_mod_id.trim().is_empty()).then(|| self.port_mod_id.trim().to_owned());
        let mod_name = (!self.port_mod_name.trim().is_empty()).then(|| self.port_mod_name.trim().to_owned());
        let package = (!self.port_package.trim().is_empty()).then(|| self.port_package.trim().to_owned());
        let neo_version = self.port_neoforge_version.trim().to_owned();
        let overwrite = self.port_overwrite;
        let build_after = self.port_build_after_conversion;

        let (sender, receiver) = mpsc::channel();
        self.background_rx = Some(receiver);
        self.background_busy = true;
        self.background_progress = "Starting native Rust mod analysis…".to_owned();
        self.background_error.clear();
        self.background_log.clear();
        self.port_result = None;
        self.build_result = None;
        self.build_project_path = output.to_string_lossy().to_string();

        thread::spawn(move || {
            let _ = sender.send(BackgroundMessage::Progress(
                "Scanning source roots and resolving the legacy API version…".to_owned()
            ));
            match port_to_neoforge(
                &roots,
                &output,
                mod_id.as_deref(),
                mod_name.as_deref(),
                package.as_deref(),
                &neo_version,
                None,
                overwrite,
            ) {
                Ok(report) => {
                    let output_root = report.target_project.output_root.clone();
                    let summary = port_ui_summary(&report);
                    let _ = sender.send(BackgroundMessage::PortCompleted(summary));
                    if build_after {
                        let _ = sender.send(BackgroundMessage::Progress(
                            "Conversion finished. Running Gradle build for the generated NeoForge mod…".to_owned()
                        ));
                        match compile_workspace(&output_root, &[], false, true, false) {
                            Ok(build) => {
                                let _ = sender.send(BackgroundMessage::BuildCompleted(build_ui_summary(build)));
                            }
                            Err(error) => {
                                let _ = sender.send(BackgroundMessage::Failed(format!(
                                    "Conversion completed, but Gradle could not be started: {error:#}"
                                )));
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(BackgroundMessage::Failed(format!(
                        "Mod conversion failed: {error:#}"
                    )));
                }
            }
            let _ = sender.send(BackgroundMessage::Finished);
        });
    }

    fn begin_gradle_build(&mut self) {
        if self.background_busy {
            return;
        }
        let root = PathBuf::from(self.build_project_path.trim());
        if self.build_project_path.trim().is_empty()
            || (!root.join("build.gradle").is_file() && !root.join("build.gradle.kts").is_file())
        {
            self.background_error = format!(
                "No Gradle build file was found in '{}'. Select a generated NeoForge project folder first.",
                root.display()
            );
            return;
        }
        let tasks = self.build_tasks
            .split(|character: char| character.is_whitespace() || character == ',')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let offline = self.build_offline;
        let (sender, receiver) = mpsc::channel();
        self.background_rx = Some(receiver);
        self.background_busy = true;
        self.background_progress = format!(
            "Running Gradle task(s): {}…",
            if tasks.is_empty() { "build".to_owned() } else { tasks.join(", ") }
        );
        self.background_error.clear();
        self.background_log.clear();
        self.build_result = None;

        thread::spawn(move || {
            let _ = sender.send(BackgroundMessage::Progress("Gradle is running in the background…".to_owned()));
            match compile_workspace(&root, &tasks, offline, true, false) {
                Ok(report) => {
                    let _ = sender.send(BackgroundMessage::BuildCompleted(build_ui_summary(report)));
                }
                Err(error) => {
                    let _ = sender.send(BackgroundMessage::Failed(format!(
                        "Gradle build could not be started: {error:#}"
                    )));
                }
            }
            let _ = sender.send(BackgroundMessage::Finished);
        });
    }

    fn poll_background_job(&mut self, context: &egui::Context) {
        let mut messages = Vec::new();
        let mut disconnected = false;
        if let Some(receiver) = self.background_rx.as_ref() {
            loop {
                match receiver.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        let mut finished = false;
        for message in messages {
            match message {
                BackgroundMessage::Progress(value) => {
                    self.background_progress = value.clone();
                    self.background_log.push(value);
                }
                BackgroundMessage::PortCompleted(summary) => {
                    self.build_project_path = summary.output_root.clone();
                    self.background_progress = format!(
                        "Ported {} · {} item(s), {} block(s), {} mob(s).",
                        summary.mod_id, summary.item_count, summary.block_count, summary.entity_count
                    );
                    self.background_log.push(self.background_progress.clone());
                    self.port_result = Some(summary);
                }
                BackgroundMessage::BuildCompleted(summary) => {
                    self.background_progress = if summary.success {
                        format!("Gradle build passed in {:.1}s.", summary.duration_seconds)
                    } else {
                        format!("Gradle build failed (exit {:?}).", summary.exit_code)
                    };
                    self.background_log.push(self.background_progress.clone());
                    self.build_result = Some(summary);
                }
                BackgroundMessage::Failed(error) => {
                    self.background_error = error.clone();
                    self.background_progress = "Task finished with an error.".to_owned();
                    self.background_log.push(error);
                }
                BackgroundMessage::Finished => finished = true,
            }
        }
        if self.background_busy {
            context.request_repaint_after(Duration::from_millis(120));
        }
        if finished || disconnected {
            self.background_busy = false;
            self.background_rx = None;
            if disconnected && !finished {
                self.background_error = "The background conversion/build task stopped unexpectedly.".to_owned();
            }
            context.request_repaint();
        }
    }

    fn show_porter(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Convert a legacy or existing mod");
        ui.label(RichText::new(
            "Choose one or more source inputs. Modport merges compiled classes, decompiled Java, original source, and mod archives before generating a native Minecraft 1.21.1 / NeoForge project."
        ).color(MUTED));
        ui.add_space(10.0);

        ui.group(|ui| {
            ui.heading("Source inputs");
            if self.port_sources.is_empty() {
                ui.label(RichText::new("No input selected yet. Add a mod folder, JAR/ZIP, or loose Java/class files.").color(MUTED));
            }
            let mut remove_index = None;
            for (index, path) in self.port_sources.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(path)
                        .desired_width((ui.available_width() - 42.0).max(140.0)));
                    if ui.small_button("×").clicked() {
                        remove_index = Some(index);
                    }
                });
            }
            if let Some(index) = remove_index {
                self.port_sources.remove(index);
            }
            ui.horizontal(|ui| {
                if ui.button("＋ Add folder").clicked() {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("Add mod source folder")
                        .pick_folder()
                    {
                        let value = folder.to_string_lossy().to_string();
                        if !self.port_sources.contains(&value) {
                            self.port_sources.push(value);
                        }
                    }
                }
                if ui.button("＋ Add JAR / ZIP / source files").clicked() {
                    if let Some(files) = rfd::FileDialog::new()
                        .set_title("Add mod JAR, ZIP, Java, or class files")
                        .add_filter("Mod archives and source", &["jar", "zip", "java", "class"])
                        .pick_files()
                    {
                        for file in files {
                            let value = file.to_string_lossy().to_string();
                            if !self.port_sources.contains(&value) {
                                self.port_sources.push(value);
                            }
                        }
                    }
                }
                if ui.button("Use current project sources").clicked() {
                    for path in self.project_sources.paths() {
                        let value = path.to_string_lossy().to_string();
                        if !self.port_sources.contains(&value) {
                            self.port_sources.push(value);
                        }
                    }
                }
                if ui.button("Clear inputs").clicked() {
                    self.port_sources.clear();
                }
            });
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.heading("NeoForge output");
            ui.horizontal(|ui| {
                ui.label("Output folder");
                ui.add(egui::TextEdit::singleline(&mut self.port_output)
                    .desired_width((ui.available_width() - 95.0).max(180.0)));
                if ui.button("Browse…").clicked() {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("Choose converted NeoForge project output folder")
                        .pick_folder()
                    {
                        self.port_output = folder.to_string_lossy().to_string();
                    }
                }
            });
            egui::Grid::new("port_target_options").num_columns(2).spacing([12.0, 7.0]).show(ui, |ui| {
                ui.label("Target mod ID");
                ui.add(egui::TextEdit::singleline(&mut self.port_mod_id)
                    .desired_width(f32::INFINITY).hint_text("Auto-detect from mod metadata"));
                ui.end_row();
                ui.label("Display name");
                ui.add(egui::TextEdit::singleline(&mut self.port_mod_name)
                    .desired_width(f32::INFINITY).hint_text("Auto-detect from mod metadata"));
                ui.end_row();
                ui.label("Java package");
                ui.add(egui::TextEdit::singleline(&mut self.port_package)
                    .desired_width(f32::INFINITY).hint_text("Auto: com.modport.<modid>"));
                ui.end_row();
                ui.label("NeoForge version");
                ui.add(egui::TextEdit::singleline(&mut self.port_neoforge_version).desired_width(f32::INFINITY));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.port_overwrite, "Replace generated files if they already exist");
                ui.checkbox(&mut self.port_build_after_conversion, "Build the generated mod after porting");
            });
            ui.label(RichText::new(
                "The output is a real Gradle project. A successful conversion does not imply that every custom AI routine, special ability, event, or network behavior has been reconstructed; check port-analysis.json for recovered properties and pending adapters."
            ).size(10.0).color(MUTED));
            ui.add_space(5.0);
            let can_start = !self.background_busy && !self.port_sources.is_empty()
                && !self.port_output.trim().is_empty() && !self.port_neoforge_version.trim().is_empty();
            if ui.add_enabled(can_start, egui::Button::new("▶  Start mod conversion")
                .fill(ACCENT).min_size(egui::vec2(190.0, 36.0))).clicked()
            {
                self.begin_port_conversion();
            }
        });

        self.show_background_status(ui);
        if let Some(result) = self.port_result.as_ref() {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.heading("Conversion result");
                    ui.label(RichText::new(&result.conversion_state).size(9.0).color(MUTED));
                });
                ui.label(RichText::new(format!("{} ({})", result.display_name, result.mod_id)).strong());
                ui.label(format!("Output: {}", result.output_root));
                egui::Grid::new("port_result_counts").num_columns(4).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(format!("Items / tools: {}", result.item_count));
                    ui.label(format!("Blocks: {}", result.block_count));
                    ui.label(format!("Entities: {}", result.entity_count));
                    ui.label(format!("Armor sets: {}", result.armor_set_count));
                    ui.end_row();
                    ui.label(format!("Projectiles: {}", result.projectile_count));
                    ui.label(format!("Recipes: {}", result.recipe_count));
                    ui.label(format!("Resources copied: {}", result.resource_count));
                    ui.label(format!("Warnings: {}", result.warning_count));
                    ui.end_row();
                });
                ui.label(RichText::new(format!("Analysis report: {}", result.analysis_file)).size(9.0).color(MUTED));
                if ui.button("Open Build & Test").clicked() {
                    self.active_page = "Build & Test".to_owned();
                }
            });
        }
    }

    fn show_build_test(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Build and verify a generated mod");
        ui.label(RichText::new(
            "Run Gradle against a generated NeoForge project. Build logs and structured diagnostics are written beside the project so failed Java compilation can be inspected."
        ).color(MUTED));
        ui.add_space(10.0);
        ui.group(|ui| {
            ui.heading("Gradle project");
            ui.horizontal(|ui| {
                ui.label("Project folder");
                ui.add(egui::TextEdit::singleline(&mut self.build_project_path)
                    .desired_width((ui.available_width() - 95.0).max(180.0)));
                if ui.button("Browse…").clicked() {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("Select a Gradle / NeoForge project")
                        .pick_folder()
                    {
                        self.build_project_path = folder.to_string_lossy().to_string();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Gradle tasks");
                ui.add(egui::TextEdit::singleline(&mut self.build_tasks)
                    .desired_width(240.0).hint_text("build"));
                ui.checkbox(&mut self.build_offline, "Offline");
            });
            let root = PathBuf::from(self.build_project_path.trim());
            let has_gradle = root.join("build.gradle").is_file() || root.join("build.gradle.kts").is_file();
            if !has_gradle && !self.build_project_path.trim().is_empty() {
                ui.label(RichText::new("No build.gradle or build.gradle.kts found in the selected folder.").size(10.0).color(Color32::from_rgb(238, 167, 115)));
            }
            if ui.add_enabled(!self.background_busy && has_gradle, egui::Button::new("⚒  Run Gradle build")
                .fill(ACCENT).min_size(egui::vec2(170.0, 34.0))).clicked()
            {
                self.begin_gradle_build();
            }
        });
        self.show_background_status(ui);
        if let Some(result) = self.build_result.as_ref() {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.heading(if result.success { "Build successful" } else { "Build failed" });
                    ui.label(RichText::new(format!("{:.1}s · exit {:?}", result.duration_seconds, result.exit_code)).color(MUTED));
                });
                ui.label(format!("Build log: {}", result.log_file));
                ui.label(format!("Build report: {}", result.report_file));
                for diagnostic in result.diagnostics.iter().take(30) {
                    ui.label(RichText::new(diagnostic).size(10.0).color(
                        if diagnostic.to_ascii_lowercase().starts_with("error") {
                            Color32::from_rgb(243, 137, 127)
                        } else {
                            Color32::from_rgb(236, 184, 109)
                        }
                    ));
                }
                if !result.output_tail.trim().is_empty() {
                    egui::CollapsingHeader::new("Gradle output (last lines)")
                        .default_open(!result.success)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(
                                RichText::new(&result.output_tail).monospace()
                            ).wrap());
                        });
                }
            });
        }
    }

    fn show_background_status(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if self.background_busy {
                ui.spinner();
            } else {
                ui.label(RichText::new("●").color(if self.background_error.is_empty() {
                    Color32::from_rgb(105, 207, 151)
                } else {
                    Color32::from_rgb(243, 137, 127)
                }));
            }
            ui.label(RichText::new(&self.background_progress).color(MUTED));
        });
        if !self.background_error.is_empty() {
            ui.label(RichText::new(&self.background_error).color(Color32::from_rgb(243, 137, 127)));
        }
        if !self.background_log.is_empty() {
            egui::CollapsingHeader::new("Task history").default_open(false).show(ui, |ui| {
                for entry in self.background_log.iter().rev().take(12).rev() {
                    ui.label(RichText::new(entry).size(10.0).color(MUTED));
                }
            });
        }
    }

    fn show_projects(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.heading("Saved projects");
        ui.label(RichText::new("Saved projects remember the original source locations and can be rescanned when parsing support improves.").color(MUTED));
        ui.add_space(8.0);
        let projects = project::list_projects();
        if projects.is_empty() {
            ui.label("No saved projects yet. Import one from Project Converter.");
        } else {
            egui::ScrollArea::vertical().id_salt("saved-projects-list")
                .max_height(ui.available_height().max(80.0))
                .show(ui, |ui| {
                    for saved in &projects {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&saved.name).strong());
                                if ui.button("Load and rescan").clicked() {
                                    self.project_sources = saved.sources.clone();
                                    self.active_project_name = saved.name.clone();
                                    self.active_project_manifest = Some(saved.manifest_path.clone());
                                    self.update_current_project(context);
                                }
                            });
                            for (role, path) in [
                                ("Compiled", saved.sources.compiled.as_str()),
                                ("Decompiled", saved.sources.decompiled.as_str()),
                                ("Source", saved.sources.source.as_str()),
                            ] {
                                if !path.is_empty() {
                                    ui.label(RichText::new(format!("{role}: {path}")).size(9.0).color(MUTED));
                                }
                            }
                        });
                    }
                });
        }
        if ui.button("Open Project Converter").clicked() {
            self.active_page = "Project Converter".to_owned();
        }
    }

    fn show_browser(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let browser_width = (available.x * 0.34).clamp(340.0, 475.0);
        let category_width = 134.0;
        let item_width = (browser_width - category_width - 10.0).max(190.0);
        let detail_width = (available.x - browser_width - 18.0).max(300.0);
        let height = available.y;

        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(category_width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.show_categories(ui),
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(item_width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.show_items(context, ui),
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(detail_width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("content-browser-details")
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.show_detail(context, ui));
                },
            );
        });
    }

    fn show_categories(&mut self, ui: &mut egui::Ui) {
        ui.heading("Categories");
        ui.label(RichText::new(format!("{} indexed", self.browser.entries.len())).size(10.0).color(MUTED));
        ui.add_space(7.0);
        // Eight fixed categories fit in the navigation rail; keeping this list
        // non-scrollable prevents it from capturing wheel input intended for
        // the neighboring item and detail panes.
        ui.vertical(|ui| {
            for category in browser::CATEGORIES {
                let count = self.browser.in_category(category).len();
                let label = format!("{category}  ({count})");
                if ui.selectable_value(&mut self.category, (*category).to_owned(), label).changed() {
                    self.item_search.clear();
                    self.selected_entry = None;
                    self.scene = None;
                    self.sprite_3d_preview = false;
                    self.texture_paths.clear();
                    self.texture_images.clear();
                    self.preview_image = None;
                    self.preview_texture = None;
                    self.texture_animation = None;
                    self.selected_error.clear();
                }
            }
        });
    }

    fn show_items(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        ui.heading(&self.category);
        ui.add(
            egui::TextEdit::singleline(&mut self.item_search)
                .desired_width(f32::INFINITY)
                .hint_text("Search items"),
        );
        let items = self.browser.in_category(&self.category);
        ui.label(RichText::new(format!("{} entries", items.len())).size(10.0).color(MUTED));
        ui.add_space(5.0);
        egui::ScrollArea::vertical()
            .id_salt("content-browser-items")
            .max_height(ui.available_height().max(48.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
            for index in items {
                let Some(entry) = self.browser.entries.get(index).cloned() else { continue; };
                if !self.item_search.trim().is_empty()
                    && !entry.name.to_ascii_lowercase().contains(&self.item_search.to_ascii_lowercase())
                {
                    continue;
                }
                let selected = self.selected_entry == Some(index);
                let icon = if entry.icon_kind == IconKind::MobEgg {
                    None
                } else {
                    entry.icon_path.as_ref().and_then(|path| self.image_texture(context, path))
                };
                let clicked = ui.horizontal(|ui| {
                    if let Some(icon) = icon {
                        ui.add(egui::Image::new((icon.id(), egui::vec2(30.0, 30.0))).fit_to_exact_size(egui::vec2(30.0, 30.0)));
                    } else if entry.icon_kind == IconKind::MobEgg {
                        renderer::draw_spawn_egg(ui, 30.0, selected);
                    } else {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(52, 66, 87));
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "◆", egui::FontId::proportional(13.0), ACCENT);
                    }
                    ui.selectable_label(selected, entry.name.as_str()).clicked()
                }).inner;
                if clicked {
                    self.activate_entry(context, index);
                }
            }
        });
        if let Some(warning) = self.browser.warnings.first() {
            ui.add_space(5.0);
            ui.label(RichText::new(warning).size(9.0).color(MUTED));
        }
    }

    fn show_detail(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        let Some(index) = self.selected_entry else {
            ui.group(|ui| {
                ui.heading("Preview and editor");
                ui.label("Select a mob, block, item, or model from the browser.");
            });
            return;
        };
        let Some(entry) = self.browser.entries.get(index).cloned() else { return; };
        let available = ui.available_size();
        let preview_height = if self.expanded_viewer {
            available.y - 12.0
        } else {
            (available.y * 0.55).clamp(270.0, 450.0)
        };

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.heading(if self.scene.is_some() { "Model / animation viewer" } else { "Asset preview" });
                if sprite_preview_available(&entry) {
                    let toggle_label = if self.sprite_3d_preview { "Show 2D" } else { "Extrude to 3D" };
                    if ui.button(toggle_label).clicked() {
                        self.sprite_3d_preview = !self.sprite_3d_preview;
                        if self.sprite_3d_preview {
                            self.rebuild_sprite_preview();
                        } else {
                            self.scene = None;
                            self.preview_texture = None;
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(if self.expanded_viewer { "Collapse" } else { "Expand" }).clicked() {
                        self.expanded_viewer = !self.expanded_viewer;
                    }
                    if ui.button("Reset camera").clicked() {
                        self.camera.reset();
                    }
                });
            });
            if let Some(shape) = entry.render_shape.as_deref() {
                ui.label(RichText::new(format!("In-game block model: {shape}"))
                    .size(10.0).color(MUTED));
            }
            self.show_viewer_controls(context, ui);
            self.show_viewport(context, ui, &entry, preview_height);
        });

        if !self.expanded_viewer {
            ui.add_space(8.0);
            egui::ScrollArea::vertical().id_salt("asset-editor-scroll").max_height((available.y - preview_height - 20.0).max(180.0)).show(ui, |ui| {
                ui.group(|ui| self.show_editor(ui));
            });
        }
    }

    fn show_viewer_controls(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(scene) = self.scene.as_ref() {
                if scene.animations.is_empty() {
                    ui.add_enabled(false, egui::Button::new("No baked animations"));
                } else {
                    let selected_name = scene.animations.get(self.selected_animation)
                        .map(|animation| animation.name.as_str()).unwrap_or("Select animation");
                    egui::ComboBox::from_id_salt("animation_selector")
                        .selected_text(selected_name)
                        .width(210.0)
                        .show_ui(ui, |ui| {
                            for (index, animation) in scene.animations.iter().enumerate() {
                                if ui.selectable_value(&mut self.selected_animation, index, &animation.name).clicked() {
                                    self.animation_time = 0.0;
                                }
                            }
                        });
                    let label = if self.playing { "Ⅱ Pause" } else { "▶ Play" };
                    if ui.button(label).clicked() {
                        if !self.playing {
                            if let Some(animation) = scene.animations.get(self.selected_animation) {
                                if self.animation_time >= animation.length {
                                    self.animation_time = 0.0;
                                }
                            }
                        }
                        self.playing = !self.playing;
                        self.last_frame = Instant::now();
                    }
                }

            } else {
                ui.label(RichText::new("No model source is linked to this asset yet.").size(10.0).color(MUTED));
            }
        });

        if let Some(scene) = self.scene.as_ref() {
            if let Some(animation) = scene.animations.get(self.selected_animation) {
                let length = animation.length.max(0.01);
                let was_playing = self.playing;
                ui.add(egui::Slider::new(&mut self.animation_time, 0.0..=length).show_value(true).text("Time"));
                if was_playing && !self.playing {
                    context.request_repaint();
                }
            }
        }
    }

    fn show_viewport(
        &mut self,
        context: &egui::Context,
        ui: &mut egui::Ui,
        entry: &BrowserEntry,
        requested_height: f32,
    ) {
        let width = ui.available_width().max(180.0);
        let height = requested_height.max(190.0);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(width, height),
            egui::Sense::click_and_drag(),
        );
        ui.painter().rect_filled(rect, 5.0, Color32::from_rgb(18, 23, 33));
        if response.dragged() {
            let delta = context.input(|input| input.pointer.delta());
            if context.input(|input| input.pointer.button_down(egui::PointerButton::Secondary)) {
                self.camera.pan(delta.x, delta.y);
            } else {
                self.camera.orbit(delta.x, delta.y);
            }
        }
        if response.hovered() && context.input(|input| input.modifiers.ctrl) {
            let wheel = context.input(|input| input.smooth_scroll_delta.y);
            if wheel.abs() > f32::EPSILON { self.camera.zoom(wheel); }
        }

        if let Some(scene) = self.scene.as_ref() {
            let pose = scene.sample_pose(self.selected_animation, self.animation_time);
            let render_scale = (960.0 / rect.width().max(1.0))
                .min(720.0 / rect.height().max(1.0))
                .min(1.0);
            let image_size = [
                (rect.width() * render_scale).round().max(96.0) as usize,
                (rect.height() * render_scale).round().max(96.0) as usize,
            ];
            let rendered = renderer::render_scene_with_textures(
                scene,
                &pose,
                &self.texture_images,
                self.selected_texture,
                image_size,
                &mut self.camera,
            );
            if let Some(handle) = self.preview_texture.as_mut() {
                handle.set(rendered, TextureOptions::NEAREST);
            } else {
                self.preview_texture = Some(context.load_texture(
                    "model_viewport",
                    rendered,
                    TextureOptions::NEAREST,
                ));
            }
            if let Some(handle) = self.preview_texture.as_ref() {
                let source_size = handle.size_vec2();
                let contain_scale = (rect.width() / source_size.x.max(1.0))
                    .min(rect.height() / source_size.y.max(1.0));
                let draw_size = source_size * contain_scale;
                let draw_rect = egui::Rect::from_center_size(rect.center(), draw_size);
                ui.painter().image(
                    handle.id(),
                    draw_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            if !scene.warnings.is_empty() {
                ui.painter().text(
                    rect.left_bottom() + egui::vec2(8.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    scene.warnings.len().to_string() + " model warning(s)",
                    egui::FontId::proportional(10.0),
                    Color32::from_rgb(236, 184, 109),
                );
            }
        } else if let Some(image) = self.preview_image.as_ref() {
            let dimensions = [image.width() as usize, image.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(dimensions, image.as_raw());
            if let Some(handle) = self.preview_texture.as_mut() {
                handle.set(color, TextureOptions::NEAREST);
            } else {
                self.preview_texture = Some(context.load_texture("asset_preview", color, TextureOptions::NEAREST));
            }
            if let Some(handle) = self.preview_texture.as_ref() {
                let source_size = handle.size_vec2();
                let contain_scale = (width * 0.85 / source_size.x.max(1.0))
                    .min(height * 0.85 / source_size.y.max(1.0));
                let image_rect = egui::Rect::from_center_size(rect.center(), source_size * contain_scale);
                ui.painter().image(
                    handle.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else if entry.icon_kind == IconKind::MobEgg {
            let center = rect.center();
            let mut outline = Vec::new();
            for index in 0..32 {
                let angle = std::f32::consts::TAU * index as f32 / 32.0;
                let vertical = if angle.sin() < 0.0 { 0.82 } else { 1.08 };
                outline.push(center + egui::vec2(angle.cos() * 42.0, angle.sin() * 58.0 * vertical));
            }
            ui.painter().add(egui::Shape::convex_polygon(
                outline,
                Color32::from_rgb(230, 230, 218),
                egui::Stroke::new(2.0, Color32::from_rgb(7, 9, 12)),
            ));
            ui.painter().circle_filled(center + egui::vec2(2.0, 8.0), 24.0, Color32::from_rgb(93, 153, 71));
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "MODEL SOURCE NOT AVAILABLE",
                egui::FontId::proportional(13.0),
                MUTED,
            );
        }
        ui.label(RichText::new("Drag to orbit · right-drag to pan · Ctrl+scroll to zoom").size(9.0).color(MUTED));
    }

    fn show_editor(&mut self, ui: &mut egui::Ui) {
        ui.heading("Basic item editor");
        if self.texture_paths.len() > 1 {
            let selected = self.texture_paths.get(self.selected_texture)
                .and_then(|path| path.file_name()).map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "Texture".to_owned());
            let previous = self.selected_texture;
            egui::ComboBox::from_id_salt("editor_texture_selector")
                .selected_text(selected)
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (index, path) in self.texture_paths.iter().enumerate() {
                        let label = path.file_name().map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.to_string_lossy().to_string());
                        ui.selectable_value(&mut self.selected_texture, index, label);
                    }
                });
            if previous != self.selected_texture {
                self.refresh_preview_image();
                if self.sprite_3d_preview { self.rebuild_sprite_preview(); }
            }
        } else if let Some(path) = self.texture_paths.first() {
            ui.label(RichText::new(path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string())).size(10.0).color(MUTED));
        }
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut self.draft_name).desired_width(f32::INFINITY));
        });
        if !self.draft_lore.trim().is_empty() {
            ui.label("Detected lore");
            ui.add(egui::TextEdit::multiline(&mut self.draft_lore)
                .desired_rows(3).desired_width(f32::INFINITY)
                .hint_text("One lore line per row. Use § color/style codes."));
        }
        if self.components_json.trim() != "{}" && !self.components_json.trim().is_empty() {
            ui.heading("Item components");
            show_json_object_editor(ui, &mut self.components_json, "components");
        }
        if self.metadata_json.trim() != "{}" && !self.metadata_json.trim().is_empty() {
            ui.heading("Mob metadata");
            show_json_object_editor(ui, &mut self.metadata_json, "mob_metadata");
        }

        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.heading("In-game tooltip preview");
            ui.label(RichText::new(if self.minecraftia_available { "Minecraftia font" } else { "Fallback font" }).size(9.0).color(MUTED));
        });
        egui::Frame::new()
            .fill(Color32::from_rgb(14, 15, 22))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(75, 82, 103)))
            .inner_margin(12.0)
            .corner_radius(4.0)
            .show(ui, |ui| {
                tooltip::draw_formatted_text(
                    ui,
                    &self.draft_name,
                    Color32::WHITE,
                    self.minecraftia_available,
                );
                for line in self.draft_lore.lines() {
                    if !line.is_empty() {
                        tooltip::draw_formatted_text_with_italic(
                            ui,
                            line,
                            Color32::from_rgb(170, 170, 170),
                            true,
                            self.minecraftia_available,
                        );
                    }
                }
                if self.components_json.trim() != "{}" && !self.components_json.trim().is_empty() {
                    match serde_json::from_str::<serde_json::Value>(&self.components_json) {
                        Ok(value) => tooltip::draw_component_json(
                            ui,
                            &value,
                            Color32::WHITE,
                            self.minecraftia_available,
                        ),
                        Err(error) => {
                            ui.label(RichText::new(format!("Component JSON error: {error}")).size(9.0).color(Color32::from_rgb(240, 130, 120)));
                        }
                    }
                }
            });
        ui.label(RichText::new("Draft fields are local to this editor preview; workspace write-back is a separate editing stage.").size(9.0).color(MUTED));
    }
}

fn show_json_field_editor(ui: &mut egui::Ui, label: &str, value: &mut serde_json::Value) -> bool {
    use serde_json::Value;
    match value {
        Value::Object(object) => {
            let mut changed = false;
            egui::CollapsingHeader::new(label).default_open(true).show(ui, |ui| {
                for (key, field) in object.iter_mut() {
                    changed |= show_json_field_editor(ui, key, field);
                }
            });
            changed
        }
        Value::Array(values) => {
            let mut changed = false;
            egui::CollapsingHeader::new(label).default_open(true).show(ui, |ui| {
                for (index, field) in values.iter_mut().enumerate() {
                    if let Value::String(text) = field {
                        ui.horizontal(|ui| {
                            ui.label(format!("[{index}]"));
                            changed |= ui.text_edit_singleline(text).changed();
                        });
                    } else {
                        changed |= show_json_field_editor(ui, &format!("[{index}]"), field);
                    }
                }
            });
            changed
        }
        Value::Number(_) => {
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(label);
                if let Some(mut integer) = value.as_i64() {
                    if ui.add(egui::DragValue::new(&mut integer).speed(1.0)).changed() {
                        *value = Value::from(integer);
                        changed = true;
                    }
                } else if let Some(mut number) = value.as_f64() {
                    if ui.add(egui::DragValue::new(&mut number).speed(0.05)).changed() {
                        if let Some(number) = serde_json::Number::from_f64(number) {
                            *value = Value::Number(number);
                            changed = true;
                        }
                    }
                }
            });
            changed
        }
        Value::Bool(flag) => ui.checkbox(flag, label).changed(),
        Value::String(text) => {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.text_edit_singleline(text).changed()
            }).inner
        }
        Value::Null => {
            ui.label(format!("{label}: not recovered"));
            false
        }
    }
}

fn show_json_object_editor(ui: &mut egui::Ui, encoded: &mut String, id: &str) {
    match serde_json::from_str::<serde_json::Value>(encoded) {
        Ok(serde_json::Value::Object(mut object)) => {
            let mut changed = false;
            for (key, value) in object.iter_mut() {
                changed |= show_json_field_editor(ui, key, value);
            }
            if changed {
                *encoded = serde_json::to_string_pretty(&serde_json::Value::Object(object))
                    .unwrap_or_else(|_| "{}".to_owned());
            }
        }
        Ok(_) => {
            ui.label(format!("{id}: expected an object"));
        }
        Err(error) => {
            ui.label(format!("{id} JSON error: {error}"));
            ui.add(egui::TextEdit::multiline(encoded).desired_rows(3).desired_width(f32::INFINITY));
        }
    }
}

impl eframe::App for MojModelBridgeApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background_job(context);
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame).as_secs_f32().clamp(0.0, 0.1);
        self.last_frame = now;
        if self.playing {
            if let Some(animation) = self.scene.as_ref().and_then(|scene| scene.animations.get(self.selected_animation)) {
                self.animation_time += delta;
                if animation.looping && animation.length > f32::EPSILON {
                    self.animation_time = self.animation_time.rem_euclid(animation.length);
                } else if self.animation_time >= animation.length {
                    self.animation_time = animation.length;
                    self.playing = false;
                }
                context.request_repaint_after(Duration::from_millis(16));
            } else {
                self.playing = false;
            }
        }
        let mut sprite_frame_changed = false;
        if let Some(animation) = self.texture_animation.as_mut() {
            animation.elapsed_seconds += delta;
            let next = animation.frame_at(animation.elapsed_seconds);
            if next != animation.current_frame {
                animation.current_frame = next;
                if let Some(path) = self.texture_paths.get(self.selected_texture) {
                    if let Ok(image) = image::open(path) {
                        let frame = animation.crop(&image.to_rgba8(), next);
                        self.preview_image = Some(frame.clone());
                        if let Some(texture) = self.texture_images.get_mut(self.selected_texture) {
                            *texture = frame;
                        }
                        sprite_frame_changed = true;
                    }
                }
            }
            context.request_repaint_after(Duration::from_millis(50));
        }
        if sprite_frame_changed && self.sprite_3d_preview {
            self.rebuild_sprite_preview();
        }
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show_sidebar(root_ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(14.0))
            .show(root_ui, |ui| {
                let context = ui.ctx().clone();
                self.show_header(&context, ui);
                ui.separator();
                let active_page = self.active_page.clone();
                match active_page.as_str() {
                    "Content Browser" => self.show_browser(&context, ui),
                    "Project Converter" => self.show_project_converter(&context, ui),
                    "Porter" => self.show_porter(ui),
                    "Build & Test" => self.show_build_test(ui),
                    "Projects" => self.show_projects(&context, ui),
                    "Dashboard" => self.show_dashboard(ui),
                    _ => {
                        ui.add_space(18.0);
                        ui.group(|ui| {
                            ui.heading(&self.active_page);
                            ui.label("This native Rust workspace panel is being brought online after the content browser and model viewer.");
                            ui.label(RichText::new("The navigation layout is in place and can be collapsed from the left rail.").color(MUTED));
                        });
                    }
                }
            });
    }
}

fn show_source_folder_row(ui: &mut egui::Ui, label: &str, path: &mut String) {
    ui.horizontal(|ui| {
        ui.add_sized([100.0, 24.0], egui::Label::new(RichText::new(label).strong()));
        ui.add(egui::TextEdit::singleline(path)
            .desired_width((ui.available_width() - 104.0).max(120.0))
            .hint_text("Choose a folder"));
        if ui.button("Browse…").clicked() {
            let mut dialog = rfd::FileDialog::new().set_title(format!("Select {label} folder"));
            if Path::new(path).is_dir() {
                dialog = dialog.set_directory(path.as_str());
            }
            if let Some(folder) = dialog.pick_folder() {
                *path = folder.to_string_lossy().to_string();
            }
        }
        if ui.button("Clear").clicked() {
            path.clear();
        }
    });
}

fn sprite_preview_available(entry: &BrowserEntry) -> bool {
    !matches!(entry.category.as_str(), "Blocks" | "Mobs" | "Models")
        && entry.model_source.is_none()
        && entry.resource_model_source.is_none()
        && (entry.icon_path.is_some() || !entry.texture_paths.is_empty())
}

fn load_minecraftia_font(context: &egui::Context) -> bool {
    let executable_directory = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let mut paths = vec![
        PathBuf::from("assets/fonts/Minecraftia-Regular.ttf"),
        PathBuf::from("Minecraftia-Regular.ttf"),
    ];
    if let Some(directory) = executable_directory {
        paths.insert(0, directory.join("assets/fonts/Minecraftia-Regular.ttf"));
        paths.insert(1, directory.join("Minecraftia-Regular.ttf"));
    }
    let Some(bytes) = paths.iter().find_map(|path| std::fs::read(path).ok()) else {
        return false;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "minecraftia".to_owned(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    fonts.families.entry(FontFamily::Name("minecraftia".into()))
        .or_default().push("minecraftia".to_owned());
    context.set_fonts(fonts);
    true
}

#[cfg(test)]
mod texture_animation_tests {
    use super::*;

    #[test]
    fn parses_mcmeta_frame_order_and_per_frame_durations() {
        let metadata = serde_json::json!({
            "animation": {
                "width": 16,
                "height": 16,
                "frametime": 2,
                "frames": [0, {"index": 2, "time": 4}]
            }
        });
        let animation = parse_animated_texture((32, 48), &metadata).unwrap();
        assert_eq!(animation.columns, 2);
        assert_eq!(animation.frame_width, 16);
        assert_eq!(animation.frame_height, 16);
        assert_eq!(animation.frames.len(), 2);
        assert_eq!(animation.frame_at(0.05), 0);
        assert_eq!(animation.frame_at(0.15), 1);
        assert_eq!(animation.frame_at(0.31), 0);
    }

    #[test]
    fn crops_vertical_and_multi_column_animation_frames() {
        let mut image = RgbaImage::new(32, 48);
        image.put_pixel(0, 16, image::Rgba([10, 20, 30, 255]));
        image.put_pixel(16, 0, image::Rgba([40, 50, 60, 255]));
        let metadata = serde_json::json!({
            "animation": {"width": 16, "height": 16, "frames": [2, 1]}
        });
        let animation = parse_animated_texture((32, 48), &metadata).unwrap();
        let frame = animation.crop(&image, 0);
        assert_eq!(frame.dimensions(), (16, 16));
        assert_eq!(frame.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(animation.crop(&image, 1).get_pixel(0, 0).0, [40, 50, 60, 255]);
    }
}

#[cfg(test)]
mod armor_texture_selection_tests {
    use super::*;

    #[test]
    fn animated_texture_is_preferred_for_multi_texture_blocks() {
        let temp = tempfile::tempdir().unwrap();
        let static_texture = temp.path().join("stone.png");
        let animated_texture = temp.path().join("water.png");
        fs::write(&static_texture, b"png").unwrap();
        fs::write(&animated_texture, b"png").unwrap();
        fs::write(
            PathBuf::from(format!("{}.mcmeta", animated_texture.display())),
            r#"{"animation":{"frametime":2}}"#,
        ).unwrap();
        let textures = vec![static_texture, animated_texture];
        let entry = BrowserEntry {
            category: "Blocks".into(),
            name: "Animated Water".into(),
            detail: "animated_water".into(),
            icon_kind: IconKind::Texture,
            icon_path: None,
            model_source: None,
            resource_model_source: None,
            resource_model_rotation: [0, 0],
            additional_resource_model_sources: Vec::new(),
            additional_resource_model_rotations: Vec::new(),
            texture_paths: textures.clone(),
            render_shape: Some("custom".into()),
            lore: Vec::new(),
            components_json: "{}".into(),
            metadata_json: "{}".into(),
        };
        assert_eq!(preferred_texture_index(&entry, &textures), 1);
    }

    #[test]
    fn leggings_select_layer_two_while_other_armor_selects_layer_one() {
        let temp = tempfile::tempdir().unwrap();
        let layer_one = temp.path().join("dragon_layer_1.png");
        let layer_two = temp.path().join("dragon_layer_2.png");
        fs::write(&layer_one, b"png").unwrap();
        fs::write(&layer_two, b"png").unwrap();
        fs::write(
            PathBuf::from(format!("{}.mcmeta", layer_one.display())),
            r#"{"animation":{"frametime":2}}"#,
        ).unwrap();
        let textures = vec![layer_one.clone(), layer_two.clone()];

        let leggings = BrowserEntry {
            category: "Armor".into(),
            name: "Dragon Leggings".into(),
            detail: "dragon_leggings · dragon armor set".into(),
            icon_kind: IconKind::Texture,
            icon_path: Some(layer_one.clone()),
            model_source: None,
            resource_model_source: None,
            resource_model_rotation: [0, 0],
            additional_resource_model_sources: Vec::new(),
            additional_resource_model_rotations: Vec::new(),
            texture_paths: textures.clone(),
            render_shape: None,
            lore: Vec::new(),
            components_json: "{}".into(),
            metadata_json: "{}".into(),
        };
        assert_eq!(preferred_texture_index(&leggings, &textures), 1);

        let chestplate = BrowserEntry {
            name: "Dragon Chestplate".into(),
            detail: "dragon_chestplate · dragon armor set".into(),
            ..leggings
        };
        assert_eq!(preferred_texture_index(&chestplate, &textures), 0);
    }
}
