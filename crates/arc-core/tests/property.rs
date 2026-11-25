//! Invariant 7 (docs/02 §9): applying a batch and then its inverse restores a document whose
//! canonical hash is identical; redo returns to the post-batch hash. Every committed state must
//! also satisfy all other invariants (`validate_project`).

mod common;

use arc_core::command::validate_project;
use arc_core::engine::{Engine, Source};
use common::*;
use proptest::prelude::*;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
enum Op {
    AddClip {
        track: u8,
        start: u16,
        dur: u16,
        src: u16,
    },
    AddTitle {
        start: u16,
        dur: u16,
    },
    Move {
        pick: u16,
        start: u16,
    },
    Trim {
        pick: u16,
        out_edge: bool,
        to: u16,
    },
    Split {
        pick: u16,
        at: u16,
    },
    Remove {
        pick: u16,
        ripple: u8,
    },
    Effect {
        pick: u16,
        exposure: i8,
    },
    Gain {
        pick: u16,
        db: i8,
    },
    Marker {
        at: u16,
    },
    Rename {
        n: u8,
    },
    AddTrack {
        video: bool,
    },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0u8..3, 0u16..300, 1u16..90, 0u16..900).prop_map(|(track, start, dur, src)| Op::AddClip {
            track,
            start,
            dur,
            src
        }),
        (0u16..300, 1u16..90).prop_map(|(start, dur)| Op::AddTitle { start, dur }),
        (any::<u16>(), 0u16..300).prop_map(|(pick, start)| Op::Move { pick, start }),
        (any::<u16>(), any::<bool>(), 0u16..400).prop_map(|(pick, out_edge, to)| Op::Trim {
            pick,
            out_edge,
            to
        }),
        (any::<u16>(), 0u16..400).prop_map(|(pick, at)| Op::Split { pick, at }),
        (any::<u16>(), 0u8..3).prop_map(|(pick, ripple)| Op::Remove { pick, ripple }),
        (any::<u16>(), -5i8..=5).prop_map(|(pick, exposure)| Op::Effect { pick, exposure }),
        (any::<u16>(), -30i8..=6).prop_map(|(pick, db)| Op::Gain { pick, db }),
        (0u16..400).prop_map(|at| Op::Marker { at }),
        any::<u8>().prop_map(|n| Op::Rename { n }),
        any::<bool>().prop_map(|video| Op::AddTrack { video }),
    ]
}

/// Frames at 30 fps as an exact rational.
fn f(n: u16) -> Value {
    json!({"num": n as i64, "den": 30})
}

fn concrete(e: &Engine, op: &Op) -> Option<Value> {
    let p = e.snapshot();
    let s = p.sequence()?;
    let clips: Vec<_> = s.tracks.iter().flat_map(|t| t.clips.iter()).collect();
    let pick = |i: u16| {
        clips
            .get(i as usize % clips.len().max(1))
            .map(|c| c.id.to_string())
    };
    Some(match op {
        Op::AddClip {
            track,
            start,
            dur,
            src,
        } => {
            let tr = ["tr_v1", "tr_v2", "tr_a1"][*track as usize];
            json!({"op":"timeline.add_clip","track":tr,"asset":"as_a","start":f(*start),"duration":f(*dur),"source_in":f(*src)})
        }
        Op::AddTitle { start, dur } => {
            json!({"op":"title.add","track":"tr_v2","text":"T","start":f(*start),"duration":f(*dur)})
        }
        Op::Move { pick: i, start } => {
            json!({"op":"timeline.move_clip","clip":pick(*i)?,"start":f(*start)})
        }
        Op::Trim {
            pick: i,
            out_edge,
            to,
        } => {
            let edge = if *out_edge { "out" } else { "in" };
            json!({"op":"timeline.trim_clip","clip":pick(*i)?,"edge":edge,"to":f(*to)})
        }
        Op::Split { pick: i, at } => {
            json!({"op":"timeline.split_clip","clip":pick(*i)?,"at":f(*at)})
        }
        Op::Remove { pick: i, ripple } => {
            let r = ["none", "track", "all"][*ripple as usize];
            json!({"op":"timeline.remove_clip","clip":pick(*i)?,"ripple":r})
        }
        Op::Effect { pick: i, exposure } => {
            json!({"op":"effect.add","clip":pick(*i)?,"effect":"core.color.correct","params":{"exposure":{"static":*exposure as f64}}})
        }
        Op::Gain { pick: i, db } => {
            json!({"op":"audio.set_gain","clip":pick(*i)?,"gain_db":*db as f64})
        }
        Op::Marker { at } => json!({"op":"marker.add","at":f(*at),"label":"m"}),
        Op::Rename { n } => {
            let name = format!("P{n}");
            json!({"op":"project.rename","name":name})
        }
        Op::AddTrack { video } => {
            let kind = if *video { "video" } else { "audio" };
            json!({"op":"track.add","kind":kind})
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, max_shrink_iters: 2_000, .. ProptestConfig::default() })]

    #[test]
    fn apply_undo_redo_round_trips(batches in prop::collection::vec(prop::collection::vec(op(), 1..4), 1..6)) {
        let mut e = engine_with_media();
        let cat = catalog();
        for batch in &batches {
            let list: Vec<Value> = batch.iter().filter_map(|o| concrete(&e, o)).collect();
            if list.is_empty() { continue; }
            let before = e.snapshot().canonical_hash();
            let rev = e.revision();
            match e.apply(req(cmds(Value::Array(list)))) {
                Ok(_) => {
                    let after = e.snapshot();
                    prop_assert!(validate_project(&after, cat.as_ref()).is_ok());
                    let after_hash = after.canonical_hash();
                    e.undo(Source::Human).expect("undo");
                    prop_assert_eq!(e.snapshot().canonical_hash(), before.clone());
                    e.redo(Source::Human).expect("redo");
                    prop_assert_eq!(e.snapshot().canonical_hash(), after_hash);
                }
                Err(_) => {
                    // rejected batches change nothing
                    prop_assert_eq!(e.snapshot().canonical_hash(), before);
                    prop_assert_eq!(e.revision(), rev);
                }
            }
        }
        // unwinding everything returns to the initial state
        let mut guard = 0;
        while e.can_undo() && guard < 100 { e.undo(Source::Human).expect("undo all"); guard += 1; }
        let fresh = engine_with_media();
        let a = e.snapshot();
        let b = fresh.snapshot();
        prop_assert_eq!(a.sequence().map(|s| s.tracks.len()), Some(0));
        prop_assert_eq!(b.sequence().map(|s| s.tracks.len()), Some(3));
    }
}
