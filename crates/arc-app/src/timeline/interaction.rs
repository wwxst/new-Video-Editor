//! Phase 3A clip interaction state: stable references, selection, hover zones, marquee and the one
//! geometry function shared by painting and hit testing. No project/engine mutation.

use super::draw;
use super::model::TrackKind;
use egui::{Pos2, Rect};
use std::collections::BTreeSet;

/// Stable reference to one clip. Indices are only valid for the current timeline snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClipRef {
    pub track: usize,
    pub clip: usize,
}

/// Multi-selection with a primary item. `primary` is the last item added and the only clip that
/// displays trim handles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    items: BTreeSet<ClipRef>,
    primary: Option<ClipRef>,
}

impl Selection {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn contains(&self, r: ClipRef) -> bool {
        self.items.contains(&r)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn primary(&self) -> Option<ClipRef> {
        self.primary
    }

    pub fn is_primary(&self, r: ClipRef) -> bool {
        self.primary == Some(r)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.primary = None;
    }

    pub fn select_only(&mut self, r: ClipRef) {
        self.items.clear();
        self.items.insert(r);
        self.primary = Some(r);
    }

    pub fn toggle(&mut self, r: ClipRef) {
        if self.items.remove(&r) {
            self.fix_primary();
        } else {
            self.items.insert(r);
            self.primary = Some(r);
        }
    }

    pub fn add(&mut self, r: ClipRef) {
        if self.items.insert(r) {
            self.primary = Some(r);
        }
    }

    /// Keep only references accepted by `keep`. The primary falls back to the last remaining item.
    pub fn retain(&mut self, mut keep: impl FnMut(ClipRef) -> bool) {
        self.items.retain(|r| keep(*r));
        self.fix_primary();
    }

    fn fix_primary(&mut self) {
        if self.primary.is_none_or(|p| !self.items.contains(&p)) {
            self.primary = self.items.last().copied();
        }
    }
}

/// One pointer zone inside a clip. Audio controls (fade/volume) are deliberately absent until 3B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitZone {
    Body,
    TrimLeft,
    TrimRight,
    Audio,
    /// Volume node / horizontal envelope line.
    Volume,
    FadeIn,
    FadeOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipHit {
    pub r: ClipRef,
    pub zone: HitZone,
    pub locked: bool,
}

/// The shared clip layout. `rect` is the true clip rectangle, not the viewport intersection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipGeometry {
    pub rect: Rect,
    pub header: Rect,
    pub thumb: Option<Rect>,
    pub audio: Option<Rect>,
    pub image: Option<Rect>,
    pub wave: Option<Rect>,
}

pub fn clip_geometry(kind: TrackKind, rect: Rect, has_audio: bool) -> ClipGeometry {
    let mut g = ClipGeometry {
        rect,
        header: rect,
        thumb: None,
        audio: None,
        image: None,
        wave: None,
    };
    match kind {
        TrackKind::Video => {
            g.header =
                Rect::from_min_size(rect.min, egui::vec2(rect.width(), draw::VIDEO_HEADER_H));
            let thumb_h = if has_audio {
                draw::VIDEO_THUMB_H
            } else {
                (rect.height() - draw::VIDEO_HEADER_H).max(1.0)
            };
            g.thumb = Some(Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + draw::VIDEO_HEADER_H),
                egui::vec2(rect.width(), thumb_h),
            ));
            if has_audio {
                g.audio = Some(Rect::from_min_size(
                    egui::pos2(
                        rect.left(),
                        rect.top() + draw::VIDEO_HEADER_H + draw::VIDEO_THUMB_H,
                    ),
                    egui::vec2(rect.width(), draw::VIDEO_AUDIO_H),
                ));
            }
        }
        TrackKind::Image => {
            g.header =
                Rect::from_min_size(rect.min, egui::vec2(rect.width(), draw::IMAGE_HEADER_H));
            g.image = Some(Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + draw::IMAGE_HEADER_H),
                egui::vec2(
                    rect.width(),
                    (rect.height() - draw::IMAGE_HEADER_H).max(1.0),
                ),
            ));
        }
        TrackKind::Audio => {
            g.header =
                Rect::from_min_size(rect.min, egui::vec2(rect.width(), draw::AUDIO_HEADER_H));
            g.wave = Some(Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + draw::AUDIO_HEADER_H),
                egui::vec2(rect.width(), draw::AUDIO_WAVE_H),
            ));
        }
        TrackKind::Text | TrackKind::Effect | TrackKind::Sticker => {}
    }
    g
}

