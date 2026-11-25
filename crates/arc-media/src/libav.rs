//! Safe wrappers over the libav shim (`csrc/arc_av.c`).
//!
//! Error codes and Rust panics are recoverable per clip; a native fault inside FFmpeg is not, which
//! is why untested media goes through the subprocess import check first (docs/04 §1.1–1.2).
#![allow(unsafe_code)]

use arc_core::time::{Ratio, Round, Time};
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::Path;
use std::sync::OnceLock;

#[repr(C)]
struct ArcAv {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RawInfo {
    width: c_int,
    height: c_int,
    fr_num: c_int,
    fr_den: c_int,
    tb_num: c_int,
    tb_den: c_int,
    start_pts: i64,
    duration_us: i64,
    sample_rate: c_int,
    channels: c_int,
    vfr: c_int,
    codec: [c_char; 32],
    pix_fmt: [c_char; 32],
}

#[repr(C)]
struct RawRuntime {
    build: [u32; 5],
    runtime: [u32; 5],
    license: [[c_char; 48]; 5],
    ident: [c_char; 64],
}

/// Names of the five libraries, in the shim's `ArcAvRuntime` order.
pub const LIBRARIES: [&str; 5] = ["avutil", "avcodec", "avformat", "swscale", "swresample"];

/// D1: the shipped build is LGPL v2.1 or later (no `--enable-version3`).
pub const APPROVED_LICENSE: &str = "LGPL version 2.1 or later";

/// Identity of the FFmpeg the process is actually using; computed once (spec §4.5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuntimeReport {
    pub ident: String,
    /// library name → "major.minor.micro" of the loaded DLL
    pub libraries: BTreeMap<String, String>,
    pub license: String,
    /// SHA-256 of `third_party/ffmpeg/manifest.json`
    pub build_id: String,
    #[serde(rename = "override")]
    pub override_used: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn version_string(v: u32) -> String {
    format!("{}.{}.{}", v >> 16, (v >> 8) & 0xFF, v & 0xFF)
}

/// Pure comparison used by the runtime check and its unit tests.
fn validate_runtime(
    build: &[u32; 5],
    runtime: &[u32; 5],
    licenses: &[String; 5],
) -> Vec<String> {
    let mut problems = Vec::new();
    for (i, name) in LIBRARIES.iter().enumerate() {
        let (b, r) = (build[i], runtime[i]);
        let (bm, bmin) = (b >> 16, (b >> 8) & 0xFF);
        let (rm, rmin) = (r >> 16, (r >> 8) & 0xFF);
        if rm != bm {
            problems.push(format!(
                "{name}: run-time major {rm} does not match build major {bm} ({} vs {})",
                version_string(r),
                version_string(b)
            ));
        } else if rmin < bmin {
            problems.push(format!(
                "{name}: run-time {} is older than build {}",
                version_string(r),
                version_string(b)
            ));
        }
        let lic = &licenses[i];
        let lower = lic.to_ascii_lowercase();
        if lower.contains("nonfree") {
            problems.push(format!("{name}: nonfree licence {lic:?}"));
        } else if lower.contains("gpl version") && !lower.contains("lgpl") {
            problems.push(format!("{name}: GPL (not LGPL) licence {lic:?}"));
        } else if lic != APPROVED_LICENSE {
            problems.push(format!(
                "{name}: licence {lic:?} is not the approved {APPROVED_LICENSE:?} (decision D1)"
            ));
        }
    }
    problems
}

const BUILD_IDENT: &str = match option_env!("ARC_FFMPEG_BUILD_IDENT") {
    Some(v) => v,
    None => "unpinned",
};
const MANIFEST_SHA256: &str = match option_env!("ARC_FFMPEG_MANIFEST_SHA256") {
    Some(v) => v,
    None => "unpinned",
};

struct RuntimeState {
    report: RuntimeReport,
    refusal: Option<String>,
}

fn runtime_state() -> &'static RuntimeState {
    static STATE: OnceLock<RuntimeState> = OnceLock::new();
    STATE.get_or_init(load_runtime)
}

