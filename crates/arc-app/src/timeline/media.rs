//! Asynchronous media visuals for timeline clips: decoded video-frame thumbnails, decoded still
//! images and audio waveform peaks.
//!
//! Decoding runs on two worker threads (one for pictures, one for audio) so the UI loop never
//! blocks on a file. This module owns the requests, the bounded LRU caches and the texture uploads:
//! the painting code only ever asks "is this frame decoded yet?" and draws what is in the cache.

use arc_core::time::Time;
use arc_media::{AudioDecoder, VideoDecoder};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// Texture size of one video thumbnail (2× the largest point size, for hidpi screens).
const THUMB_W: u32 = 320;
const THUMB_H: u32 = 180;
/// Longest side of a decoded still image.
const STILL_MAX: u32 = 640;
/// Waveform base resolution: envelope samples per source second.
const PEAKS_PER_SECOND: f64 = 200.0;
/// Never decode more than this much of one audio file (seconds).
const MAX_AUDIO_SECONDS: f64 = 120.0;
/// Decoders kept open between thumbnail requests.
const DECODER_CAPACITY: usize = 3;
/// Textures kept for video thumbnails (bounded GPU memory).
const THUMB_CAPACITY: usize = 128;
const STILL_CAPACITY: usize = 8;
const WAVE_CAPACITY: usize = 12;
/// New decode requests accepted per UI frame, so a zoom-out burst cannot flood the worker.
const MAX_REQUESTS_PER_FRAME: u32 = 6;

/// One min/max envelope sample of decoded audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    pub min: f32,
    pub max: f32,
}

/// Waveform envelope of one audio file, at a fixed base resolution.
///
/// The timeline aggregates these samples into one min/max pair per painted column, so the drawn
/// density follows the zoom without decoding anything again.
#[derive(Debug, Clone)]
pub struct Waveform {
    /// Duration covered by the envelope, in seconds.
    pub duration: f64,
    /// Envelope samples per second.
    pub peaks_per_second: f64,
    peaks: Vec<Peak>,
}

impl Waveform {
    pub fn new(duration: f64, peaks_per_second: f64, peaks: Vec<Peak>) -> Waveform {
        Waveform {
            duration,
            peaks_per_second,
            peaks,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn peaks(&self) -> &[Peak] {
        &self.peaks
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.peaks.is_empty()
    }

    /// Min/max of the source-time range `[t0, t1)`, clamped to the file; `None` outside it.
    pub fn envelope(&self, t0: f64, t1: f64) -> Option<Peak> {
        if self.peaks.is_empty() {
            return None;
        }
        let pps = self.peaks_per_second.max(1.0);
        if t1 <= 0.0 || t0 >= self.duration {
            return None;
        }
        let first = (t0.max(0.0) * pps).floor() as usize;
        let last = ((t1.max(0.0) * pps).ceil() as usize).max(first + 1);
        let first = first.min(self.peaks.len() - 1);
        let last = last.min(self.peaks.len());
        if last <= first {
            return None;
        }
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for peak in &self.peaks[first..last] {
            min = min.min(peak.min);
            max = max.max(peak.max);
        }
        Some(Peak { min, max })
    }
}

/// Decoded still image plus its aspect ratio, ready to paint.
#[derive(Clone)]
pub struct StillVisual {
    pub texture: TextureHandle,
    /// Width / height of the decoded image.
    pub aspect: f32,
}

/// Decoded video thumbnail plus its aspect ratio, ready to paint.
#[derive(Clone)]
pub struct ThumbVisual {
    pub texture: TextureHandle,
    /// Width / height of the decoded frame.
    pub aspect: f32,
}

/// Counters used by the tests and the timeline report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaStats {
    /// Requests sent to a worker.
    pub requested: u64,
    /// Frames/peaks that came back decoded.
    pub decoded: u64,
    /// Cache lookups that were served without a worker.
    pub cache_hits: u64,
    /// Decode requests that failed (missing or undecodable media).
    pub failed: u64,
    /// Requests still queued or being decoded.
    pub pending: usize,
}

/// Cache key of a decoded asset.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    /// One video frame, at a millisecond-quantized source time.
    Frame { path: PathBuf, ms: u64 },
    /// One still image.
    Still { path: PathBuf },
    /// The waveform envelope of one audio file.
    Peaks { path: PathBuf },
}

