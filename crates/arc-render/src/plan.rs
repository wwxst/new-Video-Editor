//! Frame planning: what to draw at a timeline frame (docs/03 §2). Pure; no GPU, no decoding.
//!
//! The exporter and the viewer both call [`plan_frame`], fetch the listed source frames, and hand
//! them to the compositor — so preview and export evaluate the timeline identically.

use arc_core::ids::{AssetId, ClipId, TrackId};
use arc_core::model::{Clip, Project, Track, TrackKind, Transform};
use arc_core::time::{FrameRate, Time};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct EffectPlan {
    pub effect: String,
    /// Parameter values evaluated at the clip-local time.
    pub params: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourcePlan {
    /// Decode `asset` at `time` (relative to the media start).
    Media { asset: AssetId, time: Time },
    /// Render a title (text rendering lands with the text renderer; see HANDOVER).
    Title { text: String, style: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemPlan {
    pub clip: ClipId,
    pub source: SourcePlan,
    pub transform: Transform,
    /// Opacity multiplier: transform opacity × transition weight.
    pub opacity: f64,
    /// Items of a cross dissolve are summed (weights add to 1) rather than layered.
    pub additive: bool,
    pub effects: Vec<EffectPlan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerPlan {
    pub track: TrackId,
    pub opacity: f64,
    pub items: Vec<ItemPlan>,
}

fn item(clip: &Clip, t: Time, weight: f64, additive: bool) -> Option<ItemPlan> {
    let source = match (&clip.asset, &clip.title) {
        (Some(a), _) => SourcePlan::Media {
            asset: a.clone(),
            time: clip.source_time_at(t).ok()?,
        },
        (None, Some(ti)) => SourcePlan::Title {
            text: ti.text.clone(),
            style: ti.style.clone(),
        },
        _ => return None,
    };
    let local = t.checked_sub(clip.start).ok()?;
    let effects = clip
        .effects
        .iter()
        .filter(|e| e.enabled)
        .map(|e| EffectPlan {
            effect: e.effect.clone(),
            params: e
                .params
                .iter()
                .map(|(k, v)| (k.clone(), v.eval(local)))
                .collect(),
        })
        .collect();
    Some(ItemPlan {
        clip: clip.id.clone(),
        source,
        transform: clip.transform,
        opacity: clip.transform.opacity * weight,
        additive,
        effects,
    })
}

fn plan_track(track: &Track, t: Time) -> Vec<ItemPlan> {
    let mut items = Vec::new();
    let mut in_transition: Vec<&ClipId> = Vec::new();
    for x in track.transitions.iter() {
        let (Some(a), Some(b)) = (
            track.clips.iter().find(|c| c.id == x.a),
            track.clips.iter().find(|c| c.id == x.b),
        ) else {
            continue;
        };
        let half = Time::new(x.duration.num(), 2 * x.duration.den() as i64).unwrap_or(Time::ZERO);
        let (Ok(from), Ok(to)) = (b.start.checked_sub(half), b.start.checked_add(half)) else {
            continue;
        };
        if t >= from && t < to {
            let w = t
                .checked_sub(from)
                .map(|d| d.to_f64_lossy() / x.duration.to_f64_lossy())
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            if a.enabled {
                items.extend(item(a, t, 1.0 - w, true));
            }
            if b.enabled {
                items.extend(item(b, t, w, true));
            }
            in_transition.push(&a.id);
            in_transition.push(&b.id);
        }
    }
    for c in track.clips.iter() {
        if c.enabled && c.contains(t) && !in_transition.contains(&&c.id) {
            items.extend(item(c, t, 1.0, false));
        }
    }
    items
}

/// Plan the frame at `frame` (timeline frame index at the sequence rate). Video tracks only,
/// bottom to top; disabled tracks and clips are skipped.
pub fn plan_frame(project: &Project, frame: i64) -> Vec<LayerPlan> {
    let Some(seq) = project.sequence() else {
        return Vec::new();
    };
    let t = seq.frame_rate().time_of_frame(frame);
    plan_at(project, t)
}

/// Plan at an exact timeline time.
pub fn plan_at(project: &Project, t: Time) -> Vec<LayerPlan> {
    let Some(seq) = project.sequence() else {
        return Vec::new();
    };
    seq.tracks
        .iter()
        .filter(|tr| tr.kind == TrackKind::Video && tr.enabled)
        .map(|tr| LayerPlan {
            track: tr.id.clone(),
            opacity: tr.opacity,
            items: plan_track(tr, t),
        })
        .filter(|l| !l.items.is_empty())
        .collect()
}

/// Number of frames in the sequence (end of the last clip, rounded up).
pub fn frame_count(project: &Project) -> i64 {
    let Some(seq) = project.sequence() else {
        return 0;
    };
    let rate: FrameRate = seq.frame_rate();
    rate.frame_at(seq.duration(), arc_core::time::Round::Ceil)
        .unwrap_or(0)
}
