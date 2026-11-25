//! End-to-end CLI tests: Phase A exit criteria (docs/10 §2).

use serde_json::{Value, json};
use std::process::Command;

fn cli(args: &[&str], stdin: Option<&str>) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_arcline-cli"));
    c.args(args);
    let out = match stdin {
        Some(input) => {
            use std::io::Write;
            let mut child = c
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
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
    )
}

fn j(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("not json ({e}): {s}"))
}

#[test]
fn new_apply_describe_timeline_undo() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path().join("demo.arcline");
    let ps = p.to_str().expect("utf8");

    let (code, out) = cli(&["new", ps, "--name", "Demo", "--fps", "29.97"], None);
    assert_eq!(code, 0, "{out}");
    assert_eq!(j(&out)["sequence"]["frame_rate"], json!("30000/1001"));

    let batch = json!([
        {"op":"track.add","id":"tr_v1","kind":"video","name":"A-Roll"},
        {"op":"asset.import","id":"as_a","path":"media/a.mov","kind":"video",
         "probe":{"duration":{"num":60,"den":1},"video":{"width":1920,"height":1080,"frame_rate":{"num":30000,"den":1001}}}},
        {"op":"timeline.add_clip","id":"cl_1","track":"tr_v1","asset":"as_a","start":"00:00:00:00","duration":"00:00:10:00","source_in":"00:00:01:00"},
        {"op":"effect.add","clip":"cl_1","effect":"core.color.correct","params":{"exposure":{"static":0.5}}},
        {"op":"marker.add","at":"00:00:05;00","label":"df input accepted"}
    ]);
    let (code, out) = cli(&["apply", ps], Some(&batch.to_string()));
    assert_eq!(code, 0, "{out}");
    let r = j(&out);
    assert_eq!(r["revision"], json!(1));
    assert_eq!(r["applied"], json!(5));

    let (code, out) = cli(&["describe", ps], None);
    assert_eq!(code, 0);
    let d = j(&out);
    assert_eq!(d["revision"], json!(1));
    assert_eq!(d["clips"][0]["end"], json!("00:00:10:00"));
    assert_eq!(d["clips"][0]["effects"], json!(["core.color.correct"]));
    assert_eq!(
        d["markers"][0]["at"],
        json!("00:00:05:00"),
        "always emits NDF"
    );

    let (code, text) = cli(&["timeline", ps], None);
    assert_eq!(code, 0);
    assert!(
        text.starts_with("# arcline timeline v1\nSEQ \"Main\" 1920x1080 fps=30000/1001"),
        "{text}"
    );
    assert!(text.contains("TRK V1 name=\"A-Roll\" op=1"), "{text}");
    assert!(
        text.contains(
            "  [cl_1] \"a.mov\" in=00:00:01:00 len=00:00:10:00 @00:00:00:00 fx=core.color.correct("
        ),
        "{text}"
    );

    // typed error, nothing applied
    let bad = json!([{"op":"timeline.add_clip","track":"tr_v1","asset":"as_a","start":"00:00:05:00","duration":"00:00:01:00"}]);
    let (code, out) = cli(&["apply", ps], Some(&bad.to_string()));
    assert_eq!(code, 2);
    assert_eq!(j(&out)["error"]["kind"], json!("overlap"));

    // the rejected batch changed nothing on disk
    let (_, out) = cli(&["describe", ps], None);
    let d = j(&out);
    assert_eq!(d["revision"], json!(1));
    assert_eq!(d["clips"].as_array().map(Vec::len), Some(1));
}

/// Audit F-04: `new` must not replace an existing project by accident, and the lock failure must be
/// reported as `project_locked` (not `io_error`).
#[test]
fn new_refuses_existing_project_and_overwrite_opts_in() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path().join("dup.arcline");
    let ps = p.to_str().expect("utf8");
    let (code, out) = cli(&["new", ps, "--name", "PROJECT A"], None);
    assert_eq!(code, 0, "{out}");

    let (code, out) = cli(&["new", ps, "--name", "PROJECT B"], None);
    assert_eq!(code, 2, "{out}");
    assert_eq!(j(&out)["error"]["kind"], json!("project_exists"));
    let (code, out) = cli(&["describe", ps], None);
    assert_eq!(code, 0, "{out}");
    assert_eq!(j(&out)["project"]["name"], json!("PROJECT A"));

    let (code, out) = cli(&["new", ps, "--name", "PROJECT B", "--overwrite"], None);
    assert_eq!(code, 0, "{out}");
    assert_eq!(j(&out)["project"]["name"], json!("PROJECT B"));
}

