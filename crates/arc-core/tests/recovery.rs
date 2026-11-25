//! Recovery regressions (audit F-01, F-04, F-05; docs/08 §4.6).

mod common;

use arc_core::effects::StaticCatalog;
use arc_core::engine::{ApplyRequest, ApplyResult, Source};
use arc_core::ids::{AssetId, ClipId, EffectId, TrackId};
use arc_core::journal::Durability;
use arc_core::model::{AssetKind, TrackKind};
use arc_core::runtime::EngineHost;
use arc_core::store::ProjectPaths;
use arc_core::{Command, FrameRate, TimeInput};
use common::*;
use std::sync::Arc;

fn empty_catalog() -> Arc<StaticCatalog> {
    Arc::new(StaticCatalog::new(std::iter::empty()))
}

fn track() -> TrackId {
    TrackId::parse("tr_v1").expect("track id")
}
fn asset() -> AssetId {
    AssetId::parse("as_a").expect("asset id")
}
fn clip() -> ClipId {
    ClipId::parse("cl_a").expect("clip id")
}

fn setup() -> Vec<Command> {
    vec![
        Command::TrackAdd {
            id: Some(track()),
            kind: TrackKind::Video,
            name: None,
            index: None,
        },
        Command::AssetImport {
            id: Some(asset()),
            path: "a.mov".into(),
            kind: AssetKind::Video,
            probe: Some(video_probe(60)),
            status: Default::default(),
        },
        Command::AddClip {
            id: Some(clip()),
            track: track(),
            asset: Some(asset()),
            start: TimeInput::Timecode("00:00:00:00".into()),
            duration: TimeInput::Timecode("00:00:01:00".into()),
            source_in: None,
            speed: None,
        },
    ]
}

fn apply(host: &EngineHost, commands: Vec<Command>) -> ApplyResult {
    host.apply(ApplyRequest {
        commands,
        source: Some(Source::Human),
        ..Default::default()
    })
    .expect("apply")
}

/// Audit F-04 (experiment H): `create` took the session lock only *after* writing the container, so
/// `new` on a project that another session had open destroyed that session's saved work. Creation
/// must take the lock first and leave the existing container alone.
#[test]
fn create_over_locked_project_is_refused_without_touching_it() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("h.arcline");
    let a = EngineHost::create(
        &path,
        arc_core::model::Project::new(
            "PROJECT A",
            FrameRate::Fps30,
            arc_core::model::Resolution { w: 640, h: 360 },
        ),
        catalog(),
        Durability::Default,
        false,
    )
    .expect("create A");
    apply(&a, setup());
    assert_eq!(a.save().expect("save A"), 1);

    let err = EngineHost::create(
        &path,
        arc_core::model::Project::new(
            "PROJECT B",
            FrameRate::Fps30,
            arc_core::model::Resolution { w: 640, h: 360 },
        ),
        catalog(),
        Durability::Default,
        false,
    )
    .expect_err("locked");
    assert!(
        matches!(
            err,
            arc_core::runtime::HostError::Store(arc_core::store::StoreError::Locked(_))
        ),
        "{err:?}"
    );

    // The container must still be A's saved revision, not B.
    let (on_disk, m) = arc_core::store::load_container(&path).expect("container");
    assert_eq!(on_disk.meta.name, "PROJECT A");
    assert_eq!(m.revision, 1);

    a.shutdown(); // clean close removes the sidecar
    let reopened = EngineHost::open(&path, catalog(), Durability::Default).expect("reopen");
    assert_eq!(reopened.published().project.meta.name, "PROJECT A");
    assert_eq!(reopened.published().revision, 1);
    reopened.shutdown();
}

