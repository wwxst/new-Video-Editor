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
    run_ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=2",
            "-c:v",
            "mpeg2video",
            "-q:v",
            "2",
            "-f",
            "mpegts",
            "a.ts",
        ],
    );
    run_ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30:duration=2",
            "-c:v",
            "mpeg2video",
            "-q:v",
            "2",
            "-f",
            "mpegts",
            "b.ts",
        ],
    );
    let mut bytes = std::fs::read(dir.join("a.ts")).expect("a.ts");
    bytes.extend_from_slice(&std::fs::read(dir.join("b.ts")).expect("b.ts"));
    let path = dir.join("change.ts");
    std::fs::write(&path, bytes).expect("write");
    path
}

/// The stream reports 320x180. Frames that arrive as 640x360 must be scaled into the buffer the
/// decoder reported, never written at their own stride.
#[test]
fn resolution_change_is_scaled_to_the_reported_size() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = resolution_change_ts(dir.path());
    let mut d = VideoDecoder::open(&path).expect("open");
    assert_eq!((d.info().width, d.info().height), (320, 180));
    let rate = FrameRate::Fps30;
    for n in 0..120i64 {
        let f = d
            .frame_at(rate.time_of_frame(n))
            .unwrap_or_else(|e| panic!("frame {n}: {e}"));
        assert_eq!((f.width, f.height), (320, 180), "frame {n}");
        assert_eq!(f.rgba.len(), 320 * 180 * 4, "frame {n}");
    }
    // The second half is a different source size; it must still decode (scaled), not fail.
    let f = d
        .frame_at(rate.time_of_frame(90))
        .expect("frame in second segment");
    assert_eq!((f.width, f.height), (320, 180));
    assert!(
        f.rgba.chunks_exact(4).any(|p| p[0] != 0 || p[1] != 0 || p[2] != 0),
        "scaled frame has content"
    );
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
