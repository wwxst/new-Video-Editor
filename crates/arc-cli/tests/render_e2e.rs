//! End-to-end pipeline acceptance (docs/10 §3 Phase B exit): import → edit → render → decode,
//! frame-exact.

use arc_core::time::FrameRate;
use arc_media::encode::VideoCodec;
use arc_media::fixtures::{read_counter, write_counter_video};
use arc_media::{AudioDecoder, VideoDecoder};
use serde_json::{Value, json};
use std::process::Command;

/// True when this machine has a usable encoder for `codec`. The reference machine has hardware
/// H.264/HEVC (QSV); a CI runner without a GPU has neither, and the hardware-only codec paths
/// cannot be exercised there. The probe is still run, so the message shows what was tried.
fn has_encoder(codec: VideoCodec) -> bool {
    match arc_media::encode::select_encoder(codec) {
        Ok(name) => {
            eprintln!("encoder probe: {codec:?} -> {name}");
            true
        }
        Err(e) => {
            eprintln!("skipping the hardware-encoder assertions: {e}");
            false
        }
    }
}
fn cli(args: &[&str], stdin: Option<&str>) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_arcline-cli"));
    c.args(args);
    let out = match stdin {
        Some(input) => {
            use std::io::Write;
            let mut child = c
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(input.as_bytes())
                .expect("write");
            child.wait_with_output().expect("wait")
        }
        None => c.output().expect("run"),
    };
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    )
}

fn j(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("not json ({e}): {s}"))
}

/// Builds the project: V1 = source frames 10..40 then 100..120; A1 = the same media's audio.
fn build(dir: &std::path::Path) -> std::path::PathBuf {
    let rate = FrameRate::Fps29_97;
    let media = dir.join("media").join("counter.mkv");
    std::fs::create_dir_all(media.parent().expect("parent")).expect("mkdir");
    write_counter_video(&media, 150, rate.rate(), 320, 180, true).expect("fixture");

    let p = dir.join("e2e.arcline");
    let ps = p.to_str().expect("utf8");
    let (code, out, err) = cli(
        &[
            "new", ps, "--fps", "29.97", "--res", "320x180", "--name", "E2E",
        ],
        None,
    );
    assert_eq!(code, 0, "{out} {err}");

    let (code, out, err) = cli(&["import", ps, media.to_str().expect("utf8")], None);
    assert_eq!(code, 0, "{out} {err}");
    let imp = j(&out);
    assert_eq!(imp["checks"][0]["result"]["status"], json!("ok"), "{imp}");
    let asset = imp["imported"][0]["id"]
        .as_str()
        .expect("asset id")
        .to_string();

    let (_, out, _) = cli(&["describe", ps], None);
    assert_eq!(
        j(&out)["assets"][0]["path"],
        json!("media/counter.mkv"),
        "stored relative to the project"
    );

    let batch = json!([
        {"op":"track.add","id":"tr_v1","kind":"video"},
        {"op":"track.add","id":"tr_a1","kind":"audio"},
        {"op":"timeline.add_clip","id":"cl_a","track":"tr_v1","asset":asset,"start":"00:00:00:00","duration":"00:00:01:00","source_in":"00:00:00:10"},
        {"op":"timeline.add_clip","id":"cl_b","track":"tr_v1","asset":asset,"start":"00:00:01:00","duration":"00:00:00:20","source_in":"00:00:03:10"},
        {"op":"timeline.add_clip","id":"cl_m","track":"tr_a1","asset":asset,"start":"00:00:00:00","duration":"00:00:01:20"}
    ]);
    let (code, out, err) = cli(&["apply", ps], Some(&batch.to_string()));
    assert_eq!(code, 0, "{out} {err}");
    p
}

