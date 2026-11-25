//! Frame planning (pure): active clips, source times, transition weights, effect evaluation.

use arc_core::engine::{ApplyRequest, Engine};
use arc_core::ids::AssetId;
use arc_core::{Command, FrameRate, Time};
use arc_render::plan::{SourcePlan, frame_count, plan_frame};
use serde_json::json;

fn engine() -> Engine {
    let p = arc_core::model::Project::new(
        "t",
        FrameRate::Fps30,
        arc_core::model::Resolution { w: 640, h: 360 },
    );
    let mut e = Engine::new(p, 0, arc_render::builtin::catalog()).expect("engine");
    let cmds: Vec<Command> = serde_json::from_value(json!([
        {"op":"track.add","id":"tr_v1","kind":"video"},
        {"op":"track.add","id":"tr_v2","kind":"video"},
        {"op":"asset.import","id":"as_a","path":"a.mkv","kind":"video","probe":{"duration":{"num":60,"den":1}}},
        {"op":"timeline.add_clip","id":"cl_a","track":"tr_v1","asset":"as_a","start":"00:00:00:00","duration":"00:00:02:00","source_in":"00:00:10:00"},
        {"op":"timeline.add_clip","id":"cl_b","track":"tr_v1","asset":"as_a","start":"00:00:02:00","duration":"00:00:02:00","source_in":"00:00:30:00"},
        {"op":"timeline.add_transition","a":"cl_a","b":"cl_b","duration":"00:00:00:10"},
        {"op":"effect.add","clip":"cl_b","effect":"core.color.correct","params":{"exposure":{"curve":[
            {"t":{"num":0,"den":1},"v":0.0,"interp":"linear"},{"t":{"num":2,"den":1},"v":2.0,"interp":"linear"}]}}},
        {"op":"title.add","id":"cl_t","track":"tr_v2","text":"Hello","start":"00:00:01:00","duration":"00:00:01:00"}
    ]))
    .expect("cmds");
    e.apply(ApplyRequest {
        commands: cmds,
        ..Default::default()
    })
    .expect("setup");
    e
}

fn asset(s: &str) -> AssetId {
    AssetId::parse(s).expect("asset id")
}

#[test]
fn plans_sources_transitions_and_effects() {
    let e = engine();
    let p = e.snapshot();
    assert_eq!(frame_count(&p), 120);
    // frame 15: only cl_a, source 10 s + 0.5 s
    let l = plan_frame(&p, 15);
    assert_eq!(l.len(), 1);
    assert_eq!(
        l[0].items[0].source,
        SourcePlan::Media {
            asset: asset("as_a"),
            time: Time::new(21, 2).expect("t")
        }
    );
    // frame 30: title on V2 above V1
    let l = plan_frame(&p, 30);
    assert_eq!(l.len(), 2);
    assert!(matches!(l[1].items[0].source, SourcePlan::Title { .. }));
    // frame 58: inside the 10-frame dissolve centered on frame 60 (window 55..65): w = 3/10
    let l = plan_frame(&p, 58);
    let items = &l[0].items;
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i.additive));
    assert!((items[0].opacity - 0.7).abs() < 1e-9 && (items[1].opacity - 0.3).abs() < 1e-9);
    // cl_b before its start uses its source handle: 30 s - 2 frames
    let expect = Time::secs(30)
        .checked_sub(Time::new(2, 30).expect("t"))
        .expect("s");
    assert_eq!(
        items[1].source,
        SourcePlan::Media {
            asset: asset("as_a"),
            time: expect
        }
    );
    // frame 90: cl_b local time 1 s -> exposure 1.0
    let l = plan_frame(&p, 90);
    let fx = &l[0].items[0].effects[0];
    assert_eq!(fx.params.get("exposure").copied(), Some(1.0));
    assert!(plan_frame(&p, 200).is_empty());
}
