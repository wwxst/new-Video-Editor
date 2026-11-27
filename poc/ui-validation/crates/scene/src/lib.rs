//! Deterministic timeline data and measurement helpers shared by both UI proofs of concept.
//!
//! The same scene is drawn by the `egui-poc` and the `slint-poc` binary, so the two frame-time
//! numbers describe the same amount of content. Nothing here depends on a UI toolkit.

use serde_json::json;
use std::time::Instant;

/// What a lane shows. Mirrors Arcline's six content-derived track types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Video,
    Overlay,
    Title,
    Audio,
    Music,
    Sfx,
}

impl Kind {
    pub fn lane_height(self) -> f32 {
        match self {
            Kind::Video => 76.0,
            Kind::Overlay | Kind::Audio | Kind::Music | Kind::Sfx => 50.0,
            Kind::Title => 24.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Video => "Video",
            Kind::Overlay => "Overlay",
            Kind::Title => "Title",
            Kind::Audio => "Audio",
            Kind::Music => "Music",
            Kind::Sfx => "SFX",
        }
    }
}

/// One keyframe marker on a clip.
#[derive(Debug, Clone, Copy)]
pub struct Key {
    /// Seconds from the clip start.
    pub at: f32,
    /// 0..1, drawn as the diamond's vertical position in the clip.
    pub value: f32,
}

/// One waveform point: normalized -1..1.
pub type WavePoint = f32;

#[derive(Debug, Clone)]
pub struct Clip {
    /// Stable id across a session; the UI uses it for selection.
    pub id: usize,
    /// Index into [`Scene::tracks`].
    pub track: usize,
    pub start: f32,
    pub duration: f32,
    pub label: String,
    pub color: [u8; 3],
    /// Whether the clip shows a thumbnail (video and stills do).
    pub thumb: bool,
    pub wave: Vec<WavePoint>,
    pub keys: Vec<Key>,
}

impl Clip {
    pub fn end(&self) -> f32 {
        self.start + self.duration
    }
}

#[derive(Debug, Clone)]
pub struct Track {
    pub id: usize,
    pub kind: Kind,
    pub name: String,
    pub muted: bool,
    pub locked: bool,
    /// Lane top relative to the first lane, in points.
    pub top: f32,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub tracks: Vec<Track>,
    /// End of the last clip, seconds.
    pub duration: f32,
}

/// Small deterministic PRNG so a run is reproducible without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 10_000) as f32 / 10_000.0
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

const PALETTE: [[u8; 3]; 6] = [
    [0x2e, 0x5b, 0x8a],
    [0x3d, 0x6b, 0x4f],
    [0x7a, 0x4a, 0x3a],
    [0x5a, 0x46, 0x7a],
    [0x6b, 0x5f, 0x2e],
    [0x3a, 0x6a, 0x72],
];

