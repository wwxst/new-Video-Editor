//! Asynchronous media visuals for timeline clips: decoded video-frame thumbnails, decoded still
//! images and audio waveform peaks.
//!
//! Decoding runs on two worker threads (one for pictures, one for audio) so the UI loop never
//! blocks on a file. This module owns the requests, the bounded LRU caches and the texture uploads:
//! the painting code only ever asks "is this frame decoded yet?" and draws what is in the cache.

use arc_core::time::{FrameRate, Round, Time};
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
/// Waveform resolutions, from fine to coarse, in envelope samples per source second.
const WAVE_LEVELS: [f64; 3] = [4_000.0, 1_000.0, 200.0];
/// Never decode more than this much of one audio file (seconds).
const MAX_AUDIO_SECONDS: f64 = 120.0;
/// Decoders kept open between thumbnail requests.
const DECODER_CAPACITY: usize = 3;
/// Textures kept for video thumbnails (bounded GPU memory).
const THUMB_CAPACITY: usize = 128;
const STILL_CAPACITY: usize = 8;
const WAVE_CAPACITY: usize = 6;
/// New decode requests accepted per UI frame, so a zoom-out burst cannot flood the worker.
const MAX_REQUESTS_PER_FRAME: u32 = 6;
/// Total in-flight requests; keeps the worker queue bounded when decoding cannot keep up.
const MAX_PENDING: usize = 24;

/// One min/max envelope sample of decoded audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    pub min: f32,
    pub max: f32,
}

/// One resolution of a waveform envelope.
#[derive(Debug, Clone)]
pub struct WaveLevel {
    /// Envelope samples per second.
    pub peaks_per_second: f64,
    peaks: Vec<Peak>,
}

impl WaveLevel {
    /// Min/max of the source-time range `[t0, t1)`, clamped to the level; `None` outside it.
    fn envelope(&self, t0: f64, t1: f64, duration: f64) -> Option<Peak> {
        if self.peaks.is_empty() {
            return None;
        }
        let pps = self.peaks_per_second.max(1.0);
        if t1 <= 0.0 || t0 >= duration {
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

    fn bytes(&self) -> usize {
        self.peaks.len() * std::mem::size_of::<Peak>()
    }
}

/// Multi-resolution waveform envelope of one audio file.
///
/// Audio is decoded once at 8 kHz and reduced to 4000 / 1000 / 200 peaks per second in the same
/// pass. The painter picks the finest level whose resolution is at least the current pixel density,
/// so zooming in reveals real detail without re-decoding the file or inventing samples.
#[derive(Debug, Clone)]
pub struct Waveform {
    /// Duration covered by the envelope, in seconds.
    pub duration: f64,
    /// Fine-to-coarse levels.
    levels: Vec<WaveLevel>,
}

impl Waveform {
    /// Single-level waveform (tests and synthetic sources).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(duration: f64, peaks_per_second: f64, peaks: Vec<Peak>) -> Waveform {
        Waveform {
            duration,
            levels: vec![WaveLevel {
                peaks_per_second,
                peaks,
            }],
        }
    }

    pub fn from_levels(duration: f64, levels: Vec<WaveLevel>) -> Waveform {
        Waveform { duration, levels }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn peaks(&self) -> &[Peak] {
        self.levels
            .last()
            .map(|level| level.peaks.as_slice())
            .unwrap_or(&[])
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.levels.iter().all(|level| level.peaks.is_empty())
    }

    /// Levels, fine to coarse.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn levels(&self) -> &[WaveLevel] {
        &self.levels
    }

    /// Min/max of the source-time range at the finest available resolution.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn envelope(&self, t0: f64, t1: f64) -> Option<Peak> {
        self.levels
            .first()
            .and_then(|level| level.envelope(t0, t1, self.duration))
    }

    /// Min/max of the source-time range using the finest level that is at least as dense as the
    /// painter's pixel density. Falls back to the finest level at extreme zoom.
    pub fn envelope_at(&self, t0: f64, t1: f64, px_per_sec: f64) -> Option<Peak> {
        let level = self
            .levels
            .iter()
            .rev()
            .find(|level| level.peaks_per_second >= px_per_sec)
            .unwrap_or_else(|| self.levels.first().expect("at least one level"));
        level.envelope(t0, t1, self.duration)
    }

    /// Resolution selected for a pixel density, for tests and diagnostics.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn level_for_px(&self, px_per_sec: f64) -> f64 {
        self.levels
            .iter()
            .rev()
            .find(|level| level.peaks_per_second >= px_per_sec)
            .or_else(|| self.levels.first())
            .map(|level| level.peaks_per_second)
            .unwrap_or(0.0)
    }

    /// Bytes held by the peak buffers (all levels).
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(WaveLevel::bytes).sum()
    }
}

/// Decoded still image plus its aspect ratio, ready to paint.
#[derive(Clone)]
pub struct StillVisual {
    pub texture: TextureHandle,
    /// Width / height of the decoded image.
    pub aspect: f32,
}

/// Decoded video thumbnail plus its aspect ratio and actual presentation time, ready to paint.
#[derive(Clone)]
pub struct ThumbVisual {
    pub texture: TextureHandle,
    /// Width / height of the decoded frame.
    pub aspect: f32,
    /// Presentation timestamp of the delivered frame, in microseconds from the stream start.
    #[allow(dead_code)]
    pub pts_us: u64,
}

/// Metadata needed to sample a video without decoding frames on the UI thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoMeta {
    /// Source stream rate (average rate for VFR); `None` when the container omits it.
    pub rate: Option<FrameRate>,
    pub vfr: bool,
    pub duration: f64,
}

/// Bounded cache usage, reported for tests and performance records.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub thumb_textures: usize,
    pub still_textures: usize,
    pub thumb_bytes: usize,
    pub still_bytes: usize,
    pub wave_files: usize,
    pub wave_bytes: usize,
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

/// Cache identity of one source frame.
///
/// CFR streams use the source frame index, so several project frames that map to the same source
/// frame share one texture (a 24 fps source in a 30 fps project legitimately repeats frames). VFR
/// starts with a requested-time probe (`TimeUs`) and then caches the decoded frame by its actual
/// PTS (`Pts`); the learned display interval makes repeated project frames share that texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FrameKey {
    Index(u64),
    /// Pending VFR probe, before the decoder has told us the source PTS.
    TimeUs(u64),
    /// A decoded VFR frame, keyed by its actual presentation timestamp.
    Pts(u64),
}

