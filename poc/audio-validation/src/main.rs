//! Arcline audio validation probe.
//!
//! ```text
//! audio-probe devices
//! audio-probe play --file narration.wav --seconds 5 --mode direct|ring
//! audio-probe mixbench --seconds 30 --tracks 4 --file narration.wav
//! audio-probe cache --file narration.wav --repeat 3
//! audio-probe parity --file narration.wav --seconds 8
//! ```

mod pcm;
mod playback;

use arc_audio::{hash_samples, Mixer, SampleSource};
use arc_core::engine::{ApplyRequest, Engine};
use arc_core::model::{Project, Resolution};
use arc_core::{Command, FrameRate, StaticCatalog};
use arc_core::time::Time;
use pcm::{Cache, Key, PcmSource};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

fn args() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        if k.starts_with("--") {
            map.insert(k.trim_start_matches("--").to_string(), it.next().unwrap_or_default());
        } else {
            map.insert("_cmd".to_string(), k);
        }
    }
    map
}

fn main() {
    let a = args();
    let cmd = a.get("_cmd").cloned().unwrap_or_else(|| "devices".into());
    let result = match cmd.as_str() {
        "devices" => playback::devices(),
        "play" => cmd_play(&a),
        "mixbench" => cmd_mixbench(&a),
        "cache" => cmd_cache(&a),
        "parity" => cmd_parity(&a),
        other => Err(format!("unknown command {other}")),
    };
    match result {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
        Err(e) => {
            println!("{}", json!({"error": e}));
            std::process::exit(2);
        }
    }
}

/// Build a project with `tracks` audio tracks, each holding `per_track` clips, and (optionally) a
/// source duration large enough for the clips.
fn build_project(tracks: usize, per_track: usize, clip_frames: i64, source_secs: i64) -> Project {
    let p = Project::new(
        "audio-poc",
        FrameRate::Fps30,
        Resolution { w: 1920, h: 1080 },
    );
    let cat = StaticCatalog::default();
    let mut engine = Engine::new(p, 0, Arc::new(cat)).expect("engine");
    let mut commands = Vec::new();
    for t in 0..tracks {
        commands.push(json!({"op":"track.add","id":format!("tr_a{t}"),"kind":"audio"}));
    }
    commands.push(json!({
        "op":"asset.import","id":"as_src","path":"synthetic.wav","kind":"audio",
        "probe":{"duration":{"num":source_secs,"den":1}}
    }));
    let cmds: Vec<Command> = serde_json::from_value(Value::Array(commands)).expect("commands");
    engine
        .apply(ApplyRequest {
            commands: cmds,
            ..Default::default()
        })
        .expect("tracks");
    let fps = 30i64;
    let mut commands = Vec::new();
    for t in 0..tracks {
        for c in 0..per_track {
            let start = c as i64 * (clip_frames + 30);
            let start_tc = frames_tc(start, fps);
            let dur_tc = frames_tc(clip_frames, fps);
            commands.push(json!({
                "op":"timeline.add_clip",
                "id":format!("cl_{t}_{c}"),
                "track":format!("tr_a{t}"),
                "asset":"as_src",
                "start":start_tc,
                "duration":dur_tc,
                "source_in":"00:00:00:00"
            }));
        }
    }
    let cmds: Vec<Command> = serde_json::from_value(Value::Array(commands)).expect("clips");
    engine
        .apply(ApplyRequest {
            commands: cmds,
            ..Default::default()
        })
        .expect("clips");
    (*engine.snapshot()).clone()
}

fn frames_tc(frames: i64, fps: i64) -> String {
    let f = frames % fps;
    let total_s = frames / fps;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02}:{f:02}")
}

/// A synthetic sample source: cheap, deterministic, position-revealing.
struct Sine {
    rate: u32,
}

impl SampleSource for Sine {
    fn read(&mut self, _asset: &arc_core::ids::AssetId, start: i64, dst: &mut [f32]) -> Result<(), String> {
        for (k, fr) in dst.chunks_mut(2).enumerate() {
            let t = (start + k as i64) as f64 / self.rate as f64;
            let v = (t * 440.0 * std::f64::consts::TAU).sin() as f32 * 0.25;
            fr[0] = v;
            fr[1] = v;
        }
        Ok(())
    }
}

fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("arcline-audio-poc")
}

fn cmd_play(a: &std::collections::HashMap<String, String>) -> Result<Value, String> {
    let file = a.get("file").cloned();
    let seconds: f64 = a.get("seconds").and_then(|v| v.parse().ok()).unwrap_or(5.0);
    let ring = a.get("mode").map(|m| m.as_str()) == Some("ring");
    let project = build_project(2, 2, 30 * 30, 120);
    let mixer = Mixer::new(&project);
    let source: Box<dyn SampleSource + Send> = match &file {
        Some(f) => {
            let key = Key::of(Path::new(f), 0, (seconds * pcm::RATE as f64 * 4.0) as u64, (1, 1), "");
            let mut cache = Cache::new(cache_dir());
            let pcm = cache.get(&key)?;
            Box::new(PcmSource::new(pcm))
        }
        None => Box::new(Sine { rate: 48_000 }),
    };
    let started = Instant::now();
    let mut out = playback::play(mixer, source, seconds, ring)?;
    out["wall_s"] = json!(started.elapsed().as_secs_f64());
    Ok(out)
}

fn cmd_mixbench(a: &std::collections::HashMap<String, String>) -> Result<Value, String> {
    let seconds: i64 = a.get("seconds").and_then(|v| v.parse().ok()).unwrap_or(30);
    let tracks: usize = a.get("tracks").and_then(|v| v.parse().ok()).unwrap_or(4);
    let per_track: usize = a.get("clips").and_then(|v| v.parse().ok()).unwrap_or(2);
    let file = a.get("file").cloned();
    let project = build_project(tracks, per_track, 30 * 20, seconds + 120);
    let mut mixer = Mixer::new(&project);
    let frames = (seconds * 48_000) as usize;
    let mut out = vec![0f32; frames * 2];
    let pcm_source: Option<Arc<pcm::Pcm>> = match &file {
        Some(f) => {
            let key = Key::of(Path::new(f), 0, (seconds as u64 + 30) * pcm::RATE as u64, (1, 1), "");
            Some(Cache::new(cache_dir()).get(&key)?)
        }
        None => None,
    };
    let started = Instant::now();
    let block = 480usize;
    let mut pos = 0i64;
    match &pcm_source {
        Some(p) => {
            let mut src = PcmSource::new(Arc::clone(p));
            for chunk in out.chunks_mut(block * 2) {
                mixer.process(&mut src, pos, chunk)?;
                pos += (chunk.len() / 2) as i64;
            }
        }
        None => {
            let mut src = Sine { rate: 48_000 };
            for chunk in out.chunks_mut(block * 2) {
                mixer.process(&mut src, pos, chunk)?;
                pos += (chunk.len() / 2) as i64;
            }
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "tracks": tracks,
        "clips": tracks * per_track,
        "seconds": seconds,
        "mix_ms": ms,
        "realtime_factor": (seconds as f64 * 1000.0) / ms,
        "source": file.unwrap_or_else(|| "synthetic".into()),
        "samples": out.len() / 2,
        "checksum": hash_samples(&out),
    }))
}

fn cmd_cache(a: &std::collections::HashMap<String, String>) -> Result<Value, String> {
    let file = a.get("file").cloned().ok_or("--file required")?;
    let repeat: usize = a.get("repeat").and_then(|v| v.parse().ok()).unwrap_or(3);
    let frames: u64 = a.get("frames").and_then(|v| v.parse().ok()).unwrap_or(48_000 * 10);
    let mut cache = Cache::new(cache_dir());
    let key = Key::of(Path::new(&file), 0, frames, (1, 1), "");
    let mut runs = Vec::new();
    for _ in 0..repeat {
        let t = Instant::now();
        let pcm = cache.get(&key)?;
        runs.push(json!({
            "ms": t.elapsed().as_secs_f64() * 1000.0,
            "frames": pcm.frames(),
            "bytes": pcm.bytes(),
        }));
    }
    // A parameter change must not reuse the old file: a different span is a different key.
    let other = Key::of(Path::new(&file), 1000, frames, (1, 1), "");
    let t = Instant::now();
    let other_pcm = cache.get(&other)?;
    let other_ms = t.elapsed().as_secs_f64() * 1000.0;
    // Changing the source file in place must change the key too.
    let before = Key::of(Path::new(&file), 0, frames, (1, 1), "");
    let copy = cache_dir().join("replaced.wav");
    std::fs::copy(&file, &copy).map_err(|e| e.to_string())?;
    let first = Key::of(&copy, 0, frames, (1, 1), "");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&copy, std::fs::read(&copy).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let second = Key::of(&copy, 0, frames, (1, 1), "");
    Ok(json!({
        "runs": runs,
        "cold_builds": cache.cold_builds,
        "hits": cache.hits,
        "misses": cache.misses,
        "resident_bytes": cache.resident_bytes(),
        "entries": cache.entries(),
        "disk_bytes": cache.disk_bytes(),
        "different_span": {"ms": other_ms, "frames": other_pcm.frames()},
        "mtime_keying": {
            "before": format!("{:?}", before),
            "after_same_file": format!("{:?}", first),
            "after_touch": format!("{:?}", second),
            "key_changed_on_touch": first != second,
        },
    }))
}