fn load_runtime() -> RuntimeState {
    let override_allowed = std::env::var("ARCLINE_ALLOW_ANY_FFMPEG").as_deref() == Ok("1");
    let mut raw = RawRuntime {
        build: [0; 5],
        runtime: [0; 5],
        license: [[0; 48]; 5],
        ident: [0; 64],
    };
    // SAFETY: raw is a valid out-struct matching ArcAvRuntime's layout.
    let r = unsafe { arc_av_runtime(&mut raw) };
    let mut problems = Vec::new();
    if r != 0 {
        problems.push("arc_av_runtime failed".to_string());
    }
    let abi = unsafe { arc_av_abi_size() };
    if abi != std::mem::size_of::<RawInfo>() as c_int {
        problems.push(format!(
            "ArcAvInfo ABI size {abi} does not match the Rust mirror ({})",
            std::mem::size_of::<RawInfo>()
        ));
    }
    let licenses: [String; 5] = std::array::from_fn(|i| c_str(&raw.license[i]));
    problems.extend(validate_runtime(&raw.build, &raw.runtime, &licenses));
    let ident = c_str(&raw.ident);
    if BUILD_IDENT != "unpinned" && !ident.is_empty() && ident != BUILD_IDENT {
        problems.push(format!(
            "loaded FFmpeg ident {ident:?} does not match the build ident {BUILD_IDENT:?}"
        ));
    }
    let libraries = LIBRARIES
        .iter()
        .enumerate()
        .map(|(i, n)| (n.to_string(), version_string(raw.runtime[i])))
        .collect();
    let approved_license = licenses
        .iter()
        .find(|l| *l == APPROVED_LICENSE)
        .cloned()
        .unwrap_or_else(|| licenses[0].clone());
    let mut report = RuntimeReport {
        ident,
        libraries,
        license: approved_license,
        build_id: MANIFEST_SHA256.to_string(),
        override_used: false,
        warnings: Vec::new(),
    };
    let refusal = if problems.is_empty() {
        None
    } else if override_allowed {
        report.override_used = true;
        report.warnings = problems
            .iter()
            .map(|p| format!("ARCLINE_ALLOW_ANY_FFMPEG override: {p}"))
            .collect();
        None
    } else {
        Some(problems.join("; "))
    };
    RuntimeState { report, refusal }
}

/// Check the loaded FFmpeg once per process; the decoders refuse to open if it is not the pinned,
/// approved build (spec §4.5).
fn ensure_runtime() -> Result<(), MediaError> {
    match &runtime_state().refusal {
        None => Ok(()),
        Some(reason) => Err(MediaError::Decode(format!(
            "ffmpeg runtime refused: {reason}"
        ))),
    }
}

/// The runtime identity recorded in export reports.
pub fn runtime_report() -> RuntimeReport {
    runtime_state().report.clone()
}

/// True when the run-time check or tool verification was overridden in this process.
pub fn runtime_override_used() -> bool {
    runtime_state().report.override_used
}

unsafe extern "C" {
    fn arc_av_open(
        path: *const c_char,
        kind: c_int,
        threads: c_int,
        out: *mut *mut ArcAv,
        err: *mut c_char,
        errlen: c_int,
    ) -> c_int;
    fn arc_av_info(a: *mut ArcAv, info: *mut RawInfo) -> c_int;
    fn arc_av_video_at(
        a: *mut ArcAv,
        target: i64,
        dst: *mut u8,
        stride: c_int,
        got: *mut i64,
        err: *mut c_char,
        errlen: c_int,
    ) -> c_int;
    fn arc_av_audio_read(
        a: *mut ArcAv,
        rate: c_int,
        start: i64,
        frames: c_int,
        dst: *mut f32,
        err: *mut c_char,
        errlen: c_int,
    ) -> c_int;
    fn arc_av_close(a: *mut ArcAv);
    fn arc_av_runtime(out: *mut RawRuntime) -> c_int;
    fn arc_av_abi_size() -> c_int;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MediaError {
    #[error("{0}")]
    Decode(String),
    #[error("invalid path")]
    Path,
    #[error("end of stream")]
    EndOfStream,
}

fn c_str(buf: &[c_char]) -> String {
    // SAFETY: the shim always NUL-terminates within the buffer (snprintf).
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// Stream information reported by libav.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: Option<Ratio>,
    pub time_base: Ratio,
    pub start_pts: i64,
    pub duration: Option<Time>,
    pub sample_rate: u32,
    pub channels: u32,
    pub vfr: bool,
    pub codec: String,
    pub format: String,
}

struct Handle(*mut ArcAv);
// SAFETY: an ArcAv context is used by one thread at a time (decoders are not Sync); FFmpeg decoder
// contexts may move between threads.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: pointer came from arc_av_open and is closed exactly once.
        unsafe { arc_av_close(self.0) }
    }
}

