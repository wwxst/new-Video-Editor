//! FFmpeg differential corpus harness (spec §8.2–8.3).
//!
//! Generates (or reuses) the generated-media corpus, decodes every entry through the shim, and
//! records per entry: BLAKE3 of every decoded RGBA frame (sequential and random access), BLAKE3 of
//! the decoded audio (one long read and the mixer's 4096+1 block pattern), the parsed `Probe`, the
//! import-check outcome and the media file's own BLAKE3.
//!
//! Step 1 runs with `ARCLINE_DIFF_RECORD=<baseline.json>` against 8.1.3. Step 3 runs the same
//! binary against 9.1 with `ARCLINE_CORPUS_DIR` pointing at the step-1 corpus; corpus hashes are
//! verified first, so any difference is a decoder difference and not a different input.
//!
//! Environment:
//! - `ARCLINE_CORPUS_DIR`   corpus directory (required to run; without it the test is skipped)
//! - `ARCLINE_CORPUS_REGEN` `1` regenerates media that already exists
//! - `ARCLINE_DIFF_RECORD`  write the result as the new baseline instead of comparing
//! - `ARCLINE_DIFF_BASELINE` baseline path (default `tests/baseline/ffmpeg-8.1.3.json`)
//! - `ARCLINE_DIFF_ALLOW`   optional JSON allow-list of explained differences
//! - `ARCLINE_DIFF_REPORT`  write the comparison report JSON here

#![allow(dead_code)]

use arc_media::fixtures::counter_frame;
use arc_media::probe::{ImportOutcome, import_check, probe};
use arc_media::{AudioDecoder, VideoDecoder};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const W: u32 = 320;
const H: u32 = 180;
const FRAMES: u32 = 60;
const RATE: u32 = 30;
const AUDIO_RATE: u32 = 48_000;

pub struct Entry {
    pub name: String,
    pub file: PathBuf,
    pub note: String,
}

fn ffmpeg() -> PathBuf {
    arc_media::tools::ffmpeg()
}