/// VFR display intervals kept per source path. Each span is ~40 bytes, so this is ~160 KiB per open
/// source: enough for normal scrubbing, bounded even on a multi-hour VFR file. Evicting an old
/// interval only costs one re-probe; it never changes the decoded frame.
const MAX_VFR_SPANS: usize = 4096;

/// Display intervals learned from decoded VFR frames.
///
/// Each entry is `(display_start, display_end, pts_us)`, using exact [`Time`] values. The decoder's
/// `frame_displayed_at(t)` selects `pts` while `pts <= t + tick < next_pts`, so the display interval
/// is `[pts - tick, next_pts - tick)`, not `[pts, next_pts)`. `end = None` for the last frame.
/// Sorted by display start. Times stay rational: a microsecond-rounded end can be 0.33 µs early and
/// disagree with the decoder at the boundary.
#[derive(Debug, Default)]
struct VfrIntervals {
    spans: Vec<(Time, Option<Time>, u64)>,
}

impl VfrIntervals {
    fn find(&self, t: Time) -> Option<u64> {
        let idx = self.spans.partition_point(|(start, _, _)| *start <= t);
        let (start, end, pts) = *self.spans.get(idx.checked_sub(1)?)?;
        (t >= start && end.is_none_or(|end| t < end)).then_some(pts)
    }

    fn insert(&mut self, pts: Time, next: Option<Time>, tick: Time, pts_us: u64) {
        let start = pts.checked_sub(tick).unwrap_or(Time::ZERO);
        let end = next.and_then(|next| next.checked_sub(tick).ok());
        if let Some(existing) = self
            .spans
            .iter_mut()
            .find(|(_, _, stored)| *stored == pts_us)
        {
            if let Some(end) = end
                && existing.1.is_none_or(|old| end > old)
            {
                existing.1 = Some(end);
            }
            return;
        }
        if self.spans.len() >= MAX_VFR_SPANS {
            // Keep the most recently learned intervals; the oldest can be re-probed on a backward
            // seek. This is metadata only and never invalidates a texture in the LRU.
            self.spans.remove(0);
        }
        self.spans.push((start, end, pts_us));
        self.spans.sort_unstable_by_key(|(start, _, _)| *start);
    }
}

/// Cache key of a decoded asset.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    /// One video frame, at a source frame index (CFR) or microsecond time (VFR).
    Frame { path: PathBuf, frame: FrameKey },
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
        pts_us: u64,
        /// Exact source PTS and next PTS, used to build the VFR display interval.
        pts_time: Time,
        next_time: Option<Time>,
        /// Container timestamp tolerance used by `frame_displayed_at`.
        tick_time: Time,
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

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.map.iter()
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
    /// Source metadata; `None` marks an unreadable file.
    video_meta: HashMap<PathBuf, Option<VideoMeta>>,
    failures: HashMap<PathBuf, String>,
    pending: HashSet<Key>,
    /// VFR paths with one probe decode in flight. While set, other project times in the unknown
    /// gap wait instead of sending duplicate decodes.
    vfr_probe: HashSet<PathBuf>,
    /// Display intervals learned from decoded VFR frames.
    vfr: HashMap<PathBuf, VfrIntervals>,
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
            video_meta: HashMap::new(),
            failures: HashMap::new(),
            pending: HashSet::new(),
            vfr_probe: HashSet::new(),
            vfr: HashMap::new(),
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

    /// Decoded frame for `path` at project time `time`, or `None` while it is decoded.
    ///
    /// CFR sources are keyed by the source frame displayed at `time`; repeated project frames in a
    /// faster sequence therefore share one texture instead of being decoded as fake new frames.
    /// VFR sources probe the requested time once, learn `[pts, next_pts)`, and then key by the
    /// actual PTS, so every project frame inside a long source interval shares one texture.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn video_frame(
        &mut self,
        path: &Path,
        time: f64,
        source_rate: FrameRate,
        vfr: bool,
    ) -> Option<ThumbVisual> {
        let micros = (time.max(0.0) * 1_000_000.0).round() as i64;
        let time = Time::new(micros, 1_000_000).unwrap_or(Time::ZERO);
        self.video_frame_exact(path, time, source_rate, vfr)
    }

    /// Exact-time variant used by the timeline. CFR frame identity is derived with
    /// [`FrameRate::frame_at`] (floor) rather than `floor(t * fps)` in floating point, so 29.97 and
    /// 59.94 frame starts cannot round down into the previous frame.
    fn video_frame_exact(
        &mut self,
        path: &Path,
        time: Time,
        source_rate: FrameRate,
        vfr: bool,
    ) -> Option<ThumbVisual> {
        let request_time = time.to_f64_lossy();
        let request_us = (request_time * 1_000_000.0).round() as u64;
        let (key, request_time) = if vfr {
            // If a decoded interval already covers this project time, its actual PTS is the cache
            // key. Otherwise ask the worker once for this path and wait: a long VFR gap must not
            // launch one decode per project frame.
            match self.vfr.get(path).and_then(|intervals| intervals.find(time)) {
                Some(pts_us) => (
                    Key::Frame {
                        path: path.to_path_buf(),
                        frame: FrameKey::Pts(pts_us),
                    },
                    request_time,
                ),
                None => {
                    if self.vfr_probe.contains(path) {
                        return None;
                    }
                    (
                        Key::Frame {
                            path: path.to_path_buf(),
                            frame: FrameKey::TimeUs(request_us),
                        },
                        request_time,
                    )
                }
            }
        } else {
            let index = source_rate.frame_at(time, Round::Floor).unwrap_or(0);
            (
                Key::Frame {
                    path: path.to_path_buf(),
                    frame: FrameKey::Index(index.max(0) as u64),
                },
                source_rate.time_of_frame(index.max(0)).to_f64_lossy(),
            )
        };
        let probing = matches!(
            &key,
            Key::Frame {
                frame: FrameKey::TimeUs(_),
                ..
            }
        );
        if let Some(visual) = self.thumbs.get(&key) {
            self.stats.cache_hits += 1;
            return Some(visual.clone());
        }
        if self.admit(&key, path) {
            let request = Request::Frame {
                key: key.clone(),
                path: path.to_path_buf(),
                time: request_time,
            };
            if self.video_tx.send(request).is_ok() {
                self.commit(key);
                if probing {
                    self.vfr_probe.insert(path.to_path_buf());
                }
            }
        }
        None
    }

    /// [`video_frame`](Self::video_frame) with the exact project-frame time. The timeline computes
    /// the sample as `FrameRate::time_of_frame`, never as `frame / fps` in floating point.
    pub fn video_frame_at(
        &mut self,
        path: &Path,
        time: Time,
        source_rate: FrameRate,
        vfr: bool,
    ) -> Option<ThumbVisual> {
        self.video_frame_exact(path, time, source_rate, vfr)
    }

    /// Source metadata for a video, read once per path. Opening a decoder reads the header only;
    /// no frame is decoded here and the actual frame work stays on the worker thread.
    pub fn video_meta(&mut self, path: &Path) -> Option<VideoMeta> {
        if let Some(meta) = self.video_meta.get(path) {
            return *meta;
        }
        let meta = VideoDecoder::open(path).ok().map(|decoder| {
            let info = decoder.info();
            let rate = info
                .frame_rate
                .and_then(|ratio| FrameRate::from_ratio(ratio).ok());
            VideoMeta {
                rate,
                vfr: info.vfr,
                duration: info
                    .duration
                    .map(|duration| duration.to_f64_lossy())
                    .unwrap_or(0.0),
            }
        });
        self.video_meta.insert(path.to_path_buf(), meta);
        meta
    }

    /// Source rate plus VFR flag, read from the cached header.
    pub fn video_sampling(&mut self, path: &Path, project_rate: FrameRate) -> (FrameRate, bool) {
        let meta = self.video_meta(path);
        (
            meta.and_then(|meta| meta.rate).unwrap_or(project_rate),
            meta.is_some_and(|meta| meta.vfr),
        )
    }

    /// Bounded cache usage for the performance report.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cache_usage(&self) -> CacheUsage {
        let thumb_bytes = self.thumbs.len() * (THUMB_W * THUMB_H * 4) as usize;
        let still_bytes = self.stills.len() * (STILL_MAX * STILL_MAX * 4) as usize;
        let wave_bytes = self.waves.iter().map(|(_, wave)| wave.bytes()).sum();
        CacheUsage {
            thumb_textures: self.thumbs.len(),
            still_textures: self.stills.len(),
            thumb_bytes,
            still_bytes,
            wave_files: self.waves.len(),
            wave_bytes,
        }
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
        if self.budget == 0 || self.pending.len() >= MAX_PENDING {
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
                pts_us,
                pts_time,
                next_time,
                tick_time,
            } => {
                self.pending.remove(&key);
                if let Key::Frame { path, .. } = &key {
                    self.vfr_probe.remove(path);
                }
                if width == 0 || height == 0 || rgba.len() != (width * height * 4) as usize {
                    return;
                }
                let image =
                    ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
                let name = format!("clip-media-{}", self.stats.decoded);
                let texture = ctx.load_texture(name, image, TextureOptions::LINEAR);
                let aspect = width as f32 / height.max(1) as f32;
                match key {
                    Key::Frame { path, frame } => {
                        // CFR keeps its source-index key. A VFR probe key is replaced by the actual
                        // PTS; the [pts, next) interval lets every project frame inside a long
                        // source gap share this texture.
                        let cache_key = match frame {
                            FrameKey::Index(_) => Key::Frame { path, frame },
                            FrameKey::TimeUs(_) | FrameKey::Pts(_) => {
                                self.vfr
                                    .entry(path.clone())
                                    .or_default()
                                    .insert(pts_time, next_time, tick_time, pts_us);
                                Key::Frame {
                                    path,
                                    frame: FrameKey::Pts(pts_us),
                                }
                            }
                        };
                        self.thumbs.insert(
                            cache_key,
                            ThumbVisual {
                                texture,
                                aspect,
                                pts_us,
                            },
                        );
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
                self.vfr_probe.remove(&path);
                self.stats.failed += 1;
                eprintln!("[timeline] cannot visualise {}: {error}", path.display());
                if self.failures.len() >= 128 {
                    self.failures.clear();
                }
                self.failures.insert(path, error);
            }
        }
    }
}

