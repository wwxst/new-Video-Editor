//! `std::env::set_var` is unsafe in edition 2024; this test is single-threaded and the variables
//! only affect the child tools it spawns.
#![allow(unsafe_code)]
//! The `Quarantined` import outcome (docs/04 §1.2 step 2): media whose probe/decode *crashes* or
//! *hangs* the child process must be quarantined, never decoded in-process, and the calling process
//! must keep working.
//!
//! The tools are located through `ARCLINE_FFMPEG_DIR/bin`, which is a runtime lookup (`tools.rs`),
//! so the test can point it at a generated stub bundle. Each stub forwards to the real pinned tool
//! unless the argument matches `STUB_MATCH`, which is how one process exercises a crashing, a
//! hanging and a garbage-printing tool while still checking real media.

use arc_media::probe::{ImportOutcome, PROBE_TIMEOUT, import_check};
use arc_media::fixtures::write_counter_video;
use arc_core::time::FrameRate;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Source of the stub: proxies to the real tool by default, misbehaves when the argument contains
/// `STUB_MATCH`.
const STUB: &str = r#"
use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    let name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_default();
    let tool = if name.contains("ffprobe") { "ffprobe" } else { "ffmpeg" };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = std::env::var("STUB_MODE").unwrap_or_else(|_| "pass".into());
    let matcher = std::env::var("STUB_MATCH").unwrap_or_default();
    let matched = !matcher.is_empty() && args.iter().any(|a| a.contains(&matcher));
    if matched {
        match mode.as_str() {
            // Windows NTSTATUS 0xC0000005: tools::run classifies this as Crashed.
            "crash" => std::process::exit(-1073741819),
            "hang" => std::thread::sleep(Duration::from_secs(120)),
            "garbage" => {
                println!("this is not ffprobe JSON");
                std::process::exit(0);
            }
            _ => {}
        }
    }
    let dir = std::env::var("STUB_REAL_DIR").unwrap_or_default();
    let real = std::path::Path::new(&dir).join(format!("{tool}.exe"));
    match Command::new(&real)
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(s) => std::process::exit(s.code().unwrap_or(-1)),
        Err(e) => {
            eprintln!("stub: cannot run {}: {e}", real.display());
            std::process::exit(127);
        }
    }
}
"#;

fn real_bin_dir() -> PathBuf {
    let dir = std::env::var("ARCLINE_FFMPEG_DIR")
        .expect("set ARCLINE_FFMPEG_DIR (scripts/dev-env.sh) before running this test");
    PathBuf::from(dir).join("bin")
}

/// Compile the stub twice, as `ffmpeg.exe` and `ffprobe.exe`, into a generated bundle.
fn stub_bundle(dir: &Path) -> PathBuf {
    let src = dir.join("stub.rs");
    std::fs::write(&src, STUB).expect("write stub");
    let exe = dir.join("stub.exe");
    let out = Command::new("rustc")
        .args(["--edition", "2021", "-O", "-o"])
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("run rustc for the stub");
    assert!(
        out.status.success(),
        "rustc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bin = dir.join("bundle").join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir bundle");
    for name in ["ffmpeg.exe", "ffprobe.exe"] {
        std::fs::copy(&exe, bin.join(name)).expect("copy stub");
    }
    dir.join("bundle")
}

fn set_mode(mode: &str, matcher: &str) {
    // SAFETY: the test is single-threaded at this point and these variables are read by the child
    // tools this process spawns.
    unsafe {
        std::env::set_var("STUB_MODE", mode);
        std::env::set_var("STUB_MATCH", matcher);
    }
}

fn assert_quarantined(name: &str, outcome: ImportOutcome, expect: &str) {
    match outcome {
        ImportOutcome::Quarantined { reason, .. } => assert!(
            reason.contains(expect),
            "{name}: reason {reason:?} does not contain {expect:?}"
        ),
        other => panic!("{name}: expected Quarantined, got {other:?}"),
    }
}

/// One process exercises the whole contract: real media, a crashing tool, a hanging tool, a tool
/// that prints garbage, and real media again. A panic or an abort fails the test; the hang case
/// proves the timeout is enforced (the import check returns within the cap).
#[test]
fn quarantine_covers_crash_hang_and_garbage_and_the_process_survives() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skipping: rustc is not on PATH, cannot build the tool stub");
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let real = real_bin_dir();
    let bundle = stub_bundle(dir.path());

    // One small valid source; copies with a marker name trigger the stub.
    let good = dir.path().join("good.mkv");
    write_counter_video(&good, 30, FrameRate::Fps30.rate(), 160, 90, false).expect("fixture");
    let crashing = dir.path().join("quarantine_crash.mkv");
    let hanging = dir.path().join("quarantine_hang.mkv");
    let garbage = dir.path().join("quarantine_garbage.mkv");
    for copy in [&crashing, &hanging, &garbage] {
        std::fs::copy(&good, copy).expect("copy fixture");
    }

    // SAFETY: single-threaded test; the tools are resolved from this directory on every call.
    unsafe {
        std::env::set_var("STUB_REAL_DIR", &real);
        std::env::set_var("ARCLINE_FFMPEG_DIR", &bundle);
    }

    // 1. real media through the proxied real tools
    set_mode("pass", "");
    match import_check(&good) {
        ImportOutcome::Ok { .. } => {}
        other => panic!("good media must import through the stub proxy: {other:?}"),
    }

    // 2. a crashing tool is quarantined, with a bounded, typed reason
    set_mode("crash", "quarantine_crash.mkv");
    let started = Instant::now();
    let outcome = import_check(&crashing);
    assert!(started.elapsed() < Duration::from_secs(10), "crash path was not immediate");
    assert_quarantined("crash", outcome, "Crashed");

    // 3. a hanging tool is quarantined at the probe timeout, not left running
    set_mode("hang", "quarantine_hang.mkv");
    let started = Instant::now();
    let outcome = import_check(&hanging);
    let elapsed = started.elapsed();
    assert_quarantined("hang", outcome, "TimedOut");
    assert!(
        elapsed >= PROBE_TIMEOUT && elapsed < PROBE_TIMEOUT + Duration::from_secs(15),
        "hang must hit the {PROBE_TIMEOUT:?} cap and return, took {elapsed:?}"
    );

    // 4. a tool that prints garbage is Offline, not Quarantined (the distinction matters: offline
    //    is "not media", quarantined is "must never be decoded in-process")
    set_mode("garbage", "quarantine_garbage.mkv");
    match import_check(&garbage) {
        ImportOutcome::Offline { reason } => assert!(!reason.is_empty()),
        other => panic!("garbage output must be Offline, got {other:?}"),
    }

    // 5. mixed use: real media still works after the failures, in the same process
    set_mode("pass", "");
    match import_check(&good) {
        ImportOutcome::Ok { .. } => {}
        other => panic!("good media must still import after the quarantined attempts: {other:?}"),
    }

    // The timeout is the documented cap, not an arbitrary value.
    assert_eq!(PROBE_TIMEOUT, Duration::from_secs(20));
}
