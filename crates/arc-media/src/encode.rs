//! Export-side encoding (docs/03 §7, docs/04 §5, §7).
//!
//! Runs inside the export process (`arcline-cli render`), which is itself a child of the GUI.
//! Frames are streamed as raw RGBA to an `ffmpeg` child that encodes and muxes. Encoders are
//! chosen by a capability probe (a one-frame trial encode), never guessed; the choice is
//! recorded in the export report. H.264/HEVC use hardware encoders — never GPL x264/x265. The
//! shipped 9.1 package contains no software HEVC encoder (`libx265` is GPL and excluded by D1), so
//! `hevc` selects `hevc_nvenc`/`hevc_qsv`/`hevc_amf` and reports `NoEncoder` when none is usable.

use crate::tools::{self, Exit};
use arc_core::time::Ratio;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// Lossless RGB (FFV1): golden tests and archival.
    Ffv1,
    H264,
    Av1,
    ProRes,
    /// SMPTE VC-3 / DNxHD (89/145/220 Mbit/s family; the encoder picks the profile from the size
    /// and the 36 Mbit/s rate used here).
    DnxHd,
    /// HEVC (H.265) through a hardware encoder; the shipped package has no software encoder.
    Hevc,
}

impl VideoCodec {
    /// Every codec name accepted by [`VideoCodec::parse`], in help-text order.
    pub const NAMES: [&'static str; 6] = ["ffv1", "h264", "hevc", "av1", "prores", "dnxhd"];

    /// `--codec` help text. Kept next to [`VideoCodec::NAMES`]; a unit test keeps them in sync.
    pub const HELP: &'static str = "ffv1 | h264 | hevc | av1 | prores | dnxhd";

    pub fn parse(s: &str) -> Option<VideoCodec> {
        match s {
            "ffv1" => Some(VideoCodec::Ffv1),
            "h264" => Some(VideoCodec::H264),
            "av1" => Some(VideoCodec::Av1),
            "prores" => Some(VideoCodec::ProRes),
            "dnxhd" => Some(VideoCodec::DnxHd),
            "hevc" => Some(VideoCodec::Hevc),
            _ => None,
        }
    }

    /// Candidates in preference order (docs/04 §5).
    fn candidates(self) -> &'static [&'static str] {
        match self {
            VideoCodec::Ffv1 => &["ffv1"],
            VideoCodec::H264 => &[
                "h264_nvenc",
                "h264_qsv",
                "h264_amf",
                "h264_mf",
                "libopenh264",
            ],
            VideoCodec::Av1 => &["av1_nvenc", "av1_qsv", "av1_amf", "libsvtav1"],
            VideoCodec::ProRes => &["prores_ks"],
            VideoCodec::DnxHd => &["dnxhd"],
            // Hardware only: the shipped 9.1 build has no `libx265` (GPL, D1) and no `hevc_mf`
            // (D2), so there is no software fallback by design.
            VideoCodec::Hevc => &["hevc_nvenc", "hevc_qsv", "hevc_amf"],
        }
    }

    fn pix_fmt(self) -> &'static str {
        match self {
            VideoCodec::Ffv1 => "bgr0",
            VideoCodec::H264 | VideoCodec::Av1 | VideoCodec::Hevc => "yuv420p",
            VideoCodec::ProRes => "yuv422p10le",
            VideoCodec::DnxHd => "yuv422p",
        }
    }

    fn audio_codec(self, out: &Path) -> &'static str {
        let ext = out
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match (self, ext.as_str()) {
            (VideoCodec::ProRes, _) | (_, "mov") => "pcm_s16le",
            (_, "mkv") => "flac",
            _ => "aac",
        }
    }
}

#[derive(Debug, Clone)]
pub enum AudioInput {
    None,
    /// A WAV file rendered by the offline mixer.
    Wav(PathBuf),
    /// An `ffmpeg -f lavfi` source (fixtures only).
    Lavfi(String),
}

