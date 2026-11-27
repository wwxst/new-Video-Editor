//! Drawing primitives for the timeline: lanes, clips, ruler, playhead, track controls, toolbar
//! icons and scrollbars. Everything is painted with vector strokes, so no icon font is needed.

use arc_core::time::{FrameRate, Round, Time};
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

/// Internal regions of a 72 pt video clip: header, thumbnails and embedded audio.
pub const VIDEO_HEADER_H: f32 = 18.0;
pub const VIDEO_THUMB_H: f32 = 38.0;
pub const VIDEO_AUDIO_H: f32 = 16.0;
/// Internal regions of a 72 pt image clip (the thumbnail area uses the rest of the clip).
pub const IMAGE_HEADER_H: f32 = 18.0;
/// Internal regions of a 46 pt audio clip.
pub const AUDIO_HEADER_H: f32 = 16.0;
pub const AUDIO_WAVE_H: f32 = 30.0;
/// Shared label typography and padding.
pub const LABEL_FONT_PT: f32 = 10.5;
pub const LABEL_PAD_X: f32 = 6.0;
pub const LABEL_TEXT: Color32 = Color32::from_rgb(0xf2, 0xf4, 0xf6);
/// Bright blue used by standalone audio clips.
pub const WAVE_BLUE: Color32 = Color32::from_rgb(0x2b, 0xb3, 0xff);
/// Cyan-green used by the audio embedded inside a video clip.
pub const WAVE_EMBEDDED: Color32 = Color32::from_rgb(0x3d, 0xd6, 0xb4);

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

/// Clip body: the kind's colour and a subtle top highlight.
pub fn draw_clip_body(painter: &Painter, rect: Rect, kind: TrackKind) {
    painter.rect_filled(rect, CornerRadius::same(4), kind.clip_color());
    let top = Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), (rect.top() + 2.0).min(rect.bottom())),
    );
    painter.rect_filled(top, CornerRadius::same(2), Color32::from_white_alpha(10));
}

/// Selection outline, painted after clip content so thumbnails and images cannot cover it.
pub fn draw_clip_outline(painter: &Painter, rect: Rect, color: Color32, width: f32) {
    if width <= 0.0 || rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(width, color),
        StrokeKind::Inside,
    );
}

/// Phase 3A primary-selection trim handle: a thin vertical white bar at the real clip edge.
pub fn draw_trim_handle(painter: &Painter, rect: Rect) {
    if rect.width() > 0.0 && rect.height() > 0.0 {
        painter.rect_filled(rect, CornerRadius::same(1), Color32::WHITE);
    }
}

/// Small lock badge for a locked clip. Vector, so it scales with DPI.
pub fn draw_lock_badge(painter: &Painter, center: Pos2, color: Color32) {
    let body = Rect::from_center_size(center + egui::vec2(0.0, 1.5), egui::vec2(7.0, 5.0));
    painter.rect_stroke(
        body,
        CornerRadius::same(1),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
    painter.add(egui::Shape::line(
        vec![
            egui::pos2(center.x - 2.5, center.y + 0.5),
            egui::pos2(center.x - 2.5, center.y - 2.5),
            egui::pos2(center.x, center.y - 4.0),
            egui::pos2(center.x + 2.5, center.y - 2.5),
            egui::pos2(center.x + 2.5, center.y + 0.5),
        ],
        Stroke::new(1.0, color),
    ));
}

/// Small mute badge for the audio region of a muted clip.
pub fn draw_mute_badge(painter: &Painter, center: Pos2, color: Color32) {
    painter.add(egui::Shape::line(
        vec![
            egui::pos2(center.x - 4.0, center.y - 1.5),
            egui::pos2(center.x - 1.5, center.y - 1.5),
            egui::pos2(center.x + 1.0, center.y - 3.5),
            egui::pos2(center.x + 1.0, center.y + 3.5),
            egui::pos2(center.x - 1.5, center.y + 1.5),
            egui::pos2(center.x - 4.0, center.y + 1.5),
            egui::pos2(center.x - 4.0, center.y - 1.5),
        ],
        Stroke::new(1.0, color),
    ));
    painter.line_segment(
        [
            egui::pos2(center.x + 2.0, center.y - 2.0),
            egui::pos2(center.x + 5.0, center.y + 2.0),
        ],
        Stroke::new(1.0, color),
    );
}

/// Marquee overlay, already clipped by the caller to the content area.
pub fn draw_marquee(painter: &Painter, rect: Rect) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    painter.rect_filled(
        rect,
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(255, 255, 255, 18),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, Color32::WHITE),
        StrokeKind::Inside,
    );
}

