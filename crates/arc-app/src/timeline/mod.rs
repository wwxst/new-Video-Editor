//! Timeline module: toolbar, ruler, track-control column and the lane canvas, driven by mock data.
//!
//! Everything here is presentation and local state (zoom, scroll, selection, audio gestures).
//! The audio parameters use the engine's `ClipAudio` value type but this surface never mutates a
//! live `Project`; the mock model can be replaced by the real document feed later.
//! Lane heights are fixed by content type and are not editable state.

pub mod audio;
pub mod draw;
pub mod interaction;
pub mod media;
pub mod model;

use arc_core::model::ClipAudio;
use arc_core::time::FrameRate;
use audio::{AudioDrag, AudioEdit, AudioHistory};
use draw::{CANVAS_BG, CONTROLS_BG, DIM, TOOLBAR_BG, ToolbarIcon};
use egui::{Align2, Color32, CornerRadius, FontId, Key, Rect, Sense, Stroke, Ui, Vec2};
use interaction::{ClipGeometry, ClipHit, ClipRef, HitZone, Marquee, Selection};
use media::MediaHub;
use model::TimelineModel;
use std::path::Path;

/// Width of the left track-control column, in points.
pub const CONTROL_W: f32 = 112.0;
/// Toolbar height, in points.
pub const TOOLBAR_H: f32 = 34.0;
/// Ruler height, in points.
pub const RULER_H: f32 = 22.0;
/// Scrollbar thickness, in points.
pub const SCROLLBAR_W: f32 = 9.0;
/// Minimum horizontal zoom, in points per second.
pub const MIN_ZOOM: f32 = 12.0;
/// Right-aligned zoom controls: two 24 pt buttons around a 110 pt horizontal slider.
const ZOOM_BUTTON: f32 = 24.0;
const ZOOM_SLIDER_W: f32 = 110.0;
const ZOOM_GAP: f32 = 2.0;
const TOOLBAR_MARGIN: f32 = 10.0;
/// Right edge of the left-hand tool group, including the gap after the last tool.
const TOOLBAR_TOOLS_W: f32 = 212.0;

fn tb(ui: &mut Ui, x: &mut f32, y: f32, icon: ToolbarIcon, active: bool) -> egui::Response {
    let r = Rect::from_center_size(egui::pos2(*x + 12.0, y), Vec2::new(24.0, 24.0));
    *x += 26.0;
    tb_at(ui, r, icon, active)
}

fn tb_at(ui: &mut Ui, rect: Rect, icon: ToolbarIcon, active: bool) -> egui::Response {
    let id = ui.make_persistent_id(("tl-toolbar", icon));
    draw::icon_button(ui, rect, id, icon, active)
}

/// Right-hand zoom controls of the toolbar. `None` when the toolbar is too narrow to host them
/// without colliding with the editing tools.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ZoomControls {
    out: Rect,
    slider: Rect,
    input: Rect,
}

fn zoom_controls(toolbar: Rect) -> Option<ZoomControls> {
    let total = ZOOM_BUTTON + ZOOM_GAP + ZOOM_SLIDER_W + ZOOM_GAP + ZOOM_BUTTON;
    let left = toolbar.right() - TOOLBAR_MARGIN - total;
    if left < toolbar.left() + TOOLBAR_TOOLS_W {
        return None;
    }
    let y = toolbar.center().y;
    let button = |x: f32| {
        Rect::from_min_size(
            egui::pos2(x, y - ZOOM_BUTTON * 0.5),
            Vec2::new(ZOOM_BUTTON, ZOOM_BUTTON),
        )
    };
    let out = button(left);
    let slider = Rect::from_min_size(
        egui::pos2(out.right() + ZOOM_GAP, y - 10.0),
        Vec2::new(ZOOM_SLIDER_W, 20.0),
    );
    let input = button(slider.right() + ZOOM_GAP);
    Some(ZoomControls { out, slider, input })
}

/// Slider position of a zoom level, 0 at [`MIN_ZOOM`] and 1 at `max_zoom`.
///
/// The mapping is logarithmic because zoom itself is multiplicative: equal drags feel like equal
/// relative changes, and the whole 12 px/s … frame-max range is reachable.
fn zoom_slider_fraction(zoom: f32, max_zoom: f32) -> f32 {
    let zoom = zoom.clamp(MIN_ZOOM, max_zoom.max(MIN_ZOOM));
    (zoom / MIN_ZOOM).ln() / (max_zoom.max(MIN_ZOOM) / MIN_ZOOM).ln()
}

fn zoom_from_fraction(fraction: f32, max_zoom: f32) -> f32 {
    MIN_ZOOM * (max_zoom.max(MIN_ZOOM) / MIN_ZOOM).powf(fraction.clamp(0.0, 1.0))
}

/// Rectangles of the timeline chrome inside the timeline card.
///
/// The left column is a fixed foreground layer: [`Layout::header`] and [`Layout::controls`] are
/// painted after the canvas, so they cover whatever slides underneath. [`Layout::canvas`] is the
/// visible content area right of the column, and the origin the ruler, scroll math and playhead
/// share; [`Layout::underlay`] is the full-width body the canvas paints into, running under the
/// column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub toolbar: Rect,
    pub ruler: Rect,
    pub header: Rect,
    pub controls: Rect,
    pub canvas: Rect,
    pub underlay: Rect,
}

/// Resolve the timeline areas from the card content rect.
pub fn layout(rect: Rect) -> Layout {
    let w = rect.width().max(1.0);
    let h = rect.height().max(1.0);
    let toolbar = Rect::from_min_size(rect.min, Vec2::new(w, TOOLBAR_H.min(h)));
    let column_w = CONTROL_W.min(w - 1.0);
    let content_x = rect.left() + column_w;
    let ruler_h = RULER_H.min((h - TOOLBAR_H).max(0.0));
    let ruler = Rect::from_min_size(
        egui::pos2(content_x, rect.top() + TOOLBAR_H),
        Vec2::new((w - column_w).max(1.0), ruler_h),
    );
    let header = Rect::from_min_size(
        egui::pos2(rect.left(), rect.top() + TOOLBAR_H),
        Vec2::new(column_w, ruler_h),
    );
    let body_top = rect.top() + TOOLBAR_H + ruler_h;
    let body_h = (h - TOOLBAR_H - ruler_h).max(0.0);
    let controls = Rect::from_min_size(
        egui::pos2(rect.left(), body_top),
        Vec2::new(column_w, body_h),
    );
    let canvas = Rect::from_min_size(
        egui::pos2(content_x, body_top),
        Vec2::new((w - column_w).max(1.0), body_h),
    );
    let underlay = Rect::from_min_size(egui::pos2(rect.left(), body_top), Vec2::new(w, body_h));
    Layout {
        toolbar,
        ruler,
        header,
        controls,
        canvas,
        underlay,
    }
}

/// Time-to-pixel mapping of the timeline canvas: the single formula shared by the ruler handle, the
/// playhead line and the drag hit test.
fn time_to_x(origin: f32, view_start: f64, zoom: f32, time: f64) -> f32 {
    origin + ((time - view_start) as f32 * zoom)
}

/// Inverse of [`time_to_x`].
fn x_to_time(origin: f32, view_start: f64, zoom: f32, x: f32) -> f64 {
    view_start + ((x - origin) / zoom) as f64
}

/// Timeline UI state. All values are in points, so DPI scaling is automatic.
pub struct Timeline {
    pub model: TimelineModel,
    /// Horizontal scale: points per second.
    pub zoom: f32,
    /// Scroll offsets in points.
    pub scroll: Vec2,
    /// Multi-selection with a primary item. Phase 3A state only; no project mutation.
    pub selection: Selection,
    /// Pointer hover for this frame. Recomputed from geometry, never cached by screen position.
    hover: Option<ClipHit>,
    /// Active marquee, if the user is dragging on empty lane space.
    marquee: Option<Marquee>,
    /// Active volume/fade gesture.
    audio_drag: Option<AudioDrag>,
    /// Undo/redo for in-timeline audio gestures.
    audio_history: AudioHistory,
    pub select_mode: bool,
    pub snap: bool,
    pub link: bool,
    /// Decoder workers and texture caches; created on the first painted frame so an unpainted
    /// timeline (tests, headless use) never starts threads.
    media: Option<MediaHub>,
}

impl std::fmt::Debug for Timeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Timeline")
            .field("model", &self.model)
            .field("zoom", &self.zoom)
            .field("scroll", &self.scroll)
            .field("selection", &self.selection)
            .field("hover", &self.hover)
            .field("audio_drag", &self.audio_drag)
            .field("media", &self.media)
            .finish()
    }
}

impl Clone for Timeline {
    fn clone(&self) -> Timeline {
        Timeline {
            model: self.model.clone(),
            zoom: self.zoom,
            scroll: self.scroll,
            selection: self.selection.clone(),
            hover: self.hover,
            marquee: self.marquee.clone(),
            audio_drag: self.audio_drag,
            audio_history: self.audio_history.clone(),
            select_mode: self.select_mode,
            snap: self.snap,
            link: self.link,
            // The clone starts its own decode workers on its first painted frame.
            media: None,
        }
    }
}

impl Default for Timeline {
    fn default() -> Self {
        Timeline {
            model: TimelineModel::mock(),
            zoom: 58.0,
            scroll: Vec2::ZERO,
            selection: Selection::default(),
            hover: None,
            marquee: None,
            audio_drag: None,
            audio_history: AudioHistory::default(),
            select_mode: true,
            snap: true,
            link: false,
            media: None,
        }
    }
}

impl Timeline {
    /// Timeline at the project's exact sequence rate. The zoom ceiling is derived from it.
    pub fn new(frame_rate: FrameRate) -> Timeline {
        Timeline {
            model: TimelineModel::mock().with_frame_rate(frame_rate),
            ..Timeline::default()
        }
    }

