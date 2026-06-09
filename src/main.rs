//! linkshot — an unofficial, Lightshot-style screenshot tool.
//!
//! On launch it grabs the whole screen, then opens a borderless full-screen overlay:
//! drag to select a region, a floating toolbar appears next to it (pen / line / arrow
//! / rect, colors, upload / save / copy), and the upload link lands on your clipboard.
//! Bind it to a hotkey (e.g. PrintScreen) so it behaves like Lightshot.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod config;
mod editor;
mod imgur;

use config::Config;
use editor::{Annot, Shape, Tool, View};
use egui_phosphor::regular as icon;
use image::RgbaImage;
use std::sync::mpsc::{Receiver, TryRecvError};

fn main() -> eframe::Result<()> {
    let cfg = Config::load();
    let capture = capture::capture_fullscreen();

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_fullscreen(true)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_title("linkshot"),
        ..Default::default()
    };
    eframe::run_native(
        "linkshot",
        opts,
        Box::new(move |cc| {
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(App::new(cc, cfg, capture)))
        }),
    )
}

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Selecting,
    Editing,
}

const PALETTE: [[u8; 4]; 6] = [
    [237, 28, 36, 255],
    [255, 201, 14, 255],
    [34, 177, 76, 255],
    [0, 162, 232, 255],
    [255, 255, 255, 255],
    [0, 0, 0, 255],
];

struct App {
    cfg: Config,
    full_img: Option<RgbaImage>,
    tex: Option<egui::TextureHandle>,
    error: Option<String>,

    view_rect: egui::Rect,
    mode: Mode,
    sel: Option<egui::Rect>,
    sel_start: Option<egui::Pos2>,

    annots: Vec<Annot>,
    in_progress: Option<Annot>,
    tool: Tool,
    color: [u8; 4],
    width: f32,

    upload_rx: Option<Receiver<anyhow::Result<imgur::Uploaded>>>,
    uploading: bool,
    status: String,
    result_link: Option<String>,

