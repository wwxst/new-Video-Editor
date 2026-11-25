//! Read-side views shared by the CLI, JSON-RPC, and MCP (docs/06 §4.3, §5).

use arc_core::model::{Project, Sequence, Track, TrackKind};
use arc_core::time::{FrameRate, Time};
use serde_json::{Value, json};
use std::fmt::Write as _;

fn tc(rate: FrameRate, t: Time) -> String {
    rate.time_to_timecode(t)
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn track_label(seq: &Sequence, t: &Track) -> String {
    let n = seq
        .tracks
        .iter()
        .filter(|x| x.kind == t.kind)
        .position(|x| x.id == t.id)
        .unwrap_or(0)
        + 1;
    let k = match t.kind {
        TrackKind::Video => "V",
        TrackKind::Audio => "A",
        TrackKind::Subtitle => "S",
    };
    format!("{k}{n}")
}

/// `project.describe`: compact summary, NDF timecodes; `exact` adds rationals.
pub fn describe(project: &Project, revision: u64, exact: bool) -> Value {
    let Some(seq) = project.sequence() else {
        return json!({"revision": revision, "issues": ["project has no sequence"]});
    };
    let rate = seq.frame_rate();
    let mut clips = Vec::new();
    let mut tracks = Vec::new();
    for t in seq.tracks.iter() {
        tracks.push(json!({
            "id": t.id, "kind": t.kind, "name": t.name, "label": track_label(seq, t),
            "clips": t.clips.len(), "enabled": t.enabled, "locked": t.locked,
        }));
        for c in t.clips.iter() {
            let end = c.end().unwrap_or(c.start);
            let mut v = json!({
                "id": c.id,
                "track": t.id,
                "start": tc(rate, c.start),
                "end": tc(rate, end),
                "source_in": tc(rate, c.source_in),
                "effects": c.effects.iter().map(|e| e.effect.clone()).collect::<Vec<_>>(),
                "enabled": c.enabled,
            });
            if let Some(a) = &c.asset {
                v["asset"] = json!(a);
                if let Some(asset) = project.assets.get(a) {
                    v["media"] = json!(file_name(&asset.path));
                }
            }
            if let Some(title) = &c.title {
                v["title"] = json!(title.text);
            }
            if c.speed != arc_core::Ratio::ONE {
                v["speed"] = json!(c.speed.to_string());
            }
            if exact {
                v["exact"] =
                    json!({"start": c.start, "duration": c.duration, "source_in": c.source_in});
            }
            clips.push(v);
        }
    }
    let assets: Vec<Value> = project
        .assets
        .values()
        .map(|a| {
            let mut v = json!({"id": a.id, "path": a.path, "kind": a.kind, "status": a.status});
            if let Some(p) = &a.probe {
                v["duration"] = json!(tc(rate, p.duration));
                if let Some(vi) = &p.video {
                    v["frame_rate"] = json!(vi.frame_rate.to_string());
                    v["vfr"] = json!(vi.vfr);
                    v["resolution"] = json!([vi.width, vi.height]);
                }
                if let Some(au) = &p.audio {
                    v["audio"] = json!({"sample_rate": au.sample_rate, "channels": au.channels});
                }
            }
            v
        })
        .collect();
    let missing: Vec<Value> = project
        .assets
        .values()
        .filter(|a| a.status != arc_core::model::AssetStatus::Ok)
        .map(|a| json!({"id": a.id, "status": a.status}))
        .collect();
    json!({
        "revision": revision,
        "project": {"id": project.meta.id, "name": project.meta.name},
        "sequence": {
            "id": seq.id,
            "name": seq.name,
            "duration": tc(rate, seq.duration()),
            "frame_rate": rate.wire(),
            "resolution": [seq.resolution.w, seq.resolution.h],
            "sample_rate": seq.sample_rate,
        },
        "tracks": tracks,
        "clips": clips,
        "assets": assets,
        "markers": seq.markers.iter().map(|m| json!({"id": m.id, "at": tc(rate, m.at), "label": m.label})).collect::<Vec<_>>(),
        "issues": [],
        "missing": missing,
    })
}

fn quote(s: &str) -> String {
    // quoted-name cannot contain '"' (docs/schema/timeline-text.abnf); replace it.
    format!("\"{}\"", s.replace('"', "'"))
}

fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.to_string()
    }
}

