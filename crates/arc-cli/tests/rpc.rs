//! JSON-RPC session semantics (docs/06 §4): revisions, stale revision, dry run, idempotency,
//! undo/redo within a session, text timeline, effect discovery, errors, render jobs.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

struct Rpc {
    child: std::process::Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Rpc {
    fn start() -> Rpc {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arcline-cli"))
            .args(["rpc", "--client", "test_agent"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Rpc {
            child,
            stdin,
            stdout,
            next: 1,
        }
    }

    fn raw(&mut self, line: &str) -> Value {
        writeln!(self.stdin, "{line}").expect("write");
        self.stdin.flush().expect("flush");
        let mut s = String::new();
        self.stdout.read_line(&mut s).expect("read");
        serde_json::from_str(&s).unwrap_or_else(|e| panic!("bad response ({e}): {s}"))
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let r = self.raw(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        );
        assert_eq!(r["id"], json!(id));
        r
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params);
        assert!(r.get("error").is_none(), "{method}: {r}");
        r["result"].clone()
    }

    fn kind(&mut self, method: &str, params: Value) -> String {
        let r = self.call(method, params);
        r["error"]["data"]["kind"]
            .as_str()
            .unwrap_or_else(|| panic!("{method} should fail: {r}"))
            .to_string()
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn session_semantics() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("rpc.arcline");
    let mut s = Rpc::start();

    assert_eq!(s.kind("project.describe", json!({})), "no_project");
    let open = s.ok("project.open", json!({"path": path, "create": {"name": "RPC", "frame_rate": "29.97", "resolution": [640, 360]}}));
    assert_eq!(open["protocol"], json!("1.0"));
    assert_eq!(open["revision"], json!(0));
    assert_eq!(
        open["summary"]["sequence"]["frame_rate"],
        json!("30000/1001")
    );

    let fx = s.ok("effect.list", json!({}));
    assert_eq!(fx["effects"].as_array().map(Vec::len), Some(6));

    let r = s.ok("project.apply", json!({"base_revision": 0, "commands": [
        {"op":"track.add","id":"tr_v1","kind":"video","name":"Main"},
        {"op":"title.add","id":"cl_t","track":"tr_v1","text":"Hello","start":"00:00:00:00","duration":"00:00:02:00"}
    ]}));
    assert_eq!(r["revision"], json!(1));

    // stale revision carries the current revision
    let e = s.call(
        "project.apply",
        json!({"base_revision": 0, "commands": [{"op":"project.rename","name":"x"}]}),
    );
    assert_eq!(e["error"]["data"]["kind"], json!("stale_revision"));
    assert_eq!(e["error"]["data"]["details"]["current_revision"], json!(1));

    // dry run does not commit; idempotent retry does not double-apply
    let d = s.ok(
        "project.apply",
        json!({"commands": [{"op":"project.rename","name":"Dry"}], "dry_run": true}),
    );
    assert_eq!(d["dry_run"], json!(true));
    let a = s.ok("project.apply", json!({"commands": [{"op":"marker.add","at":"00:00:01:00","label":"m"}], "idempotency_key": "k1"}));
    let b = s.ok("project.apply", json!({"commands": [{"op":"marker.add","at":"00:00:01:00","label":"m"}], "idempotency_key": "k1"}));
    assert_eq!(a, b);
    assert_eq!(s.ok("project.describe", json!({}))["revision"], json!(2));

    // typed domain error
    assert_eq!(
        s.kind("project.apply", json!({"commands": [{"op":"title.add","track":"tr_v1","text":"B","start":"00:00:01:00","duration":"00:00:01:00"}]})),
        "overlap"
    );

    // undo/redo inside the session
    assert_eq!(s.ok("history.undo", json!({}))["revision"], json!(3));
    assert_eq!(s.ok("project.describe", json!({}))["markers"], json!([]));
    assert_eq!(s.ok("history.redo", json!({}))["revision"], json!(4));
    assert_eq!(
        s.kind("history.undo", json!({"base_revision": 1})),
        "stale_revision"
    );

    let t = s.ok("inspect.timeline", json!({}));
    assert!(
        t["text"]
            .as_str()
            .unwrap_or_default()
            .contains("<< TITLE \"Hello\" >>"),
        "{t}"
    );

    assert_eq!(s.ok("project.save", json!({}))["revision"], json!(4));
    assert_eq!(s.kind("frobnicate", json!({})), "unsupported_method");
    let pe = s.raw("{not json");
    assert_eq!(pe["error"]["code"], json!(-32700));
    let ip = s.call("project.apply", json!({"commands": "nope"}));
    assert_eq!(ip["error"]["code"], json!(-32602));
}

/// Audit F-04: creation takes the lock before writing and refuses an existing container unless
/// `create.overwrite` is true; both map to stable error kinds.
#[test]
fn create_refuses_locked_and_existing_projects() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("lock.arcline");
    let create = |name: &str| {
        json!({"path": path, "create": {"name": name, "frame_rate": "30", "resolution": [160, 90]}})
    };
    let mut a = Rpc::start();
    a.ok("project.open", create("PROJECT A"));
    a.ok("project.save", json!({}));

    let mut b = Rpc::start();
    assert_eq!(b.kind("project.open", create("PROJECT B")), "project_locked");
    let mut overwrite = create("PROJECT B");
    overwrite["create"]["overwrite"] = json!(true);
    assert_eq!(
        b.kind("project.open", overwrite),
        "project_locked",
        "overwrite must never break another session's lock"
    );

    drop(a); // kill the session; the lock is released with the process
    let mut c = Rpc::start();
    assert_eq!(
        c.kind("project.open", create("PROJECT B")),
        "project_exists"
    );
    let mut overwrite = create("PROJECT B");
    overwrite["create"]["overwrite"] = json!(true);
    let r = c.ok("project.open", overwrite);
    assert_eq!(r["summary"]["project"]["name"], json!("PROJECT B"));
    assert_eq!(r["revision"], json!(0));
    c.ok("project.save", json!({}));
}

#[test]
fn render_job_runs_out_of_process() {
    let dir = tempfile::tempdir().expect("tmp");
    let media = dir.path().join("counter.mkv");
    arc_media::fixtures::write_counter_video(
        &media,
        30,
        arc_core::FrameRate::Fps30.rate(),
        160,
        90,
        false,
    )
    .expect("fixture");
    let path = dir.path().join("job.arcline");
    let mut s = Rpc::start();
    s.ok(
        "project.open",
        json!({"path": path, "create": {"frame_rate": "30", "resolution": [160, 90]}}),
    );
    s.ok("project.apply", json!({"commands": [
        {"op":"track.add","id":"tr_v1","kind":"video"},
        {"op":"asset.import","id":"as_c","path":media,"kind":"video","probe":{"duration":{"num":1,"den":1}}},
        {"op":"timeline.add_clip","track":"tr_v1","asset":"as_c","start":"00:00:00:00","duration":"00:00:00:20","source_in":"00:00:00:05"}
    ]}));
    let out = dir.path().join("job.mkv");
    let job = s.ok(
        "job.submit",
        json!({"kind": "render", "spec": {"output": out, "codec": "ffv1"}}),
    );
    let id = job["job_id"].clone();
    let mut state = Value::Null;
    for _ in 0..600 {
        state = s.ok("job.status", json!({"job_id": id}));
        if state["state"] != json!("running") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(state["state"], json!("done"), "{state}");
    assert_eq!(state["report"]["frames"], json!(20));
    let mut d = arc_media::VideoDecoder::open(&out).expect("output");
    let f = d.frame_at(arc_core::Time::ZERO).expect("frame");
    assert_eq!(
        arc_media::fixtures::read_counter(f.rgba, f.width, f.height),
        Some(5)
    );
}