/// Audit F-04: creating over an existing closed project is refused unless the caller opts in.
#[test]
fn create_refuses_existing_project_unless_overwrite() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("exists.arcline");
    let a = EngineHost::create(
        &path,
        arc_core::model::Project::new(
            "PROJECT A",
            FrameRate::Fps30,
            arc_core::model::Resolution { w: 640, h: 360 },
        ),
        catalog(),
        Durability::Default,
        false,
    )
    .expect("create A");
    apply(&a, setup());
    a.save().expect("save");
    a.shutdown();

    let b = arc_core::model::Project::new(
        "PROJECT B",
        FrameRate::Fps30,
        arc_core::model::Resolution { w: 640, h: 360 },
    );
    let err = EngineHost::create(
        &path,
        b.clone(),
        catalog(),
        Durability::Default,
        false,
    )
    .expect_err("exists");
    assert!(
        matches!(
            err,
            arc_core::runtime::HostError::Store(arc_core::store::StoreError::Exists(_))
        ),
        "{err:?}"
    );
    assert_eq!(
        arc_core::store::load_container(&path)
            .expect("container")
            .0
            .meta
            .name,
        "PROJECT A"
    );

    let host = EngineHost::create(&path, b, catalog(), Durability::Default, true)
        .expect("overwrite");
    assert_eq!(host.published().project.meta.name, "PROJECT B");
    assert_eq!(host.published().revision, 0);
    host.shutdown();
}

/// Audit F-05 (experiment C2): a sidecar left by an unsaved session of project A must not be
/// recovered as project B when B is created at the same path after the container was deleted.
#[test]
fn create_does_not_resurrect_a_stale_sidecar() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("stale.arcline");

    // Session A: unsaved edit leaves snapshot-1 for PROJECT A.
    let a = EngineHost::create(
        &path,
        arc_core::model::Project::new(
            "PROJECT A",
            FrameRate::Fps30,
            arc_core::model::Resolution { w: 640, h: 360 },
        ),
        catalog(),
        Durability::Default,
        false,
    )
    .expect("create A");
    apply(
        &a,
        vec![Command::ProjectRename {
            name: "A saved".into(),
        }],
    );
    a.save().expect("save A");
    apply(
        &a,
        vec![Command::ProjectRename {
            name: "A unsaved".into(),
        }],
    );
    drop(a);
    assert!(ProjectPaths::new(&path).sidecar.join("snapshot-1.json").exists());

    // The user deletes the container; the sidecar survives.
    std::fs::remove_file(&path).expect("delete container");

    // An agent creates PROJECT B at the same path and also leaves without saving.
    let b = EngineHost::create(
        &path,
        arc_core::model::Project::new(
            "PROJECT B",
            FrameRate::Fps30,
            arc_core::model::Resolution { w: 640, h: 360 },
        ),
        catalog(),
        Durability::Default,
        false,
    )
    .expect("create B");
    apply(
        &b,
        vec![Command::ProjectRename {
            name: "B unsaved".into(),
        }],
    );
    drop(b);

    // Reopening must be PROJECT B, not resurrected PROJECT A.
    let c = EngineHost::open(&path, catalog(), Durability::Default).expect("open");
    assert_eq!(c.published().project.meta.name, "B unsaved");
    assert_eq!(c.published().revision, 1);
    c.shutdown();

    // The stale sidecar was moved aside, never deleted.
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .expect("dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        entries
            .iter()
            .any(|n| n.contains(".stale-") && n.starts_with("stale.arcline.autosave")),
        "{entries:?}"
    );
}