/// Build the `file:` URL for a local media file. Document asset paths are untrusted input, so only
/// canonical local files are ever passed to libav (audit F-09, spec §4.4/S1).
fn open(path: &Path, kind: c_int) -> Result<(Handle, StreamInfo), MediaError> {
    ensure_runtime()?;
    let url = crate::paths::file_url(path).ok_or(MediaError::Path)?;
    let p = CString::new(url).map_err(|_| MediaError::Path)?;
    let mut err = [0 as c_char; 256];
    let mut h: *mut ArcAv = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated path, out-pointer, and error buffer of the stated length.
    let r = unsafe {
        arc_av_open(
            p.as_ptr(),
            kind,
            0,
            &mut h,
            err.as_mut_ptr(),
            err.len() as c_int,
        )
    };
    if r != 0 || h.is_null() {
        return Err(MediaError::Decode(c_str(&err)));
    }
    let h = Handle(h);
    let mut raw = RawInfo {
        width: 0,
        height: 0,
        fr_num: 0,
        fr_den: 0,
        tb_num: 0,
        tb_den: 0,
        start_pts: 0,
        duration_us: -1,
        sample_rate: 0,
        channels: 0,
        vfr: 0,
        codec: [0; 32],
        pix_fmt: [0; 32],
    };
    // SAFETY: h is a live context; raw is a valid out-struct matching ArcAvInfo's layout.
    unsafe { arc_av_info(h.0, &mut raw) };
    let time_base = Ratio::new(raw.tb_num.max(1) as i64, raw.tb_den.max(1) as i64)
        .map_err(|e| MediaError::Decode(e.to_string()))?;
    let frame_rate = if raw.fr_num > 0 && raw.fr_den > 0 {
        Ratio::new(raw.fr_num as i64, raw.fr_den as i64).ok()
    } else {
        None
    };
    let duration = (raw.duration_us >= 0)
        .then(|| Time::new(raw.duration_us, 1_000_000).ok())
        .flatten();
    let info = StreamInfo {
        width: raw.width.max(0) as u32,
        height: raw.height.max(0) as u32,
        frame_rate,
        time_base,
        start_pts: raw.start_pts,
        duration,
        sample_rate: raw.sample_rate.max(0) as u32,
        channels: raw.channels.max(0) as u32,
        vfr: raw.vfr != 0,
        codec: c_str(&raw.codec),
        format: c_str(&raw.pix_fmt),
    };
    Ok((h, info))
}

/// In-process, frame-accurate video decoder producing packed RGBA (sRGB).
pub struct VideoDecoder {
    h: Handle,
    info: StreamInfo,
    buf: Vec<u8>,
}

impl std::fmt::Debug for VideoDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoDecoder")
            .field("info", &self.info)
            .finish()
    }
}

/// One decoded frame, borrowed from the decoder's buffer.
#[derive(Debug)]
pub struct FrameRef<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
    /// Presentation time of the delivered frame, relative to the stream start.
    pub time: Time,
}

impl VideoDecoder {
    pub fn open(path: &Path) -> Result<VideoDecoder, MediaError> {
        let (h, info) = open(path, 0)?;
        if info.width == 0 || info.height == 0 {
            return Err(MediaError::Decode("video stream has no dimensions".into()));
        }
        let buf = vec![0u8; info.width as usize * info.height as usize * 4];
        Ok(VideoDecoder { h, info, buf })
    }

    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    /// The frame displayed at source time `t` (relative to the stream start).
    ///
    /// Containers often store rounded timestamps (MKV: 1 ms), so a frame that starts at exactly
    /// `t` may be stamped slightly later. The decoder therefore aims at the middle of the source
    /// frame (`t` + half a frame at the stream's nominal rate) and returns the last frame whose
    /// timestamp is ≤ that point. For VFR or rate-less streams no bias is applied.
    pub fn frame_at(&mut self, t: Time) -> Result<FrameRef<'_>, MediaError> {
        let tb = self.info.time_base;
        let aim = match self.info.frame_rate.filter(|_| !self.info.vfr) {
            Some(fr) => {
                let half = Time::new(fr.den() as i64, 2 * fr.num())
                    .map_err(|e| MediaError::Decode(e.to_string()))?;
                t.checked_add(half)
                    .map_err(|e| MediaError::Decode(e.to_string()))?
            }
            None => t,
        };
        let ticks_t = aim
            .checked_div(tb)
            .map_err(|e| MediaError::Decode(e.to_string()))?;
        let ticks = ticks_t.num().div_euclid(ticks_t.den() as i64);
        let target = self.info.start_pts.saturating_add(ticks);
        let mut err = [0 as c_char; 256];
        let mut got = 0i64;
        let stride = self.info.width as c_int * 4;
        // SAFETY: buf holds width*height*4 bytes. The shim scales every decoded frame to the
        // stream dimensions reported by `arc_av_info` (even when a frame changes size mid-stream),
        // and rejects a stride smaller than width*4 (audit F-02).
        let r = unsafe {
            arc_av_video_at(
                self.h.0,
                target,
                self.buf.as_mut_ptr(),
                stride,
                &mut got,
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        match r {
            0 => {}
            1 => return Err(MediaError::EndOfStream),
            _ => return Err(MediaError::Decode(c_str(&err))),
        }
        let rel = got - self.info.start_pts;
        let time = Time::new(rel, 1)
            .and_then(|x| x.checked_mul(tb))
            .unwrap_or(Time::ZERO);
        Ok(FrameRef {
            width: self.info.width,
            height: self.info.height,
            rgba: &self.buf,
            time,
        })
    }
}

/// In-process audio decoder: interleaved stereo f32 at a fixed output rate.
pub struct AudioDecoder {
    h: Handle,
    info: StreamInfo,
    rate: u32,
}

impl std::fmt::Debug for AudioDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDecoder")
            .field("info", &self.info)
            .field("rate", &self.rate)
            .finish()
    }
}

