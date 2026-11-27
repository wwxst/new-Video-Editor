//! Preview pipeline (docs/specs/phase-c-plan.md C1).
//!
//! WARP, the software adapter CI uses, crashes when several wgpu devices are created concurrently
//! in one process (batch 3), so the tests in this binary are serialized.

use arc_cli::export::{ExportSpec, export};
use arc_cli::preview::{PreviewEngine, PreviewError, PreviewQuality, TITLE_WARNING};
use arc_core::engine::{ApplyRequest, Engine};
use arc_core::ids::AssetId;
use arc_core::model::{Project, Resolution};
use arc_core::time::{FrameRate, Time};
use arc_core::{Command, StaticCatalog};
use arc_media::VideoDecoder;
use arc_media::encode::VideoCodec;
use arc_media::fixtures::{read_counter, write_counter_video};
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

static GPU_SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    GPU_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 30-frame, 1 s, 320×180 counter video and a project with one clip on V1.
fn project_with_clip(dir: &Path) -> Project {
    project_with_source_rate(dir, FrameRate::Fps30, 30)
}

fn project_with_source_rate(dir: &Path, source_rate: FrameRate, frames: u32) -> Project {
    project_with_rates(dir, FrameRate::Fps30, source_rate, frames)
}

fn project_with_rates(
    dir: &Path,
    project_rate: FrameRate,
    source_rate: FrameRate,
    frames: u32,
) -> Project {
    let media = dir.join("counter.mkv");
    write_counter_video(&media, frames, source_rate.rate(), 320, 180, false).expect("fixture");
    let source_duration = source_rate.time_of_frame(frames as i64);
    let mut e = Engine::new(
        Project::new("Preview", project_rate, Resolution { w: 320, h: 180 }),
        0,
        Arc::new(StaticCatalog::default()),
    )
    .expect("engine");
    // 29.97/59.94 need a whole number of project frames, not 1.0 s (which is off-boundary).
    let duration = project_rate.time_of_frame(project_rate.nominal());
    let commands: Vec<Command> = vec![
        json!({"op":"track.add","id":"tr_v1","kind":"video"}),
        json!({"op":"asset.import","id":"as_a","path":media.to_str().expect("utf8"),"kind":"video","probe":{"duration":{"num":source_duration.num(),"den":source_duration.den()}}}),
        json!({"op":"timeline.add_clip","id":"cl_1","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":duration.num(),"den":duration.den()}}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).expect("command"))
    .collect();
    e.apply(ApplyRequest {
        commands,
        ..Default::default()
    })
    .expect("apply");
    (*e.snapshot()).clone()
}

fn engine(project: &Project, dir: &Path) -> PreviewEngine {
    PreviewEngine::new(project.clone(), dir.to_path_buf(), PreviewQuality::Full).expect("preview")
}

/// C1 acceptance 1: the preview frame is the export frame, pixel for pixel (FFV1 is lossless).
#[test]
fn preview_frame_matches_the_export() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());

    // Compose frame 10, then drop the engine so only one wgpu device is alive at a time (WARP).
    let preview = {
        let mut e = engine(&project, dir.path());
        e.frame(10).expect("preview frame")
    };
    assert_eq!((preview.width, preview.height), (320, 180));
    assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);

    let out = dir.path().join("frame10.mkv");
    export(
        &project,
        &ExportSpec {
            output: out.clone(),
            codec: VideoCodec::Ffv1,
            from: Some(10),
            to: Some(11),
            media_root: dir.path().to_path_buf(),
        },
        |_, _| {},
    )
    .expect("export");

    let mut decoded = VideoDecoder::open(&out).expect("open export");
    let frame = decoded.frame_at(Time::ZERO).expect("exported frame");
    assert_eq!((frame.width, frame.height), (preview.width, preview.height));
    assert_eq!(
        frame.rgba, preview.rgba,
        "preview frame 10 must equal the exported frame 10"
    );
}

/// A 24 fps source in a 30 fps project must repeat its frame in the second half of each source
/// interval. Preview and thumbnails use `frame_displayed_at`, so project frame 1 shows source frame
/// 0, not source frame 1 (the half-frame seek-bias answer).
#[test]
fn preview_uses_the_displayed_source_frame_at_project_rate_mismatch() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    // A 24 fps source in each supported project rate. Frame times are exact project-frame starts;
    // the expected source frame is the one whose display interval contains that time.
    for project_rate in [
        FrameRate::Fps24,
        FrameRate::Fps25,
        FrameRate::Fps30,
        FrameRate::Fps29_97,
        FrameRate::Fps59_94,
    ] {
        let project = project_with_rates(dir.path(), project_rate, FrameRate::Fps24, 60);
        let mut e = engine(&project, dir.path());
        let last = project_rate.nominal() - 1;
        for frame in [0, 1, 2, project_rate.nominal() / 2, last - 1, last] {
            if frame < 0 {
                continue;
            }
            let f = e.frame(frame).expect("preview frame");
            let expected = FrameRate::Fps24
                .frame_at(project_rate.time_of_frame(frame), arc_core::time::Round::Floor)
                .expect("source frame") as u32;
            assert_eq!(
                read_counter(&f.rgba, f.width, f.height),
                Some(expected),
                "{project_rate:?} project frame {frame}: source 24 fps frame {expected}"
            );
        }
    }
}