/// Audit F-05: when the container identifies a different project than the sidecar snapshots, the
/// sidecar is not replayed (and the journal that belongs to the foreign project is not applied).
#[test]
fn snapshots_from_another_project_are_ignored() {
    let dir = tempfile::tempdir().expect("tmp");
    let a_path = dir.path().join("a.arcline");
    let b_path = dir.path().join("b.arcline");

    // PROJECT A saves revision 1, then dies with unsaved revision 2 in its sidecar.
    let a = EngineHost::create(
        &a_path,
        arc_core::model::Project::new(
            "PROJECT A",
            FrameRate::Fps30,
            arc_core::model::Resolution { w: 640, h: 360 },
        ),
        catalog(),
        Durability::Default,
        false,
    )
    .expect("create A");
    apply(
        &a,
        vec![Command::TrackAdd {
            id: Some(track()),
            kind: TrackKind::Video,
            name: None,
            index: None,
        }],
    );
    a.save().expect("save A");
    apply(
        &a,
        vec![Command::ProjectRename {
            name: "A unsaved".into(),
        }],
    );
    drop(a);
    let stale_sidecar = ProjectPaths::new(&a_path).sidecar;
    assert!(stale_sidecar.join("snapshot-1.json").exists());
    assert!(stale_sidecar.join("journal-1.bin").exists());

    // PROJECT B is saved cleanly elsewhere, then its container replaces A's file while A's
    // sidecar is still there.
    let b = EngineHost::create(
        &b_path,
        arc_core::model::Project::new(
            "PROJECT B",
            FrameRate::Fps30,
            arc_core::model::Resolution { w: 640, h: 360 },
        ),
        catalog(),
        Durability::Default,
        false,
    )
    .expect("create B");
    apply(
        &b,
        vec![Command::TrackAdd {
            id: Some(track()),
            kind: TrackKind::Video,
            name: None,
            index: None,
        }],
    );
    b.save().expect("save B");
    b.shutdown();
    std::fs::copy(&b_path, &a_path).expect("replace container");

    let opened = EngineHost::open(&a_path, catalog(), Durability::Default).expect("open");
    assert_eq!(opened.published().project.meta.name, "PROJECT B");
    assert_eq!(opened.published().revision, 1);
    assert_eq!(opened.published().project.sequence().map(|s| s.tracks.len()), Some(1));
    opened.shutdown();
}

/// Audit F-01 (experiment F): a journal record that decodes but fails replay validation must not
/// silently swallow later acknowledged edits. The rejected record is kept for support, the journal
/// restarts at the recovered revision, and the next edits survive the next open.
#[test]
fn rejected_replay_record_does_not_lose_later_edits() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("reject.arcline");

    // Session 1: build and save revision 1, then add an effect (revision 2) and die without saving.
    let host = EngineHost::create(
        &path,
        project(FrameRate::Fps30),
        catalog(),
        Durability::Default,
        false,
    )
    .expect("create");
    apply(&host, setup());
    assert_eq!(host.save().expect("save"), 1);
    apply(
        &host,
        vec![Command::EffectAdd {
            id: Some(EffectId::parse("fx_a").expect("effect id")),
            clip: clip(),
            effect: "core.color.correct".into(),
            params: Default::default(),
        }],
    );
    drop(host); // the sidecar keeps revision 2

    // Session 2: a catalog that no longer knows the effect rejects revision 2's record.
    let host = EngineHost::open(&path, empty_catalog(), Durability::Default).expect("open 2");
    assert!(
        host.recovery().discarded_records,
        "replay rejection was reported"
    );
    assert_eq!(host.recovery().replayed, 0);
    let r = host
        .apply(ApplyRequest {
            commands: vec![Command::ProjectRename {
                name: "Edited in session 2".into(),
            }],
            ..Default::default()
        })
        .expect("edit after rejection");
    assert_eq!(r.revision, 2);
    drop(host);

    // The rejected bytes are kept for support (docs/08 §4.6).
    let paths = arc_core::store::ProjectPaths::new(&path);
    let rejected = paths.sidecar.join("journal-1.bin.rejected");
    assert!(
        std::fs::metadata(&rejected).is_ok_and(|m| m.len() > 0),
        "rejected journal kept at {}",
        rejected.display()
    );

    // Session 3: the acknowledged session-2 edit must be there.
    let host = EngineHost::open(&path, catalog(), Durability::Default).expect("open 3");
    assert_eq!(host.recovery().replayed, 1);
    assert_eq!(host.published().revision, 2);
    assert_eq!(host.published().project.meta.name, "Edited in session 2");
    host.shutdown();
}
