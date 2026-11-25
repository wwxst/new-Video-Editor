//! The export job (docs/03 §7, docs/04 §7): runs in its own process (`arcline-cli render`),
//! reading a document snapshot, so a crash fails the job and never the editing session.
//!
//! plan_frame → in-process libav decode → GPU compositor → encoder; audio is mixed once by the
//! same mixer as playback and muxed by the encoder.

use arc_audio::{Mixer, SampleSource};
use arc_core::ids::AssetId;
use arc_core::model::{Project, Resolution};
use arc_core::time::Round;
use arc_media::encode::{AudioInput, EncodeSpec, Encoder, VideoCodec};
use arc_media::{AudioDecoder, VideoDecoder};
use arc_render::plan::{SourcePlan, frame_count, plan_frame};
use arc_render::{DrawItem, DrawLayer, Image, Renderer};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ExportSpec {
    pub output: PathBuf,
    pub codec: VideoCodec,
    /// Timeline frame range [from, to); defaults to the whole sequence.
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Base directory for relative asset paths (the project folder).
    pub media_root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    pub output: String,
    pub encoder: String,
    pub codec: VideoCodec,
    pub frames: u64,
    pub frame_rate: String,
    pub resolution: [u32; 2],
    pub duration: String,
    pub audio: Option<String>,
    pub renderer: RendererInfo,
    /// Identity of the FFmpeg this export actually used (spec §4.5).
    pub ffmpeg: arc_media::libav::RuntimeReport,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RendererInfo {
    pub version: String,
    pub adapter: String,
    /// BLAKE3 of the compositor shader source; changes whenever rendering math changes (docs/03 §9).
    pub shader_hash: String,
}

/// width, height, RGBA8 pixels
type DecodedFrame = (u32, u32, Vec<u8>);

fn resolve(root: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

struct AudioSources<'a> {
    project: &'a Project,
    root: &'a Path,
    rate: u32,
    decoders: HashMap<AssetId, Option<AudioDecoder>>,
    warnings: Vec<String>,
}

impl SampleSource for AudioSources<'_> {
    fn read(&mut self, asset: &AssetId, start: i64, dst: &mut [f32]) -> Result<(), String> {
        let (project, root, rate) = (self.project, self.root, self.rate);
        let warnings = &mut self.warnings;
        let dec = self.decoders.entry(asset.clone()).or_insert_with(|| {
            let a = project.assets.get(asset)?;
            match AudioDecoder::open(&resolve(root, &a.path), rate) {
                Ok(d) => Some(d),
                Err(e) => {
                    warnings.push(format!("audio for {asset} unavailable: {e}"));
                    None
                }
            }
        });
        match dec {
            Some(d) => d.read(start, dst).map(|_| ()).map_err(|e| e.to_string()),
            None => {
                dst.fill(0.0);
                Ok(())
            }
        }
    }
}