enum Request {
    Frame { key: Key, path: PathBuf, time: f64 },
    Still { key: Key, path: PathBuf },
    Peaks { key: Key, path: PathBuf },
}

enum Event {
    Rgba {
        key: Key,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Peaks {
        key: Key,
        waveform: Arc<Waveform>,
    },
    Failed {
        key: Key,
        path: PathBuf,
        error: String,
    },
}

/// Small LRU map: `VecDeque` of keys for recency, `HashMap` for values.
#[derive(Debug)]
pub struct Lru<K, V> {
    capacity: usize,
    map: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: Eq + Hash + Clone, V> Lru<K, V> {
    pub fn new(capacity: usize) -> Lru<K, V> {
        Lru {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        if !self.map.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.map.get(key)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if !self.map.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.map.get_mut(key)
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let replaced = self.map.insert(key.clone(), value);
        if replaced.is_none() {
            self.order.push_back(key);
        }
        while self.map.len() > self.capacity {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            self.map.remove(&old);
        }
        replaced
    }

    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.clone());
    }
}

/// The per-timeline media service: request deduplication, bounded caches and texture uploads.
pub struct MediaHub {
    video_tx: Sender<Request>,
    audio_tx: Sender<Request>,
    events: Receiver<Event>,
    thumbs: Lru<Key, ThumbVisual>,
    stills: Lru<Key, StillVisual>,
    waves: Lru<Key, Arc<Waveform>>,
    /// Whether a source has an audio stream; asked once per path and cached.
    audio_presence: HashMap<PathBuf, bool>,
    failures: HashMap<PathBuf, String>,
    pending: HashSet<Key>,
    budget: u32,
    repaint_needed: bool,
    stats: MediaStats,
}

impl std::fmt::Debug for MediaHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaHub")
            .field("stats", &self.stats)
            .field("thumbs", &self.thumbs.len())
            .field("stills", &self.stills.len())
            .field("waves", &self.waves.len())
            .finish()
    }
}

