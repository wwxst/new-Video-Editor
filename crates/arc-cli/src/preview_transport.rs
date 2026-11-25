//! Preview transport for the viewer (docs/specs/phase-c-plan.md C3).
//!
//! Play/pause/seek/step over a [`PreviewScheduler`], plus a small **composited-frame cache**: frames
//! that the pipeline already composed are kept (LRU, full quality only) so scrubbing back over the
//! same range does not re-compose them. This is the minimal cache for C3; the per-clip GPU texture
//! cache of docs/03 §3 (key: clip, source frame, effect-chain hash, parameter hash; 2 GB budget)
//! needs per-item render targets in `arc-render` and is deferred (see the C3 report).

use crate::preview::{PreviewFrame, PreviewQuality};
use crate::preview_scheduler::PreviewScheduler;
use arc_core::time::Ratio;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

/// Composited frames kept for instant re-display (full quality only; half frames are transient).
const DEFAULT_CACHE_FRAMES: usize = 8;

/// Viewer transport state: the playhead, the play flag and the latest composed frame.
pub struct PreviewTransport {
    scheduler: PreviewScheduler,
    total_frames: i64,
    frame_rate: Ratio,
    playhead: i64,
    playing: bool,
    /// Time carried over between ticks, in seconds.
    accumulated: f64,
    last_frame: Option<Arc<PreviewFrame>>,
    last_quality: Option<PreviewQuality>,
    serial: u64,
    cache: VecDeque<(i64, Arc<PreviewFrame>)>,
    cache_capacity: usize,
    /// Requests that hit the cache instead of the scheduler (diagnostics).
    cache_hits: u64,
}

impl std::fmt::Debug for PreviewTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewTransport")
            .field("playhead", &self.playhead)
            .field("playing", &self.playing)
            .field("total_frames", &self.total_frames)
            .field("serial", &self.serial)
            .field("cache_hits", &self.cache_hits)
            .finish()
    }
}

impl PreviewTransport {
    /// Transport over a scheduler; the first frame (0) is requested immediately.
    pub fn new(scheduler: PreviewScheduler, total_frames: i64, frame_rate: Ratio) -> PreviewTransport {
        let mut transport = PreviewTransport {
            scheduler,
            total_frames: total_frames.max(1),
            frame_rate,
            playhead: 0,
            playing: false,
            accumulated: 0.0,
            last_frame: None,
            last_quality: None,
            serial: 0,
            cache: VecDeque::new(),
            cache_capacity: DEFAULT_CACHE_FRAMES,
            cache_hits: 0,
        };
        let first = transport.playhead;
        transport.request(first);
        transport
    }

    pub fn playhead(&self) -> i64 {
        self.playhead
    }

    pub fn total_frames(&self) -> i64 {
        self.total_frames
    }

    pub fn frame_rate(&self) -> Ratio {
        self.frame_rate
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Start playing; from the last frame this restarts at 0.
    pub fn play(&mut self) {
        if self.playhead >= self.total_frames - 1 {
            self.seek(0);
        }
        self.playing = true;
        self.accumulated = 0.0;
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.accumulated = 0.0;
    }

    pub fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Move the playhead (clamped) and request that frame.
    pub fn seek(&mut self, frame: i64) -> i64 {
        let frame = frame.clamp(0, self.total_frames - 1);
        self.playhead = frame;
        self.accumulated = 0.0;
        self.request(frame);
        frame
    }

    pub fn step(&mut self, delta: i64) -> i64 {
        self.seek(self.playhead.saturating_add(delta))
    }

    /// Advance playback by `elapsed`; returns true when a new frame was requested.
    pub fn advance(&mut self, elapsed: Duration) -> bool {
        if !self.playing {
            return false;
        }
        let fps = self.frame_rate.to_f64_lossy();
        if fps <= 0.0 {
            return false;
        }
        let per_frame = 1.0 / fps;
        self.accumulated += elapsed.as_secs_f64();
        let steps = (self.accumulated / per_frame).floor();
        if steps < 1.0 {
            return false;
        }
        self.accumulated -= steps * per_frame;
        let next = self
            .playhead
            .saturating_add(steps as i64)
            .min(self.total_frames - 1);
        if next == self.playhead {
            self.pause();
            return false;
        }
        self.playhead = next;
        self.request(next);
        if self.playhead >= self.total_frames - 1 {
            self.pause();
        }
        true
    }

    /// Take the newest composed frame from the scheduler; returns true when the displayed frame
    /// changed (and `serial` was bumped).
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(result) = self.scheduler.take_result() {
            let frame = result.result.ok().map(Arc::new);
            if let Some(frame) = &frame
                && result.quality == PreviewQuality::Full
            {
                self.cache_frame(frame.clone());
            }
            self.last_frame = frame;
            self.last_quality = Some(result.quality);
            self.serial += 1;
            changed = true;
        }
        changed
    }

    pub fn frame(&self) -> Option<&PreviewFrame> {
        self.last_frame.as_deref()
    }

    /// Quality of the displayed frame (half while scrubbing, full when settled).
    pub fn displayed_quality(&self) -> Option<PreviewQuality> {
        self.last_quality
    }

    /// Bumped whenever the displayed frame changes; the viewer uses it to re-upload its texture.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// Requests served from the composited-frame cache instead of the scheduler.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits
    }

    fn request(&mut self, frame: i64) {
        if let Some((_, cached)) = self.cache.iter().find(|(f, _)| *f == frame) {
            let cached = cached.clone();
            self.cache_hits += 1;
            self.last_frame = Some(cached);
            self.last_quality = Some(PreviewQuality::Full);
            self.serial += 1;
            return;
        }
        self.scheduler.request(frame);
    }

    fn cache_frame(&mut self, frame: Arc<PreviewFrame>) {
        let index = frame.frame;
        if let Some(pos) = self.cache.iter().position(|(f, _)| *f == index) {
            self.cache.remove(pos);
        }
        self.cache.push_back((index, frame));
        while self.cache.len() > self.cache_capacity {
            self.cache.pop_front();
        }
    }
}
