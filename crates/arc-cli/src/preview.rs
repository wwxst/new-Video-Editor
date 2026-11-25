//! Headless preview frame pipeline (docs/03 §3, docs/specs/phase-c-plan.md C1).
//!
//! The viewer's engine side: compose one RGBA frame for a timeline frame from the same `plan_frame`
//! and `Renderer` the exporter uses, with one decoder per asset and a byte-bounded LRU of **decoded
//! source frames** (keyed by asset + source time, because effects are applied after decode). The
//! GPU-side *composited*-frame cache of docs/03 §3 (keyed by clip, effect chain and parameters)
//! lands with the viewer texture in C3. The GUI only uploads the returned pixels; it never decodes
//! or composes, so preview and export cannot drift apart silently.

use crate::export::resolve;
use arc_core::ids::AssetId;
use arc_core::model::{Asset, Project};
use arc_core::time::Time;
use arc_media::VideoDecoder;
use arc_render::plan::{frame_count, plan_frame, SourcePlan};
use arc_render::{DrawItem, DrawLayer, Image, Renderer};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use std::sync::Arc;

/// Decoded source bytes kept in memory (docs/03 §3: in-memory, no disk cache in v1). A byte budget,
/// not a frame count: one 4K frame is 33 MB, so a fixed count would blow the budget on large media.
const DEFAULT_CACHE_BYTES: u64 = 256 * 1024 * 1024;

pub const TITLE_WARNING: &str = "title clips are not rendered yet (text renderer pending); they were skipped";

#[derive(Debug, Clone, thiserror::Error)]
pub enum PreviewError {
    #[error("no GPU adapter for the preview renderer: {0}")]
    Renderer(String),
    #[error("project has no sequence")]
    NoSequence,
    #[error("frame {frame} is outside the sequence (0..{end})")]
    OutOfRange { frame: i64, end: i64 },
    #[error("preview render failed: {0}")]
    Render(String),
}

/// Scrub quality: the editor renders half resolution while the playhead moves and full quality when
/// it settles (docs/03 §3, docs/10 Phase C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewQuality {
    Full,
    Half,
}

impl PreviewQuality {
    pub fn divisor(self) -> u32 {
        match self {
            PreviewQuality::Full => 1,
            PreviewQuality::Half => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PreviewQuality::Full => "full",
            PreviewQuality::Half => "half",
        }
    }
}

/// One composed frame, ready to be uploaded as a texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    pub frame: i64,
    pub width: u32,
    pub height: u32,
    /// RGBA8, `width × height × 4` bytes.
    pub rgba: Vec<u8>,
    /// Problems met while composing this frame (unavailable assets, unsupported effects, titles).
    pub warnings: Vec<String>,
}

/// Instrumentation used by the tests and by the scheduler's diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewStats {
    /// Frames composed.
    pub frames: u64,
    /// Source-frame requests that missed the cache and reached a decoder (a decoder may serve a
    /// sequential neighbour from its own look-ahead, so this counts requests, not seeks).
    pub decodes: u64,
    /// Requests served from the decoded-frame cache.
    pub cache_hits: u64,
}

#[derive(Debug)]
struct CachedFrame {
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
}

impl CachedFrame {
    fn bytes(&self) -> u64 {
        self.rgba.len() as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FrameKey {
    asset: AssetId,
    time: Time,
}

/// Identity of an asset file on disk: path, size and modification time. A repaired or replaced file
/// at the same path changes this, so a decoder or an "unavailable" verdict can be invalidated
/// without waiting for an edit (P3-4).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

fn file_fingerprint(root: &Path, asset: &Asset) -> FileFingerprint {
    let path = resolve(root, &asset.path);
    let (len, modified) = std::fs::metadata(&path)
        .map(|m| (m.len(), m.modified().ok()))
        .unwrap_or((0, None));
    FileFingerprint { path, len, modified }
}

fn push_warning(warnings: &mut Vec<String>, message: String) {
    if !warnings.contains(&message) {
        warnings.push(message);
    }
}

/// Composes timeline frames from a project snapshot. One renderer, one decoder per asset, one
/// bounded frame cache; all state is owned by this struct, so the caller (UI thread or worker) only
/// asks for frames.
pub struct PreviewEngine {
    project: Project,
    media_root: PathBuf,
    quality: PreviewQuality,
    renderer: Renderer,
    /// Open decoders with the file identity they were opened from.
    decoders: HashMap<AssetId, (FileFingerprint, VideoDecoder)>,
    /// Assets whose file could not be opened, with the identity that failed: a changed file is
    /// retried on the next frame (repair in place or relink).
    unavailable: HashMap<AssetId, FileFingerprint>,
    cache: HashMap<FrameKey, Arc<CachedFrame>>,
    order: VecDeque<FrameKey>,
    cache_bytes: u64,
    budget_bytes: u64,
    stats: PreviewStats,
}

impl std::fmt::Debug for PreviewEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewEngine")
            .field("quality", &self.quality)
            .field("decoders", &self.decoders.len())
            .field("cached_frames", &self.cache.len())
            .field("stats", &self.stats)
            .finish()
    }
}

