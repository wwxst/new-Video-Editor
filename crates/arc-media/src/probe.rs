//! Probe and import check in child processes (docs/04 §1.2 step 1, §2).

use crate::tools::{self, Exit};
use arc_core::model::{AssetKind, AudioInfo, Probe, VideoInfo};
use arc_core::time::{Ratio, Time};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome {
    /// Safe to decode in-process.
    Ok { kind: AssetKind, probe: Probe },
    /// The import check crashed or hung: never decode in-process (docs/04 §1.2 step 2).
    Quarantined {
        reason: String,
        kind: Option<AssetKind>,
        probe: Option<Probe>,
    },
    /// The file could not be read as media.
    Offline { reason: String },
}

fn rational(s: &str) -> Option<Ratio> {
    let (n, d) = s.split_once('/')?;
    let (n, d) = (n.trim().parse::<i64>().ok()?, d.trim().parse::<i64>().ok()?);
    (n > 0 && d > 0).then(|| Ratio::new(n, d).ok()).flatten()
}

/// Parse a decimal seconds string exactly ("60.0060" → 600060/10000).
fn decimal_time(s: &str) -> Option<Time> {
    let s = s.trim();
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let frac = &frac[..frac.len().min(9)];
    let den = 10i64.pow(frac.len() as u32);
    let n = int
        .parse::<i64>()
        .ok()?
        .checked_mul(den)?
        .checked_add(if frac.is_empty() {
            0
        } else {
            frac.parse::<i64>().ok()?
        })?;
    Time::new(n, den).ok()
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// Run `ffprobe` and map its JSON to the document model.
pub fn probe(path: &Path) -> Result<(AssetKind, Probe), String> {
    let mut cmd = Command::new(tools::ffprobe());
    cmd.args([
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ])
    .arg(path);
    let out = tools::run(cmd, PROBE_TIMEOUT).map_err(|e| format!("cannot run ffprobe: {e}"))?;
    match out.exit {
        Exit::Ok => {}
        other => return Err(format!("ffprobe {other:?}: {}", out.stderr.trim())),
    }
    let v: Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe output: {e}"))?;
    parse_probe(&v)
}

pub fn parse_probe(v: &Value) -> Result<(AssetKind, Probe), String> {
    let streams = v
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let format = v.get("format").cloned().unwrap_or(Value::Null);
    let vs = streams.iter().find(|s| {
        str_of(s, "codec_type") == Some("video")
            && s.get("disposition")
                .and_then(|d| d.get("attached_pic"))
                .and_then(Value::as_i64)
                != Some(1)
    });
    let as_ = streams
        .iter()
        .find(|s| str_of(s, "codec_type") == Some("audio"));
    if vs.is_none() && as_.is_none() {
        return Err("no audio or video stream".into());
    }
    let format_name = str_of(&format, "format_name").unwrap_or("");
    let video = vs.map(|s| {
        let avg = str_of(s, "avg_frame_rate").and_then(rational);
        let rfr = str_of(s, "r_frame_rate").and_then(rational);
        let rotation = s
            .get("side_data_list")
            .and_then(Value::as_array)
            .and_then(|l| {
                l.iter()
                    .find_map(|d| d.get("rotation").and_then(Value::as_f64))
            })
            .or_else(|| {
                s.get("tags")
                    .and_then(|t| str_of(t, "rotate"))
                    .and_then(|r| r.parse::<f64>().ok())
            })
            .unwrap_or(0.0);
        VideoInfo {
            width: s.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
            height: s.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
            frame_rate: avg.or(rfr).unwrap_or(Ratio::ONE),
            vfr: matches!((avg, rfr), (Some(a), Some(r)) if a != r),
            codec: str_of(s, "codec_name").unwrap_or("").into(),
            pix_fmt: str_of(s, "pix_fmt").unwrap_or("").into(),
            rotation: rotation.round() as i32,
        }
    });
    let audio = as_.map(|s| AudioInfo {
        sample_rate: str_of(s, "sample_rate")
            .and_then(|x| x.parse().ok())
            .unwrap_or(48_000),
        channels: s.get("channels").and_then(Value::as_u64).unwrap_or(2) as u32,
        codec: str_of(s, "codec_name").unwrap_or("").into(),
    });
    // exact duration: stream duration_ts × time_base when present, else the container's decimal
    let exact = |s: &Value| -> Option<Time> {
        let tb = str_of(s, "time_base").and_then(rational)?;
        let ts = s.get("duration_ts").and_then(Value::as_i64)?;
        Time::new(ts, 1).ok()?.checked_mul(tb).ok()
    };
    let duration = vs
        .and_then(exact)
        .or_else(|| as_.and_then(exact))
        .or_else(|| str_of(&format, "duration").and_then(decimal_time))
        .unwrap_or(Time::ZERO);
    let single_frame = vs
        .and_then(|s| str_of(s, "nb_frames"))
        .and_then(|n| n.parse::<u64>().ok())
        == Some(1);
    let kind = if video.is_some() {
        if format_name.contains("image2") || format_name.ends_with("_pipe") || single_frame {
            AssetKind::Image
        } else {
            AssetKind::Video
        }
    } else {
        AssetKind::Audio
    };
    Ok((
        kind,
        Probe {
            duration,
            video,
            audio,
        },
    ))
}

fn test_decode(path: &Path, seek: Option<Time>) -> std::io::Result<tools::Output> {
    let mut cmd = Command::new(tools::ffmpeg());
    cmd.args(["-hide_banner", "-v", "error", "-nostdin"]);
    if let Some(t) = seek {
        cmd.args(["-ss", &format!("{:.6}", t.to_f64_lossy())]);
    }
    cmd.arg("-i").arg(path).args(["-t", "1", "-f", "null", "-"]);
    tools::run(cmd, PROBE_TIMEOUT)
}

/// The subprocess import check (docs/04 §1.2 step 1): probe, then test-decode the start, the
/// middle, and the last second. Crashes and hangs quarantine; errors make the asset offline.
pub fn import_check(path: &Path) -> ImportOutcome {
    if !path.exists() {
        return ImportOutcome::Offline {
            reason: "file not found".into(),
        };
    }
    let (kind, probe) = match probe(path) {
        Ok(x) => x,
        Err(e) if e.contains("Crashed") || e.contains("TimedOut") => {
            return ImportOutcome::Quarantined {
                reason: e,
                kind: None,
                probe: None,
            };
        }
        Err(e) => return ImportOutcome::Offline { reason: e },
    };
    let d = probe.duration;
    let mut points = vec![None];
    if kind != AssetKind::Image && d > Time::secs(2) {
        points.push(d.checked_div(Ratio::new(2, 1).unwrap_or(Ratio::ONE)).ok());
        points.push(d.checked_sub(Time::secs(1)).ok());
    }
    for p in points {
        match test_decode(path, p) {
            Ok(o) => match o.exit {
                Exit::Ok => {}
                Exit::Failed(_) => {
                    return ImportOutcome::Offline {
                        reason: format!("test decode failed: {}", o.stderr.trim()),
                    };
                }
                Exit::Crashed(c) => {
                    return ImportOutcome::Quarantined {
                        reason: format!("test decode crashed (code {c})"),
                        kind: Some(kind),
                        probe: Some(probe),
                    };
                }
                Exit::TimedOut => {
                    return ImportOutcome::Quarantined {
                        reason: "test decode timed out".into(),
                        kind: Some(kind),
                        probe: Some(probe),
                    };
                }
            },
            Err(e) => {
                return ImportOutcome::Offline {
                    reason: format!("cannot run ffmpeg: {e}"),
                };
            }
        }
    }
    ImportOutcome::Ok { kind, probe }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_and_rational_parsing() {
        assert_eq!(decimal_time("60.000000"), Time::new(60, 1).ok());
        assert_eq!(decimal_time("1.5"), Time::new(3, 2).ok());
        assert_eq!(rational("30000/1001"), Ratio::new(30000, 1001).ok());
        assert_eq!(rational("0/0"), None);
    }

    #[test]
    fn maps_ffprobe_json() {
        let v = serde_json::json!({
            "streams": [
                {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"pix_fmt":"yuv420p",
                 "avg_frame_rate":"30000/1001","r_frame_rate":"30000/1001","time_base":"1/30000","duration_ts":300300,
                 "side_data_list":[{"rotation":-90}]},
                {"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2}
            ],
            "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10.010000"}
        });
        let (kind, p) = parse_probe(&v).expect("probe");
        assert_eq!(kind, AssetKind::Video);
        assert_eq!(p.duration, Time::new(1001, 100).expect("t"));
        let vi = p.video.expect("video");
        assert_eq!(vi.rotation, -90);
        assert!(!vi.vfr);
        assert_eq!(p.audio.expect("audio").sample_rate, 48_000);
    }
}