#[derive(Debug, Clone)]
pub struct EncodeSpec {
    pub width: u32,
    pub height: u32,
    pub frame_rate: Ratio,
    pub codec: VideoCodec,
    pub audio: AudioInput,
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncodeReport {
    pub encoder: String,
    pub codec: VideoCodec,
    pub frames: u64,
    pub output: String,
    pub audio_codec: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("no working encoder for {0:?} (tried {1:?})")]
    NoEncoder(VideoCodec, Vec<String>),
    #[error("encoder i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("encoder failed: {0}")]
    Failed(String),
    #[error("frame has {got} bytes, expected {expected}")]
    FrameSize { got: usize, expected: usize },
}

fn trial(encoder: &str, codec: VideoCodec) -> bool {
    // DNxHD only accepts the SMPTE VC-3 frame sizes (1920x1080 etc.), so its trial uses one; the
    // other candidates can be tried cheaply at 256x144.
    let size = match codec {
        VideoCodec::DnxHd => "1920x1080",
        _ => "256x144",
    };
    let mut cmd = Command::new(tools::ffmpeg());
    cmd.args([
        "-hide_banner",
        "-v",
        "error",
        "-nostdin",
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=gray:s={size}:d=0.2:r=30"),
    ])
    .args(["-c:v", encoder, "-pix_fmt", codec.pix_fmt()]);
    if codec == VideoCodec::DnxHd {
        cmd.args(["-b:v", "36M"]);
    }
    cmd.args(["-f", "null", "-"]);
    matches!(
        tools::run(cmd, Duration::from_secs(30)).map(|o| o.exit),
        Ok(Exit::Ok)
    )
}

/// Pick the first candidate that is compiled in and actually encodes on this machine (cached).
pub fn select_encoder(codec: VideoCodec) -> Result<String, EncodeError> {
    tools::verify().map_err(EncodeError::Failed)?;
    static CACHE: OnceLock<Mutex<std::collections::HashMap<VideoCodec, Option<String>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(&codec).cloned()) {
        return hit.ok_or_else(|| {
            EncodeError::NoEncoder(
                codec,
                codec.candidates().iter().map(|s| s.to_string()).collect(),
            )
        });
    }
    let available = tools::encoders();
    let chosen = codec
        .candidates()
        .iter()
        .find(|c| available.iter().any(|a| a == *c) && trial(c, codec))
        .map(|s| s.to_string());
    if let Ok(mut c) = cache.lock() {
        c.insert(codec, chosen.clone());
    }
    chosen.ok_or_else(|| {
        EncodeError::NoEncoder(
            codec,
            codec.candidates().iter().map(|s| s.to_string()).collect(),
        )
    })
}

/// Streams RGBA frames into an `ffmpeg` encoder process.
pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    spec: EncodeSpec,
    encoder: String,
    frames: u64,
    stderr: std::thread::JoinHandle<String>,
}

impl std::fmt::Debug for Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder")
            .field("encoder", &self.encoder)
            .field("frames", &self.frames)
            .finish()
    }
}

