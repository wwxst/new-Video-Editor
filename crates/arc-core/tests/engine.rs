mod common;

use arc_core::engine::{ApplyRequest, Control, Source};
use arc_core::error::ErrorKind;
use arc_core::{Command, Time};
use common::*;
use serde_json::json;

fn kind_of(r: Result<impl std::fmt::Debug, arc_core::CommandError>) -> ErrorKind {
    r.expect_err("expected error").kind
}

#[test]
fn add_clip_and_describe_state() {
    let mut e = engine_with_media();
    let r = e
        .apply(req(cmds(json!([
            {"op": "timeline.add_clip", "id": "cl_1", "track": "tr_v1", "asset": "as_a",
             "start": "00:00:00:00", "duration": "00:00:05:00", "source_in": "00:00:10:00"}
        ]))))
        .expect("add");
    assert_eq!(r.result.revision, 2);
    assert_eq!(r.result.diff.clips_added.len(), 1);
    let p = e.snapshot();
    let c = p
        .sequence()
        .expect("seq")
        .clip(&"cl_1".parse_clip())
        .expect("clip")
        .clone();
    assert_eq!(c.start, Time::ZERO);
    assert_eq!(c.duration, Time::secs(5));
    assert_eq!(c.source_in, Time::secs(10));
    // the journaled form is resolved: timecodes became exact rationals
    let j = serde_json::to_value(&r.entry.commands[0]).expect("ser");
    assert_eq!(j["start"], json!({"num": 0, "den": 1}));
}

trait ParseClip {
    fn parse_clip(&self) -> arc_core::ids::ClipId;
}
impl ParseClip for &str {
    fn parse_clip(&self) -> arc_core::ids::ClipId {
        arc_core::ids::ClipId::parse(self).expect("clip id")
    }
}

#[test]
fn rejects_overlap_and_off_boundary_and_unknowns() {
    let mut e = engine_with_media();
    e.apply(req(cmds(json!([{"op":"timeline.add_clip","id":"cl_1","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":5,"den":1}}])))).expect("ok");
    let overlap = cmds(
        json!([{"op":"timeline.add_clip","track":"tr_v1","asset":"as_a","start":{"num":4,"den":1},"duration":{"num":2,"den":1}}]),
    );
    assert_eq!(kind_of(e.apply(req(overlap))), ErrorKind::Overlap);
    let off = cmds(
        json!([{"op":"timeline.add_clip","track":"tr_v2","asset":"as_a","start":{"num":1,"den":60},"duration":{"num":1,"den":1}}]),
    );
    assert_eq!(kind_of(e.apply(req(off))), ErrorKind::OffFrameBoundary);
    let unknown =
        cmds(json!([{"op":"timeline.move_clip","clip":"cl_nope","start":{"num":0,"den":1}}]));
    assert_eq!(kind_of(e.apply(req(unknown))), ErrorKind::UnknownClip);
    let past_media = cmds(
        json!([{"op":"timeline.add_clip","track":"tr_v2","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":61,"den":1}}]),
    );
    assert_eq!(
        kind_of(e.apply(req(past_media))),
        ErrorKind::SourceRangeExceeded
    );
    let df_wrong_rate = cmds(json!([{"op":"marker.add","at":"00:00:01;00","label":"x"}]));
    assert_eq!(
        kind_of(e.apply(req(df_wrong_rate))),
        ErrorKind::DropFrameNotAllowed
    );
    let wrong_kind = cmds(
        json!([{"op":"title.add","track":"tr_a1","text":"hi","start":{"num":0,"den":1},"duration":{"num":1,"den":1}}]),
    );
    assert_eq!(
        kind_of(e.apply(req(wrong_kind))),
        ErrorKind::TrackKindMismatch
    );
    // failed batches leave state untouched
    assert_eq!(e.revision(), 2);
}

#[test]
fn batches_are_atomic_and_errors_carry_paths() {
    let mut e = engine_with_media();
    let before = e.snapshot().canonical_hash();
    let err = e
        .apply(req(cmds(json!([
            {"op":"timeline.add_clip","id":"cl_ok","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":1,"den":1}},
            {"op":"timeline.add_clip","track":"tr_missing","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":1,"den":1}}
        ]))))
        .expect_err("second command fails");
    assert_eq!(err.kind, ErrorKind::UnknownTrack);
    assert_eq!(err.path.as_deref(), Some("track"));
    assert_eq!(e.snapshot().canonical_hash(), before);
    assert_eq!(e.revision(), 1);
}

