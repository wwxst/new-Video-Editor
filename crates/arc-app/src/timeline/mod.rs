//! Timeline module: toolbar, ruler, track-control column and the lane canvas, driven by mock data.
//!
//! Everything here is presentation and local state (zoom, scroll, selection);
//! no engine types are touched, so the mock model can be replaced by the real document feed later.
//! Lane heights are fixed by content type and are not editable state.

pub mod draw;
pub mod media;
pub mod model;

use arc_core::time::FrameRate;
use draw::{CANVAS_BG, CONTROLS_BG, DIM, TOOLBAR_BG, ToolbarIcon};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, Vec2};
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
    Some(ZoomControls {
        out,
        slider,
        input,
    })
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
    pub selected: Option<(usize, usize)>,
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
            .field("selected", &self.selected)
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
            selected: self.selected,
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
            selected: None,
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
        self.scroll = Vec2::new(self.scroll.x.clamp(0.0, max.x), self.scroll.y.clamp(0.0, max.y));
        let view_start = self.view_start();

        let body = self.body_rect(l.underlay);
        let origin = l.canvas.left();

        // Resolve the playhead before painting: a drag in this frame must move the handle and the
        // line together, and both are painted from this one x.
        self.drag_playhead(ui, l.ruler, view_start, origin);
        let playhead_x = self.playhead_x(origin, view_start);

        self.ruler(ui, l.ruler, view_start, playhead_x);
        let rows = self.lanes(ui, &l, body, origin, view_start, playhead_x);
        self.control_panel(ui, &l, body, &rows);
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
            egui::pos2(underlay.right() - SCROLLBAR_W, underlay.bottom() - SCROLLBAR_W),
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
            self.paint_zoom_slider(ui, track, fraction, response.dragged() || response.hovered());
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
        painter.rect_filled(track, CornerRadius::same(2), Color32::from_rgb(0x33, 0x37, 0x3f));
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
        let (delta, ctrl, shift) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.modifiers.ctrl,
                i.modifiers.shift,
            )
        });
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
            selected,
            zoom,
            ..
        } = self;
        let track = &model.tracks[index];
        let kind = track.kind();
        let clip_h = kind.map(|k| k.clip_height()).unwrap_or(track.height() - 4.0);
        let clip_y = lane.top() + (lane.height() - clip_h) * 0.5;
        for (clip_index, clip) in track.clips.iter().enumerate() {
            let x = origin + ((clip.start - view_start) as f32 * *zoom);
            let w = (clip.duration as f32 * *zoom).max(3.0);
            let rect = Rect::from_min_size(egui::pos2(x, clip_y), Vec2::new(w, clip_h));
            let visible = rect.intersect(body);
            if visible.width() < 2.0 || visible.height() < 2.0 {
                continue;
            }
            let is_selected = *selected == Some((index, clip_index));
            draw::draw_clip_body(&painter, visible, clip.kind);
            paint_clip_content(
                media.as_mut(),
                &painter,
                body,
                rect,
                clip,
                *zoom,
                model.frame_rate,
            );
            draw::draw_clip_outline(&painter, visible, is_selected);
            let label_rect = clip_label_rect(clip.kind, rect, content);
            draw::draw_clip_label(&painter, label_rect, &clip_label(clip, model.frame_rate));

            let hit = visible.intersect(content);
            if hit.width() < 2.0 || hit.height() < 2.0 {
                continue;
            }
            let id = ui.make_persistent_id(("tl-clip", index, clip_index));
            let response = ui.interact(hit, id, Sense::click());
            if response.clicked() {
                *selected = Some((index, clip_index));
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
                column.rect_filled(clipped, CornerRadius::ZERO, Color32::from_rgb(0x1c, 0x1f, 0x24));
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
        self.scroll = Vec2::new(self.scroll.x.clamp(0.0, max.x), self.scroll.y.clamp(0.0, max.y));
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
                media,
                painter,
                body,
                clip_rect,
                thumb_rect,
                source,
                zoom,
                frame_rate,
            );
            if has_audio {
                let audio_rect = Rect::from_min_size(
                    egui::pos2(clip_rect.left(), thumb_rect.bottom()),
                    Vec2::new(clip_rect.width(), draw::VIDEO_AUDIO_H),
                );
                paint_video_audio(media, painter, body, clip_rect, audio_rect, source, zoom);
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
            paint_audio(media, painter, body, clip_rect, wave_rect, source, zoom);
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
    let source_rate = media.video_rate(source, frame_rate);
    let source_fps = source_rate.rate().to_f64_lossy();
    let step_frames = media::thumb_step_frames(zoom, tile_w, source_fps);
    let step = media::thumb_step_seconds(step_frames, source_fps);
    let view_start = ((body.left() - clip_rect.left()) / zoom) as f64;
    let view_end = ((body.right() - clip_rect.left()) / zoom) as f64;
    let clip_dur = (clip_rect.width() / zoom) as f64;
    let tiles = media::visible_tiles(0.0, clip_dur, step, view_start, view_end);
    let tile_px = (step as f32 * zoom).max(1.0);
    for index in tiles {
        let rect = Rect::from_min_size(
            egui::pos2(
                clip_rect.left() + index as f32 * tile_px,
                thumb_rect.top(),
            ),
            Vec2::new(tile_px, thumb_rect.height()),
        );
        if !rect.intersects(visible) {
            continue;
        }
        // Aim at the centre of the source frame range using a frame index, never by accumulating
        // floating-point seconds; `frame_at` resolves the actual PTS.
        let frame_index = index as u64 * step_frames;
        let time = source_rate
            .time_of_frame(frame_index as i64)
            .to_f64_lossy()
            + (step_frames as f64 / 2.0) / source_fps;
        if let Some(visual) = media.video_frame(source, time) {
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
        draw::WAVE_EMBEDDED,
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
        draw::WAVE_BLUE,
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
        assert_eq!(t.lane_rows(body), rows, "horizontal scroll moved the lane rows");

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
        let partly_under_panel = Rect::from_min_size(egui::pos2(40.0, 0.0), egui::vec2(200.0, 72.0));
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
        assert!(
            clip_label_rect(model::TrackKind::Video, behind_panel, content).width() < 14.0
        );

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
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerButton {
            pos: v,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(v + egui::vec2(0.0, 40.0))]);
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerButton {
            pos: v + egui::vec2(0.0, 40.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(t.scroll.y > 0.0, "vertical scrollbar drag: scroll.y={}", t.scroll.y);

        let hbar = Rect::from_min_max(
            egui::pos2(l.canvas.left(), l.canvas.bottom() - SCROLLBAR_W),
            egui::pos2(body.right(), l.canvas.bottom()),
        );
        let h = hbar.center();
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(h)]);
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerButton {
            pos: h,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(h + egui::vec2(60.0, 0.0))]);
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerButton {
            pos: h + egui::vec2(60.0, 0.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(t.scroll.x > 0.0, "horizontal scrollbar drag: scroll.x={}", t.scroll.x);
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
            egui::pos2(l.canvas.right() - SCROLLBAR_W, l.canvas.bottom() - SCROLLBAR_W),
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
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerButton {
            pos: edge,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerMoved(moved)]);
        frame(&ctx, &mut t, screen, vec![egui::Event::PointerButton {
            pos: moved,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);

        assert_eq!(heights(&t), before, "a lane edge drag must not resize tracks");
        assert_eq!(t.model.content_height(), content_h);
        assert_eq!(t.scroll.y, 0.0, "the drag must not scroll either");
    }

    #[test]
    fn horizontal_zoom_does_not_change_track_heights() {
        let mut t = Timeline::default();
        let heights = |t: &Timeline| -> Vec<f32> { t.model.tracks.iter().map(|tr| tr.height()).collect() };
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
        frame(ctx, t, screen, vec![egui::Event::PointerMoved(pos)]);
        frame(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            ctx,
            t,
            screen,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
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
        assert_eq!(
            t.selected, None,
            "a click on the fixed column must not select a clip beneath it"
        );

        // Right of the boundary the visible part of the same clip is reachable.
        click(&ctx, &mut t, screen, egui::pos2(l.canvas.left() + 20.0, row_y));
        assert_eq!(t.selected, Some((0, 0)));
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
        painted_playhead(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerMoved(start)],
        );
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
        let dragged = painted_playhead(
            &ctx,
            &mut t,
            screen,
            vec![egui::Event::PointerMoved(end)],
        );

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
        let half_frame_px =
            t.zoom / t.frame_rate().rate().to_f64_lossy() as f32 / 2.0 + 0.01;
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
        assert!(
            (zoom_from_fraction(0.5, max_zoom) - (MIN_ZOOM * max_zoom).sqrt()).abs() < 0.01
        );
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
        let track = Rect::from_center_size(
            z.slider.center(),
            egui::vec2(z.slider.width() - 10.0, 3.0),
        );
        click(&ctx, &mut t, screen, egui::pos2(track.right(), track.center().y));
        assert!(
            (t.zoom - max_zoom).abs() < 0.5,
            "slider right edge: {} (max {max_zoom})",
            t.zoom
        );
        // And the left end reaches the minimum.
        click(&ctx, &mut t, screen, egui::pos2(track.left(), track.center().y));
        assert!((t.zoom - MIN_ZOOM).abs() < 0.5, "slider left edge: {}", t.zoom);
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