    /// Empty timeline (used by the empty-state rendering and tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn empty() -> Timeline {
        Timeline {
            model: TimelineModel::empty(),
            ..Timeline::default()
        }
    }

    pub fn frame_rate(&self) -> FrameRate {
        self.model.frame_rate
    }

    /// Right-panel readout for the primary audio-capable clip. Values come from the same
    /// `ClipAudio` the waveform and mixer use, so they update during a drag without a second model.
    pub fn audio_inspector(&mut self, ui: &mut Ui) {
        let Some(r) = self.selection.primary() else {
            ui.label(
                egui::RichText::new("No clip selected")
                    .size(11.0)
                    .color(DIM),
            );
            return;
        };
        let Some(track) = self.model.tracks.get(r.track) else {
            return;
        };
        let Some(clip) = track.clips.get(r.clip) else {
            return;
        };
        let (kind, name, audio, enabled, muted, locked) = (
            clip.kind,
            clip.name.clone(),
            clip.audio,
            clip.enabled,
            track.muted,
            track.locked,
        );
        let has_audio = kind == model::TrackKind::Audio
            || (kind == model::TrackKind::Video && self.clip_has_audio(r.track, r.clip));
        ui.label(
            egui::RichText::new(name)
                .size(12.0)
                .strong()
                .color(draw::TEXT),
        );
        if !has_audio {
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("No embedded audio")
                    .size(11.0)
                    .color(DIM),
            );
            return;
        }
        ui.add_space(6.0);
        let row = |ui: &mut Ui, label: &str, value: String| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).size(10.5).color(DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(value)
                            .size(11.0)
                            .monospace()
                            .color(draw::TEXT),
                    );
                });
            });
        };
        row(ui, "VOLUME", format!("{:+.1} dB", audio.gain_db));
        row(
            ui,
            "FADE IN",
            format!("{:.1} s", audio::time_secs(audio.fade_in)),
        );
        row(
            ui,
            "FADE OUT",
            format!("{:.1} s", audio::time_secs(audio.fade_out)),
        );
        ui.add_space(6.0);
        let status = if !enabled {
            "DISABLED"
        } else if muted {
            "MUTED · AUDIO CONTROLS HIDDEN"
        } else if locked {
            "LOCKED"
        } else {
            "Hover the clip to edit its envelope"
        };
        let color = if enabled && !muted && !locked {
            DIM
        } else {
            Color32::from_rgb(0xc9, 0xa0, 0x60)
        };
        ui.label(egui::RichText::new(status).size(9.5).color(color));
    }

    /// Highest horizontal zoom: 200 px per project frame, so it scales with the real rate
    /// (24 fps → 4800 px/s, 30 fps → 6000 px/s, 59.94 fps → 11988 px/s).
    pub fn max_zoom(&self) -> f32 {
        self.frame_rate().rate().to_f64_lossy() as f32 * model::MAX_FRAME_PX
    }

    /// First visible timeline position, in seconds.
    pub fn view_start(&self) -> f64 {
        (self.scroll.x / self.zoom) as f64
    }

    pub fn max_scroll(&self, canvas: Rect) -> Vec2 {
        let content_w = self.model.content_width(self.zoom);
        let content_h = self.model.content_height();
        Vec2::new(
            (content_w - canvas.width()).max(0.0),
            (content_h - canvas.height()).max(0.0),
        )
    }

    /// Draw and drive the timeline. `rect` is the timeline card's content area.
    ///
    /// The timeline is two layers: the canvas (lane backgrounds, clips, playhead) is painted first
    /// across the full body width, then the fixed left column is painted opaquely on top. A
    /// horizontal scroll therefore slides the canvas beneath the column instead of meeting it at a
    /// shared edge.
    pub fn ui(&mut self, ui: &mut Ui, rect: Rect) {
        let l = layout(rect);
        if l.toolbar.width() < 40.0 || l.canvas.height() < 20.0 {
            return;
        }
        // Media workers are created on the first painted frame, so a timeline that is never drawn
        // (tests, headless tools) does not start decode threads.
        if self.media.is_none() {
            self.media = Some(MediaHub::new());
        }
        if let Some(media) = self.media.as_mut() {
            media.poll(ui.ctx());
        }
        ui.painter()
            .rect_filled(l.toolbar, CornerRadius::ZERO, TOOLBAR_BG);
        ui.painter()
            .rect_filled(l.underlay, CornerRadius::ZERO, CANVAS_BG);

        self.toolbar(ui, l.toolbar, l.canvas);
        self.wheel(ui, &l);

        // Clamp scroll against the current content, then resolve the view.
        let max = self.max_scroll(l.canvas);
        self.scroll = Vec2::new(
            self.scroll.x.clamp(0.0, max.x),
            self.scroll.y.clamp(0.0, max.y),
        );
        let view_start = self.view_start();

        let body = self.body_rect(l.underlay);
        let origin = l.canvas.left();

        // Resolve the playhead before painting: a drag in this frame must move the handle and the
        // line together, and both are painted from this one x.
        self.drag_playhead(ui, l.ruler, view_start, origin);
        let playhead_x = self.playhead_x(origin, view_start);

        self.prune_selection();
        self.clip_input(ui, &l, body, origin, view_start);
        self.ruler(ui, l.ruler, view_start, playhead_x);
        let rows = self.lanes(ui, &l, body, origin, view_start, playhead_x);
        if let Some(m) = &self.marquee {
            let content = Rect::from_min_max(egui::pos2(l.controls.right(), body.top()), body.max);
            draw::draw_marquee(&ui.painter().with_clip_rect(content), m.rect(content));
        }
        self.control_panel(ui, &l, body, &rows);
        // A lock/visibility toggle happens in the foreground layer. Drop any newly invalid
        // selection immediately; the canvas redraws the white border next frame.
        self.prune_selection();
        self.scrollbars(ui, l.canvas, body);
    }

    /// Screen x of the playhead. The ruler handle, the vertical line and the drag hit test all use
    /// this one time-to-pixel mapping, so they cannot drift apart.
    fn playhead_x(&self, origin: f32, view_start: f64) -> f32 {
        time_to_x(origin, view_start, self.zoom, self.model.playhead)
    }

    /// Update the playhead from a click or drag on the ruler. Runs before painting so the handle and
    /// the line move in the same frame as the pointer. The time snaps to the nearest project frame.
    fn drag_playhead(&mut self, ui: &mut Ui, ruler: Rect, view_start: f64, origin: f32) {
        let id = ui.make_persistent_id("tl-playhead");
        let response = ui.interact(ruler, id, Sense::click_and_drag());
        if (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let t = x_to_time(origin, view_start, self.zoom, pos.x);
            let snapped = model::snap_to_frame(t, self.model.frame_rate);
            self.model.playhead = snapped.clamp(0.0, self.model.duration.max(0.0));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }

    /// Full body minus the scrollbar strips: the canvas paints here, under the fixed column too.
    fn body_rect(&self, underlay: Rect) -> Rect {
        Rect::from_min_max(
            underlay.min,
            egui::pos2(
                underlay.right() - SCROLLBAR_W,
                underlay.bottom() - SCROLLBAR_W,
            ),
        )
    }

    fn toolbar(&mut self, ui: &mut Ui, rect: Rect, canvas: Rect) {
        let y = rect.center().y;
        let mut x = rect.left() + 8.0;
        if tb(ui, &mut x, y, ToolbarIcon::Select, self.select_mode)
            .on_hover_text("Selection tool")
            .clicked()
        {
            self.select_mode = !self.select_mode;
        }
        if tb(ui, &mut x, y, ToolbarIcon::Undo, false)
            .on_hover_text("Undo (mock)")
            .clicked()
        {}
        if tb(ui, &mut x, y, ToolbarIcon::Redo, false)
            .on_hover_text("Redo (mock)")
            .clicked()
        {}
        x += 6.0;
        if tb(ui, &mut x, y, ToolbarIcon::Split, false)
            .on_hover_text("Split (mock)")
            .clicked()
        {}
        if tb(ui, &mut x, y, ToolbarIcon::Delete, false)
            .on_hover_text("Delete (mock)")
            .clicked()
        {}
        x += 6.0;
        if tb(ui, &mut x, y, ToolbarIcon::Link, self.link)
            .on_hover_text("Link clips (mock)")
            .clicked()
        {
            self.link = !self.link;
        }
        if tb(ui, &mut x, y, ToolbarIcon::Snap, self.snap)
            .on_hover_text("Snapping")
            .clicked()
        {
            self.snap = !self.snap;
        }

        // Zoom cluster, right-aligned: [-] [horizontal slider] [+]. The technical px/s readout is
        // gone; the slider is the one continuous zoom control.
        let mut requested = self.zoom;
        if let Some(z) = zoom_controls(rect) {
            let max_zoom = self.max_zoom();
            if tb_at(ui, z.out, ToolbarIcon::ZoomOut, false)
                .on_hover_text("Zoom out")
                .clicked()
            {
                requested *= 0.8;
            }
            if tb_at(ui, z.input, ToolbarIcon::ZoomIn, false)
                .on_hover_text("Zoom in")
                .clicked()
            {
                requested *= 1.25;
            }
            let id = ui.make_persistent_id("tl-zoom-slider");
            let response = ui.interact(z.slider, id, Sense::click_and_drag());
            let track = Rect::from_center_size(
                z.slider.center(),
                Vec2::new((z.slider.width() - 10.0).max(1.0), 3.0),
            );
            let mut fraction = zoom_slider_fraction(requested, max_zoom);
            if (response.clicked() || response.dragged())
                && let Some(pos) = response.interact_pointer_pos()
            {
                fraction = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
                requested = zoom_from_fraction(fraction, max_zoom);
            }
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(if response.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                });
            }
            self.paint_zoom_slider(
                ui,
                track,
                fraction,
                response.dragged() || response.hovered(),
            );
            response.on_hover_text("Timeline zoom");
        }
        if (requested - self.zoom).abs() > 0.001 {
            self.set_zoom(requested, canvas);
        }

        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.bottom() - 0.5),
                egui::pos2(rect.right(), rect.bottom() - 0.5),
            ],
            Stroke::new(1.0, Color32::from_rgb(0x23, 0x26, 0x2c)),
        );
    }

    /// Paint the slider track, the filled part and the knob. `fraction` is already the value the
    /// frame should show, so a click moves the knob in the same frame that the zoom changes.
    fn paint_zoom_slider(&self, ui: &Ui, track: Rect, fraction: f32, active: bool) {
        let painter = ui.painter();
        painter.rect_filled(
            track,
            CornerRadius::same(2),
            Color32::from_rgb(0x33, 0x37, 0x3f),
        );
        let knob_x = track.left() + fraction.clamp(0.0, 1.0) * track.width();
        if knob_x > track.left() + 0.5 {
            painter.rect_filled(
                Rect::from_min_max(track.min, egui::pos2(knob_x, track.max.y)),
                CornerRadius::same(2),
                draw::ACCENT,
            );
        }
        let knob = egui::pos2(knob_x, track.center().y);
        let radius = if active { 5.5 } else { 4.5 };
        painter.circle_filled(knob, radius, Color32::from_rgb(0xe6, 0xe8, 0xec));
        painter.circle_stroke(
            knob,
            radius,
            Stroke::new(1.0, Color32::from_rgb(0x12, 0x13, 0x16)),
        );
    }

    /// Change the horizontal scale. The playhead stays under its current pixel when it is visible;
    /// otherwise the canvas centre is the anchor, so a toolbar zoom never jumps.
    fn set_zoom(&mut self, zoom: f32, canvas: Rect) {
        let zoom = zoom.clamp(MIN_ZOOM, self.max_zoom());
        if (zoom - self.zoom).abs() < 0.001 {
            return;
        }
        let origin = canvas.left();
        let playhead_x = time_to_x(origin, self.view_start(), self.zoom, self.model.playhead);
        let anchor = if playhead_x >= canvas.left() && playhead_x <= canvas.right() {
            playhead_x
        } else {
            canvas.center().x
        };
        let time = x_to_time(origin, self.view_start(), self.zoom, anchor);
        self.zoom = zoom;
        self.scroll.x = (time as f32 * self.zoom - (anchor - origin)).max(0.0);
    }

    /// Zoom around a canvas pixel: the time under `pointer_x` stays fixed.
    fn zoom_at_pointer(&mut self, pointer_x: f32, origin: f32, factor: f32) {
        let time = x_to_time(origin, self.view_start(), self.zoom, pointer_x);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, self.max_zoom());
        self.scroll.x = (time as f32 * self.zoom - (pointer_x - origin)).max(0.0);
    }

    fn wheel(&mut self, ui: &Ui, l: &Layout) {
        let Some(pos) = ui.ctx().pointer_hover_pos() else {
            return;
        };
        if !(l.canvas.contains(pos)
            || l.controls.contains(pos)
            || l.header.contains(pos)
            || l.ruler.contains(pos))
        {
            return;
        }
        let (delta, ctrl, shift) =
            ui.input(|i| (i.smooth_scroll_delta, i.modifiers.ctrl, i.modifiers.shift));
        if delta == Vec2::ZERO {
            return;
        }
        if ctrl {
            let factor: f32 = (1.0 + delta.y * 0.004).clamp(0.8, 1.25);
            self.zoom_at_pointer(pos.x, l.canvas.left(), factor);
        } else if shift {
            self.scroll.x -= delta.y;
        } else {
            self.scroll.y -= delta.y;
            self.scroll.x -= delta.x;
        }
    }

    fn ruler(&self, ui: &mut Ui, rect: Rect, view_start: f64, playhead_x: f32) {
        let visible = playhead_x >= rect.left() - 2.0 && playhead_x <= rect.right() + 2.0;
        draw::draw_ruler(
            ui.painter(),
            rect,
            view_start,
            self.zoom,
            self.model.frame_rate,
            visible.then_some(playhead_x),
        );
    }

    /// Remove locked, hidden or disabled clips from the current selection. Runs before painting so
    /// a track state toggle cannot leave a stale white border.
    pub fn prune_selection(&mut self) {
        let model = &self.model;
        self.selection.retain(|r| {
            model
                .tracks
                .get(r.track)
                .and_then(|track| {
                    track
                        .clips
                        .get(r.clip)
                        .filter(|clip| clip.enabled && track.visible && !track.locked)
                })
                .is_some()
        });
    }

    /// One clip geometry at its current screen position. `has_audio` is precomputed for video.
    fn geometry_at(
        &self,
        lane: Rect,
        track_index: usize,
        clip_index: usize,
        origin: f32,
        view_start: f64,
        has_audio: bool,
    ) -> Option<ClipGeometry> {
        let track = self.model.tracks.get(track_index)?;
        let clip = track.clips.get(clip_index)?;
        let kind = track.kind()?;
        let clip_h = kind.clip_height();
        let clip_y = lane.top() + (lane.height() - clip_h) * 0.5;
        let x = origin + ((clip.start - view_start) as f32 * self.zoom);
        let w = (clip.duration as f32 * self.zoom).max(3.0);
        Some(interaction::clip_geometry(
            kind,
            Rect::from_min_size(egui::pos2(x, clip_y), Vec2::new(w, clip_h)),
            has_audio,
        ))
    }

    fn clip_has_audio(&mut self, track_index: usize, clip_index: usize) -> bool {
        let Some(track) = self.model.tracks.get(track_index) else {
            return false;
        };
        let Some(clip) = track.clips.get(clip_index) else {
            return false;
        };
        if clip.kind == model::TrackKind::Audio {
            return true;
        }
        if clip.kind != model::TrackKind::Video {
            return false;
        }
        let Some(source) = clip.source.as_deref() else {
            return false;
        };
        self.media
            .as_mut()
            .is_some_and(|media| media.has_audio(source))
    }

    fn clip_hit_at(
        &mut self,
        body: Rect,
        content: Rect,
        origin: f32,
        view_start: f64,
        pointer: egui::Pos2,
    ) -> Option<ClipHit> {
        let rows = self.lane_rows(body);
        for (track_index, lane) in rows.iter().enumerate() {
            let Some(track) = self.model.tracks.get(track_index) else {
                continue;
            };
            let (visible, locked, muted, clip_count) =
                (track.visible, track.locked, track.muted, track.clips.len());
            if !visible {
                continue;
            }
            for clip_index in 0..clip_count {
                if !self.model.tracks[track_index].clips[clip_index].enabled {
                    continue;
                }
                let has_audio = self.clip_has_audio(track_index, clip_index);
                let Some(g) = self.geometry_at(
                    *lane,
                    track_index,
                    clip_index,
                    origin,
                    view_start,
                    has_audio,
                ) else {
                    continue;
                };
                let r = ClipRef {
                    track: track_index,
                    clip: clip_index,
                };
                if let Some(mut hit) = interaction::hit_test_clip(r, &g, content, pointer, locked) {
                    // Audio controls live inside the audio region and upgrade the base Audio hit.
                    // They are inert for locked/muted clips and never overlap trim handles.
                    if !locked && !muted && has_audio && hit.zone == HitZone::Audio {
                        let clip = &self.model.tracks[track_index].clips[clip_index];
                        if let Some(c) = audio::controls(&g, &clip.audio, clip.duration) {
                            if let Some(zone) = audio::hit_test(&c, pointer) {
                                hit.zone = zone;
                            }
                        }
                    }
                    return Some(hit);
                }
            }
        }
        None
    }

    fn marquee_hits(
        &mut self,
        body: Rect,
        content: Rect,
        origin: f32,
        view_start: f64,
        rect: Rect,
    ) -> Vec<ClipRef> {
        let rows = self.lane_rows(body);
        let mut hits = Vec::new();
        for (track_index, lane) in rows.iter().enumerate() {
            let Some(track) = self.model.tracks.get(track_index) else {
                continue;
            };
            let (visible, locked, clip_count) = (track.visible, track.locked, track.clips.len());
            if !visible || locked {
                continue;
            }
            for clip_index in 0..clip_count {
                if !self.model.tracks[track_index].clips[clip_index].enabled {
                    continue;
                }
                let has_audio = self.clip_has_audio(track_index, clip_index);
                let Some(g) = self.geometry_at(
                    *lane,
                    track_index,
                    clip_index,
                    origin,
                    view_start,
                    has_audio,
                ) else {
                    continue;
                };
                if g.rect.intersect(content).intersects(rect) {
                    hits.push(ClipRef {
                        track: track_index,
                        clip: clip_index,
                    });
                }
            }
        }
        hits
    }

    /// Phase 3A pointer state: hover, click selection, Ctrl/Cmd multi-select and marquee. No drag,
    /// trim, split, snap or project mutation is performed here.
    fn begin_audio_drag(&mut self, hit: ClipHit, start: egui::Pos2) {
        let Some(clip) = self
            .model
            .tracks
            .get(hit.r.track)
            .and_then(|t| t.clips.get(hit.r.clip))
        else {
            return;
        };
        self.selection.select_only(hit.r);
        self.audio_drag = Some(AudioDrag {
            r: hit.r,
            zone: hit.zone,
            before: clip.audio,
            start,
        });
    }

    fn update_audio_drag(&mut self, pointer: egui::Pos2) {
        let Some(drag) = self.audio_drag else {
            return;
        };
        let Some(clip) = self
            .model
            .tracks
            .get(drag.r.track)
            .and_then(|t| t.clips.get(drag.r.clip))
        else {
            return;
        };
        let delta = pointer - drag.start;
        let duration = clip.duration.max(0.0);
        let mut next = drag.before;
        match drag.zone {
            HitZone::Volume => {
                next.gain_db = (drag.before.gain_db - delta.y as f64 * 0.8)
                    .clamp(audio::DB_MIN, audio::DB_MAX);
            }
            HitZone::FadeIn => {
                let secs = (audio::time_secs(drag.before.fade_in)
                    + delta.x as f64 / self.zoom as f64)
                    .clamp(0.0, duration);
                next.fade_in = audio::secs_to_time(secs);
            }
            HitZone::FadeOut => {
                let secs = (audio::time_secs(drag.before.fade_out)
                    - delta.x as f64 / self.zoom as f64)
                    .clamp(0.0, duration);
                next.fade_out = audio::secs_to_time(secs);
            }
            _ => return,
        }
        if let Some(clip) = self
            .model
            .tracks
            .get_mut(drag.r.track)
            .and_then(|t| t.clips.get_mut(drag.r.clip))
        {
            clip.audio = next;
        }
    }

    fn finish_audio_drag(&mut self) {
        let Some(drag) = self.audio_drag.take() else {
            return;
        };
        let Some(after) = self
            .model
            .tracks
            .get(drag.r.track)
            .and_then(|t| t.clips.get(drag.r.clip))
            .map(|c| c.audio)
        else {
            return;
        };
        self.audio_history.push(AudioEdit {
            r: drag.r,
            before: drag.before,
            after,
        });
    }

    fn restore_audio(&mut self, drag: AudioDrag) {
        if let Some(clip) = self
            .model
            .tracks
            .get_mut(drag.r.track)
            .and_then(|t| t.clips.get_mut(drag.r.clip))
        {
            clip.audio = drag.before;
        }
    }

    fn undo_audio(&mut self) {
        let Some(edit) = self.audio_history.undo() else {
            return;
        };
        if let Some(clip) = self
            .model
            .tracks
            .get_mut(edit.r.track)
            .and_then(|t| t.clips.get_mut(edit.r.clip))
        {
            clip.audio = edit.before;
        }
    }

    fn redo_audio(&mut self) {
        let Some(edit) = self.audio_history.redo() else {
            return;
        };
        if let Some(clip) = self
            .model
            .tracks
            .get_mut(edit.r.track)
            .and_then(|t| t.clips.get_mut(edit.r.clip))
        {
            clip.audio = edit.after;
        }
    }

    fn clip_input(&mut self, ui: &mut Ui, l: &Layout, body: Rect, origin: f32, view_start: f64) {
        let content = Rect::from_min_max(egui::pos2(l.controls.right(), body.top()), body.max);
        let response = ui.interact(
            content,
            ui.make_persistent_id("tl-clip-canvas"),
            Sense::click_and_drag(),
        );
        let pointer = ui.ctx().pointer_hover_pos();
        let modifiers = ui.input(|i| i.modifiers);
        let additive = modifiers.shift || modifiers.ctrl || modifiers.command;

        // Escape cancels an audio gesture or marquee and restores the pre-drag state.
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            if let Some(d) = self.audio_drag.take() {
                self.restore_audio(d);
            } else if let Some(m) = self.marquee.take() {
                self.selection = m.before;
            } else {
                self.selection.clear();
            }
            ui.ctx().request_repaint();
        }

        // Audio undo/redo is local to this timeline prototype; project-level history remains the
        // engine's job once the editor is wired to a live project.
        if modifiers.command || modifiers.ctrl {
            if ui.input(|i| i.key_pressed(Key::Z)) && !modifiers.shift {
                self.undo_audio();
                ui.ctx().request_repaint();
            } else if (ui.input(|i| i.key_pressed(Key::Z)) && modifiers.shift)
                || ui.input(|i| i.key_pressed(Key::Y))
            {
                self.redo_audio();
                ui.ctx().request_repaint();
            }
        }

        let hit = pointer
            .filter(|p| content.contains(*p))
            .and_then(|p| self.clip_hit_at(body, content, origin, view_start, p));
        self.hover = if self.marquee.is_some() { None } else { hit };

        if response.drag_started() {
            let start = ui
                .input(|i| i.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos());
            if let Some(start) = start
                && content.contains(start)
            {
                let start_hit = self.clip_hit_at(body, content, origin, view_start, start);
                match start_hit {
                    Some(h)
                        if !h.locked
                            && matches!(
                                h.zone,
                                HitZone::Volume | HitZone::FadeIn | HitZone::FadeOut
                            ) =>
                    {
                        self.begin_audio_drag(h, start);
                        ui.ctx().request_repaint();
                    }
                    Some(_) => {}
                    None => {
                        self.marquee = Some(Marquee::new(start, additive, self.selection.clone()));
                        ui.ctx().request_repaint();
                    }
                }
            }
        }

        if let Some(mut m) = self.marquee.take() {
            if let Some(pos) = response.interact_pointer_pos() {
                m.current = pos;
            }
            let rect = m.rect(content);
            let before = m.before.clone();
            let marquee_additive = m.additive;
            let hits = self.marquee_hits(body, content, origin, view_start, rect);
            let mut next = before;
            if !marquee_additive {
                next.clear();
            }
            for hit in hits {
                next.add(hit);
            }
            self.selection = next;
            self.marquee = Some(m);
        }

        if self.audio_drag.is_some()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.update_audio_drag(pos);
        }

        if response.drag_stopped() {
            if self.audio_drag.is_some() {
                self.finish_audio_drag();
            }
            if self.marquee.is_some() {
                self.marquee = None;
            }
            self.hover = pointer
                .filter(|p| content.contains(*p))
                .and_then(|p| self.clip_hit_at(body, content, origin, view_start, p));
            ui.ctx().request_repaint();
        }

        if self.marquee.is_none()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            match self.clip_hit_at(body, content, origin, view_start, pos) {
                Some(hit) if !hit.locked => {
                    if modifiers.ctrl || modifiers.command {
                        self.selection.toggle(hit.r);
                    } else if self.selection.contains(hit.r) && self.selection.len() > 1 {
                        self.selection.select_only(hit.r);
                    } else {
                        self.selection.select_only(hit.r);
                    }
                }
                Some(_) => {}
                None => self.selection.clear(),
            }
            ui.ctx().request_repaint();
        }

        if let Some(hit) = self.hover {
            if hit.locked {
                ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
            } else if matches!(
                hit.zone,
                HitZone::TrimLeft | HitZone::TrimRight | HitZone::FadeIn | HitZone::FadeOut
            ) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            } else if hit.zone == HitZone::Volume {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            } else {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
        } else if pointer.is_some_and(|p| content.contains(p)) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        }
    }

    /// Lane rows in screen space for the current vertical scroll. The canvas and the fixed column
    /// share this one list, so the rows on both sides cannot drift apart.
    fn lane_rows(&self, body: Rect) -> Vec<Rect> {
        let mut y = body.top() - self.scroll.y;
        self.model
            .tracks
            .iter()
            .map(|track| {
                let row = Rect::from_min_size(
                    egui::pos2(body.left(), y),
                    Vec2::new(body.width(), track.height()),
                );
                y += track.height() + model::LANE_GAP;
                row
            })
            .collect()
    }

    /// Canvas layer: lane backgrounds, clips and playhead, clipped to `body`, which runs under the
    /// fixed column. Returns the lane rows so the foreground column can align with them.
    fn lanes(
        &mut self,
        ui: &mut Ui,
        l: &Layout,
        body: Rect,
        origin: f32,
        view_start: f64,
        playhead_x: f32,
    ) -> Vec<Rect> {
        let painter = ui.painter().with_clip_rect(body);
        painter.rect_filled(body, CornerRadius::ZERO, CANVAS_BG);

        // The fixed column's own area never receives canvas-content interactions.
        let content = Rect::from_min_max(egui::pos2(l.controls.right(), body.top()), body.max);
        if self.model.is_empty() {
            painter.text(
                content.center(),
                Align2::CENTER_CENTER,
                "Drag media here to start a timeline",
                FontId::proportional(13.0),
                DIM,
            );
            return Vec::new();
        }

        let rows = self.lane_rows(body);
        for (index, lane) in rows.iter().enumerate() {
            if lane.bottom() < body.top() || lane.top() > body.bottom() {
                continue;
            }
            // Backgrounds span the full body width; the fixed column is painted over them later.
            let clipped = Rect::from_min_max(
                egui::pos2(body.left(), lane.top().max(body.top())),
                egui::pos2(body.right(), lane.bottom().min(body.bottom())),
            );
            painter.rect_filled(clipped, CornerRadius::ZERO, draw::lane_bg(index));
            self.lane_content(ui, body, content, *lane, index, origin, view_start);
        }

        // Playhead across every lane; the foreground column covers the part scrolled beneath it.
        if playhead_x >= body.left() && playhead_x <= body.right() {
            painter.line_segment(
                [
                    egui::pos2(playhead_x, body.top()),
                    egui::pos2(playhead_x, body.bottom()),
                ],
                Stroke::new(1.5, draw::PLAYHEAD),
            );
        }
        rows
    }

    /// Clips of one lane. They are painted clipped to the full `body` (so a horizontal scroll slides
    /// them beneath the fixed column) but hit-tested only inside `content`, right of the column.
    #[allow(clippy::too_many_arguments)]
    fn lane_content(
        &mut self,
        ui: &mut Ui,
        body: Rect,
        content: Rect,
        lane: Rect,
        index: usize,
        origin: f32,
        view_start: f64,
    ) {
        let painter = ui.painter().with_clip_rect(body);
        let Timeline {
            model,
            media,
            selection,
            hover,
            audio_drag,
            zoom,
            ..
        } = self;
        let track = &model.tracks[index];
        if !track.visible {
            return;
        }
        let kind = track.kind();
        let clip_h = kind
            .map(|k| k.clip_height())
            .unwrap_or(track.height() - 4.0);
        let clip_y = lane.top() + (lane.height() - clip_h) * 0.5;
        for (clip_index, clip) in track.clips.iter().enumerate() {
            let x = origin + ((clip.start - view_start) as f32 * *zoom);
            let w = (clip.duration as f32 * *zoom).max(3.0);
            let rect = Rect::from_min_size(egui::pos2(x, clip_y), Vec2::new(w, clip_h));
            let visible = rect.intersect(body);
            if visible.width() < 2.0 || visible.height() < 2.0 {
                continue;
            }
            let r = ClipRef {
                track: index,
                clip: clip_index,
            };
            let has_audio = if clip.kind == model::TrackKind::Video {
                clip.source
                    .as_deref()
                    .is_some_and(|source| media.as_mut().is_some_and(|m| m.has_audio(source)))
            } else {
                clip.kind == model::TrackKind::Audio
            };
            let g = interaction::clip_geometry(clip.kind, rect, has_audio);
            let is_selected = selection.contains(r);
            let hovered = hover.filter(|h| h.r == r);

            draw::draw_clip_body(&painter, visible, clip.kind);
            paint_clip_content(
                media.as_mut(),
                &painter,
                body,
                rect,
                clip,
                *zoom,
                model.frame_rate,
                track.muted,
            );
            if let Some(h) = hovered
                && !h.locked
                && clip.enabled
            {
                painter.rect_filled(visible, CornerRadius::same(4), Color32::from_white_alpha(4));
            }

            // Selection is white for every selected clip. Only the primary gets trim handles.
            if is_selected {
                draw::draw_clip_outline(&painter, rect, Color32::WHITE, 1.5);
            } else if let Some(h) = hovered
                && !h.locked
                && clip.enabled
            {
                draw::draw_clip_outline(
                    &painter,
                    rect,
                    Color32::from_rgba_unmultiplied(76, 141, 255, 210),
                    1.5,
                );
            }

            let label_rect = clip_label_rect(clip.kind, rect, content);
            draw::draw_clip_label(&painter, label_rect, &clip_label(clip, model.frame_rate));

            if clip.enabled {
                if let Some(h) = hovered
                    && matches!(h.zone, HitZone::TrimLeft | HitZone::TrimRight)
                    && !h.locked
                    && !selection.is_primary(r)
                {
                    let edge = if h.zone == HitZone::TrimLeft {
                        Rect::from_min_max(
                            egui::pos2(rect.left(), rect.top() + 4.0),
                            egui::pos2(rect.left() + 2.0, rect.bottom() - 4.0),
                        )
                    } else {
                        Rect::from_min_max(
                            egui::pos2(rect.right() - 2.0, rect.top() + 4.0),
                            egui::pos2(rect.right(), rect.bottom() - 4.0),
                        )
                    };
                    draw::draw_trim_handle(&painter, edge.intersect(content));
                }
                if selection.is_primary(r) {
                    if let Some(left) = interaction::primary_handle_rect(&g, content, true) {
                        draw::draw_trim_handle(&painter, left);
                    }
                    if let Some(right) = interaction::primary_handle_rect(&g, content, false) {
                        draw::draw_trim_handle(&painter, right);
                    }
                }
                if let Some(h) = hovered
                    && h.zone == HitZone::Audio
                    && let Some(audio) = g.audio
                {
                    let audio = audio.intersect(content);
                    if audio.width() > 1.0 && audio.height() > 1.0 {
                        painter.rect_filled(
                            audio,
                            CornerRadius::ZERO,
                            Color32::from_white_alpha(9),
                        );
                    }
                }

                // Audio controls are hover-only, and never appear on locked/muted/disabled clips.
                // The envelope and nodes are Arcline-owned shapes; the waveform itself is only
                // scaled by the same gain/fade parameters, never clipped away.
                let dragging = audio_drag.as_ref().is_some_and(|d| d.r == r);
                if has_audio && !track.locked && !track.muted && (hovered.is_some() || dragging) {
                    if let Some(c) = audio::controls(&g, &clip.audio, clip.duration) {
                        audio::draw(
                            &painter.with_clip_rect(content),
                            &c,
                            &clip.audio,
                            hovered.map(|h| h.zone),
                            dragging,
                        );
                    }
                }
            }

            if !clip.enabled {
                painter.rect_filled(
                    visible,
                    CornerRadius::same(4),
                    Color32::from_black_alpha(150),
                );
            } else if track.locked {
                painter.rect_filled(
                    visible,
                    CornerRadius::same(4),
                    Color32::from_black_alpha(30),
                );
                draw::draw_lock_badge(
                    &painter,
                    egui::pos2(g.header.right() - 9.0, g.header.center().y),
                    Color32::from_rgb(0xc9, 0xcd, 0xd3),
                );
            } else if track.muted
                && let Some(audio) = g.audio.or(g.wave)
            {
                let audio = audio.intersect(content);
                if audio.width() > 1.0 && audio.height() > 1.0 {
                    painter.rect_filled(audio, CornerRadius::ZERO, Color32::from_black_alpha(115));
                    draw::draw_mute_badge(
                        &painter,
                        egui::pos2(audio.right() - 9.0, audio.center().y),
                        Color32::from_rgb(0xb8, 0xbe, 0xc6),
                    );
                }
            }
        }
    }

    /// Foreground layer: the fixed control column, painted opaquely over the canvas so clips and the
    /// playhead always disappear beneath it, plus the boundary that separates the two layers.
    fn control_panel(&mut self, ui: &mut Ui, l: &Layout, body: Rect, rows: &[Rect]) {
        let painter = ui.painter();
        painter.rect_filled(l.header, CornerRadius::ZERO, CONTROLS_BG);
        painter.rect_filled(l.controls, CornerRadius::ZERO, CONTROLS_BG);
        // Continue the ruler baseline under the header cell, so the column reads as one piece.
        painter.line_segment(
            [
                egui::pos2(l.header.left(), l.header.bottom() - 0.5),
                egui::pos2(l.header.right(), l.header.bottom() - 0.5),
            ],
            Stroke::new(1.0, draw::FAINT),
        );
        draw::draw_panel_edge(painter, l.header.union(l.controls), body);

        let column = ui.painter().with_clip_rect(l.controls);
        for (index, row) in rows.iter().enumerate() {
            if row.bottom() < l.controls.top() || row.top() > l.controls.bottom() {
                continue;
            }
            let clipped = Rect::from_min_max(
                egui::pos2(l.controls.left(), row.top().max(l.controls.top())),
                egui::pos2(l.controls.right(), row.bottom().min(l.controls.bottom())),
            );
            let hovered = ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|p| clipped.contains(p));
            if hovered {
                column.rect_filled(
                    clipped,
                    CornerRadius::ZERO,
                    Color32::from_rgb(0x1c, 0x1f, 0x24),
                );
            }
            draw::draw_track_controls(
                ui,
                &column,
                l.controls,
                clipped,
                &mut self.model.tracks[index],
                index,
            );
        }
    }

    fn scrollbars(&mut self, ui: &mut Ui, canvas: Rect, body: Rect) {
        let content_h = self.model.content_height();
        let content_w = self.model.content_width(self.zoom);
        if content_h > body.height() {
            let bar = Rect::from_min_max(
                egui::pos2(canvas.right() - SCROLLBAR_W, canvas.top()),
                egui::pos2(canvas.right(), body.bottom()),
            );
            let thumb = draw::draw_scrollbar(ui.painter(), bar, content_h, self.scroll.y, true);
            // A 9 pt bar is hard to grab; the hit area extends 6 pt into the canvas.
            let hit = Rect::from_min_max(bar.min - Vec2::new(6.0, 0.0), bar.max);
            let id = ui.make_persistent_id("tl-scroll-v");
            let response = ui.interact(hit, id, Sense::drag());
            if response.dragged() {
                let max = (content_h - body.height()).max(1.0);
                let travel = (bar.height() - thumb.height()).max(1.0);
                self.scroll.y += response.drag_delta().y / travel * max;
            }
        }
        if content_w > body.width() {
            let bar = Rect::from_min_max(
                egui::pos2(canvas.left(), canvas.bottom() - SCROLLBAR_W),
                egui::pos2(body.right(), canvas.bottom()),
            );
            let thumb = draw::draw_scrollbar(ui.painter(), bar, content_w, self.scroll.x, false);
            let hit = Rect::from_min_max(bar.min - Vec2::new(0.0, 6.0), bar.max);
            let id = ui.make_persistent_id("tl-scroll-h");
            let response = ui.interact(hit, id, Sense::drag());
            if response.dragged() {
                let max = (content_w - body.width()).max(1.0);
                let travel = (bar.width() - thumb.width()).max(1.0);
                self.scroll.x += response.drag_delta().x / travel * max;
            }
        }
        let max = self.max_scroll(canvas);
        self.scroll = Vec2::new(
            self.scroll.x.clamp(0.0, max.x),
            self.scroll.y.clamp(0.0, max.y),
        );
    }
}