fn cmd_parity(a: &std::collections::HashMap<String, String>) -> Result<Value, String> {
    let file = a.get("file").cloned().ok_or("--file required")?;
    let seconds: i64 = a.get("seconds").and_then(|v| v.parse().ok()).unwrap_or(8);
    let project = build_project(2, 2, 30 * 20, seconds + 30);
    let frames = (seconds * 48_000) as usize;
    let mut mixer = Mixer::new(&project);
    let key = Key::of(Path::new(&file), 0, (seconds as u64 + 30) * pcm::RATE as u64, (1, 1), "");
    let pcm = Cache::new(cache_dir()).get(&key)?;
    let mut cached = PcmSource::new(pcm);
    let mut via_cache = vec![0f32; frames * 2];
    let mut pos = 0i64;
    for chunk in via_cache.chunks_mut(960) {
        mixer.process(&mut cached, pos, chunk)?;
        pos += (chunk.len() / 2) as i64;
    }
    let mut direct = Vec::new();
    let mut dec = arc_media::AudioDecoder::open(Path::new(&file), 48_000).map_err(|e| e.to_string())?;
    let mut buf = vec![0f32; frames * 2];
    let mut got = 0usize;
    while got < frames {
        let n = dec.read(got as i64, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        direct.extend_from_slice(&buf[..n * 2]);
        got += n;
    }
    direct.resize(frames * 2, 0.0);
    let mut mixer2 = Mixer::new(&project);
    let mut via_direct = vec![0f32; frames * 2];
    let mut src = DirectSource { decoder: arc_media::AudioDecoder::open(Path::new(&file), 48_000).map_err(|e| e.to_string())? };
    let mut pos = 0i64;
    for chunk in via_direct.chunks_mut(960) {
        mixer2.process(&mut src, pos, chunk)?;
        pos += (chunk.len() / 2) as i64;
    }
    let mut max_diff = 0f32;
    let mut sum_sq = 0f64;
    for (a, b) in via_cache.iter().zip(via_direct.iter()) {
        let d = (a - b).abs();
        if d > max_diff { max_diff = d; }
        sum_sq += (d as f64) * (d as f64);
    }
    let rms = (sum_sq / via_cache.len().max(1) as f64).sqrt();
    let shared = Time::new(seconds, 1).map_err(|e| e.to_string())?;
    Ok(json!({
        "seconds": seconds,
        "cache_checksum": hash_samples(&via_cache),
        "direct_checksum": hash_samples(&via_direct),
        "identical": hash_samples(&via_cache) == hash_samples(&via_direct),
        "cache_len": via_cache.len(),
        "max_abs_diff": max_diff,
        "rms_diff": rms,
        "direct_len": via_direct.len(),
        "duration": format!("{shared:?}"),
    }))
}

/// A `SampleSource` that decodes whenever the mixer asks: the current Arcline preview path.
struct DirectSource {
    decoder: arc_media::AudioDecoder,
}

impl SampleSource for DirectSource {
    fn read(&mut self, _asset: &arc_core::ids::AssetId, start: i64, dst: &mut [f32]) -> Result<(), String> {
        let frames = dst.len() / 2;
        self.decoder
            .read(start, &mut dst[..frames * 2])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