#[test]
fn render_is_frame_exact_with_audio() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = build(dir.path());
    let out = dir.path().join("out.mkv");
    let (code, stdout, err) = cli(
        &[
            "render",
            p.to_str().expect("utf8"),
            "-o",
            out.to_str().expect("utf8"),
            "--codec",
            "ffv1",
        ],
        None,
    );
    assert_eq!(code, 0, "{stdout} {err}");
    let report = j(&stdout);
    assert_eq!(report["frames"], json!(50));
    assert_eq!(report["encoder"], json!("ffv1"));
    assert_eq!(report["duration"], json!("00:00:01:20"));
    // Run-time identity block (spec §4.5 / AC-3).
    assert_eq!(
        report["ffmpeg"]["license"],
        json!("LGPL version 2.1 or later")
    );
    assert_eq!(report["ffmpeg"]["override"], json!(false));
    assert!(
        report["ffmpeg"]["ident"]
            .as_str()
            .is_some_and(|s| s.starts_with("9.1")),
        "{report}"
    );
    assert_eq!(report["ffmpeg"]["libraries"]["avcodec"], json!("63.24.100"));
    assert_eq!(
        report["ffmpeg"]["build_id"].as_str().map(str::len),
        Some(64),
        "manifest sha256"
    );
    assert!(
        report["warnings"].as_array().is_some_and(Vec::is_empty),
        "{report}"
    );
    let mut rp = out.clone().into_os_string();
    rp.push(".report.json");
    assert!(std::path::Path::new(&rp).exists());

    // every output frame shows exactly the planned source frame
    let rate = FrameRate::Fps29_97;
    let mut d = VideoDecoder::open(&out).expect("open output");
    for k in 0..50i64 {
        let f = d.frame_at(rate.time_of_frame(k)).expect("frame");
        let expected = if k < 30 { 10 + k } else { 100 + (k - 30) };
        assert_eq!(
            read_counter(f.rgba, f.width, f.height),
            Some(expected as u32),
            "output frame {k}"
        );
    }
    // audio is present for the whole range
    let mut a = AudioDecoder::open(&out, 48_000).expect("audio");
    let mut buf = vec![0f32; 2 * 48_000];
    let got = a.read(0, &mut buf).expect("read");
    assert!(got >= 48_000, "{got}");
    let rms = (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt();
    assert!(rms > 0.05, "tone survives the mix and mux (rms {rms})");
}

#[test]
fn av1_and_prores_exports_are_decodable() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = build(dir.path());
    let ps = p.to_str().expect("utf8");

    // AV1 through libsvtav1 (the only software AV1 encoder in the shipped profile).
    let av1 = dir.path().join("out_av1.mkv");
    let (code, stdout, err) = cli(
        &[
            "render",
            ps,
            "-o",
            av1.to_str().expect("utf8"),
            "--codec",
            "av1",
            "--from",
            "00:00:00:00",
            "--to",
            "00:00:00:10",
        ],
        None,
    );
    assert_eq!(code, 0, "{stdout} {err}");
    let r = j(&stdout);
    let av1_encoder = r["encoder"].as_str().unwrap_or_default();
    assert!(
        ["av1_qsv", "av1_nvenc", "av1_amf", "libsvtav1"].contains(&av1_encoder),
        "{av1_encoder} ({r})"
    );
    let mut d = VideoDecoder::open(&av1).expect("decode av1");
    let f = d.frame_at(arc_core::Time::ZERO).expect("av1 frame");
    assert_eq!((f.width, f.height), (320, 180));

    // ProRes (prores_ks) is decodable by the same decoder.
    let pro = dir.path().join("out_prores.mov");
    let (code, stdout, err) = cli(
        &[
            "render",
            ps,
            "-o",
            pro.to_str().expect("utf8"),
            "--codec",
            "prores",
            "--from",
            "00:00:00:00",
            "--to",
            "00:00:00:10",
        ],
        None,
    );
    assert_eq!(code, 0, "{stdout} {err}");
    assert_eq!(j(&stdout)["encoder"], json!("prores_ks"));
    let mut d = VideoDecoder::open(&pro).expect("decode prores");
    let f = d.frame_at(arc_core::Time::ZERO).expect("prores frame");
    assert_eq!((f.width, f.height), (320, 180));
}

/// Spec §8.4: DNxHD exports (SMPTE VC-3) produce decodable files. The DNxHD encoder only accepts
/// the standard VC-3 frame sizes, so this uses a 1920x1080 project.
#[test]
fn dnxhd_exports_are_decodable() {
    let dir = tempfile::tempdir().expect("tmp");
    let media = dir.path().join("counter1080.mkv");
    write_counter_video(
        &media,
        15,
        FrameRate::Fps30.rate(),
        1920,
        1080,
        false,
    )
    .expect("fixture");
    let p = dir.path().join("dnx.arcline");
    let ps = p.to_str().expect("utf8");
    let (code, out, err) = cli(
        &["new", ps, "--fps", "30", "--res", "1920x1080", "--name", "DNx"],
        None,
    );
    assert_eq!(code, 0, "{out} {err}");
    let batch = json!([
        {"op":"asset.import","id":"as_d","path":media.to_str().expect("utf8"),"kind":"video"},
        {"op":"track.add","id":"tr_v1","kind":"video"},
        {"op":"timeline.add_clip","id":"cl_d","track":"tr_v1","asset":"as_d","start":"00:00:00:00","duration":"00:00:00:10"}
    ]);
    let (code, out, err) = cli(&["apply", ps], Some(&batch.to_string()));
    assert_eq!(code, 0, "{out} {err}");

    let output = dir.path().join("out.mov");
    let (code, stdout, err) = cli(
        &[
            "render",
            ps,
            "-o",
            output.to_str().expect("utf8"),
            "--codec",
            "dnxhd",
        ],
        None,
    );
    assert_eq!(code, 0, "{stdout} {err}");
    let r = j(&stdout);
    assert_eq!(r["encoder"], json!("dnxhd"), "{r}");
    assert_eq!(r["frames"], json!(10));
    assert_eq!(r["ffmpeg"]["override"], json!(false));

    let mut d = VideoDecoder::open(&output).expect("decode dnxhd output");
    assert_eq!((d.info().width, d.info().height), (1920, 1080));
    let f = d.frame_at(arc_core::Time::ZERO).expect("dnxhd frame");
    assert_eq!((f.width, f.height), (1920, 1080));
}