/// Header rect a clip label may occupy, clipped to the visible content area.
///
/// The intersection with `content` is what keeps the label at the leftmost visible part of a
/// horizontally scrolled clip and off the fixed control column. Text/effect/sticker clips use the
/// whole clip; media clips use their dedicated header strip.
fn clip_label_rect(kind: model::TrackKind, clip_rect: Rect, content: Rect) -> Rect {
    let label = match kind {
        model::TrackKind::Video => Rect::from_min_size(
            clip_rect.min,
            Vec2::new(clip_rect.width(), draw::VIDEO_HEADER_H),
        ),
        model::TrackKind::Image => Rect::from_min_size(
            clip_rect.min,
            Vec2::new(clip_rect.width(), draw::IMAGE_HEADER_H),
        ),
        model::TrackKind::Audio => Rect::from_min_size(
            clip_rect.min,
            Vec2::new(clip_rect.width(), draw::AUDIO_HEADER_H),
        ),
        _ => clip_rect,
    };
    label.intersect(content)
}

/// Label of one clip: actual text/effect/sticker content and `HH:MM:SS:FF` durations, never a
/// decimal duration. Text clips carry only their text; media, effects and stickers also show the
/// clip duration on the same line when the clip is wide enough.
fn clip_label(clip: &model::Clip, frame_rate: FrameRate) -> String {
    let duration = draw::timecode_ndf(clip.duration, frame_rate);
    match clip.kind {
        model::TrackKind::Text => clip.name.clone(),
        model::TrackKind::Effect | model::TrackKind::Sticker => {
            format!("{}  {duration}", clip.name)
        }
        _ => {
            let name = clip.source_name().unwrap_or_else(|| clip.name.clone());
            format!("{name}  {duration}")
        }
    }
}