fn run_ffmpeg(args: &[&str]) -> Result<(), String> {
    let out = Command::new(ffmpeg())
        .args(["-hide_banner", "-v", "error", "-y"])
        .args(args)
        .output()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ffmpeg {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

/// Encode `FRAMES` counter frames (raw RGBA on stdin) with the given encoder arguments.
fn encode_counter(path: &Path, extra: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(ffmpeg());
    cmd.args([
        "-hide_banner",
        "-v",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        "320x180",
        "-r",
        "30",
        "-i",
        "pipe:0",
    ])
    .args(extra)
    .arg(path)
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn ffmpeg: {e}"))?;
    {
        let stdin = child.stdin.as_mut().ok_or("ffmpeg stdin")?;
        for i in 0..FRAMES {
            stdin
                .write_all(&counter_frame(i, W, H))
                .map_err(|e| format!("write frame: {e}"))?;
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "encode {path:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

fn copy_fixture(name: &str, dst: &Path) -> Result<(), String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::copy(&src, dst)
        .map(|_| ())
        .map_err(|e| format!("copy {src:?}: {e}"))
}

fn add(
    entries: &mut Vec<Entry>,
    root: &Path,
    name: &str,
    note: &str,
    regen: bool,
    make: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let path = root.join(name);
    if regen || !path.exists() {
        make(&path)?;
    }
    entries.push(Entry {
        name: name.into(),
        file: path,
        note: note.into(),
    });
    Ok(())
}

fn sine(rate: u32, freq: u32) -> String {
    format!("sine=frequency={freq}:sample_rate={rate}:duration=2")
}

/// Generate the whole corpus into `root` (existing files are reused unless `regen`).
pub fn generate(root: &Path, regen: bool) -> Result<Vec<Entry>, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("create {}: {e}", root.display()))?;
    let mut e = Vec::new();

    // Lossless counter video: frame-exact sequential and random access.
    for (name, rate) in [
        ("counter_ffv1_2997.mkv", arc_core::FrameRate::Fps29_97),
        ("counter_ffv1_25.mkv", arc_core::FrameRate::Fps25),
        ("counter_ffv1_30.mkv", arc_core::FrameRate::Fps30),
    ] {
        add(
            &mut e,
            root,
            name,
            "counter FFV1/MKV; frame-exact",
            regen,
            |p| {
                arc_media::fixtures::write_counter_video(p, FRAMES, rate.rate(), W, H, false)
                    .map(|_| ())
                    .map_err(|x| x.to_string())
            },
        )?;
    }
    // Long-GOP / B-frame video in several containers (accuracy defects F-06/07/08 are out of
    // scope; the entries exist so the differential sees the same decode behaviour).
    add(
        &mut e,
        root,
        "counter_mpeg2_bframes.mkv",
        "counter MPEG-2/MKV open GOP, B-frames",
        regen,
        |p| encode_counter(p, &["-c:v", "mpeg2video", "-q:v", "2", "-bf", "2", "-g", "15"]),
    )?;
    add(
        &mut e,
        root,
        "counter_mpeg2_bframes.ts",
        "counter MPEG-2/MPEG-TS, non-zero start; random access is F-08 (expected)",
        regen,
        |p| {
            encode_counter(
                p,
                &[
                    "-c:v",
                    "mpeg2video",
                    "-q:v",
                    "2",
                    "-bf",
                    "2",
                    "-g",
                    "15",
                    "-f",
                    "mpegts",
                    "-output_ts_offset",
                    "1.433",
                ],
            )
        },
    )?;
    add(
        &mut e,
        root,
        "counter_mpeg4_bframes.mp4",
        "counter MPEG-4 Part 2/MP4 with B-frames; random access is F-08 (expected)",
        regen,
        |p| encode_counter(p, &["-c:v", "mpeg4", "-q:v", "2", "-bf", "2", "-g", "15"]),
    )?;
    add(
        &mut e,
        root,
        "counter_h264_bframes.mp4",
        "H.264/MP4 hardware-encoded fixture with B-frames (committed, provenance in tests/fixtures)",
        false,
        |p| copy_fixture("counter_h264_qsv.mp4", p),
    )?;
    // Stills.
    for (name, extra) in [
        ("still.png", vec!["-frames:v", "1"]),
        ("still.jpg", vec!["-frames:v", "1", "-q:v", "2"]),
    ] {
        add(&mut e, root, name, "still image; exact pixels", regen, |p| {
            let pstr = p.to_string_lossy().to_string();
            let mut cmd = vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=1",
                "-pix_fmt",
                "rgb24",
            ];
            cmd.extend(extra.iter().copied());
            cmd.push(&pstr);
            run_ffmpeg(&cmd)
        })?;
    }
    // Mid-stream resolution change (audit F-02, experiment J).
    add(
        &mut e,
        root,
        "change_resolution.mkv",
        "MPEG-2 320x180 -> 640x360 in one MKV (F-02 fixed)",
        regen,
        |p| {
            let a = p.with_file_name("j_a.mkv");
            let b = p.with_file_name("j_b.mkv");
            run_ffmpeg(&[
                "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=2", "-c:v",
                "mpeg2video", "-q:v", "2", a.to_str().ok_or("utf8")?,
            ])?;
            run_ffmpeg(&[
                "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=2", "-c:v",
                "mpeg2video", "-q:v", "2", b.to_str().ok_or("utf8")?,
            ])?;
            let list = p.with_file_name("j_list.txt");
            std::fs::write(&list, "file 'j_a.mkv'\nfile 'j_b.mkv'\n").map_err(|x| x.to_string())?;
            run_ffmpeg(&[
                "-f", "concat", "-safe", "0", "-i", list.to_str().ok_or("utf8")?, "-c", "copy",
                "-map", "0", p.to_str().ok_or("utf8")?,
            ])
        },
    )?;

    // Audio: lossless exact, lossy per codec notes.
    add(
        &mut e,
        root,
        "audio_flac48.flac",
        "FLAC 48 kHz; exact",
        regen,
        |p| run_ffmpeg(&["-f", "lavfi", "-i", &sine(48000, 1000), "-c:a", "flac", p.to_str().ok_or("utf8")?]),
    )?;
    add(
        &mut e,
        root,
        "audio_pcm48.wav",
        "PCM s16 48 kHz; exact",
        regen,
        |p| run_ffmpeg(&["-f", "lavfi", "-i", &sine(48000, 1000), "-c:a", "pcm_s16le", p.to_str().ok_or("utf8")?]),
    )?;
    add(
        &mut e,
        root,
        "audio_flac44.flac",
        "FLAC 44.1 kHz resampled to 48 kHz",
        regen,
        |p| run_ffmpeg(&["-f", "lavfi", "-i", &sine(44100, 1000), "-c:a", "flac", p.to_str().ok_or("utf8")?]),
    )?;
    add(
        &mut e,
        root,
        "audio_mp3_48.mp3",
        "MP3 48 kHz fixture (committed; libmp3lame, provenance in tests/fixtures)",
        false,
        |p| copy_fixture("sine_48k.mp3", p),
    )?;
    add(
        &mut e,
        root,
        "audio_opus.ogg",
        "Opus/Ogg native encoder; F-07 alignment note",
        regen,
        |p| {
            run_ffmpeg(&[
                "-f", "lavfi", "-i", &sine(48000, 1000), "-c:a", "opus", "-strict",
                "experimental", "-b:a", "96k", p.to_str().ok_or("utf8")?,
            ])
        },
    )?;
    add(
        &mut e,
        root,
        "audio_opus.mkv",
        "Opus/MKV native encoder; F-07 1 ms timestamp note",
        regen,
        |p| {
            run_ffmpeg(&[
                "-f", "lavfi", "-i", &sine(48000, 1000), "-c:a", "opus", "-strict",
                "experimental", "-b:a", "96k", p.to_str().ok_or("utf8")?,
            ])
        },
    )?;
    add(
        &mut e,
        root,
        "audio_aac_sine.m4a",
        "AAC 48 kHz sine; F-25 decode-history dependence",
        regen,
        |p| {
            run_ffmpeg(&[
                "-f", "lavfi", "-i", &sine(48000, 1000), "-c:a", "aac", "-b:a", "128k",
                p.to_str().ok_or("utf8")?,
            ])
        },
    )?;
    add(
        &mut e,
        root,
        "audio_aac_noise.m4a",
        "AAC 48 kHz white noise (seeded); F-25",
        regen,
        |p| {
            run_ffmpeg(&[
                "-f", "lavfi",
                "-i",
                "anoisesrc=color=white:sample_rate=48000:duration=2:seed=42",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                p.to_str().ok_or("utf8")?,
            ])
        },
    )?;
    // Mid-stream audio channel change (audit F-03, experiment K).
    add(
        &mut e,
        root,
        "change_channels.mp2",
        "MP2 stereo 2 s then mono 2 s (F-03 fixed)",
        regen,
        |p| {
            let stereo = p.with_file_name("k_stereo.mp2");
            let mono = p.with_file_name("k_mono.mp2");
            run_ffmpeg(&[
                "-f", "lavfi", "-i", &sine(48000, 440), "-ac", "2", "-c:a", "mp2", "-b:a",
                "192k", stereo.to_str().ok_or("utf8")?,
            ])?;
            run_ffmpeg(&[
                "-f", "lavfi", "-i", &sine(48000, 880), "-ac", "1", "-c:a", "mp2", "-b:a",
                "96k", mono.to_str().ok_or("utf8")?,
            ])?;
            let mut bytes = std::fs::read(&stereo).map_err(|x| x.to_string())?;
            bytes.extend_from_slice(&std::fs::read(&mono).map_err(|x| x.to_string())?);
            std::fs::write(p, bytes).map_err(|x| x.to_string())
        },
    )?;

    // Broken / non-media files: import check must classify them and the shim must not be reached.
    add(&mut e, root, "broken_truncated.mkv", "truncated container", regen, |p| {
        let src = root.join("counter_ffv1_30.mkv");
        let bytes = std::fs::read(&src).map_err(|x| x.to_string())?;
        std::fs::write(p, &bytes[..bytes.len().min(4096)]).map_err(|x| x.to_string())
    })?;
    add(&mut e, root, "broken_zero.mkv", "zero-byte file", regen, |p| {
        std::fs::write(p, b"").map_err(|x| x.to_string())
    })?;
    add(&mut e, root, "broken_text.mp4", "text renamed .mp4", regen, |p| {
        std::fs::write(p, b"this is not a video\n").map_err(|x| x.to_string())
    })?;

    Ok(e)
}

// ---- hashing ------------------------------------------------------------------------------------

fn file_blake3(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    blake3::hash(&bytes).to_hex().to_string()
}

fn f32_hash(samples: &[f32]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(&(samples.len() as u64).to_le_bytes());
    for s in samples {
        h.update(&s.to_bits().to_le_bytes());
    }
    h.finalize().to_hex().to_string()
}

fn video_hash(v: &[u8]) -> String {
    blake3::hash(v).to_hex().to_string()
}

fn frame_time(rate: arc_core::Ratio, i: i64) -> arc_core::Time {
    let frame = arc_core::Ratio::new(rate.den() as i64, rate.num() as i64);
    match (arc_core::Time::new(i, 1), frame) {
        (Ok(t), Ok(r)) => t.checked_mul(r).unwrap_or(arc_core::Time::ZERO),
        _ => arc_core::Time::ZERO,
    }
}

// ---- result collection --------------------------------------------------------------------------

fn collect_video(path: &Path, note: &str) -> Value {
    let mut out = json!({"note": note});
    let mut d = match VideoDecoder::open(path) {
        Ok(d) => d,
        Err(_) => return Value::Null,
    };
    let info = d.info();
    out["stream"] = json!({
        "width": info.width, "height": info.height, "codec": info.codec,
        "format": info.format, "vfr": info.vfr,
        "frame_rate": info.frame_rate.map(|r| r.to_string()),
        "time_base": info.time_base.to_string(),
    });
    let rate = info.frame_rate.unwrap_or(arc_core::Ratio::new(30, 1).expect("rate"));
    let mut sequential = Vec::new();
    for i in 0..120i64 {
        let t = frame_time(rate, i);
        match d.frame_at(t) {
            Ok(f) => sequential.push(json!({
                "index": i,
                "width": f.width, "height": f.height,
                "time": f.time.to_string(),
                "hash": video_hash(f.rgba),
            })),
            Err(arc_media::MediaError::EndOfStream) => break,
            Err(e) => {
                sequential.push(json!({"index": i, "error": e.to_string()}));
                break;
            }
        }
    }
    let n = sequential.len() as i64;
    let mut random = Vec::new();
    if n > 0 {
        for i in [n - 1, 0, n / 2, 1, (n / 3).max(1), n - 2, 2] {
            if i < 0 || i >= n {
                continue;
            }
            let t = frame_time(rate, i);
            match d.frame_at(t) {
                Ok(f) => random.push(json!({"index": i, "hash": video_hash(f.rgba)})),
                Err(e) => random.push(json!({"index": i, "error": e.to_string()})),
            }
        }
    }
    out["sequential"] = Value::Array(sequential);
    out["random"] = Value::Array(random);
    out
}

fn collect_audio(path: &Path, duration_samples: i64) -> Value {
    let mut a = match AudioDecoder::open(path, AUDIO_RATE) {
        Ok(a) => a,
        Err(_) => return Value::Null,
    };
    let total = duration_samples.clamp(1, 4 * AUDIO_RATE as i64) as usize;
    let mut long = vec![0f32; total * 2];
    let got = a.read(0, &mut long).unwrap_or(0);
    let mut out = json!({
        "rate": AUDIO_RATE,
        "long": {"got": got, "hash": f32_hash(&long[..got * 2])},
    });
    let mut blocks = Vec::new();
    let mut all = Vec::new();
    let mut start = 0i64;
    while start < total as i64 && blocks.len() < 64 {
        let mut buf = vec![0f32; 2 * 4097];
        let got = a.read(start, &mut buf).unwrap_or(0);
        blocks.push(json!({"start": start, "got": got, "hash": f32_hash(&buf)}));
        all.extend_from_slice(&buf);
        if got == 0 {
            break;
        }
        start += 4096;
    }
    out["blocks"] = json!({"list": blocks, "hash": f32_hash(&all)});
    out
}

fn collect_entry(entry: &Entry) -> Value {
    let mut out = json!({
        "note": entry.note,
        "file_blake3": file_blake3(&entry.file),
    });
    let import = import_check(&entry.file);
    let imports_ok = matches!(import, ImportOutcome::Ok { .. });
    out["import"] = match &import {
        ImportOutcome::Ok { kind, .. } => json!({"status": "ok", "kind": kind}),
        ImportOutcome::Quarantined { kind, .. } => json!({"status": "quarantined", "kind": kind}),
        ImportOutcome::Offline { .. } => json!({"status": "offline"}),
    };
    match probe(&entry.file) {
        Ok((kind, p)) => {
            out["asset_kind"] = serde_json::to_value(kind).unwrap_or(Value::Null);
            out["probe"] = serde_json::to_value(&p).unwrap_or(Value::Null);
            // Only media the product would decode in-process is decoded here.
            if imports_ok {
                let duration_samples = arc_core::time::sample_index(
                    p.duration,
                    AUDIO_RATE as u32,
                    arc_core::time::Round::Floor,
                )
                .unwrap_or(0);
                if VideoDecoder::open(&entry.file).is_ok() {
                    out["video"] = collect_video(&entry.file, &entry.note);
                }
                if p.audio.is_some() {
                    out["audio"] = collect_audio(&entry.file, duration_samples);
                }
            } else {
                out["video"] = Value::Null;
                out["audio"] = Value::Null;
            }
        }
        Err(_) => {
            out["asset_kind"] = Value::Null;
            out["probe"] = Value::Null;
            out["video"] = Value::Null;
            out["audio"] = Value::Null;
        }
    }
    out
}

// ---- comparison ---------------------------------------------------------------------------------

fn flatten(prefix: &str, v: &Value, out: &mut BTreeMap<String, Value>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                flatten(&format!("{prefix}.{k}"), val, out);
            }
        }
        _ => {
            out.insert(prefix.to_string(), v.clone());
        }
    }
}