/// Trim hit width: 6 pt normally, 4 pt for narrow clips, none below 12 pt.
pub fn trim_hit_width(visible_width: f32) -> f32 {
    if visible_width >= 20.0 {
        6.0
    } else if visible_width >= 12.0 {
        4.0
    } else {
        0.0
    }
}

fn trim_rects(g: &ClipGeometry, content: Rect) -> (Option<Rect>, Option<Rect>) {
    let visible = g.rect.intersect(content);
    let hit_w = trim_hit_width(visible.width());
    if hit_w <= 0.0 || visible.height() < 4.0 {
        return (None, None);
    }
    let y0 = g.rect.top() + 4.0;
    let y1 = (g.rect.bottom() - 4.0).max(y0 + 1.0);
    // Only the real clip edge produces a handle. A viewport edge must never become a fake handle.
    let left = (g.rect.left() >= content.left() - 0.5).then(|| {
        Rect::from_min_max(
            egui::pos2(g.rect.left(), y0),
            egui::pos2(g.rect.left() + hit_w, y1),
        )
    });
    let right = (g.rect.right() <= content.right() + 0.5).then(|| {
        Rect::from_min_max(
            egui::pos2(g.rect.right() - hit_w, y0),
            egui::pos2(g.rect.right(), y1),
        )
    });
    (left, right)
}

/// Hit test one clip. The caller supplies the already-computed geometry and model flags.
pub fn hit_test_clip(
    r: ClipRef,
    g: &ClipGeometry,
    content: Rect,
    pointer: Pos2,
    locked: bool,
) -> Option<ClipHit> {
    let visible = g.rect.intersect(content);
    if visible.width() < 1.0 || visible.height() < 1.0 || !visible.contains(pointer) {
        return None;
    }
    if locked {
        return Some(ClipHit {
            r,
            zone: HitZone::Body,
            locked: true,
        });
    }
    let (left, right) = trim_rects(g, content);
    if left.is_some_and(|rect| rect.contains(pointer)) {
        return Some(ClipHit {
            r,
            zone: HitZone::TrimLeft,
            locked: false,
        });
    }
    if right.is_some_and(|rect| rect.contains(pointer)) {
        return Some(ClipHit {
            r,
            zone: HitZone::TrimRight,
            locked: false,
        });
    }
    if g.audio
        .or(g.wave)
        .is_some_and(|audio| audio.intersect(content).contains(pointer))
    {
        return Some(ClipHit {
            r,
            zone: HitZone::Audio,
            locked: false,
        });
    }
    Some(ClipHit {
        r,
        zone: HitZone::Body,
        locked: false,
    })
}

/// Visible handle rectangle for the primary selection. `side` is -1 for left, +1 for right.
pub fn primary_handle_rect(g: &ClipGeometry, content: Rect, left: bool) -> Option<Rect> {
    let visible = g.rect.intersect(content);
    if visible.width() < 12.0 || visible.height() < 8.0 {
        return None;
    }
    let y0 = g.rect.top() + 4.0;
    let y1 = (g.rect.bottom() - 4.0).max(y0 + 1.0);
    // Inset the grip 4 pt from the real edge so it reads as a handle rather than a thicker border.
    if left {
        (g.rect.left() >= content.left() - 0.5).then(|| {
            Rect::from_min_max(
                egui::pos2(g.rect.left() + 4.0, y0),
                egui::pos2(g.rect.left() + 7.0, y1),
            )
        })
    } else {
        (g.rect.right() <= content.right() + 0.5).then(|| {
            Rect::from_min_max(
                egui::pos2(g.rect.right() - 7.0, y0),
                egui::pos2(g.rect.right() - 4.0, y1),
            )
        })
    }
}