#[test]
fn export_uses_the_same_displayed_source_frame_as_preview() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_rates(dir.path(), FrameRate::Fps30, FrameRate::Fps24, 24);
    let preview = {
        let mut e = engine(&project, dir.path());
        e.frame(1).expect("preview frame 1")
    };
    assert_eq!(
        read_counter(&preview.rgba, preview.width, preview.height),
        Some(0),
        "preview project frame 1 must show source frame 0"
    );

    let out = dir.path().join("export-frame1.mkv");
    export(
        &project,
        &ExportSpec {
            output: out.clone(),
            codec: VideoCodec::Ffv1,
            from: Some(1),
            to: Some(2),
            media_root: dir.path().to_path_buf(),
        },
        |_, _| {},
    )
    .expect("export");
    let mut decoded = VideoDecoder::open(&out).expect("open export");
    let frame = decoded.frame_at(Time::ZERO).expect("exported frame");
    assert_eq!(
        read_counter(frame.rgba, frame.width, frame.height),
        Some(0),
        "export project frame 1 must match preview's source frame 0"
    );
}

/// C1 acceptance 2: half quality halves the output while decoding the source at native size.
#[test]
fn half_quality_halves_the_frame() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let mut e = PreviewEngine::new(project, dir.path().to_path_buf(), PreviewQuality::Half)
        .expect("preview");
    let f = e.frame(0).expect("half frame");
    assert_eq!((f.width, f.height), (160, 90));
    assert_eq!(f.rgba.len(), 160 * 90 * 4);
}

/// C1 acceptance 3: the decoded-frame cache serves repeats and a one-frame step decodes one frame.
#[test]
fn repeated_frames_are_cache_hits_and_steps_decode_once() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let mut e = engine(&project, dir.path());

    let first = e.frame(10).expect("frame 10");
    assert_eq!(e.stats().decodes, 1, "one source frame for one clip");
    let again = e.frame(10).expect("frame 10 again");
    assert_eq!(again.rgba, first.rgba);
    assert_eq!(e.stats().decodes, 1, "a repeat must not decode");
    assert_eq!(e.stats().cache_hits, 1);

    e.frame(11).expect("frame 11");
    assert_eq!(e.stats().decodes, 2, "a one-frame step decodes one frame");
    e.frame(10).expect("frame 10 after 11");
    assert_eq!(e.stats().decodes, 2, "cached frames are served from memory");
    assert_eq!(e.stats().frames, 4);
}

/// C1 acceptance 4: an unavailable asset is reported and the frame still renders (black source).
#[test]
fn unavailable_asset_warns_and_renders_the_background() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    std::fs::remove_file(dir.path().join("counter.mkv")).expect("remove media");

    let mut e = engine(&project, dir.path());
    let f = e.frame(0).expect("frame with a missing source");
    assert!(
        f.warnings.iter().any(|w| w.contains("unavailable")),
        "warnings: {:?}",
        f.warnings
    );
    assert!(
        f.rgba
            .iter()
            .enumerate()
            .all(|(i, b)| i % 4 == 3 || *b == 0),
        "a missing source must render the background, not a stale frame"
    );
    assert_eq!(e.stats().decodes, 0);
}

/// C1 acceptance 4: title clips are reported as unsupported and skipped, like the exporter does.
#[test]
fn title_clips_are_reported_and_skipped() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let mut e = Engine::new(
        Project::new("Titles", FrameRate::Fps30, Resolution { w: 320, h: 180 }),
        0,
        Arc::new(StaticCatalog::default()),
    )
    .expect("engine");
    let commands: Vec<Command> = vec![
        json!({"op":"track.add","id":"tr_v1","kind":"video"}),
        json!({"op":"title.add","track":"tr_v1","text":"Hello","start":{"num":0,"den":1},"duration":{"num":1,"den":1}}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).expect("command"))
    .collect();
    e.apply(ApplyRequest {
        commands,
        ..Default::default()
    })
    .expect("apply");
    let project = (*e.snapshot()).clone();

    let mut preview = engine(&project, dir.path());
    let f = preview.frame(0).expect("title frame");
    assert!(
        f.warnings.iter().any(|w| w == TITLE_WARNING),
        "warnings: {:?}",
        f.warnings
    );
}

