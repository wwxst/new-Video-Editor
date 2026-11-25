//! Safe wrappers over the libav shim (`csrc/arc_av.c`).
//!
//! Error codes and Rust panics are recoverable per clip; a native fault inside FFmpeg is not, which
//! is why untested media goes through the subprocess import check first (docs/04 §1.1–1.2).
#![allow(unsafe_code)]

use arc_core::time::{Ratio, Round, Time};
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::Path;

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
fn file_url(path: &Path) -> Result<CString, MediaError> {
    let canon = std::fs::canonicalize(path).map_err(|_| MediaError::Path)?;
    // Map `\\?\C:\...` to a drive path and `\\?\UNC\server\...` back to `\\server\...`; a
    // verbatim prefix left in place, or stripped to `UNC\...`, makes FFmpeg treat NAS media as a
    // relative path.
    let s = crate::paths::normalize_verbatim(&canon.to_string_lossy());
    if !crate::paths::is_local_absolute(&s) {
        return Err(MediaError::Path);
    }
    CString::new(format!("file:{s}")).map_err(|_| MediaError::Path)
}

fn open(path: &Path, kind: c_int) -> Result<(Handle, StreamInfo), MediaError> {
    let p = file_url(path)?;
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