impl AudioDecoder {
    pub fn open(path: &Path, out_rate: u32) -> Result<AudioDecoder, MediaError> {
        let (h, info) = open(path, 1)?;
        Ok(AudioDecoder {
            h,
            info,
            rate: out_rate,
        })
    }

    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Fill `dst` (interleaved stereo, len = 2 × frames) starting at output sample `start`.
    /// Samples outside the stream are zero. Returns frames that came from the stream.
    pub fn read(&mut self, start: i64, dst: &mut [f32]) -> Result<usize, MediaError> {
        let frames = (dst.len() / 2) as c_int;
        let mut err = [0 as c_char; 256];
        // SAFETY: dst has room for `frames` interleaved stereo frames.
        let r = unsafe {
            arc_av_audio_read(
                self.h.0,
                self.rate as c_int,
                start,
                frames,
                dst.as_mut_ptr(),
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        if r < 0 {
            return Err(MediaError::Decode(c_str(&err)));
        }
        Ok(r as usize)
    }

    /// Read starting at a source time (exact → sample index, floor).
    pub fn read_at(&mut self, t: Time, dst: &mut [f32]) -> Result<usize, MediaError> {
        let start = arc_core::time::sample_index(t, self.rate, Round::Floor)
            .map_err(|e| MediaError::Decode(e.to_string()))?;
        self.read(start, dst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u32, minor: u32, micro: u32) -> u32 {
        (major << 16) | (minor << 8) | micro
    }

    fn licenses() -> [String; 5] {
        std::array::from_fn(|_| APPROVED_LICENSE.to_string())
    }

    #[test]
    fn runtime_check_accepts_the_pinned_build() {
        let b = [v(61, 19, 100), v(63, 24, 100), v(63, 16, 100), v(10, 5, 100), v(7, 6, 100)];
        assert!(validate_runtime(&b, &b, &licenses()).is_empty());
        // A later 9.x minor is fine (ABI rule within a major).
        let r = [v(61, 20, 100), v(63, 26, 100), v(63, 17, 100), v(10, 6, 100), v(7, 7, 100)];
        assert!(validate_runtime(&b, &r, &licenses()).is_empty());
    }

    #[test]
    fn runtime_check_refuses_major_minor_and_licence_mismatches() {
        let b = [v(61, 19, 100), v(63, 24, 100), v(63, 16, 100), v(10, 5, 100), v(7, 6, 100)];
        let mut r = b;
        r[1] = v(62, 24, 100);
        assert!(validate_runtime(&b, &r, &licenses()).iter().any(|p| p.contains("major")));
        let mut r = b;
        r[4] = v(7, 5, 100);
        assert!(validate_runtime(&b, &r, &licenses()).iter().any(|p| p.contains("older")));
        let mut l = licenses();
        l[2] = "GPL version 3 or later".into();
        assert!(validate_runtime(&b, &b, &l).iter().any(|p| p.contains("GPL")));
        let mut l = licenses();
        l[0] = "nonfree software".into();
        assert!(validate_runtime(&b, &b, &l).iter().any(|p| p.contains("nonfree")));
        let mut l = licenses();
        l[3] = "LGPL version 3 or later".into();
        assert!(validate_runtime(&b, &b, &l).iter().any(|p| p.contains("approved")));
    }
}