#[test]
fn stale_revision_dry_run_idempotency() {
    let mut e = engine_with_media();
    let c = cmds(json!([{"op":"marker.add","at":{"num":1,"den":1},"label":"m"}]));
    let stale = ApplyRequest {
        base_revision: Some(0),
        commands: c.clone(),
        ..Default::default()
    };
    let err = e.apply(stale).expect_err("stale");
    assert_eq!(err.kind, ErrorKind::StaleRevision);
    assert_eq!(
        err.details
            .as_ref()
            .and_then(|d| d["current_revision"].as_u64()),
        Some(1)
    );

    let dry = ApplyRequest {
        commands: c.clone(),
        dry_run: true,
        ..Default::default()
    };
    let r = e.apply(dry).expect("dry");
    assert!(r.result.dry_run);
    assert_eq!(e.revision(), 1);

    let k = ApplyRequest {
        commands: c.clone(),
        idempotency_key: Some("k1".into()),
        ..Default::default()
    };
    let first = e.apply(k.clone()).expect("first");
    let again = e.apply(k).expect("again");
    assert_eq!(first.result, again.result);
    assert!(
        again.entry.commands.is_empty(),
        "idempotent replay must not be journaled"
    );
    assert_eq!(e.revision(), 2);
}

#[test]
fn clients_cannot_send_internal_restore() {
    let mut e = engine_with_media();
    let inv = arc_core::command::inverse_of(&e.snapshot(), &e.snapshot());
    let r = ApplyRequest {
        commands: vec![inv],
        source: Some(Source::Client("agent".into())),
        ..Default::default()
    };
    assert_eq!(kind_of(e.apply(r)), ErrorKind::ForbiddenOp);
}

#[test]
fn trim_split_ripple_and_transitions() {
    let mut e = engine_with_media();
    e.apply(req(cmds(json!([
        {"op":"timeline.add_clip","id":"cl_a","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":4,"den":1},"source_in":{"num":2,"den":1}},
        {"op":"timeline.add_clip","id":"cl_b","track":"tr_v1","asset":"as_a","start":{"num":4,"den":1},"duration":{"num":4,"den":1},"source_in":{"num":20,"den":1}},
        {"op":"timeline.add_clip","id":"cl_c","track":"tr_v1","asset":"as_a","start":{"num":10,"den":1},"duration":{"num":2,"den":1},"source_in":{"num":30,"den":1}}
    ])))).expect("clips");
    // transition needs adjacency and handles
    e.apply(req(cmds(json!([{"op":"timeline.add_transition","id":"tx_1","a":"cl_a","b":"cl_b","duration":"00:00:01:00"}])))).expect("tx");
    let bad = cmds(
        json!([{"op":"timeline.add_transition","a":"cl_b","b":"cl_c","duration":{"num":1,"den":1}}]),
    );
    assert_eq!(kind_of(e.apply(req(bad))), ErrorKind::TransitionBroken);
    // moving cl_b breaks adjacency → rejected
    let mv = cmds(json!([{"op":"timeline.move_clip","clip":"cl_b","start":{"num":5,"den":1}}]));
    assert_eq!(kind_of(e.apply(req(mv))), ErrorKind::TransitionBroken);
    // split cl_a: transition follows the right part
    e.apply(req(cmds(
        json!([{"op":"timeline.split_clip","clip":"cl_a","at":{"num":2,"den":1},"new_id":"cl_a2"}]),
    )))
    .expect("split");
    let p = e.snapshot();
    let s = p.sequence().expect("seq");
    let a2 = s.clip(&"cl_a2".parse_clip()).expect("a2");
    assert_eq!(a2.source_in, Time::secs(4));
    assert_eq!(s.tracks[0].transitions[0].a, "cl_a2".parse_clip());
    // ripple remove shifts later clips left on the same track
    e.apply(req(cmds(
        json!([{"op":"timeline.remove_clip","clip":"cl_c","ripple":"none"}]),
    )))
    .expect("rm c");
    e.apply(req(cmds(
        json!([{"op":"timeline.trim_clip","clip":"cl_b","edge":"out","to":{"num":7,"den":1}}]),
    )))
    .expect("trim");
    e.apply(req(cmds(
        json!([{"op":"timeline.remove_clip","clip":"cl_a","ripple":"track"}]),
    )))
    .expect("ripple");
    let p = e.snapshot();
    let s = p.sequence().expect("seq");
    assert_eq!(s.clip(&"cl_a2".parse_clip()).expect("a2").start, Time::ZERO);
    assert_eq!(
        s.clip(&"cl_b".parse_clip()).expect("b").start,
        Time::secs(2)
    );
    // in-edge trim adjusts source_in
    e.apply(req(cmds(
        json!([{"op":"timeline.trim_clip","clip":"cl_b","edge":"in","to":{"num":3,"den":1}}]),
    )))
    .expect_err("breaks transition");
}

