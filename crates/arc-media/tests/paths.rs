//! Local-file-only opening (audit F-09, spec S1): document asset paths are untrusted, so the
//! decoder canonicalises a real local file and opens it as `file:` only.

use arc_core::FrameRate;
use arc_media::fixtures::write_counter_video;
use arc_media::{AudioDecoder, MediaError, VideoDecoder};
use std::path::Path;

fn fixture(dir: &Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    write_counter_video(&p, 30, FrameRate::Fps30.rate(), 160, 90, true).expect("fixture");
    p
}

#[test]
fn decoder_refuses_non_local_paths_before_io() {
    for p in [
        "http://127.0.0.1:9/x.mp4",
        "https://example.com/x.mkv",
        "handle:1",
        "concat:a|b",
        "file:C:/x.mp4",
        "udp://127.0.0.1:9",
    ] {
        let e = VideoDecoder::open(Path::new(p)).expect_err(p);
        assert!(matches!(e, MediaError::Path), "{p}: {e:?}");
        let e = AudioDecoder::open(Path::new(p), 48_000).expect_err(p);
        assert!(matches!(e, MediaError::Path), "{p}: {e:?}");
    }
    // A plain local path that does not exist is a path error, not a libav open attempt.
    let missing = std::env::temp_dir().join("arcline-does-not-exist-9f2.mkv");
    assert!(matches!(
        VideoDecoder::open(&missing),
        Err(MediaError::Path)
    ));
}

#[test]
fn decoder_opens_local_paths_with_spaces_and_non_ascii() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture(dir.path(), "counter a b \u{fc}#1.mkv");
    let mut d = VideoDecoder::open(&p).expect("open spaces/non-ascii");
    assert_eq!((d.info().width, d.info().height), (160, 90));
    let f = d.frame_at(arc_core::Time::ZERO).expect("frame");
    assert_eq!(
        arc_media::fixtures::read_counter(f.rgba, f.width, f.height),
        Some(0)
    );
    let mut a = AudioDecoder::open(&p, 48_000).expect("audio");
    let mut buf = vec![0f32; 2 * 1000];
    assert!(a.read(0, &mut buf).expect("read") > 0);
}
