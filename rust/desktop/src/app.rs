use crate::browser::{self, BrowserEntry, BrowserIndex, IconKind};
use crate::renderer::{self, Camera};
use crate::tooltip;
use eframe::egui::{self, Color32, FontFamily, RichText, TextureHandle, TextureOptions};
use image::RgbaImage;
use mojmodelbridge::pipeline::load_model_preview;
use mojmodelbridge::viewer::ViewerScene;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

pub struct MojModelBridgeApp {
    active_page: String,
    sidebar_collapsed: bool,
    root_path: String,
    scan_status: String,
    browser: BrowserIndex,
    category: String,
    item_search: String,
    selected_entry: Option<usize>,
    selected_error: String,

    scene: Option<ViewerScene>,
    texture_paths: Vec<PathBuf>,
    selected_texture: usize,
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
    minecraftia_available: bool,
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

        Self {
            active_page: "Dashboard".to_owned(),
            sidebar_collapsed: false,
            root_path: String::new(),
            scan_status: "No mod loaded".to_owned(),
            browser: BrowserIndex::default(),
            category: "Mobs".to_owned(),
            item_search: String::new(),
            selected_entry: None,
            selected_error: String::new(),
            scene: None,
            texture_paths: Vec::new(),
            selected_texture: 0,
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
            minecraftia_available,
        }
    }

    fn scan(&mut self, context: &egui::Context) {
        let root = PathBuf::from(self.root_path.trim());
        match browser::scan_mod(&root) {
            Ok(index) => {
                let display = index.display_name.clone();
                let count = index.entries.len();
                self.browser = index;
                self.active_page = "Content Browser".to_owned();
                self.scan_status = format!("Indexed {count} assets · {display}");
                self.selected_entry = None;
                self.scene = None;
                self.texture_paths.clear();
                self.preview_image = None;
                self.preview_texture = None;
                self.selected_error.clear();
                self.category = browser::CATEGORIES.iter()
                    .copied()
                    .find(|category| !self.browser.in_category(category).is_empty())
                    .unwrap_or("Mobs").to_owned();
                if let Some(first) = self.browser.in_category(&self.category).first().copied() {
                    self.activate_entry(context, first);
                }
            }
            Err(error) => {
                self.scan_status = "Scan failed".to_owned();
                self.selected_error = format!("{error:#}");
            }
        }
    }

    fn activate_entry(&mut self, context: &egui::Context, index: usize) {
        let Some(entry) = self.browser.entries.get(index).cloned() else { return; };
        self.selected_entry = Some(index);
        self.draft_name = entry.name.clone();
        self.draft_lore = entry.lore.join("\n");
        self.components_json = entry.components_json.clone();
        self.scene = None;
        self.texture_paths.clear();
        self.selected_texture = 0;
        self.preview_image = None;
        self.texture_animation = None;
        self.camera.reset();
        self.selected_animation = 0;
        self.animation_time = 0.0;
        self.playing = false;
        self.selected_error.clear();

        if let Some(source) = entry.model_source.as_ref() {
            match load_model_preview(source, &self.browser.texture_roots) {
                Ok(asset) => match ViewerScene::from_parsed_model(&asset.model) {
                    Ok(scene) => {
                        if entry.category == "Armor" && !entry.texture_paths.is_empty() {
                            self.texture_paths = entry.texture_paths.clone();
                            for path in &asset.texture_paths {
                                if !self.texture_paths.contains(path) {
                                    self.texture_paths.push(path.clone());
                                }
                            }
                        } else {
                            self.texture_paths = asset.texture_paths;
                            if self.texture_paths.is_empty() {
                                self.texture_paths = entry.texture_paths.clone();
                            }
                        }
                        self.selected_texture = if entry.category == "Armor" {
                            0
                        } else {
                            self.texture_paths.iter()
                                .position(|path| path.to_string_lossy() == asset.model.primary_texture)
                                .unwrap_or(0)
                        };
                        self.scene = Some(scene);
                        self.refresh_preview_image();
                    }
                    Err(error) => self.selected_error = format!("Could not create model scene: {error:#}"),
                },
                Err(error) => self.selected_error = format!("Model preview could not be loaded: {error:#}"),
            }
        } else if let Some(source) = entry.resource_model_source.as_ref() {
            match crate::resource_model::load_item_model(source, &self.browser.texture_roots) {
                Ok((scene, textures)) => {
                    self.scene = Some(scene);
                    self.texture_paths = if textures.is_empty() {
                        entry.texture_paths.clone()
                    } else {
                        textures
                    };
                    self.selected_texture = 0;
                    self.refresh_preview_image();
                }
                Err(error) => {
                    self.selected_error = format!("Resource model preview could not be loaded: {error:#}");
                    if let Some(path) = entry.icon_path.as_ref() {
                        self.texture_paths.push(path.clone());
                        self.refresh_preview_image();
                    }
                }
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
        self.preview_image = self.texture_paths.get(self.selected_texture)
            .and_then(|path| image::open(path).ok())
            .map(|image| image.to_rgba8());

        let Some(path) = self.texture_paths.get(self.selected_texture) else { return; };
        let Some(image) = self.preview_image.as_ref() else { return; };
        let metadata_path = PathBuf::from(format!("{}.mcmeta", path.display()));
        let Ok(bytes) = fs::read(&metadata_path) else { return; };
        let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return; };
        let Some(animation) = metadata.get("animation").and_then(serde_json::Value::as_object) else { return; };

        let width = animation.get("width").and_then(serde_json::Value::as_u64)
            .map(|value| value as u32).unwrap_or(image.width());
        let height = animation.get("height").and_then(serde_json::Value::as_u64)
            .map(|value| value as u32).unwrap_or(width);
        if width == 0 || height == 0 || width > image.width() || height > image.height() {
            return;
        }
        let columns = image.width() / width;
        let rows = image.height() / height;
        if columns == 0 || rows == 0 { return; }
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
                (index < frame_count).then_some(AnimatedTextureFrame { index, duration_seconds: duration })
            }).collect::<Vec<_>>()
        } else {
            (0..frame_count).map(|index| AnimatedTextureFrame {
                index,
                duration_seconds: default_duration,
            }).collect()
        };
        if frames.is_empty() { return; }

        let mut animated = AnimatedTexture {
            frame_width: width,
            frame_height: height,
            columns,
            frames,
            elapsed_seconds: 0.0,
            current_frame: 0,
        };
        self.preview_image = Some(animated.crop(image, 0));
        animated.current_frame = animated.frame_at(0.0);
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
                        ui.label(RichText::new("oldVer retained as reference").size(9.0).color(MUTED));
                    } else {
                        ui.label(RichText::new("●").color(Color32::from_rgb(105, 207, 151)));
                    }
                });
            });
    }

    fn show_header(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(self.active_page.as_str());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Scan directory").clicked() {
                    self.scan(context);
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.root_path)
                        .desired_width(300.0)
                        .hint_text("Extracted mod/project directory"),
                );
                ui.label(RichText::new("PROJECT").size(9.0).color(MUTED));
            });
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
                |ui| self.show_detail(context, ui),
            );
        });
    }

    fn show_categories(&mut self, ui: &mut egui::Ui) {
        ui.heading("Categories");
        ui.label(RichText::new(format!("{} indexed", self.browser.entries.len())).size(10.0).color(MUTED));
        ui.add_space(7.0);
        egui::ScrollArea::vertical()
            .max_height(ui.available_height().max(48.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
            for category in browser::CATEGORIES {
                let count = self.browser.in_category(category).len();
                let label = format!("{category}  ({count})");
                if ui.selectable_label(self.category == *category, label).clicked() {
                    self.category = (*category).to_owned();
                    self.selected_entry = None;
                    self.scene = None;
                    self.texture_paths.clear();
                    self.preview_image = None;
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(if self.expanded_viewer { "Collapse" } else { "Expand" }).clicked() {
                        self.expanded_viewer = !self.expanded_viewer;
                    }
                    if ui.button("Reset camera").clicked() {
                        self.camera.reset();
                    }
                });
            });
            self.show_viewer_controls(context, ui);
            self.show_viewport(context, ui, &entry, preview_height);
        });

        if !self.expanded_viewer {
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height((available.y - preview_height - 20.0).max(180.0)).show(ui, |ui| {
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
        if response.hovered() {
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
            let rendered = renderer::render_scene(
                scene,
                &pose,
                self.preview_image.as_ref(),
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
        ui.label(RichText::new("Drag to orbit · right-drag to pan · scroll to zoom").size(9.0).color(MUTED));
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
            }
        } else if let Some(path) = self.texture_paths.first() {
            ui.label(RichText::new(path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string())).size(10.0).color(MUTED));
        }
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut self.draft_name).desired_width(f32::INFINITY));
        });
        ui.label("Lore");
        ui.add(
            egui::TextEdit::multiline(&mut self.draft_lore)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("One lore line per row. Use § color/style codes."),
        );
        ui.label("Components (Minecraft component JSON)");
        ui.add(
            egui::TextEdit::multiline(&mut self.components_json)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text(r#"{"minecraft:custom_name":{"text":"Legendary Sword","color":"gold","bold":true},"minecraft:lore":[{"text":"Forged in the deep","color":"gray","italic":true}]}"#),
        );

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

impl eframe::App for MojModelBridgeApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
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
        if let Some(animation) = self.texture_animation.as_mut() {
            animation.elapsed_seconds += delta;
            let next = animation.frame_at(animation.elapsed_seconds);
            if next != animation.current_frame {
                animation.current_frame = next;
                if let Some(path) = self.texture_paths.get(self.selected_texture) {
                    if let Ok(image) = image::open(path) {
                        self.preview_image = Some(animation.crop(&image.to_rgba8(), next));
                    }
                }
            }
            context.request_repaint_after(Duration::from_millis(50));
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