/// Plain clip label: no background box, left-aligned, vertically centred and elided to fit.
///
/// `label_rect` is already the leftmost visible part of the clip (clipped to the content area and
/// the clip boundary), so the text never slides under the fixed control column or a neighbour.
pub fn draw_clip_label(painter: &Painter, label_rect: Rect, text: &str) {
    if label_rect.width() < 14.0 || label_rect.height() < 9.0 || text.is_empty() {
        return;
    }
    let font = FontId::proportional(LABEL_FONT_PT);
    let max_width = (label_rect.width() - 2.0 * LABEL_PAD_X).max(4.0);
    let text = elide(painter, text, &font, max_width);
    let pos = egui::pos2(label_rect.left() + LABEL_PAD_X, label_rect.center().y);
    painter
        .with_clip_rect(label_rect)
        .text(pos, Align2::LEFT_CENTER, text, font, LABEL_TEXT);
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

/// Dense, bottom-anchored waveform bars for one clip.
///
/// Every painted column aggregates the decoded envelope over the source-time range that column
/// covers at the current zoom. Bars grow upward from a single baseline, so silence stays flat and
/// loud passages stand tall; the source-time aggregation is what makes the density follow the
/// timeline zoom without re-decoding.
pub fn draw_waveform_bars(
    painter: &Painter,
    rect: Rect,
    clip_left: f32,
    zoom: f32,
    wave: &super::media::Waveform,
    color: Color32,
) {
    if rect.width() < 2.0 || rect.height() < 4.0 || zoom <= 0.0 {
        return;
    }
    let baseline = rect.bottom() - 1.0;
    let amplitude = (rect.height() - 2.0).max(1.0);
    painter.line_segment(
        [
            egui::pos2(rect.left(), baseline),
            egui::pos2(rect.right(), baseline),
        ],
        Stroke::new(
            0.7,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 70),
        ),
    );
    let mut x = rect.left();
    while x < rect.right() {
        let t0 = ((x - clip_left) / zoom) as f64;
        let t1 = t0 + 1.0 / zoom as f64;
        if let Some(peak) = wave.envelope_at(t0, t1, zoom as f64) {
            let level = peak.max.max(-peak.min).clamp(0.0, 1.0);
            if level > 0.004 {
                let top = (baseline - level * amplitude).max(rect.top());
                painter.line_segment(
                    [egui::pos2(x, baseline), egui::pos2(x, top)],
                    Stroke::new(1.0, color),
                );
            }
        }
        x += 1.0;
    }
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

/// Minimum distance between two ruler labels, in points.
const LABEL_GAP: f32 = 78.0;
/// Rule heights for the three tick levels.
const TICK_SECOND_H: f32 = 9.0;
const TICK_FRAME_LABEL_H: f32 = 6.0;
const TICK_MINOR_H: f32 = 4.0;
const TICK_FRAME_H: f32 = 3.0;

/// A frame grid: the exact project rate plus its nominal integer rate for `mm:ss`/frame labels.
///
/// `frame_time` converts a frame index through [`FrameRate::time_of_frame`], never through an
/// accumulated floating-point second count. `nominal` is `fps.round()` and is the number of frame
/// labels inside one timecode second. For 29.97/59.94 the nominal rates are 30/60, so frame ticks
/// land on real frame boundaries and `00:01` is frame 30/60, not a rounded decimal second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGrid {
    pub rate: FrameRate,
    pub nominal: u32,
}

impl FrameGrid {
    pub fn new(rate: FrameRate) -> FrameGrid {
        FrameGrid {
            rate,
            nominal: rate.nominal() as u32,
        }
    }

    pub fn fps(&self) -> f64 {
        self.rate.rate().to_f64_lossy()
    }

    /// Exact start time of `frame` in real seconds.
    pub fn frame_time(&self, frame: u64) -> f64 {
        self.rate.time_of_frame(frame as i64).to_f64_lossy()
    }
}

