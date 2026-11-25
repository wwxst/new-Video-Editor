//! The document model (docs/02 §1–3). Immutable snapshots built on persistent collections.
//!
//! The engine is the only writer; everything else holds `Arc<Project>` snapshots.

use crate::ids::*;
use crate::time::{FrameRate, Ratio, Time, TimeError};
use im::{OrdMap, Vector};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Current schema version written by this build (docs/schema/CHANGELOG.md).
pub const SCHEMA_VERSION: u32 = 7;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub meta: Meta,
    pub settings: Settings,
    pub assets: OrdMap<AssetId, Asset>,
    pub sequences: OrdMap<SequenceId, Sequence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: ProjectId,
    pub name: String,
    /// RFC 3339 creation timestamp (informational).
    #[serde(default)]
    pub created: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The active sequence. v1 projects have exactly one.
    pub sequence: SequenceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub w: u32,
    pub h: u32,
}

/// Stored timebase: the frame duration. The frame rate is its exact reciprocal (docs/02 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TimebaseWire", into = "TimebaseWire")]
pub struct Timebase {
    rate: FrameRate,
}

#[derive(Serialize, Deserialize)]
struct TimebaseWire {
    frame_duration: Time,
}

impl TryFrom<TimebaseWire> for Timebase {
    type Error = TimeError;
    fn try_from(w: TimebaseWire) -> Result<Self, TimeError> {
        let d = w.frame_duration;
        let rate = FrameRate::from_ratio(Ratio::new(d.den() as i64, d.num())?)?;
        Ok(Timebase { rate })
    }
}
impl From<Timebase> for TimebaseWire {
    fn from(t: Timebase) -> Self {
        TimebaseWire {
            frame_duration: t.rate.frame_duration(),
        }
    }
}

