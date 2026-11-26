//! Drawing primitives for the timeline: lanes, clips, ruler, playhead, track controls, toolbar
//! icons and scrollbars. Everything is painted with vector strokes, so no icon font is needed.

use super::model::TrackKind;
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

/// Boundary between the fixed control column and the scrolling canvas: a hairline at the column's
/// right edge plus a short, soft shadow that falls on the canvas, so the column reads as a layer
/// above the timeline instead of an equal neighbour.
pub fn draw_panel_edge(painter: &Painter, column: Rect, bounds: Rect) {
    let x = column.right() - 0.5;
    painter.line_segment(
        [
            egui::pos2(x, column.top()),
            egui::pos2(x, column.bottom()),
        ],
        Stroke::new(1.0, Color32::from_rgb(0x0e, 0x0f, 0x12)),
    );
    const STEPS: usize = 6;
    for i in 0..STEPS {
        let alpha = 44 - 7 * i as u8;
        let x0 = column.right() + i as f32;
        let strip = Rect::from_min_max(
            egui::pos2(x0, column.top()),
            egui::pos2(x0 + 1.0, column.bottom()),
        )
        .intersect(bounds);
        if strip.width() <= 0.0 || strip.height() <= 0.0 {
            continue;
        }
        painter.rect_filled(strip, CornerRadius::ZERO, Color32::from_black_alpha(alpha));
    }
}

/// Clip body: the kind's colour, a subtle top highlight and the selection outline.
pub fn draw_clip_body(painter: &Painter, rect: Rect, kind: TrackKind, selected: bool) {
    painter.rect_filled(rect, CornerRadius::same(4), kind.clip_color());
    if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.5, ACCENT),
            StrokeKind::Inside,
        );
    }
    let top = Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), (rect.top() + 2.0).min(rect.bottom())),
    );
    painter.rect_filled(top, CornerRadius::same(2), Color32::from_white_alpha(10));
}

/// Compact label in the clip's top-left corner, elided so it never leaves the clip.
pub fn draw_clip_label(painter: &Painter, rect: Rect, text: &str) {
    if rect.width() < 16.0 || rect.height() < 12.0 || text.is_empty() {
        return;
    }
    let font = FontId::proportional(10.5);
    let text = elide(painter, text, &font, (rect.width() - 10.0).max(6.0));
    let pos = rect.left_top() + egui::vec2(5.0, 3.0);
    let galley = painter.layout_no_wrap(text, font, Color32::from_rgb(0xf2, 0xf4, 0xf6));
    let bg = Rect::from_min_size(pos - egui::vec2(2.0, 1.0), galley.size() + egui::vec2(4.0, 2.0))
        .intersect(rect.shrink(0.5));
    if bg.width() > 4.0 {
        painter.rect_filled(bg, CornerRadius::same(3), Color32::from_black_alpha(120));
    }
    painter.with_clip_rect(rect.shrink(1.0)).galley(
        pos,
        galley,
        Color32::from_rgb(0xf2, 0xf4, 0xf6),
    );
}

/// Clear fallback for missing or undecodable media: a hatched body and a warning label. Never a
/// fake thumbnail or waveform, so a broken file cannot masquerade as real content.
pub fn draw_missing(painter: &Painter, rect: Rect, message: &str) {
    painter.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(0x2b, 0x1e, 0x1e));
    let hatch = Color32::from_rgba_unmultiplied(0x8a, 0x4a, 0x44, 90);
    let mut x = rect.left() - rect.height();
    while x < rect.right() {
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom()),
                egui::pos2(x + rect.height(), rect.top()),
            ],
            Stroke::new(1.0, hatch),
        );
        x += 10.0;
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        message,
        FontId::proportional(9.5),
        Color32::from_rgb(0xd8, 0x9a, 0x92),
    );
}

