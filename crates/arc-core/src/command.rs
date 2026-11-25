//! Typed document commands (docs/02 §4).
//!
//! Every state change is a `Command`. Applying a command first **resolves** it in place
//! (generated IDs filled in, timecodes converted to exact rationals), so the resolved command
//! list can be journaled and replayed deterministically.

use crate::effects::EffectCatalog;
use crate::error::{CmdResult, CommandError, ErrorKind};
use crate::ids::*;
use crate::model::*;
use crate::time::{FrameRate, MAX_SEQUENCE_SECONDS, Ratio, Time};
use im::Vector;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

/// A time given by a client: an exact rational (seconds) or a timecode string at the sequence
/// frame rate (NDF `HH:MM:SS:FF`, or DF `HH:MM:SS;FF` at 29.97/59.94). Resolution always yields
/// the exact form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TimeInput {
    Exact(Time),
    Timecode(String),
}

impl From<Time> for TimeInput {
    fn from(t: Time) -> Self {
        TimeInput::Exact(t)
    }
}

impl TimeInput {
    fn resolve(&mut self, rate: FrameRate) -> CmdResult<Time> {
        let t = match self {
            TimeInput::Exact(t) => *t,
            TimeInput::Timecode(s) => rate.timecode_to_time(s)?,
        };
        *self = TimeInput::Exact(t);
        Ok(t)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ripple {
    #[default]
    None,
    Track,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetPatch {
    pub id: AssetId,
    pub asset: Option<Asset>,
}

/// Engine-generated inverse. Rejected when sent by any client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Restore {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<Track>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markers: Option<Vec<Marker>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<AssetPatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Command {
    #[serde(rename = "project.rename")]
    ProjectRename { name: String },

    #[serde(rename = "track.add")]
    TrackAdd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<TrackId>,
        kind: TrackKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Insert position (0 = bottom). Default: top.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    #[serde(rename = "track.remove")]
    TrackRemove { track: TrackId },
    #[serde(rename = "track.set_properties")]
    TrackSetProperties {
        track: TrackId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locked: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gain_db: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
    },

    /// Resolved form: the session layer has already checked the path and probed the file.
    #[serde(rename = "asset.import")]
    AssetImport {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<AssetId>,
        path: String,
        kind: AssetKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        probe: Option<Probe>,
        #[serde(default)]
        status: AssetStatus,
    },
    #[serde(rename = "asset.remove")]
    AssetRemove { asset: AssetId },

    #[serde(rename = "timeline.add_clip")]
    AddClip {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ClipId>,
        track: TrackId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        asset: Option<AssetId>,
        start: TimeInput,
        duration: TimeInput,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_in: Option<TimeInput>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speed: Option<Ratio>,
    },
    #[serde(rename = "timeline.remove_clip")]
    RemoveClip {
        clip: ClipId,
        #[serde(default)]
        ripple: Ripple,
    },
    #[serde(rename = "timeline.move_clip")]
    MoveClip {
        clip: ClipId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<TrackId>,
        start: TimeInput,
    },
    #[serde(rename = "timeline.trim_clip")]
    TrimClip {
        clip: ClipId,
        edge: Edge,
        to: TimeInput,
        #[serde(default)]
        ripple: Ripple,
    },
    #[serde(rename = "timeline.split_clip")]
    SplitClip {
        clip: ClipId,
        at: TimeInput,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_id: Option<ClipId>,
        /// IDs for the right part's copies of the clip's effects, in order. Filled during
        /// resolution so redo and journal replay are deterministic.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_effect_ids: Option<Vec<EffectId>>,
    },
    #[serde(rename = "timeline.set_speed")]
    SetSpeed { clip: ClipId, speed: Ratio },
    #[serde(rename = "timeline.set_transform")]
    SetTransform {
        clip: ClipId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<Vec2>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<Vec2>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation_deg: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Vec2>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<f64>,
    },
    #[serde(rename = "timeline.set_enabled")]
    SetEnabled { clip: ClipId, enabled: bool },
    #[serde(rename = "timeline.set_label")]
    SetLabel { clip: ClipId, label: Option<String> },

    #[serde(rename = "timeline.add_transition")]
    AddTransition {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<TransitionId>,
        a: ClipId,
        b: ClipId,
        duration: TimeInput,
        #[serde(default = "default_transition_kind")]
        kind: TransitionKind,
    },
    #[serde(rename = "timeline.remove_transition")]
    RemoveTransition { transition: TransitionId },

    #[serde(rename = "effect.add")]
    EffectAdd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<EffectId>,
        clip: ClipId,
        effect: String,
        #[serde(default)]
        params: BTreeMap<String, Param>,
    },
    #[serde(rename = "effect.remove")]
    EffectRemove { clip: ClipId, effect: EffectId },
    #[serde(rename = "effect.set_params")]
    EffectSetParams {
        clip: ClipId,
        effect: EffectId,
        params: BTreeMap<String, Param>,
    },
    #[serde(rename = "effect.set_enabled")]
    EffectSetEnabled {
        clip: ClipId,
        effect: EffectId,
        enabled: bool,
    },

    #[serde(rename = "audio.set_gain")]
    AudioSetGain { clip: ClipId, gain_db: f64 },
    #[serde(rename = "audio.set_pan")]
    AudioSetPan { clip: ClipId, pan: f64 },
    #[serde(rename = "audio.set_fades")]
    AudioSetFades {
        clip: ClipId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fade_in: Option<TimeInput>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fade_out: Option<TimeInput>,
    },

    #[serde(rename = "title.add")]
    TitleAdd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ClipId>,
        track: TrackId,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        start: TimeInput,
        duration: TimeInput,
    },
    #[serde(rename = "title.set_text")]
    TitleSetText { clip: ClipId, text: String },