#[test]
fn render_range_and_h264() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = build(dir.path());
    let ps = p.to_str().expect("utf8");
    let out = dir.path().join("range.mkv");
    let (code, stdout, err) = cli(
        &[
            "render",
            ps,
            "-o",
            out.to_str().expect("utf8"),
            "--codec",
            "ffv1",
            "--from",
            "00:00:00:25",
            "--to",
            "00:00:01:05",
        ],
        None,
    );
    assert_eq!(code, 0, "{stdout} {err}");
    assert_eq!(j(&stdout)["frames"], json!(10));
    let mut d = VideoDecoder::open(&out).expect("open");
    let f = d.frame_at(arc_core::Time::ZERO).expect("first");
    assert_eq!(
        read_counter(f.rgba, f.width, f.height),
        Some(35),
        "range starts at timeline frame 25 = source 35"
    );

    // lossy delivery codec: encoder chosen by capability probe, never GPL x264. A machine without a
    // hardware encoder (a GPU-less CI runner) has no H.264 path in the shipped build.
    if !has_encoder(VideoCodec::H264) {
        return;
    }
    let mp4 = dir.path().join("out.mp4");
    let (code, stdout, err) = cli(
        &[
            "render",
            ps,
            "-o",
            mp4.to_str().expect("utf8"),
            "--codec",
            "h264",
        ],
        None,
    );
    assert_eq!(code, 0, "{stdout} {err}");
    let r = j(&stdout);
    let enc = r["encoder"].as_str().unwrap_or_default();
    assert!(
        [
            "h264_nvenc",
            "h264_qsv",
            "h264_amf",
            "h264_mf",
            "libopenh264"
        ]
        .contains(&enc),
        "{enc}"
    );
    let mut d = VideoDecoder::open(&mp4).expect("open mp4");
    let f = d
        .frame_at(FrameRate::Fps29_97.time_of_frame(40))
        .expect("frame");
    assert_eq!(
        read_counter(f.rgba, f.width, f.height),
        Some(110),
        "frame-accurate through a lossy codec"
    );
}
/// Goal A: HEVC export. The shipped FFmpeg 9.1 package has no software HEVC encoder (libx265 is
/// GPL and excluded by decision D1), so the `hevc` codec selects a hardware encoder and fails with
/// a typed error when none is usable. The output is validated with `ffprobe`: codec, resolution,
/// frame rate and pixel format.
#[test]
fn hevc_exports_are_decodable_and_match_the_spec() {
    if !has_encoder(VideoCodec::Hevc) {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let p = build(dir.path());
    let ps = p.to_str().expect("utf8");
    let out = dir.path().join("out_hevc.mp4");
    let (code, stdout, err) = cli(
        &[
            "render",
            ps,
            "-o",
            out.to_str().expect("utf8"),
            "--codec",
            "hevc",
            "--from",
            "00:00:00:00",
            "--to",
            "00:00:00:10",
        ],
        None,
    );
    assert_eq!(code, 0, "{stdout} {err}");
    let r = j(&stdout);
    let enc = r["encoder"].as_str().unwrap_or_default();
    assert!(
        ["hevc_nvenc", "hevc_qsv", "hevc_amf"].contains(&enc),
        "hardware HEVC encoder expected, got {enc:?} ({r})"
    );
    assert_eq!(r["codec"], json!("hevc"));
    assert_eq!(r["frames"], json!(10));
    assert_eq!(r["ffmpeg"]["override"], json!(false));

    // ffprobe end-to-end: codec, resolution, frame rate, pixel format.
    let probe = Command::new(arc_media::tools::ffprobe_checked().expect("ffprobe"))
        .args([
            "-v",
            "error",
            "-of",
            "json",
            "-show_entries",
            "stream=codec_name,width,height,pix_fmt,r_frame_rate",
        ])
        .arg(format!("file:{}", out.display()))
        .output()
        .expect("run ffprobe");
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let streams: Value = serde_json::from_slice(&probe.stdout).expect("ffprobe json");
    let s = &streams["streams"][0];
    assert_eq!(s["codec_name"], json!("hevc"), "{s}");
    assert_eq!(s["width"], json!(320));
    assert_eq!(s["height"], json!(180));
    assert_eq!(s["pix_fmt"], json!("yuv420p"), "{s}");
    let rate: f64 = {
        let text = s["r_frame_rate"].as_str().expect("r_frame_rate");
        let (num, den) = text.split_once('/').unwrap_or((text, "1"));
        num.parse::<f64>().unwrap_or(0.0) / den.parse::<f64>().unwrap_or(1.0)
    };
    assert!((rate - 29.97).abs() < 0.02, "frame rate {rate} ({s})");

    // The same decoder reads it back, so the file is a normal Arcline source too.
    let mut d = VideoDecoder::open(&out).expect("decode hevc output");
    assert_eq!((d.info().width, d.info().height), (320, 180));
    assert_eq!(d.info().codec, "hevc");
    let f = d.frame_at(arc_core::Time::ZERO).expect("hevc frame");
    assert_eq!((f.width, f.height), (320, 180));
}

/// Goal B: one unreadable asset must not stop the export. The report marks it unavailable, the
/// other clips still render frame-exactly, and the project stays usable afterwards.
#[test]
fn mixed_media_export_survives_an_unreadable_asset() {
    let dir = tempfile::tempdir().expect("tmp");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("mkdir");
    let good = media.join("good.mkv");
    let broken = media.join("broken.mkv");
    write_counter_video(&good, 60, FrameRate::Fps30.rate(), 320, 180, false).expect("fixture");
    std::fs::copy(&good, &broken).expect("copy");

    let p = dir.path().join("mixed.arcline");
    let ps = p.to_str().expect("utf8");
    let (code, out, err) = cli(
        &["new", ps, "--fps", "30", "--res", "320x180", "--name", "Mixed"],
        None,
    );
    assert_eq!(code, 0, "{out} {err}");
    let (code, out, err) = cli(
        &[
            "import",
            ps,
            good.to_str().expect("utf8"),
            broken.to_str().expect("utf8"),
        ],
        None,
    );
    assert_eq!(code, 0, "{out} {err}");
    let imp = j(&out);
    let good_id = imp["imported"][0]["id"].as_str().expect("good id").to_string();
    let broken_id = imp["imported"][1]["id"]
        .as_str()
        .expect("broken id")
        .to_string();

    // The media is replaced after import: the project references a file that no longer decodes.
    std::fs::write(&broken, b"corrupted after import, no longer media\n").expect("corrupt");

    let batch = json!([
        {"op":"track.add","id":"tr_v1","kind":"video"},
        {"op":"track.add","id":"tr_v2","kind":"video"},
        {"op":"timeline.add_clip","id":"cl_good","track":"tr_v1","asset":good_id,"start":"00:00:00:00","duration":"00:00:01:00"},
        {"op":"timeline.add_clip","id":"cl_broken","track":"tr_v2","asset":broken_id,"start":"00:00:01:00","duration":"00:00:01:00"}
    ]);
    let (code, out, err) = cli(&["apply", ps], Some(&batch.to_string()));
    assert_eq!(code, 0, "{out} {err}");

    let output = dir.path().join("out.mkv");
    let (code, stdout, err) = cli(
        &[
            "render",
            ps,
            "-o",
            output.to_str().expect("utf8"),
            "--codec",
            "ffv1",
        ],
        None,
    );
    assert_eq!(code, 0, "export must survive: {stdout} {err}");
    let report = j(&stdout);
    assert_eq!(report["frames"], json!(60), "{report}");
    let warnings = report["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str())
        .collect::<Vec<_>>();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("unavailable") && w.contains(&broken_id)),
        "the broken asset must be reported as unavailable: {warnings:?}"
    );

    // The good clip is still frame-exact; the unavailable one renders the background.
    let rate = FrameRate::Fps30;
    let mut d = VideoDecoder::open(&output).expect("open output");
    for k in 0..30i64 {
        let f = d.frame_at(rate.time_of_frame(k)).expect("good frame");
        assert_eq!(
            read_counter(f.rgba, f.width, f.height),
            Some(k as u32),
            "frame {k} of the intact clip"
        );
    }
    let f = d.frame_at(rate.time_of_frame(45)).expect("broken-frame slot");
    assert!(
        f.rgba.chunks_exact(4).all(|px| px[0] == 0 && px[1] == 0 && px[2] == 0),
        "an unavailable source renders black, not a stale frame"
    );

    // The project and the session are untouched by the failed asset.
    let (code, out, err) = cli(&["describe", ps], None);
    assert_eq!(code, 0, "{out} {err}");
    assert_eq!(j(&out)["project"]["name"], json!("Mixed"));
}