fn allowed(
    allow: &Value,
    path: &str,
) -> Option<String> {
    let list = allow.get("allowed")?.as_array()?;
    for a in list {
        let entry = a.get("entry")?.as_str()?;
        let field = a.get("field")?.as_str()?;
        let prefix = format!("entries.{entry}.{field}");
        if path == prefix || path.starts_with(&format!("{prefix}.")) {
            return a.get("reason").and_then(Value::as_str).map(str::to_string);
        }
    }
    None
}

fn compare(baseline: &Value, current: &Value, allow: &Value) -> Vec<Value> {
    let mut diffs = Vec::new();
    let empty = serde_json::Map::new();
    let base_entries = baseline
        .get("entries")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let cur_entries = current
        .get("entries")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for name in base_entries.keys() {
        if !cur_entries.contains_key(name) {
            diffs.push(json!({"path": format!("entries.{name}"), "baseline": "present", "current": "missing"}));
        }
    }
    for name in cur_entries.keys() {
        if !base_entries.contains_key(name) {
            diffs.push(json!({"path": format!("entries.{name}"), "baseline": "missing", "current": "present"}));
        }
    }
    for (name, b) in base_entries {
        let Some(c) = cur_entries.get(name) else {
            continue;
        };
        // The corpus file itself must be identical, otherwise the comparison is meaningless.
        let bh = b.get("file_blake3").and_then(Value::as_str).unwrap_or("");
        let ch = c.get("file_blake3").and_then(Value::as_str).unwrap_or("");
        if bh != ch {
            diffs.push(json!({
                "path": format!("entries.{name}.file_blake3"),
                "baseline": bh, "current": ch,
                "reason": "corpus media changed; regenerate the corpus from the step-1 baseline",
            }));
        }
        let mut bf = BTreeMap::new();
        let mut cf = BTreeMap::new();
        flatten("", b, &mut bf);
        flatten("", c, &mut cf);
        for (path, bv) in &bf {
            if path.ends_with("file_blake3") || path.ends_with(".note") {
                continue;
            }
            let full = format!("entries.{name}{path}");
            match cf.get(path) {
                Some(cv) if cv == bv => {}
                other => {
                    let reason = allowed(allow, &full);
                    diffs.push(json!({
                        "path": full,
                        "baseline": bv,
                        "current": other.cloned().unwrap_or(Value::Null),
                        "reason": reason,
                    }));
                }
            }
        }
        for (path, cv) in &cf {
            if path.ends_with("file_blake3") || path.ends_with(".note") {
                continue;
            }
            if !bf.contains_key(path) {
                let full = format!("entries.{name}{path}");
                diffs.push(json!({
                    "path": full,
                    "baseline": Value::Null,
                    "current": cv,
                    "reason": allowed(allow, &full),
                }));
            }
        }
    }
    diffs
}

