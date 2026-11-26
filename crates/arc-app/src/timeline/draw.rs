//! Drawing primitives for the timeline: lanes, clips, ruler, playhead, track controls, toolbar
//! icons and scrollbars. Everything is painted with vector strokes, so no icon font is needed.

use super::model::{Clip, TrackKind};
use egui::{
    Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    Ui,
};

pub const CANVAS_BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1c);
pub const LANE_BG_A: Color32 = Color32::from_rgb(0x19, 0x1b, 0x20);
pub const LANE_BG_B: Color32 = Color32::from_rgb(0x1c, 0x1e, 0x24);
pub const RULER_BG: Color32 = Color32::from_rgb(0x18, 0x1a, 0x1f);
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(0x18, 0x1a, 0x1f);
pub const CONTROLS_BG: Color32 = Color32::from_rgb(0x17, 0x19, 0x1e);
pub const TEXT: Color32 = Color32::from_rgb(0xdf, 0xe2, 0xe7);
pub const DIM: Color32 = Color32::from_rgb(0x86, 0x8c, 0x95);
pub const FAINT: Color32 = Color32::from_rgb(0x3a, 0x3e, 0x46);
pub const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xff);
pub const PLAYHEAD: Color32 = Color32::from_rgb(0xe8, 0x5d, 0x5d);
pub const TICK: Color32 = Color32::from_rgb(0x4a, 0x4f, 0x58);

pub fn lane_bg(index: usize) -> Color32 {
    if index % 2 == 0 {
        LANE_BG_A
    } else {
        LANE_BG_B
    }
}

/// Solid clip body plus a subtle top highlight, with the kind's colour.
pub fn draw_clip(painter: &Painter, rect: Rect, clip: &Clip, selected: bool) {
    let fill = clip.kind.clip_color();
    painter.rect_filled(rect, CornerRadius::same(4), fill);
    if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.5, ACCENT),
            StrokeKind::Inside,
        );
    }

    // Video and image clips get mock thumbnails; audio clips a waveform.
    match clip.kind {
        TrackKind::Video => {
            draw_thumbnails(painter, rect.shrink(2.0), &clip.name, 26.0, Some(0.72));
            draw_waveform(painter, strip(rect, 0.74, 0.98), &clip.name, 0.35);
        }
        TrackKind::Image => draw_thumbnails(painter, rect.shrink(2.0), &clip.name, 26.0, None),
        TrackKind::Audio => {
            painter.rect_filled(rect, CornerRadius::same(4), fill);
            draw_waveform(painter, rect.shrink2(egui::vec2(3.0, 4.0)), &clip.name, 0.85);
        }
        _ => {}
    }

    // Name (and file) on a dark pill so it stays readable over thumbnails and waveforms.
    let label = match &clip.file {
        Some(file) if clip.kind != TrackKind::Text => format!("{file}  {:.1}s", clip.duration),
        _ => format!("{}  {:.1}s", clip.name, clip.duration),
    };
    let text_pos = rect.left_top() + egui::vec2(6.0, 4.0);
    let galley = painter.layout_no_wrap(label.clone(), FontId::proportional(10.5), TEXT);
    let bg = Rect::from_min_size(text_pos - egui::vec2(2.0, 1.0), galley.size() + egui::vec2(4.0, 2.0))
        .intersect(rect.shrink(1.0));
    if bg.width() > 6.0 {
        painter.rect_filled(bg, CornerRadius::same(3), Color32::from_black_alpha(110));
    }
    painter.with_clip_rect(rect.shrink(2.0)).galley(
        text_pos,
        galley,
        Color32::from_rgb(0xf2, 0xf4, 0xf6),
    );
}

fn strip(rect: Rect, top: f32, bottom: f32) -> Rect {
    Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + rect.height() * top),
        egui::pos2(rect.right(), rect.top() + rect.height() * bottom),
    )
}