#[test]
fn effects_validate_against_manifest() {
    let mut e = engine_with_media();
    e.apply(req(cmds(json!([{"op":"timeline.add_clip","id":"cl_1","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":2,"den":1}}])))).expect("clip");
    let r = e
        .apply(req(cmds(
            json!([{"op":"effect.add","id":"fx_1","clip":"cl_1","effect":"core.color.correct"}]),
        )))
        .expect("fx");
    // defaults are filled into the resolved command
    let j = serde_json::to_value(&r.entry.commands[0]).expect("ser");
    assert_eq!(j["params"]["contrast"], json!({"static": 1.0}));
    let out_of_range = cmds(
        json!([{"op":"effect.set_params","clip":"cl_1","effect":"fx_1","params":{"exposure":{"static":9.0}}}]),
    );
    assert_eq!(kind_of(e.apply(req(out_of_range))), ErrorKind::OutOfRange);
    let unknown_param = cmds(
        json!([{"op":"effect.set_params","clip":"cl_1","effect":"fx_1","params":{"gamma":{"static":1.0}}}]),
    );
    assert_eq!(
        kind_of(e.apply(req(unknown_param))),
        ErrorKind::InvalidValue
    );
    let unknown_fx = cmds(json!([{"op":"effect.add","clip":"cl_1","effect":"core.nope"}]));
    assert_eq!(kind_of(e.apply(req(unknown_fx))), ErrorKind::UnknownEffect);
    let curve = cmds(
        json!([{"op":"effect.set_params","clip":"cl_1","effect":"fx_1","params":{"exposure":{"curve":[
        {"t":{"num":0,"den":1},"v":0.0,"interp":"linear"},{"t":{"num":1,"den":1},"v":1.0,"interp":"linear"}]}}}]),
    );
    e.apply(req(curve)).expect("curve");
}

#[test]
fn undo_redo_restore_exact_state() {
    let mut e = engine_with_media();
    let h0 = e.snapshot().canonical_hash();
    e.apply(req(cmds(json!([{"op":"timeline.add_clip","id":"cl_1","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":2,"den":1}}])))).expect("a");
    let h1 = e.snapshot().canonical_hash();
    e.apply(req(cmds(json!([{"op":"project.rename","name":"Renamed"}]))))
        .expect("b");
    e.undo(Source::Human).expect("undo 1");
    assert_eq!(e.snapshot().canonical_hash(), h1);
    e.undo(Source::Human).expect("undo 2");
    assert_eq!(e.snapshot().canonical_hash(), h0);
    e.redo(Source::Human).expect("redo");
    assert_eq!(e.snapshot().canonical_hash(), h1);
    assert_eq!(e.revision(), 6, "undo and redo each create a revision");
    // a new edit clears redo
    e.apply(req(cmds(
        json!([{"op":"marker.add","at":{"num":0,"den":1},"label":"m"}]),
    )))
    .expect("c");
    assert_eq!(kind_of(e.redo(Source::Human)), ErrorKind::NothingToRedo);
}