    #[serde(rename = "marker.add")]
    MarkerAdd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<MarkerId>,
        at: TimeInput,
        label: String,
    },
    #[serde(rename = "marker.remove")]
    MarkerRemove { marker: MarkerId },

    /// Engine-only inverse used for undo and journal replay.
    #[serde(rename = "internal.restore")]
    InternalRestore { restore: Restore },
}

fn default_transition_kind() -> TransitionKind {
    TransitionKind::CrossDissolve
}

impl Command {
    pub fn op(&self) -> &'static str {
        match self {
            Command::ProjectRename { .. } => "project.rename",
            Command::TrackAdd { .. } => "track.add",
            Command::TrackRemove { .. } => "track.remove",
            Command::TrackSetProperties { .. } => "track.set_properties",
            Command::AssetImport { .. } => "asset.import",
            Command::AssetRemove { .. } => "asset.remove",
            Command::AddClip { .. } => "timeline.add_clip",
            Command::RemoveClip { .. } => "timeline.remove_clip",
            Command::MoveClip { .. } => "timeline.move_clip",
            Command::TrimClip { .. } => "timeline.trim_clip",
            Command::SplitClip { .. } => "timeline.split_clip",
            Command::SetSpeed { .. } => "timeline.set_speed",
            Command::SetTransform { .. } => "timeline.set_transform",
            Command::SetEnabled { .. } => "timeline.set_enabled",
            Command::SetLabel { .. } => "timeline.set_label",
            Command::AddTransition { .. } => "timeline.add_transition",
            Command::RemoveTransition { .. } => "timeline.remove_transition",
            Command::EffectAdd { .. } => "effect.add",
            Command::EffectRemove { .. } => "effect.remove",
            Command::EffectSetParams { .. } => "effect.set_params",
            Command::EffectSetEnabled { .. } => "effect.set_enabled",
            Command::AudioSetGain { .. } => "audio.set_gain",
            Command::AudioSetPan { .. } => "audio.set_pan",
            Command::AudioSetFades { .. } => "audio.set_fades",
            Command::TitleAdd { .. } => "title.add",
            Command::TitleSetText { .. } => "title.set_text",
            Command::MarkerAdd { .. } => "marker.add",
            Command::MarkerRemove { .. } => "marker.remove",
            Command::InternalRestore { .. } => "internal.restore",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------------------------

fn err(kind: ErrorKind, msg: impl Into<String>) -> CommandError {
    CommandError::new(kind, msg)
}

fn seq_mut(p: &mut Project) -> CmdResult<&mut Sequence> {
    let id = p.settings.sequence.clone();
    p.sequences
        .get_mut(&id)
        .ok_or_else(|| err(ErrorKind::InvalidCommand, "project has no active sequence"))
}

fn rate_of(p: &Project) -> CmdResult<FrameRate> {
    p.sequence()
        .map(|s| s.frame_rate())
        .ok_or_else(|| err(ErrorKind::InvalidCommand, "project has no active sequence"))
}

fn locate(seq: &Sequence, clip: &ClipId) -> CmdResult<(usize, usize)> {
    seq.find_clip(clip)
        .ok_or_else(|| err(ErrorKind::UnknownClip, format!("unknown clip {clip}")))
}

fn track_idx(seq: &Sequence, track: &TrackId) -> CmdResult<usize> {
    seq.track_index(track)
        .ok_or_else(|| err(ErrorKind::UnknownTrack, format!("unknown track {track}")))
}

fn ensure_unlocked(seq: &Sequence, ti: usize) -> CmdResult<()> {
    match seq.tracks.get(ti) {
        Some(t) if t.locked => Err(err(
            ErrorKind::TrackLocked,
            format!("track {} is locked", t.id),
        )),
        _ => Ok(()),
    }
}

/// Mutable access to a clip, enforcing the track lock.
fn with_clip<R>(
    p: &mut Project,
    clip: &ClipId,
    f: impl FnOnce(&mut Clip) -> CmdResult<R>,
) -> CmdResult<R> {
    let seq = seq_mut(p)?;
    let (ti, ci) = locate(seq, clip)?;
    ensure_unlocked(seq, ti)?;
    let track = seq
        .tracks
        .get_mut(ti)
        .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
    let c = track
        .clips
        .get_mut(ci)
        .ok_or_else(|| err(ErrorKind::UnknownClip, "clip vanished"))?;
    f(c)
}

fn sort_track(track: &mut Track) {
    track.clips.sort_by(|a, b| a.start.cmp(&b.start));
}

/// Shift clips that start at or after `from` by `delta` (may be negative).
fn ripple_shift(
    seq: &mut Sequence,
    scope: Ripple,
    track: usize,
    from: Time,
    delta: Time,
) -> CmdResult<()> {
    if scope == Ripple::None || delta.is_zero() {
        return Ok(());
    }
    for (ti, t) in seq.tracks.iter_mut().enumerate() {
        if scope == Ripple::Track && ti != track {
            continue;
        }
        if t.locked && ti != track {
            continue;
        }
        for c in t.clips.iter_mut() {
            if c.start >= from {
                c.start = c.start.checked_add(delta)?;
            }
        }
        sort_track(t);
    }
    Ok(())
}

fn check_new_id<T: PartialEq + std::fmt::Display>(
    mut existing: impl Iterator<Item = T>,
    id: &T,
    label: &str,
) -> CmdResult<()> {
    if existing.any(|e| &e == id) {
        return Err(err(
            ErrorKind::DuplicateId,
            format!("{label} {id} already exists"),
        ));
    }
    Ok(())
}

fn all_clip_ids(seq: &Sequence) -> impl Iterator<Item = ClipId> + '_ {
    seq.tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.id.clone()))
}

fn all_effect_ids(seq: &Sequence) -> impl Iterator<Item = EffectId> + '_ {
    seq.tracks.iter().flat_map(|t| {
        t.clips
            .iter()
            .flat_map(|c| c.effects.iter().map(|e| e.id.clone()))
    })
}

