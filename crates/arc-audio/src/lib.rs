//! Arcline audio (docs/05): the sample-accurate mix graph shared by playback and export.
//!
//! Parity contract (docs/05 §7): [`Mixer::process`] computes every output sample from the sample
//! index alone, so rendering a range in blocks of any size yields identical samples. Real-time
//! playback and offline export call the same function; transport-speed WSOLA and scrub live in
//! the monitoring stage after the master bus and are not part of this crate's output.
//!
//! Sources are supplied through [`SampleSource`] so this crate never depends on FFmpeg.

use arc_core::ids::{AssetId, ClipId};
use arc_core::model::{Project, TrackKind};
use arc_core::time::{Ratio, Round, sample_index};
use std::io::Write;

/// Provides decoded audio at the sequence sample rate (interleaved stereo f32).
pub trait SampleSource {
    /// Fill `dst` (2 × frames) with the asset's samples starting at `start` (source sample index
    /// relative to the media start). Out-of-range samples must be zero.
    fn read(&mut self, asset: &AssetId, start: i64, dst: &mut [f32]) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq)]
struct ClipMix {
    clip: ClipId,
    asset: AssetId,
    /// Timeline sample range [start, end).
    start: i64,
    end: i64,
    /// Source sample at `start`.
    source_in: i64,
    speed: Ratio,
    gain: f32,
    pan_l: f32,
    pan_r: f32,
    fade_in: i64,
    fade_out: i64,
}