/// Render a project snapshot to a file. `progress(done, total)` is called after every frame.
pub fn export(
    project: &Project,
    spec: &ExportSpec,
    mut progress: impl FnMut(u64, u64),
) -> Result<ExportReport, String> {
    let seq = project.sequence().ok_or("project has no sequence")?;
    let rate = seq.frame_rate();
    let Resolution { w, h } = seq.resolution;
    let total = frame_count(project);
    let from = spec.from.unwrap_or(0).clamp(0, total);
    let to = spec.to.unwrap_or(total).clamp(from, total);
    if to <= from {
        return Err("nothing to render: the range is empty".into());
    }
    let mut warnings = Vec::new();

    // audio first: one offline pass of the shared mixer, written to a temp WAV
    let mut mixer = Mixer::new(project);
    let tmp_dir = spec
        .output
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let wav = tmp_dir.join(format!(
        ".{}.audio.wav",
        spec.output
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("export")
    ));
    let audio = if mixer.is_empty() {
        AudioInput::None
    } else {
        let sr = mixer.sample_rate();
        let s0 = arc_core::time::sample_index(rate.time_of_frame(from), sr, Round::Nearest)
            .map_err(|e| e.to_string())?;
        let s1 = arc_core::time::sample_index(rate.time_of_frame(to), sr, Round::Nearest)
            .map_err(|e| e.to_string())?;
        let mut src = AudioSources {
            project,
            root: &spec.media_root,
            rate: sr,
            decoders: HashMap::new(),
            warnings: Vec::new(),
        };
        let samples = mixer.render(&mut src, s0, s1, 4096)?;
        warnings.append(&mut src.warnings);
        arc_audio::write_wav_f32(&wav, sr, &samples).map_err(|e| format!("write audio: {e}"))?;
        AudioInput::Wav(wav.clone())
    };

    let mut renderer = Renderer::new_headless().map_err(|e| e.to_string())?;
    let mut encoder = Encoder::start(EncodeSpec {
        width: w,
        height: h,
        frame_rate: rate.rate(),
        codec: spec.codec,
        audio,
        output: spec.output.clone(),
    })
    .map_err(|e| e.to_string())?;

    let mut decoders: HashMap<AssetId, Option<VideoDecoder>> = HashMap::new();
    let mut titles_skipped = false;
    let mut unsupported = Vec::new();
    for f in from..to {
        let plan = plan_frame(project, f);
        // decode every source this frame needs, one entry per planned item
        let mut sources: Vec<Vec<Option<DecodedFrame>>> = Vec::with_capacity(plan.len());
        for layer in &plan {
            let mut row = Vec::with_capacity(layer.items.len());
            for item in &layer.items {
                let decoded = match &item.source {
                    SourcePlan::Media { asset, time } => {
                        let dec = decoders.entry(asset.clone()).or_insert_with(|| {
                            let a = project.assets.get(asset)?;
                            match VideoDecoder::open(&resolve(&spec.media_root, &a.path)) {
                                Ok(d) => Some(d),
                                Err(e) => {
                                    warnings.push(format!("video for {asset} unavailable: {e}"));
                                    None
                                }
                            }
                        });
                        match dec.as_mut().map(|d| d.frame_at(*time)) {
                            Some(Ok(fr)) => Some((fr.width, fr.height, fr.rgba.to_vec())),
                            Some(Err(e)) => {
                                warnings.push(format!("frame {f}: {asset} at {time}: {e}"));
                                None
                            }
                            None => None,
                        }
                    }
                    SourcePlan::Title { .. } => {
                        titles_skipped = true;
                        None
                    }
                };
                row.push(decoded);
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
                    .filter_map(|(item, src)| {
                        let (fw, fh, px) = src.as_ref()?;
                        Some(DrawItem::from_plan(
                            item,
                            Image {
                                width: *fw,
                                height: *fh,
                                rgba: px,
                            },
                        ))
                    })
                    .collect(),
            })
            .collect();
        let (pixels, rep) = renderer
            .render(w, h, [0.0, 0.0, 0.0, 1.0], &layers)
            .map_err(|e| e.to_string())?;
        for e in rep.unsupported_effects {
            if !unsupported.contains(&e) {
                unsupported.push(e);
            }
        }
        encoder.write_frame(&pixels).map_err(|e| e.to_string())?;
        progress((f - from + 1) as u64, (to - from) as u64);
    }
    let enc = encoder.finish().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&wav);
    if titles_skipped {
        warnings.push(
            "title clips are not rendered yet (text renderer pending); they were skipped".into(),
        );
    }
    for e in &unsupported {
        warnings.push(format!(
            "effect {e} has no GPU implementation yet; passed through"
        ));
    }
    warnings.dedup();
    let duration = rate.time_of_frame(to - from);
    let mut ffmpeg = arc_media::libav::runtime_report();
    ffmpeg.override_used |= arc_media::tools::override_used();
    Ok(ExportReport {
        output: spec.output.display().to_string(),
        encoder: enc.encoder,
        codec: enc.codec,
        frames: enc.frames,
        frame_rate: rate.wire(),
        resolution: [w, h],
        duration: rate.time_to_timecode(duration),
        audio: enc.audio_codec,
        renderer: RendererInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            adapter: renderer.adapter_info().to_string(),
            shader_hash: shader_hash(),
        },
        ffmpeg,
        warnings,
    })
}

fn shader_hash() -> String {
    let src = include_str!("../../arc-render/src/shaders/compositor.wgsl");
    blake3::hash(src.as_bytes()).to_hex()[..16].to_string()
}