impl Encoder {
    pub fn start(spec: EncodeSpec) -> Result<Encoder, EncodeError> {
        let encoder = select_encoder(spec.codec)?;
        let ffmpeg = tools::ffmpeg_checked().map_err(EncodeError::Failed)?;
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-hide_banner", "-v", "error", "-y"])
            .args(["-protocol_whitelist", "pipe"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", spec.width, spec.height)])
            .args(["-framerate", &spec.frame_rate.to_string()])
            .args(["-i", "pipe:0"]);
        let has_audio = match &spec.audio {
            AudioInput::None => false,
            AudioInput::Wav(p) => {
                let url = crate::paths::file_url(p).ok_or_else(|| {
                    EncodeError::Failed(format!("audio input is not a local file: {}", p.display()))
                })?;
                cmd.args(["-protocol_whitelist", "file", "-i"]).arg(url);
                true
            }
            AudioInput::Lavfi(src) => {
                cmd.args(["-f", "lavfi", "-i", src]);
                true
            }
        };
        cmd.args(["-map", "0:v"]);
        if has_audio {
            cmd.args(["-map", "1:a", "-shortest"]);
        }
        cmd.args(["-c:v", &encoder, "-pix_fmt", spec.codec.pix_fmt()]);
        if spec.codec != VideoCodec::Ffv1 {
            // RGB → YUV with BT.709, tagged (docs/03 §8)
            cmd.args(["-vf", "scale=out_color_matrix=bt709:out_range=tv"])
                .args([
                    "-colorspace",
                    "bt709",
                    "-color_primaries",
                    "bt709",
                    "-color_trc",
                    "bt709",
                ]);
        }
        match spec.codec {
            VideoCodec::Ffv1 => {
                cmd.args(["-level", "3", "-g", "1"]);
            }
            VideoCodec::ProRes => {
                cmd.args(["-profile:v", "3"]);
            }
            VideoCodec::DnxHd => {
                // SMPTE VC-3: the DNxHD profile follows from the frame size and this rate.
                cmd.args(["-b:v", "36M"]);
            }
            _ => {}
        }
        let audio_codec = has_audio.then(|| spec.codec.audio_codec(&spec.output).to_string());
        if let Some(ac) = &audio_codec {
            cmd.args(["-c:a", ac]);
        }
        let out_url = crate::paths::output_file_url(&spec.output).ok_or_else(|| {
            EncodeError::Failed(format!(
                "output is not a local path in an existing directory: {}",
                spec.output.display()
            ))
        })?;
        cmd.arg(out_url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let mut err = child.stderr.take();
        let stderr = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(e) = err.as_mut() {
                let _ = std::io::Read::read_to_string(e, &mut s);
            }
            s
        });
        Ok(Encoder {
            child,
            stdin,
            spec,
            encoder,
            frames: 0,
            stderr,
        })
    }

    pub fn encoder_name(&self) -> &str {
        &self.encoder
    }

    pub fn write_frame(&mut self, rgba: &[u8]) -> Result<(), EncodeError> {
        let expected = self.spec.width as usize * self.spec.height as usize * 4;
        if rgba.len() != expected {
            return Err(EncodeError::FrameSize {
                got: rgba.len(),
                expected,
            });
        }
        match self.stdin.as_mut() {
            Some(s) => s.write_all(rgba)?,
            None => return Err(EncodeError::Failed("encoder input closed".into())),
        }
        self.frames += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<EncodeReport, EncodeError> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        let stderr = self.stderr.join().unwrap_or_default();
        if !status.success() {
            return Err(EncodeError::Failed(stderr.trim().to_string()));
        }
        let has_audio = !matches!(self.spec.audio, AudioInput::None);
        Ok(EncodeReport {
            encoder: self.encoder,
            codec: self.spec.codec,
            frames: self.frames,
            output: self.spec.output.display().to_string(),
            audio_codec: has_audio
                .then(|| self.spec.codec.audio_codec(&self.spec.output).to_string()),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped package has no software HEVC encoder (libx265 is GPL and excluded), so the
    /// candidate list is hardware only and the failure is a typed error naming every candidate.
    /// The CLI help and the parser must offer the same codec names (P3-3).
    #[test]
    fn help_lists_every_parsable_codec_name() {
        assert_eq!(VideoCodec::NAMES.len(), 6);
        for name in VideoCodec::NAMES {
            assert!(VideoCodec::parse(name).is_some(), "{name} must parse");
            assert!(VideoCodec::HELP.contains(name), "{name} missing from HELP");
        }
        assert_eq!(VideoCodec::HELP.matches('|').count(), VideoCodec::NAMES.len() - 1);
    }

    #[test]
    fn hevc_candidates_are_hardware_only_and_the_error_names_them() {
        assert_eq!(
            VideoCodec::Hevc.candidates(),
            &["hevc_nvenc", "hevc_qsv", "hevc_amf"]
        );
        assert_eq!(VideoCodec::Hevc.pix_fmt(), "yuv420p");
        assert_eq!(VideoCodec::parse("hevc"), Some(VideoCodec::Hevc));
        let err = EncodeError::NoEncoder(
            VideoCodec::Hevc,
            VideoCodec::Hevc.candidates()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        let msg = err.to_string();
        for c in VideoCodec::Hevc.candidates() {
            assert!(msg.contains(c), "{msg}");
        }
        assert!(msg.contains("no working encoder"), "{msg}");
    }
}