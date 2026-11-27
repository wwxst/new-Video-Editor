//! Generated fixture media (docs/08 §6: no third-party media in CI).
//!
//! A "counter" video encodes each frame's index as 32 black/white blocks across the top band, so
//! frame accuracy can be verified exactly after any decode → render → encode → decode round trip.

use crate::encode::{AudioInput, EncodeError, EncodeReport, EncodeSpec, Encoder, VideoCodec};
use arc_core::time::Ratio;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub const BITS: u32 = 32;

/// Paint one counter frame: the top eighth carries the index, the rest a gradient.
pub fn counter_frame(index: u32, w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    let band = (h / 8).max(4);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let (r, g, b) = if y < band {
                let bit = (x * BITS / w).min(BITS - 1);
                let on = (index >> (BITS - 1 - bit)) & 1 == 1;
                if on { (255, 255, 255) } else { (0, 0, 0) }
            } else {
                ((x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 128)
            };
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = 255;
        }
    }
    px
}

/// Read the index from a counter frame (robust to mild filtering and scaling).
pub fn read_counter(rgba: &[u8], w: u32, h: u32) -> Option<u32> {
    let band = (h / 8).max(4);
    let y = band / 2;
    let mut v = 0u32;
    for bit in 0..BITS {
        let x = ((bit * 2 + 1) * w) / (BITS * 2);
        let i = ((y * w + x) * 4) as usize;
        let l = *rgba.get(i)? as u32 + *rgba.get(i + 1)? as u32 + *rgba.get(i + 2)? as u32;
        v = (v << 1) | u32::from(l > 3 * 128);
    }
    Some(v)
}

/// Write a counter video: `frames` frames at `rate`, lossless FFV1 in MKV, optional 1 kHz tone.
pub fn write_counter_video(
    path: &Path,
    frames: u32,
    rate: Ratio,
    w: u32,
    h: u32,
    tone: bool,
) -> Result<EncodeReport, EncodeError> {
    let audio = if tone {
        let secs = frames as f64 * rate.den() as f64 / rate.num() as f64;
        AudioInput::Lavfi(format!(
            "sine=frequency=1000:sample_rate=48000:duration={secs:.6}"
        ))
    } else {
        AudioInput::None
    };
    let mut enc = Encoder::start(EncodeSpec {
        width: w,
        height: h,
        frame_rate: rate,
        codec: VideoCodec::Ffv1,
        audio,
        output: path.to_path_buf(),
    })?;
    for i in 0..frames {
        enc.write_frame(&counter_frame(i, w, h))?;
    }
    enc.finish()
}

/// Write a true VFR counter video with a deliberate gap every `gap_every` frames.
///
/// The file is an MP4 with FFV1 frames (lossless, unique counter per frame). `setpts` adds
/// `gap_seconds` of screen time at each boundary, so a frame is displayed for more than one project
/// frame at normal rates. MP4 (unlike MKV here) reports an average rate that differs from the
/// nominal rate, which is what `StreamInfo::vfr` detects.
///
/// `path` must end in `.mp4`: the MKV muxer keeps writing the nominal `DefaultDuration` and hides
/// the VFR flag even when the PTS are uneven.
pub fn write_vfr_counter_video(
    path: &Path,
    frames: u32,
    base_rate: Ratio,
    w: u32,
    h: u32,
    gap_every: u32,
    gap_seconds: f64,
) -> Result<(), EncodeError> {
    if path.extension().and_then(|e| e.to_str()) != Some("mp4") {
        return Err(EncodeError::Failed(
            "VFR counter fixture must be an .mp4 (MKV hides the VFR flag)".to_string(),
        ));
    }
    if gap_every == 0 || !(gap_seconds > 0.0) {
        return Err(EncodeError::Failed(
            "VFR fixture needs a positive gap interval and duration".to_string(),
        ));
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!(
        "arcline-vfr-base-{}-{nanos}.mkv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&base);
    write_counter_video(&base, frames, base_rate, w, h, false)?;

    let ffmpeg = crate::tools::ffmpeg_checked().map_err(EncodeError::Failed)?;
    let filter = format!("setpts=PTS+{gap_seconds}/TB*floor(N/{gap_every})");
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-v", "error", "-y", "-i"])
        .arg(&base)
        .args([
            "-vf",
            &filter,
            "-fps_mode",
            "passthrough",
            "-c:v",
            "ffv1",
            "-level",
            "3",
            "-g",
            "1",
            "-an",
        ])
        .arg(path);
    let output = crate::tools::run(cmd, Duration::from_secs(60))
        .map_err(|error| EncodeError::Failed(error.to_string()))?;
    let _ = std::fs::remove_file(&base);
    if output.exit != crate::tools::Exit::Ok {
        return Err(EncodeError::Failed(format!(
            "VFR fixture remux failed: {}",
            output.stderr
        )));
    }
    Ok(())
}

/// Expected presentation timestamps (microseconds) for [`write_vfr_counter_video`].
pub fn vfr_counter_pts_us(
    frames: u32,
    base_rate: Ratio,
    gap_every: u32,
    gap_seconds: f64,
) -> Vec<u64> {
    let fps = base_rate.num() as f64 / base_rate.den() as f64;
    (0..frames)
        .map(|n| {
            let gap = gap_seconds * (n / gap_every.max(1)) as f64;
            (((n as f64 / fps) + gap) * 1_000_000.0).round() as u64
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_round_trip_in_memory() {
        for i in [0u32, 1, 2, 1799, 1800, 107_892, u32::MAX] {
            let f = counter_frame(i, 320, 180);
            assert_eq!(read_counter(&f, 320, 180), Some(i));
        }
    }
}