/// Mock thumbnails: a row of tinted cells derived from the clip name.
fn draw_thumbnails(painter: &Painter, rect: Rect, seed: &str, cell: f32, height_frac: Option<f32>) {
    let area = match height_frac {
        Some(frac) => Rect::from_min_max(
            rect.min,
            egui::pos2(rect.right(), rect.top() + rect.height() * frac),
        ),
        None => rect,
    };
    if area.width() < 2.0 || area.height() < 2.0 {
        return;
    }
    let base = mix(seed, 0x20, 0x80);
    let mut x = area.left();
    let mut i = 0u32;
    let clipped = painter.with_clip_rect(area.intersect(rect));
    while x < area.right() {
        let w = cell.min(area.right() - x);
        let tint = 0.55 + 0.1 * ((i as f32 * 0.9).sin() * 0.5 + 0.5);
        clipped.rect_filled(
            Rect::from_min_size(egui::pos2(x, area.top()), egui::vec2(w - 1.0, area.height())),
            CornerRadius::same(2),
            scale(base, tint),
        );
        x += cell;
        i += 1;
    }
}

fn scale(color: Color32, factor: f32) -> Color32 {
    let f = factor.clamp(0.0, 1.0);
    Color32::from_rgb(
        (color.r() as f32 * f) as u8,
        (color.g() as f32 * f) as u8,
        (color.b() as f32 * f) as u8,
    )
}

fn mix(seed: &str, min: u8, max: u8) -> Color32 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in seed.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    let t = (h % 3) as f32 / 2.0;
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    Color32::from_rgb(lerp(min, max), lerp(0x60, 0x9a), lerp(0x50, 0x7e))
}

/// Mock waveform: vertical bars whose heights are a deterministic function of the name.
pub fn draw_waveform(painter: &Painter, rect: Rect, seed: &str, strength: f32) {
    if rect.width() < 2.0 || rect.height() < 4.0 {
        return;
    }
    let mut h = 0u64;
    for b in seed.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u64);
    }
    let mid = rect.center().y;
    let color = Color32::from_rgba_unmultiplied(0xcf, 0xe6, 0xff, 190);
    let mut x = rect.left() + 1.0;
    let mut i = 0u32;
    while x < rect.right() {
        let phase = (i as f32 * 0.55) + (h % 97) as f32 * 0.13;
        let noise = ((h.wrapping_add(i as u64).wrapping_mul(2654435761) >> 8) as f32 % 100.0) / 100.0;
        let amp = ((phase.sin() * 0.5 + 0.5) * 0.65 + noise * 0.35) * strength;
        let half = (rect.height() * 0.5 - 1.0) * amp;
        painter.line_segment(
            [egui::pos2(x, mid - half), egui::pos2(x, mid + half)],
            Stroke::new(1.0, color),
        );
        x += 2.0;
        i += 1;
    }
}

/// Ruler: adaptive major/minor ticks and timecodes, top-aligned to the canvas.
pub fn draw_ruler(
    painter: &Painter,
    rect: Rect,
    view_start: f64,
    px_per_sec: f32,
    playhead_x: Option<f32>,
) {
    painter.rect_filled(rect, CornerRadius::ZERO, RULER_BG);
    if rect.width() < 8.0 {
        return;
    }
    let step = tick_step(px_per_sec);
    let minor = step / 5.0;
    let first = (view_start / minor).floor() * minor;
    let mut t = first;
    while t <= view_start + (rect.width() / px_per_sec) as f64 + step {
        let x = rect.left() + ((t - view_start) as f32 * px_per_sec);
        if x >= rect.left() - 1.0 && x <= rect.right() + 1.0 {
            let major = (t / step).round() * step - t;
            let is_major = major.abs() < minor * 0.01;
            let h = if is_major { 8.0 } else { 4.0 };
            let col = if is_major { TICK } else { FAINT };
            painter.line_segment(
                [
                    egui::pos2(x, rect.bottom() - h),
                    egui::pos2(x, rect.bottom()),
                ],
                Stroke::new(1.0, col),
            );
            if is_major {
                painter.text(
                    egui::pos2(x + 3.0, rect.top() + 2.0),
                    Align2::LEFT_TOP,
                    timecode(t.max(0.0)),
                    FontId::proportional(10.0),
                    DIM,
                );
            }
        }
        t += minor;
    }
    // Bottom hairline separating the ruler from the lanes.
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.bottom() - 0.5),
            egui::pos2(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, FAINT),
    );
    if let Some(x) = playhead_x {
        draw_playhead_handle(painter, x, rect);
    }
}