/// An in-progress marquee. `before` is restored on Escape.
#[derive(Debug, Clone)]
pub struct Marquee {
    pub start: Pos2,
    pub current: Pos2,
    pub additive: bool,
    pub before: Selection,
}

impl Marquee {
    pub fn new(start: Pos2, additive: bool, before: Selection) -> Marquee {
        Marquee {
            start,
            current: start,
            additive,
            before,
        }
    }

    pub fn rect(&self, content: Rect) -> Rect {
        Rect::from_two_pos(self.start, self.current).intersect(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video_rect() -> Rect {
        Rect::from_min_size(egui::pos2(100.0, 20.0), egui::vec2(300.0, 72.0))
    }

    #[test]
    fn geometry_matches_the_fixed_clip_regions() {
        let g = clip_geometry(TrackKind::Video, video_rect(), true);
        assert_eq!(g.header.height(), 18.0);
        assert_eq!(g.thumb.unwrap().height(), 38.0);
        assert_eq!(g.audio.unwrap().height(), 16.0);
        let g = clip_geometry(TrackKind::Video, video_rect(), false);
        assert_eq!(g.thumb.unwrap().height(), 54.0);
        assert!(g.audio.is_none());
        let g = clip_geometry(TrackKind::Audio, video_rect(), false);
        assert_eq!(g.header.height(), 16.0);
        assert_eq!(g.wave.unwrap().height(), 30.0);
    }

    #[test]
    fn trim_zones_shrink_on_narrow_clips_and_use_real_edges() {
        let content = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 300.0));
        let mut g = clip_geometry(TrackKind::Video, video_rect(), true);
        assert_eq!(trim_hit_width(300.0), 6.0);
        assert_eq!(trim_hit_width(16.0), 4.0);
        assert_eq!(trim_hit_width(10.0), 0.0);

        // Real left edge inside the content: trim hit.
        let hit = hit_test_clip(
            ClipRef { track: 0, clip: 0 },
            &g,
            content,
            egui::pos2(101.0, 40.0),
            false,
        )
        .unwrap();
        assert_eq!(hit.zone, HitZone::TrimLeft);

        // Clip starts off-screen left: the viewport edge must not become a trim handle.
        g.rect = Rect::from_min_size(egui::pos2(-50.0, 20.0), egui::vec2(300.0, 72.0));
        let hit = hit_test_clip(
            ClipRef { track: 0, clip: 0 },
            &g,
            content,
            egui::pos2(1.0, 40.0),
            false,
        )
        .unwrap();
        assert_eq!(hit.zone, HitZone::Body);
    }

    #[test]
    fn selection_tracks_primary_and_toggle() {
        let a = ClipRef { track: 0, clip: 0 };
        let b = ClipRef { track: 1, clip: 2 };
        let mut s = Selection::default();
        s.select_only(a);
        assert!(s.is_primary(a));
        s.toggle(b);
        assert_eq!(s.len(), 2);
        assert!(s.is_primary(b));
        s.toggle(b);
        assert_eq!(s.len(), 1);
        assert!(s.is_primary(a));
        s.toggle(a);
        assert!(s.is_empty());
        assert_eq!(s.primary(), None);
    }

    #[test]
    fn marquee_is_clipped_to_content() {
        let content = Rect::from_min_size(egui::pos2(100.0, 10.0), egui::vec2(200.0, 100.0));
        let m = Marquee::new(egui::pos2(90.0, 0.0), false, Selection::default());
        let mut m = m;
        m.current = egui::pos2(400.0, 200.0);
        assert_eq!(m.rect(content), content);
    }
}
