//! Embedding PoC workload (docs/13 §2.4): a dock-like layout, a scrolling timeline canvas with
//! thousands of shapes, a 60 fps animated texture, and an IME-capable text field — plus frame
//! pacing statistics reported over the embed channel.

use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FrameStats {
    pub frames: u64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Frames whose interval exceeded 1.5× the median (visible hitches).
    pub hitches: u64,
    pub embedded: bool,
    pub width: u32,
    pub height: u32,
}

pub struct Workload {
    started: Instant,
    last: Option<Instant>,
    intervals_ms: Vec<f64>,
    pub text: String,
    texture: Option<egui::TextureHandle>,
    pixels: Vec<egui::Color32>,
}

impl std::fmt::Debug for Workload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workload")
            .field("frames", &self.intervals_ms.len())
            .finish()
    }
}

const TEX: usize = 256;

impl Workload {
    pub fn new() -> Self {
        Workload {
            started: Instant::now(),
            last: None,
            intervals_ms: Vec::with_capacity(4096),
            text: String::from("IME test: 输入中文"),
            texture: None,
            pixels: vec![egui::Color32::BLACK; TEX * TEX],
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        if let Some(l) = self.last {
            self.intervals_ms.push((now - l).as_secs_f64() * 1000.0);
        }
        self.last = Some(now);
    }

    pub fn stats(&self, embedded: bool, size: (u32, u32)) -> FrameStats {
        // skip the first 30 intervals (shader compile, swapchain warm-up)
        let mut v: Vec<f64> = self.intervals_ms.iter().skip(30).copied().collect();
        if v.is_empty() {
            return FrameStats {
                embedded,
                width: size.0,
                height: size.1,
                ..Default::default()
            };
        }
        v.sort_by(|a, b| a.total_cmp(b));
        let pct = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        let median = pct(0.5);
        FrameStats {
            frames: v.len() as u64,
            mean_ms: v.iter().sum::<f64>() / v.len() as f64,
            p50_ms: median,
            p95_ms: pct(0.95),
            p99_ms: pct(0.99),
            max_ms: *v.last().unwrap_or(&0.0),
            hitches: v.iter().filter(|x| **x > median * 1.5).count() as u64,
            embedded,
            width: size.0,
            height: size.1,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, title: &str, live: &str) {
        let t = self.started.elapsed().as_secs_f32();
        egui::Panel::top("poc_top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(title);
                ui.separator();
                ui.label(live);
            });
        });
        egui::Panel::left("poc_bin")
            .resizable(true)
            .default_size(180.0)
            .show(ui, |ui| {
                ui.heading("Media");
                for i in 0..12 {
                    ui.label(format!("clip_{i:02}.mov"));
                }
                ui.separator();
                ui.label("Text (IME):");
                ui.text_edit_singleline(&mut self.text);
            });
        egui::Panel::bottom("poc_timeline")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));
                let tracks = 8;
                let th = rect.height() / tracks as f32;
                let scroll = (t * 120.0) % 400.0;
                // ~2,000 clip rectangles + labels, as in the timeline budget (docs/08 §2)
                for tr in 0..tracks {
                    let y = rect.top() + tr as f32 * th;
                    for i in 0..250 {
                        let x = rect.left() + i as f32 * 40.0 - scroll;
                        if x > rect.right() || x + 36.0 < rect.left() {
                            continue;
                        }
                        let r = egui::Rect::from_min_size(
                            egui::pos2(x, y + 2.0),
                            egui::vec2(36.0, th - 4.0),
                        );
                        let c = egui::Color32::from_rgb(
                            60 + (tr * 20) as u8,
                            90,
                            140 + (i % 5 * 20) as u8,
                        );
                        painter.rect_filled(r, 3.0, c);
                        if i % 4 == 0 {
                            painter.text(
                                r.left_top() + egui::vec2(3.0, 2.0),
                                egui::Align2::LEFT_TOP,
                                format!("{i}"),
                                egui::FontId::monospace(9.0),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                }
                let ph = rect.left() + rect.width() * 0.4;
                painter.line_segment(
                    [egui::pos2(ph, rect.top()), egui::pos2(ph, rect.bottom())],
                    egui::Stroke::new(2.0, egui::Color32::RED),
                );
            });
        egui::CentralPanel::default().show(ui, |ui| {
            // animated texture standing in for decoded video frames
            let phase = (t * 60.0) as usize;
            for y in 0..TEX {
                for x in 0..TEX {
                    let v = ((x + phase) ^ y) as u8;
                    self.pixels[y * TEX + x] =
                        egui::Color32::from_rgb(v, v.wrapping_mul(3), 255 - v);
                }
            }
            let img = egui::ColorImage::new([TEX, TEX], self.pixels.clone());
            let tex = match &mut self.texture {
                Some(t) => {
                    t.set(img, egui::TextureOptions::LINEAR);
                    t.clone()
                }
                None => {
                    let t = ui
                        .ctx()
                        .load_texture("poc_video", img, egui::TextureOptions::LINEAR);
                    self.texture = Some(t.clone());
                    t
                }
            };
            let size = ui.available_size();
            let side = size.x.min(size.y);
            ui.centered_and_justified(|ui| {
                ui.add(egui::Image::new((
                    tex.id(),
                    egui::vec2(side * 16.0 / 9.0, side).min(size),
                )));
            });
        });
    }
}

impl Default for Workload {
    fn default() -> Self {
        Self::new()
    }
}