fn all_transition_ids(seq: &Sequence) -> impl Iterator<Item = TransitionId> + '_ {
    seq.tracks
        .iter()
        .flat_map(|t| t.transitions.iter().map(|x| x.id.clone()))
}

fn finite(v: f64, what: &str) -> CmdResult<f64> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(err(
            ErrorKind::InvalidValue,
            format!("{what} must be finite"),
        ))
    }
}

/// Apply one command to a working copy. Resolves `cmd` in place and returns its `resulting` entry.
pub fn apply_command(
    p: &mut Project,
    cmd: &mut Command,
    catalog: &dyn EffectCatalog,
) -> CmdResult<Value> {
    let rate = rate_of(p)?;
    match cmd {
        Command::ProjectRename { name } => {
            if name.trim().is_empty() {
                return Err(err(ErrorKind::InvalidValue, "name must not be empty").at("name"));
            }
            p.meta.name = name.clone();
            Ok(json!({"op": "project.rename"}))
        }

        Command::TrackAdd {
            id,
            kind,
            name,
            index,
        } => {
            let seq = seq_mut(p)?;
            let new_id = id.clone().unwrap_or_else(TrackId::generate);
            check_new_id(seq.tracks.iter().map(|t| t.id.clone()), &new_id, "track")?;
            *id = Some(new_id.clone());
            let n = seq.tracks.iter().filter(|t| t.kind == *kind).count() + 1;
            let prefix = match kind {
                TrackKind::Video => "V",
                TrackKind::Audio => "A",
                TrackKind::Subtitle => "S",
            };
            let nm = name.clone().unwrap_or_else(|| format!("{prefix}{n}"));
            *name = Some(nm.clone());
            let t = Track {
                id: new_id.clone(),
                kind: *kind,
                name: nm,
                enabled: true,
                locked: false,
                opacity: 1.0,
                gain_db: 0.0,
                muted: false,
                clips: Vector::new(),
                transitions: Vector::new(),
            };
            let at = index.unwrap_or(seq.tracks.len());
            if at > seq.tracks.len() {
                return Err(err(ErrorKind::OutOfRange, "track index out of range").at("index"));
            }
            *index = Some(at);
            seq.tracks.insert(at, t);
            Ok(json!({"op": "track.add", "id": new_id}))
        }

        Command::TrackRemove { track } => {
            let seq = seq_mut(p)?;
            let ti = track_idx(seq, track)?;
            seq.tracks.remove(ti);
            Ok(json!({"op": "track.remove", "id": track}))
        }

        Command::TrackSetProperties {
            track,
            name,
            enabled,
            locked,
            opacity,
            gain_db,
            muted,
        } => {
            let seq = seq_mut(p)?;
            let ti = track_idx(seq, track)?;
            let t = seq
                .tracks
                .get_mut(ti)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            if let Some(n) = name {
                t.name = n.clone();
            }
            if let Some(v) = enabled {
                t.enabled = *v;
            }
            if let Some(v) = locked {
                t.locked = *v;
            }
            if let Some(v) = opacity {
                t.opacity = finite(*v, "opacity")?;
            }
            if let Some(v) = gain_db {
                t.gain_db = finite(*v, "gain_db")?;
            }
            if let Some(v) = muted {
                t.muted = *v;
            }
            Ok(json!({"op": "track.set_properties", "id": track}))
        }

        Command::AssetImport {
            id,
            path,
            kind,
            probe,
            status,
        } => {
            let new_id = id.clone().unwrap_or_else(AssetId::generate);
            if p.assets.contains_key(&new_id) {
                return Err(err(
                    ErrorKind::DuplicateId,
                    format!("asset {new_id} already exists"),
                )
                .at("id"));
            }
            *id = Some(new_id.clone());
            p.assets.insert(
                new_id.clone(),
                Asset {
                    id: new_id.clone(),
                    kind: *kind,
                    path: path.clone(),
                    status: *status,
                    probe: probe.clone(),
                    content_hash: None,
                },
            );
            Ok(json!({"op": "asset.import", "id": new_id, "status": status}))
        }

        Command::AssetRemove { asset } => {
            if !p.assets.contains_key(asset) {
                return Err(err(
                    ErrorKind::UnknownAsset,
                    format!("unknown asset {asset}"),
                ));
            }
            let used = p.sequences.values().any(|s| {
                s.tracks
                    .iter()
                    .any(|t| t.clips.iter().any(|c| c.asset.as_ref() == Some(asset)))
            });
            if used {
                return Err(err(
                    ErrorKind::AssetInUse,
                    format!("asset {asset} is used by clips"),
                ));
            }
            p.assets.remove(asset);
            Ok(json!({"op": "asset.remove", "id": asset}))
        }

        Command::AddClip {
            id,
            track,
            asset,
            start,
            duration,
            source_in,
            speed,
        } => {
            let start = start.resolve(rate).map_err(|e| e.at("start"))?;
            let duration = duration.resolve(rate).map_err(|e| e.at("duration"))?;
            let source_in_t = match source_in {
                Some(s) => s.resolve(rate).map_err(|e| e.at("source_in"))?,
                None => {
                    *source_in = Some(TimeInput::Exact(Time::ZERO));
                    Time::ZERO
                }
            };
            let sp = speed.unwrap_or(Ratio::ONE);
            *speed = Some(sp);
            let Some(asset_id) = asset.clone() else {
                return Err(err(
                    ErrorKind::InvalidCommand,
                    "asset is required (use title.add for titles)",
                )
                .at("asset"));
            };
            if !p.assets.contains_key(&asset_id) {
                return Err(
                    err(ErrorKind::UnknownAsset, format!("unknown asset {asset_id}")).at("asset"),
                );
            }
            let seq = seq_mut(p)?;
            let ti = track_idx(seq, track).map_err(|e| e.at("track"))?;
            ensure_unlocked(seq, ti)?;
            let new_id = id.clone().unwrap_or_else(ClipId::generate);
            check_new_id(all_clip_ids(seq), &new_id, "clip").map_err(|e| e.at("id"))?;
            *id = Some(new_id.clone());
            let clip = Clip {
                id: new_id.clone(),
                asset: Some(asset_id),
                title: None,
                start,
                duration,
                source_in: source_in_t,
                speed: sp,
                transform: Transform::default(),
                effects: Vector::new(),
                audio: ClipAudio::default(),
                label: None,
                enabled: true,
            };
            let t = seq
                .tracks
                .get_mut(ti)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            t.clips.push_back(clip);
            sort_track(t);
            Ok(json!({"op": "timeline.add_clip", "id": new_id}))
        }

        Command::TitleAdd {
            id,
            track,
            text,
            style,
            start,
            duration,
        } => {
            let start = start.resolve(rate).map_err(|e| e.at("start"))?;
            let duration = duration.resolve(rate).map_err(|e| e.at("duration"))?;
            let seq = seq_mut(p)?;
            let ti = track_idx(seq, track).map_err(|e| e.at("track"))?;
            ensure_unlocked(seq, ti)?;
            let new_id = id.clone().unwrap_or_else(ClipId::generate);
            check_new_id(all_clip_ids(seq), &new_id, "clip").map_err(|e| e.at("id"))?;
            *id = Some(new_id.clone());
            let st = style.clone().unwrap_or_else(|| "lower_third".into());
            *style = Some(st.clone());
            let clip = Clip {
                id: new_id.clone(),
                asset: None,
                title: Some(Title {
                    text: text.clone(),
                    style: st,
                }),
                start,
                duration,
                source_in: Time::ZERO,
                speed: Ratio::ONE,
                transform: Transform::default(),
                effects: Vector::new(),
                audio: ClipAudio::default(),
                label: None,
                enabled: true,
            };
            let t = seq
                .tracks
                .get_mut(ti)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            t.clips.push_back(clip);
            sort_track(t);
            Ok(json!({"op": "title.add", "id": new_id}))
        }

        Command::TitleSetText { clip, text } => {
            with_clip(p, clip, |c| match &mut c.title {
                Some(t) => {
                    t.text = text.clone();
                    Ok(())
                }
                None => Err(err(ErrorKind::InvalidCommand, "clip is not a title")),
            })?;
            Ok(json!({"op": "title.set_text", "id": clip}))
        }

        Command::RemoveClip { clip, ripple } => {
            let seq = seq_mut(p)?;
            let (ti, ci) = locate(seq, clip)?;
            ensure_unlocked(seq, ti)?;
            let t = seq
                .tracks
                .get_mut(ti)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            let removed = t.clips.remove(ci);
            // cascade: transitions referencing the clip are removed with it
            let before = t.transitions.len();
            t.transitions.retain(|x| x.a != *clip && x.b != *clip);
            let dropped_transitions = before - t.transitions.len();
            let end = removed.end()?;
            let delta = Time::ZERO.checked_sub(removed.duration)?;
            ripple_shift(seq, *ripple, ti, end, delta)?;
            Ok(
                json!({"op": "timeline.remove_clip", "id": clip, "removed_transitions": dropped_transitions}),
            )
        }

        Command::MoveClip { clip, track, start } => {
            let start = start.resolve(rate).map_err(|e| e.at("start"))?;
            let seq = seq_mut(p)?;
            let (ti, ci) = locate(seq, clip)?;
            ensure_unlocked(seq, ti)?;
            let dest = match track {
                Some(tid) => track_idx(seq, tid).map_err(|e| e.at("track"))?,
                None => ti,
            };
            ensure_unlocked(seq, dest)?;
            let src = seq
                .tracks
                .get_mut(ti)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            if dest != ti && src.transitions.iter().any(|x| x.a == *clip || x.b == *clip) {
                return Err(err(
                    ErrorKind::TransitionBroken,
                    "remove the clip's transitions before moving it to another track",
                ));
            }
            let mut c = src.clips.remove(ci);
            c.start = start;
            let d = seq
                .tracks
                .get_mut(dest)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            d.clips.push_back(c);
            sort_track(d);
            Ok(json!({"op": "timeline.move_clip", "id": clip}))
        }

        Command::TrimClip {
            clip,
            edge,
            to,
            ripple,
        } => {
            let to = to.resolve(rate).map_err(|e| e.at("to"))?;
            let seq = seq_mut(p)?;
            let (ti, ci) = locate(seq, clip)?;
            ensure_unlocked(seq, ti)?;
            let t = seq
                .tracks
                .get_mut(ti)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            let c = t
                .clips
                .get_mut(ci)
                .ok_or_else(|| err(ErrorKind::UnknownClip, "clip vanished"))?;
            let old_end = c.end()?;
            match edge {
                Edge::Out => {
                    if to <= c.start {
                        return Err(err(
                            ErrorKind::OutOfRange,
                            "out edge must be after the clip start",
                        )
                        .at("to"));
                    }
                    c.duration = to.checked_sub(c.start)?;
                    let delta = to.checked_sub(old_end)?;
                    ripple_shift(seq, *ripple, ti, old_end, delta)?;
                }
                Edge::In => {
                    if *ripple != Ripple::None {
                        return Err(err(
                            ErrorKind::UnsupportedRipple,
                            "ripple is only supported on the out edge",
                        ));
                    }
                    if to >= old_end {
                        return Err(err(
                            ErrorKind::OutOfRange,
                            "in edge must be before the clip end",
                        )
                        .at("to"));
                    }
                    let shift = to.checked_sub(c.start)?; // timeline delta
                    c.source_in = c.source_in.checked_add(shift.checked_mul(c.speed)?)?;
                    c.start = to;
                    c.duration = old_end.checked_sub(to)?;
                    sort_track(t);
                }
            }
            Ok(json!({"op": "timeline.trim_clip", "id": clip}))
        }

        Command::SplitClip {
            clip,
            at,
            new_id,
            new_effect_ids,
        } => {
            let at_t = at.resolve(rate).map_err(|e| e.at("at"))?;
            let seq = seq_mut(p)?;
            let (ti, ci) = locate(seq, clip)?;
            ensure_unlocked(seq, ti)?;
            let nid = new_id.clone().unwrap_or_else(ClipId::generate);
            check_new_id(all_clip_ids(seq), &nid, "clip").map_err(|e| e.at("new_id"))?;
            *new_id = Some(nid.clone());
            let t = seq
                .tracks
                .get_mut(ti)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            let c = t
                .clips
                .get_mut(ci)
                .ok_or_else(|| err(ErrorKind::UnknownClip, "clip vanished"))?;
            let end = c.end()?;
            if at_t <= c.start || at_t >= end {
                return Err(err(
                    ErrorKind::OutOfRange,
                    "split point must be strictly inside the clip",
                )
                .at("at"));
            }
            let left_dur = at_t.checked_sub(c.start)?;
            let fx_ids: Vec<EffectId> = match new_effect_ids {
                Some(v) if v.len() == c.effects.len() => v.clone(),
                Some(_) => {
                    return Err(err(
                        ErrorKind::InvalidValue,
                        "new_effect_ids must match the clip's effect count",
                    )
                    .at("new_effect_ids"));
                }
                None => c.effects.iter().map(|_| EffectId::generate()).collect(),
            };
            *new_effect_ids = Some(fx_ids.clone());
            let mut right = c.clone();
            right.id = nid.clone();
            right.start = at_t;
            right.duration = end.checked_sub(at_t)?;
            right.source_in = c.source_in.checked_add(left_dur.checked_mul(c.speed)?)?;
            right.effects = right
                .effects
                .iter()
                .zip(fx_ids)
                .map(|(e, id)| EffectInstance { id, ..e.clone() })
                .collect();
            right.audio.fade_in = Time::ZERO;
            c.duration = left_dur;
            c.audio.fade_out = Time::ZERO;
            t.clips.insert(ci + 1, right);
            // a transition leaving the original clip now leaves the right part
            for x in t.transitions.iter_mut() {
                if x.a == *clip {
                    x.a = nid.clone();
                }
            }
            Ok(json!({"op": "timeline.split_clip", "id": clip, "new_id": nid}))
        }

        Command::SetSpeed { clip, speed } => {
            let sp = *speed;
            with_clip(p, clip, |c| {
                c.speed = sp;
                Ok(())
            })?;
            Ok(json!({"op": "timeline.set_speed", "id": clip}))
        }

        Command::SetTransform {
            clip,
            position,
            scale,
            rotation_deg,
            anchor,
            opacity,
        } => {
            let (pos, sc, rot, anc, op) = (*position, *scale, *rotation_deg, *anchor, *opacity);
            with_clip(p, clip, |c| {
                if let Some(v) = pos {
                    c.transform.position = v;
                }
                if let Some(v) = sc {
                    c.transform.scale = v;
                }
                if let Some(v) = rot {
                    c.transform.rotation_deg = v;
                }
                if let Some(v) = anc {
                    c.transform.anchor = v;
                }
                if let Some(v) = op {
                    c.transform.opacity = v;
                }
                Ok(())
            })?;
            Ok(json!({"op": "timeline.set_transform", "id": clip}))
        }

        Command::SetEnabled { clip, enabled } => {
            let v = *enabled;
            with_clip(p, clip, |c| {
                c.enabled = v;
                Ok(())
            })?;
            Ok(json!({"op": "timeline.set_enabled", "id": clip}))
        }

        Command::SetLabel { clip, label } => {
            let v = label.clone();
            with_clip(p, clip, |c| {
                c.label = v;
                Ok(())
            })?;
            Ok(json!({"op": "timeline.set_label", "id": clip}))
        }

        Command::AddTransition {
            id,
            a,
            b,
            duration,
            kind,
        } => {
            let d = duration.resolve(rate).map_err(|e| e.at("duration"))?;
            let seq = seq_mut(p)?;
            let (ta, _) = locate(seq, a).map_err(|e| e.at("a"))?;
            let (tb, _) = locate(seq, b).map_err(|e| e.at("b"))?;
            if ta != tb {
                return Err(err(
                    ErrorKind::TransitionInvalid,
                    "a and b must be on the same track",
                ));
            }
            ensure_unlocked(seq, ta)?;
            let nid = id.clone().unwrap_or_else(TransitionId::generate);
            check_new_id(all_transition_ids(seq), &nid, "transition").map_err(|e| e.at("id"))?;
            *id = Some(nid.clone());
            let t = seq
                .tracks
                .get_mut(ta)
                .ok_or_else(|| err(ErrorKind::UnknownTrack, "track vanished"))?;
            t.transitions.push_back(Transition {
                id: nid.clone(),
                kind: *kind,
                a: a.clone(),
                b: b.clone(),
                duration: d,
            });
            Ok(json!({"op": "timeline.add_transition", "id": nid}))
        }

        Command::RemoveTransition { transition } => {
            let seq = seq_mut(p)?;
            for (ti, t) in seq.tracks.iter().enumerate() {
                if let Some(xi) = t.transitions.iter().position(|x| x.id == *transition) {
                    ensure_unlocked(seq, ti)?;
                    if let Some(t) = seq.tracks.get_mut(ti) {
                        t.transitions.remove(xi);
                    }
                    return Ok(json!({"op": "timeline.remove_transition", "id": transition}));
                }
            }
            Err(err(
                ErrorKind::UnknownTransition,
                format!("unknown transition {transition}"),
            ))
        }

        Command::EffectAdd {
            id,
            clip,
            effect,
            params,
        } => {
            let manifest = catalog.manifest(effect).ok_or_else(|| {
                err(ErrorKind::UnknownEffect, format!("unknown effect {effect}")).at("effect")
            })?;
            // fill defaults so the stored instance is complete
            for spec in &manifest.params {
                params
                    .entry(spec.key.clone())
                    .or_insert(Param::constant(spec.default));
            }
            let nid = id.clone().unwrap_or_else(EffectId::generate);
            {
                let seq = seq_mut(p)?;
                check_new_id(all_effect_ids(seq), &nid, "effect").map_err(|e| e.at("id"))?;
            }
            *id = Some(nid.clone());
            let inst = EffectInstance {
                id: nid.clone(),
                effect: effect.clone(),
                enabled: true,
                params: params.clone(),
            };
            with_clip(p, clip, |c| {
                c.effects.push_back(inst);
                Ok(())
            })?;
            Ok(json!({"op": "effect.add", "id": nid}))
        }

        Command::EffectRemove { clip, effect } => {
            let e = effect.clone();
            with_clip(p, clip, |c| {
                let i = c.effects.iter().position(|x| x.id == e).ok_or_else(|| {
                    err(
                        ErrorKind::UnknownEffect,
                        format!("unknown effect instance {e}"),
                    )
                })?;
                c.effects.remove(i);
                Ok(())
            })?;
            Ok(json!({"op": "effect.remove", "id": effect}))
        }

        Command::EffectSetParams {
            clip,
            effect,
            params,
        } => {
            let e = effect.clone();
            let ps = params.clone();
            with_clip(p, clip, |c| {
                let inst = c.effects.iter_mut().find(|x| x.id == e).ok_or_else(|| {
                    err(
                        ErrorKind::UnknownEffect,
                        format!("unknown effect instance {e}"),
                    )
                })?;
                for (k, v) in ps {
                    inst.params.insert(k, v);
                }
                Ok(())
            })?;
            Ok(json!({"op": "effect.set_params", "id": effect}))
        }

        Command::EffectSetEnabled {
            clip,
            effect,
            enabled,
        } => {
            let (e, en) = (effect.clone(), *enabled);
            with_clip(p, clip, |c| {
                let inst = c.effects.iter_mut().find(|x| x.id == e).ok_or_else(|| {
                    err(
                        ErrorKind::UnknownEffect,
                        format!("unknown effect instance {e}"),
                    )
                })?;
                inst.enabled = en;
                Ok(())
            })?;
            Ok(json!({"op": "effect.set_enabled", "id": effect}))
        }

        Command::AudioSetGain { clip, gain_db } => {
            let g = *gain_db;
            with_clip(p, clip, |c| {
                c.audio.gain_db = g;
                Ok(())
            })?;
            Ok(json!({"op": "audio.set_gain", "id": clip}))
        }

        Command::AudioSetPan { clip, pan } => {
            let v = *pan;
            with_clip(p, clip, |c| {
                c.audio.pan = v;
                Ok(())
            })?;
            Ok(json!({"op": "audio.set_pan", "id": clip}))
        }

        Command::AudioSetFades {
            clip,
            fade_in,
            fade_out,
        } => {
            // Fades are sample-grained: exact rationals are accepted off frame boundaries.
            let fi = match fade_in {
                Some(x) => Some(x.resolve(rate).map_err(|e| e.at("fade_in"))?),
                None => None,
            };
            let fo = match fade_out {
                Some(x) => Some(x.resolve(rate).map_err(|e| e.at("fade_out"))?),
                None => None,
            };
            with_clip(p, clip, |c| {
                if let Some(v) = fi {
                    c.audio.fade_in = v;
                }
                if let Some(v) = fo {
                    c.audio.fade_out = v;
                }
                Ok(())
            })?;
            Ok(json!({"op": "audio.set_fades", "id": clip}))
        }

        Command::MarkerAdd { id, at, label } => {
            let at_t = at.resolve(rate).map_err(|e| e.at("at"))?;
            let seq = seq_mut(p)?;
            let nid = id.clone().unwrap_or_else(MarkerId::generate);
            check_new_id(seq.markers.iter().map(|m| m.id.clone()), &nid, "marker")
                .map_err(|e| e.at("id"))?;
            *id = Some(nid.clone());
            seq.markers.push_back(Marker {
                id: nid.clone(),
                at: at_t,
                label: label.clone(),
            });
            seq.markers
                .sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
            Ok(json!({"op": "marker.add", "id": nid}))
        }

        Command::MarkerRemove { marker } => {
            let seq = seq_mut(p)?;
            let i = seq
                .markers
                .iter()
                .position(|m| m.id == *marker)
                .ok_or_else(|| err(ErrorKind::UnknownMarker, format!("unknown marker {marker}")))?;
            seq.markers.remove(i);
            Ok(json!({"op": "marker.remove", "id": marker}))
        }

        Command::InternalRestore { restore } => {
            if let Some(n) = &restore.name {
                p.meta.name = n.clone();
            }
            for patch in &restore.assets {
                match &patch.asset {
                    Some(a) => {
                        p.assets.insert(patch.id.clone(), a.clone());
                    }
                    None => {
                        p.assets.remove(&patch.id);
                    }
                }
            }
            let seq = seq_mut(p)?;
            if let Some(tr) = &restore.tracks {
                seq.tracks = tr.iter().cloned().collect();
            }
            if let Some(m) = &restore.markers {
                seq.markers = m.iter().cloned().collect();
            }
            Ok(json!({"op": "internal.restore"}))
        }
    }
}

