//! Mid-stream parameter changes in the libav shim (audit F-02, F-03; experiments J and K).
//!
//! The subprocess import check classifies both fixtures as `ok`; the in-process decoder must be
//! safe on them by construction, because the CLI's filters reconfigure and the shim must too.

use arc_core::time::FrameRate;
use arc_media::VideoDecoder;
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

/// Experiment J: MPEG-2 320x180 for 2 s, then 640x360 for 2 s, concatenated into one MKV.
fn resolution_change_mkv(dir: &Path) -> PathBuf {
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
            "a.mkv",
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
            "b.mkv",
        ],
    );
    std::fs::write(dir.join("list.txt"), "file 'a.mkv'\nfile 'b.mkv'\n").expect("list");
    run_ffmpeg(
        dir,
        &[
            "-f", "concat", "-safe", "0", "-i", "list.txt", "-c", "copy", "-map", "0", "change.mkv",
        ],
    );
    dir.join("change.mkv")
}

/// The stream reports 320x180. Frames that arrive as 640x360 must be scaled into the buffer the
/// decoder reported, never written at their own stride.
#[test]
fn resolution_change_is_scaled_to_the_reported_size() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = resolution_change_mkv(dir.path());
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