fn draw_playhead_handle(painter: &Painter, x: f32, ruler: Rect) {
    let y = ruler.bottom() - 1.0;
    let pts = vec![
        egui::pos2(x - 5.0, ruler.top() + 1.0),
        egui::pos2(x + 5.0, ruler.top() + 1.0),
        egui::pos2(x + 5.0, y - 2.0),
        egui::pos2(x, y),
        egui::pos2(x - 5.0, y - 2.0),
    ];
    painter.add(egui::Shape::convex_polygon(
        pts,
        PLAYHEAD,
        Stroke::new(1.0, PLAYHEAD),
    ));
}

/// The left control column: type icon, lock, eye/mute, centred on each lane.
pub fn draw_track_controls(
    ui: &mut Ui,
    painter: &Painter,
    column: Rect,
    lane: Rect,
    track: &mut super::model::Track,
    track_index: usize,
) {
    let center_y = lane.center().y;
    let mut x = column.left() + 12.0;
    if let Some(kind) = track.kind() {
        draw_track_icon(painter, kind, egui::pos2(x, center_y), TEXT, 12.0);
        let hit = Rect::from_center_size(egui::pos2(x, center_y), egui::vec2(16.0, 16.0));
        let id = ui.make_persistent_id(("track-kind", track_index));
        ui.interact(hit, id, Sense::hover())
            .on_hover_text(format!("{} · {}", kind.label(), track.name));
    }
    x += 22.0;

    let visual = track.kind().is_some_and(|k| k.is_visual());
    let audio = track.kind().is_some_and(|k| k.has_audio());
    let controls = [
        (ControlIcon::Lock, true, track.locked),
        (ControlIcon::Eye, visual, !track.visible),
        (ControlIcon::Mute, audio, track.muted),
    ];
    for (slot, (icon, applicable, active)) in controls.into_iter().enumerate() {
        if !applicable {
            x += 20.0;
            continue;
        }
        let hit = Rect::from_center_size(egui::pos2(x + 8.0, center_y), egui::vec2(18.0, 18.0));
        let id = ui.make_persistent_id(("track-ctl", track_index, slot));
        let response = ui.interact(hit, id, Sense::click());
        let color = if active {
            DIM
        } else if response.hovered() {
            TEXT
        } else {
            Color32::from_rgb(0x9a, 0xa0, 0xa9)
        };
        draw_control_icon(painter, icon, hit, color);
        if response.clicked() {
            match icon {
                ControlIcon::Lock => track.locked = !track.locked,
                ControlIcon::Eye => track.visible = !track.visible,
                ControlIcon::Mute => track.muted = !track.muted,
            }
        }
        if response.hovered() {
            let tip = match icon {
                ControlIcon::Lock if track.locked => "Unlock track",
                ControlIcon::Lock => "Lock track",
                ControlIcon::Eye if track.visible => "Hide track",
                ControlIcon::Eye => "Show track",
                ControlIcon::Mute if track.muted => "Unmute track",
                ControlIcon::Mute => "Mute track",
            };
            response.on_hover_text(tip);
        }
        x += 20.0;
    }
}
#[derive(Clone, Copy)]
pub enum ControlIcon {
    Lock,
    Eye,
    Mute,
}

