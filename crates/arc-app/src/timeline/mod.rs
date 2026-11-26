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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub toolbar: Rect,
    pub ruler: Rect,
    pub controls: Rect,
    pub canvas: Rect,
}

/// Resolve the timeline areas from the card content rect.
pub fn layout(rect: Rect) -> Layout {
    let w = rect.width().max(1.0);
    let h = rect.height().max(1.0);
    let toolbar = Rect::from_min_size(rect.min, Vec2::new(w, TOOLBAR_H.min(h)));
    let ruler_h = RULER_H.min((h - TOOLBAR_H).max(0.0));
    let ruler = Rect::from_min_size(
        egui::pos2(rect.left() + CONTROL_W.min(w - 1.0), rect.top() + TOOLBAR_H),
        Vec2::new((w - CONTROL_W).max(1.0), ruler_h),
    );
    let body_top = rect.top() + TOOLBAR_H + ruler_h;
    let body_h = (h - TOOLBAR_H - ruler_h).max(0.0);
    let controls = Rect::from_min_size(
        egui::pos2(rect.left(), body_top),
        Vec2::new(CONTROL_W.min(w), body_h),
    );
    let canvas = Rect::from_min_size(
        egui::pos2(rect.left() + CONTROL_W.min(w), body_top),
        Vec2::new((w - CONTROL_W).max(1.0), body_h),
    );
    Layout {
        toolbar,
        ruler,
        controls,
        canvas,
    }
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
    pub fn ui(&mut self, ui: &mut Ui, rect: Rect) {
        let l = layout(rect);
        if l.toolbar.width() < 40.0 || l.canvas.height() < 20.0 {
            return;
        }
        ui.painter()
            .rect_filled(l.toolbar, CornerRadius::ZERO, TOOLBAR_BG);
        ui.painter()
            .rect_filled(l.canvas, CornerRadius::ZERO, CANVAS_BG);
        ui.painter()
            .rect_filled(l.controls, CornerRadius::ZERO, CONTROLS_BG);

        self.toolbar(ui, l.toolbar);
        self.wheel(ui, &l);

        // Clamp scroll against the current content, then resolve the view.
        let max = self.max_scroll(l.canvas);
        self.scroll = Vec2::new(self.scroll.x.clamp(0.0, max.x), self.scroll.y.clamp(0.0, max.y));
        let view_start = self.view_start();

        let body = self.body_rect(l.canvas);
        self.ruler(ui, l.ruler, view_start, body);
        self.lanes(ui, &l, body, view_start);
        self.scrollbars(ui, l.canvas, body);
    }

    /// Canvas minus the scrollbar strips (the lane viewport).
    fn body_rect(&self, canvas: Rect) -> Rect {
        Rect::from_min_max(
            canvas.min,
            egui::pos2(canvas.right() - SCROLLBAR_W, canvas.bottom() - SCROLLBAR_W),
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
        if !(l.canvas.contains(pos) || l.controls.contains(pos) || l.ruler.contains(pos)) {
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
            let t = self.view_start() + ((pos.x - l.canvas.left()) / self.zoom) as f64;
            let factor: f32 = (1.0 + delta.y * 0.004).clamp(0.8, 1.25);
            self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            self.scroll.x = (t as f32 * self.zoom - (pos.x - l.canvas.left())).max(0.0);
        } else if shift {
            self.scroll.x -= delta.y;
        } else {
            self.scroll.y -= delta.y;
            self.scroll.x -= delta.x;
        }
    }

    fn ruler(&mut self, ui: &mut Ui, rect: Rect, view_start: f64, body: Rect) {
        let playhead_x = body.left() + ((self.model.playhead - view_start) as f32 * self.zoom);
        let visible = playhead_x >= body.left() - 2.0 && playhead_x <= body.right() + 2.0;
        draw::draw_ruler(
            ui.painter(),
            rect,
            view_start,
            self.zoom,
            visible.then_some(playhead_x),
        );

        let id = ui.make_persistent_id("tl-playhead");
        let response = ui.interact(rect, id, Sense::click_and_drag());
        if (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let t = view_start + ((pos.x - body.left()) / self.zoom) as f64;
            self.model.playhead = t.clamp(0.0, self.model.duration.max(0.0));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }

    fn lanes(&mut self, ui: &mut Ui, l: &Layout, body: Rect, view_start: f64) {
        let painter = ui.painter().with_clip_rect(body);
        painter.rect_filled(body, CornerRadius::ZERO, CANVAS_BG);

        if self.model.is_empty() {
            painter.text(
                body.center(),
                Align2::CENTER_CENTER,
                "Drag media here to start a timeline",
                FontId::proportional(13.0),
                DIM,
            );
            return;
        }

        // Lane rectangles in content space, shifted by the vertical scroll. Heights are fixed per
        // content type (model::Track::height); zoom only scales the horizontal axis.
        let heights: Vec<f32> = self.model.tracks.iter().map(|t| t.height()).collect();
        let mut y = body.top() - self.scroll.y;
        let mut lanes = Vec::with_capacity(heights.len());
        for height in heights {
            lanes.push(Rect::from_min_size(
                egui::pos2(body.left(), y),
                Vec2::new(body.width(), height),
            ));
            y += height + model::LANE_GAP;
        }

        // Lane backgrounds: alternating subtle tones, no dividers, continuous canvas.
        for (index, lane) in lanes.iter().enumerate() {
            if lane.bottom() < body.top() || lane.top() > body.bottom() {
                continue;
            }
            let clipped = Rect::from_min_max(
                egui::pos2(body.left(), lane.top().max(body.top())),
                egui::pos2(body.right(), lane.bottom().min(body.bottom())),
            );
            painter.rect_filled(clipped, CornerRadius::ZERO, draw::lane_bg(index));
            self.lane_content(ui, body, *lane, index, view_start);
        }

        // Playhead across every lane.
        let x = body.left() + ((self.model.playhead - view_start) as f32 * self.zoom);
        if x >= body.left() && x <= body.right() {
            painter.line_segment(
                [
                    egui::pos2(x, body.top()),
                    egui::pos2(x, body.bottom()),
                ],
                Stroke::new(1.5, draw::PLAYHEAD),
            );
        }

        // Track-control column, vertically in sync with the lanes.
        self.controls(ui, l.controls, &lanes);

        // Vertical hairline between the control column and the canvas.
        painter.line_segment(
            [
                egui::pos2(body.left() - 0.5, body.top()),
                egui::pos2(body.left() - 0.5, body.bottom()),
            ],
            Stroke::new(1.0, Color32::from_rgb(0x22, 0x25, 0x2a)),
        );
    }

    fn lane_content(
        &mut self,
        ui: &mut Ui,
        body: Rect,
        lane: Rect,
        index: usize,
        view_start: f64,
    ) {
        let painter = ui.painter().with_clip_rect(body);
        let track = &self.model.tracks[index];
        let kind = track.kind();
        let clip_h = kind.map(|k| k.clip_height()).unwrap_or(track.height() - 4.0);
        let clip_y = lane.top() + (lane.height() - clip_h) * 0.5;
        for (clip_index, clip) in track.clips.iter().enumerate() {
            let x = body.left() + ((clip.start - view_start) as f32 * self.zoom);
            let w = (clip.duration as f32 * self.zoom).max(3.0);
            let rect = Rect::from_min_size(egui::pos2(x, clip_y), Vec2::new(w, clip_h));
            let visible = rect.intersect(body);
            if visible.width() < 2.0 || visible.height() < 2.0 {
                continue;
            }
            let selected = self.selected == Some((index, clip_index));
            draw::draw_clip(&painter, visible, clip, selected);

            let id = ui.make_persistent_id(("tl-clip", index, clip_index));
            let response = ui.interact(visible, id, Sense::click());
            if response.clicked() {
                self.selected = Some((index, clip_index));
            }
        }
    }

    fn controls(&mut self, ui: &mut Ui, column: Rect, lanes: &[Rect]) {
        let painter = ui.painter().with_clip_rect(column);
        for (index, lane) in lanes.iter().enumerate() {
            if lane.bottom() < column.top() || lane.top() > column.bottom() {
                continue;
            }
            let clipped = Rect::from_min_max(
                egui::pos2(column.left(), lane.top().max(column.top())),
                egui::pos2(column.right(), lane.bottom().min(column.bottom())),
            );
            let hovered = ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|p| clipped.contains(p));
            if hovered {
                painter.rect_filled(clipped, CornerRadius::ZERO, Color32::from_rgb(0x1c, 0x1f, 0x24));
            }
            draw::draw_track_controls(
                ui,
                &painter,
                column,
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
        assert_eq!(l.ruler.left(), l.canvas.left());
        assert_eq!(l.canvas.top(), l.controls.top());
        assert_eq!(l.controls.width(), CONTROL_W);
        assert_eq!(l.ruler.width(), l.canvas.width());
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
        let body = Rect::from_min_size(egui::pos2(112.0, 60.0), egui::vec2(800.0, 300.0));
        let x = body.left() + ((t.model.playhead - t.view_start()) as f32 * t.zoom);
        assert!((x - (112.0 + t.model.playhead as f32 * t.zoom)).abs() < 0.01);
    }

    /// The scrollbar thumbs must be draggable; this exercises the same hit rects the app uses.
    #[test]
    fn dragging_the_scrollbars_changes_the_scroll_offsets() {
        let ctx = egui::Context::default();
        let mut t = Timeline::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 420.0));
        frame(&ctx, &mut t, screen, Vec::new());
        let l = layout(screen);
        let body = Rect::from_min_max(
            l.canvas.min,
            egui::pos2(l.canvas.right() - SCROLLBAR_W, l.canvas.bottom() - SCROLLBAR_W),
        );
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
        let edge = egui::pos2(body.left() + 60.0, body.top() + t.model.tracks[0].height());
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

    #[test]
    fn tick_steps_grow_when_zoomed_out() {
        assert!(draw::tick_step(400.0) <= draw::tick_step(40.0));
        assert!(draw::tick_step(40.0) <= draw::tick_step(4.0));
        assert_eq!(draw::tick_step(4000.0), 0.5);
    }
}
