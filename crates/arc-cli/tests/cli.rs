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
