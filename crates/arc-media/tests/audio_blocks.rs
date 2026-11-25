//! F-06: the mixer's block pattern must not disturb the decoder.
//!
//! `Mixer::process` reads `last − first + 2` frames per block — one frame of look-ahead — so
//! consecutive reads overlap by one sample. The shim used to treat that overlap as a backward seek
//! and reset the decoder and the resampler on every block, which corrupted lossy and resampled
//! audio every 4,096 samples (audit F-06, experiment D/D′). Reads are now served from a buffer
//! with a history window, so a block pattern and one long read must be identical sample for
//! sample.
//!
//! AAC is included: both paths start a fresh decoder at position 0, so the decode history is the
//! same on both sides (F-25 only forbids comparing two different decode histories).

use arc_media::AudioDecoder;
use std::path::{Path, PathBuf};
use std::process::Command;

const RATE: u32 = 48_000;
const TOTAL: usize = 96_000; // 2 s
const BLOCK: usize = 4096;

fn generate(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let o = Command::new(arc_media::tools::ffmpeg())
        .args(["-hide_banner", "-v", "error", "-y"])
        .args(args)
        .arg(&out)
        .output()
        .expect("run ffmpeg");
    assert!(
        o.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    out
}

fn tone(rate: u32) -> String {
    format!("sine=frequency=1000:sample_rate={rate}:duration=2")
}

/// One long read from position 0.
fn long_read(path: &Path) -> Vec<f32> {
    let mut d = AudioDecoder::open(path, RATE).expect("open");
    let mut buf = vec![0f32; TOTAL * 2];
    let got = d.read(0, &mut buf).expect("long read");
    assert!(got >= TOTAL - 64, "{path:?}: only {got} frames");
    buf
}

/// The mixer's pattern: advance by BLOCK, read BLOCK + 2 frames (the mixer reads `last − first + 2`).
fn assert_block_pattern_matches(path: &Path, long: &[f32]) {
    let mut d = AudioDecoder::open(path, RATE).expect("open");
    let mut start = 0i64;
    while start < TOTAL as i64 {
        let mut buf = vec![0f32; (BLOCK + 2) * 2];
        d.read(start, &mut buf).expect("block read");
        let n = (TOTAL as i64 - start).min(BLOCK as i64) as usize;
        for k in 0..n {
            let i = start as usize + k;
            assert_eq!(
                buf[2 * k].to_bits(),
                long[2 * i].to_bits(),
                "{path:?}: sample {i} differs from the continuous decode (block starting {start})"
            );
        }
        start += BLOCK as i64;
    }
}

#[test]
fn lossless_and_native_rate_audio_survive_the_block_pattern() {
    let dir = tempfile::tempdir().expect("tmp");
    let cases = vec![
        (
            "flac48.flac",
            generate(
                dir.path(),
                "flac48.flac",
                &["-f", "lavfi", "-i", &tone(48_000), "-c:a", "flac"],
            ),
        ),
        (
            "mp3",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sine_48k.mp3"),
        ),
        (
            "opus.ogg",
            generate(
                dir.path(),
                "opus.ogg",
                &[
                    "-f", "lavfi", "-i", &tone(48_000), "-c:a", "opus", "-strict", "experimental",
                    "-b:a", "96k",
                ],
            ),
        ),
        (
            "opus.mkv",
            generate(
                dir.path(),
                "opus.mkv",
                &[
                    "-f", "lavfi", "-i", &tone(48_000), "-c:a", "opus", "-strict", "experimental",
                    "-b:a", "96k",
                ],
            ),
        ),
        (
            "aac.m4a",
            generate(
                dir.path(),
                "aac.m4a",
                &["-f", "lavfi", "-i", &tone(48_000), "-c:a", "aac", "-b:a", "128k"],
            ),
        ),
    ];
    for (name, path) in cases {
        let long = long_read(&path);
        assert_block_pattern_matches(&path, &long);
        println!("{name}: block pattern identical to the long read");
    }
}

#[test]
fn resampled_audio_survives_the_block_pattern() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = generate(
        dir.path(),
        "flac44.flac",
        &["-f", "lavfi", "-i", &tone(44_100), "-c:a", "flac"],
    );
    let long = long_read(&path);
    assert_block_pattern_matches(&path, &long);
}
