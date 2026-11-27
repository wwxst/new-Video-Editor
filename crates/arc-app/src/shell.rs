//! Four-panel editor shell (media / preview / inspector / timeline).
//!
//! Layout only: the media, preview and inspector panels are containers with placeholder titles.
//! The timeline is a working surface and deliberately has no card title, so its toolbar is the
//! first row of the module. The arrangement follows the approved reference: a left media column, a
//! wide preview in the middle, an inspector on the right and a full-width timeline at the bottom,
//! all as separated cards with draggable splitters.

use egui::{Color32, CornerRadius, CursorIcon, Rect, Sense, Stroke, Ui, Vec2};

/// Window background behind the cards.
pub const WINDOW_BG: Color32 = Color32::from_rgb(0x12, 0x13, 0x16);
/// Card background.
pub const CARD_BG: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x25);
/// Card border.
pub const BORDER: Color32 = Color32::from_rgb(0x2f, 0x33, 0x3b);
/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe8, 0xec);
/// Secondary text.
pub const DIM: Color32 = Color32::from_rgb(0x8a, 0x90, 0x99);
/// Hover/active accent for the splitters.
pub const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xff);

/// Gap between cards, in points (DPI independent).
const GAP: f32 = 10.0;
/// Window edge margin, in points.
const MARGIN: f32 = 12.0;
/// Smallest sensible card size, in points.
const MIN_CARD: f32 = 120.0;

/// Sizes of the panels as fractions of the available area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelSizes {
    /// Left (media) column width, as a fraction of the window width.
    pub left: f32,
    /// Right (inspector) column width, as a fraction of the window width.
    pub right: f32,
    /// Bottom (timeline) height, as a fraction of the window height.
    pub bottom: f32,
}

impl Default for PanelSizes {
    fn default() -> Self {
        // Matches the reference proportions: 24.5 % / 48 % / 24.5 %, bottom ~34 %.
        PanelSizes {
            left: 0.245,
            right: 0.245,
            bottom: 0.34,
        }
    }
}

/// The resolved rectangles of one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Panels {
    pub left: Rect,
    pub center: Rect,
    pub right: Rect,
    pub bottom: Rect,
    /// Draggable splitter bars: left|center, center|right, top(bottom row boundary).
    pub splitters: [Rect; 3],
}

/// Resolve the four panel rectangles and their splitter bars inside `rect`.
///
/// Pure geometry so it can be unit tested and reused by any renderer.
pub fn layout(rect: Rect, sizes: PanelSizes) -> Panels {
    let inner = rect.shrink(MARGIN);
    let width = inner.width().max(1.0);
    let height = inner.height().max(1.0);

    let bottom_h = (height * sizes.bottom).clamp(MIN_CARD, height - MIN_CARD);
    let top_h = (height - bottom_h - GAP).max(MIN_CARD);

    let left_w = (width * sizes.left).clamp(MIN_CARD, width - 2.0 * MIN_CARD - 2.0 * GAP);
    let right_w = (width * sizes.right).clamp(MIN_CARD, width - left_w - MIN_CARD - 2.0 * GAP);
    let center_w = (width - left_w - right_w - 2.0 * GAP).max(MIN_CARD);

    let top = Rect::from_min_size(inner.min, Vec2::new(width, top_h));
    let bottom = Rect::from_min_size(
        egui::pos2(inner.min.x, inner.min.y + top_h + GAP),
        Vec2::new(width, bottom_h),
    );

    let left = Rect::from_min_size(top.min, Vec2::new(left_w, top_h));
    let center = Rect::from_min_size(
        egui::pos2(left.max.x + GAP, top.min.y),
        Vec2::new(center_w, top_h),
    );
    let right = Rect::from_min_size(
        egui::pos2(center.max.x + GAP, top.min.y),
        Vec2::new(right_w.min(top.max.x - (center.max.x + GAP)), top_h),
    );

    let splitter = |x: f32, y: f32, w: f32, h: f32| {
        Rect::from_min_size(egui::pos2(x, y), Vec2::new(w, h))
    };
    Panels {
        left,
        center,
        right,
        bottom,
        splitters: [
            // The gaps double as hit areas; a bar in the middle looks like a grip.
            splitter(left.max.x, top.min.y, GAP, top_h),
            splitter(center.max.x, top.min.y, GAP, top_h),
            splitter(inner.min.x, top.max.y, width, GAP),
        ],
    }
}

