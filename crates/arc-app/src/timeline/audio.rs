//! Arcline audio envelope: parameters, screen controls, hit zones and a local gesture history.
//!
//! The data type is `arc_core::model::ClipAudio`, the same struct consumed by `arc-audio`'s
//! mixer and `arc-cli`'s exporter. This module only maps it to the timeline.

use super::draw;
use super::interaction::{ClipGeometry, ClipRef, HitZone};
use arc_core::model::ClipAudio;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, Vec2};

pub const DB_MIN: f64 = -60.0;
pub const DB_MAX: f64 = 12.0;
/// Visual full-scale amplitude: +2 dB is the top of the audio region.
const AMP_FULL: f32 = 1.2589;

pub fn db_to_amp(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

/// Envelope multiplier at clip-local time `t` (seconds).
pub fn fade_gain(t: f64, duration: f64, fade_in: f64, fade_out: f64) -> f32 {
    let mut g = 1.0f32;
    if fade_in > 0.0 && t < fade_in {
        g *= (t / fade_in).clamp(0.0, 1.0) as f32;
    }
    let remaining = duration - t;
    if fade_out > 0.0 && remaining < fade_out {
        g *= (remaining / fade_out).clamp(0.0, 1.0) as f32;
    }
    g
}

pub fn time_secs(t: arc_core::time::Time) -> f64 {
    t.to_f64_lossy()
}

pub fn secs_to_time(secs: f64) -> arc_core::time::Time {
    arc_core::time::Time::new((secs * 1_000_000.0).round() as i64, 1_000_000)
        .unwrap_or(arc_core::time::Time::ZERO)
}

#[derive(Debug, Clone, Copy)]
pub struct AudioControls {
    pub region: Rect,
    pub baseline: f32,
    pub line_y: f32,
    pub fade_in_x: f32,
    pub fade_out_x: f32,
    pub volume_node: Rect,
    pub fade_in_node: Rect,
    pub fade_out_node: Rect,
    pub fade_in: f32,
    pub fade_out: f32,
}

pub fn controls(g: &ClipGeometry, audio: &ClipAudio, duration: f64) -> Option<AudioControls> {
    let region = g.audio.or(g.wave)?;
    if region.width() < 28.0 || region.height() < 12.0 || duration <= 0.0 {
        return None;
    }
    let baseline = region.bottom() - 1.0;
    let amp = db_to_amp(audio.gain_db).clamp(0.0, AMP_FULL);
    let usable = (region.height() - 4.0).max(1.0);
    let line_y = (baseline - amp / AMP_FULL * usable).clamp(region.top() + 3.0, baseline);
    let fade_in = time_secs(audio.fade_in).clamp(0.0, duration);
    let fade_out = time_secs(audio.fade_out).clamp(0.0, duration);
    let width = region.width();
    let fade_in_x = (region.left() + (fade_in / duration) as f32 * width)
        .clamp(region.left() + 8.0, region.right() - 8.0);
    let fade_out_x = (region.right() - (fade_out / duration) as f32 * width)
        .clamp(region.left() + 8.0, region.right() - 8.0);
    let node = |c: Pos2, r: f32| Rect::from_center_size(c, Vec2::splat(r));
    Some(AudioControls {
        region,
        baseline,
        line_y,
        fade_in_x,
        fade_out_x,
        volume_node: node(egui::pos2(region.center().x, line_y), 18.0),
        fade_in_node: node(egui::pos2(fade_in_x, line_y), 10.0),
        fade_out_node: node(egui::pos2(fade_out_x, line_y), 10.0),
        fade_in: fade_in as f32,
        fade_out: fade_out as f32,
    })
}

/// Hit test for the visible audio controls; called only when the pointer is in the audio region.
pub fn hit_test(c: &AudioControls, p: Pos2) -> Option<HitZone> {
    if c.fade_in_node.expand(2.0).contains(p) {
        return Some(HitZone::FadeIn);
    }
    if c.fade_out_node.expand(2.0).contains(p) {
        return Some(HitZone::FadeOut);
    }
    if c.volume_node.expand(2.0).contains(p) {
        return Some(HitZone::Volume);
    }
    if (p.y - c.line_y).abs() <= 4.0 && p.x >= c.fade_in_x - 4.0 && p.x <= c.fade_out_x + 4.0 {
        return Some(HitZone::Volume);
    }
    None
}

/// A `>` chevron rotated by 90°: tip down when the node is above the audio-region centre,
/// tip up when it is below. At dead centre it points down (a stable, non-flickering default).
pub fn volume_tip_down(c: &AudioControls) -> bool {
    c.volume_node.center().y <= c.region.center().y
}

/// Two rounded white strokes, no base, no shadow. This is a direct-on-blue control.
fn draw_volume_chevron(painter: &Painter, c: &AudioControls, active: bool) {
    let center = c.volume_node.center();
    let w = 8.0;
    let h = 7.0;
    let color = if active {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(235)
    };
    let (points, stroke) = if volume_tip_down(c) {
        (
            [
                egui::pos2(center.x - w, center.y - h),
                egui::pos2(center.x, center.y + h),
                egui::pos2(center.x + w, center.y - h),
            ],
            Stroke::new(if active { 2.4 } else { 2.0 }, color),
        )
    } else {
        (
            [
                egui::pos2(center.x - w, center.y + h),
                egui::pos2(center.x, center.y - h),
                egui::pos2(center.x + w, center.y + h),
            ],
            Stroke::new(if active { 2.4 } else { 2.0 }, color),
        )
    };
    // Two separate segments keep the tip crisp; small end caps keep the line soft rather than
    // square. The caps are part of the stroke, not a shadow or a base.
    painter.line_segment([points[0], points[1]], stroke);
    painter.line_segment([points[1], points[2]], stroke);
    for p in points {
        painter.circle_filled(p, stroke.width * 0.5, stroke.color);
    }
}

pub fn draw(
    painter: &Painter,
    c: &AudioControls,
    audio: &ClipAudio,
    hovered: Option<HitZone>,
    dragging: bool,
) {
    let left = c.region.left();
    let right = c.region.right();
    let y = c.line_y;
    let start_y = if c.fade_in > 0.0 { c.baseline } else { y };
    let end_y = if c.fade_out > 0.0 { c.baseline } else { y };

    // Fade regions: light fill under the two ramps, never clipping the waveform.
    let fill = Color32::from_rgba_unmultiplied(76, 141, 255, 18);
    if c.fade_in > 0.0 {
        painter.add(Shape::convex_polygon(
            vec![
                egui::pos2(left, c.baseline),
                egui::pos2(c.fade_in_x, y),
                egui::pos2(c.fade_in_x, c.baseline),
            ],
            fill,
            Stroke::NONE,
        ));
    }
    if c.fade_out > 0.0 {
        painter.add(Shape::convex_polygon(
            vec![
                egui::pos2(c.fade_out_x, y),
                egui::pos2(right, c.baseline),
                egui::pos2(c.fade_out_x, c.baseline),
            ],
            fill,
            Stroke::NONE,
        ));
    }

    let curve = vec![
        egui::pos2(left, start_y),
        egui::pos2(c.fade_in_x, y),
        egui::pos2(c.fade_out_x, y),
        egui::pos2(right, end_y),
    ];
    painter.add(Shape::line(
        curve.clone(),
        Stroke::new(3.5, Color32::from_black_alpha(105)),
    ));
    painter.add(Shape::line(curve, Stroke::new(1.8, draw::ACCENT)));

    // Fades: diamonds, visually distinct from the chevron volume node and trim bars.
    for (zone, rect) in [
        (HitZone::FadeIn, c.fade_in_node),
        (HitZone::FadeOut, c.fade_out_node),
    ] {
        let color = if hovered == Some(zone) {
            Color32::WHITE
        } else {
            draw::WAVE_EMBEDDED
        };
        let center = rect.center();
        let r = 5.0;
        painter.add(Shape::convex_polygon(
            vec![
                egui::pos2(center.x, center.y - r),
                egui::pos2(center.x + r, center.y),
                egui::pos2(center.x, center.y + r),
                egui::pos2(center.x - r, center.y),
            ],
            color,
            Stroke::new(1.0, Color32::WHITE),
        ));
    }

    // A compact live readout sits above the line; no permanent canvas chrome.
    let text = format!("{:.1} dB", audio.gain_db);
    let readout = Rect::from_min_size(
        egui::pos2(
            (c.region.right() - 52.0).max(c.region.left() + 2.0),
            (c.baseline - 15.0).max(c.region.top() + 1.0),
        ),
        Vec2::new(48.0, 13.0),
    );
    painter.rect_filled(
        readout,
        CornerRadius::same(2),
        Color32::from_black_alpha(160),
    );
    painter.text(
        readout.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::monospace(8.5),
        Color32::WHITE,
    );

    // White `>` chevron, rotated by the node's side. No base, fill, shadow or glow.
    draw_volume_chevron(painter, c, dragging || hovered == Some(HitZone::Volume));
}

#[derive(Debug, Clone, Copy)]
pub struct AudioEdit {
    pub r: ClipRef,
    pub before: ClipAudio,
    pub after: ClipAudio,
}

/// Active audio gesture. `start` is in screen points; `before` is restored on Escape.
#[derive(Debug, Clone, Copy)]
pub struct AudioDrag {
    pub r: ClipRef,
    pub zone: HitZone,
    pub before: ClipAudio,
    pub start: Pos2,
}

/// Small, local undo stack for inspector/timeline audio gestures. Project-level Ctrl+Z remains the
/// engine's history; this stack only owns in-progress mock-timeline audio edits.
#[derive(Debug, Clone, Default)]
pub struct AudioHistory {
    undo: Vec<AudioEdit>,
    redo: Vec<AudioEdit>,
}

impl AudioHistory {
    pub fn push(&mut self, edit: AudioEdit) {
        if edit.before == edit.after {
            return;
        }
        self.undo.push(edit);
        self.redo.clear();
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
    }

    pub fn undo(&mut self) -> Option<AudioEdit> {
        let edit = self.undo.pop()?;
        self.redo.push(edit);
        Some(edit)
    }

    pub fn redo(&mut self) -> Option<AudioEdit> {
        let edit = self.redo.pop()?;
        self.undo.push(edit);
        Some(edit)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::model::TrackKind;

    fn geometry() -> ClipGeometry {
        let rect = Rect::from_min_size(egui::pos2(100.0, 20.0), egui::vec2(300.0, 72.0));
        crate::timeline::interaction::clip_geometry(TrackKind::Audio, rect, false)
    }

    #[test]
    fn gain_mapping_and_fade_envelope() {
        assert!((db_to_amp(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_amp(-6.0206) - 0.5).abs() < 1e-3);
        assert_eq!(fade_gain(0.0, 10.0, 2.0, 2.0), 0.0);
        assert_eq!(fade_gain(1.0, 10.0, 2.0, 2.0), 0.5);
        assert_eq!(fade_gain(5.0, 10.0, 2.0, 2.0), 1.0);
        assert_eq!(fade_gain(8.0, 10.0, 2.0, 2.0), 1.0);
        assert_eq!(fade_gain(9.0, 10.0, 2.0, 2.0), 0.5);
        assert_eq!(fade_gain(10.0, 10.0, 2.0, 2.0), 0.0);
    }

    #[test]
    fn controls_expose_gain_and_both_fade_nodes() {
        let a = ClipAudio {
            gain_db: -6.0,
            pan: 0.0,
            fade_in: arc_core::time::Time::new(1, 1).unwrap(),
            fade_out: arc_core::time::Time::new(2, 1).unwrap(),
        };
        let c = controls(&geometry(), &a, 10.0).expect("controls");
        assert!(c.line_y < c.baseline);
        assert!(c.fade_in_node.left() >= c.region.left() + 8.0);
        assert!(c.fade_out_node.right() <= c.region.right() - 8.0);
        assert_eq!(hit_test(&c, c.fade_in_node.center()), Some(HitZone::FadeIn));
        assert_eq!(
            hit_test(&c, c.fade_out_node.center()),
            Some(HitZone::FadeOut)
        );
        assert_eq!(hit_test(&c, c.volume_node.center()), Some(HitZone::Volume));
    }

    #[test]
    fn chevron_rotates_when_the_node_crosses_the_audio_centre() {
        let up = ClipAudio {
            gain_db: 6.0,
            ..ClipAudio::default()
        };
        let c = controls(&geometry(), &up, 10.0).expect("high controls");
        assert!(volume_tip_down(&c), "above the centre must point down");
        let down = ClipAudio {
            gain_db: -30.0,
            ..ClipAudio::default()
        };
        let c = controls(&geometry(), &down, 10.0).expect("low controls");
        assert!(!volume_tip_down(&c), "below the centre must point up");
        assert_eq!(c.volume_node.width(), 18.0, "larger visual hit target");
    }

    #[test]
    fn local_history_undoes_and_redoes() {
        let before = ClipAudio::default();
        let after = ClipAudio {
            gain_db: -3.0,
            ..before
        };
        let r = ClipRef { track: 0, clip: 0 };
        let mut h = AudioHistory::default();
        h.push(AudioEdit { r, before, after });
        assert!(h.can_undo());
        let u = h.undo().expect("undo");
        assert_eq!(u.before, before);
        assert!(h.can_redo());
        assert_eq!(h.redo().expect("redo").after, after);
    }
}