fn draw_control_icon(painter: &Painter, icon: ControlIcon, rect: Rect, color: Color32) {
    let c = rect.center();
    match icon {
        ControlIcon::Lock => {
            painter.rect_stroke(
                Rect::from_center_size(c + egui::vec2(0.0, 2.0), egui::vec2(8.0, 6.0)),
                CornerRadius::same(1),
                Stroke::new(1.0, color),
                StrokeKind::Inside,
            );
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - 3.0, c.y + 1.0),
                    egui::pos2(c.x - 3.0, c.y - 3.0),
                    egui::pos2(c.x, c.y - 4.5),
                    egui::pos2(c.x + 3.0, c.y - 3.0),
                    egui::pos2(c.x + 3.0, c.y + 1.0),
                ],
                Stroke::new(1.0, color),
            ));
        }
        ControlIcon::Eye => {
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - 6.0, c.y + 1.0),
                    egui::pos2(c.x - 3.0, c.y - 2.5),
                    egui::pos2(c.x, c.y - 4.0),
                    egui::pos2(c.x + 3.0, c.y - 2.5),
                    egui::pos2(c.x + 6.0, c.y + 1.0),
                    egui::pos2(c.x + 3.0, c.y + 3.5),
                    egui::pos2(c.x, c.y + 4.5),
                    egui::pos2(c.x - 3.0, c.y + 3.5),
                    egui::pos2(c.x - 6.0, c.y + 1.0),
                ],
                Stroke::new(1.0, color),
            ));
            painter.circle_filled(c + egui::vec2(0.0, 0.5), 1.8, color);
        }
        ControlIcon::Mute => {
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - 5.0, c.y - 2.0),
                    egui::pos2(c.x - 2.0, c.y - 2.0),
                    egui::pos2(c.x + 1.0, c.y - 4.5),
                    egui::pos2(c.x + 1.0, c.y + 4.5),
                    egui::pos2(c.x - 2.0, c.y + 2.0),
                    egui::pos2(c.x - 5.0, c.y + 2.0),
                    egui::pos2(c.x - 5.0, c.y - 2.0),
                ],
                Stroke::new(1.0, color),
            ));
            painter.line_segment(
                [
                    egui::pos2(c.x + 3.0, c.y - 2.5),
                    egui::pos2(c.x + 6.5, c.y + 2.5),
                ],
                Stroke::new(1.0, color),
            );
        }
    }
}

/// Type icon per content kind, drawn from strokes so it scales with DPI.
pub fn draw_track_icon(painter: &Painter, kind: TrackKind, center: Pos2, color: Color32, size: f32) {
    let h = size * 0.5;
    let s = Stroke::new(1.0, color);
    match kind {
        TrackKind::Text => {
            painter.text(
                center,
                Align2::CENTER_CENTER,
                "T",
                FontId::proportional(size + 1.0),
                color,
            );
        }
        TrackKind::Effect => {
            // Four-point star.
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(center.x, center.y - h),
                    egui::pos2(center.x + h * 0.3, center.y - h * 0.3),
                    egui::pos2(center.x + h, center.y),
                    egui::pos2(center.x + h * 0.3, center.y + h * 0.3),
                    egui::pos2(center.x, center.y + h),
                    egui::pos2(center.x - h * 0.3, center.y + h * 0.3),
                    egui::pos2(center.x - h, center.y),
                    egui::pos2(center.x - h * 0.3, center.y - h * 0.3),
                    egui::pos2(center.x, center.y - h),
                ],
                s,
            ));
        }
        TrackKind::Sticker => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(size, size)),
                CornerRadius::same(3),
                s,
                StrokeKind::Inside,
            );
            painter.circle_filled(center + egui::vec2(0.0, 0.0), 1.6, color);
        }
        TrackKind::Video => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(size + 1.0, size * 0.8)),
                CornerRadius::same(2),
                s,
                StrokeKind::Inside,
            );
            for i in 0..3 {
                let x = center.x - size * 0.3 + i as f32 * size * 0.3;
                painter.line_segment(
                    [egui::pos2(x, center.y - h * 0.55), egui::pos2(x, center.y - h * 0.3)],
                    s,
                );
            }
        }
        TrackKind::Image => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(size + 1.0, size * 0.8)),
                CornerRadius::same(2),
                s,
                StrokeKind::Inside,
            );
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(center.x - h * 0.7, center.y + h * 0.25),
                    egui::pos2(center.x - h * 0.15, center.y - h * 0.35),
                    egui::pos2(center.x + h * 0.2, center.y + h * 0.05),
                    egui::pos2(center.x + h * 0.5, center.y - h * 0.2),
                    egui::pos2(center.x + h * 0.7, center.y + h * 0.25),
                ],
                s,
            ));
        }
        TrackKind::Audio => {
            for i in 0..4 {
                let x = center.x - size * 0.4 + i as f32 * size * 0.27;
                let ah = h * (0.35 + 0.2 * (i as f32 * 1.1).sin().abs());
                painter.line_segment([egui::pos2(x, center.y - ah), egui::pos2(x, center.y + ah)], s);
            }
        }
    }
}

