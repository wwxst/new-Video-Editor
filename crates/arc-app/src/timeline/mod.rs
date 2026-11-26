//! Timeline module: toolbar, ruler, track-control column and the lane canvas, driven by mock data.
//!
//! Everything here is presentation and local state (zoom, scroll, selection);
//! no engine types are touched, so the mock model can be replaced by the real document feed later.
//! Lane heights are fixed by content type and are not editable state.

pub mod draw;
pub mod model;

use draw::{CANVAS_BG, CONTROLS_BG, DIM, TOOLBAR_BG, ToolbarIcon};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, Vec2};
use model::TimelineModel;

/// Width of the left track-control column, in points.
pub const CONTROL_W: f32 = 112.0;
/// Toolbar height, in points.
pub const TOOLBAR_H: f32 = 34.0;
/// Ruler height, in points.
pub const RULER_H: f32 = 22.0;
/// Scrollbar thickness, in points.
pub const SCROLLBAR_W: f32 = 9.0;
/// Zoom limits, in points per second.
pub const MIN_ZOOM: f32 = 12.0;
pub const MAX_ZOOM: f32 = 480.0;

fn tb(ui: &mut Ui, x: &mut f32, y: f32, icon: ToolbarIcon, active: bool) -> egui::Response {
    let r = Rect::from_center_size(egui::pos2(*x + 12.0, y), Vec2::new(24.0, 24.0));
    *x += 26.0;
    let id = ui.make_persistent_id(("tl-toolbar", icon));
    draw::icon_button(ui, r, id, icon, active)
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
#[derive(Debug, Clone)]
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
        }
    }
}

impl Timeline {
    /// Empty timeline (used by the empty-state rendering and tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn empty() -> Timeline {
        Timeline {
            model: TimelineModel::empty(),
            ..Timeline::default()
        }
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
        ui.painter()
            .rect_filled(l.toolbar, CornerRadius::ZERO, TOOLBAR_BG);
        ui.painter()
            .rect_filled(l.underlay, CornerRadius::ZERO, CANVAS_BG);

        self.toolbar(ui, l.toolbar);
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
    /// the line move in the same frame as the pointer.
    fn drag_playhead(&mut self, ui: &mut Ui, ruler: Rect, view_start: f64, origin: f32) {
        let id = ui.make_persistent_id("tl-playhead");
        let response = ui.interact(ruler, id, Sense::click_and_drag());
        if (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let t = x_to_time(origin, view_start, self.zoom, pos.x);
            self.model.playhead = t.clamp(0.0, self.model.duration.max(0.0));
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

    fn toolbar(&mut self, ui: &mut Ui, rect: Rect) {
        let y = rect.center().y;
        let mut x = rect.left() + 8.0;
        let mut zoom_delta = 0.0f32;
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
        x += 6.0;
        if tb(ui, &mut x, y, ToolbarIcon::ZoomOut, false)
            .on_hover_text("Zoom out")
            .clicked()
        {
            zoom_delta = -1.0;
        }
        let label_rect = Rect::from_center_size(egui::pos2(x + 30.0, y), Vec2::new(60.0, 20.0));
        x += 62.0;
        ui.painter().text(
            label_rect.center(),
            Align2::CENTER_CENTER,
            format!("{:.0} px/s", self.zoom),
            FontId::proportional(10.0),
            DIM,
        );
        if tb(ui, &mut x, y, ToolbarIcon::ZoomIn, false)
            .on_hover_text("Zoom in")
            .clicked()
        {
            zoom_delta = 1.0;
        }
        if zoom_delta != 0.0 {
            self.zoom = (self.zoom * if zoom_delta > 0.0 { 1.25 } else { 0.8 })
                .clamp(MIN_ZOOM, MAX_ZOOM);
        }

        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.bottom() - 0.5),
                egui::pos2(rect.right(), rect.bottom() - 0.5),
            ],
            Stroke::new(1.0, Color32::from_rgb(0x23, 0x26, 0x2c)),
        );
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
            let origin = l.canvas.left();
            let view_start = self.view_start();
            let t = x_to_time(origin, view_start, self.zoom, pos.x);
            let factor: f32 = (1.0 + delta.y * 0.004).clamp(0.8, 1.25);
            self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            // Keep the time under the pointer where it is while zooming.
            self.scroll.x = (t as f32 * self.zoom - (pos.x - origin)).max(0.0);
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
        let track = &self.model.tracks[index];
        let kind = track.kind();
        let clip_h = kind.map(|k| k.clip_height()).unwrap_or(track.height() - 4.0);
        let clip_y = lane.top() + (lane.height() - clip_h) * 0.5;
        for (clip_index, clip) in track.clips.iter().enumerate() {
            let x = origin + ((clip.start - view_start) as f32 * self.zoom);
            let w = (clip.duration as f32 * self.zoom).max(3.0);
            let rect = Rect::from_min_size(egui::pos2(x, clip_y), Vec2::new(w, clip_h));
            let visible = rect.intersect(body);
            if visible.width() < 2.0 || visible.height() < 2.0 {
                continue;
            }
            let selected = self.selected == Some((index, clip_index));
            draw::draw_clip(&painter, visible, clip, selected);

            let hit = visible.intersect(content);
            if hit.width() < 2.0 || hit.height() < 2.0 {
                continue;
            }
            let id = ui.make_persistent_id(("tl-clip", index, clip_index));
            let response = ui.interact(hit, id, Sense::click());
            if response.clicked() {
                self.selected = Some((index, clip_index));
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
        assert_eq!(rows[0].height(), 24.0);
        assert_eq!(rows[3].height(), 76.0);
        assert_eq!(rows[6].height(), 50.0);
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

        // In the real card the canvas is much shorter than eight lanes; a short viewport must scroll.
        let short = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 300.0));
        let small = layout(short).canvas;
        let max = t.max_scroll(small);
        assert!(max.y > 0.0, "eight mock tracks overflow a {small:?} canvas");
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
        for zoom in [MIN_ZOOM, 40.0, 58.0, 200.0, MAX_ZOOM] {
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

        for zoom in [MIN_ZOOM, 58.0, 120.0, MAX_ZOOM] {
            t.zoom = zoom;
            let max = t.max_scroll(l.canvas).x;
            for cursor in [0.25f64, 0.5, 1.0] {
                // Place the playhead `cursor` seconds into the visible canvas.
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
        assert!((dragged.line_x - end.x).abs() < 0.01, "the line is under the pointer");
        assert!(
            (dragged.handle_x - end.x).abs() < 0.01,
            "the handle centre is under the pointer"
        );
        let expected_time = x_to_time(origin, t.view_start(), t.zoom, end.x);
        assert!((t.model.playhead - expected_time).abs() < 0.001);
    }

    #[test]
    fn tick_steps_grow_when_zoomed_out() {
        assert!(draw::tick_step(400.0) <= draw::tick_step(40.0));
        assert!(draw::tick_step(40.0) <= draw::tick_step(4.0));
        assert_eq!(draw::tick_step(4000.0), 0.5);
    }
}
