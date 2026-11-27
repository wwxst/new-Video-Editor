//! In-process decode accuracy and the subprocess import check (docs/04 §1–3).

use arc_core::model::AssetKind;
use arc_core::time::{FrameRate, Ratio, Time};
use arc_media::fixtures::{read_counter, write_counter_video};
use arc_media::probe::{ImportOutcome, import_check};
use arc_media::{AudioDecoder, VideoDecoder};

const W: u32 = 320;
const H: u32 = 180;

fn fixture(dir: &std::path::Path, rate: FrameRate, frames: u32) -> std::path::PathBuf {
    let p = dir.join(format!("counter_{}.mkv", rate.label()));
    write_counter_video(&p, frames, rate.rate(), W, H, true).expect("fixture");
    p
}

#[test]
fn frame_accurate_random_sequential_and_backward_access() {
    let dir = tempfile::tempdir().expect("tmp");
    for rate in [FrameRate::Fps29_97, FrameRate::Fps25] {
        let p = fixture(dir.path(), rate, 120);
        let mut d = VideoDecoder::open(&p).expect("open");
        assert_eq!(d.info().width, W);
        let check = |d: &mut VideoDecoder, n: i64| {
            let t = rate.time_of_frame(n);
            let f = d.frame_at(t).expect("frame");
            assert_eq!(
                read_counter(f.rgba, f.width, f.height),
                Some(n as u32),
                "{rate:?} frame {n}"
            );
            // the container stores 1 ms timestamps; the frame's own time is within 1 ms of exact
            let diff = f.time.checked_sub(t).expect("sub").to_f64_lossy().abs();
            assert!(diff <= 0.001, "{rate:?} frame {n}: pts {} vs {}", f.time, t);
        };
        for n in 0..120 {
            check(&mut d, n); // sequential
        }
        for n in [100, 3, 57, 57, 0, 119, 1, 60] {
            check(&mut d, n); // random, repeated, backward
        }
        // a time inside frame 10's interval shows frame 10
        let mid = rate
            .time_of_frame(10)
            .checked_add(Time::new(1, 1000).expect("t"))
            .expect("add");
        let f = d.frame_at(mid).expect("mid");
        assert_eq!(read_counter(f.rgba, W, H), Some(10));
    }
}

#[test]
fn displayed_at_is_interval_based_and_seek_bias_is_documented() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture(dir.path(), FrameRate::Fps24, 2);
    let mut d = VideoDecoder::open(&p).expect("open");
    // At a 30 fps project frame 1 the 24 fps source has not advanced: frame 0 is displayed until
    // 1/24 s. The interval-based API returns frame 0; `frame_at` is the half-frame seek helper and
    // returns frame 1 here. The preview/export paths use `frame_displayed_at` so they match the
    // timeline thumbnail.
    let t = Time::new(1, 30).expect("t");
    let displayed = d.frame_displayed_at(t).expect("displayed");
    assert_eq!(read_counter(displayed.rgba, W, H), Some(0));
    let next = displayed.next_time.expect("next frame PTS");
    assert!(
        (next.to_f64_lossy() - 1.0 / 24.0).abs() <= 0.0011,
        "frame 0's display interval ends at source frame 1 (within the 1 ms container tick), got {next}"
    );
    let seek = d.frame_at(t).expect("seek");
    assert_eq!(
        read_counter(seek.rgba, W, H),
        Some(1),
        "frame_at's half-frame bias is intentionally different and not used by the UI"
    );
}

#[test]
fn audio_decode_is_sample_addressed() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture(dir.path(), FrameRate::Fps30, 60); // 2 s of 1 kHz tone
    let mut a = AudioDecoder::open(&p, 48_000).expect("audio");
    let mut buf = vec![0f32; 2 * 4800];
    let got = a.read(0, &mut buf).expect("read");
    assert_eq!(got, 4800);
    let rms = (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt();
    assert!(rms > 0.05, "tone present (rms {rms})");
    // seek to 1.5 s, and reading past the end zero-fills
    let got = a
        .read_at(Time::new(3, 2).expect("t"), &mut buf)
        .expect("seek");
    assert_eq!(got, 4800);
    let got = a.read(96_000 - 100, &mut buf).expect("tail");
    assert!(got <= 100, "only 100 frames remain, got {got}");
    assert!(
        buf[2 * 200..].iter().all(|s| *s == 0.0),
        "zero-filled past the end"
    );
}

#[test]
fn import_check_classifies_media() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture(dir.path(), FrameRate::Fps30, 90);
    match import_check(&p) {
        ImportOutcome::Ok { kind, probe } => {
            assert_eq!(kind, AssetKind::Video);
            assert_eq!(
                probe.video.as_ref().map(|v| v.frame_rate),
                Ratio::new(30, 1).ok()
            );
            assert_eq!(probe.duration, Time::secs(3));
            assert!(probe.audio.is_some());
        }
        other => panic!("expected ok, got {other:?}"),
    }
    assert!(matches!(
        import_check(&dir.path().join("missing.mkv")),
        ImportOutcome::Offline { .. }
    ));
    let junk = dir.path().join("junk.mkv");
    std::fs::write(&junk, b"definitely not a video file").expect("write");
    assert!(matches!(import_check(&junk), ImportOutcome::Offline { .. }));
}