/// Ruler tick spacing: the smallest step from a fixed ladder that keeps labels apart.
pub fn tick_step(px_per_sec: f32) -> f64 {
    const LADDER: [f64; 10] = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0];
    for step in LADDER {
        if step as f32 * px_per_sec >= 78.0 {
            return step;
        }
    }
    *LADDER.last().expect("ladder")
}

pub fn timecode(seconds: f64) -> String {
    let s = seconds.max(0.0);
    let minutes = (s / 60.0).floor() as i64;
    let rest = s - minutes as f64 * 60.0;
    if minutes > 0 {
        format!("{minutes}:{:05.2}", rest)
    } else {
        format!("{rest:05.2}")
    }
}

/// Compact icon button; returns the interaction response for tooltips.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ToolbarIcon {
    Select,
    Undo,
    Redo,
    Split,
    Delete,
    Link,
    Snap,
    ZoomOut,
    ZoomIn,
}

pub fn icon_button(
    ui: &mut Ui,
    rect: Rect,
    id: egui::Id,
    icon: ToolbarIcon,
    active: bool,
) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    let hovered = response.hovered();
    let bg = if active {
        Color32::from_rgb(0x2a, 0x3f, 0x66)
    } else if hovered {
        Color32::from_rgb(0x26, 0x29, 0x30)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(5), bg);
    let color = if active {
        Color32::from_rgb(0x9d, 0xc0, 0xff)
    } else if hovered {
        TEXT
    } else {
        DIM
    };
    draw_toolbar_icon(ui.painter(), icon, rect.shrink(6.0), color);
    response
}