/// The inverse of going from `before` to `after`, as an engine-only restore command.
pub fn inverse_of(before: &Project, after: &Project) -> Command {
    let mut r = Restore {
        name: None,
        tracks: None,
        markers: None,
        assets: Vec::new(),
    };
    if before.meta.name != after.meta.name {
        r.name = Some(before.meta.name.clone());
    }
    if let (Some(b), Some(a)) = (before.sequence(), after.sequence()) {
        if b.tracks != a.tracks {
            r.tracks = Some(b.tracks.iter().cloned().collect());
        }
        if b.markers != a.markers {
            r.markers = Some(b.markers.iter().cloned().collect());
        }
    }
    let mut ids: Vec<&AssetId> = before.assets.keys().chain(after.assets.keys()).collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        let (b, a) = (before.assets.get(id), after.assets.get(id));
        if b != a {
            r.assets.push(AssetPatch {
                id: id.clone(),
                asset: b.cloned(),
            });
        }
    }
    Command::InternalRestore { restore: r }
}

// ---------------------------------------------------------------------------------------------
// Invariants (docs/02 §9) — checked after every batch against the whole working copy.
// ---------------------------------------------------------------------------------------------

pub fn validate_project(p: &Project, catalog: &dyn EffectCatalog) -> CmdResult<()> {
    if p.sequences.len() != 1 {
        return Err(err(
            ErrorKind::InvalidCommand,
            "v1 projects have exactly one sequence",
        ));
    }
    let seq = p
        .sequence()
        .ok_or_else(|| err(ErrorKind::InvalidCommand, "active sequence missing"))?;
    let rate = seq.frame_rate();
    let cap = Time::secs(MAX_SEQUENCE_SECONDS);

    let mut track_ids = HashSet::new();
    let mut clip_ids = HashSet::new();
    let mut effect_ids = HashSet::new();
    let mut transition_ids = HashSet::new();

    for t in seq.tracks.iter() {
        let tp = format!("track {}", t.id);
        if !track_ids.insert(t.id.clone()) {
            return Err(err(
                ErrorKind::DuplicateId,
                format!("duplicate track {}", t.id),
            ));
        }
        if !(0.0..=1.0).contains(&t.opacity) {
            return Err(err(
                ErrorKind::OutOfRange,
                format!("{tp}: opacity must be within 0..=1"),
            ));
        }
        if !t.gain_db.is_finite() || !(-96.0..=24.0).contains(&t.gain_db) {
            return Err(err(
                ErrorKind::OutOfRange,
                format!("{tp}: gain_db must be within -96..=24"),
            ));
        }
        let mut prev_end: Option<(Time, &ClipId)> = None;
        for c in t.clips.iter() {
            validate_clip(p, t, c, rate, cap, catalog)?;
            if !clip_ids.insert(c.id.clone()) {
                return Err(err(
                    ErrorKind::DuplicateId,
                    format!("duplicate clip {}", c.id),
                ));
            }
            for e in c.effects.iter() {
                if !effect_ids.insert(e.id.clone()) {
                    return Err(err(
                        ErrorKind::DuplicateId,
                        format!("duplicate effect {}", e.id),
                    ));
                }
            }
            if let Some((end, other)) = prev_end {
                if c.start < end {
                    return Err(err(
                        ErrorKind::Overlap,
                        format!("clip {} overlaps clip {} on {}", c.id, other, t.id),
                    )
                    .with_details(serde_json::json!({"track": t.id, "clips": [other, c.id]})));
                }
            }
            prev_end = Some((c.end()?, &c.id));
        }
        for x in t.transitions.iter() {
            if !transition_ids.insert(x.id.clone()) {
                return Err(err(
                    ErrorKind::DuplicateId,
                    format!("duplicate transition {}", x.id),
                ));
            }
            validate_transition(p, t, x, rate)?;
        }
    }
    for m in seq.markers.iter() {
        if m.at.is_negative() || m.at > cap {
            return Err(err(
                ErrorKind::OutOfRange,
                format!("marker {} outside the sequence range", m.id),
            ));
        }
    }
    Ok(())
}