/// Paint the decoded media of one clip into its internal regions. The clip rectangle is the full
/// clip; the painter is already clipped to the canvas body. Missing or undecodable sources get the
/// explicit fallback, never a fake thumbnail or waveform.
fn paint_clip_content(
    media: Option<&mut MediaHub>,
    painter: &egui::Painter,
    body: Rect,
    clip_rect: Rect,
    clip: &model::Clip,
    zoom: f32,
    frame_rate: FrameRate,
    muted: bool,
) {
    if !matches!(
        clip.kind,
        model::TrackKind::Video | model::TrackKind::Image | model::TrackKind::Audio
    ) {
        return;
    }
    let visible = clip_rect.intersect(body);
    if visible.width() < 2.0 || visible.height() < 4.0 {
        return;
    }
    let Some(media) = media else {
        draw::draw_missing(painter, visible, "media unavailable");
        return;
    };
    let Some(source) = clip.source.as_deref() else {
        draw::draw_missing(painter, visible, "media unavailable");
        return;
    };
    if media.failure(source).is_some() {
        draw::draw_missing(painter, visible, "media unavailable");
        return;
    }
    match clip.kind {
        model::TrackKind::Video => {
            let has_audio = media.has_audio(source);
            let header_h = draw::VIDEO_HEADER_H;
            let thumb_h = if has_audio {
                draw::VIDEO_THUMB_H
            } else {
                (clip_rect.height() - header_h).max(1.0)
            };
            let thumb_rect = Rect::from_min_size(
                egui::pos2(clip_rect.left(), clip_rect.top() + header_h),
                Vec2::new(clip_rect.width(), thumb_h),
            );
            paint_video(
                media, painter, body, clip_rect, thumb_rect, source, zoom, frame_rate,
            );
            if has_audio {
                let audio_rect = Rect::from_min_size(
                    egui::pos2(clip_rect.left(), thumb_rect.bottom()),
                    Vec2::new(clip_rect.width(), draw::VIDEO_AUDIO_H),
                );
                paint_video_audio(
                    media,
                    painter,
                    body,
                    clip_rect,
                    audio_rect,
                    source,
                    zoom,
                    &clip.audio,
                    clip.duration,
                    muted,
                );
            }
        }
        model::TrackKind::Image => {
            let image_rect = Rect::from_min_size(
                egui::pos2(clip_rect.left(), clip_rect.top() + draw::IMAGE_HEADER_H),
                Vec2::new(
                    clip_rect.width(),
                    (clip_rect.height() - draw::IMAGE_HEADER_H).max(1.0),
                ),
            );
            paint_image(media, painter, body, clip_rect, image_rect, source);
        }
        model::TrackKind::Audio => {
            let wave_rect = Rect::from_min_size(
                egui::pos2(clip_rect.left(), clip_rect.top() + draw::AUDIO_HEADER_H),
                Vec2::new(clip_rect.width(), draw::AUDIO_WAVE_H),
            );
            paint_audio(
                media,
                painter,
                body,
                clip_rect,
                wave_rect,
                source,
                zoom,
                &clip.audio,
                clip.duration,
                muted,
            );
        }
        _ => {}
    }
}

