//! Robustness corpus (docs/08 §6): malformed media must fail as a structured error at the decode
//! boundary and be marked unavailable, never crash the process.
//!
//! Every fixture is generated here (no third-party media) and stays well below 2 MB. The test does
//! not assert *which* error each file produces — only that the import check refuses it and that
//! opening/decoding either succeeds or returns a typed `MediaError`; a panic or an abort fails the
//! test. The mixed-media export test in `arc-cli/tests/render_e2e.rs` proves the engine keeps
//! rendering when one asset is unavailable.

use arc_core::model::AssetKind;
use arc_core::time::FrameRate;
use arc_media::fixtures::write_counter_video;
use arc_media::probe::{ImportOutcome, import_check};
use arc_media::{AudioDecoder, MediaError, VideoDecoder};
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_FIXTURE_BYTES: u64 = 2 * 1024 * 1024;

fn ffmpeg() -> PathBuf {
    arc_media::tools::ffmpeg()
}

fn run_ffmpeg(dir: &Path, args: &[&str]) {
    let o = Command::new(ffmpeg())
        .args(["-hide_banner", "-v", "error", "-y"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run ffmpeg");
    assert!(
        o.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// A small valid FFV1/MKV source the corrupt fixtures are derived from.
fn base_media(dir: &Path) -> PathBuf {
    let p = dir.join("base.mkv");
    write_counter_video(&p, 120, FrameRate::Fps30.rate(), 160, 90, false).expect("4 s base fixture");
    p
}

fn bytes_of(p: &Path) -> Vec<u8> {
    std::fs::read(p).expect("read base")
}

/// Truncate to `fraction` of the file.
fn truncated(dir: &Path, base: &[u8], name: &str, fraction: f64) -> PathBuf {
    let p = dir.join(name);
    let n = ((base.len() as f64) * fraction) as usize;
    std::fs::write(&p, &base[..n]).expect("write");
    p
}

/// Overwrite `len` bytes at `offset` with a fixed pattern.
fn scribble(dir: &Path, base: &[u8], name: &str, offset: usize, len: usize, pattern: u8) -> PathBuf {
    let p = dir.join(name);
    let mut bytes = base.to_vec();
    let end = (offset + len).min(bytes.len());
    bytes[offset..end].fill(pattern);
    std::fs::write(&p, bytes).expect("write");
    p
}

/// A malformed fixture. `refused` means the import check must reject it (Offline/Quarantined);
/// a degraded file parses and decodes (with corrupt pixels), but must still never panic.
struct Fixture {
    name: &'static str,
    path: PathBuf,
    refused: bool,
}

/// Build the whole malformed corpus. Every fixture is smaller than 2 MB.
fn corpus(dir: &Path) -> Vec<Fixture> {
    let base_path = base_media(dir);
    let base = bytes_of(&base_path);
    let mut out: Vec<Fixture> = Vec::new();
    let mut add = |name: &'static str, path: PathBuf, refused: bool| {
        out.push(Fixture { name, path, refused })
    };

    // Empty and non-media files.
    let p = dir.join("empty.mkv");
    std::fs::write(&p, b"").expect("empty");
    add("empty file", p, true);
    let p = dir.join("text.mp4");
    std::fs::write(&p, b"this is not a video file\n").expect("text");
    add("text renamed .mp4", p, true);

    // Truncated container / truncated header.
    add(
        "truncated to 25%",
        truncated(dir, &base, "truncated.mkv", 0.25),
        true,
    );
    add("truncated to 50%", truncated(dir, &base, "half.mkv", 0.50), true);
    add(
        "EBML magic zeroed",
        scribble(dir, &base, "no_magic.mkv", 0, 16, 0x00),
        true,
    );

    // Corrupted header: the first 512 bytes are overwritten.
    add(
        "header overwritten",
        scribble(dir, &base, "bad_header.mkv", 0, 512, 0xFF),
        true,
    );

    // Corrupted payload in the middle: the container and index still parse, and FFV1 conceals the
    // damage, so this one is expected to decode (with corrupt frames) — the contract is no crash.
    let mid = base.len() / 2;
    add(
        "payload corrupted mid-file",
        scribble(dir, &base, "mid_corrupt.mkv", mid, 4096, 0xA5),
        false,
    );

    // A valid MP4 whose `moov` index is scribbled over: the index claims a layout that is not there.
    let mp4 = dir.join("base.mp4");
    run_ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=30:duration=1",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "base.mp4",
        ],
    );
    let mp4_bytes = bytes_of(&mp4);
    let moov = mp4_bytes
        .windows(4)
        .position(|w| w == b"moov")
        .expect("moov box");
    add(
        "MP4 index (moov) overwritten",
        scribble(dir, &mp4_bytes, "bad_index.mp4", moov, 256, 0x5A),
        true,
    );

    // A JPEG renamed to a video extension.
    let jpg = dir.join("still.jpg");
    run_ffmpeg(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=1",
            "-frames:v",
            "1",
            "still.jpg",
        ],
    );
    let p = dir.join("jpeg_as_video.mkv");
    std::fs::copy(&jpg, &p).expect("copy jpeg");
    // ffprobe identifies media by content, so a JPEG with a video extension is a valid still image,
    // not a malformed file: it must import as `Image`, and the point is that content wins.
    add("JPEG renamed .mkv (valid image)", p, false);

    for f in &out {
        let size = std::fs::metadata(&f.path).map(|m| m.len()).unwrap_or(0);
        assert!(
            size < MAX_FIXTURE_BYTES,
            "{}: fixture is {size} bytes, over the 2 MB limit",
            f.name
        );
    }
    out
}

