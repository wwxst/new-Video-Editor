//! Runtime identity of the loaded FFmpeg and of the resolved CLI tools (spec §4.5/§4.6, AC-1/AC-3).

use arc_media::libav::{APPROVED_LICENSE, runtime_report};
use arc_media::tools;

#[test]
fn runtime_identity_matches_the_pinned_package() {
    let r = runtime_report();
    assert!(r.ident.starts_with("9.1"), "ident {}", r.ident);
    assert_eq!(r.license, APPROVED_LICENSE);
    assert!(!r.override_used, "no override expected in the test environment");
    assert_ne!(r.build_id, "unpinned", "manifest hash is baked in");
    assert_eq!(r.build_id.len(), 64, "sha256 hex");
    for (name, expect_major) in [
        ("avutil", "61"),
        ("avcodec", "63"),
        ("avformat", "63"),
        ("swscale", "10"),
        ("swresample", "7"),
    ] {
        let v = r.libraries.get(name).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(
            v.split('.').next(),
            Some(expect_major),
            "{name} version {v}"
        );
    }
}

#[test]
fn tools_are_verified_and_the_encoder_set_is_the_shipped_one() {
    tools::verify().expect("bundled ffmpeg/ffprobe match the pinned build");
    let enc = tools::encoders();
    for expected in ["ffv1", "libsvtav1", "prores_ks", "h264_qsv", "aac", "flac"] {
        assert!(
            enc.iter().any(|e| e == expected),
            "encoder {expected} missing from {enc:?}"
        );
    }
    for forbidden in ["libopenh264", "h264_mf", "hevc_mf"] {
        assert!(
            !enc.iter().any(|e| e == forbidden),
            "encoder {forbidden} must not exist (D2/F-19)"
        );
    }
}

/// AC-8: the software AV1 encoder (`libsvtav1`, enabled by the configure profile) really encodes,
/// and the result decodes through the shim. Encoder selection may prefer a hardware AV1 encoder.
#[test]
fn libsvtav1_encodes_and_the_result_decodes() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("av1.mkv");
    let ffmpeg = tools::ffmpeg_checked().expect("ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:s=320x180:d=0.2:r=30",
            "-frames:v",
            "5",
            "-c:v",
            "libsvtav1",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&out)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "libsvtav1 encode failed");
    let mut d = arc_media::VideoDecoder::open(&out).expect("decode libsvtav1 output");
    let f = d.frame_at(arc_core::Time::ZERO).expect("frame");
    assert_eq!((f.width, f.height), (320, 180));
}