/// Project-frame step between thumbnails.
///
/// The step is always a whole number of project frames. At extreme zoom it reaches 1, so a tile is
/// one project frame; a lower-rate source repeats the previous source frame where no new frame
/// exists (the cache keys CFR frames by source index, so repeats reuse one texture).
pub fn thumbnail_step_frames(zoom: f32, tile_px: f32, project_fps: f64) -> u64 {
    let frame_px = zoom.max(1.0) as f64 / project_fps.max(1.0);
    (tile_px as f64 / frame_px).ceil().max(1.0) as u64
}

/// Seconds covered by `step_frames` project frames.
pub fn thumbnail_step_seconds(step_frames: u64, project_fps: f64) -> f64 {
    step_frames.max(1) as f64 / project_fps.max(1.0)
}

/// Exact start time of the project frame that starts thumbnail tile `tile`.
///
/// A 1f tile samples the project frame's **start**, the same time `plan_frame` uses for preview and
/// export. Sampling the middle of the frame (the old behaviour) disagreed with a lower-rate source:
/// at a 30 fps project frame 1, 50 ms shows source frame 1 from a 24 fps source while 33.33 ms shows
/// source frame 0.
pub fn thumbnail_sample_time(frame_rate: FrameRate, tile: u64, step_frames: u64) -> Time {
    let frame = tile.saturating_mul(step_frames.max(1));
    frame_rate.time_of_frame(frame.min(i64::MAX as u64) as i64)
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
    let last =
        (((view_end - clip_start) / step).ceil() as i64).clamp(first as i64, total as i64) as usize;
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
                Ok(decoded) => Event::Rgba {
                    key,
                    width: decoded.width,
                    height: decoded.height,
                    rgba: decoded.rgba,
                    pts_us: decoded.pts_us,
                    pts_time: decoded.pts_time,
                    next_time: decoded.next_time,
                    tick_time: decoded.tick_time,
                },
                Err(error) => Event::Failed { key, path, error },
            },
            Request::Still { key, path } => match decode_still(&mut decoders, &path) {
                Ok((width, height, rgba, pts_us)) => Event::Rgba {
                    key,
                    width,
                    height,
                    rgba,
                    pts_us,
                    pts_time: Time::ZERO,
                    next_time: None,
                    tick_time: Time::ZERO,
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
    decoders
        .get_mut(&key)
        .ok_or_else(|| "decoder lost".to_string())
}

struct DecodedFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    pts_us: u64,
    pts_time: Time,
    next_time: Option<Time>,
    tick_time: Time,
}