/// Build a scene with `clips` clips spread over six lanes.
///
/// Video/overlay/title clips carry 8 keyframes each; audio lanes carry a 48-point waveform.
pub fn generate(clips: usize, seed: u64) -> Scene {
    let kinds = [
        Kind::Video,
        Kind::Overlay,
        Kind::Title,
        Kind::Audio,
        Kind::Music,
        Kind::Sfx,
    ];
    let mut tracks: Vec<Track> = kinds
        .iter()
        .enumerate()
        .map(|(id, &kind)| Track {
            id,
            kind,
            name: kind.name().to_string(),
            muted: false,
            locked: false,
            top: 0.0,
            clips: Vec::new(),
        })
        .collect();
    let mut top = 0.0;
    for t in &mut tracks {
        t.top = top;
        top += t.kind.lane_height();
    }
    let mut rng = Rng(seed);
    // Timeline length grows with the content so density stays roughly constant.
    let per_track = clips.div_ceil(tracks.len()).max(1);
    for (ti, track) in tracks.iter_mut().enumerate() {
        let mut cursor = rng.range(0.0, 3.0);
        for i in 0..per_track {
            let duration = match track.kind {
                Kind::Title => rng.range(0.6, 2.5),
                Kind::Audio | Kind::Music | Kind::Sfx => rng.range(2.0, 8.0),
                _ => rng.range(1.5, 6.5),
            };
            let wave: Vec<f32> = if matches!(track.kind, Kind::Audio | Kind::Music | Kind::Sfx) {
                (0..48)
                    .map(|k| {
                        let envelope = (k as f32 / 48.0 * std::f32::consts::PI).sin().abs();
                        (rng.unit() * 0.9 + 0.1) * envelope
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let keys: Vec<Key> = if matches!(track.kind, Kind::Video | Kind::Overlay | Kind::Title)
            {
                (0..8)
                    .map(|k| Key {
                        at: duration * (k as f32 + 0.5) / 8.0,
                        value: rng.unit(),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let id = track.clips.len() + ti * per_track;
            track.clips.push(Clip {
                id,
                track: ti,
                start: cursor,
                duration,
                label: format!("{}-{ti}-{i}", track.kind.name()),
                color: PALETTE[(ti + i) % PALETTE.len()],
                thumb: matches!(track.kind, Kind::Video | Kind::Overlay),
                wave,
                keys,
            });
            cursor += duration + rng.range(0.0, 1.2);
        }
    }
    let duration = tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(Clip::end))
        .fold(0.0f32, f32::max);
    Scene { tracks, duration }
}

impl Scene {
    pub fn total_clips(&self) -> usize {
        self.tracks.iter().map(|t| t.clips.len()).sum()
    }

    /// The clip with `id`, together with its track.
    pub fn clip(&self, id: usize) -> Option<(&Track, &Clip)> {
        self.tracks
            .iter()
            .find_map(|t| t.clips.iter().find(|c| c.id == id).map(|c| (t, c)))
    }

    /// Total lane height, in points.
    pub fn lanes_height(&self) -> f32 {
        self.tracks.iter().map(|t| t.kind.lane_height()).sum()
    }

    /// Clips that intersect `[from, to)`, per track. This is the same "only paint what is
    /// visible" question both toolkits have to answer; the POC draws everything so the cost of
    /// skipping is visible rather than assumed.
    pub fn visible(&self, from: f32, to: f32) -> Vec<(&Track, Vec<&Clip>)> {
        self.tracks
            .iter()
            .map(|t| {
                let clips = t
                    .clips
                    .iter()
                    .filter(|c| c.end() > from && c.start < to)
                    .collect();
                (t, clips)
            })
            .collect()
    }
}

/// A frame-time recorder: one sample per rendered frame, in microseconds.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub frames: Vec<u64>,
    /// Optional separate cost of the UI build/paint step (µs).
    pub paint: Vec<u64>,
    /// Time between two completed frames (µs), for pacing/dropped-frame analysis.
    pub intervals: Vec<u64>,
    /// Cost of preparing a compositor frame (draw + readback or texture import), µs.
    pub prepare: Vec<u64>,
}

impl Metrics {
    pub fn record(&mut self, us: u64) {
        self.frames.push(us);
    }

    pub fn record_interval(&mut self, us: u64) {
        self.intervals.push(us);
    }

    pub fn record_prepare(&mut self, us: u64) {
        self.prepare.push(us);
    }

    pub fn percentile_unsorted(values: &[u64], p: f64) -> u64 {
        let mut v = values.to_vec();
        v.sort_unstable();
        Self::percentile(&v, p)
    }

    pub fn percentile(sorted: &[u64], p: f64) -> u64 {
        if sorted.is_empty() {
            return 0;
        }
        let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    pub fn report(&self, label: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut sorted = self.frames.clone();
        sorted.sort_unstable();
        let mean = if sorted.is_empty() {
            0.0
        } else {
            sorted.iter().sum::<u64>() as f64 / sorted.len() as f64
        };
        // The first ten frames are warm-up: shader compilation, font atlas, first upload.
        let steady: Vec<u64> = self.frames.iter().skip(10).copied().collect();
        let mut steady_sorted = steady.clone();
        steady_sorted.sort_unstable();
        let mut intervals = self.intervals.clone();
        intervals.sort_unstable();
        let intervals_steady: Vec<u64> = intervals.iter().skip(10).copied().collect();
        let mut intervals_steady_sorted = intervals_steady.clone();
        intervals_steady_sorted.sort_unstable();
        let dropped = intervals.iter().filter(|&&us| us > 20_000).count();
        let mut prepare = self.prepare.clone();
        prepare.sort_unstable();
        json!({
            "label": label,
            "frames": sorted.len(),
            "frame_us_mean": mean,
            "frame_us_p50": Self::percentile(&sorted, 0.50),
            "frame_us_p95": Self::percentile(&sorted, 0.95),
            "frame_us_max": sorted.last().copied().unwrap_or(0),
            "steady_frames": steady_sorted.len(),
            "steady_us_mean": if steady_sorted.is_empty() { 0.0 } else { steady_sorted.iter().sum::<u64>() as f64 / steady_sorted.len() as f64 },
            "steady_us_p50": Self::percentile(&steady_sorted, 0.50),
            "steady_us_p95": Self::percentile(&steady_sorted, 0.95),
            "steady_us_max": steady_sorted.last().copied().unwrap_or(0),
            "interval_us_p50": Self::percentile(&intervals_steady_sorted, 0.50),
            "interval_us_p95": Self::percentile(&intervals_steady_sorted, 0.95),
            "interval_us_p99": Self::percentile(&intervals_steady_sorted, 0.99),
            "interval_us_max": intervals.last().copied().unwrap_or(0),
            "dropped_over_20ms": dropped,
            "prepare_us_p50": Self::percentile(&prepare, 0.50),
            "prepare_us_p95": Self::percentile(&prepare, 0.95),
            "prepare_us_max": prepare.last().copied().unwrap_or(0),
            "fps_from_mean": if mean > 0.0 { 1_000_000.0 / mean } else { 0.0 },
            "extra": extra,
        })
    }
}

/// A moving RGBA frame, the CPU-side stand-in for a decoded video frame.
pub fn synthetic_frame(width: u32, height: u32, tick: u32, out: &mut Vec<u8>) {
    let w = width as usize;
    let h = height as usize;
    out.resize(w * h * 4, 0);
    let phase = tick as f32 * 0.02;
    for y in 0..h {
        let fy = y as f32 / h as f32;
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let i = (y * w + x) * 4;
            out[i] = ((fx * 255.0 + phase.sin() * 40.0) as i32).clamp(0, 255) as u8;
            out[i + 1] = ((fy * 255.0 + phase.cos() * 40.0) as i32).clamp(0, 255) as u8;
            out[i + 2] = (((fx + fy) * 0.5 * 255.0) as i32).clamp(0, 255) as u8;
            out[i + 3] = 255;
        }
    }
}

/// Wall-clock stopwatch returning whole microseconds.
pub struct Stop(Instant);

impl Stop {
    pub fn start() -> Stop {
        Stop(Instant::now())
    }
    pub fn us(self) -> u64 {
        self.0.elapsed().as_micros() as u64
    }
}

/// Parse `--key value` arguments into a map (both stacks use the same harness).
pub fn parse_args() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let key = k.trim_start_matches("--").to_string();
        let value = it.next().unwrap_or_default();
        map.insert(key, value);
    }
    map
}

pub fn arg_usize(
    args: &std::collections::HashMap<String, String>,
    key: &str,
    default: usize,
) -> usize {
    args.get(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn arg_f32(args: &std::collections::HashMap<String, String>, key: &str, default: f32) -> f32 {
    args.get(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn arg_str<'a>(
    args: &'a std::collections::HashMap<String, String>,
    key: &str,
    default: &'a str,
) -> &'a str {
    args.get(key).map(String::as_str).unwrap_or(default)
}