fn draw_toolbar_icon(painter: &Painter, icon: ToolbarIcon, rect: Rect, color: Color32) {
    let s = Stroke::new(1.3, color);
    let c = rect.center();
    let h = rect.height() * 0.5;
    match icon {
        ToolbarIcon::Select => {
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - h * 0.55, c.y - h * 0.75),
                    egui::pos2(c.x + h * 0.55, c.y + h * 0.05),
                    egui::pos2(c.x + h * 0.02, c.y + h * 0.12),
                    egui::pos2(c.x + h * 0.35, c.y + h * 0.75),
                    egui::pos2(c.x + h * 0.12, c.y + h * 0.85),
                    egui::pos2(c.x - h * 0.2, c.y + h * 0.2),
                    egui::pos2(c.x - h * 0.55, c.y + h * 0.45),
                    egui::pos2(c.x - h * 0.55, c.y - h * 0.75),
                ],
                s,
            ));
        }
        ToolbarIcon::Undo | ToolbarIcon::Redo => {
            let dir = if icon == ToolbarIcon::Undo { -1.0 } else { 1.0 };
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x + dir * h * 0.7, c.y - h * 0.35),
                    egui::pos2(c.x - dir * h * 0.1, c.y - h * 0.35),
                    egui::pos2(c.x - dir * h * 0.55, c.y + h * 0.05),
                    egui::pos2(c.x - dir * h * 0.1, c.y + h * 0.45),
                    egui::pos2(c.x + dir * h * 0.7, c.y + h * 0.45),
                ],
                s,
            ));
            painter.line_segment(
                [
                    egui::pos2(c.x + dir * h * 0.7, c.y - h * 0.35),
                    egui::pos2(c.x + dir * h * 0.7, c.y + h * 0.45),
                ],
                s,
            );
        }
        ToolbarIcon::Split => {
            painter.line_segment(
                [
                    egui::pos2(c.x - h * 0.7, c.y - h * 0.8),
                    egui::pos2(c.x + h * 0.45, c.y + h * 0.55),
                ],
                s,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x + h * 0.7, c.y - h * 0.8),
                    egui::pos2(c.x - h * 0.45, c.y + h * 0.55),
                ],
                s,
            );
            painter.circle_stroke(
                egui::pos2(c.x - h * 0.55, c.y + h * 0.6),
                2.2,
                s,
            );
            painter.circle_stroke(
                egui::pos2(c.x + h * 0.55, c.y + h * 0.6),
                2.2,
                s,
            );
        }
        ToolbarIcon::Delete => {
            painter.rect_stroke(
                Rect::from_center_size(c + egui::vec2(0.0, 1.5), egui::vec2(h * 1.1, h * 1.5)),
                CornerRadius::same(1),
                s,
                StrokeKind::Inside,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x - h * 0.8, c.y - h * 0.55),
                    egui::pos2(c.x + h * 0.8, c.y - h * 0.55),
                ],
                s,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x - h * 0.25, c.y - h * 0.85),
                    egui::pos2(c.x + h * 0.25, c.y - h * 0.85),
                ],
                s,
            );
            for i in 0..2 {
                let x = c.x - h * 0.25 + i as f32 * h * 0.5;
                painter.line_segment(
                    [egui::pos2(x, c.y - h * 0.1), egui::pos2(x, c.y + h * 0.75)],
                    s,
                );
            }
        }
        ToolbarIcon::Link => {
            painter.rect_stroke(
                Rect::from_center_size(c + egui::vec2(-h * 0.35, 0.0), egui::vec2(h, h * 0.8)),
                CornerRadius::same(3),
                s,
                StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_center_size(c + egui::vec2(h * 0.35, 0.0), egui::vec2(h, h * 0.8)),
                CornerRadius::same(3),
                s,
                StrokeKind::Inside,
            );
        }
        ToolbarIcon::Snap => {
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - h * 0.6, c.y - h * 0.8),
                    egui::pos2(c.x - h * 0.6, c.y + h * 0.1),
                    egui::pos2(c.x - h * 0.3, c.y + h * 0.7),
                    egui::pos2(c.x + h * 0.3, c.y + h * 0.7),
                    egui::pos2(c.x + h * 0.6, c.y + h * 0.1),
                    egui::pos2(c.x + h * 0.6, c.y - h * 0.8),
                ],
                s,
            ));
            painter.line_segment(
                [
                    egui::pos2(c.x - h * 0.6, c.y - h * 0.35),
                    egui::pos2(c.x + h * 0.6, c.y - h * 0.35),
                ],
                s,
            );
        }
        ToolbarIcon::ZoomOut | ToolbarIcon::ZoomIn => {
            painter.circle_stroke(c + egui::vec2(-h * 0.15, -h * 0.15), h * 0.55, s);
            painter.line_segment(
                [
                    egui::pos2(c.x + h * 0.3, c.y + h * 0.3),
                    egui::pos2(c.x + h * 0.85, c.y + h * 0.85),
                ],
                s,
            );
            let cx = c.x - h * 0.15;
            painter.line_segment(
                [egui::pos2(cx - h * 0.25, c.y - h * 0.15), egui::pos2(cx + h * 0.25, c.y - h * 0.15)],
                s,
            );
            if icon == ToolbarIcon::ZoomIn {
                painter.line_segment(
                    [
                        egui::pos2(cx, c.y - h * 0.4),
                        egui::pos2(cx, c.y + h * 0.1),
                    ],
                    s,
                );
            }
        }
    }
}

/// Thin scrollbar; returns the thumb rect for dragging.
pub fn draw_scrollbar(painter: &Painter, rect: Rect, content: f32, scroll: f32, vertical: bool) -> Rect {
    let color = Color32::from_rgb(0x33, 0x37, 0x3e);
    if vertical {
        painter.rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(0x14, 0x16, 0x1a));
        let frac = (rect.height() / content.max(1.0)).min(1.0);
        let len = (rect.height() * frac).max(18.0);
        let max_scroll = (content - rect.height()).max(1.0);
        let t = (scroll / max_scroll).clamp(0.0, 1.0);
        let y = rect.top() + t * (rect.height() - len);
        let thumb = Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), len));
        painter.rect_filled(thumb, CornerRadius::same(3), color);
        thumb
    } else {
        painter.rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(0x14, 0x16, 0x1a));
        let frac = (rect.width() / content.max(1.0)).min(1.0);
        let len = (rect.width() * frac).max(24.0);
        let max_scroll = (content - rect.width()).max(1.0);
        let t = (scroll / max_scroll).clamp(0.0, 1.0);
        let x = rect.left() + t * (rect.width() - len);
        let thumb = Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(len, rect.height()));
        painter.rect_filled(thumb, CornerRadius::same(3), color);
        thumb
    }
    .intersect(rect)
}