/// Dense min/max waveform for the visible part of an audio clip. `clip_left` is the clip's content
/// x and `zoom` the timeline scale, so every painted column maps to its real source time.
pub fn draw_waveform(
    painter: &Painter,
    rect: Rect,
    clip_left: f32,
    zoom: f32,
    wave: &super::media::Waveform,
) {
    if rect.width() < 2.0 || rect.height() < 6.0 || zoom <= 0.0 {
        return;
    }
    let mid = rect.center().y;
    let amplitude = rect.height() * 0.5 - 2.0;
    let color = Color32::from_rgba_unmultiplied(0xcf, 0xe6, 0xff, 190);
    let mut x = rect.left();
    while x < rect.right() {
        let t0 = ((x - clip_left) / zoom) as f64;
        let t1 = t0 + 1.0 / zoom as f64;
        if let Some(peak) = wave.envelope(t0, t1) {
            let a = mid - peak.max.clamp(-1.0, 1.0) * amplitude;
            let b = mid - peak.min.clamp(-1.0, 1.0) * amplitude;
            let (top, bottom) = (a.min(b), a.max(b));
            painter.line_segment(
                [egui::pos2(x, top), egui::pos2(x, bottom.max(top + 0.5))],
                Stroke::new(1.0, color),
            );
        }
        x += 1.0;
    }
    painter.line_segment(
        [
            egui::pos2(rect.left(), mid),
            egui::pos2(rect.right(), mid),
        ],
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xcf, 0xe6, 0xff, 40)),
    );
}

/// UV rectangle that centre-crops an image of `aspect` (width / height) into `rect` without
/// distorting it.
pub fn uv_fill(aspect: f32, rect: Rect) -> Rect {
    let src = aspect.max(0.01);
    let dst = (rect.width() / rect.height().max(0.01)).max(0.01);
    if dst > src {
        let keep = (src / dst).clamp(0.0, 1.0);
        let top = (1.0 - keep) * 0.5;
        Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(1.0, 1.0 - top))
    } else {
        let keep = (dst / src).clamp(0.0, 1.0);
        let left = (1.0 - keep) * 0.5;
        Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(1.0 - left, 1.0))
    }
}

/// Truncate `text` to `max_width`, appending an ellipsis. Pure, so it can be unit tested.
pub fn elide_text(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> String {
    if text.is_empty() || measure(text) <= max_width {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let mut candidate: String = chars.iter().collect();
        candidate.push('…');
        if measure(&candidate) <= max_width {
            return candidate;
        }
    }
    "…".to_string()
}

fn elide(painter: &Painter, text: &str, font: &FontId, max_width: f32) -> String {
    elide_text(text, max_width, |candidate| {
        painter
            .layout_no_wrap(
                candidate.to_string(),
                font.clone(),
                Color32::WHITE,
            )
            .size()
            .x
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(text: &str) -> f32 {
        text.chars().count() as f32 * 6.0
    }

    #[test]
    fn elide_keeps_short_text_and_truncates_long_text() {
        assert_eq!(elide_text("Blur", 100.0, measure), "Blur");
        let long = elide_text("Color correct  4.0s", 60.0, measure);
        assert!(long.ends_with('…'), "{long}");
        assert!(measure(&long) <= 60.0, "{long}");
        assert!(long.chars().count() < "Color correct  4.0s".chars().count());
        // A width below even the ellipsis still returns a single truncation marker.
        assert_eq!(elide_text("Sharpen", 2.0, measure), "…");
    }

    #[test]
    fn uv_fill_crops_without_distortion() {
        // A square source in a wide tile keeps the full width and crops the top and bottom.
        let wide = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(160.0, 72.0));
        let uv = uv_fill(1.0, wide);
        assert!((uv.width() - 1.0).abs() < 1e-6 && uv.height() < 0.999);
        // A 16:9 source in a square tile keeps the full height and crops the sides.
        let square = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(72.0, 72.0));
        let uv = uv_fill(16.0 / 9.0, square);
        assert!((uv.height() - 1.0).abs() < 1e-6 && uv.width() < 0.999);
        // Matching aspects use the whole texture.
        let wide_tile = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(160.0, 90.0));
        let uv = uv_fill(16.0 / 9.0, wide_tile);
        assert!((uv.width() - 1.0).abs() < 1e-6 && (uv.height() - 1.0).abs() < 1e-6);
    }
}