    show_settings: bool,
    client_id_input: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, cfg: Config, cap: anyhow::Result<RgbaImage>) -> Self {
        let client_id_input = cfg.imgur_client_id.clone().unwrap_or_default();
        let (full_img, tex, error) = match cap {
            Ok(img) => {
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                let tex = cc.egui_ctx.load_texture("screen", color, egui::TextureOptions::LINEAR);
                (Some(img), Some(tex), None)
            }
            Err(e) => (None, None, Some(e.to_string())),
        };
        Self {
            cfg,
            full_img,
            tex,
            error,
            view_rect: egui::Rect::ZERO,
            mode: Mode::Selecting,
            sel: None,
            sel_start: None,
            annots: Vec::new(),
            in_progress: None,
            tool: Tool::Arrow,
            color: PALETTE[0],
            width: 4.0,
            upload_rx: None,
            uploading: false,
            status: String::new(),
            result_link: None,
            show_settings: false,
            client_id_input,
        }
    }

    fn close(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Map a screen-logical point into cropped-physical image coordinates.
    fn map_point(&self, p: [f32; 2], sel: egui::Rect, sx: f32, sy: f32) -> [f32; 2] {
        [(p[0] - sel.min.x) * sx, (p[1] - sel.min.y) * sy]
    }

    /// Build the cropped image + annotations baked into physical coordinates, encode PNG.
    fn export_png(&self) -> anyhow::Result<Vec<u8>> {
        let img = self.full_img.as_ref().ok_or_else(|| anyhow::anyhow!("no image"))?;
        let sel = self.sel.ok_or_else(|| anyhow::anyhow!("no selection"))?;
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let sx = iw / self.view_rect.width().max(1.0);
        let sy = ih / self.view_rect.height().max(1.0);

        let x0 = ((sel.min.x - self.view_rect.min.x) * sx).max(0.0).round() as u32;
        let y0 = ((sel.min.y - self.view_rect.min.y) * sy).max(0.0).round() as u32;
        let w = (sel.width() * sx).round() as u32;
        let h = (sel.height() * sy).round() as u32;
        let w = w.min(img.width().saturating_sub(x0)).max(1);
        let h = h.min(img.height().saturating_sub(y0)).max(1);

        let cropped = image::imageops::crop_imm(img, x0, y0, w, h).to_image();

        let scale_w = (sx + sy) * 0.5;
        let mapped: Vec<Annot> = self
            .annots
            .iter()
            .map(|a| {
                let shape = match &a.shape {
                    Shape::Pen(pts) => {
                        Shape::Pen(pts.iter().map(|p| self.map_point(*p, sel, sx, sy)).collect())
                    }
                    Shape::Line(p, q) => {
                        Shape::Line(self.map_point(*p, sel, sx, sy), self.map_point(*q, sel, sx, sy))
                    }
                    Shape::Arrow(p, q) => {
                        Shape::Arrow(self.map_point(*p, sel, sx, sy), self.map_point(*q, sel, sx, sy))
                    }
                    Shape::Rect(p, q) => {
                        Shape::Rect(self.map_point(*p, sel, sx, sy), self.map_point(*q, sel, sx, sy))
                    }
                };
                Annot { shape, color: a.color, width: a.width * scale_w }
            })
            .collect();

        editor::render_png(&cropped, &mapped)
    }

    fn start_upload(&mut self) {
        let Some(cid) = self.cfg.client_id() else {
            self.show_settings = true;
            self.status = "Set an Imgur Client-ID first.".into();
            return;
        };
        let png = match self.export_png() {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Render error: {e}");
                return;
            }
        };
        self.uploading = true;
        self.status = "Uploading…".into();
        let (tx, rx) = std::sync::mpsc::channel();
        self.upload_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(imgur::upload_png(&cid, png));
        });
    }

    fn poll_upload(&mut self, ctx: &egui::Context) {
        let done = match self.upload_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(res)) => Some(res),
            Some(Err(TryRecvError::Disconnected)) => Some(Err(anyhow::anyhow!("upload thread died"))),
            _ => None,
        };
        if let Some(res) = done {
            self.upload_rx = None;
            self.uploading = false;
            match res {
                Ok(up) => {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(up.link.clone());
                    }
                    self.status = match &up.deletehash {
                        Some(h) => format!("Uploaded — link copied. Delete hash: {h}"),
                        None => "Uploaded — link copied to clipboard.".into(),
                    };
                    self.result_link = Some(up.link);
                }
                Err(e) => self.status = format!("Upload failed: {e}"),
            }
            ctx.request_repaint();
        }
    }

    fn save_to_file(&mut self) {
        let png = match self.export_png() {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Render error: {e}");
                return;
            }
        };
        let mut path = directories::UserDirs::new()
            .and_then(|d| d.picture_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(std::env::temp_dir);
        path.push(format!("linkshot-{}.png", std::process::id()));
        match std::fs::write(&path, png) {
            Ok(_) => self.status = format!("Saved to {}", path.display()),
            Err(e) => self.status = format!("Save error: {e}"),
        }
    }
}

impl eframe::App for App {
    // Transparent overlay needs a clear background.
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.upload_rx.is_some() {
            self.poll_upload(ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }

        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                App::close(ctx);
            }
            if i.modifiers.command && i.key_pressed(egui::Key::Z) {
                self.annots.pop();
            }
        });

        if self.error.is_some() {
            self.error_ui(ctx);
            return;
        }

        self.overlay_ui(ctx);
        if self.mode == Mode::Editing {
            self.toolbar_ui(ctx);
        }
        if self.show_settings {
            self.settings_ui(ctx);
        }
        self.hint_ui(ctx);
    }
}