impl Timebase {
    pub fn new(rate: FrameRate) -> Self {
        Timebase { rate }
    }
    pub fn frame_rate(self) -> FrameRate {
        self.rate
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimecodeDisplay {
    #[default]
    Ndf,
    Df,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub id: SequenceId,
    pub name: String,
    pub timebase: Timebase,
    pub resolution: Resolution,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    #[serde(default)]
    pub timecode_display: TimecodeDisplay,
    /// Bottom to top for video (higher index composites on top).
    pub tracks: Vector<Track>,
    #[serde(default)]
    pub markers: Vector<Marker>,
}

fn default_sample_rate() -> u32 {
    48_000
}
fn default_true() -> bool {
    true
}
fn default_one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub locked: bool,
    /// Video tracks: track opacity 0..=1.
    #[serde(default = "default_one")]
    pub opacity: f64,
    /// Audio tracks: track gain in dB.
    #[serde(default)]
    pub gain_db: f64,
    #[serde(default)]
    pub muted: bool,
    /// Sorted by `start`, never overlapping.
    pub clips: Vector<Clip>,
    #[serde(default)]
    pub transitions: Vector<Transition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Title {
    pub text: String,
    #[serde(default = "default_title_style")]
    pub style: String,
}
fn default_title_style() -> String {
    "lower_third".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    /// Exactly one of `asset` / `title` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Title>,
    /// Timeline position.
    pub start: Time,
    /// Timeline duration.
    pub duration: Time,
    /// Source offset (source time).
    #[serde(default = "zero_time")]
    pub source_in: Time,
    /// Playback speed; `duration × speed` = source range.
    #[serde(default = "one_ratio")]
    pub speed: Ratio,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub effects: Vector<EffectInstance>,
    #[serde(default)]
    pub audio: ClipAudio,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn zero_time() -> Time {
    Time::ZERO
}
fn one_ratio() -> Ratio {
    Ratio::ONE
}

impl Clip {
    pub fn end(&self) -> Result<Time, TimeError> {
        self.start.checked_add(self.duration)
    }
    pub fn source_range(&self) -> Result<Time, TimeError> {
        self.duration.checked_mul(self.speed)
    }
    pub fn source_out(&self) -> Result<Time, TimeError> {
        self.source_in.checked_add(self.source_range()?)
    }
    /// Source time shown at timeline time `t` (caller ensures `t` is inside the clip).
    pub fn source_time_at(&self, t: Time) -> Result<Time, TimeError> {
        self.source_in
            .checked_add(t.checked_sub(self.start)?.checked_mul(self.speed)?)
    }
    pub fn contains(&self, t: Time) -> bool {
        match self.end() {
            Ok(end) => t >= self.start && t < end,
            Err(_) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

/// Static transform in normalized frame units: position offset as a fraction of output size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub position: Vec2,
    pub scale: Vec2,
    pub rotation_deg: f64,
    pub anchor: Vec2,
    pub opacity: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            position: Vec2 { x: 0.0, y: 0.0 },
            scale: Vec2 { x: 1.0, y: 1.0 },
            rotation_deg: 0.0,
            anchor: Vec2 { x: 0.5, y: 0.5 },
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClipAudio {
    pub gain_db: f64,
    pub pan: f64,
    pub fade_in: Time,
    pub fade_out: Time,
}

impl Default for ClipAudio {
    fn default() -> Self {
        ClipAudio {
            gain_db: 0.0,
            pan: 0.0,
            fade_in: Time::ZERO,
            fade_out: Time::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectInstance {
    pub id: EffectId,
    /// Manifest id, e.g. `core.color.correct`.
    pub effect: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub params: BTreeMap<String, Param>,
}

/// A parameter value: static or keyframed (docs/02 §3.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Param {
    Static {
        #[serde(rename = "static")]
        value: f64,
    },
    Curve {
        curve: Vec<Keyframe>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interp {
    Linear,
    Hold,
    /// Cubic-bezier easing using `ease` control points (CSS semantics).
    Bezier,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Relative to clip start.
    pub t: Time,
    pub v: f64,
    pub interp: Interp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease: Option<[f64; 4]>,
}

impl Param {
    pub fn constant(value: f64) -> Param {
        Param::Static { value }
    }

    /// Evaluate at clip-local time. Pure; identical in preview and export.
    pub fn eval(&self, local: Time) -> f64 {
        match self {
            Param::Static { value } => *value,
            Param::Curve { curve } => eval_curve(curve, local),
        }
    }
}

fn eval_curve(curve: &[Keyframe], local: Time) -> f64 {
    let (Some(first), Some(last)) = (curve.first(), curve.last()) else {
        return 0.0;
    };
    if local <= first.t {
        return first.v;
    }
    if local >= last.t {
        return last.v;
    }
    for w in curve.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if local >= a.t && local < b.t {
            let span =
                b.t.checked_sub(a.t)
                    .map(|d| d.to_f64_lossy())
                    .unwrap_or(1.0);
            let x = local
                .checked_sub(a.t)
                .map(|d| d.to_f64_lossy())
                .unwrap_or(0.0)
                / span;
            let p = match a.interp {
                Interp::Hold => 0.0,
                Interp::Linear => x,
                Interp::Bezier => cubic_bezier(a.ease.unwrap_or([0.25, 0.1, 0.25, 1.0]), x),
            };
            return a.v + (b.v - a.v) * p;
        }
    }
    last.v
}

/// CSS cubic-bezier(x1, y1, x2, y2) evaluated at progress `x` via Newton + bisection.
fn cubic_bezier(c: [f64; 4], x: f64) -> f64 {
    let [x1, y1, x2, y2] = c;
    let bez = |t: f64, p1: f64, p2: f64| {
        let u = 1.0 - t;
        3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
    };
    let (mut lo, mut hi, mut t) = (0.0f64, 1.0f64, x);
    for _ in 0..40 {
        let v = bez(t, x1, x2);
        if (v - x).abs() < 1e-9 {
            break;
        }
        if v < x {
            lo = t;
        } else {
            hi = t;
        }
        t = 0.5 * (lo + hi);
    }
    bez(t, y1, y2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    CrossDissolve,
}

/// A transition between two adjacent clips on the same track (docs/02 §3.5).
/// Centered on the cut; uses source handles; never moves clips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub id: TransitionId,
    pub kind: TransitionKind,
    pub a: ClipId,
    pub b: ClipId,
    pub duration: Time,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    pub at: Time,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Video,
    Audio,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetStatus {
    #[default]
    Ok,
    Quarantined,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: Ratio,
    #[serde(default)]
    pub vfr: bool,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub pix_fmt: String,
    #[serde(default)]
    pub rotation: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u32,
    #[serde(default)]
    pub codec: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    pub duration: Time,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub id: AssetId,
    pub kind: AssetKind,
    /// Relative to the project folder where possible; may be missing (offline).
    pub path: String,
    #[serde(default)]
    pub status: AssetStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<Probe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

impl Project {
    /// A new empty project with one sequence.
    pub fn new(name: &str, rate: FrameRate, resolution: Resolution) -> Project {
        let seq_id = SequenceId::generate();
        let seq = Sequence {
            id: seq_id.clone(),
            name: "Main".into(),
            timebase: Timebase::new(rate),
            resolution,
            sample_rate: 48_000,
            timecode_display: TimecodeDisplay::Ndf,
            tracks: Vector::new(),
            markers: Vector::new(),
        };
        Project {
            schema_version: SCHEMA_VERSION,
            meta: Meta {
                id: ProjectId::generate(),
                name: name.into(),
                created: String::new(),
            },
            settings: Settings {
                sequence: seq_id.clone(),
            },
            assets: OrdMap::new(),
            sequences: OrdMap::unit(seq_id, seq),
        }
    }

    pub fn sequence(&self) -> Option<&Sequence> {
        self.sequences.get(&self.settings.sequence)
    }

    /// Canonical JSON bytes: deterministic field and key order (docs/02 §6).
    pub fn canonical_json(&self) -> Vec<u8> {
        // Serialization of this model cannot fail: all keys are strings and no floats are NaN
        // (validation rejects non-finite values before they enter a snapshot).
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// BLAKE3 hash of the canonical JSON, hex.
    pub fn canonical_hash(&self) -> String {
        blake3::hash(&self.canonical_json()).to_hex().to_string()
    }
}

impl Sequence {
    pub fn frame_rate(&self) -> FrameRate {
        self.timebase.frame_rate()
    }

    pub fn track_index(&self, id: &TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| &t.id == id)
    }

    /// (track index, clip index) of a clip.
    pub fn find_clip(&self, id: &ClipId) -> Option<(usize, usize)> {
        for (ti, t) in self.tracks.iter().enumerate() {
            if let Some(ci) = t.clips.iter().position(|c| &c.id == id) {
                return Some((ti, ci));
            }
        }
        None
    }

    pub fn clip(&self, id: &ClipId) -> Option<&Clip> {
        let (ti, ci) = self.find_clip(id)?;
        self.tracks.get(ti)?.clips.get(ci)
    }

    /// End of the last clip on any track.
    pub fn duration(&self) -> Time {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| c.end().ok())
            .max()
            .unwrap_or(Time::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_and_hash_stable() {
        let p = Project::new("Demo", FrameRate::Fps29_97, Resolution { w: 1920, h: 1080 });
        let a = p.canonical_json();
        let back: Project = serde_json::from_slice(&a).expect("de");
        assert_eq!(back, p);
        assert_eq!(back.canonical_hash(), p.canonical_hash());
        let s = String::from_utf8(a).expect("utf8");
        assert!(
            s.contains(r#""timebase":{"frame_duration":{"num":1001,"den":30000}}"#),
            "{s}"
        );
    }

    #[test]
    fn params_eval() {
        let c = Param::Curve {
            curve: vec![
                Keyframe {
                    t: Time::ZERO,
                    v: 0.0,
                    interp: Interp::Linear,
                    ease: None,
                },
                Keyframe {
                    t: Time::secs(2),
                    v: 10.0,
                    interp: Interp::Hold,
                    ease: None,
                },
                Keyframe {
                    t: Time::secs(4),
                    v: 20.0,
                    interp: Interp::Linear,
                    ease: None,
                },
            ],
        };
        assert_eq!(c.eval(Time::secs(1)), 5.0);
        assert_eq!(c.eval(Time::secs(3)), 10.0);
        assert_eq!(c.eval(Time::secs(9)), 20.0);
        let j = serde_json::to_string(&Param::constant(0.5)).expect("ser");
        assert_eq!(j, r#"{"static":0.5}"#);
        let b = cubic_bezier([0.42, 0.0, 0.58, 1.0], 0.5);
        assert!((b - 0.5).abs() < 1e-6);
    }
}