fn decode_frame(
    decoders: &mut Lru<PathBuf, VideoDecoder>,
    path: &Path,
    time: f64,
) -> Result<DecodedFrame, String> {
    let decoder = open_decoder(decoders, path)?;
    let duration = decoder
        .info()
        .duration
        .map(|d| d.to_f64_lossy())
        .unwrap_or(f64::INFINITY);
    let t = time.clamp(0.0, (duration - 0.001).max(0.0));
    let micros = (t * 1_000_000.0).round() as i64;
    let time = Time::new(micros, 1_000_000).map_err(|error| error.to_string())?;
    let tick_time = Time::new(
        decoder.info().time_base.num(),
        i64::from(decoder.info().time_base.den()),
    )
    .map_err(|error| error.to_string())?;
    let frame = decoder
        .frame_displayed_at(time)
        .map_err(|error| error.to_string())?;
    let pts_us = (frame.time.to_f64_lossy() * 1_000_000.0).round().max(0.0) as u64;
    let (width, height, rgba) = downscale(frame.rgba, frame.width, frame.height, THUMB_W, THUMB_H);
    Ok(DecodedFrame {
        width,
        height,
        rgba,
        pts_us,
        pts_time: frame.time,
        next_time: frame.next_time,
        tick_time,
    })
}

fn decode_still(
    decoders: &mut Lru<PathBuf, VideoDecoder>,
    path: &Path,
) -> Result<(u32, u32, Vec<u8>, u64), String> {
    let decoder = open_decoder(decoders, path)?;
    let frame = decoder
        .frame_at(Time::ZERO)
        .map_err(|error| error.to_string())?;
    let (w, h, rgba) = downscale(frame.rgba, frame.width, frame.height, STILL_MAX, STILL_MAX);
    Ok((w, h, rgba, 0))
}

fn decode_peaks(path: &Path) -> Result<Waveform, String> {
    const RATE: u32 = 8_000;
    /// Two 8 kHz samples per 4000 Hz envelope peak.
    const SAMPLES_PER_FINE_PEAK: usize = 2;
    let mut decoder = AudioDecoder::open(path, RATE).map_err(|error| error.to_string())?;
    let mut block = vec![0.0f32; 4096 * 2];
    let mut fine = Vec::new();
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
            if in_peak == SAMPLES_PER_FINE_PEAK {
                fine.push(Peak { min, max });
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
        fine.push(Peak { min, max });
    }

    let fine_pps = RATE as f64 / SAMPLES_PER_FINE_PEAK as f64;
    let mut levels = Vec::with_capacity(WAVE_LEVELS.len());
    levels.push(WaveLevel {
        peaks_per_second: fine_pps,
        peaks: fine,
    });
    for target_pps in WAVE_LEVELS.iter().copied().skip(1) {
        let previous = levels.last().expect("fine level");
        let factor = (previous.peaks_per_second / target_pps).round().max(1.0) as usize;
        let peaks = downsample_peaks(&previous.peaks, factor);
        levels.push(WaveLevel {
            peaks_per_second: target_pps,
            peaks,
        });
    }
    let duration = levels
        .first()
        .map(|level| level.peaks.len() as f64 / fine_pps)
        .unwrap_or(0.0);
    Ok(Waveform::from_levels(duration, levels))
}