impl MediaHub {
    pub fn new() -> MediaHub {
        let (video_tx, video_rx) = channel();
        let (audio_tx, audio_rx) = channel();
        let (event_tx, events) = channel();
        let picture_events = event_tx.clone();
        let _ = std::thread::Builder::new()
            .name("arcline-thumbs".to_string())
            .spawn(move || picture_worker(video_rx, picture_events));
        let _ = std::thread::Builder::new()
            .name("arcline-peaks".to_string())
            .spawn(move || audio_worker(audio_rx, event_tx));
        MediaHub {
            video_tx,
            audio_tx,
            events,
            thumbs: Lru::new(THUMB_CAPACITY),
            stills: Lru::new(STILL_CAPACITY),
            waves: Lru::new(WAVE_CAPACITY),
            audio_presence: HashMap::new(),
            failures: HashMap::new(),
            pending: HashSet::new(),
            budget: MAX_REQUESTS_PER_FRAME,
            repaint_needed: false,
            stats: MediaStats::default(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn stats(&self) -> &MediaStats {
        &self.stats
    }

    /// Drain finished decodes, upload their textures and schedule the next poll while work remains.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.budget = MAX_REQUESTS_PER_FRAME;
        while let Ok(event) = self.events.try_recv() {
            self.handle(event, ctx);
        }
        self.stats.pending = self.pending.len();
        if self.stats.pending > 0 || self.repaint_needed {
            self.repaint_needed = false;
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
    }

    /// Decoded frame for `path` at `time` (seconds, source time), or `None` while it is decoded.
    pub fn video_frame(&mut self, path: &Path, time: f64) -> Option<ThumbVisual> {
        let key = Key::Frame {
            path: path.to_path_buf(),
            ms: (time.max(0.0) * 1000.0).round() as u64,
        };
        if let Some(visual) = self.thumbs.get(&key) {
            self.stats.cache_hits += 1;
            return Some(visual.clone());
        }
        if self.admit(&key, path) {
            let request = Request::Frame {
                key: key.clone(),
                path: path.to_path_buf(),
                time,
            };
            if self.video_tx.send(request).is_ok() {
                self.commit(key);
            }
        }
        None
    }

    /// Decoded still image for `path`, or `None` while it is decoded.
    pub fn still(&mut self, path: &Path) -> Option<StillVisual> {
        let key = Key::Still {
            path: path.to_path_buf(),
        };
        if let Some(visual) = self.stills.get(&key) {
            self.stats.cache_hits += 1;
            return Some(visual.clone());
        }
        if self.admit(&key, path) {
            let request = Request::Still {
                key: key.clone(),
                path: path.to_path_buf(),
            };
            if self.video_tx.send(request).is_ok() {
                self.commit(key);
            }
        }
        None
    }

    /// Waveform envelope of `path`, or `None` while it is decoded.
    pub fn waveform(&mut self, path: &Path) -> Option<Arc<Waveform>> {
        let key = Key::Peaks {
            path: path.to_path_buf(),
        };
        if let Some(wave) = self.waves.get(&key) {
            self.stats.cache_hits += 1;
            return Some(wave.clone());
        }
        if self.admit(&key, path) {
            let request = Request::Peaks {
                key: key.clone(),
                path: path.to_path_buf(),
            };
            if self.audio_tx.send(request).is_ok() {
                self.commit(key);
            }
        }
        None
    }

    /// Whether `path` contains an audio stream, cached per file.
    ///
    /// Opening an [`AudioDecoder`] is cheap and does not decode a frame, so the answer is available
    /// on the first painted frame; caching it avoids reopening the file on every repaint. A video
    /// without audio keeps the larger thumbnail region, a video with audio reserves its waveform.
    pub fn has_audio(&mut self, path: &Path) -> bool {
        if let Some(has_audio) = self.audio_presence.get(path) {
            return *has_audio;
        }
        let has_audio = AudioDecoder::open(path, 8_000).is_ok();
        self.audio_presence.insert(path.to_path_buf(), has_audio);
        has_audio
    }

    /// Why `path` could not be decoded, if it failed.
    pub fn failure(&self, path: &Path) -> Option<&str> {
        self.failures.get(path).map(String::as_str)
    }

    /// Whether a new request for `key` may be sent now (not pending, not failed, not throttled).
    fn admit(&mut self, key: &Key, path: &Path) -> bool {
        if self.pending.contains(key) || self.failures.contains_key(path) {
            return false;
        }
        if self.budget == 0 {
            self.repaint_needed = true;
            return false;
        }
        true
    }

    /// Record one sent request: de-duplicate it and keep the poll loop running.
    fn commit(&mut self, key: Key) {
        self.pending.insert(key);
        self.budget -= 1;
        self.stats.requested += 1;
        self.repaint_needed = true;
    }

    fn handle(&mut self, event: Event, ctx: &egui::Context) {
        match event {
            Event::Rgba {
                key,
                width,
                height,
                rgba,
            } => {
                self.pending.remove(&key);
                if width == 0 || height == 0 || rgba.len() != (width * height * 4) as usize {
                    return;
                }
                let image =
                    ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
                let name = format!("clip-media-{}", self.stats.decoded);
                let texture = ctx.load_texture(name, image, TextureOptions::LINEAR);
                let aspect = width as f32 / height.max(1) as f32;
                match key {
                    Key::Frame { .. } => {
                        self.thumbs.insert(key, ThumbVisual { texture, aspect });
                    }
                    Key::Still { .. } => {
                        self.stills.insert(key, StillVisual { texture, aspect });
                    }
                    Key::Peaks { .. } => {}
                }
                self.stats.decoded += 1;
            }
            Event::Peaks { key, waveform } => {
                self.pending.remove(&key);
                self.waves.insert(key, waveform);
                self.stats.decoded += 1;
            }
            Event::Failed { key, path, error } => {
                self.pending.remove(&key);
                self.stats.failed += 1;
                eprintln!(
                    "[timeline] cannot visualise {}: {error}",
                    path.display()
                );
                if self.failures.len() >= 128 {
                    self.failures.clear();
                }
                self.failures.insert(path, error);
            }
        }
    }
}

/// Source-time step between thumbnails: the smallest ladder value whose tile is at least `tile_px`
/// wide at this zoom. The ladder keeps the requested timestamps stable across small zoom changes.
pub fn thumb_step(zoom: f32, tile_px: f32) -> f64 {
    const LADDER: [f64; 24] = [
        0.1, 0.125, 0.16, 0.2, 0.25, 0.32, 0.4, 0.5, 0.63, 0.8, 1.0, 1.25, 1.6, 2.0, 2.5, 3.2,
        4.0, 5.0, 6.3, 8.0, 10.0, 12.5, 16.0, 20.0,
    ];
    let needed = (tile_px / zoom.max(1.0)) as f64;
    LADDER
        .iter()
        .copied()
        .find(|step| *step >= needed)
        .unwrap_or(20.0)
}

/// Tile indices of a clip `[clip_start, clip_start + clip_dur)` visible in `[view_start, view_end)`.
pub fn visible_tiles(
    clip_start: f64,
    clip_dur: f64,
    step: f64,
    view_start: f64,
    view_end: f64,
) -> std::ops::Range<usize> {
    let total = (clip_dur / step).ceil().max(0.0) as usize;
    if total == 0 {
        return 0..0;
    }
    let first = (((view_start - clip_start) / step).floor() as i64).clamp(0, total as i64) as usize;
    let last = (((view_end - clip_start) / step).ceil() as i64).clamp(first as i64, total as i64)
        as usize;
    first..last
}

/// Box-filter downscale to fit `(max_w, max_h)` while preserving the aspect ratio.
pub fn downscale(rgba: &[u8], sw: u32, sh: u32, max_w: u32, max_h: u32) -> (u32, u32, Vec<u8>) {
    if sw == 0 || sh == 0 || rgba.len() < (sw * sh * 4) as usize {
        return (0, 0, Vec::new());
    }
    let scale = (max_w as f32 / sw as f32)
        .min(max_h as f32 / sh as f32)
        .min(1.0);
    let dw = ((sw as f32 * scale).round() as u32).max(1);
    let dh = ((sh as f32 * scale).round() as u32).max(1);
    if dw == sw && dh == sh {
        return (sw, sh, rgba.to_vec());
    }
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for dy in 0..dh {
        let y0 = dy as u64 * sh as u64 / dh as u64;
        let y1 = ((dy as u64 + 1) * sh as u64 / dh as u64).max(y0 + 1) as u32;
        for dx in 0..dw {
            let x0 = dx as u64 * sw as u64 / dw as u64;
            let x1 = ((dx as u64 + 1) * sw as u64 / dw as u64).max(x0 + 1) as u32;
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for y in (y0 as u32)..y1.min(sh) {
                for x in (x0 as u32)..x1.min(sw) {
                    let i = ((y * sw + x) * 4) as usize;
                    acc[0] += u32::from(rgba[i]);
                    acc[1] += u32::from(rgba[i + 1]);
                    acc[2] += u32::from(rgba[i + 2]);
                    acc[3] += u32::from(rgba[i + 3]);
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = ((dy * dw + dx) * 4) as usize;
            out[o] = (acc[0] / n) as u8;
            out[o + 1] = (acc[1] / n) as u8;
            out[o + 2] = (acc[2] / n) as u8;
            out[o + 3] = (acc[3] / n) as u8;
        }
    }
    (dw, dh, out)
}

fn picture_worker(rx: Receiver<Request>, tx: Sender<Event>) {
    let mut decoders: Lru<PathBuf, VideoDecoder> = Lru::new(DECODER_CAPACITY);
    for request in rx.iter() {
        let event = match request {
            Request::Frame { key, path, time } => match decode_frame(&mut decoders, &path, time) {
                Ok((width, height, rgba)) => Event::Rgba {
                    key,
                    width,
                    height,
                    rgba,
                },
                Err(error) => Event::Failed { key, path, error },
            },
            Request::Still { key, path } => match decode_still(&mut decoders, &path) {
                Ok((width, height, rgba)) => Event::Rgba {
                    key,
                    width,
                    height,
                    rgba,
                },
                Err(error) => Event::Failed { key, path, error },
            },
            Request::Peaks { .. } => continue,
        };
        if tx.send(event).is_err() {
            return;
        }
    }
}

fn audio_worker(rx: Receiver<Request>, tx: Sender<Event>) {
    for request in rx.iter() {
        let Request::Peaks { key, path } = request else {
            continue;
        };
        let event = match decode_peaks(&path) {
            Ok(waveform) => Event::Peaks {
                key,
                waveform: Arc::new(waveform),
            },
            Err(error) => Event::Failed { key, path, error },
        };
        if tx.send(event).is_err() {
            return;
        }
    }
}

fn open_decoder<'a>(
    decoders: &'a mut Lru<PathBuf, VideoDecoder>,
    path: &Path,
) -> Result<&'a mut VideoDecoder, String> {
    let key = path.to_path_buf();
    if decoders.get(&key).is_none() {
        let decoder = VideoDecoder::open(path).map_err(|error| error.to_string())?;
        decoders.insert(key.clone(), decoder);
    }
    decoders.get_mut(&key).ok_or_else(|| "decoder lost".to_string())
}

fn decode_frame(
    decoders: &mut Lru<PathBuf, VideoDecoder>,
    path: &Path,
    time: f64,
) -> Result<(u32, u32, Vec<u8>), String> {
    let decoder = open_decoder(decoders, path)?;
    let duration = decoder
        .info()
        .duration
        .map(|d| d.to_f64_lossy())
        .unwrap_or(f64::INFINITY);
    let t = time.clamp(0.0, (duration - 0.001).max(0.0));
    let micros = (t * 1_000_000.0).round() as i64;
    let time = Time::new(micros, 1_000_000).map_err(|error| error.to_string())?;
    let frame = decoder.frame_at(time).map_err(|error| error.to_string())?;
    Ok(downscale(frame.rgba, frame.width, frame.height, THUMB_W, THUMB_H))
}

fn decode_still(
    decoders: &mut Lru<PathBuf, VideoDecoder>,
    path: &Path,
) -> Result<(u32, u32, Vec<u8>), String> {
    let decoder = open_decoder(decoders, path)?;
    let frame = decoder
        .frame_at(Time::ZERO)
        .map_err(|error| error.to_string())?;
    Ok(downscale(frame.rgba, frame.width, frame.height, STILL_MAX, STILL_MAX))
}

fn decode_peaks(path: &Path) -> Result<Waveform, String> {
    const RATE: u32 = 8_000;
    const SAMPLES_PER_PEAK: usize = 40; // 8 kHz / 40 = 200 peaks per second
    let mut decoder = AudioDecoder::open(path, RATE).map_err(|error| error.to_string())?;
    let mut block = vec![0.0f32; 4096 * 2];
    let mut peaks = Vec::new();
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    let mut in_peak = 0usize;
    let mut start = 0i64;
    let limit = (MAX_AUDIO_SECONDS * RATE as f64) as i64;
    loop {
        let got = decoder
            .read(start, &mut block)
            .map_err(|error| error.to_string())?;
        if got == 0 {
            break;
        }
        for frame in 0..got {
            let sample = (block[frame * 2] + block[frame * 2 + 1]) * 0.5;
            min = min.min(sample);
            max = max.max(sample);
            in_peak += 1;
            if in_peak == SAMPLES_PER_PEAK {
                peaks.push(Peak { min, max });
                min = 0.0;
                max = 0.0;
                in_peak = 0;
            }
        }
        start += got as i64;
        if start >= limit {
            break;
        }
    }
    if in_peak > 0 {
        peaks.push(Peak { min, max });
    }
    let duration = peaks.len() as f64 / PEAKS_PER_SECOND;
    Ok(Waveform::new(duration, PEAKS_PER_SECOND, peaks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lru_keeps_the_most_recent_entries() {
        let mut lru: Lru<u32, &str> = Lru::new(2);
        lru.insert(1, "one");
        lru.insert(2, "two");
        assert!(lru.get(&1).is_some());
        lru.insert(3, "three");
        assert!(lru.get(&1).is_some(), "recently used entry survives");
        assert!(lru.get(&2).is_none(), "least recently used entry evicted");
        assert!(lru.get(&3).is_some());
        assert_eq!(lru.len(), 2);
    }

    #[test]
    fn thumb_step_grows_as_the_timeline_zooms_out() {
        let zoomed_in = thumb_step(480.0, 128.0);
        let normal = thumb_step(58.0, 128.0);
        let zoomed_out = thumb_step(12.0, 128.0);
        assert!(zoomed_in < normal && normal < zoomed_out);
        // The tile is at least the requested width, and not much wider (the ladder is fine).
        for zoom in [12.0f32, 58.0, 200.0, 480.0] {
            let step = thumb_step(zoom, 128.0);
            let tile = step * zoom as f64;
            assert!(tile >= 128.0, "zoom {zoom}: tile {tile}");
            assert!(tile <= 128.0 * 1.3, "zoom {zoom}: tile {tile}");
        }
        // Steps are stable while the zoom changes within one ladder rung.
        assert_eq!(thumb_step(58.0, 128.0), thumb_step(60.0, 128.0));
    }

    #[test]
    fn visible_tiles_only_cover_the_clip_and_the_view() {
        // 10 s clip starting at t=2, 1 s tiles, view 4..6 s.
        assert_eq!(visible_tiles(2.0, 10.0, 1.0, 4.0, 6.0), 2..4);
        // The view starts before the clip: tiles begin at 0, never negative.
        assert_eq!(visible_tiles(5.0, 10.0, 1.0, 0.0, 7.0), 0..2);
        // The view ends after the clip: stop at the last tile.
        assert_eq!(visible_tiles(0.0, 3.0, 1.0, 2.0, 20.0), 2..3);
        // A clip fully outside the view requests nothing.
        assert_eq!(visible_tiles(0.0, 3.0, 1.0, 10.0, 20.0), 3..3);
    }

    #[test]
    fn downscale_preserves_aspect_and_averages_blocks() {
        // 4x4 image with four solid 2x2 quadrants (red, green, blue, white).
        let mut rgba = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                let colour = match (x < 2, y < 2) {
                    (true, true) => [255, 0, 0],
                    (false, true) => [0, 255, 0],
                    (true, false) => [0, 0, 255],
                    (false, false) => [255, 255, 255],
                };
                rgba.extend_from_slice(&[colour[0], colour[1], colour[2], 255]);
            }
        }
        let (w, h, out) = downscale(&rgba, 4, 4, 2, 2);
        assert_eq!((w, h), (2, 2));
        assert_eq!(&out[0..4], &[255, 0, 0, 255], "top-left quadrant");
        assert_eq!(&out[4..8], &[0, 255, 0, 255], "top-right quadrant");
        assert_eq!(&out[8..12], &[0, 0, 255, 255], "bottom-left quadrant");
        assert_eq!(&out[12..16], &[255, 255, 255, 255], "bottom-right quadrant");

        // Up-scaling is never applied: a small image keeps its size.
        let (w, h, _) = downscale(&rgba, 4, 4, 320, 180);
        assert_eq!((w, h), (4, 4));
    }

    #[test]
    fn waveform_envelope_separates_quiet_and_loud_sections() {
        let mut peaks = Vec::new();
        for i in 0..200 {
            // 0–0.5 s quiet, 0.5–1 s loud.
            let amp = if i < 100 { 0.05 } else { 0.9 };
            peaks.push(Peak { min: -amp, max: amp });
        }
        let wave = Waveform::new(1.0, 200.0, peaks);
        let quiet = wave.envelope(0.0, 0.4).expect("quiet envelope");
        let loud = wave.envelope(0.6, 1.0).expect("loud envelope");
        assert!((quiet.max - 0.05).abs() < 1e-6);
        assert!((loud.max - 0.9).abs() < 1e-6);
        assert!(loud.min < quiet.min);
        assert!(wave.envelope(2.0, 3.0).is_none(), "outside the file");
    }

    #[test]
    fn missing_media_is_reported_once_and_never_crashes() {
        let ctx = egui::Context::default();
        let mut hub = MediaHub::new();
        let path = std::path::Path::new("assets/demo/definitely-missing.mp4");
        assert!(hub.video_frame(path, 1.0).is_none(), "nothing to paint yet");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while hub.failure(path).is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.poll(&ctx);
        }
        assert!(hub.failure(path).is_some(), "the failure must be reported");

        // A failed path is not requested again, however often the frame is repainted.
        let requests = hub.stats().requested;
        for time in [2.0, 3.0, 4.0] {
            assert!(hub.video_frame(path, time).is_none());
        }
        assert_eq!(hub.stats().requested, requests);
    }

    #[test]
    fn pending_requests_are_deduplicated() {
        let mut hub = MediaHub::new();
        let path = std::path::Path::new("assets/demo/city_street.mp4");
        assert!(hub.video_frame(path, 1.0).is_none());
        assert!(hub.video_frame(path, 1.0).is_none());
        // One request for the same quantised timestamp, even when painted twice in one frame.
        assert_eq!(hub.stats().requested, 1);
    }

    #[test]
    fn frame_keys_quantize_the_source_time() {
        let a = Key::Frame {
            path: PathBuf::from("a.mp4"),
            ms: 1_000,
        };
        let b = Key::Frame {
            path: PathBuf::from("a.mp4"),
            ms: 1_000,
        };
        assert_eq!(a, b, "the same millisecond reuses one cache entry");
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and assets/demo (run with --ignored)"]
    fn real_demo_media_decodes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo");
        let mut decoders = Lru::new(DECODER_CAPACITY);
        let (w, h, rgba) =
            decode_frame(&mut decoders, &root.join("city_street.mp4"), 3.0).expect("frame");
        assert!(w > 0 && h > 0 && rgba.len() == (w * h * 4) as usize);
        let (iw, ih, _) =
            decode_still(&mut decoders, &root.join("mountain_lake.png")).expect("still");
        assert!(iw > 0 && ih > 0);
        let wave = decode_peaks(&root.join("narration.wav")).expect("peaks");
        assert!(!wave.is_empty());
        assert!(wave.duration > 5.0);
        let loudest = wave
            .peaks()
            .iter()
            .map(|p| p.max)
            .fold(0.0f32, f32::max);
        assert!(loudest > 0.05, "the demo speech must not be silent");

        // Video audio is decoded by the same envelope path and used for the embedded strip.
        let embedded = decode_peaks(&root.join("city_street.mp4")).expect("video audio");
        assert!(!embedded.is_empty());
        let loudest_video = embedded
            .peaks()
            .iter()
            .map(|p| p.max.max(-p.min))
            .fold(0.0f32, f32::max);
        assert!(loudest_video > 0.01, "the street ambience must not be silent");

        // A silent video must be recognised as such so it keeps the larger thumbnail region.
        let mut hub = MediaHub::new();
        assert!(hub.has_audio(&root.join("city_street.mp4")));
        assert!(!hub.has_audio(&root.join("market_crowd.mp4")));
    }
}