/// How the ruler is labelled at the current scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TickMode {
    /// Whole-second labels such as `00:02`; `major_frames` is the same step in frames, so labels
    /// land on frame boundaries even for 29.97/59.94.
    Seconds {
        grid: FrameGrid,
        seconds: u64,
        major_frames: u64,
    },
    /// Frame-level labels. `step` is a divisor of the nominal rate: e.g. 4 frames at 24 fps
    /// (`04f`, `08f`, …), 10 frames at 29.97/30 fps (`10f`, `20f`, …).
    Frames { grid: FrameGrid, step: u32 },
}

/// Pick the ruler scale for a zoom level.
///
/// Seconds are preferred while the whole-second labels still fit. Once a second is wide enough to
/// split into frame ticks, the ruler switches to the densest whole-frame subdivision of the
/// project rate that keeps labels [`LABEL_GAP`] apart. The step always divides the nominal rate,
/// so `mm:ss` labels stay on frame-aligned second boundaries.
pub fn ruler_mode(px_per_sec: f32, rate: FrameRate) -> TickMode {
    let grid = FrameGrid::new(rate);
    let nominal = grid.nominal;
    let px_per_frame = px_per_sec / grid.fps() as f32;
    let mut step = None;
    for n in 2..=nominal {
        if !nominal.is_multiple_of(n) {
            continue;
        }
        let candidate = nominal / n;
        if candidate as f32 * px_per_frame >= LABEL_GAP {
            step = Some(candidate);
        }
    }
    if let Some(step) = step.filter(|step| *step < nominal) {
        TickMode::Frames { grid, step }
    } else {
        let seconds = tick_step(px_per_sec) as u64;
        TickMode::Seconds {
            grid,
            seconds,
            major_frames: seconds * nominal as u64,
        }
    }
}