fn downsample_peaks(src: &[Peak], factor: usize) -> Vec<Peak> {
    if factor <= 1 {
        return src.to_vec();
    }
    src.chunks(factor)
        .map(|chunk| {
            let mut min = f32::INFINITY;
            let mut max = f32::NEG_INFINITY;
            for peak in chunk {
                min = min.min(peak.min);
                max = max.max(peak.max);
            }
            Peak { min, max }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Request one thumbnail through the real worker/cache and return its actual source PTS.
    fn settle_thumbnail(
        hub: &mut MediaHub,
        ctx: &egui::Context,
        path: &Path,
        time: Time,
        rate: FrameRate,
        vfr: bool,
    ) -> u64 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(visual) = hub.video_frame_at(path, time, rate, vfr) {
                return visual.pts_us;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "thumbnail for {time} did not settle"
            );
            hub.poll(ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

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
    fn cache_usage_stays_within_the_bounded_budgets() {
        let hub = MediaHub::new();
        let usage = hub.cache_usage();
        assert_eq!(usage.thumb_textures, 0);
        assert!(usage.thumb_bytes <= THUMB_CAPACITY * (THUMB_W * THUMB_H * 4) as usize);
        assert!(usage.wave_files <= WAVE_CAPACITY);
        assert!(usage.wave_bytes <= WAVE_CAPACITY * 6 * 1024 * 1024);
    }

    #[test]
    fn thumbnail_tiles_step_by_whole_project_frames() {
        // 24 fps project: at 4800 px/s the step reaches 1 (200 px), regardless of source rate.
        assert_eq!(thumbnail_step_frames(4_800.0, 128.0, 24.0), 1);
        let normal = thumbnail_step_frames(58.0, 128.0, 24.0);
        let wide = thumbnail_step_frames(12.0, 128.0, 24.0);
        assert!(normal > 1 && wide > normal);
        for zoom in [12.0f32, 58.0, 480.0, 4_800.0] {
            let step = thumbnail_step_frames(zoom, 128.0, 24.0);
            let tile = step as f64 / 24.0 * zoom as f64;
            assert!(tile >= 128.0 - 0.01, "zoom {zoom}: tile {tile}");
            if zoom < 4_800.0 {
                assert!(
                    tile <= 128.0 + zoom as f64 / 24.0,
                    "zoom {zoom}: tile {tile}"
                );
            }
        }
        // A 30 fps project reaches per-frame tiles at 6000 px/s.
        assert_eq!(thumbnail_step_frames(6_000.0, 128.0, 30.0), 1);
        assert!(thumbnail_step_frames(600.0, 128.0, 30.0) > 1);

        // The zoom ceiling is 200 px per project frame for every supported rate; at that point a
        // 128 px tile is always exactly one project frame, never a fractional/duplicated step.
        for fps in [24.0, 25.0, 30.0, 30_000.0 / 1001.0, 60_000.0 / 1001.0] {
            assert_eq!(
                thumbnail_step_frames(200.0 * fps as f32, 128.0, fps),
                1,
                "{fps} fps at the zoom ceiling"
            );
        }
    }

    #[test]
    fn thumbnail_sample_time_is_the_exact_project_frame_start() {
        for rate in [
            FrameRate::Fps24,
            FrameRate::Fps25,
            FrameRate::Fps30,
            FrameRate::Fps29_97,
            FrameRate::Fps59_94,
        ] {
            let sample = thumbnail_sample_time(rate, 1, 1);
            assert_eq!(
                sample,
                rate.time_of_frame(1),
                "{rate:?} must sample the project frame start"
            );
            assert!(
                sample < rate.time_of_frame(2),
                "{rate:?} samples before the next frame"
            );
        }
        // The old midpoint formula sampled project frame 1 at 50 ms (the next 24 fps source frame);
        // the start is 33.33 ms.
        assert_eq!(
            thumbnail_sample_time(FrameRate::Fps30, 1, 1),
            Time::new(1, 30).expect("1/30")
        );
    }

    #[test]
    fn vfr_interval_metadata_is_half_open_and_bounded() {
        let us = |n: i64| Time::new(n, 1_000_000).expect("microseconds");
        let mut intervals = VfrIntervals::default();
        intervals.insert(us(300_000), Some(us(533_333)), Time::ZERO, 300_000);
        assert_eq!(intervals.find(us(300_000)), Some(300_000));
        assert_eq!(intervals.find(us(533_332)), Some(300_000));
        assert_eq!(
            intervals.find(us(533_333)),
            None,
            "the end belongs to the next frame"
        );
        intervals.insert(us(533_333), None, Time::ZERO, 533_333);
        assert_eq!(intervals.find(us(9_999_999)), Some(533_333));

        // The decoder adds one container tick to the requested time; the cache interval is shifted
        // by the same tick so the two select the same frame at the boundary.
        let mut biased = VfrIntervals::default();
        biased.insert(us(300_000), Some(us(533_333)), us(1_000), 300_000);
        assert_eq!(biased.find(us(299_000)), Some(300_000));
        assert_eq!(biased.find(us(298_999)), None);
        assert_eq!(biased.find(us(532_332)), Some(300_000));
        assert_eq!(
            biased.find(us(532_333)),
            None,
            "one tick before the next PTS the decoder has already advanced"
        );
        for i in 0..MAX_VFR_SPANS + 8 {
            let start = 1_000_000 + i as i64 * 1_000;
            intervals.insert(us(start), Some(us(start + 1_000)), Time::ZERO, start as u64);
        }
        assert_eq!(
            intervals.spans.len(),
            MAX_VFR_SPANS,
            "VFR interval metadata must stay bounded"
        );
        assert!(
            intervals
                .find(us(1_000_000 + (MAX_VFR_SPANS as i64 + 7) * 1_000))
                .is_some(),
            "newest intervals survive the cap"
        );
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
            // 0-.5 s quiet, 0.5- s loud.
            let amp = if i < 100 { 0.05 } else { 0.9 };
            peaks.push(Peak {
                min: -amp,
                max: amp,
            });
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
        assert!(
            hub.video_frame(path, 1.0, FrameRate::Fps24, false)
                .is_none(),
            "nothing to paint yet"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while hub.failure(path).is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.poll(&ctx);
        }
        assert!(hub.failure(path).is_some(), "the failure must be reported");

        // A failed path is not requested again, however often the frame is repainted.
        let requests = hub.stats().requested;
        for time in [2.0, 3.0, 4.0] {
            assert!(
                hub.video_frame(path, time, FrameRate::Fps24, false)
                    .is_none()
            );
        }
        assert_eq!(hub.stats().requested, requests);
    }

    #[test]
    fn pending_requests_are_deduplicated() {
        let mut hub = MediaHub::new();
        let path = std::path::Path::new("assets/demo/city_street.mp4");
        assert!(
            hub.video_frame(path, 1.0, FrameRate::Fps24, false)
                .is_none()
        );
        assert!(
            hub.video_frame(path, 1.0, FrameRate::Fps24, false)
                .is_none()
        );
        // One request for the same quantised timestamp, even when painted twice in one frame.
        assert_eq!(hub.stats().requested, 1);
    }

    #[test]
    fn waveform_picks_a_resolution_for_the_zoom_and_keeps_silence_flat() {
        let mut levels = Vec::new();
        for (pps, amp) in [(4_000.0, 0.9), (1_000.0, 0.6), (200.0, 0.3)] {
            let count = (pps * 1.0) as usize;
            levels.push(WaveLevel {
                peaks_per_second: pps,
                peaks: vec![
                    Peak {
                        min: -amp,
                        max: amp
                    };
                    count
                ],
            });
        }
        let wave = Waveform::from_levels(1.0, levels);
        assert_eq!(wave.levels().len(), 3);
        assert_eq!(wave.level_for_px(12.0), 200.0);
        assert_eq!(wave.level_for_px(480.0), 1_000.0);
        assert_eq!(wave.level_for_px(4_800.0), 4_000.0);
        let low = wave.envelope_at(0.0, 0.1, 12.0).expect("low envelope");
        let high = wave.envelope_at(0.0, 0.1, 4_800.0).expect("high envelope");
        assert!((low.max - 0.3).abs() < 1e-6, "coarse level for low zoom");
        assert!((high.max - 0.9).abs() < 1e-6, "fine level for extreme zoom");
        assert!(wave.bytes() > 0);

        let silent = Waveform::new(1.0, 200.0, vec![Peak { min: 0.0, max: 0.0 }; 200]);
        let peak = silent.envelope_at(0.2, 0.4, 4_800.0).expect("silent range");
        assert_eq!(peak, Peak { min: 0.0, max: 0.0 });
    }

    #[test]
    fn frame_keys_quantize_the_source_time() {
        let a = Key::Frame {
            path: PathBuf::from("a.mp4"),
            frame: FrameKey::Index(24),
        };
        let b = Key::Frame {
            path: PathBuf::from("a.mp4"),
            frame: FrameKey::Index(24),
        };
        let c = Key::Frame {
            path: PathBuf::from("a.mp4"),
            frame: FrameKey::Index(25),
        };
        assert_eq!(a, b, "the same source frame reuses one cache entry");
        assert_ne!(a, c, "adjacent source frames have distinct keys");

        let p = Key::Frame {
            path: PathBuf::from("a.mp4"),
            frame: FrameKey::Pts(300_000),
        };
        let q = Key::Frame {
            path: PathBuf::from("a.mp4"),
            frame: FrameKey::Pts(300_000),
        };
        assert_eq!(
            p, q,
            "VFR textures are keyed by actual PTS, not requested time"
        );
    }

    #[test]
    fn vfr_intervals_cover_a_long_source_gap() {
        let us = |n: i64| Time::new(n, 1_000_000).expect("microseconds");
        let mut intervals = VfrIntervals::default();
        intervals.insert(us(300_000), Some(us(533_333)), Time::ZERO, 300_000);
        assert_eq!(intervals.find(us(300_000)), Some(300_000));
        assert_eq!(intervals.find(us(400_000)), Some(300_000));
        assert_eq!(intervals.find(us(533_332)), Some(300_000));
        assert_eq!(intervals.find(us(533_333)), None);
        intervals.insert(us(533_333), None, Time::ZERO, 533_333);
        assert_eq!(intervals.find(us(9_999_999)), Some(533_333));
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and assets/demo (run with --ignored)"]
    fn extreme_zoom_thumbnail_cache_reuses_textures() {
        let ctx = egui::Context::default();
        let mut hub = MediaHub::new();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/demo/city_street.mp4");
        // Eight adjacent 24 fps source frames, as the 1f zoom level requests them.
        let times: Vec<f64> = (0..8)
            .map(|frame| 1.0 + (frame as f64 + 0.5) / 24.0)
            .collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let mut missing = 0;
            for &time in &times {
                if hub
                    .video_frame(&path, time, FrameRate::Fps24, false)
                    .is_none()
                {
                    missing += 1;
                }
            }
            hub.poll(&ctx);
            if missing == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "thumbnail cache did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let requested = hub.stats().requested;
        assert_eq!(
            requested,
            times.len() as u64,
            "one request per source frame"
        );
        for &time in &times {
            assert!(
                hub.video_frame(&path, time, FrameRate::Fps24, false)
                    .is_some()
            );
        }
        assert_eq!(
            hub.stats().requested,
            requested,
            "cached frames must not be requested again"
        );
        let usage = hub.cache_usage();
        assert!(usage.thumb_textures >= times.len());
        assert!(usage.thumb_bytes <= THUMB_CAPACITY * (THUMB_W * THUMB_H * 4) as usize);
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and assets/demo (run with --ignored)"]
    fn project_frame_sampling_maps_repeats_to_source_pts() {
        use arc_core::time::Ratio;
        use arc_media::fixtures::{read_counter, write_counter_video};

        let make = |frames: u32, fps: i64, tag: &str| {
            let path = std::env::temp_dir().join(format!(
                "arcline-counter-{tag}-{}-{fps}.mkv",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            write_counter_video(&path, frames, Ratio::new(fps, 1).unwrap(), 320, 180, false)
                .expect("counter video");
            path
        };

        // A true 30 fps source: every project frame maps to its own source frame.
        let path_30 = make(30, 30, "30fps");
        let mut decoder = VideoDecoder::open(&path_30).expect("open 30");
        for project_frame in 0..30 {
            let t = (project_frame as f64 + 0.5) / 30.0;
            let time = Time::new((t * 1e6).round() as i64, 1_000_000).unwrap();
            let frame = decoder.frame_displayed_at(time).expect("frame");
            let counter = read_counter(frame.rgba, frame.width, frame.height).expect("counter");
            assert_eq!(counter, project_frame, "30 fps source frame index");
            assert!(
                (frame.time.to_f64_lossy() - project_frame as f64 / 30.0).abs() < 0.0011,
                "30 fps PTS {} for project frame {project_frame}",
                frame.time.to_f64_lossy()
            );
        }
        drop(decoder);
        let _ = std::fs::remove_file(&path_30);

        // A 24 fps source inside a 30 fps project: six project frames per second must repeat the
        // previous source frame; they must not be decoded as fabricated new frames.
        let path_24 = make(24, 24, "24fps");
        let mut decoder = VideoDecoder::open(&path_24).expect("open 24");
        let mut counters = Vec::new();
        for project_frame in 0..30 {
            let t = (project_frame as f64 + 0.5) / 30.0;
            let time = Time::new((t * 1e6).round() as i64, 1_000_000).unwrap();
            let frame = decoder.frame_displayed_at(time).expect("frame");
            let counter = read_counter(frame.rgba, frame.width, frame.height).expect("counter");
            counters.push(counter);
            let expected = ((t * 24.0).floor() as u32).min(23);
            assert_eq!(
                counter, expected,
                "24 fps source in 30 fps project at project frame {project_frame}"
            );
        }
        assert_eq!(
            counters
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            24,
            "a 24 fps source contributes exactly 24 distinct source frames per second"
        );
        assert!(
            counters.windows(2).any(|pair| pair[0] == pair[1]),
            "a 24 fps source in a 30 fps project must repeat at least one source frame"
        );
        drop(decoder);
        let _ = std::fs::remove_file(&path_24);
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and generated fixture support (run with --ignored)"]
    fn vfr_counter_maps_every_project_frame_to_expected_pts() {
        use arc_core::time::Ratio;
        use arc_media::fixtures::{read_counter, vfr_counter_pts_us, write_vfr_counter_video};

        let path =
            std::env::temp_dir().join(format!("arcline-vfr-counter-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let base_rate = Ratio::new(30, 1).unwrap();
        write_vfr_counter_video(&path, 30, base_rate, 320, 180, 10, 0.2).expect("VFR fixture");

        let mut decoder = VideoDecoder::open(&path).expect("open VFR");
        assert!(decoder.info().vfr, "fixture must be flagged VFR");
        let expected = vfr_counter_pts_us(30, base_rate, 10, 0.2);
        // The same 30 fps VFR source is sampled by several project rates. The source PTS table is
        // fixed; only the project-frame times move.
        for project_fps in [24.0_f64, 25.0, 30.0, 30_000.0 / 1001.0, 60_000.0 / 1001.0] {
            let mut seen = Vec::new();
            let mut seen_pts = Vec::new();
            let frames = project_fps.ceil() as u32;
            for project_frame in 0..frames {
                let t = (project_frame as f64 + 0.5) / project_fps;
                let time = Time::new((t * 1e6).round() as i64, 1_000_000).unwrap();
                let frame = decoder.frame_displayed_at(time).expect("frame");
                let pts_us = (frame.time.to_f64_lossy() * 1e6).round() as u64;
                let expected_idx = expected
                    .iter()
                    .rposition(|pts| *pts <= (t * 1e6).round() as u64 + 1000)
                    .expect("a source frame at this time");
                assert_eq!(
                    pts_us, expected[expected_idx],
                    "actual PTS for {project_fps} fps project frame {project_frame} must match the fixture"
                );
                let counter = read_counter(frame.rgba, frame.width, frame.height).expect("counter");
                assert_eq!(
                    counter, expected_idx as u32,
                    "{project_fps} fps project frame {project_frame} must show source frame {expected_idx}"
                );
                seen.push(counter);
                seen_pts.push(pts_us);
            }
            assert!(
                seen.windows(2).any(|pair| pair[0] == pair[1]),
                "{project_fps} fps: the 0.2 s VFR gap must repeat a source frame"
            );
            assert!(
                seen_pts.windows(2).any(|pair| pair[0] == pair[1]),
                "{project_fps} fps: the repeated source frame must repeat the same actual PTS"
            );
        }
        drop(decoder);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and generated fixture support (run with --ignored)"]
    fn vfr_thumbnail_cache_shares_one_texture_per_source_pts() {
        use arc_core::time::Ratio;
        use arc_media::fixtures::write_vfr_counter_video;

        let path =
            std::env::temp_dir().join(format!("arcline-vfr-cache-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&path);
        write_vfr_counter_video(&path, 30, Ratio::new(30, 1).unwrap(), 320, 180, 10, 0.2)
            .expect("VFR fixture");

        let ctx = egui::Context::default();
        let mut hub = MediaHub::new();
        let times: Vec<f64> = (0..30).map(|frame| (frame as f64 + 0.5) / 30.0).collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut first_poll = true;
        loop {
            let mut missing = 0;
            for &time in &times {
                // Until the first probe lands there is no interval; this returns None without
                // sending a second request for the same path.
                if hub
                    .video_frame(&path, time, FrameRate::Fps30, true)
                    .is_none()
                {
                    missing += 1;
                }
                assert!(
                    hub.stats().pending <= MAX_PENDING,
                    "VFR probe must stay bounded"
                );
            }
            if first_poll {
                assert_eq!(
                    hub.stats().decoded,
                    0,
                    "no decode may be counted before the UI-thread poll uploads it"
                );
                first_poll = false;
            }
            hub.poll(&ctx);
            if missing == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "VFR thumbnail cache did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Project frames 0..29 at 30 fps only reach source frame 19 before the 0.2 s gap pushes
        // frame 20 to 1.067 s; 20 source frames therefore cover 30 thumbnail tiles.
        assert_eq!(
            hub.stats().requested,
            20,
            "one decode per distinct source PTS"
        );
        assert_eq!(
            hub.cache_usage().thumb_textures,
            20,
            "the repeated project frames in the VFR gap share one texture"
        );
        assert!(hub.stats().cache_hits >= 10, "repeated tiles hit the cache");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and generated fixtures (run with --ignored)"]
    fn thumbnail_source_frame_matches_displayed_at_cfr_rates_and_reverse() {
        use arc_media::fixtures::{read_counter, write_counter_video};

        // Same-rate projects plus the 24-in-30 mismatch that originally exposed the midpoint bug.
        let cases = [
            (FrameRate::Fps24, FrameRate::Fps24),
            (FrameRate::Fps25, FrameRate::Fps25),
            (FrameRate::Fps30, FrameRate::Fps30),
            (FrameRate::Fps29_97, FrameRate::Fps29_97),
            (FrameRate::Fps59_94, FrameRate::Fps59_94),
            (FrameRate::Fps30, FrameRate::Fps24),
        ];
        for (project_rate, source_rate) in cases {
            let source_frames = source_rate.nominal() as u32;
            let project_frames = project_rate.nominal() as u32;
            let path = std::env::temp_dir().join(format!(
                "arcline-thumb-{}-{}-{}.mkv",
                project_rate.label(),
                source_rate.label(),
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            write_counter_video(&path, source_frames, source_rate.rate(), 320, 180, false)
                .expect("counter fixture");
            let mut decoder = VideoDecoder::open(&path).expect("open counter");
            let ctx = egui::Context::default();
            let mut hub = MediaHub::new();
            let mut order = vec![
                0,
                1,
                2,
                project_frames / 2,
                project_frames.saturating_sub(2),
                project_frames - 1,
            ];
            order.sort_unstable();
            order.dedup();
            for pass in 0..2 {
                let frames: Vec<u32> = if pass == 0 {
                    order.clone()
                } else {
                    order.iter().rev().copied().collect()
                };
                for project_frame in frames {
                    let sample = thumbnail_sample_time(project_rate, project_frame as u64, 1);
                    let expected = decoder.frame_displayed_at(sample).expect("displayed frame");
                    let expected_pts =
                        (expected.time.to_f64_lossy() * 1e6).round().max(0.0) as u64;
                    let expected_counter =
                        read_counter(expected.rgba, expected.width, expected.height)
                            .expect("counter");
                    let expected_index = source_rate.frame_at(sample, Round::Floor).unwrap() as u32;
                    assert_eq!(
                        expected_counter, expected_index,
                        "{project_rate:?} project / {source_rate:?} source, frame {project_frame}: counter must be the displayed source frame"
                    );
                    let thumb_pts = settle_thumbnail(
                        &mut hub,
                        &ctx,
                        &path,
                        sample,
                        source_rate,
                        false,
                    );
                    assert_eq!(
                        thumb_pts, expected_pts,
                        "{project_rate:?} project / {source_rate:?} source, frame {project_frame}, pass {pass}"
                    );
                }
            }
            drop(hub);
            drop(decoder);
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and generated fixtures (run with --ignored)"]
    fn thumbnail_source_frame_matches_displayed_at_vfr_boundaries_and_reverse() {
        use arc_core::time::Ratio;
        use arc_media::fixtures::{read_counter, vfr_counter_pts_us, write_vfr_counter_video};

        let path = std::env::temp_dir().join(format!(
            "arcline-vfr-thumb-{}.mp4",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let base_rate = Ratio::new(30, 1).unwrap();
        write_vfr_counter_video(&path, 30, base_rate, 320, 180, 10, 0.2)
            .expect("VFR fixture");
        let expected = vfr_counter_pts_us(30, base_rate, 10, 0.2);
        let mut decoder = VideoDecoder::open(&path).expect("open VFR");
        let ctx = egui::Context::default();
        let mut hub = MediaHub::new();
        // 9 is the frame before the first gap, 10 the first repeated project frame, 16 the first
        // project frame at the next source PTS, 29 the last project frame.
        let milestones = [0u32, 1, 9, 10, 15, 16, 17, 29];
        for pass in 0..2 {
            let frames: Vec<u32> = if pass == 0 {
                milestones.to_vec()
            } else {
                milestones.iter().rev().copied().collect()
            };
            for project_frame in frames {
                let sample = thumbnail_sample_time(FrameRate::Fps30, project_frame as u64, 1);
                let expected_frame = decoder.frame_displayed_at(sample).expect("displayed");
                let expected_pts =
                    (expected_frame.time.to_f64_lossy() * 1e6).round().max(0.0) as u64;
                let expected_counter =
                    read_counter(expected_frame.rgba, expected_frame.width, expected_frame.height)
                        .expect("counter");
                let thumb_pts = settle_thumbnail(
                    &mut hub,
                    &ctx,
                    &path,
                    sample,
                    FrameRate::Fps30,
                    true,
                );
                assert_eq!(
                    thumb_pts, expected_pts,
                    "VFR project frame {project_frame}, pass {pass}"
                );
                assert_eq!(
                    expected_counter as usize,
                    expected
                        .iter()
                        .rposition(|pts| *pts <= (sample.to_f64_lossy() * 1e6).round() as u64 + 1_000)
                        .expect("expected source frame"),
                    "the fixture counter and PTS table must agree"
                );
            }
        }

        // The last source frame has no look-ahead frame; its cache interval is open-ended.
        let tail = Time::new(139, 100).expect("1.39 s");
        let last = decoder.frame_displayed_at(tail).expect("last frame");
        assert_eq!(read_counter(last.rgba, last.width, last.height), Some(29));
        assert_eq!(last.next_time, None, "last frame has no next PTS");

        // The learned interval matches the decoder's one-tick boundary exactly: the cache switches
        // to frame 10 at the same rational time `frame_displayed_at` does.
        let tick = decoder.info().time_base;
        let tick_time =
            Time::new(tick.num(), i64::from(tick.den())).expect("container tick");
        let next_pts = {
            let frame9 = decoder
                .frame_displayed_at(Time::new(300_000, 1_000_000).expect("0.3 s"))
                .expect("frame 9");
            frame9.next_time.expect("next PTS")
        };
        let boundary = next_pts.checked_sub(tick_time).expect("boundary");
        let before = boundary
            .checked_sub(Time::new(1, 1_000_000).expect("1 us"))
            .expect("before");
        let spans = hub.vfr.get(&path).expect("learned intervals");
        assert_eq!(spans.find(before), Some(expected[9]));
        assert_eq!(spans.find(boundary), Some(expected[10]));
        let before_counter = {
            let frame = decoder.frame_displayed_at(before).expect("boundary-1us");
            read_counter(frame.rgba, frame.width, frame.height)
        };
        let at_counter = {
            let frame = decoder.frame_displayed_at(boundary).expect("boundary");
            read_counter(frame.rgba, frame.width, frame.height)
        };
        assert_eq!(before_counter, Some(9));
        assert_eq!(at_counter, Some(10));
        drop(hub);
        drop(decoder);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and assets/demo (run with --ignored)"]
    fn lower_rate_source_repeats_without_extra_decode() {
        use arc_core::time::Ratio;
        use arc_media::fixtures::write_counter_video;

        let path =
            std::env::temp_dir().join(format!("arcline-counter-repeat-{}.mkv", std::process::id()));
        let _ = std::fs::remove_file(&path);
        write_counter_video(&path, 24, Ratio::new(24, 1).unwrap(), 320, 180, false)
            .expect("counter video");

        let ctx = egui::Context::default();
        let mut hub = MediaHub::new();
        // One project frame per tile at a 30 fps project; 30 tiles map to 24 source frames.
        let times: Vec<f64> = (0..30).map(|frame| (frame as f64 + 0.5) / 30.0).collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let mut missing = 0;
            for &time in &times {
                if hub
                    .video_frame(&path, time, FrameRate::Fps24, false)
                    .is_none()
                {
                    missing += 1;
                }
            }
            hub.poll(&ctx);
            if missing == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "repeat-frame cache did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            hub.stats().requested,
            24,
            "30 project frames at a 24 fps source request 24 source frames, not 30"
        );
        assert_eq!(
            hub.cache_usage().thumb_textures,
            24,
            "repeated project frames share one source texture"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and assets/demo (run with --ignored)"]
    fn real_demo_thumbnail_throughput() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo");
        let mut decoders = Lru::new(DECODER_CAPACITY);
        let started = std::time::Instant::now();
        for frame in 0..24 {
            let time = 1.0 + frame as f64 / 24.0;
            decode_frame(&mut decoders, &root.join("city_street.mp4"), time).expect("frame");
        }
        let per_frame = started.elapsed().as_secs_f64() * 1000.0 / 24.0;
        let started = std::time::Instant::now();
        let wave = decode_peaks(&root.join("narration.wav")).expect("peaks");
        let wave_ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "thumbnail decode+downscale: {per_frame:.2} ms/frame; waveform 3-level decode: {wave_ms:.1} ms, {} bytes, levels {:?}",
            wave.bytes(),
            wave.levels()
                .iter()
                .map(|l| l.peaks_per_second)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[ignore = "needs the pinned FFmpeg DLLs and assets/demo (run with --ignored)"]
    fn real_demo_media_decodes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo");
        let mut decoders = Lru::new(DECODER_CAPACITY);
        let a = decode_frame(&mut decoders, &root.join("city_street.mp4"), 3.0).expect("frame");
        assert!(
            a.width > 0 && a.height > 0 && a.rgba.len() == (a.width * a.height * 4) as usize
        );
        // Adjacent 24 fps source frames must be different real frames.
        let b = decode_frame(
            &mut decoders,
            &root.join("city_street.mp4"),
            3.0 + 1.0 / 24.0,
        )
        .expect("next frame");
        assert_ne!(a.pts_us, b.pts_us, "the decoder must advance to the next PTS");
        assert_ne!(a.rgba, b.rgba, "adjacent source frames must not be identical");
        let (iw, ih, _, _) =
            decode_still(&mut decoders, &root.join("mountain_lake.png")).expect("still");
        assert!(iw > 0 && ih > 0);
        let wave = decode_peaks(&root.join("narration.wav")).expect("peaks");
        assert!(!wave.is_empty());
        assert!(wave.duration > 5.0);
        assert_eq!(wave.level_for_px(12.0), 200.0);
        assert_eq!(wave.level_for_px(4_800.0), 4_000.0);
        let loudest = wave.peaks().iter().map(|p| p.max).fold(0.0f32, f32::max);
        assert!(loudest > 0.05, "the demo speech must not be silent");

        // Video audio is decoded by the same envelope path and used for the embedded strip.
        let embedded = decode_peaks(&root.join("city_street.mp4")).expect("video audio");
        assert!(!embedded.is_empty());
        let loudest_video = embedded
            .peaks()
            .iter()
            .map(|p| p.max.max(-p.min))
            .fold(0.0f32, f32::max);
        assert!(
            loudest_video > 0.01,
            "the street ambience must not be silent"
        );

        // A silent video must be recognised as such so it keeps the larger thumbnail region.
        let mut hub = MediaHub::new();
        assert!(hub.has_audio(&root.join("city_street.mp4")));
        assert!(!hub.has_audio(&root.join("market_crowd.mp4")));
        let meta = hub
            .video_meta(&root.join("city_street.mp4"))
            .expect("video meta");
        assert_eq!(meta.rate, Some(FrameRate::Fps24));
        assert!(!meta.vfr);
    }
}