impl PreviewEngine {
    pub fn new(
        project: Project,
        media_root: impl Into<PathBuf>,
        quality: PreviewQuality,
    ) -> Result<PreviewEngine, PreviewError> {
        let renderer =
            Renderer::new_headless().map_err(|e| PreviewError::Renderer(e.to_string()))?;
        Ok(PreviewEngine::with_renderer(
            project, media_root, quality, renderer,
        ))
    }

    /// Share an existing device (the GUI's `Renderer::with_device`) instead of creating one.
    pub fn with_renderer(
        project: Project,
        media_root: impl Into<PathBuf>,
        quality: PreviewQuality,
        renderer: Renderer,
    ) -> PreviewEngine {
        PreviewEngine {
            project,
            media_root: media_root.into(),
            quality,
            renderer,
            decoders: HashMap::new(),
            unavailable: HashMap::new(),
            cache: HashMap::new(),
            order: VecDeque::new(),
            cache_bytes: 0,
            budget_bytes: DEFAULT_CACHE_BYTES,
            stats: PreviewStats::default(),
        }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Replace the snapshot after an edit or a document change. Decoders of removed assets are
    /// dropped and the decoded-frame cache is cleared; a later slice can invalidate incrementally.
    pub fn set_project(&mut self, project: Project) {
        // Keep a decoder (and an "unavailable" verdict) only while the asset's path is unchanged;
        // a relink or a repaired file must be retried instead of serving a stale decoder.
        self.decoders.retain(|id, (known, _)| {
            project
                .assets
                .get(id)
                .is_some_and(|asset| *known == file_fingerprint(&self.media_root, asset))
        });
        self.unavailable.retain(|id, known| {
            project
                .assets
                .get(id)
                .is_some_and(|asset| *known == file_fingerprint(&self.media_root, asset))
        });
        self.cache.clear();
        self.order.clear();
        self.cache_bytes = 0;
        self.project = project;
    }

    pub fn quality(&self) -> PreviewQuality {
        self.quality
    }

    /// Switch between scrub and full quality; the cache keys stay valid because they describe
    /// source frames, not output frames.
    pub fn set_quality(&mut self, quality: PreviewQuality) {
        self.quality = quality;
    }

    pub fn stats(&self) -> PreviewStats {
        self.stats.clone()
    }

    /// Decoded-source memory budget; larger frames therefore cache fewer entries.
    pub fn set_cache_budget(&mut self, bytes: u64) {
        self.budget_bytes = bytes.max(1);
        self.evict();
    }

    /// Bytes currently held by the decoded-frame cache (tests and diagnostics).
    pub fn cache_bytes(&self) -> u64 {
        self.cache_bytes
    }

    /// Size of a cached decoded source frame; proves the decoder works at native resolution even
    /// when the preview composes at half size (diagnostics and tests).
    pub fn cached_source_size(&self, asset: &AssetId, time: Time) -> Option<(u32, u32)> {
        self.cache
            .get(&FrameKey {
                asset: asset.clone(),
                time,
            })
            .map(|c| (c.width, c.height))
    }

    /// Compose timeline frame `frame` (0-based, at the sequence frame rate).
    pub fn frame(&mut self, frame: i64) -> Result<PreviewFrame, PreviewError> {
        let (seq_w, seq_h, total) = {
            let seq = self.project.sequence().ok_or(PreviewError::NoSequence)?;
            (seq.resolution.w, seq.resolution.h, frame_count(&self.project))
        };
        if frame < 0 || frame >= total {
            return Err(PreviewError::OutOfRange { frame, end: total });
        }
        let divisor = self.quality.divisor();
        let width = (seq_w / divisor).max(1);
        let height = (seq_h / divisor).max(1);
        let plan = plan_frame(&self.project, frame);

        let mut warnings = Vec::new();
        let mut sources: Vec<Vec<Option<Arc<CachedFrame>>>> = Vec::with_capacity(plan.len());
        for layer in &plan {
            let mut row = Vec::with_capacity(layer.items.len());
            for item in &layer.items {
                match &item.source {
                    SourcePlan::Media { asset, time } => {
                        row.push(self.decode(asset, *time, &mut warnings))
                    }
                    SourcePlan::Title { .. } => {
                        push_warning(&mut warnings, TITLE_WARNING.to_string());
                        row.push(None);
                    }
                }
            }
            sources.push(row);
        }

        let layers: Vec<DrawLayer<'_>> = plan
            .iter()
            .zip(&sources)
            .map(|(layer, row)| DrawLayer {
                opacity: layer.opacity,
                items: layer
                    .items
                    .iter()
                    .zip(row)
                    .filter_map(|(item, source)| {
                        let cached = source.as_ref()?;
                        Some(DrawItem::from_plan(
                            item,
                            Image {
                                width: cached.width,
                                height: cached.height,
                                rgba: &cached.rgba,
                            },
                        ))
                    })
                    .collect(),
            })
            .collect();

        let (pixels, report) = self
            .renderer
            .render(width, height, [0.0, 0.0, 0.0, 1.0], &layers)
            .map_err(|e| PreviewError::Render(e.to_string()))?;
        for e in report.unsupported_effects {
            push_warning(
                &mut warnings,
                format!("effect {e} has no GPU implementation yet; passed through"),
            );
        }
        self.stats.frames += 1;
        Ok(PreviewFrame {
            frame,
            width,
            height,
            rgba: pixels,
            warnings,
        })
    }

    /// Decoded source frame for `(asset, time)`, from the cache when possible.
    fn decode(
        &mut self,
        asset: &AssetId,
        time: Time,
        warnings: &mut Vec<String>,
    ) -> Option<Arc<CachedFrame>> {
        let key = FrameKey {
            asset: asset.clone(),
            time,
        };
        if let Some(cached) = self.cache.get(&key).cloned() {
            self.stats.cache_hits += 1;
            self.touch(&key);
            return Some(cached);
        }
        let Some(asset_info) = self.project.assets.get(asset) else {
            push_warning(warnings, format!("video for {asset} unavailable: unknown asset"));
            return None;
        };
        let current = file_fingerprint(&self.media_root, asset_info);
        if let Some(failed) = self.unavailable.get(asset) {
            if *failed == current {
                push_warning(warnings, format!("video for {asset} unavailable"));
                return None;
            }
            // The file changed since it failed (repaired in place or relinked): retry it.
            self.unavailable.remove(asset);
        }
        if self
            .decoders
            .get(asset)
            .is_some_and(|(known, _)| *known != current)
        {
            // The file was replaced under an open decoder: drop it so the new file is decoded.
            self.decoders.remove(asset);
        }
        if !self.decoders.contains_key(asset) {
            let path = current.path.clone();
            match VideoDecoder::open(&path) {
                Ok(d) => {
                    self.decoders.insert(asset.clone(), (current.clone(), d));
                }
                Err(e) => {
                    push_warning(warnings, format!("video for {asset} unavailable: {e}"));
                    self.unavailable.insert(asset.clone(), current);
                    return None;
                }
            }
        }
        let (_, decoder) = self.decoders.get_mut(asset)?;
        match decoder.frame_at(time) {
            Ok(f) => {
                self.stats.decodes += 1;
                let cached = Arc::new(CachedFrame {
                    width: f.width,
                    height: f.height,
                    rgba: Arc::from(f.rgba.to_vec()),
                });
                self.insert(key, cached.clone());
                Some(cached)
            }
            Err(e) => {
                push_warning(warnings, format!("frame {time} of {asset}: {e}"));
                None
            }
        }
    }

    fn insert(&mut self, key: FrameKey, frame: Arc<CachedFrame>) {
        self.cache_bytes += frame.bytes();
        if self.cache.insert(key.clone(), frame).is_none() {
            self.order.push_back(key);
        }
        self.evict();
    }

    /// Drop least-recently-used frames until the byte budget is met, always keeping the newest one
    /// (a single frame larger than the budget must still render).
    fn evict(&mut self) {
        while self.cache_bytes > self.budget_bytes && self.order.len() > 1 {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(frame) = self.cache.remove(&old) {
                self.cache_bytes -= frame.bytes();
            }
        }
    }

    fn touch(&mut self, key: &FrameKey) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.clone());
    }
}