fn ffmpeg_version() -> String {
    let out = Command::new(ffmpeg())
        .args(["-hide_banner", "-version"])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .to_string(),
        Err(_) => String::new(),
    }
}

/// Entry point for `tests/differential.rs`.
pub fn run() {
    let Ok(dir) = std::env::var("ARCLINE_CORPUS_DIR") else {
        eprintln!("differential: ARCLINE_CORPUS_DIR not set; skipping the corpus comparison");
        return;
    };
    let root = PathBuf::from(dir);
    let regen = std::env::var("ARCLINE_CORPUS_REGEN").as_deref() == Ok("1");
    let entries = generate(&root, regen).unwrap_or_else(|e| panic!("corpus: {e}"));

    let mut out_entries = serde_json::Map::new();
    for entry in &entries {
        eprintln!("differential: {}", entry.name);
        out_entries.insert(entry.name.clone(), collect_entry(entry));
    }
    let current = json!({
        "generator": "crates/arc-media/tests/differential.rs",
        "ffmpeg": ffmpeg_version(),
        "entries": out_entries,
    });

    if let Ok(record) = std::env::var("ARCLINE_DIFF_RECORD") {
        let text = serde_json::to_vec_pretty(&current).expect("json");
        std::fs::write(&record, text).unwrap_or_else(|e| panic!("write {record}: {e}"));
        eprintln!("differential: baseline recorded in {record}");
        return;
    }

    let baseline_path = std::env::var("ARCLINE_DIFF_BASELINE").unwrap_or_else(|_| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/baseline/ffmpeg-8.1.3.json")
            .to_string_lossy()
            .to_string()
    });
    let text = std::fs::read_to_string(&baseline_path)
        .unwrap_or_else(|e| panic!("read baseline {baseline_path}: {e}"));
    let baseline: Value = serde_json::from_str(&text).expect("baseline json");
    let allow: Value = match std::env::var("ARCLINE_DIFF_ALLOW") {
        Ok(p) => serde_json::from_str(&std::fs::read_to_string(&p).expect("allow read"))
            .expect("allow json"),
        Err(_) => json!({"allowed": []}),
    };
    let diffs = compare(&baseline, &current, &allow);
    let unexplained: Vec<&Value> = diffs
        .iter()
        .filter(|d| d.get("reason").and_then(Value::as_str).is_none())
        .collect();
    let report = json!({
        "corpus_dir": root.to_string_lossy(),
        "baseline_ffmpeg": baseline.get("ffmpeg"),
        "current_ffmpeg": current.get("ffmpeg"),
        "differences": diffs,
        "unexplained": unexplained.len(),
    });
    let report_text = serde_json::to_vec_pretty(&report).expect("report json");
    if let Ok(path) = std::env::var("ARCLINE_DIFF_REPORT") {
        std::fs::write(&path, &report_text).unwrap_or_else(|e| panic!("write report: {e}"));
    } else {
        eprintln!("{}", String::from_utf8_lossy(&report_text));
    }
    assert!(
        unexplained.is_empty(),
        "{} unexplained differential difference(s); see the report",
        unexplained.len()
    );
}