/// Audit F-09: `apply` must resolve every client `asset.import` itself: reject URL/protocol paths
/// before any I/O, run the import check, and ignore client-supplied kind/probe/status.
#[test]
fn apply_checks_client_imports_and_rejects_urls() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path().join("imp.arcline");
    let ps = p.to_str().expect("utf8");
    let (code, out) = cli(&["new", ps], None);
    assert_eq!(code, 0, "{out}");

    let url = json!([{"op":"asset.import","path":"http://127.0.0.1:9/x.mp4","kind":"video","status":"ok"}]);
    let (code, out) = cli(&["apply", ps], Some(&url.to_string()));
    assert_eq!(code, 2, "{out}");
    assert_eq!(j(&out)["error"]["kind"], json!("invalid_value"));
    let (_, out) = cli(&["describe", ps], None);
    assert_eq!(j(&out)["assets"], json!([]), "nothing was imported");

    // Windows device namespace / verbatim paths are the handle-style access F-09 closes.
    for bad in [
        r"\\.\pipe\arcline",
        r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1\x.mp4",
        "//./pipe/arcline",
        "//?/C:/x.mp4",
    ] {
        let batch = json!([{"op":"asset.import","path":bad,"kind":"video","status":"ok"}]);
        let (code, out) = cli(&["apply", ps], Some(&batch.to_string()));
        assert_eq!(code, 2, "{bad}: {out}");
        assert_eq!(j(&out)["error"]["kind"], json!("invalid_value"), "{bad}");
    }
    let (_, out) = cli(&["describe", ps], None);
    assert_eq!(j(&out)["assets"], json!([]), "no device path was imported");

    // A lying client: wrong kind/status/probe for a real local file.
    let media = dir.path().join("media").join("counter.mkv");
    std::fs::create_dir_all(media.parent().expect("parent")).expect("mkdir");
    arc_media::fixtures::write_counter_video(
        &media,
        90,
        arc_core::FrameRate::Fps30.rate(),
        160,
        90,
        false,
    )
    .expect("fixture");
    let batch = json!([{
        "op":"asset.import",
        "path": media.to_str().expect("utf8"),
        "kind":"audio",
        "status":"offline",
        "probe":{"duration":{"num":1,"den":1}}
    }]);
    let (code, out) = cli(&["apply", ps], Some(&batch.to_string()));
    assert_eq!(code, 0, "{out}");
    let (_, out) = cli(&["describe", ps], None);
    let a = &j(&out)["assets"][0];
    assert_eq!(a["status"], json!("ok"), "client status ignored");
    assert_eq!(a["kind"], json!("video"), "client kind ignored");
    assert_eq!(a["path"], json!("media/counter.mkv"));
    assert_eq!(a["duration"], json!("00:00:03:00"), "client probe ignored");

    // The tool subcommands reject scheme paths too (no probe/open attempt).
    let (code, out) = cli(&["probe", "http://127.0.0.1:9/x.mp4"], None);
    assert_eq!(code, 2, "{out}");
    assert_eq!(j(&out)["error"]["kind"], json!("invalid_value"));
    let (code, out) = cli(&["import", ps, "handle:1"], None);
    assert_eq!(code, 2, "{out}");
    assert_eq!(j(&out)["error"]["kind"], json!("invalid_value"));
}

#[test]
fn usage_and_frame_rate_errors() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path().join("x.arcline");
    let (code, out) = cli(&["new", p.to_str().expect("utf8"), "--fps", "29.976"], None);
    assert_eq!(code, 2);
    assert_eq!(j(&out)["error"]["kind"], json!("inexact_frame_rate"));
    let (code, _) = cli(&["frobnicate"], None);
    assert_eq!(code, 64);
}
/// P3-3: `render --help` and the codec parser share one list, so `hevc` must appear in both.
#[test]
fn render_help_and_parse_offer_the_same_codecs() {
    let (code, out) = cli(&["render", "--help"], None);
    assert_eq!(code, 0, "{out}");
    for name in ["ffv1", "h264", "hevc", "av1", "prores", "dnxhd"] {
        assert!(out.contains(name), "{name} missing from --help: {out}");
    }
    let (code, out) = cli(&["render", "unused.arcline", "-o", "out.mkv", "--codec", "bogus"], None);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("codec must be one of"), "{out}");
    assert!(out.contains("hevc"), "{out}");
}