/// The shell state: panel fractions, adjusted by dragging the splitters.
#[derive(Debug, Clone)]
pub struct Shell {
    pub sizes: PanelSizes,
    pub timeline: crate::timeline::Timeline,
}

impl Default for Shell {
    fn default() -> Self {
        Shell {
            sizes: PanelSizes::default(),
            timeline: crate::timeline::Timeline::default(),
        }
    }
}

impl Shell {
    /// Shell whose timeline uses the real sequence frame rate from a loaded project.
    pub fn with_frame_rate(frame_rate: arc_core::time::FrameRate) -> Shell {
        Shell {
            sizes: PanelSizes::default(),
            timeline: crate::timeline::Timeline::new(frame_rate),
        }
    }

    /// Draw the four panels into the root UI of one frame.
    pub fn ui(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, CornerRadius::ZERO, WINDOW_BG);

        let panels = layout(rect, self.sizes);

        card(ui, "MEDIA", panels.left);
        card(ui, "PREVIEW", panels.center);
        card(ui, "INSPECTOR", panels.right);
        // The timeline is a working surface, not a titled placeholder: no card title, so the
        // timeline toolbar is the first row of the module.
        card_body(ui, panels.bottom, |ui, content| {
            self.timeline.ui(ui, content)
        });

        self.splitters(&panels, ui);
    }

    /// Hit-test, drag and paint all three splitters in one pass (one interactive widget per bar).
    fn splitters(&mut self, panels: &Panels, ui: &mut Ui) {
        let width = panels.left.width() + panels.center.width() + panels.right.width() + 2.0 * GAP;
        let height = panels.bottom.height() + panels.center.height() + GAP;
        for (index, splitter) in panels.splitters.iter().enumerate() {
            let response = ui.interact(
                *splitter,
                ui.make_persistent_id(("splitter", index)),
                Sense::click_and_drag(),
            );
            let active = response.hovered() || response.dragged();
            if active {
                ui.ctx().set_cursor_icon(if index == 2 {
                    CursorIcon::ResizeVertical
                } else {
                    CursorIcon::ResizeHorizontal
                });
            }
            if response.dragged() {
                let delta = response.drag_delta();
                match index {
                    0 => self.sizes.left = (self.sizes.left + delta.x / width).clamp(0.12, 0.40),
                    1 => self.sizes.right = (self.sizes.right - delta.x / width).clamp(0.12, 0.40),
                    _ => self.sizes.bottom = (self.sizes.bottom - delta.y / height).clamp(0.18, 0.50),
                }
            }
            let color = if active {
                ACCENT.gamma_multiply(0.55)
            } else {
                Color32::from_rgb(0x24, 0x27, 0x2d)
            };
            let bar = if index == 2 {
                Rect::from_center_size(splitter.center(), Vec2::new(46.0, 3.0))
            } else {
                Rect::from_center_size(splitter.center(), Vec2::new(3.0, 46.0))
            };
            ui.painter().rect_filled(bar, 2.0, color);
        }
    }
}
fn card_body(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui, Rect)) {
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        CARD_BG,
        Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
    // No title row: the timeline toolbar starts directly under the top border, with only the
    // 10 pt side padding and 8 pt bottom padding of the card.
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + 10.0, rect.top() + 1.0),
        egui::pos2(rect.right() - 10.0, rect.bottom() - 8.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    add(&mut child, inner);
}
fn card(ui: &mut Ui, title: &str, rect: Rect) {
    // The card fills its rect; the inner Ui only lays out the title block.
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        CARD_BG,
        Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(14.0, 12.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.label(egui::RichText::new(title).size(13.0).strong().color(TEXT));
    child.add_space(2.0);
    child.label(egui::RichText::new("content pending").size(11.0).color(DIM));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 900.0))
    }

    #[test]
    fn proportions_match_the_reference() {
        let p = layout(rect(), PanelSizes::default());
        let inner_w = 1600.0 - 2.0 * MARGIN;
        let inner_h = 900.0 - 2.0 * MARGIN;
        assert!((p.left.width() / inner_w - 0.245).abs() < 0.01);
        assert!((p.right.width() / inner_w - 0.245).abs() < 0.01);
        assert!((p.center.width() / inner_w - 0.51).abs() < 0.02);
        assert!((p.bottom.height() / inner_h - 0.34).abs() < 0.01);
    }

    #[test]
    fn panels_are_separated_by_gaps_without_overlap() {
        let p = layout(rect(), PanelSizes::default());
        assert!(p.left.max.x < p.center.min.x);
        assert!(p.center.max.x < p.right.min.x);
        assert!(p.center.max.y < p.bottom.min.y);
        assert!((p.center.min.x - p.left.max.x - GAP).abs() < 0.01);
        assert!((p.bottom.min.y - p.center.max.y - GAP).abs() < 0.01);
        assert!(p.right.max.x <= rect().max.x - MARGIN + 0.01);
    }

    #[test]
    fn splitters_are_centred_on_the_gaps() {
        let p = layout(rect(), PanelSizes::default());
        assert!((p.splitters[0].center().x - (p.left.max.x + p.center.min.x) / 2.0).abs() < 0.01);
        assert!((p.splitters[1].center().x - (p.center.max.x + p.right.min.x) / 2.0).abs() < 0.01);
        assert!((p.splitters[2].center().y - (p.center.max.y + p.bottom.min.y) / 2.0).abs() < 0.01);
    }

    #[test]
    fn tiny_windows_keep_every_card_usable() {
        let p = layout(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 320.0)), PanelSizes::default());
        for r in [p.left, p.center, p.right, p.bottom] {
            assert!(r.width() >= MIN_CARD - 0.01, "{r:?}");
            assert!(r.height() >= MIN_CARD - 0.01, "{r:?}");
        }
    }

    #[test]
    fn dragging_the_vertical_splitter_resizes_the_left_panel() {
        let ctx = egui::Context::default();
        let mut shell = Shell::default();
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 720.0));
        fn run(ctx: &egui::Context, shell: &mut Shell, screen: Rect, events: Vec<egui::Event>) {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(screen);
            input.events = events;
            let mut out = ctx.run_ui(input, |ui| shell.ui(ui));
            out.textures_delta.clear();
        }

        run(&ctx, &mut shell, screen, Vec::new());
        let splitter = layout(screen, PanelSizes::default()).splitters[0].center();
        run(&ctx, &mut shell, screen, vec![egui::Event::PointerMoved(splitter)]);
        run(
            &ctx,
            &mut shell,
            screen,
            vec![egui::Event::PointerButton {
                pos: splitter,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        run(
            &ctx,
            &mut shell,
            screen,
            vec![egui::Event::PointerMoved(splitter + egui::vec2(20.0, 0.0))],
        );
        run(
            &ctx,
            &mut shell,
            screen,
            vec![egui::Event::PointerMoved(splitter + egui::vec2(40.0, 0.0))],
        );
        run(
            &ctx,
            &mut shell,
            screen,
            vec![egui::Event::PointerButton {
                pos: splitter + egui::vec2(40.0, 0.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        assert!(
            shell.sizes.left > PanelSizes::default().left + 0.01,
            "left {} should have grown",
            shell.sizes.left
        );
    }
    #[test]
    fn fractions_can_reach_the_clamps() {
        let wide = PanelSizes {
            left: 0.40,
            right: 0.40,
            bottom: 0.50,
        };
        let p = layout(rect(), wide);
        assert!(p.center.width() >= MIN_CARD - 0.01);
        assert!(p.right.max.x <= rect().max.x - MARGIN + 0.01);
    }
}
