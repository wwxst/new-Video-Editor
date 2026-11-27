//! A PCM cache in the shape Concat uses: decode once to a 48 kHz stereo s16 file, memory-map it,
//! and key the file by everything that changes the samples.
//!
//! The key deliberately includes the source file's length and modification time as well as the
//! decode parameters, because Concat's key does not and therefore serves stale audio after a file
//! is replaced in place.

use arc_audio::SampleSource;
use arc_core::ids::AssetId;
use arc_media::AudioDecoder;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

pub const RATE: u32 = 48_000;
pub const CHANNELS: u32 = 2;

/// Everything that determines the decoded samples. Any change produces a different key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub path: String,
    pub file_len: u64,
    pub mtime_secs: u64,
    /// First source sample (at `source_rate`) to keep.
    pub start: i64,
    pub frames: u64,
    pub out_rate: u32,
    pub speed_num: i64,
    pub speed_den: i64,
    /// Filters applied at decode time, e.g. `"loudnorm:I=-16"`.
    pub chain: String,
    /// Bumped whenever the decoder or mix semantics change.
    pub format_version: u32,
}

impl Key {
    pub fn of(path: &Path, start: i64, frames: u64, speed: (i64, i64), chain: &str) -> Key {
        let meta = std::fs::metadata(path).ok();
        let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Key {
            path: path.to_string_lossy().to_string(),
            file_len: len,
            mtime_secs: mtime,
            start,
            frames,
            out_rate: RATE,
            speed_num: speed.0,
            speed_den: speed.1,
            chain: chain.to_string(),
            format_version: 1,
        }
    }

    pub fn id(&self) -> String {
        let mut h = blake3::Hasher::new();
        h.update(self.path.as_bytes());
        for v in [
            self.file_len,
            self.mtime_secs,
            self.start as u64,
            self.frames,
            self.out_rate as u64,
            self.speed_num as u64,
            self.speed_den as u64,
            self.format_version as u64,
        ] {
            h.update(&v.to_le_bytes());
        }
        h.update(self.chain.as_bytes());
        h.finalize().to_hex()[..16].to_string()
    }
}

/// One cached span, memory-mapped: 44-byte WAV header then interleaved s16 stereo.
pub struct Pcm {
    map: memmap2::Mmap,
    frames: u64,
}

impl Pcm {
    pub fn frames(&self) -> u64 {
        self.frames
    }
    pub fn bytes(&self) -> u64 {
        self.map.len() as u64
    }
    fn sample(&self, frame: i64, channel: usize) -> f32 {
        if frame < 0 || frame as u64 >= self.frames {
            return 0.0;
        }
        let offset = 44 + (frame as usize * CHANNELS as usize + channel) * 2;
        let bytes = &self.map[offset..offset + 2];
        i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32768.0
    }
}

