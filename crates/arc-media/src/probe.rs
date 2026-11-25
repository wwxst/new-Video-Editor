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

/// Run `ffprobe` and map its JSON to the document model. Inputs are passed as `file:` URLs with
/// `protocol_whitelist=file` (spec §4.6), never as bare protocol strings.
pub fn probe(path: &Path) -> Result<(AssetKind, Probe), String> {
    let url = crate::paths::file_url(path).ok_or_else(|| "not a local media file".to_string())?;
    let ffprobe = tools::ffprobe_checked()?;
    let mut cmd = Command::new(ffprobe);
    cmd.args([
        "-v",
        "error",
        "-of",
        "json",
        "-show_format",
        "-show_streams",
        "-protocol_whitelist",
        "file",
        "-i",
    ])
    .arg(&url);
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

fn test_decode(path: &Path, seek: Option<Time>) -> Result<tools::Output, String> {
    let url = crate::paths::file_url(path).ok_or_else(|| "not a local media file".to_string())?;
    let ffmpeg = tools::ffmpeg_checked()?;
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-v", "error", "-nostdin"]);
    if let Some(t) = seek {
        cmd.args(["-ss", &format!("{:.6}", t.to_f64_lossy())]);
    }
    // `-progress` reports `frame=` on stdout: a decode that produces no frames means the file is
    // truncated or unreadable at that point, which the exit code alone does not reveal.
    cmd.args(["-protocol_whitelist", "file", "-i"])
        .arg(&url)
        .args(["-t", "1", "-nostats", "-progress", "pipe:1", "-f", "null", "-"]);
    tools::run(cmd, PROBE_TIMEOUT).map_err(|e| e.to_string())
}

/// Last value of a key in ffmpeg's `-progress` output (`key=value` lines).
fn progress_value(o: &tools::Output, key: &str) -> Option<String> {
    let text = String::from_utf8_lossy(&o.stdout);
    let mut last = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix(key)
            && let Some(v) = v.strip_prefix('=')
        {
            last = Some(v.trim().to_string());
        }
    }
    last
}

/// Frames the test decode produced, from ffmpeg's `-progress` output.
fn decoded_frames(o: &tools::Output) -> Option<u64> {
    progress_value(o, "frame").and_then(|v| v.parse().ok())
}

/// Decoded output time in microseconds, from ffmpeg's `-progress` output.
fn decoded_out_time_us(o: &tools::Output) -> Option<i64> {
    progress_value(o, "out_time_us").and_then(|v| v.parse().ok())
}

/// Why a successful test decode still shows the media is shorter than the probe promised.
///
/// Video is judged by decoded frames: a truncated container yields zero frames while ffmpeg still
/// exits 0. Audio-only media has no frames to count, so it is judged by the decoded output time
/// against the window that was requested (the probe's remaining duration, at most one second); this
/// keeps a short but valid audio file from being classified as truncated. Images are not judged.
fn decode_shortfall(
    kind: AssetKind,
    seek: Option<Time>,
    duration: Time,
    frames: Option<u64>,
    out_time_us: Option<i64>,
) -> Option<&'static str> {
    if kind == AssetKind::Image {
        return None;
    }
    if kind == AssetKind::Video {
        return (frames == Some(0))
            .then_some("test decode produced no video frames (truncated or unreadable media)");
    }
    let seek_us = seek
        .map(|t| (t.to_f64_lossy() * 1_000_000.0) as i64)
        .unwrap_or(0);
    let duration_us = (duration.to_f64_lossy() * 1_000_000.0) as i64;
    let expected_us = (duration_us - seek_us).clamp(0, 1_000_000);
    if expected_us < 100_000 {
        return None; // shorter than 0.1 s: not enough signal to judge
    }
    match out_time_us {
        Some(t) if t * 2 >= expected_us => None,
        _ => Some(
            "test decode produced no audio output for the expected window (truncated or unreadable media)",
        ),
    }
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
                Exit::Ok => {
                    if let Some(reason) =
                        decode_shortfall(kind, p, d, decoded_frames(&o), decoded_out_time_us(&o))
                    {
                        return ImportOutcome::Offline {
                            reason: reason.into(),
                        };
                    }
                }
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
                return ImportOutcome::Offline { reason: e };
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

    /// P3-1: the "short decode" gate is video-only for frames; audio-only media is judged by its
    /// decoded output time against the requested window, and images are never judged.
    #[test]
    fn decode_shortfall_gates_video_frames_and_audio_output_time() {
        let secs = Time::secs;
        // Video: zero frames means truncated; any frame is accepted.
        assert!(decode_shortfall(AssetKind::Video, None, secs(4), Some(0), None).is_some());
        assert!(decode_shortfall(AssetKind::Video, None, secs(4), Some(30), None).is_none());
        // Audio: no output at all, or much less than the promised second, is truncated.
        assert!(decode_shortfall(AssetKind::Audio, None, secs(4), None, None).is_some());
        assert!(decode_shortfall(AssetKind::Audio, None, secs(4), None, Some(1_000_000)).is_none());
        assert!(decode_shortfall(AssetKind::Audio, None, secs(4), None, Some(200_000)).is_some());
        // A short but valid audio file is judged against its own duration, not a full second.
        let half = Time::new(1, 2).expect("t");
        assert!(decode_shortfall(AssetKind::Audio, None, half, None, Some(500_000)).is_none());
        // Images are not judged (a still is a single frame; no duration).
        assert!(decode_shortfall(AssetKind::Image, None, Time::ZERO, Some(0), None).is_none());
    }
}