/// C1: a frame outside the sequence is a typed error, not a panic or a wrong frame.
#[test]
fn out_of_range_frame_is_a_typed_error() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let mut e = engine(&project, dir.path());
    match e.frame(30) {
        Err(PreviewError::OutOfRange { frame, end }) => {
            assert_eq!((frame, end), (30, 30));
        }
        other => panic!("expected OutOfRange, got {other:?}"),
    }
}

/// The engine accepts a replaced snapshot (an edit) and keeps working.
#[test]
fn set_project_keeps_the_engine_usable() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let mut e = engine(&project, dir.path());
    e.frame(0).expect("before edit");
    e.set_project(project.clone());
    let f = e.frame(1).expect("after edit");
    assert_eq!((f.width, f.height), (320, 180));
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
}
/// Two tracks with a transformed overlay: the preview must still equal the export frame, which
/// exercises the multi-layer plan (Phase C is multi-track compositing).
fn project_with_two_layers(dir: &Path) -> Project {
    let media = dir.join("layers.mkv");
    write_counter_video(&media, 60, FrameRate::Fps30.rate(), 320, 180, false).expect("fixture");
    let mut e = Engine::new(
        Project::new("Layers", FrameRate::Fps30, Resolution { w: 320, h: 180 }),
        0,
        Arc::new(StaticCatalog::default()),
    )
    .expect("engine");
    let commands: Vec<Command> = vec![
        json!({"op":"track.add","id":"tr_v1","kind":"video"}),
        json!({"op":"track.add","id":"tr_v2","kind":"video"}),
        json!({"op":"asset.import","id":"as_a","path":media.to_str().expect("utf8"),"kind":"video","probe":{"duration":{"num":2,"den":1}}}),
        json!({"op":"timeline.add_clip","id":"cl_a","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":1,"den":1}}),
        json!({"op":"timeline.add_clip","id":"cl_b","track":"tr_v2","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":1,"den":1},"source_in":{"num":1,"den":1}}),
        json!({"op":"timeline.set_transform","clip":"cl_b","scale":{"x":0.5,"y":0.5},"position":{"x":0.4,"y":0.0}}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).expect("command"))
    .collect();
    e.apply(ApplyRequest {
        commands,
        ..Default::default()
    })
    .expect("apply");
    (*e.snapshot()).clone()
}

#[test]
fn preview_matches_the_export_with_two_layers() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_two_layers(dir.path());
    let preview = {
        let mut e = engine(&project, dir.path());
        e.frame(5).expect("preview frame")
    };
    let out = dir.path().join("layers5.mkv");
    export(
        &project,
        &ExportSpec {
            output: out.clone(),
            codec: VideoCodec::Ffv1,
            from: Some(5),
            to: Some(6),
            media_root: dir.path().to_path_buf(),
        },
        |_, _| {},
    )
    .expect("export");
    let mut decoded = VideoDecoder::open(&out).expect("open export");
    let frame = decoded.frame_at(Time::ZERO).expect("exported frame");
    assert_eq!(
        frame.rgba, preview.rgba,
        "two-layer preview must equal the export"
    );
}

/// The cache is bounded by bytes, so large frames cannot grow it without limit; eviction is LRU.
#[test]
fn cache_eviction_respects_the_byte_budget() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let mut e = engine(&project, dir.path());
    let frame_bytes = 320u64 * 180 * 4;
    e.set_cache_budget(frame_bytes + 1);
    e.frame(10).expect("frame 10");
    assert!(e.cache_bytes() <= frame_bytes + 1);
    e.frame(11).expect("frame 11");
    assert_eq!(e.stats().decodes, 2);
    assert!(
        e.cache_bytes() <= frame_bytes + 1,
        "evicted down to the budget"
    );
    e.frame(10).expect("frame 10 again");
    assert_eq!(e.stats().decodes, 3, "frame 10 was evicted");
}
/// Project with a chosen resolution, one clip, and a probe that may lie about the media duration.
fn project_with(dir: &Path, w: u32, h: u32, probe_seconds: i64, source_in: i64) -> Project {
    let media = dir.join("counter.mkv");
    write_counter_video(&media, 30, FrameRate::Fps30.rate(), 320, 180, false).expect("fixture");
    let mut e = Engine::new(
        Project::new("Sized", FrameRate::Fps30, Resolution { w, h }),
        0,
        Arc::new(StaticCatalog::default()),
    )
    .expect("engine");
    let commands: Vec<Command> = vec![
        json!({"op":"track.add","id":"tr_v1","kind":"video"}),
        json!({"op":"asset.import","id":"as_a","path":media.to_str().expect("utf8"),"kind":"video","probe":{"duration":{"num":probe_seconds,"den":1}}}),
        json!({"op":"timeline.add_clip","id":"cl_1","track":"tr_v1","asset":"as_a","start":{"num":0,"den":1},"duration":{"num":1,"den":1},"source_in":{"num":source_in,"den":1}}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).expect("command"))
    .collect();
    e.apply(ApplyRequest {
        commands,
        ..Default::default()
    })
    .expect("apply");
    (*e.snapshot()).clone()
}

