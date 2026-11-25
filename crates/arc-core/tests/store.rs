//! Container, sidecar, lock, and recovery through the runtime (docs/08 §4).

mod common;

use arc_core::FrameRate;
use arc_core::engine::{ApplyRequest, Control, Source};
use arc_core::error::ErrorKind;
use arc_core::journal::Durability;
use arc_core::runtime::{EngineHost, Event};
use arc_core::store::{self, ProjectPaths, StoreError};
use common::*;
use serde_json::json;
use std::sync::Arc;

fn rename(name: &str) -> ApplyRequest {
    req(cmds(json!([{"op":"project.rename","name":name}])))
}

#[test]
fn container_round_trip_and_checksum() {
    let p = project(FrameRate::Fps29_97);
    let bytes = store::encode_container(&p, 7).expect("enc");
    let (back, m) = store::decode_container(&bytes).expect("dec");
    assert_eq!(back, p);
    assert_eq!(m.revision, 7);
    assert_eq!(m.schema_version, 7);
}

#[test]
fn future_schema_is_refused() {
    let p = project(FrameRate::Fps30);
    let mut v = serde_json::to_value(&p).expect("v");
    v["schema_version"] = json!(99);
    let err = store::parse_document(&serde_json::to_vec(&v).expect("b")).expect_err("future");
    assert!(matches!(err, StoreError::FutureSchema { found: 99, .. }));
}

#[test]
fn create_edit_save_reopen() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("demo.arcline");
    let host = EngineHost::create(
        &path,
        project(FrameRate::Fps30),
        catalog(),
        Durability::Default,
    )
    .expect("create");
    host.apply(rename("First")).expect("edit");
    let saved = host.save().expect("save");
    assert_eq!(saved, 1);
    host.shutdown();
    assert!(
        !ProjectPaths::new(&path).sidecar.exists(),
        "clean close removes the sidecar"
    );
    let host = EngineHost::open(&path, catalog(), Durability::Default).expect("open");
    assert_eq!(host.published().project.meta.name, "First");
    assert_eq!(host.published().revision, 1);
    assert!(!host.recovery().recovered);
}

#[test]
fn second_open_is_locked() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("lock.arcline");
    let _host = EngineHost::create(
        &path,
        project(FrameRate::Fps30),
        catalog(),
        Durability::Default,
    )
    .expect("create");
    let err = EngineHost::open(&path, catalog(), Durability::Default).expect_err("locked");
    assert!(
        matches!(
            err,
            arc_core::runtime::HostError::Store(StoreError::Locked(_))
        ),
        "{err:?}"
    );
}

/// Child half of the forced-kill test: edits, then dies without any cleanup.
#[test]
#[ignore = "spawned by forced_kill_recovery"]
fn forced_kill_child() {
    let Ok(dir) = std::env::var("ARC_KILL_DIR") else {
        return;
    };
    let n: usize = std::env::var("ARC_KILL_EDITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let path = std::path::Path::new(&dir).join("kill.arcline");
    let host = EngineHost::create(
        &path,
        project(FrameRate::Fps30),
        catalog(),
        Durability::Default,
    )
    .expect("create");
    for i in 0..n {
        host.apply(rename(&format!("v{i}"))).expect("edit");
    }
    // every acknowledged batch is in the OS page cache; now die like a crash / TerminateProcess
    std::process::abort();
}

/// Forced kill (docs/08 §4.7): no acknowledged batch may be lost to a process crash.
#[test]
fn forced_kill_recovery() {
    for edits in [1usize, 29, 30, 31, 75] {
        let dir = tempfile::tempdir().expect("tmp");
        let status = std::process::Command::new(std::env::current_exe().expect("exe"))
            .args([
                "--exact",
                "forced_kill_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("ARC_KILL_DIR", dir.path())
            .env("ARC_KILL_EDITS", edits.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("spawn child");
        assert!(!status.success(), "child must die abnormally");
        let path = dir.path().join("kill.arcline");
        let host =
            EngineHost::open(&path, catalog(), Durability::Default).expect("reopen after kill");
        let p = host.published();
        assert_eq!(p.revision, edits as u64, "edits={edits}");
        assert_eq!(p.project.meta.name, format!("v{}", edits - 1));
    }
}

#[test]
fn torn_journal_tail_is_discarded_and_kept_for_support() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("torn.arcline");
    let host = EngineHost::create(
        &path,
        project(FrameRate::Fps30),
        catalog(),
        Durability::Default,
    )
    .expect("create");
    host.apply(rename("ok-1")).expect("1");
    host.apply(rename("ok-2")).expect("2");
    host.set_control(Control::Shared);
    let paths = ProjectPaths::new(&path);
    drop(host); // orderly drop flushes; the sidecar stays because nothing was saved
    // append garbage to simulate a torn record
    let j = paths.sidecar.join("journal-0.bin");
    let mut bytes = std::fs::read(&j).expect("journal");
    bytes.extend_from_slice(&[0x41, 0x52, 0x43, 0x4A, 200, 0, 0, 0, 1, 2]);
    std::fs::write(&j, &bytes).expect("write");
    let host = EngineHost::open(&path, catalog(), Durability::Default).expect("open");
    assert_eq!(host.published().project.meta.name, "ok-2");
    assert!(host.recovery().discarded_records);
    assert!(paths.sidecar.join("journal-0.bin.rejected").exists());
}

#[test]
fn acknowledgement_and_notifications() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("events.arcline");
    let host = EngineHost::create(
        &path,
        project(FrameRate::Fps30),
        catalog(),
        Durability::Durable,
    )
    .expect("create");
    let events = host.subscribe();
    let r = host
        .apply(ApplyRequest {
            source: Some(Source::Client("a1".into())),
            ..rename("X")
        })
        .expect("apply");
    assert_eq!(r.revision, 1);
    let p = host.published();
    assert_eq!(
        p.confirmed_revision, 1,
        "reply arrives only after the journal write"
    );
    match events
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("event")
    {
        Event::Changed {
            revision, source, ..
        } => {
            assert_eq!(revision, 1);
            assert_eq!(source, Source::Client("a1".into()));
        }
        other => panic!("unexpected {other:?}"),
    }
    host.set_control(Control::Human);
    let err = host
        .apply(ApplyRequest {
            source: Some(Source::Client("a1".into())),
            ..rename("Y")
        })
        .expect_err("not in control");
    assert_eq!(err.kind, ErrorKind::NotInControl);
    let undo = host.undo(Source::Human).expect("undo");
    assert_eq!(undo.revision, 2);
    assert_eq!(host.published().project.meta.name, "Test");
    let _ = Arc::strong_count(&host.published());
}
