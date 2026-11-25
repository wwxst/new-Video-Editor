#![allow(dead_code)]

use arc_core::effects::{EffectManifest, ParamSpec, ParamType, StaticCatalog};
use arc_core::engine::{ApplyRequest, Engine};
use arc_core::model::{AssetKind, AudioInfo, Probe, Project, Resolution, VideoInfo};
use arc_core::{Command, FrameRate, Ratio, Time, TimeInput};
use std::sync::Arc;

pub fn catalog() -> Arc<StaticCatalog> {
    Arc::new(StaticCatalog::new([EffectManifest {
        id: "core.color.correct".into(),
        version: 1,
        display: "Color Correction".into(),
        category: vec!["color".into()],
        params: vec![
            ParamSpec {
                key: "exposure".into(),
                ty: ParamType::Float,
                range: [-5.0, 5.0],
                default: 0.0,
                unit: Some("stops".into()),
            },
            ParamSpec {
                key: "contrast".into(),
                ty: ParamType::Float,
                range: [0.2, 2.0],
                default: 1.0,
                unit: None,
            },
        ],
    }]))
}

pub fn project(rate: FrameRate) -> Project {
    Project::new("Test", rate, Resolution { w: 1920, h: 1080 })
}

pub fn secs(s: i64) -> TimeInput {
    TimeInput::Exact(Time::secs(s))
}

pub fn t(n: i64, d: i64) -> TimeInput {
    TimeInput::Exact(Time::new(n, d).expect("time"))
}

pub fn tc(s: &str) -> TimeInput {
    TimeInput::Timecode(s.into())
}

pub fn video_probe(seconds: i64) -> Probe {
    Probe {
        duration: Time::secs(seconds),
        video: Some(VideoInfo {
            width: 1920,
            height: 1080,
            frame_rate: Ratio::new(30, 1).expect("r"),
            vfr: false,
            codec: "h264".into(),
            pix_fmt: "yuv420p".into(),
            rotation: 0,
        }),
        audio: Some(AudioInfo {
            sample_rate: 48_000,
            channels: 2,
            codec: "aac".into(),
        }),
    }
}

pub fn req(commands: Vec<Command>) -> ApplyRequest {
    ApplyRequest {
        commands,
        ..Default::default()
    }
}

pub fn cmds(json: serde_json::Value) -> Vec<Command> {
    serde_json::from_value(json).expect("commands")
}

/// Engine at 30 fps with tracks tr_v1/tr_v2 (video), tr_a1 (audio) and a 60 s asset as_a.
pub fn engine_with_media() -> Engine {
    let mut e = Engine::new(project(FrameRate::Fps30), 0, catalog()).expect("engine");
    e.apply(req(vec![
        Command::TrackAdd {
            id: Some("tr_v1".parse_id()),
            kind: arc_core::model::TrackKind::Video,
            name: None,
            index: None,
        },
        Command::TrackAdd {
            id: Some("tr_v2".parse_id()),
            kind: arc_core::model::TrackKind::Video,
            name: None,
            index: None,
        },
        Command::TrackAdd {
            id: Some("tr_a1".parse_id()),
            kind: arc_core::model::TrackKind::Audio,
            name: None,
            index: None,
        },
        Command::AssetImport {
            id: Some(arc_core::ids::AssetId::parse("as_a").expect("id")),
            path: "a.mov".into(),
            kind: AssetKind::Video,
            probe: Some(video_probe(60)),
            status: Default::default(),
        },
    ]))
    .expect("setup");
    e
}

pub trait ParseId<T> {
    fn parse_id(&self) -> T;
}
impl ParseId<arc_core::ids::TrackId> for &str {
    fn parse_id(&self) -> arc_core::ids::TrackId {
        arc_core::ids::TrackId::parse(self).expect("track id")
    }
}
