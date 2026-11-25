//! Viewer and transport panel (docs/specs/phase-c-plan.md C3).
//!
//! The first editor surface: a viewer showing composed frames, play/pause, frame stepping and a
//! timeline slider. Frames come from [`PreviewTransport`] (scheduler + composited-frame cache); the
//! app only uploads the newest frame as an egui texture and draws it. Editing commands land with the
//! timeline canvas (C4); this panel reads the saved project snapshot.

use arc_cli::preview::{PreviewEngine, PreviewQuality};
use arc_cli::preview_scheduler::{PreviewPolicy, PreviewScheduler};
use arc_cli::preview_transport::PreviewTransport;
use arc_core::model::Project;
use arc_core::time::Ratio;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Fit `frame` into `available` keeping the aspect ratio and never upscaling beyond the frame.
pub fn fit_size(frame: egui::Vec2, available: egui::Vec2) -> egui::Vec2 {
    if frame.x <= 0.0 || frame.y <= 0.0 {
        return egui::Vec2::ZERO;
    }
    let fit = (available.x / frame.x).min(available.y / frame.y);
    let scale = fit.clamp(0.0, 1.0);
    frame * scale
}

/// `m:ss.ff`-style label for a frame index at `fps`.
pub fn timecode(frame: i64, fps: f64) -> String {
    if fps <= 0.0 {
        return frame.to_string();
    }
    let total = frame.max(0) as f64 / fps;
    let minutes = (total / 60.0).floor() as i64;
    let seconds = total - minutes as f64 * 60.0;
    format!("{minutes}:{seconds:05.2}")
}

pub struct Editor {
    media_root: PathBuf,
    project: Option<Project>,
    rate: Ratio,
    total_frames: i64,
    transport: Option<PreviewTransport>,
    texture: Option<egui::TextureHandle>,
    texture_serial: u64,
    error: Option<String>,
    last_tick: Instant,
    name: String,
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("name", &self.name)
            .field("total_frames", &self.total_frames)
            .field("attached", &self.transport.is_some())
            .field("error", &self.error)
            .finish()
    }
}

impl Editor {
    /// Load a saved project (read-only snapshot for this slice) and prepare the viewer.
    pub fn open(path: &Path) -> Editor {
        let mut editor = Editor {
            media_root: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            project: None,
            rate: Ratio::ONE,
            total_frames: 1,
            transport: None,
            texture: None,
            texture_serial: 0,
            error: None,
            last_tick: Instant::now(),
            name: path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        };
        match arc_core::store::load_container(path) {
            Ok((project, _meta)) => {
                let Some(sequence) = project.sequence() else {
                    editor.error = Some("project has no sequence".into());
                    return editor;
                };
                editor.rate = sequence.frame_rate().rate();
                editor.total_frames = arc_render::plan::frame_count(&project).max(1);
                editor.name = project.meta.name.clone();
                editor.project = Some(project);
            }
            Err(e) => {
                editor.error = Some(format!("cannot open {}: {e}", path.display()));
            }
        }
        editor
    }


    /// Create the preview pipeline on first use (needs a GPU adapter).
    fn attach(&mut self) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let engine = match PreviewEngine::new(
            project,
            self.media_root.clone(),
            PreviewQuality::Full,
        ) {
            Ok(engine) => engine,
            Err(e) => {
                self.error = Some(format!("preview unavailable: {e}"));
                return;
            }
        };
        let scheduler = PreviewScheduler::new(engine, PreviewPolicy::default());
        self.transport = Some(PreviewTransport::new(
            scheduler,
            self.total_frames,
            self.rate,
        ));
    }

    /// Draw the viewer and transport bar. Called once per redraw from the app loop.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;

        if self.transport.is_none() && self.error.is_none() {
            self.attach();
        }
        if let Some(error) = &self.error {
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            });
            return;
        }
        let Some(transport) = self.transport.as_mut() else {
            return;
        };

        transport.advance(elapsed);
        transport.poll();
        if transport.is_playing() {
            ui.ctx().request_repaint();
        }

        // Upload the newest composed frame when the viewer serial moved.
        if transport.serial() != self.texture_serial
            && let Some(frame) = transport.frame()
        {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.rgba,
            );
            self.texture = Some(ui.ctx().load_texture(
                "preview-frame",
                image,
                egui::TextureOptions::LINEAR,
            ));
            self.texture_serial = transport.serial();
        }

        let fps = self.rate.to_f64_lossy();
        let available = ui.available_size();
        let bar_height = 34.0;
        let viewer_size = egui::vec2(available.x, (available.y - bar_height).max(40.0));
        let texture = self.texture.clone();
        ui.allocate_ui(viewer_size, |ui| {
            match &texture {
                Some(texture) => {
                    let size = fit_size(texture.size_vec2(), ui.available_size());
                    ui.centered_and_justified(|ui| {
                        ui.image(egui::load::SizedTexture::new(texture.id(), size));
                    });
                }
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.label("composing the first frame…");
                    });
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&self.name).strong());
            let playing = transport.is_playing();
            if ui
                .button(if playing { "Pause" } else { "Play" })
                .clicked()
            {
                transport.toggle();
            }
            if ui.button("|<").clicked() {
                transport.seek(0);
            }
            if ui.button("<").clicked() {
                transport.step(-1);
            }
            if ui.button(">").clicked() {
                transport.step(1);
            }
            let mut frame = transport.playhead();
            let slider = ui.add(
                egui::Slider::new(&mut frame, 0..=(transport.total_frames() - 1).max(0))
                    .show_value(false),
            );
            if slider.changed() {
                transport.seek(frame);
            }
            ui.label(format!(
                "{}  {:>5} / {}  {}",
                timecode(transport.playhead(), fps),
                transport.playhead(),
                transport.total_frames(),
                if playing { "playing" } else { "paused" }
            ));
            if ui.button("Full").clicked() {
                transport.seek(transport.playhead());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_size_keeps_the_aspect_ratio_and_never_upscales() {
        let frame = egui::vec2(1920.0, 1080.0);
        let fits = fit_size(frame, egui::vec2(960.0, 1080.0));
        assert!((fits.x - 960.0).abs() < 0.01);
        assert!((fits.y - 540.0).abs() < 0.01);
        assert_eq!(fit_size(frame, egui::vec2(4000.0, 3000.0)), frame);
        assert_eq!(fit_size(egui::Vec2::ZERO, egui::vec2(100.0, 100.0)), egui::Vec2::ZERO);
    }

    #[test]
    fn timecode_formats_minutes_and_seconds() {
        assert_eq!(timecode(0, 30.0), "0:00.00");
        assert_eq!(timecode(45, 30.0), "0:01.50");
        assert_eq!(timecode(1800, 30.0), "1:00.00");
        assert_eq!(timecode(10, 0.0), "10");
    }
}