impl App {
    fn error_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Capture failed");
                    if let Some(e) = &self.error {
                        ui.label(e);
                    }
                    if ui.button(format!("{} Close", icon::X)).clicked() {
                        App::close(ctx);
                    }
                });
            });
        });
    }

    fn overlay_ui(&mut self, ctx: &egui::Context) {
        let frame = egui::Frame::none().fill(egui::Color32::TRANSPARENT);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let screen = ctx.screen_rect();
            self.view_rect = screen;
            let Some(tex) = self.tex.clone() else { return };

            let (_id, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
            let painter = ui.painter_at(screen);
            let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

            // Frozen screenshot, dimmed.
            painter.image(tex.id(), screen, full_uv, egui::Color32::WHITE);
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(130));

            // Bright, un-dimmed selection.
            if let Some(sel) = self.sel {
                let uv = egui::Rect::from_min_max(
                    egui::pos2(
                        (sel.min.x - screen.min.x) / screen.width(),
                        (sel.min.y - screen.min.y) / screen.height(),
                    ),
                    egui::pos2(
                        (sel.max.x - screen.min.x) / screen.width(),
                        (sel.max.y - screen.min.y) / screen.height(),
                    ),
                );
                painter.image(tex.id(), sel, uv, egui::Color32::WHITE);
                painter.rect_stroke(sel, 0.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(0, 162, 232)));
                let dim = format!("{} × {}", sel.width().round(), sel.height().round());
                painter.text(
                    sel.min + egui::vec2(2.0, -4.0),
                    egui::Align2::LEFT_BOTTOM,
                    dim,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }

            self.handle_pointer(ctx, &response, screen);

            // Draw annotations (coords are already in screen space).
            let view = View { origin: egui::Pos2::ZERO, scale: 1.0 };
            let clip = self.sel.unwrap_or(screen);
            let ann_painter = painter.with_clip_rect(clip);
            for a in &self.annots {
                editor::draw_egui(&ann_painter, a, &view);
            }
            if let Some(a) = &self.in_progress {
                editor::draw_egui(&ann_painter, a, &view);
            }
        });
    }

    fn handle_pointer(&mut self, ctx: &egui::Context, response: &egui::Response, screen: egui::Rect) {
        match self.mode {
            Mode::Selecting => {
                if response.drag_started() {
                    self.sel_start = response.interact_pointer_pos();
                }
                if response.dragged() {
                    if let (Some(s), Some(p)) = (self.sel_start, response.interact_pointer_pos()) {
                        self.sel = Some(egui::Rect::from_two_pos(s, p).intersect(screen));
                    }
                }
                if response.drag_stopped() {
                    match self.sel {
                        Some(r) if r.width() > 8.0 && r.height() > 8.0 => self.mode = Mode::Editing,
                        _ => {
                            self.sel = None;
                        }
                    }
                }
            }
            Mode::Editing => {
                let sel = self.sel.unwrap_or(screen);
                let inside = |p: Option<egui::Pos2>| p.map(|p| sel.contains(p)).unwrap_or(false);
                let clamp = |p: egui::Pos2| -> [f32; 2] {
                    [p.x.clamp(sel.min.x, sel.max.x), p.y.clamp(sel.min.y, sel.max.y)]
                };

                if response.drag_started()
                    && inside(response.interact_pointer_pos())
                    && !ctx.is_pointer_over_area()
                {
                    if let Some(p) = response.interact_pointer_pos() {
                        let s = clamp(p);
                        let shape = match self.tool {
                            Tool::Pen => Shape::Pen(vec![s]),
                            Tool::Line => Shape::Line(s, s),
                            Tool::Arrow => Shape::Arrow(s, s),
                            Tool::Rect => Shape::Rect(s, s),
                        };
                        self.in_progress = Some(Annot { shape, color: self.color, width: self.width });
                    }
                }
                if response.dragged() {
                    if let (Some(p), Some(ann)) =
                        (response.interact_pointer_pos(), self.in_progress.as_mut())
                    {
                        let cur = clamp(p);
                        match &mut ann.shape {
                            Shape::Pen(pts) => pts.push(cur),
                            Shape::Line(_, q) | Shape::Arrow(_, q) | Shape::Rect(_, q) => *q = cur,
                        }
                    }
                }
                if response.drag_stopped() {
                    if let Some(ann) = self.in_progress.take() {
                        self.annots.push(ann);
                    }
                }
            }
        }
    }

    fn toolbar_ui(&mut self, ctx: &egui::Context) {
        let Some(sel) = self.sel else { return };
        let screen = ctx.screen_rect();
        // Place the bar just below the selection, or above if there's no room.
        let bar_h = 46.0;
        let mut pos = egui::pos2(sel.min.x, sel.max.y + 8.0);
        if pos.y + bar_h > screen.max.y {
            pos.y = (sel.min.y - bar_h - 8.0).max(screen.min.y + 4.0);
        }
        pos.x = pos.x.min(screen.max.x - 360.0).max(screen.min.x + 4.0);

        egui::Area::new(egui::Id::new("toolbar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let tools = [
                                (Tool::Pen, icon::PENCIL_SIMPLE),
                                (Tool::Line, icon::LINE_SEGMENT),
                                (Tool::Arrow, icon::ARROW_UP_RIGHT),
                                (Tool::Rect, icon::RECTANGLE),
                            ];
                            for (t, ic) in tools {
                                if ui.selectable_label(self.tool == t, ic).clicked() {
                                    self.tool = t;
                                }
                            }
                            ui.separator();
                            for c in PALETTE {
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(18.0, 18.0),
                                    egui::Sense::click(),
                                );
                                ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                                if self.color == c {
                                    ui.painter().rect_stroke(
                                        rect,
                                        3.0,
                                        egui::Stroke::new(2.0, egui::Color32::GRAY),
                                    );
                                }
                                if resp.clicked() {
                                    self.color = c;
                                }
                            }
                            ui.separator();
                            ui.add(
                                egui::DragValue::new(&mut self.width)
                                    .range(1.0..=24.0)
                                    .speed(0.2)
                                    .prefix("w "),
                            );
                            ui.separator();

                            if ui.button(icon::ARROW_COUNTER_CLOCKWISE).on_hover_text("Undo").clicked() {
                                self.annots.pop();
                            }
                            if ui.button(icon::CROP).on_hover_text("Reselect").clicked() {
                                self.mode = Mode::Selecting;
                                self.sel = None;
                                self.annots.clear();
                                self.in_progress = None;
                            }

                            ui.add_enabled_ui(!self.uploading, |ui| {
                                if ui.button(icon::FLOPPY_DISK).on_hover_text("Save PNG").clicked() {
                                    self.save_to_file();
                                }
                                if ui
                                    .button(format!("{} Upload", icon::CLOUD_ARROW_UP))
                                    .on_hover_text("Upload to Imgur")
                                    .clicked()
                                {
                                    self.start_upload();
                                }
                            });

                            if self.result_link.is_some()
                                && ui.button(icon::COPY).on_hover_text("Copy link").clicked()
                            {
                                if let (Some(link), Ok(mut cb)) =
                                    (self.result_link.clone(), arboard::Clipboard::new())
                                {
                                    let _ = cb.set_text(link);
                                }
                            }
                            ui.separator();
                            if ui.button(icon::X).on_hover_text("Close (Esc)").clicked() {
                                App::close(ctx);
                            }
                        });
                    });
            });
    }

    fn hint_ui(&mut self, ctx: &egui::Context) {
        // Top-center status / hint line.
        let text = if !self.status.is_empty() {
            self.status.clone()
        } else if self.mode == Mode::Selecting {
            "Drag to select a region · Esc to cancel".to_string()
        } else {
            String::new()
        };
        if text.is_empty() && self.result_link.is_none() {
            return;
        }
        egui::Area::new(egui::Id::new("hint"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        if !text.is_empty() {
                            ui.label(text);
                        }
                        if let Some(link) = self.result_link.clone() {
                            if ui.link(&link).clicked() {
                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                    let _ = cb.set_text(link);
                                }
                            }
                        }
                    });
                });
            });
    }

    fn settings_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("Imgur settings")
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Anonymous uploads need a free Imgur Client-ID.");
                ui.horizontal(|ui| {
                    ui.label("Client-ID:");
                    ui.text_edit_singleline(&mut self.client_id_input);
                });
                ui.hyperlink_to("Get one here", "https://api.imgur.com/oauth2/addclient");
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.cfg.imgur_client_id = Some(self.client_id_input.trim().to_string());
                        match self.cfg.save() {
                            Ok(_) => self.status = "Client-ID saved.".into(),
                            Err(e) => self.status = format!("Save error: {e}"),
                        }
                        self.show_settings = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_settings = false;
                    }
                });
            });
    }
}
