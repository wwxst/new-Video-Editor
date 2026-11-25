//! End-to-end pipeline acceptance (docs/10 §3 Phase B exit): import → edit → render → decode,
//! frame-exact.

use arc_core::time::FrameRate;
use arc_media::fixtures::{read_counter, write_counter_video};
use arc_media::{AudioDecoder, VideoDecoder};
use serde_json::{Value, json};
use std::process::Command;

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

    // lossy delivery codec: encoder chosen by capability probe, never GPL x264
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