fn validate_clip(
    p: &Project,
    t: &Track,
    c: &Clip,
    rate: FrameRate,
    cap: Time,
    catalog: &dyn EffectCatalog,
) -> CmdResult<()> {
    let cp = |m: &str| format!("clip {}: {m}", c.id);
    match (&c.asset, &c.title) {
        (Some(a), None) => {
            let asset = p
                .assets
                .get(a)
                .ok_or_else(|| err(ErrorKind::UnknownAsset, cp(&format!("unknown asset {a}"))))?;
            let ok = match (t.kind, asset.kind) {
                (TrackKind::Video, AssetKind::Video | AssetKind::Image) => true,
                (TrackKind::Audio, AssetKind::Audio) => true,
                (TrackKind::Audio, AssetKind::Video) => {
                    asset.probe.as_ref().is_some_and(|pr| pr.audio.is_some())
                }
                _ => false,
            };
            if !ok {
                return Err(err(
                    ErrorKind::TrackKindMismatch,
                    cp(&format!(
                        "{:?} asset cannot go on a {:?} track",
                        asset.kind, t.kind
                    )),
                ));
            }
            if asset.kind != AssetKind::Image {
                if let Some(pr) = &asset.probe {
                    if c.source_out()? > pr.duration {
                        return Err(err(ErrorKind::SourceRangeExceeded, cp("source range exceeds the media duration"))
                            .with_details(serde_json::json!({"source_out": c.source_out()?, "media_duration": pr.duration})));
                    }
                }
            }
        }
        (None, Some(_)) => {
            if t.kind == TrackKind::Audio {
                return Err(err(
                    ErrorKind::TrackKindMismatch,
                    cp("titles cannot go on audio tracks"),
                ));
            }
        }
        _ => {
            return Err(err(
                ErrorKind::InvalidCommand,
                cp("exactly one of asset / title must be set"),
            ));
        }
    }
    if c.duration <= Time::ZERO {
        return Err(err(ErrorKind::OutOfRange, cp("duration must be positive")));
    }
    if c.start.is_negative() || c.source_in.is_negative() {
        return Err(err(
            ErrorKind::OutOfRange,
            cp("start and source_in must be non-negative"),
        ));
    }
    if c.end()? > cap {
        return Err(err(
            ErrorKind::OutOfRange,
            cp("clip ends past the 24 h sequence limit"),
        ));
    }
    for (name, v) in [
        ("start", c.start),
        ("duration", c.duration),
        ("source_in", c.source_in),
    ] {
        if !rate.is_on_boundary(v) {
            return Err(err(
                ErrorKind::OffFrameBoundary,
                cp(&format!(
                    "{name} {v} is not on a {} frame boundary",
                    rate.label()
                )),
            ));
        }
    }
    if !c.speed.is_positive() || c.speed > Ratio::new(16, 1)? {
        return Err(err(
            ErrorKind::OutOfRange,
            cp("speed must be within (0, 16]"),
        ));
    }
    let tr = c.transform;
    for v in [
        tr.position.x,
        tr.position.y,
        tr.scale.x,
        tr.scale.y,
        tr.rotation_deg,
        tr.anchor.x,
        tr.anchor.y,
        tr.opacity,
    ] {
        finite(v, "transform")?;
    }
    if !(0.0..=1.0).contains(&tr.opacity) {
        return Err(err(
            ErrorKind::OutOfRange,
            cp("opacity must be within 0..=1"),
        ));
    }
    let a = c.audio;
    if !a.gain_db.is_finite() || !(-96.0..=24.0).contains(&a.gain_db) {
        return Err(err(
            ErrorKind::OutOfRange,
            cp("gain_db must be within -96..=24"),
        ));
    }
    if !a.pan.is_finite() || !(-1.0..=1.0).contains(&a.pan) {
        return Err(err(ErrorKind::OutOfRange, cp("pan must be within -1..=1")));
    }
    if a.fade_in.is_negative()
        || a.fade_out.is_negative()
        || a.fade_in.checked_add(a.fade_out)? > c.duration
    {
        return Err(err(
            ErrorKind::OutOfRange,
            cp("fades must be non-negative and fit in the clip"),
        ));
    }
    for e in c.effects.iter() {
        let m = catalog.manifest(&e.effect).ok_or_else(|| {
            err(
                ErrorKind::UnknownEffect,
                cp(&format!("unknown effect {}", e.effect)),
            )
        })?;
        for (k, v) in &e.params {
            let spec = m.param(k).ok_or_else(|| {
                err(
                    ErrorKind::InvalidValue,
                    cp(&format!("effect {} has no parameter {k}", e.effect)),
                )
            })?;
            let values: Vec<f64> = match v {
                Param::Static { value } => vec![*value],
                Param::Curve { curve } => {
                    if curve.is_empty() {
                        return Err(err(
                            ErrorKind::InvalidValue,
                            cp(&format!("{k}: curve must not be empty")),
                        ));
                    }
                    if curve.windows(2).any(|w| w[0].t >= w[1].t) {
                        return Err(err(
                            ErrorKind::InvalidValue,
                            cp(&format!("{k}: keyframe times must strictly increase")),
                        ));
                    }
                    curve.iter().map(|kf| kf.v).collect()
                }
            };
            for x in values {
                if !x.is_finite() || x < spec.range[0] || x > spec.range[1] {
                    return Err(err(
                        ErrorKind::OutOfRange,
                        cp(&format!(
                            "{k}={x} outside [{}, {}]",
                            spec.range[0], spec.range[1]
                        )),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_transition(p: &Project, t: &Track, x: &Transition, rate: FrameRate) -> CmdResult<()> {
    let find = |id: &ClipId| t.clips.iter().position(|c| &c.id == id);
    let (Some(ia), Some(ib)) = (find(&x.a), find(&x.b)) else {
        return Err(err(
            ErrorKind::TransitionBroken,
            format!(
                "transition {} references a clip that is not on its track",
                x.id
            ),
        ));
    };
    let (a, b) = (&t.clips[ia], &t.clips[ib]);
    if ib != ia + 1 || a.end()? != b.start {
        return Err(err(
            ErrorKind::TransitionBroken,
            format!(
                "transition {}: clips {} and {} are not adjacent",
                x.id, a.id, b.id
            ),
        ));
    }
    if x.duration <= Time::ZERO || !rate.is_on_boundary(x.duration) {
        return Err(err(
            ErrorKind::TransitionInvalid,
            format!(
                "transition {}: duration must be positive and frame-aligned",
                x.id
            ),
        ));
    }
    if x.duration > a.duration || x.duration > b.duration {
        return Err(err(
            ErrorKind::TransitionInvalid,
            format!("transition {} is longer than one of its clips", x.id),
        ));
    }
    // centered: needs half the duration of source handle on both sides
    let half = x.duration.checked_div(Ratio::new(2, 1)?)?;
    let a_need = a.source_out()?.checked_add(half.checked_mul(a.speed)?)?;
    let b_need = b.source_in.checked_sub(half.checked_mul(b.speed)?)?;
    let a_dur = a
        .asset
        .as_ref()
        .and_then(|id| p.assets.get(id))
        .and_then(|s| s.probe.as_ref())
        .map(|pr| pr.duration);
    if a.title.is_none() {
        if let Some(d) = a_dur {
            if a_need > d {
                return Err(err(
                    ErrorKind::TransitionInvalid,
                    format!(
                        "transition {}: clip {} lacks source handle after its out point",
                        x.id, a.id
                    ),
                ));
            }
        }
    }
    if b.title.is_none() && b_need.is_negative() {
        return Err(err(
            ErrorKind::TransitionInvalid,
            format!(
                "transition {}: clip {} lacks source handle before its in point",
                x.id, b.id
            ),
        ));
    }
    Ok(())
}