fn db_to_lin(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

/// Constant-power pan normalized to unity at center: pan ∈ [-1, 1].
fn pan_gains(pan: f64) -> (f32, f32) {
    let theta = (pan.clamp(-1.0, 1.0) + 1.0) * std::f64::consts::FRAC_PI_4;
    let k = std::f64::consts::SQRT_2;
    ((k * theta.cos()) as f32, (k * theta.sin()) as f32)
}

/// The mix of one sequence at one sample rate.
#[derive(Debug, Clone)]
pub struct Mixer {
    rate: u32,
    clips: Vec<ClipMix>,
    master_gain: f32,
    scratch: Vec<f32>,
}

impl Mixer {
    /// Build the mix from a snapshot: enabled, unmuted audio tracks; enabled media clips.
    /// Timeline positions map to samples with `Round::Nearest` (29.97 frame boundaries are not
    /// sample-aligned: one frame = 1601.6 samples at 48 kHz).
    pub fn new(project: &Project) -> Mixer {
        let rate = project.sequence().map(|s| s.sample_rate).unwrap_or(48_000);
        let mut clips = Vec::new();
        if let Some(seq) = project.sequence() {
            for t in seq
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio && t.enabled && !t.muted)
            {
                for c in t.clips.iter().filter(|c| c.enabled) {
                    let Some(asset) = &c.asset else { continue };
                    let s = |x| sample_index(x, rate, Round::Nearest).unwrap_or(0);
                    let start = s(c.start);
                    let end = c.end().map(s).unwrap_or(start);
                    let (pan_l, pan_r) = pan_gains(c.audio.pan);
                    clips.push(ClipMix {
                        clip: c.id.clone(),
                        asset: asset.clone(),
                        start,
                        end,
                        source_in: s(c.source_in),
                        speed: c.speed,
                        gain: db_to_lin(c.audio.gain_db + t.gain_db),
                        pan_l,
                        pan_r,
                        fade_in: s(c.audio.fade_in),
                        fade_out: s(c.audio.fade_out),
                    });
                }
            }
        }
        Mixer {
            rate,
            clips,
            master_gain: 1.0,
            scratch: Vec::new(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Total length in samples (end of the last audio clip).
    pub fn len_samples(&self) -> i64 {
        self.clips.iter().map(|c| c.end).max().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// Mix timeline samples `[start, start + out.len()/2)` into `out` (interleaved stereo).
    pub fn process(
        &mut self,
        source: &mut dyn SampleSource,
        start: i64,
        out: &mut [f32],
    ) -> Result<(), String> {
        out.fill(0.0);
        let frames = (out.len() / 2) as i64;
        let end = start + frames;
        for c in &self.clips {
            let (a, b) = (start.max(c.start), end.min(c.end));
            if a >= b {
                continue;
            }
            // source position of output sample n: source_in + (n - start_c) × speed (exact)
            let (num, den) = (c.speed.num(), c.speed.den() as i64);
            let pos = |n: i64| -> (i64, i64) {
                let p = (n - c.start) * num;
                (c.source_in + p.div_euclid(den), p.rem_euclid(den))
            };
            let (first, _) = pos(a);
            let (last, _) = pos(b - 1);
            let need = (last - first + 2) as usize; // +1 for interpolation
            self.scratch.resize(need * 2, 0.0);
            source.read(&c.asset, first, &mut self.scratch)?;
            for n in a..b {
                let (i, rem) = pos(n);
                let j = (i - first) as usize;
                let f = rem as f32 / den as f32;
                let (l0, r0) = (self.scratch[j * 2], self.scratch[j * 2 + 1]);
                let (l, r) = if rem == 0 {
                    (l0, r0)
                } else {
                    let (l1, r1) = (self.scratch[j * 2 + 2], self.scratch[j * 2 + 3]);
                    (l0 + (l1 - l0) * f, r0 + (r1 - r0) * f)
                };
                let mut g = c.gain;
                if c.fade_in > 0 && n - c.start < c.fade_in {
                    g *= (n - c.start) as f32 / c.fade_in as f32;
                }
                if c.fade_out > 0 && c.end - n <= c.fade_out {
                    g *= (c.end - n - 1).max(0) as f32 / c.fade_out as f32;
                }
                let o = ((n - start) * 2) as usize;
                out[o] += l * g * c.pan_l;
                out[o + 1] += r * g * c.pan_r;
            }
        }
        if self.master_gain != 1.0 {
            for s in out.iter_mut() {
                *s *= self.master_gain;
            }
        }
        Ok(())
    }

    /// Offline render of `[start, end)` in fixed blocks.
    pub fn render(
        &mut self,
        source: &mut dyn SampleSource,
        start: i64,
        end: i64,
        block: usize,
    ) -> Result<Vec<f32>, String> {
        let mut out = vec![0f32; ((end - start).max(0) as usize) * 2];
        let mut pos = start;
        for chunk in out.chunks_mut(block * 2) {
            self.process(source, pos, chunk)?;
            pos += (chunk.len() / 2) as i64;
        }
        Ok(out)
    }
}

/// Write interleaved stereo f32 samples as a 32-bit float WAV.
pub fn write_wav_f32(path: &std::path::Path, rate: u32, samples: &[f32]) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data_len = (samples.len() * 4) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?; // IEEE float
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * 8).to_le_bytes())?;
    f.write_all(&8u16.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}

/// BLAKE3-free, dependency-free stable hash of samples for parity checks (FNV-1a over bits).
pub fn hash_samples(samples: &[f32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for s in samples {
        for b in s.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_core::engine::{ApplyRequest, Engine};
    use arc_core::{Command, FrameRate};
    use serde_json::json;

    /// Synthetic source: left = sample index / 1e5, right = −left (exact, position-revealing).
    struct Ramp;
    impl SampleSource for Ramp {
        fn read(&mut self, _a: &AssetId, start: i64, dst: &mut [f32]) -> Result<(), String> {
            for (k, fr) in dst.chunks_mut(2).enumerate() {
                let i = start + k as i64;
                let v = if (0..480_000).contains(&i) {
                    i as f32 / 100_000.0
                } else {
                    0.0
                };
                fr[0] = v;
                fr[1] = -v;
            }
            Ok(())
        }
    }

    fn project(extra: serde_json::Value) -> arc_core::model::Project {
        let p = arc_core::model::Project::new(
            "a",
            FrameRate::Fps29_97,
            arc_core::model::Resolution { w: 64, h: 64 },
        );
        let cat = arc_core::StaticCatalog::default();
        let mut e = Engine::new(p, 0, std::sync::Arc::new(cat)).expect("engine");
        let mut cmds = vec![
            json!({"op":"track.add","id":"tr_a1","kind":"audio"}),
            json!({"op":"asset.import","id":"as_m","path":"m.wav","kind":"audio","probe":{"duration":{"num":10,"den":1}}}),
        ];
        if let serde_json::Value::Array(a) = extra {
            cmds.extend(a);
        }
        let cmds: Vec<Command> =
            serde_json::from_value(serde_json::Value::Array(cmds)).expect("cmds");
        e.apply(ApplyRequest {
            commands: cmds,
            ..Default::default()
        })
        .expect("apply");
        (*e.snapshot()).clone()
    }

    #[test]
    fn block_size_independent_parity() {
        let p = project(json!([
            {"op":"timeline.add_clip","id":"cl_1","track":"tr_a1","asset":"as_m","start":"00:00:00:15","duration":"00:00:02:00","source_in":"00:00:01:00"},
            {"op":"timeline.add_clip","id":"cl_2","track":"tr_a1","asset":"as_m","start":"00:00:03:00","duration":"00:00:01:00","speed":{"num":1,"den":2}},
            {"op":"audio.set_gain","clip":"cl_1","gain_db":-6.0},
            {"op":"audio.set_pan","clip":"cl_2","pan":0.4},
            {"op":"audio.set_fades","clip":"cl_1","fade_in":{"num":1,"den":4},"fade_out":{"num":1,"den":3}}
        ]));
        let mut m = Mixer::new(&p);
        let end = m.len_samples();
        let reference = m.render(&mut Ramp, 0, end, 1024).expect("offline");
        for block in [1usize, 64, 128, 441, 4096] {
            let other = m.render(&mut Ramp, 0, end, block).expect("blocks");
            assert_eq!(
                hash_samples(&other),
                hash_samples(&reference),
                "block {block}"
            );
        }
    }

    #[test]
    fn positions_gain_pan_fades_speed() {
        let p = project(json!([
            {"op":"timeline.add_clip","id":"cl_1","track":"tr_a1","asset":"as_m","start":"00:00:00:00","duration":"00:00:01:00","source_in":"00:00:01:00"}
        ]));
        let mut m = Mixer::new(&p);
        // 29.97: 30 frames = 1.001 s = 48048 samples; source_in 1.001 s = 48048
        assert_eq!(m.len_samples(), 48_048);
        let out = m.render(&mut Ramp, 0, 10, 10).expect("r");
        assert!(
            (out[0] - 0.480_48).abs() < 1e-6,
            "timeline sample 0 plays source sample 48048"
        );
        assert!(
            (out[1] + 0.480_48).abs() < 1e-6,
            "right channel, center pan = unity"
        );

        let p = project(json!([
            {"op":"timeline.add_clip","id":"cl_1","track":"tr_a1","asset":"as_m","start":"00:00:00:00","duration":"00:00:01:00","speed":{"num":1,"den":2}},
            {"op":"audio.set_gain","clip":"cl_1","gain_db":-6.020_599_913},
            {"op":"audio.set_pan","clip":"cl_1","pan":-1.0}
        ]));
        let mut m = Mixer::new(&p);
        let out = m.render(&mut Ramp, 0, 4, 4).expect("r");
        // half speed: sample 3 → source 1.5 (interpolated), gain 0.5, hard left √2
        let want = 1.5 / 100_000.0 * 0.5 * std::f32::consts::SQRT_2;
        assert!((out[6] - want).abs() < 1e-9, "{} vs {want}", out[6]);
        assert!(out[7].abs() < 1e-9, "hard left silences the right channel");
    }

    #[test]
    fn wav_writer_header() {
        let dir = std::env::temp_dir().join(format!("arc-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let p = dir.join("t.wav");
        write_wav_f32(&p, 48_000, &[0.0, 0.5, -0.5, 1.0]).expect("wav");
        let b = std::fs::read(&p).expect("read");
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(b.len(), 44 + 16);
        let _ = std::fs::remove_dir_all(dir);
    }
}