pub struct Cache {
    dir: PathBuf,
    ram: HashMap<String, Arc<Pcm>>,
    pub cold_builds: u64,
    pub hits: u64,
    pub misses: u64,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Cache {
        std::fs::create_dir_all(&dir).ok();
        Cache {
            dir,
            ram: HashMap::new(),
            cold_builds: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Return the span, building and mapping it on first use.
    pub fn get(&mut self, key: &Key) -> Result<Arc<Pcm>, String> {
        let id = key.id();
        if let Some(pcm) = self.ram.get(&id) {
            self.hits += 1;
            return Ok(Arc::clone(pcm));
        }
        let path = self.dir.join(format!("{id}.pcm"));
        let pcm = if path.exists() {
            self.hits += 1;
            Arc::new(open_map(&path)?)
        } else {
            self.misses += 1;
            self.cold_builds += 1;
            let tmp = path.with_extension("pcm.part");
            decode_to(key, &tmp)?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            Arc::new(open_map(&path)?)
        };
        self.ram.insert(id, Arc::clone(&pcm));
        Ok(pcm)
    }

    pub fn resident_bytes(&self) -> u64 {
        self.ram.values().map(|p| p.bytes()).sum()
    }
    pub fn entries(&self) -> usize {
        self.ram.len()
    }
    pub fn disk_bytes(&self) -> u64 {
        std::fs::read_dir(&self.dir)
            .map(|it| {
                it.filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }
}

fn open_map(path: &Path) -> Result<Pcm, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    // SAFETY: the cache file is written once under a temporary name and renamed; nothing mutates it
    // afterwards, and the mapping is read-only.
    let map = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| e.to_string())?;
    if map.len() < 44 {
        return Err("cache file short".into());
    }
    let frames = ((map.len() - 44) / (CHANNELS as usize * 2)) as u64;
    Ok(Pcm { map, frames })
}

/// Decode the span to a WAV file of 48 kHz stereo s16 (the format the cache stores).
fn decode_to(key: &Key, out: &Path) -> Result<(), String> {
    let mut dec = AudioDecoder::open(Path::new(&key.path), key.out_rate).map_err(|e| e.to_string())?;
    let mut samples = Vec::with_capacity(key.frames as usize * CHANNELS as usize);
    let mut buf = vec![0f32; 8192 * 2];
    let mut pos = key.start;
    while (samples.len() as u64) < key.frames * CHANNELS as u64 {
        let want = ((key.frames * CHANNELS as u64 - samples.len() as u64) as usize).min(buf.len());
        let n = dec
            .read(pos, &mut buf[..want])
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        samples.extend_from_slice(&buf[..n * CHANNELS as usize]);
        pos += n as i64;
    }
    samples.resize(key.frames as usize * CHANNELS as usize, 0.0);
    // Speed is a decode-time property in this design: resample by linear interpolation.
    if key.speed_num != key.speed_den && key.speed_den != 0 {
        let ratio = key.speed_num as f64 / key.speed_den as f64;
        let out_frames = (key.frames as f64 / ratio) as usize;
        let mut sped = Vec::with_capacity(out_frames * CHANNELS as usize);
        for i in 0..out_frames {
            let src = (i as f64 * ratio) as usize;
            let a = src.min(samples.len() / CHANNELS as usize - 1);
            let b = (a + 1).min(samples.len() / CHANNELS as usize - 1);
            let f = (i as f64 * ratio) - src as f64;
            for ch in 0..CHANNELS as usize {
                let s = samples[a * CHANNELS as usize + ch] * (1.0 - f as f32)
                    + samples[b * CHANNELS as usize + ch] * f as f32;
                sped.push(s);
            }
        }
        samples = sped;
    }
    let frames = samples.len() / CHANNELS as usize;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + samples.len() * 2) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&(CHANNELS as u16).to_le_bytes());
    bytes.extend_from_slice(&key.out_rate.to_le_bytes());
    bytes.extend_from_slice(&(key.out_rate * CHANNELS * 2).to_le_bytes());
    bytes.extend_from_slice(&((CHANNELS * 2) as u16).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&((samples.len() * 2) as u32).to_le_bytes());
    for s in &samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(out, &bytes).map_err(|e| e.to_string())?;
    let _ = frames;
    Ok(())
}

/// A `SampleSource` reading from a cached span: no I/O, no FFmpeg, no allocation.
pub struct PcmSource {
    pcm: Arc<Pcm>,
}

impl PcmSource {
    pub fn new(pcm: Arc<Pcm>) -> PcmSource {
        PcmSource { pcm }
    }
}

impl SampleSource for PcmSource {
    fn read(&mut self, _asset: &AssetId, start: i64, dst: &mut [f32]) -> Result<(), String> {
        for (k, frame) in dst.chunks_mut(2).enumerate() {
            let i = start + k as i64;
            frame[0] = self.pcm.sample(i, 0);
            frame[1] = self.pcm.sample(i, 1);
        }
        Ok(())
    }
}