/// C1 acceptance 2: half quality still decodes the source at its native size.
#[test]
fn half_quality_decodes_the_source_at_native_size() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let mut e = PreviewEngine::new(project, dir.path().to_path_buf(), PreviewQuality::Half)
        .expect("preview");
    let f = e.frame(0).expect("half frame");
    assert_eq!((f.width, f.height), (160, 90));
    let asset = AssetId::parse("as_a").expect("asset id");
    assert_eq!(
        e.cached_source_size(&asset, Time::ZERO),
        Some((320, 180)),
        "the decoder must run at the source resolution"
    );
}

/// Switching quality reuses the decoded sources (cache keys describe source frames).
#[test]
fn quality_switch_reuses_cached_sources() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let mut e = engine(&project, dir.path());
    let full = e.frame(10).expect("full");
    assert_eq!(full.width, 320);
    e.set_quality(PreviewQuality::Half);
    let half = e.frame(10).expect("half");
    assert_eq!((half.width, half.height), (160, 90));
    assert_eq!(e.stats().decodes, 1, "the source frame is already cached");
}

/// An odd sequence width floors at half quality (320 → 160, 321 → 160) rather than rounding up.
#[test]
fn odd_size_half_quality_floors() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with(dir.path(), 321, 181, 1, 0);
    let mut e = PreviewEngine::new(project, dir.path().to_path_buf(), PreviewQuality::Half)
        .expect("preview");
    let f = e.frame(0).expect("odd half frame");
    assert_eq!((f.width, f.height), (160, 90));
}

/// A project without a sequence is a typed error, not a panic.
#[test]
fn no_sequence_is_a_typed_error() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let mut project = project_with_clip(dir.path());
    project.sequences.clear();
    let mut e = engine(&project, dir.path());
    match e.frame(0) {
        Err(PreviewError::NoSequence) => {}
        other => panic!("expected NoSequence, got {other:?}"),
    }
}

/// A changed asset path (relink/repair) must reopen the decoder instead of serving stale frames.
#[test]
fn changing_an_asset_path_reopens_the_decoder() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let mut e = engine(&project, dir.path());
    let before = e.frame(0).expect("before").rgba;

    // A different source at another resolution produces different composed pixels.
    let other = dir.path().join("other.mkv");
    write_counter_video(&other, 30, FrameRate::Fps30.rate(), 160, 90, false).expect("fixture");
    let mut edited = project.clone();
    let asset_id = AssetId::parse("as_a").expect("asset id");
    let mut asset = edited.assets.get(&asset_id).expect("asset").clone();
    asset.path = other.to_str().expect("utf8").to_string();
    edited.assets.insert(asset_id, asset);
    e.set_project(edited);

    let after = e.frame(0).expect("after relink").rgba;
    assert_ne!(
        before, after,
        "the new source must be decoded, not the old one"
    );
    assert_eq!(e.stats().decodes, 2);
}
/// P3-4: a file repaired in place (same path) must be picked up, not served from the stale
/// "unavailable" verdict. No project edit is needed: the file fingerprint changed.
#[test]
fn repairing_the_file_at_the_same_path_is_picked_up() {
    let _serial = serial();
    let dir = tempfile::tempdir().expect("tmp");
    let project = project_with_clip(dir.path());
    let media = dir.path().join("counter.mkv");
    std::fs::remove_file(&media).expect("remove media");

    let mut e = engine(&project, dir.path());
    let missing = e.frame(0).expect("frame with a missing source");
    assert!(
        missing.warnings.iter().any(|w| w.contains("unavailable")),
        "{:?}",
        missing.warnings
    );
    assert!(
        missing
            .rgba
            .iter()
            .enumerate()
            .all(|(i, b)| i % 4 == 3 || *b == 0)
    );

    // The same path gets a valid file again (different size and mtime).
    write_counter_video(&media, 20, FrameRate::Fps30.rate(), 320, 180, false).expect("repaired");
    let repaired = e.frame(0).expect("frame after repair");
    assert!(repaired.warnings.is_empty(), "{:?}", repaired.warnings);
    assert!(
        repaired
            .rgba
            .iter()
            .enumerate()
            .any(|(i, b)| i % 4 != 3 && *b != 0),
        "the repaired source must be decoded"
    );
}
