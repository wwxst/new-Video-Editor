//! Mid-stream parameter changes in the libav shim (audit F-02, F-03; experiments J and K).
//!
//! The subprocess import check classifies both fixtures as `ok`; the in-process decoder must be
//! safe on them by construction, because the CLI's filters reconfigure and the shim must too.

use arc_core::time::FrameRate;
use arc_media::{AudioDecoder, VideoDecoder};
use std::path::{Path, PathBuf};
use std::process::Command;

fn run_ffmpeg(dir: &Path, args: &[&str]) {
    let out = Command::new(arc_media::tools::ffmpeg())
        .args(["-hide_banner", "-v", "error", "-y"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Experiment J: MPEG-2 320x180 for 2 s, then 640x360 for 2 s, concatenated as an MPEG-TS
/// elementary stream (byte concatenation keeps working with the shipped build, which disables the
/// `concat` demuxer/protocol for security; spec S2/S3).
fn resolution_change_ts(dir: &Path) -> PathBuf {
    encode_ts_segment(dir, "a.ts", "320x180", &[]);
    encode_ts_segment(dir, "b.ts", "640x360", &[]);
    let mut bytes = std::fs::read(dir.join("a.ts")).expect("a.ts");
    bytes.extend_from_slice(&std::fs::read(dir.join("b.ts")).expect("b.ts"));
    let path = dir.join("change.ts");
    std::fs::write(&path, bytes).expect("write");
    path
}

fn encode_ts_segment(dir: &Path, name: &str, size: &str, extra: &[&str]) {
    let src = format!("testsrc2=size={size}:rate=30:duration=2");
    let mut args = vec!["-f", "lavfi", "-i", src.as_str()];
    args.extend_from_slice(&["-c:v", "mpeg2video", "-q:v", "2", "-f", "mpegts"]);
    args.extend_from_slice(extra);
    args.push(name);
    run_ffmpeg(dir, &args);
}

/// The audited shape (experiment J): one MKV with *continuous* timestamps, no concat demuxer.
/// Segment B is muxed with `-output_ts_offset` so its timestamps follow A's, then the two TS byte
/// streams are remuxed to MKV with stream copy.
fn resolution_change_continuous_mkv(dir: &Path) -> PathBuf {
    encode_ts_segment(dir, "a_cont.ts", "320x180", &[]);
    encode_ts_segment(
        dir,
        "b_cont.ts",
        "640x360",
        &["-output_ts_offset", "2.1"],
    );
    let mut bytes = std::fs::read(dir.join("a_cont.ts")).expect("a_cont.ts");
    bytes.extend_from_slice(&std::fs::read(dir.join("b_cont.ts")).expect("b_cont.ts"));
    std::fs::write(dir.join("cont.ts"), bytes).expect("write cont.ts");
    run_ffmpeg(
        dir,
        &[
            "-f", "mpegts", "-i", "cont.ts", "-c", "copy", "-map", "0", "cont.mkv",
        ],
    );
    dir.join("cont.mkv")
}

fn check_resolution_change(path: &Path, min_frames: usize, random_access: bool) {
    let mut d = VideoDecoder::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    assert_eq!((d.info().width, d.info().height), (320, 180), "{path:?}");
    let rate = FrameRate::Fps30;
    let mut n = 0usize;
    for i in 0..200i64 {
        match d.frame_at(rate.time_of_frame(i)) {
            Ok(f) => {
                assert_eq!((f.width, f.height), (320, 180), "{path:?} frame {i}");
                assert_eq!(f.rgba.len(), 320 * 180 * 4, "{path:?} frame {i}");
                if i >= 60 {
                    assert!(
                        f.rgba.chunks_exact(4).any(|p| p[0] != 0 || p[1] != 0 || p[2] != 0),
                        "{path:?}: second-segment frame {i} has content"
                    );
                }
                n += 1;
            }
            Err(arc_media::MediaError::EndOfStream) => break,
            Err(e) => panic!("{path:?} frame {i}: {e}"),
        }
    }
    assert!(n >= min_frames, "{path:?} decoded only {n} frames");
    if !random_access {
        return;
    }
    // Random access uses a fresh decoder, as the audit's per-frame process experiment did.
    let mut d = VideoDecoder::open(path).unwrap_or_else(|e| panic!("reopen {path:?}: {e}"));
    for i in [90i64, 10, 100, 0, 60] {
        if (i as usize) < n {
            let f = d
                .frame_at(rate.time_of_frame(i))
                .unwrap_or_else(|e| panic!("{path:?} random frame {i}: {e}"));
            assert_eq!((f.width, f.height), (320, 180), "{path:?} random {i}");
        }
    }
}

/// The stream reports 320x180. Frames that arrive as 640x360 must be scaled into the buffer the
/// decoder reported, never written at their own stride.
#[test]
fn resolution_change_is_scaled_to_the_reported_size() {
    let dir = tempfile::tempdir().expect("tmp");
    // Byte-concatenated TS: sequential access only (un-indexed TS random access is audit F-08,
    // a P2 item; the fixture's second segment restarts timestamps).
    check_resolution_change(&resolution_change_ts(dir.path()), 120, false);
    // The audited single-MKV shape with continuous timestamps; MKV random access is exact (F-08
    // lists MPEG-2 in MKV as passing). The TS join loses at most one frame in the demuxer.
    check_resolution_change(&resolution_change_continuous_mkv(dir.path()), 110, true);
}

/// Experiment K: MP2 elementary stream, stereo for 2 s then mono for 2 s. The decoder must rebuild
/// its resampler (draining the old one first) instead of feeding a mono frame to a stereo one.
#[test]
fn audio_channel_change_is_resampled_safely() {
    let dir = tempfile::tempdir().expect("tmp");
    run_ffmpeg(
        dir.path(),
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-ac",
            "2",
            "-c:a",
            "mp2",
            "-b:a",
            "192k",
            "stereo.mp2",
        ],
    );
    run_ffmpeg(
        dir.path(),
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000:duration=2",
            "-ac",
            "1",
            "-c:a",
            "mp2",
            "-b:a",
            "96k",
            "mono.mp2",
        ],
    );
    let mut bytes = std::fs::read(dir.path().join("stereo.mp2")).expect("stereo");
    bytes.extend_from_slice(&std::fs::read(dir.path().join("mono.mp2")).expect("mono"));
    let path = dir.path().join("change.mp2");
    std::fs::write(&path, bytes).expect("write");

    let mut a = AudioDecoder::open(&path, 48_000).expect("open");
    // One continuous read crosses the parameter change with no reposition in between.
    let mut long = vec![0f32; 2 * 192_000];
    let got = a.read(0, &mut long).expect("long read");
    assert_eq!(got, 192_000, "all four seconds decode in one read");
    let rms = (long.iter().map(|s| s * s).sum::<f32>() / long.len() as f32).sqrt();
    assert!(rms > 0.02, "both segments carry audio (rms {rms})");
    // The mixer reads a block plus one interpolation sample; the parameter change lands mid-read.
    let mut buf = vec![0f32; 2 * 4097];
    let mut got_total = 0i64;
    for start in (0..192_000).step_by(4096) {
        let got = a
            .read(start, &mut buf)
            .unwrap_or_else(|e| panic!("block {start}: {e}"));
        assert!(got > 0, "block {start} produced samples");
        got_total += got as i64;
    }
    assert!(got_total >= 3 * 48_000, "all four seconds decode: {got_total}");

    // Both segments carry a tone; a mis-configured resampler would silence or crash on one.
    for start in [0i64, 48_000, 96_000, 144_000] {
        a.read(start, &mut buf).expect("re-read");
        let rms = (buf[..2 * 4096].iter().map(|s| s * s).sum::<f32>() / (2.0 * 4096.0)).sqrt();
        assert!(rms > 0.05, "tone at sample {start} (rms {rms})");
    }
}