/// Real decoded frames, tiled continuously across the thumbnail region. Each tile is one source
/// timestamp; the tile width is a ladder-quantised source-time step, so thumbnails stay cached while
/// the timeline scrolls. `uv_fill` centre-crops each frame into its tile, so nothing is distorted.
#[allow(clippy::too_many_arguments)]
fn paint_video(
    media: &mut MediaHub,
    painter: &egui::Painter,
    body: Rect,
    clip_rect: Rect,
    thumb_rect: Rect,
    source: &Path,
    zoom: f32,
    frame_rate: FrameRate,
) {
    let zoom = zoom.max(1.0);
    let visible = thumb_rect.intersect(body);
    if visible.width() < 2.0 || visible.height() < 2.0 {
        return;
    }
    let painter = painter.with_clip_rect(visible);
    let tile_w = (thumb_rect.height() * 16.0 / 9.0).max(24.0);
    let (source_rate, vfr) = media.video_sampling(source, frame_rate);
    // Tiles step by PROJECT frames, not source frames. A 24 fps source in a 30 fps project shows
    // the same source frame on the project frames where no new source frame exists; the media cache
    // keys CFR frames by source index, so repeats share one texture and no fake frame is decoded.
    let project_fps = frame_rate.rate().to_f64_lossy();
    let step_frames = media::thumbnail_step_frames(zoom, tile_w, project_fps);
    let step = media::thumbnail_step_seconds(step_frames, project_fps);
    let view_start = ((body.left() - clip_rect.left()) / zoom) as f64;
    let view_end = ((body.right() - clip_rect.left()) / zoom) as f64;
    let clip_dur = (clip_rect.width() / zoom) as f64;
    let tiles = media::visible_tiles(0.0, clip_dur, step, view_start, view_end);
    let tile_px = (step as f32 * zoom).max(1.0);
    for index in tiles {
        let rect = Rect::from_min_size(
            egui::pos2(clip_rect.left() + index as f32 * tile_px, thumb_rect.top()),
            Vec2::new(tile_px, thumb_rect.height()),
        );
        if !rect.intersects(visible) {
            continue;
        }
        // Start of the project-frame span represented by this tile. This is the same exact
        // `FrameRate::time_of_frame` used by preview/export; a midpoint would disagree with a
        // lower-rate source near the end of its display interval.
        let time = media::thumbnail_sample_time(frame_rate, index as u64, step_frames);
        if let Some(visual) = media.video_frame_at(source, time, source_rate, vfr) {
            let uv = draw::uv_fill(visual.aspect, rect);
            painter.image(visual.texture.id(), rect, uv, Color32::WHITE);
        }
    }
}

/// Embedded audio of a video, drawn in the reserved bottom strip with the same decoded envelope and
/// bottom-anchored bar style as the audio tracks.
fn paint_video_audio(
    media: &mut MediaHub,
    painter: &egui::Painter,
    body: Rect,
    clip_rect: Rect,
    audio_rect: Rect,
    source: &Path,
    zoom: f32,
    audio: &ClipAudio,
    duration: f64,
    muted: bool,
) {
    let Some(wave) = media.waveform(source) else {
        return;
    };
    let visible = audio_rect.intersect(body);
    if visible.width() < 3.0 || visible.height() < 4.0 {
        return;
    }
    let rect = Rect::from_min_max(
        egui::pos2(visible.left() + 2.0, visible.top() + 1.0),
        egui::pos2(
            (visible.right() - 2.0).max(visible.left() + 3.0),
            (visible.bottom() - 1.0).max(visible.top() + 2.0),
        ),
    );
    draw::draw_waveform_bars(
        painter,
        rect,
        clip_rect.left(),
        zoom.max(1.0),
        &wave,
        if muted {
            Color32::from_rgba_unmultiplied(
                draw::WAVE_EMBEDDED.r(),
                draw::WAVE_EMBEDDED.g(),
                draw::WAVE_EMBEDDED.b(),
                105,
            )
        } else {
            draw::WAVE_EMBEDDED
        },
        audio,
        duration,
    );
}

/// The decoded still image, repeated across the image region. Every repeat keeps the image's
/// proportions via `uv_fill` (centre crop), so the row has no empty areas and no distortion.
fn paint_image(
    media: &mut MediaHub,
    painter: &egui::Painter,
    body: Rect,
    clip_rect: Rect,
    image_rect: Rect,
    source: &Path,
) {
    let Some(still) = media.still(source) else {
        return;
    };
    let visible = image_rect.intersect(body);
    if visible.width() < 2.0 || visible.height() < 2.0 {
        return;
    }
    let painter = painter.with_clip_rect(visible);
    let tile_w = 72.0f32.max(image_rect.height() * 1.2);
    let mut x = clip_rect.left();
    while x < clip_rect.right() {
        let rect = Rect::from_min_size(
            egui::pos2(x, image_rect.top()),
            Vec2::new(tile_w, image_rect.height()),
        );
        if rect.intersects(visible) {
            let uv = draw::uv_fill(still.aspect, rect);
            painter.image(still.texture.id(), rect, uv, Color32::WHITE);
        }
        x += tile_w;
    }
}

