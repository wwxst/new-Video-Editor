//! FFmpeg 8.1.3 vs 9.1 differential harness (spec §8.3, AC-7/AC-9/AC-10).
//!
//! The test is skipped unless `ARCLINE_CORPUS_DIR` points at the corpus generated in step 1; the
//! baseline (`tests/baseline/ffmpeg-8.1.3.json`) records both the decode results and the hash of
//! every corpus file. See `tests/support/mod.rs` for the environment and comparison rules.

mod support;

#[test]
fn corpus_differential() {
    support::run();
}