/// `mm:ss`, or `hh:mm:ss` past an hour, from a whole number of timecode seconds.
pub fn ruler_timecode_seconds(total: u64) -> String {
    let s = total % 60;
    let m = (total / 60) % 60;
    let h = total / 3600;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Label of a frame-grid tick: whole seconds read `mm:ss`, intermediate frame ticks read `NNf`
/// where `NN` is the frame position inside the nominal second.
pub fn frame_tick_label(frame: u64, grid: FrameGrid) -> String {
    let within = frame % grid.nominal as u64;
    if within == 0 {
        ruler_timecode_seconds(frame / grid.nominal as u64)
    } else {
        format!("{within:02}f")
    }
}

/// Ruler: adaptive ticks and labels, top-aligned to the canvas.
pub fn draw_ruler(
    painter: &Painter,
    rect: Rect,
    view_start: f64,
    px_per_sec: f32,
    rate: FrameRate,
    playhead_x: Option<f32>,
) {
    painter.rect_filled(rect, CornerRadius::ZERO, RULER_BG);
    if rect.width() < 8.0 {
        return;
    }
    let view_end = view_start + (rect.width() / px_per_sec.max(1.0)) as f64;
    let mode = ruler_mode(px_per_sec, rate);
    match mode {
        TickMode::Seconds {
            grid,
            major_frames,
            ..
        } => draw_second_ticks(
            painter,
            rect,
            view_start,
            view_end,
            px_per_sec,
            grid,
            major_frames,
        ),
        TickMode::Frames { grid, step } => {
            draw_frame_ticks(painter, rect, view_start, px_per_sec, grid, step)
        }
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

fn draw_second_ticks(
    painter: &Painter,
    rect: Rect,
    view_start: f64,
    view_end: f64,
    px_per_sec: f32,
    grid: FrameGrid,
    major_frames: u64,
) {
    // Major labels sit on frame-aligned second boundaries. The five minor ticks between them are
    // evenly spaced in real time and stay unlabelled, as before.
    let first = ((view_start * grid.fps()) / major_frames as f64).floor().max(0.0) as u64
        * major_frames;
    let mut frame = first;
    loop {
        let t = grid.frame_time(frame);
        if t > view_end + grid.frame_time(major_frames) {
            break;
        }
        let x = rect.left() + ((t - view_start) as f32 * px_per_sec);
        if x >= rect.left() - 1.0 && x <= rect.right() + 1.0 {
            draw_tick(painter, rect, x, TICK_SECOND_H, TICK);
            draw_tick_label(
                painter,
                rect,
                x,
                &ruler_timecode_seconds(frame / grid.nominal as u64),
            );
        }
        let next = grid.frame_time(frame + major_frames);
        for k in 1..5 {
            let mt = t + (next - t) * (k as f64 / 5.0);
            let mx = rect.left() + ((mt - view_start) as f32 * px_per_sec);
            if mx >= rect.left() - 1.0 && mx <= rect.right() + 1.0 {
                draw_tick(painter, rect, mx, TICK_MINOR_H, FAINT);
            }
        }
        frame += major_frames;
    }
}

fn draw_frame_ticks(
    painter: &Painter,
    rect: Rect,
    view_start: f64,
    px_per_sec: f32,
    grid: FrameGrid,
    step: u32,
) {
    let px_per_frame = px_per_sec / grid.fps() as f32;
    let step_time = grid.frame_time(step as u64);
    let view_end = view_start + (rect.width() / px_per_sec.max(1.0)) as f64;
    let first = ((view_start * grid.fps()) / step as f64).floor().max(0.0) as u64 * step as u64;
    let mut frame = first;
    while grid.frame_time(frame) <= view_end + step_time {
        let t = grid.frame_time(frame);
        let x = rect.left() + ((t - view_start) as f32 * px_per_sec);
        if x >= rect.left() - 1.0 && x <= rect.right() + 1.0 {
            let label = frame_tick_label(frame, grid);
            let height = if frame.is_multiple_of(grid.nominal as u64) {
                TICK_SECOND_H
            } else {
                TICK_FRAME_LABEL_H
            };
            draw_tick(painter, rect, x, height, TICK);
            draw_tick_label(painter, rect, x, &label);
            // Individual frame ticks between two labelled ticks; skipped when they would be
            // closer than ~3 pt and read as a solid line.
            if px_per_frame >= 3.0 {
                for k in 1..step {
                    let mx = x + k as f32 * px_per_frame;
                    if mx > rect.right() {
                        break;
                    }
                    draw_tick(painter, rect, mx, TICK_FRAME_H, FAINT);
                }
            }
        }
        frame += step as u64;
    }
}

fn draw_tick(painter: &Painter, rect: Rect, x: f32, height: f32, color: Color32) {
    painter.line_segment(
        [
            egui::pos2(x, rect.bottom() - height),
            egui::pos2(x, rect.bottom()),
        ],
        Stroke::new(1.0, color),
    );
}

fn draw_tick_label(painter: &Painter, rect: Rect, x: f32, label: &str) {
    painter.with_clip_rect(rect).text(
        egui::pos2(x + 3.0, rect.top() + 2.0),
        Align2::LEFT_TOP,
        label,
        FontId::proportional(10.0),
        DIM,
    );
}

fn draw_playhead_handle(painter: &Painter, x: f32, ruler: Rect) {
    // Keep the handle in the lower part of the ruler so it can never cover a tick label; the
    // playhead line still starts at the ruler's bottom edge.
    let y = ruler.bottom() - 1.0;
    let top = (ruler.bottom() - 8.0).max(ruler.top() + 1.0);
    let pts = vec![
        egui::pos2(x - 5.0, top),
        egui::pos2(x + 5.0, top),
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

/// Seconds between labelled ruler ticks: the smallest step from a fixed ladder that keeps labels
/// apart. Sub-second steps are handled by [`ruler_mode`]'s frame scale instead, so every label here
/// falls on a whole second.
pub fn tick_step(px_per_sec: f32) -> f64 {
    const LADDER: [f64; 10] = [1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0];
    for step in LADDER {
        if step as f32 * px_per_sec >= LABEL_GAP {
            return step;
        }
    }
    *LADDER.last().expect("ladder")
}

/// Exact frame time of a real-seconds instant. Used for labels; the ruler itself works in frame
/// indices.
pub fn time_of_seconds(seconds: f64) -> Time {
    let micros = (seconds.max(0.0) * 1_000_000.0).round() as i64;
    Time::new(micros, 1_000_000).unwrap_or(Time::ZERO)
}

/// NDF timecode for a duration at the project's exact rate: `HH:MM:SS:FF`, never a decimal.
pub fn timecode_ndf(seconds: f64, rate: FrameRate) -> String {
    let frame = rate
        .frame_at(time_of_seconds(seconds), Round::Nearest)
        .unwrap_or(0);
    rate.format_ndf(frame)
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

    #[test]
    fn ruler_labels_are_mmss_and_never_decimal_seconds() {
        assert_eq!(ruler_timecode_seconds(0), "00:00");
        assert_eq!(ruler_timecode_seconds(2), "00:02");
        assert_eq!(ruler_timecode_seconds(4), "00:04");
        assert_eq!(ruler_timecode_seconds(59), "00:59");
        assert_eq!(ruler_timecode_seconds(60), "01:00");
        assert_eq!(ruler_timecode_seconds(3661), "01:01:01");
        assert!(!ruler_timecode_seconds(2).contains('.'));
    }

    #[test]
    fn ruler_levels_follow_zoom_and_project_rate() {
        assert_eq!(tick_step(20.0), 5.0);
        assert_eq!(tick_step(30.0), 3.0);
        assert_eq!(tick_step(58.0), 2.0);

        match ruler_mode(58.0, FrameRate::Fps24) {
            TickMode::Seconds {
                grid,
                seconds,
                major_frames,
            } => {
                assert_eq!(seconds, 2);
                assert_eq!(major_frames, 48);
                assert_eq!(grid, FrameGrid::new(FrameRate::Fps24));
            }
            other => panic!("expected second ticks, got {other:?}"),
        }
        match ruler_mode(180.0, FrameRate::Fps30) {
            TickMode::Frames { grid, step } => {
                assert_eq!(step, 15, "fine zoom shows 15f at 30 fps");
                assert_eq!(grid.nominal, 30);
            }
            other => panic!("expected 15-frame ticks, got {other:?}"),
        }
        match ruler_mode(600.0, FrameRate::Fps30) {
            TickMode::Frames { step, .. } => assert_eq!(step, 5, "5f level"),
            other => panic!("expected 5-frame ticks, got {other:?}"),
        }
        match ruler_mode(1_500.0, FrameRate::Fps30) {
            TickMode::Frames { step, .. } => assert_eq!(step, 2, "2f level"),
            other => panic!("expected 2-frame ticks, got {other:?}"),
        }
        match ruler_mode(3_000.0, FrameRate::Fps30) {
            TickMode::Frames { step, .. } => assert_eq!(step, 1, "1f level"),
            other => panic!("expected 1-frame ticks, got {other:?}"),
        }
        // 29.97/59.94 use their nominal rates for labels and the real rate for positions.
        match ruler_mode(240.0, FrameRate::Fps29_97) {
            TickMode::Frames { grid, step } => {
                assert_eq!(step, 10);
                assert_eq!(grid.nominal, 30);
                assert!((grid.fps() - 30_000.0 / 1001.0).abs() < 1e-9);
            }
            other => panic!("expected 10-frame ticks at 29.97, got {other:?}"),
        }
        match ruler_mode(480.0, FrameRate::Fps59_94) {
            TickMode::Frames { grid, step } => {
                assert_eq!(step, 10);
                assert_eq!(grid.nominal, 60);
                assert!((grid.fps() - 60_000.0 / 1001.0).abs() < 1e-9);
            }
            other => panic!("expected 10-frame ticks at 59.94, got {other:?}"),
        }
        // 25 fps cannot split a second into frame labels until 5f is wide enough.
        match ruler_mode(480.0, FrameRate::Fps25) {
            TickMode::Frames { step, .. } => assert_eq!(step, 5),
            other => panic!("expected 5-frame ticks at 25 fps, got {other:?}"),
        }
    }

    #[test]
    fn ruler_labels_never_overlap_across_rates_and_zooms() {
        for rate in [
            FrameRate::Fps24,
            FrameRate::Fps25,
            FrameRate::Fps30,
            FrameRate::Fps29_97,
            FrameRate::Fps59_94,
        ] {
            let max = rate.rate().to_f64_lossy() as f32 * 200.0;
            for zoom in [
                12.0f32, 20.0, 30.0, 58.0, 78.0, 120.0, 156.0, 180.0, 240.0, 480.0, 600.0,
                1_200.0, 1_500.0, 2_400.0, 3_000.0, 4_800.0, 6_000.0, 12_000.0,
            ] {
                if zoom > max {
                    continue;
                }
                let spacing = match ruler_mode(zoom, rate) {
                    TickMode::Seconds { grid, major_frames, .. } => {
                        major_frames as f64 / grid.fps() * zoom as f64
                    }
                    TickMode::Frames { grid, step } => {
                        step as f64 / grid.fps() * zoom as f64
                    }
                };
                assert!(
                    spacing >= LABEL_GAP as f64 - 0.01,
                    "{rate:?} {zoom} px/s: label spacing {spacing}"
                );
            }
        }
    }

    #[test]
    fn frame_tick_labels_follow_the_project_frame_rate() {
        let at_24 = FrameGrid::new(FrameRate::Fps24);
        assert_eq!(frame_tick_label(0, at_24), "00:00");
        assert_eq!(frame_tick_label(8, at_24), "08f");
        assert_eq!(frame_tick_label(16, at_24), "16f");
        assert_eq!(frame_tick_label(24, at_24), "00:01");

        let at_30 = FrameGrid::new(FrameRate::Fps29_97);
        assert_eq!(frame_tick_label(10, at_30), "10f");
        assert_eq!(frame_tick_label(20, at_30), "20f");
        assert_eq!(frame_tick_label(30, at_30), "00:01");

        let at_60 = FrameGrid::new(FrameRate::Fps59_94);
        assert_eq!(frame_tick_label(10, at_60), "10f");
        assert_eq!(frame_tick_label(50, at_60), "50f");
        assert_eq!(frame_tick_label(60, at_60), "00:01");

        let at_25 = FrameGrid::new(FrameRate::Fps25);
        assert_eq!(frame_tick_label(5, at_25), "05f");
        assert_eq!(frame_tick_label(25, at_25), "00:01");
    }

    #[test]
    fn non_integer_rates_place_frames_on_real_boundaries() {
        let at_29_97 = FrameGrid::new(FrameRate::Fps29_97);
        let f30 = 30.0 * 1001.0 / 30_000.0;
        assert!((at_29_97.frame_time(30) - f30).abs() < 1e-12);
        assert!((at_29_97.frame_time(30) - 1.001).abs() < 1e-9);
        assert_eq!(
            (at_29_97.frame_time(1234) * at_29_97.fps()).round() as u64,
            1234
        );

        let at_59_94 = FrameGrid::new(FrameRate::Fps59_94);
        let f60 = 60.0 * 1001.0 / 60_000.0;
        assert!((at_59_94.frame_time(60) - f60).abs() < 1e-12);
        assert!((at_59_94.frame_time(60) - 1.001).abs() < 1e-9);
        assert_eq!(
            (at_59_94.frame_time(987) * at_59_94.fps()).round() as u64,
            987
        );
    }

    #[test]
    fn clip_timecode_is_frame_accurate_and_never_decimal() {
        assert_eq!(timecode_ndf(0.0, FrameRate::Fps24), "00:00:00:00");
        assert_eq!(timecode_ndf(5.0, FrameRate::Fps24), "00:00:05:00");
        assert_eq!(timecode_ndf(4.5, FrameRate::Fps24), "00:00:04:12");
        assert_eq!(timecode_ndf(18.0, FrameRate::Fps24), "00:00:18:00");
        assert_eq!(timecode_ndf(3661.5, FrameRate::Fps24), "01:01:01:12");
        assert_eq!(timecode_ndf(1.0, FrameRate::Fps29_97), "00:00:01:00");
        assert_eq!(timecode_ndf(0.5, FrameRate::Fps30), "00:00:00:15");
        assert_eq!(timecode_ndf(0.48, FrameRate::Fps25), "00:00:00:12");
    }

    #[test]
    fn internal_clip_regions_fill_the_approved_heights() {
        assert_eq!(VIDEO_HEADER_H + VIDEO_THUMB_H + VIDEO_AUDIO_H, 72.0);
        assert_eq!(VIDEO_HEADER_H + 54.0, 72.0, "silent video uses the full rest");
        assert_eq!(IMAGE_HEADER_H + 54.0, 72.0);
        assert_eq!(AUDIO_HEADER_H + AUDIO_WAVE_H, 46.0);
    }
}