/// The decoded waveform envelope for an audio clip, bottom-anchored bars in the 30 pt wave region.
fn paint_audio(
    media: &mut MediaHub,
    painter: &egui::Painter,
    body: Rect,
    clip_rect: Rect,
    wave_rect: Rect,
    source: &Path,
    zoom: f32,
    audio: &ClipAudio,
    duration: f64,
    muted: bool,
) {
    let Some(wave) = media.waveform(source) else {
        return;
    };
    let visible = wave_rect.intersect(body);
    if visible.width() < 3.0 || visible.height() < 4.0 {
        return;
    }
    let rect = Rect::from_min_max(
        egui::pos2(visible.left() + 2.0, visible.top() + 1.0),
        egui::pos2(
            (visible.right() - 2.0).max(visible.left() + 3.0),
            (visible.bottom() - 1.0).max(visible.top() + 2.0),
        ),
    );
    draw::draw_waveform_bars(
        painter,
        rect,
        clip_rect.left(),
        zoom.max(1.0),
        &wave,
        if muted {
            Color32::from_rgba_unmultiplied(
                draw::WAVE_BLUE.r(),
                draw::WAVE_BLUE.g(),
                draw::WAVE_BLUE.b(),
                95,
            )
        } else {
            draw::WAVE_BLUE
        },
        audio,
        duration,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 500.0))
    }

    /// One frame of the timeline UI with the given input events.
    fn frame(ctx: &egui::Context, t: &mut Timeline, screen: Rect, events: Vec<egui::Event>) {
        frame_mod(ctx, t, screen, events, egui::Modifiers::NONE);
    }

    fn frame_mod(
        ctx: &egui::Context,
        t: &mut Timeline,
        screen: Rect,
        mut events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| t.ui(ui, screen));
        out.textures_delta.clear();
    }

    #[test]
    fn layout_aligns_ruler_and_canvas_over_the_control_column() {
        let l = layout(rect());
        assert_eq!(l.toolbar.height(), TOOLBAR_H);
        assert_eq!(l.ruler.top(), l.toolbar.bottom());
        assert_eq!(l.header.top(), l.ruler.top());
        assert_eq!(l.header.height(), RULER_H);
        assert_eq!(l.header.right(), l.canvas.left());
        assert_eq!(l.controls.right(), l.canvas.left());
        assert_eq!(l.controls.top(), l.canvas.top());
        assert_eq!(l.controls.width(), CONTROL_W);
        assert_eq!(l.ruler.left(), l.canvas.left());
        assert_eq!(l.ruler.width(), l.canvas.width());
        // The canvas paints into a full-width underlay that runs beneath the fixed column.
        assert_eq!(l.underlay.left(), l.header.left());
        assert_eq!(l.underlay.width(), l.header.width() + l.canvas.width());
        assert_eq!(l.underlay.top(), l.canvas.top());
    }

    #[test]
    fn lane_rows_are_fixed_across_horizontal_scroll() {
        let mut t = Timeline::default();
        let body = Rect::from_min_size(egui::pos2(0.0, 56.0), egui::vec2(900.0, 300.0));
        let rows = t.lane_rows(body);
        assert_eq!(rows.len(), t.model.tracks.len());
        for (row, track) in rows.iter().zip(&t.model.tracks) {
            assert_eq!(row.height(), track.height());
            assert_eq!(row.left(), body.left());
            assert_eq!(row.width(), body.width());
        }
        assert_eq!(rows[0].height(), model::TrackKind::Text.lane_height());
        assert!(rows.iter().any(|row| row.height() == 76.0));
        assert!(rows.iter().any(|row| row.height() == 50.0));
        for pair in rows.windows(2) {
            assert_eq!(pair[1].top(), pair[0].bottom() + model::LANE_GAP);
        }

        // Horizontal scroll slides the canvas under the rows; it never moves or resizes them.
        t.scroll.x = 480.0;
        assert_eq!(
            t.lane_rows(body),
            rows,
            "horizontal scroll moved the lane rows"
        );

        // Vertical scroll moves the rows with the lanes, keeping the same fixed heights.
        t.scroll.y = 20.0;
        let shifted = t.lane_rows(body);
        assert_eq!(shifted[0].top(), rows[0].top() - 20.0);
        for (row, track) in shifted.iter().zip(&t.model.tracks) {
            assert_eq!(row.height(), track.height());
        }
    }

    #[test]
    fn clip_labels_stick_to_the_visible_left_edge_and_off_the_panel() {
        let content = Rect::from_min_max(egui::pos2(112.0, 0.0), egui::pos2(500.0, 100.0));

        // A video clipped on the left by the fixed column starts its label at the content edge.
        let partly_under_panel =
            Rect::from_min_size(egui::pos2(40.0, 0.0), egui::vec2(200.0, 72.0));
        let label = clip_label_rect(model::TrackKind::Video, partly_under_panel, content);
        assert_eq!(label.left(), content.left());
        assert_eq!(label.height(), draw::VIDEO_HEADER_H);
        assert_eq!(label.right(), partly_under_panel.right());

        // A clip wholly inside the content keeps its own left edge.
        let inside = Rect::from_min_size(egui::pos2(200.0, 0.0), egui::vec2(100.0, 72.0));
        assert_eq!(
            clip_label_rect(model::TrackKind::Video, inside, content).left(),
            200.0
        );

        // A clip wholly left of the content has no drawable label.
        let behind_panel = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(80.0, 72.0));
        assert!(clip_label_rect(model::TrackKind::Video, behind_panel, content).width() < 14.0);

        // Audio uses its shorter 16 pt header; text/effect/sticker use the whole 20 pt clip.
        let audio = Rect::from_min_size(egui::pos2(200.0, 0.0), egui::vec2(100.0, 46.0));
        assert_eq!(
            clip_label_rect(model::TrackKind::Audio, audio, content).height(),
            draw::AUDIO_HEADER_H
        );
        let text = Rect::from_min_size(egui::pos2(200.0, 0.0), egui::vec2(100.0, 20.0));
        assert_eq!(
            clip_label_rect(model::TrackKind::Text, text, content).height(),
            20.0
        );
    }

    #[test]
    fn clip_labels_use_frame_timecode_without_decimals() {
        let model = model::TimelineModel::mock();
        let clips: Vec<&model::Clip> = model.tracks.iter().flat_map(|t| t.clips.iter()).collect();

        let video = clips
            .iter()
            .find(|c| c.source_name().as_deref() == Some("city_street.mp4"))
            .expect("city street clip");
        assert_eq!(
            clip_label(video, model.frame_rate),
            "city_street.mp4  00:00:05:00"
        );
        assert!(!clip_label(video, model.frame_rate).contains(".0s"));

        let effect = clips
            .iter()
            .find(|c| c.name == "调色 · 暖阳")
            .expect("effect clip");
        assert_eq!(
            clip_label(effect, model.frame_rate),
            "调色 · 暖阳  00:00:05:00"
        );

        let text = clips
            .iter()
            .find(|c| c.name == "开场 · 山海之间")
            .expect("text clip");
        assert_eq!(clip_label(text, model.frame_rate), "开场 · 山海之间");

        let audio = clips
            .iter()
            .find(|c| c.source_name().as_deref() == Some("narration.wav"))
            .expect("narration clip");
        assert_eq!(
            clip_label(audio, model.frame_rate),
            "narration.wav  00:00:18:00"
        );

        // The duration label follows the project rate, not a fixed 24 fps.
        assert_eq!(
            clip_label(video, FrameRate::Fps29_97),
            "city_street.mp4  00:00:05:00"
        );
        assert_eq!(
            clip_label(audio, FrameRate::Fps30),
            "narration.wav  00:00:18:00"
        );
    }

    #[test]
    fn view_start_and_zoom_follow_the_scroll() {
        let mut t = Timeline::default();
        t.scroll.x = 58.0 * 3.0;
        assert!((t.view_start() - 3.0).abs() < 0.001);
        t.zoom = 116.0;
        t.scroll.x = 0.0;
        assert_eq!(t.view_start(), 0.0);
    }

    #[test]
    fn max_scroll_accounts_for_content_and_viewport() {
        let t = Timeline::default();
        let canvas = layout(rect()).canvas;
        let max = t.max_scroll(canvas);
        let content_w = t.model.content_width(t.zoom);
        assert!((max.x - (content_w - canvas.width()).max(0.0)).abs() < 0.01);

        // In the real card the canvas is much shorter than the sample lanes; a short viewport must scroll.
        let short = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 300.0));
        let small = layout(short).canvas;
        let max = t.max_scroll(small);
        assert!(max.y > 0.0, "the sample tracks overflow a {small:?} canvas");
    }

    #[test]
    fn empty_timeline_has_no_content_and_no_scroll() {
        let t = Timeline::empty();
        assert!(t.model.is_empty());
        assert_eq!(t.model.content_height(), 0.0);
        assert_eq!(t.model.duration, 0.0);
    }

    #[test]
    fn playhead_position_matches_time_and_scroll() {
        let t = Timeline::default();
        let origin = 112.0;
        let x = time_to_x(origin, t.view_start(), t.zoom, t.model.playhead);
        assert!((x - (origin + t.model.playhead as f32 * t.zoom)).abs() < 0.01);
        // The drag path inverts the same mapping, so a dragged playhead lands under the pointer.
        assert!((x_to_time(origin, t.view_start(), t.zoom, x) - t.model.playhead).abs() < 1e-9);
    }

    /// The scrollbar thumbs must be draggable; this exercises the same hit rects the app uses.
    #[test]
    fn dragging_the_scrollbars_changes_the_scroll_offsets() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let l = layout(screen);
        let body = t.body_rect(l.underlay);
        let vbar = Rect::from_min_max(
            egui::pos2(l.canvas.right() - SCROLLBAR_W, l.canvas.top()),
            egui::pos2(l.canvas.right(), body.bottom()),
        );
        let v = vbar.center();
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(v)]);
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerButton {
                pos: v,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerMoved(v + egui::vec2(0.0, 40.0))],
        );
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerButton {
                pos: v + egui::vec2(0.0, 40.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            t.scroll.y > 0.0,
            "vertical scrollbar drag: scroll.y={}",
            t.scroll.y
        );

        let hbar = Rect::from_min_max(
            egui::pos2(l.canvas.left(), l.canvas.bottom() - SCROLLBAR_W),
            egui::pos2(body.right(), l.canvas.bottom()),
        );
        let h = hbar.center();
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(h)]);
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerButton {
                pos: h,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerMoved(h + egui::vec2(60.0, 0.0))],
        );
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerButton {
                pos: h + egui::vec2(60.0, 0.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            t.scroll.x > 0.0,
            "horizontal scrollbar drag: scroll.x={}",
            t.scroll.x
        );
    }

    /// Lane heights are fixed by content type: a drag on a lane edge must neither resize the lane
    /// nor scroll the canvas, and no lane-edge hit region exists at all.
    #[test]
    fn dragging_a_lane_edge_does_not_resize_it() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());

        let l = layout(screen);
        let body = Rect::from_min_max(
            l.canvas.min,
            egui::pos2(
                l.canvas.right() - SCROLLBAR_W,
                l.canvas.bottom() - SCROLLBAR_W,
            ),
        );
        let heights =
            |t: &Timeline| -> Vec<f32> { t.model.tracks.iter().map(|tr| tr.height()).collect() };
        let before = heights(&t);
        let content_h = t.model.content_height();

        // The lower edge of the first (text) lane, in the gap before the next lane.
        let edge = egui::pos2(
            l.canvas.left() + 60.0,
            body.top() + t.model.tracks[0].height(),
        );
        let moved = edge + egui::vec2(0.0, 40.0);
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(edge)]);
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerButton {
                pos: edge,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(moved)]);
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerButton {
                pos: moved,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        assert_eq!(
            heights(&t),
            before,
            "a lane edge drag must not resize tracks"
        );
        assert_eq!(t.model.content_height(), content_h);
        assert_eq!(t.scroll.y, 0.0, "the drag must not scroll either");
    }

    #[test]
    fn horizontal_zoom_does_not_change_track_heights() {
        let mut t = Timeline::default();
        let heights =
            |t: &Timeline| -> Vec<f32> { t.model.tracks.iter().map(|tr| tr.height()).collect() };
        let before = heights(&t);
        let content_h = t.model.content_height();
        let max_zoom = t.max_zoom();
        for zoom in [MIN_ZOOM, 40.0, 58.0, 200.0, 480.0, max_zoom] {
            t.zoom = zoom;
            assert_eq!(heights(&t), before, "zoom {zoom} changed lane heights");
            assert_eq!(t.model.content_height(), content_h);
        }
    }

    /// One click at `pos` (press and release in separate frames, like a real pointer).
    fn click(ctx: &egui::Context, t: &mut Timeline, screen: Rect, pos: egui::Pos2) {
        click_mod(ctx, t, screen, pos, egui::Modifiers::NONE);
    }

    fn click_mod(
        ctx: &egui::Context,
        t: &mut Timeline,
        screen: Rect,
        pos: egui::Pos2,
        modifiers: egui::Modifiers,
    ) {
        frame_mod(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerMoved(pos)],
            modifiers,
        );
        frame_mod(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            }],
            modifiers,
        );
        frame_mod(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
    }

    fn drag(
        ctx: &egui::Context,
        t: &mut Timeline,
        screen: Rect,
        from: egui::Pos2,
        to: egui::Pos2,
        modifiers: egui::Modifiers,
    ) {
        frame_mod(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerMoved(from)],
            modifiers,
        );
        frame_mod(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            }],
            modifiers,
        );
        frame_mod(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerMoved(to)],
            modifiers,
        );
        frame_mod(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
    }

    fn audio_timeline(audio: ClipAudio) -> Timeline {
        let mut track = model::Track::new("Audio");
        track.add_clip(
            model::Clip::new(model::TrackKind::Audio, "tone.wav", 0.0, 10.0).with_audio(audio),
        );
        let mut model = model::TimelineModel::empty();
        model.duration = 10.0;
        model.tracks.push(track);
        Timeline {
            model,
            ..Timeline::default()
        }
    }

    fn audio_controls_at(
        t: &Timeline,
        screen: Rect,
    ) -> (interaction::ClipGeometry, audio::AudioControls) {
        let l = layout(screen);
        let body = t.body_rect(l.underlay);
        let lane = t.lane_rows(body)[0];
        let clip = &t.model.tracks[0].clips[0];
        let origin = l.canvas.left();
        let x = origin + ((clip.start - t.view_start()) as f32 * t.zoom);
        let w = (clip.duration as f32 * t.zoom).max(3.0);
        let h = clip.kind.clip_height();
        let rect = Rect::from_min_size(egui::pos2(x, lane.center().y - h * 0.5), Vec2::new(w, h));
        let g = interaction::clip_geometry(clip.kind, rect, false);
        let c = audio::controls(&g, &clip.audio, clip.duration).expect("audio controls");
        (g, c)
    }

    fn ctrl_z(ctx: &egui::Context, t: &mut Timeline, screen: Rect, shift: bool) {
        let mut m = egui::Modifiers::CTRL;
        m.shift = shift;
        frame_mod(
            ctx,
            t,
            screen,
            vec![egui::Event::Key {
                key: Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: m,
            }],
            m,
        );
    }

    #[test]
    fn dragging_the_volume_node_updates_gain_waveform_and_undo_redo() {
        let ctx = egui::Context::default();
        let mut t = audio_timeline(ClipAudio::default());
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let (_, c) = audio_controls_at(&t, screen);
        let start = c.volume_node.center();
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(start)]);
        assert_eq!(
            t.hover.map(|h| h.zone),
            Some(HitZone::Volume),
            "volume node hover: start={start:?} controls={c:?} content={:?}",
            layout(screen).canvas
        );
        drag(
            &ctx,
            &mut t,
            screen,
            start,
            start - egui::vec2(0.0, 18.0),
            egui::Modifiers::NONE,
        );
        let gain = t.model.tracks[0].clips[0].audio.gain_db;
        assert!(gain > 0.0, "volume drag did not raise gain: {gain}");
        assert!(t.audio_history.can_undo());
        ctrl_z(&ctx, &mut t, screen, false);
        assert_eq!(t.model.tracks[0].clips[0].audio.gain_db, 0.0);
        ctrl_z(&ctx, &mut t, screen, true);
        assert!((t.model.tracks[0].clips[0].audio.gain_db - gain).abs() < 1e-9);
    }

    #[test]
    fn dragging_the_left_and_right_diamonds_sets_fade_in_and_fade_out() {
        let ctx = egui::Context::default();
        let mut t = audio_timeline(ClipAudio::default());
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let (_, c) = audio_controls_at(&t, screen);
        let left = c.fade_in_node.center();
        drag(
            &ctx,
            &mut t,
            screen,
            left,
            left + egui::vec2(40.0, 0.0),
            egui::Modifiers::NONE,
        );
        assert!(t.model.tracks[0].clips[0].audio.fade_in.to_f64_lossy() > 0.0);

        let (_, c) = audio_controls_at(&t, screen);
        let right = c.fade_out_node.center();
        drag(
            &ctx,
            &mut t,
            screen,
            right,
            right - egui::vec2(40.0, 0.0),
            egui::Modifiers::NONE,
        );
        assert!(t.model.tracks[0].clips[0].audio.fade_out.to_f64_lossy() > 0.0);
    }

    #[test]
    fn muted_locked_and_disabled_audio_clips_cannot_be_edited() {
        for mode in 0..3 {
            let ctx = egui::Context::default();
            let mut t = audio_timeline(ClipAudio::default());
            match mode {
                0 => t.model.tracks[0].muted = true,
                1 => t.model.tracks[0].locked = true,
                _ => t.model.tracks[0].clips[0].enabled = false,
            }
            let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
            frame(&ctx, &mut t, screen, Vec::new());
            let (_, c) = audio_controls_at(&t, screen);
            let start = c.volume_node.center();
            drag(
                &ctx,
                &mut t,
                screen,
                start,
                start - egui::vec2(0.0, 18.0),
                egui::Modifiers::NONE,
            );
            assert_eq!(
                t.model.tracks[0].clips[0].audio.gain_db, 0.0,
                "mode {mode} edited a protected clip"
            );
        }
    }

    #[test]
    fn audio_controls_work_at_different_zooms() {
        for zoom in [29.0f32, 58.0, 116.0] {
            let ctx = egui::Context::default();
            let mut t = audio_timeline(ClipAudio::default());
            t.zoom = zoom;
            let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
            frame(&ctx, &mut t, screen, Vec::new());
            let (_, c) = audio_controls_at(&t, screen);
            let start = c.volume_node.center();
            drag(
                &ctx,
                &mut t,
                screen,
                start,
                start - egui::vec2(0.0, 12.0),
                egui::Modifiers::NONE,
            );
            assert!(
                t.model.tracks[0].clips[0].audio.gain_db > 0.0,
                "zoom {zoom} did not update gain"
            );
        }
    }

    #[test]
    fn a_video_with_embedded_audio_uses_the_same_controls() {
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(240.0, 72.0));
        let g = interaction::clip_geometry(model::TrackKind::Video, rect, true);
        let c = audio::controls(&g, &ClipAudio::default(), 5.0).expect("embedded controls");
        assert!(g.audio.is_some());
        assert!(g.wave.is_none());
        assert_eq!(
            audio::hit_test(&c, c.volume_node.center()),
            Some(HitZone::Volume)
        );
    }

    fn clip_pos(t: &Timeline, screen: Rect, track: usize, clip: usize, dx: f32) -> egui::Pos2 {
        let l = layout(screen);
        let body = t.body_rect(l.underlay);
        let lane = t.lane_rows(body)[track];
        let c = &t.model.tracks[track].clips[clip];
        egui::pos2(
            l.canvas.left() + ((c.start - t.view_start()) as f32 * t.zoom) + dx,
            lane.center().y,
        )
    }

    /// The fixed column is a foreground layer: a clip scrolled underneath it is not clickable there,
    /// while the visible part of the same clip right of the boundary still is.
    #[test]
    fn the_fixed_column_swallows_clicks_on_clips_scrolled_beneath_it() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let l = layout(screen);
        let body = t.body_rect(l.underlay);

        // Scroll right so the first text clip (0..2.8 s) starts beneath the fixed column.
        t.scroll.x = 120.0;
        frame(&ctx, &mut t, screen, Vec::new());
        assert!(t.view_start() > 0.0);

        let row_y = body.top() + 10.0;
        // A click over the column (x = 100) lands on the clip that is scrolled underneath.
        click(&ctx, &mut t, screen, egui::pos2(100.0, row_y));
        assert!(
            t.selection.is_empty(),
            "a click on the fixed column must not select a clip beneath it"
        );

        // Right of the boundary the visible part of the same clip is reachable.
        click(
            &ctx,
            &mut t,
            screen,
            egui::pos2(l.canvas.left() + 20.0, row_y),
        );
        assert_eq!(t.selection.primary(), Some(ClipRef { track: 0, clip: 0 }));
    }

    #[test]
    fn ctrl_click_adds_and_plain_click_collapses_the_selection() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());

        let a = clip_pos(&t, screen, 0, 0, 20.0);
        let b = clip_pos(&t, screen, 0, 1, 20.0);
        click(&ctx, &mut t, screen, a);
        assert_eq!(t.selection.primary(), Some(ClipRef { track: 0, clip: 0 }));

        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        click_mod(&ctx, &mut t, screen, b, ctrl);
        assert_eq!(t.selection.len(), 2);
        assert_eq!(t.selection.primary(), Some(ClipRef { track: 0, clip: 1 }));

        // A plain click on an already-selected clip collapses the multi-selection.
        click(&ctx, &mut t, screen, a);
        assert_eq!(t.selection.len(), 1);
        assert_eq!(t.selection.primary(), Some(ClipRef { track: 0, clip: 0 }));

        // Ctrl-click removes the primary and falls back to the remaining item.
        click_mod(&ctx, &mut t, screen, a, ctrl);
        assert!(t.selection.is_empty());
    }

    #[test]
    fn locked_hidden_and_disabled_clips_are_not_selectable() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let p = clip_pos(&t, screen, 0, 0, 20.0);

        // Locked track: the clip is visible but cannot be selected.
        t.model.tracks[0].locked = true;
        click(&ctx, &mut t, screen, p);
        assert!(t.selection.is_empty());
        let hover = {
            frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(p)]);
            t.hover
        };
        assert!(
            hover.is_some_and(|h| h.locked),
            "locked hover must be explicit"
        );
        t.model.tracks[0].locked = false;

        // Hidden track: no selection and no hover.
        t.model.tracks[0].visible = false;
        click(&ctx, &mut t, screen, p);
        assert!(t.selection.is_empty());
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(p)]);
        assert!(t.hover.is_none(), "hidden clips must not be hit");
        t.model.tracks[0].visible = true;

        // Disabled clip: still painted, but not selectable or hoverable.
        t.model.tracks[0].clips[0] = t.model.tracks[0].clips[0].clone().with_enabled(false);
        click(&ctx, &mut t, screen, p);
        assert!(t.selection.is_empty());
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(p)]);
        assert!(t.hover.is_none(), "disabled clips must not be hit");
    }

    #[test]
    fn marquee_selects_a_region_and_escape_clears() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        t.zoom = 12.0;
        t.scroll.x = 0.0;
        frame(&ctx, &mut t, screen, Vec::new());

        let l = layout(screen);
        let body = t.body_rect(l.underlay);
        let row_y = body.top() + 8.0;
        // Start after the text track's last clip (20 s * 12 px/s = 240 px), drag left across it.
        let from = egui::pos2(l.canvas.left() + 380.0, row_y);
        let to = egui::pos2(l.canvas.left() + 10.0, row_y + 8.0);
        drag(&ctx, &mut t, screen, from, to, egui::Modifiers::NONE);
        assert_eq!(t.selection.len(), 6, "marquee must select all text clips");
        assert_eq!(t.selection.primary(), Some(ClipRef { track: 0, clip: 5 }));

        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(t.selection.is_empty(), "Escape clears the selection");
    }

    #[test]
    fn additive_marquee_keeps_the_existing_selection() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        t.zoom = 12.0;
        t.scroll.x = 0.0;
        t.selection.select_only(ClipRef { track: 1, clip: 0 });
        frame(&ctx, &mut t, screen, Vec::new());

        let l = layout(screen);
        let body = t.body_rect(l.underlay);
        let row_y = body.top() + 8.0;
        let additive = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        drag(
            &ctx,
            &mut t,
            screen,
            egui::pos2(l.canvas.left() + 380.0, row_y),
            egui::pos2(l.canvas.left() + 10.0, row_y + 8.0),
            additive,
        );
        assert!(
            t.selection.contains(ClipRef { track: 1, clip: 0 }),
            "Ctrl marquee must keep the prior selection"
        );
        assert_eq!(t.selection.len(), 7, "one effect clip plus six text clips");
    }

    #[test]
    fn hover_distinguishes_body_and_real_edges_but_not_viewport_edges() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let l = layout(screen);
        let body = t.body_rect(l.underlay);
        let row = t.lane_rows(body)[0];
        let left = clip_pos(&t, screen, 0, 0, 1.0);

        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(left)]);
        assert_eq!(t.hover.map(|h| h.zone), Some(HitZone::TrimLeft));

        let body_pos = egui::pos2(left.x + 20.0, row.center().y);
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerMoved(body_pos)],
        );
        assert_eq!(t.hover.map(|h| h.zone), Some(HitZone::Body));

        // Scroll so the clip's real left edge is off-screen: the viewport edge is body, not trim.
        t.scroll.x = 120.0;
        frame(&ctx, &mut t, screen, Vec::new());
        let viewport_left = egui::pos2(l.canvas.left() + 1.0, row.center().y);
        frame(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerMoved(viewport_left)],
        );
        assert_eq!(t.hover.map(|h| h.zone), Some(HitZone::Body));
    }

    #[test]
    fn selection_is_pruned_when_a_track_becomes_locked_or_hidden() {
        let mut t = Timeline::default();
        t.selection.select_only(ClipRef { track: 0, clip: 0 });
        t.model.tracks[0].locked = true;
        t.prune_selection();
        assert!(t.selection.is_empty());

        t.selection.select_only(ClipRef { track: 1, clip: 0 });
        t.model.tracks[1].visible = false;
        t.prune_selection();
        assert!(t.selection.is_empty());

        // Muting only changes the audio rendering; it must not remove the clip from selection.
        t.selection.select_only(ClipRef { track: 4, clip: 0 });
        t.model.tracks[4].muted = true;
        t.prune_selection();
        assert!(t.selection.contains(ClipRef { track: 4, clip: 0 }));
    }

    /// The playhead as painted in one frame: the handle centre and the vertical line, with their
    /// vertical extents.
    #[derive(Debug, Clone, Copy)]
    struct PaintedPlayhead {
        handle_x: f32,
        handle_top: f32,
        handle_bottom: f32,
        line_x: f32,
        line_top: f32,
        line_bottom: f32,
    }

    /// Run one frame and read the painted playhead shapes out of it. Fails the test when either the
    /// handle or the line is missing, so a regression cannot silently pass.
    fn painted_playhead(
        ctx: &egui::Context,
        t: &mut Timeline,
        screen: Rect,
        events: Vec<egui::Event>,
    ) -> PaintedPlayhead {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| t.ui(ui, screen));
        out.textures_delta.clear();

        let mut handle = None;
        let mut line = None;
        for clipped in &out.shapes {
            match &clipped.shape {
                egui::Shape::Path(path) if path.closed && path.fill == draw::PLAYHEAD => {
                    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                    let (mut top, mut bottom) = (f32::INFINITY, f32::NEG_INFINITY);
                    for p in &path.points {
                        lo = lo.min(p.x);
                        hi = hi.max(p.x);
                        top = top.min(p.y);
                        bottom = bottom.max(p.y);
                    }
                    handle = Some(((lo + hi) * 0.5, top, bottom));
                }
                egui::Shape::LineSegment { points, stroke } if stroke.color == draw::PLAYHEAD => {
                    line = Some((
                        points[0].x,
                        points[0].y.min(points[1].y),
                        points[0].y.max(points[1].y),
                    ));
                }
                _ => {}
            }
        }
        let (handle_x, handle_top, handle_bottom) =
            handle.expect("the ruler paints the playhead handle");
        let (line_x, line_top, line_bottom) = line.expect("the canvas paints the playhead line");
        PaintedPlayhead {
            handle_x,
            handle_top,
            handle_bottom,
            line_x,
            line_top,
            line_bottom,
        }
    }

    #[test]
    fn playhead_handle_and_line_share_one_x_across_scroll_and_zoom() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        let l = layout(screen);
        let body = t.body_rect(l.underlay);
        painted_playhead(&ctx, &mut t, screen, Vec::new());

        for zoom in [MIN_ZOOM, 58.0, 120.0, 480.0, t.max_zoom()] {
            t.zoom = zoom;
            let max = t.max_scroll(l.canvas).x;
            // Place the playhead at 25 %, 50 % and 75 % of the visible canvas width, which stays
            // inside the view even at the extreme zoom levels.
            for fraction in [0.25f64, 0.5, 0.75] {
                let cursor = (l.canvas.width() as f64 * fraction) / zoom as f64;
                let scroll = ((t.model.playhead - cursor) * zoom as f64) as f32;
                t.scroll.x = scroll.clamp(0.0, max);
                let p = painted_playhead(&ctx, &mut t, screen, Vec::new());

                assert!(
                    (p.handle_x - p.line_x).abs() < 0.01,
                    "zoom {zoom}, scroll {}: handle x {} vs line x {}",
                    t.scroll.x,
                    p.handle_x,
                    p.line_x
                );
                let expected = time_to_x(l.canvas.left(), t.view_start(), t.zoom, t.model.playhead);
                assert!(
                    (p.line_x - expected).abs() < 0.01,
                    "the line must use the shared time-to-pixel mapping"
                );
                // The handle stays inside the ruler, the line runs through the tracks.
                assert!(p.handle_top >= l.ruler.top() && p.handle_bottom <= l.ruler.bottom());
                assert!((p.line_top - body.top()).abs() < 0.01);
                assert!((p.line_bottom - body.bottom()).abs() < 0.01);
            }
        }
    }

    #[test]
    fn horizontal_scroll_moves_the_handle_and_the_line_together() {
        let ctx = egui::Context::default();
        let mut t = Timeline {
            zoom: 120.0, // make the 20 s mock wider than the viewport
            ..Timeline::default()
        };
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        let max = t.max_scroll(layout(screen).canvas).x;
        assert!(max > 400.0);
        painted_playhead(&ctx, &mut t, screen, Vec::new());
        let mut previous = painted_playhead(&ctx, &mut t, screen, Vec::new());

        for step in [40.0f32, 60.0, 100.0] {
            let before_scroll = t.scroll.x;
            t.scroll.x = (t.scroll.x + step).min(max);
            let shift = t.scroll.x - before_scroll;
            assert!(shift > 0.0);
            let now = painted_playhead(&ctx, &mut t, screen, Vec::new());

            assert!(
                (previous.handle_x - now.handle_x - shift).abs() < 0.01,
                "the handle must follow the scroll"
            );
            assert!(
                (previous.line_x - now.line_x - shift).abs() < 0.01,
                "the line must follow the scroll"
            );
            assert!((now.handle_x - now.line_x).abs() < 0.01);
            previous = now;
        }
    }

    #[test]
    fn dragging_the_playhead_moves_the_handle_and_the_line_in_the_same_frame() {
        let ctx = egui::Context::default();
        // Scrolled, but with the playhead still on screen at the start of the drag.
        let mut t = Timeline {
            zoom: 58.0,
            scroll: egui::vec2(60.0, 0.0),
            ..Timeline::default()
        };
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        let l = layout(screen);
        let origin = l.canvas.left();
        painted_playhead(&ctx, &mut t, screen, Vec::new());

        let start = egui::pos2(origin + 100.0, l.ruler.center().y);
        let end = start + egui::vec2(240.0, 0.0);
        painted_playhead(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(start)]);
        painted_playhead(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let dragged = painted_playhead(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(end)]);

        // Same frame: the drag already moved both elements, so the handle cannot trail the line.
        assert!(
            (dragged.handle_x - dragged.line_x).abs() < 0.01,
            "handle x {} vs line x {}",
            dragged.handle_x,
            dragged.line_x
        );
        let expected_time = model::snap_to_frame(
            x_to_time(origin, t.view_start(), t.zoom, end.x),
            t.model.frame_rate,
        );
        let expected_x = time_to_x(origin, t.view_start(), t.zoom, expected_time);
        assert!(
            (dragged.line_x - expected_x).abs() < 0.01,
            "the line is on the snapped frame"
        );
        let half_frame_px = t.zoom / t.frame_rate().rate().to_f64_lossy() as f32 / 2.0 + 0.01;
        assert!(
            (dragged.line_x - end.x).abs() <= half_frame_px,
            "the snapped line stays within half a frame of the pointer"
        );
        assert!(
            (dragged.handle_x - dragged.line_x).abs() < 0.01,
            "the handle centre is on the snapped frame"
        );
        assert!((t.model.playhead - expected_time).abs() < 1e-9);
    }

    /// Clicking the ruler and dragging both snap to the nearest project frame.
    #[test]
    fn ruler_interaction_snaps_the_playhead_to_a_frame() {
        let ctx = egui::Context::default();
        let mut t = Timeline {
            zoom: 120.0,
            ..Timeline::default()
        };
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 500.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let l = layout(screen);

        let target = 2.03;
        let x = time_to_x(l.canvas.left(), t.view_start(), t.zoom, target);
        click(&ctx, &mut t, screen, egui::pos2(x, l.ruler.center().y));
        let expected = model::snap_to_frame(target, t.model.frame_rate);
        assert_eq!(t.model.playhead, expected);
        assert!((expected - 2.041_666_666_666_666_5).abs() < 1e-9);
    }

    /// Ctrl+wheel calls this anchor path; the time under the pointer must not drift.
    #[test]
    fn wheel_zoom_keeps_the_time_under_the_pointer() {
        let mut t = Timeline::default();
        let origin = 142.0;
        let pointer_x = origin + 320.0;
        let before = x_to_time(origin, t.view_start(), t.zoom, pointer_x);
        let before_zoom = t.zoom;
        t.zoom_at_pointer(pointer_x, origin, 1.25);
        assert!(t.zoom > before_zoom, "wheel factor did not zoom");
        let after = x_to_time(origin, t.view_start(), t.zoom, pointer_x);
        assert!(
            (after - before).abs() < 1e-6,
            "pointer time drifted {before} -> {after}"
        );
    }

    /// Toolbar zoom anchors on the playhead's pixel when the playhead is visible.
    #[test]
    fn toolbar_zoom_keeps_the_playhead_pixel() {
        let ctx = egui::Context::default();
        let mut t = Timeline {
            zoom: 58.0,
            ..Timeline::default()
        };
        t.model.playhead = 5.0;
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 500.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let l = layout(screen);
        let before = time_to_x(l.canvas.left(), t.view_start(), t.zoom, t.model.playhead);
        let z = zoom_controls(l.toolbar).expect("room for the zoom cluster");

        click(&ctx, &mut t, screen, z.input.center());
        let after = time_to_x(l.canvas.left(), t.view_start(), t.zoom, t.model.playhead);
        assert!(t.zoom > 58.0, "zoom did not change: {}", t.zoom);
        assert!(
            (after - before).abs() < 0.01,
            "playhead moved from x {before} to {after} during toolbar zoom"
        );
    }

    /// A window resize re-resolves the canvas and keeps the handle/line on the same time.
    #[test]
    fn resizing_keeps_the_playhead_mapped_to_its_time() {
        let ctx = egui::Context::default();
        let mut t = Timeline {
            zoom: 120.0,
            scroll: egui::vec2(180.0, 0.0),
            ..Timeline::default()
        };
        for size in [
            egui::vec2(800.0, 420.0),
            egui::vec2(1600.0, 900.0),
            egui::vec2(1000.0, 500.0),
        ] {
            let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), size);
            let p = painted_playhead(&ctx, &mut t, screen, Vec::new());
            let l = layout(screen);
            let expected = time_to_x(l.canvas.left(), t.view_start(), t.zoom, t.model.playhead);
            assert!(
                (p.line_x - expected).abs() < 0.01,
                "size {size:?}: line x {} vs {expected}",
                p.line_x
            );
            assert!((p.handle_x - p.line_x).abs() < 0.01);
            let body = t.body_rect(l.underlay);
            assert!((p.line_top - body.top()).abs() < 0.01);
            assert!((p.line_bottom - body.bottom()).abs() < 0.01);
        }
    }

    #[test]
    fn tick_steps_grow_when_zoomed_out() {
        assert!(draw::tick_step(400.0) <= draw::tick_step(40.0));
        assert!(draw::tick_step(40.0) <= draw::tick_step(4.0));
        assert_eq!(draw::tick_step(4000.0), 1.0);
        assert_eq!(draw::tick_step(58.0), 2.0);
    }

    #[test]
    fn zoom_controls_are_right_aligned_and_compact() {
        let toolbar = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, TOOLBAR_H));
        let z = zoom_controls(toolbar).expect("a 1200 pt toolbar has room");
        assert!((z.input.right() - (toolbar.right() - TOOLBAR_MARGIN)).abs() < 0.01);
        assert!(z.out.left() > toolbar.left() + TOOLBAR_TOOLS_W);
        assert!(z.out.right() < z.slider.left());
        assert!(z.slider.right() < z.input.left());
        assert!((z.slider.width() - ZOOM_SLIDER_W).abs() < 0.01);
        assert_eq!(z.out.width(), ZOOM_BUTTON);
        assert_eq!(z.input.width(), ZOOM_BUTTON);

        // A narrow timeline keeps the editing tools and simply drops the zoom cluster.
        let narrow = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(360.0, TOOLBAR_H));
        assert!(zoom_controls(narrow).is_none());
    }

    #[test]
    fn the_zoom_slider_covers_the_whole_scale() {
        let max_zoom = Timeline::default().max_zoom();
        assert!((zoom_from_fraction(0.0, max_zoom) - MIN_ZOOM).abs() < 0.01);
        assert!((zoom_from_fraction(1.0, max_zoom) - max_zoom).abs() < 0.01);
        let default = Timeline::default().zoom;
        let fraction = zoom_slider_fraction(default, max_zoom);
        assert!((zoom_from_fraction(fraction, max_zoom) - default).abs() < 0.01);
        // Logarithmic: the midpoint is the geometric mean, not the arithmetic one.
        assert!((zoom_from_fraction(0.5, max_zoom) - (MIN_ZOOM * max_zoom).sqrt()).abs() < 0.01);
    }

    /// Clicking the right-hand zoom-in/out buttons and dragging the slider all change the scale.
    #[test]
    fn toolbar_zoom_controls_change_the_zoom() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let max_zoom = t.max_zoom();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 500.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let toolbar = layout(screen).toolbar;
        let z = zoom_controls(toolbar).expect("room for the zoom cluster");

        let before = t.zoom;
        click(&ctx, &mut t, screen, z.input.center());
        assert!(t.zoom > before, "zoom in: {} -> {}", before, t.zoom);
        let after_in = t.zoom;
        click(&ctx, &mut t, screen, z.out.center());
        assert!(t.zoom < after_in, "zoom out: {} -> {}", after_in, t.zoom);

        // Clicking at the right end of the slider reaches the dynamic maximum.
        let track =
            Rect::from_center_size(z.slider.center(), egui::vec2(z.slider.width() - 10.0, 3.0));
        click(
            &ctx,
            &mut t,
            screen,
            egui::pos2(track.right(), track.center().y),
        );
        assert!(
            (t.zoom - max_zoom).abs() < 0.5,
            "slider right edge: {} (max {max_zoom})",
            t.zoom
        );
        // And the left end reaches the minimum.
        click(
            &ctx,
            &mut t,
            screen,
            egui::pos2(track.left(), track.center().y),
        );
        assert!(
            (t.zoom - MIN_ZOOM).abs() < 0.5,
            "slider left edge: {}",
            t.zoom
        );
    }

    #[test]
    fn max_zoom_scales_with_the_project_frame_rate() {
        for (rate, expected) in [
            (FrameRate::Fps24, 4_800.0),
            (FrameRate::Fps25, 5_000.0),
            (FrameRate::Fps30, 6_000.0),
            (FrameRate::Fps29_97, 5_994.0),
            (FrameRate::Fps59_94, 11_988.0),
        ] {
            let t = Timeline::new(rate);
            assert!(
                (t.max_zoom() - expected).abs() < 0.1,
                "{rate:?}: {} vs {expected}",
                t.max_zoom()
            );
            assert_eq!(t.frame_rate(), rate);
        }
    }
}