fn dur_text(t: Time) -> String {
    let ms = (t.to_f64_lossy() * 1000.0).round() as i64;
    if ms % 1000 == 0 {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

/// `inspect.timeline`: the read-only text view (docs/06 §5, grammar in docs/schema).
/// `range` limits clips to those overlapping `[from, to)`.
pub fn timeline_text(project: &Project, range: Option<(Time, Time)>) -> String {
    let mut out = String::from("# arcline timeline v1\n");
    let Some(seq) = project.sequence() else {
        return out;
    };
    let rate = seq.frame_rate();
    let _ = writeln!(
        out,
        "SEQ {} {}x{} fps={} dur={}",
        quote(&seq.name),
        seq.resolution.w,
        seq.resolution.h,
        rate.wire(),
        tc(rate, seq.duration())
    );
    for t in seq.tracks.iter() {
        let mut line = format!("TRK {} name={}", track_label(seq, t), quote(&t.name));
        match t.kind {
            TrackKind::Video => line.push_str(&format!(" op={}", num(t.opacity))),
            TrackKind::Audio => line.push_str(&format!(" gain={}dB", num(t.gain_db))),
            TrackKind::Subtitle => {}
        }
        if !t.enabled {
            line.push_str(" enabled=false");
        }
        if t.locked {
            line.push_str(" locked=true");
        }
        out.push_str(&line);
        out.push('\n');
        for c in t.clips.iter() {
            let end = c.end().unwrap_or(c.start);
            if let Some((from, to)) = range {
                if end <= from || c.start >= to {
                    continue;
                }
            }
            let source = match (&c.asset, &c.title) {
                (Some(a), _) => quote(
                    &project
                        .assets
                        .get(a)
                        .map(|x| file_name(&x.path))
                        .unwrap_or_else(|| a.to_string()),
                ),
                (None, Some(ti)) => format!("<< TITLE {} >>", quote(&ti.text)),
                _ => "\"?\"".into(),
            };
            let mut l = format!(
                "  [{}] {} in={} len={} @{}",
                c.id,
                source,
                tc(rate, c.source_in),
                tc(rate, c.duration),
                tc(rate, c.start)
            );
            for e in c.effects.iter().filter(|e| e.enabled) {
                let ps: Vec<String> = e
                    .params
                    .iter()
                    .map(|(k, v)| match v {
                        arc_core::model::Param::Static { value } => format!("{k}={}", num(*value)),
                        arc_core::model::Param::Curve { .. } => format!("{k}=\"curve\""),
                    })
                    .collect();
                l.push_str(&format!(" fx={}({})", e.effect, ps.join(",")));
            }
            if let Some(ti) = &c.title {
                l.push_str(&format!(" style={}", ti.style));
            }
            if !c.audio.fade_in.is_zero() || !c.audio.fade_out.is_zero() {
                l.push_str(&format!(
                    " fade=in:{},out:{}",
                    dur_text(c.audio.fade_in),
                    dur_text(c.audio.fade_out)
                ));
            }
            if c.speed != arc_core::Ratio::ONE {
                l.push_str(&format!(" speed={}x", num(c.speed.to_f64_lossy())));
            }
            if let Some(lb) = &c.label {
                l.push_str(&format!(" label={}", quote(lb)));
            }
            out.push_str(&l);
            out.push('\n');
        }
    }
    if !seq.markers.is_empty() {
        out.push_str("MARK\n");
        for m in seq.markers.iter() {
            let _ = writeln!(out, "  {} {}", tc(rate, m.at), quote(&m.label));
        }
    }
    out
}