#[test]
fn control_and_takeover() {
    let mut e = engine_with_media();
    let agent = Source::Client("agent_1".into());
    let mk = |src: &Source| ApplyRequest {
        commands: cmds(json!([{"op":"marker.add","at":{"num":0,"den":1},"label":"x"}])),
        source: Some(src.clone()),
        ..Default::default()
    };
    e.apply(mk(&agent)).expect("shared allows agents");
    e.set_control(Control::Human);
    assert_eq!(kind_of(e.apply(mk(&agent))), ErrorKind::NotInControl);
    e.set_control(Control::Agent("agent_1".into()));
    e.apply(mk(&agent)).expect("holder may edit");
    assert_eq!(
        kind_of(e.apply(mk(&Source::Client("other".into())))),
        ErrorKind::NotInControl
    );
    // a human edit while an agent holds control takes over automatically
    e.apply(mk(&Source::Human)).expect("human always allowed");
    assert_eq!(e.control(), &Control::Human);
    assert_eq!(kind_of(e.apply(mk(&agent))), ErrorKind::NotInControl);
}

#[test]
fn rollback_unconfirmed_restores_last_acknowledged_state() {
    let mut e = engine_with_media();
    e.confirm(e.revision());
    let acked = e.snapshot().canonical_hash();
    let rev = e.revision();
    e.apply(req(cmds(json!([{"op":"project.rename","name":"A"}]))))
        .expect("1");
    e.apply(req(cmds(json!([{"op":"project.rename","name":"B"}]))))
        .expect("2");
    let rolled = e.rollback_unconfirmed();
    assert_eq!(rolled, vec![rev + 1, rev + 2]);
    assert_eq!(e.revision(), rev);
    assert_eq!(e.snapshot().canonical_hash(), acked);
}

#[test]
fn replay_reproduces_revisions() {
    let mut a = engine_with_media();
    let base = (*a.snapshot()).clone();
    let base_rev = a.revision();
    let mut entries = Vec::new();
    for c in [
        json!([{"op":"timeline.add_clip","id":"cl_1","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":2,"den":1}}]),
        json!([{"op":"timeline.split_clip","clip":"cl_1","at":{"num":1,"den":1}}]),
        json!([{"op":"effect.add","clip":"cl_1","effect":"core.color.correct"}]),
    ] {
        entries.push(a.apply(req(cmds(c))).expect("apply").entry);
    }
    entries.push(a.undo(Source::Human).expect("undo").entry);
    let mut b = arc_core::Engine::new(base, base_rev, catalog()).expect("b");
    for en in &entries {
        b.replay(en).expect("replay");
    }
    assert_eq!(a.snapshot().canonical_hash(), b.snapshot().canonical_hash());
    assert_eq!(a.revision(), b.revision());
    // history rebuilt: redo works after replay
    b.redo(Source::Human).expect("redo after replay");
}

/// F-12: replaying a redo must pop the redo stack, exactly like `Engine::redo` does live.
/// Replaying it used to push a fresh undo entry and leave the redo stack intact, so a second redo
/// after recovery re-applied an already-applied batch (audit F-12, experiment E).
#[test]
fn replayed_redo_does_not_leave_a_phantom_redo() {
    let mut a = engine_with_media();
    let base = (*a.snapshot()).clone();
    let base_rev = a.revision();
    let mut entries = Vec::new();
    entries.push(
        a.apply(req(cmds(json!([
            {"op":"timeline.add_clip","id":"cl_1","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":2,"den":1}}
        ]))))
        .expect("add")
        .entry,
    );
    entries.push(a.undo(Source::Human).expect("undo").entry);
    entries.push(a.redo(Source::Human).expect("redo").entry);
    assert!(!a.can_redo(), "live redo consumed the redo stack");
    assert!(a.can_undo());

    let mut b = arc_core::Engine::new(base, base_rev, catalog()).expect("b");
    for en in &entries {
        b.replay(en).expect("replay");
    }
    assert_eq!(a.snapshot().canonical_hash(), b.snapshot().canonical_hash());
    assert_eq!(a.revision(), b.revision());
    assert!(!b.can_redo(), "a replayed redo must not leave a phantom redo");
    assert!(b.can_undo());
    let err = b.redo(Source::Human).expect_err("nothing left to redo");
    assert_eq!(err.kind, arc_core::ErrorKind::NothingToRedo);
    let _ = Command::ProjectRename {
        name: String::new(),
    };
}
