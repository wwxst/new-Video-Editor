//! F-08: random access must return the requested frame, not a later one.
//!
//! `av_seek_frame(AVSEEK_FLAG_BACKWARD)` does not land before the target on every container: the
//! MP4 index is keyed by DTS, so a seek for a frame just before a keyframe can land past it, and
//! MPEG-TS has no index at all. The shim used to return the first decoded frame in that case,
//! silently showing a later frame (audit F-08, experiment G). It now backs off and seeks again
//! until a frame at or before the target has been decoded, and only a target at or before the
//! stream start may resolve to the stream's first frame.

use arc_core::time::FrameRate;
use arc_media::fixtures::{counter_frame, read_counter};
use arc_media::VideoDecoder;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const W: u32 = 320;
const H: u32 = 180;

fn ffmpeg() -> PathBuf {
    arc_media::tools::ffmpeg()
}

/// Encode `frames` counter frames (raw RGBA on stdin) with the given encoder arguments.
fn encode_counter(dir: &Path, name: &str, frames: u32, extra: &[&str]) {
    let mut cmd = Command::new(ffmpeg());
    cmd.args([
        "-hide_banner",
        "-v",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        "320x180",
        "-r",
        "30",
        "-i",
        "pipe:0",
    ])
    .args(extra)
    .arg(dir.join(name))
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn ffmpeg");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for i in 0..frames {
            stdin.write_all(&counter_frame(i, W, H)).expect("write frame");
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("ffmpeg");
    assert!(
        out.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn expect_frame(d: &mut VideoDecoder, rate: FrameRate, n: i64, what: &str) {
    let f = match d.frame_at(rate.time_of_frame(n)) {
        Ok(f) => f,
        Err(e) => panic!("{what}: requested frame {n} failed: {e}"),
    };
    let got = read_counter(f.rgba, f.width, f.height);

    assert_eq!(
        got,
        Some(n as u32),
        "{what}: requested frame {n} (got {got:?} at time {})",
        f.time
    );
}

/// Sequential playback, then random / backward / repeated access on the same decoder.
fn check_counter_stream(path: &Path, rate: FrameRate, frames: i64, what: &str) {
    let mut d = VideoDecoder::open(path).expect("open");
    for n in 0..frames {
        expect_frame(&mut d, rate, n, what);
    }
    for n in [13, 14, 28, 29, 44, 59, 0, frames - 1, 7, 60, 15, 30] {
        if n < frames {
            expect_frame(&mut d, rate, n, what);
        }
    }
    for n in (0..frames).rev() {
        expect_frame(&mut d, rate, n, what);
    }
}

/// A fresh decoder per position, as `render` and the preview workers use them.
fn check_fresh_decoders(path: &Path, rate: FrameRate, positions: &[i64], what: &str) {
    for &n in positions {
        let mut d = VideoDecoder::open(path).expect("open");
        expect_frame(&mut d, rate, n, what);
    }
}

fn frame_hash(rgba: &[u8]) -> String {
    blake3::hash(rgba).to_hex().to_string()
}

#[test]
fn mpeg2_open_gop_in_mkv_is_frame_exact() {
    let dir = tempfile::tempdir().expect("tmp");
    encode_counter(
        dir.path(),
        "mpeg2.mkv",
        90,
        &["-c:v", "mpeg2video", "-q:v", "2", "-bf", "2", "-g", "15"],
    );
    check_counter_stream(
        &dir.path().join("mpeg2.mkv"),
        FrameRate::Fps30,
        90,
        "MPEG-2/MKV",
    );
}

#[test]
fn mpeg2_ts_with_non_zero_start_is_frame_exact() {
    let dir = tempfile::tempdir().expect("tmp");
    encode_counter(
        dir.path(),
        "mpeg2.ts",
        90,
        &[
            "-c:v",
            "mpeg2video",
            "-q:v",
            "2",
            "-bf",
            "2",
            "-g",
            "15",
            "-f",
            "mpegts",
            "-output_ts_offset",
            "1.433",
        ],
    );
    let path = dir.path().join("mpeg2.ts");
    check_counter_stream(&path, FrameRate::Fps30, 90, "MPEG-2/TS");
    check_fresh_decoders(
        &path,
        FrameRate::Fps30,
        &[0, 13, 28, 44, 59, 75, 89],
        "MPEG-2/TS fresh",
    );
}

#[test]
fn mpeg4_part2_in_mp4_with_bframes_is_frame_exact() {
    let dir = tempfile::tempdir().expect("tmp");
    encode_counter(
        dir.path(),
        "mpeg4.mp4",
        90,
        &["-c:v", "mpeg4", "-q:v", "2", "-bf", "2", "-g", "15"],
    );
    let path = dir.path().join("mpeg4.mp4");
    check_counter_stream(&path, FrameRate::Fps30, 90, "MPEG-4/MP4");
    check_fresh_decoders(
        &path,
        FrameRate::Fps30,
        &[13, 14, 28, 29, 44, 59],
        "MPEG-4/MP4 fresh",
    );
}

/// The committed hardware H.264 fixture carries `testsrc2`, not a counter, so the frame identity is
/// checked by hash against the sequential decode.
#[test]
fn h264_with_bframes_returns_the_sequential_frame() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/counter_h264_qsv.mp4");
    let rate = FrameRate::Fps30;
    let mut d = VideoDecoder::open(&path).expect("open");
    let mut hashes = Vec::new();
    for n in 0..60 {
        let f = d.frame_at(rate.time_of_frame(n)).expect("frame");
        hashes.push(frame_hash(f.rgba));
    }
    for n in [59i64, 0, 30, 13, 14, 44, 7, 59, 1] {
        let f = d.frame_at(rate.time_of_frame(n)).expect("frame");
        assert_eq!(
            frame_hash(f.rgba),
            hashes[n as usize],
            "H.264/MP4: frame {n} differs from the sequential decode"
        );
    }
    for n in (0..60).rev() {
        let f = d.frame_at(rate.time_of_frame(n)).expect("frame");
        assert_eq!(
            frame_hash(f.rgba),
            hashes[n as usize],
            "H.264/MP4: backward frame {n} differs from the sequential decode"
        );
    }
}