/// Every malformed fixture is refused by the import check (never `Ok`) when it is unreadable, and
/// the decode boundary either refuses it or reports a typed error — no panic, no abort.
#[test]
fn malformed_media_is_refused_with_a_structured_error() {
    let dir = tempfile::tempdir().expect("tmp");
    let fixtures = corpus(dir.path());
    assert!(fixtures.len() >= 8, "corpus too small");
    for f in &fixtures {
        let name = f.name;
        let path = &f.path;
        let outcome = import_check(path);
        if f.refused {
            assert!(
                !matches!(outcome, ImportOutcome::Ok { .. }),
                "{name}: the import check accepted a malformed file: {outcome:?}"
            );
        }
        match outcome {
            ImportOutcome::Ok { .. } => {}
            ImportOutcome::Quarantined { reason, .. } => {
                assert!(!reason.is_empty(), "{name}: empty quarantine reason")
            }
            ImportOutcome::Offline { reason } => {
                assert!(!reason.is_empty(), "{name}: empty offline reason")
            }
        }

        // Video: opening either fails with a typed error, or every frame request is a typed
        // result. A panic or abort here fails the test.
        match VideoDecoder::open(path) {
            Err(e) => assert_typed(name, &e),
            Ok(mut d) => {
                for k in 0..10i64 {
                    if let Err(e) = d.frame_at(FrameRate::Fps30.time_of_frame(k)) {
                        assert_typed(name, &e);
                        break;
                    }
                }
                // past the end of every fixture: a typed result, never a crash
                if let Err(e) = d.frame_at(arc_core::Time::secs(10)) {
                    assert_typed(name, &e);
                }
            }
        }

        // Audio: same contract, with a read and a seek.
        match AudioDecoder::open(path, 48_000) {
            Err(e) => assert_typed(name, &e),
            Ok(mut a) => {
                let mut buf = vec![0f32; 2 * 1024];
                if let Err(e) = a.read(0, &mut buf) {
                    assert_typed(name, &e);
                }
                if let Err(e) = a.read(48_000, &mut buf) {
                    assert_typed(name, &e);
                }
            }
        }
    }
}

fn assert_typed(name: &str, e: &MediaError) {
    match e {
        MediaError::Decode(msg) => assert!(!msg.is_empty(), "{name}: empty decode message"),
        MediaError::Path => {}
        MediaError::EndOfStream => {}
    }
}