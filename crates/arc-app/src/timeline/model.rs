//! Timeline mock model: track types, clips and the rules that derive a track's type from its
//! content. No engine types are involved; real integration replaces this module's data source.

use arc_core::time::{FrameRate, Round, Time};
use egui::Color32;

/// The six content types a track can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackKind {
    Text,
    Effect,
    Sticker,
    Video,
    Image,
    Audio,
}

impl TrackKind {
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: [TrackKind; 6] = [
        TrackKind::Text,
        TrackKind::Effect,
        TrackKind::Sticker,
        TrackKind::Video,
        TrackKind::Image,
        TrackKind::Audio,
    ];

    /// Human readable name, used in tooltips.
    pub fn label(self) -> &'static str {
        match self {
            TrackKind::Text => "Text",
            TrackKind::Effect => "Effect",
            TrackKind::Sticker => "Sticker",
            TrackKind::Video => "Video",
            TrackKind::Image => "Image",
            TrackKind::Audio => "Audio",
        }
    }

    /// Fixed lane height in logical points for this content type. It never changes: all tracks of
    /// one type share it, zoom does not scale it, and the UI offers no way to edit it.
    pub fn lane_height(self) -> f32 {
        match self {
            TrackKind::Text | TrackKind::Effect | TrackKind::Sticker => 24.0,
            TrackKind::Video | TrackKind::Image => 76.0,
            TrackKind::Audio => 50.0,
        }
    }

    /// Clip height: the lane height minus the 2 pt padding at the top and bottom.
    pub fn clip_height(self) -> f32 {
        self.lane_height() - 4.0
    }

    /// Visual tracks can be hidden; audio-only tracks cannot.
    pub fn is_visual(self) -> bool {
        self != TrackKind::Audio
    }

    /// Tracks that carry audio and therefore expose a mute toggle.
    pub fn has_audio(self) -> bool {
        matches!(self, TrackKind::Video | TrackKind::Audio)
    }

    /// Clip background colour.
    pub fn clip_color(self) -> Color32 {
        match self {
            TrackKind::Text => Color32::from_rgb(0x8a, 0x45, 0x3c),
            TrackKind::Effect => Color32::from_rgb(0x6d, 0x5a, 0x30),
            TrackKind::Sticker => Color32::from_rgb(0x5c, 0x4a, 0x6e),
            TrackKind::Video => Color32::from_rgb(0x06, 0x4f, 0x58),
            TrackKind::Image => Color32::from_rgb(0x10, 0x4a, 0x4f),
            TrackKind::Audio => Color32::from_rgb(0x10, 0x36, 0x57),
        }
    }
}

/// One clip on a lane.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub kind: TrackKind,
    /// The clip's actual content: the text body of a title, the effect name, the sticker name, or
    /// the media file name for video/image/audio clips.
    pub name: String,
    /// Start on the timeline, in seconds.
    pub start: f64,
    /// Duration, in seconds.
    pub duration: f64,
    /// Real source file for video, image and audio clips (the mock project uses `assets/demo`).
    pub source: Option<std::path::PathBuf>,
    /// Clip-level disabled state, distinct from a hidden track. A disabled clip stays visible but
    /// is dimmed and cannot be selected or edited.
    pub enabled: bool,
}

impl Clip {
    pub fn new(kind: TrackKind, name: &str, start: f64, duration: f64) -> Clip {
        Clip {
            kind,
            name: name.to_string(),
            start,
            duration,
            source: None,
            enabled: true,
        }
    }

    /// Attach the real media file this clip visualises.
    pub fn with_source(mut self, path: impl Into<std::path::PathBuf>) -> Clip {
        self.source = Some(path.into());
        self
    }

    /// Disable this clip without hiding its track. Used by the 3A visual tests and by the future
    /// inspector toggle; it never changes the clip's timing.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_enabled(mut self, enabled: bool) -> Clip {
        self.enabled = enabled;
        self
    }

    /// File name of the source, for clip labels.
    pub fn source_name(&self) -> Option<String> {
        self.source
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

/// Directory of the demo media checked into the repository, used by the mock timeline. This is
/// presentation data for the Phase 1 visualisation; imported projects will resolve real asset paths.
pub fn demo_media(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/demo")
        .join(name)
}

/// Lane height, in points, of a track that holds no clips yet. The first clip gives the track its
/// content type and with it the fixed height for that type.
pub const UNTYPED_LANE_H: f32 = 24.0;

/// A lane. Its type is not stored: it is derived from the clips it holds, so an empty track is
/// untyped and removing every clip re-opens it for another kind.
///
/// The lane height is not editable state: it is derived from the content type (see [`Track::height`]),
/// so all tracks of one type always share the same height and nothing can resize an individual lane.
#[derive(Debug, Clone)]
pub struct Track {
    pub name: String,
    pub clips: Vec<Clip>,
    pub locked: bool,
    pub visible: bool,
    pub muted: bool,
}

impl Track {
    pub fn new(name: &str) -> Track {
        Track {
            name: name.to_string(),
            clips: Vec::new(),
            locked: false,
            visible: true,
            muted: false,
        }
    }

    /// The type of the content, or `None` for an empty track.
    pub fn kind(&self) -> Option<TrackKind> {
        self.clips.first().map(|c| c.kind)
    }

    /// Fixed lane height in logical points for this track: [`TrackKind::lane_height`] once the track
    /// has content type, and [`UNTYPED_LANE_H`] while it is empty. It is a pure function of the
    /// content type, so zoom, scroll and DPI scaling cannot change it.
    pub fn height(&self) -> f32 {
        self.kind().map_or(UNTYPED_LANE_H, TrackKind::lane_height)
    }

    /// A track accepts the first content type it is given, and only that one afterwards.
    pub fn can_accept(&self, kind: TrackKind) -> bool {
        self.clips.iter().all(|c| c.kind == kind)
    }

    /// Add a clip when the content type is compatible; returns false when it must be rejected.
    pub fn add_clip(&mut self, clip: Clip) -> bool {
        if !self.can_accept(clip.kind) {
            return false;
        }
        self.clips.push(clip);
        true
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn remove_clip(&mut self, index: usize) -> Option<Clip> {
        if index >= self.clips.len() {
            return None;
        }
        Some(self.clips.remove(index))
    }
}

/// A mock timeline: enough tracks to show every content type, the ordering rule (visual content
/// above the main video, audio below) and scrolling in both directions.
#[derive(Debug, Clone)]
pub struct TimelineModel {
    pub tracks: Vec<Track>,
    pub playhead: f64,
    pub duration: f64,
    /// Exact sequence rate from the project configuration.
    pub frame_rate: FrameRate,
}

impl TimelineModel {
    /// Sample project used by the app until the engine feed is wired.
    ///
    /// The media is real, licenced footage, photographs and audio (`assets/demo/README.md`):
    /// people in a city street and a market, ocean cliffs, a passing train and seagulls behind a
    /// ferry, a landscape and a portrait photograph, piano music and a human voice reading. The
    /// text track carries Chinese titles and subtitles; all six track kinds are present, with
    /// visual tracks above the main video and audio tracks below it.
    pub fn mock() -> TimelineModel {
        if std::env::var_os("ARCLINE_TIMELINE_EMPTY").is_some() {
            return TimelineModel::empty();
        }
        let mut tracks = Vec::new();

        let mut text = Track::new("Text / subtitles");
        text.add_clip(Clip::new(TrackKind::Text, "开场 · 山海之间", 0.0, 2.5));
        text.add_clip(Clip::new(
            TrackKind::Text,
            "字幕 · 海浪拍打冰岛海岸",
            2.5,
            4.5,
        ));
        text.add_clip(Clip::new(TrackKind::Text, "字幕 · 城市街角的人流", 7.0, 3.0));
        text.add_clip(Clip::new(TrackKind::Text, "字幕 · 集市里的烟火气", 10.0, 3.0));
        text.add_clip(Clip::new(TrackKind::Text, "字幕 · 海鸥追随渡轮", 13.0, 4.0));
        text.add_clip(Clip::new(TrackKind::Text, "片尾 · 谢谢观看", 17.5, 2.5));
        tracks.push(text);

        let mut effect = Track::new("Effects");
        effect.add_clip(Clip::new(TrackKind::Effect, "调色 · 暖阳", 0.5, 5.0));
        effect.add_clip(Clip::new(TrackKind::Effect, "虚化 · 柔光", 7.5, 3.5));
        effect.add_clip(Clip::new(TrackKind::Effect, "锐化 · 细节", 13.0, 3.5));
        tracks.push(effect);

        let mut sticker = Track::new("Stickers");
        sticker.add_clip(Clip::new(TrackKind::Sticker, "海浪贴纸", 2.0, 2.5));
        sticker.add_clip(Clip::new(TrackKind::Sticker, "集市贴纸", 9.0, 3.0));
        sticker.add_clip(Clip::new(TrackKind::Sticker, "飞鸟贴纸", 14.0, 2.5));
        tracks.push(sticker);

        let mut image = Track::new("Images");
        image.add_clip(
            Clip::new(TrackKind::Image, "mountain_lake.png", 7.5, 3.5)
                .with_source(demo_media("mountain_lake.png")),
        );
        image.add_clip(
            Clip::new(TrackKind::Image, "studio_portrait.png", 14.0, 3.5)
                .with_source(demo_media("studio_portrait.png")),
        );
        tracks.push(image);

        let mut supporting = Track::new("B-roll");
        supporting.add_clip(
            Clip::new(TrackKind::Video, "ocean_cliffs.mp4", 2.0, 5.0)
                .with_source(demo_media("ocean_cliffs.mp4")),
        );
        supporting.add_clip(
            Clip::new(TrackKind::Video, "seagulls.mp4", 12.5, 4.5)
                .with_source(demo_media("seagulls.mp4")),
        );
        tracks.push(supporting);

        let mut main = Track::new("Main video");
        main.add_clip(
            Clip::new(TrackKind::Video, "city_street.mp4", 0.0, 5.0)
                .with_source(demo_media("city_street.mp4")),
        );
        main.add_clip(
            Clip::new(TrackKind::Video, "train_station.mp4", 5.0, 4.0)
                .with_source(demo_media("train_station.mp4")),
        );
        main.add_clip(
            Clip::new(TrackKind::Video, "market_crowd.mp4", 9.0, 4.0)
                .with_source(demo_media("market_crowd.mp4")),
        );
        tracks.push(main);

        let mut music = Track::new("Music");
        music.add_clip(
            Clip::new(TrackKind::Audio, "piano_theme.wav", 0.0, 20.0)
                .with_source(demo_media("piano_theme.wav")),
        );
        tracks.push(music);

        let mut narration = Track::new("Narration");
        narration.add_clip(
            Clip::new(TrackKind::Audio, "narration.wav", 1.0, 18.0)
                .with_source(demo_media("narration.wav")),
        );
        tracks.push(narration);

        TimelineModel {
            tracks,
            playhead: 2.0,
            duration: 20.0,
            frame_rate: FrameRate::Fps24,
        }
    }

    /// No clips at all: the timeline keeps its chrome and shows the drop prompt.
    pub fn empty() -> TimelineModel {
        TimelineModel {
            tracks: Vec::new(),
            playhead: 0.0,
            duration: 0.0,
            frame_rate: FrameRate::Fps24,
        }
    }

    /// Apply the real sequence rate from a loaded project. Clip times stay in seconds; every
    /// frame-index decision (ruler, playhead snapping, max zoom) reads this field.
    pub fn with_frame_rate(mut self, frame_rate: FrameRate) -> TimelineModel {
        self.frame_rate = frame_rate;
        self.playhead = snap_to_frame(self.playhead, frame_rate);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|t| t.clips.is_empty())
    }

    /// Sum of lane heights plus the gaps, in points.
    pub fn content_height(&self) -> f32 {
        let heights: f32 = self.tracks.iter().map(Track::height).sum();
        heights + LANE_GAP * self.tracks.len().saturating_sub(1) as f32
    }

    pub fn content_width(&self, px_per_sec: f32) -> f32 {
        self.duration as f32 * px_per_sec
    }
}

/// Vertical gap between adjacent lanes.
pub const LANE_GAP: f32 = 4.0;

/// Highest horizontal zoom: pixels per project frame. The zoom ceiling is
/// `frame_rate * MAX_FRAME_PX`, so 24 fps reaches 4800 px/s, 30 fps 6000 px/s and 59.94 fps
/// 11988 px/s instead of a fixed magic number.
pub const MAX_FRAME_PX: f32 = 200.0;

/// Snap a time to the nearest project frame boundary using the exact rational rate.
///
/// The frame index is computed first, then converted back through [`FrameRate::time_of_frame`];
/// 29.97/59.94 are never rounded to 30/60.
pub fn snap_to_frame(seconds: f64, frame_rate: FrameRate) -> f64 {
    let micros = (seconds.max(0.0) * 1_000_000.0).round() as i64;
    let time = Time::new(micros, 1_000_000).unwrap_or(Time::ZERO);
    let frame = frame_rate.frame_at(time, Round::Nearest).unwrap_or(0);
    frame_rate.time_of_frame(frame).to_f64_lossy()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_heights_match_the_specification() {
        assert_eq!(TrackKind::Text.lane_height(), 24.0);
        assert_eq!(TrackKind::Effect.lane_height(), 24.0);
        assert_eq!(TrackKind::Sticker.lane_height(), 24.0);
        assert_eq!(TrackKind::Video.lane_height(), 76.0);
        assert_eq!(TrackKind::Image.lane_height(), 76.0);
        assert_eq!(TrackKind::Audio.lane_height(), 50.0);
        for kind in TrackKind::ALL {
            assert_eq!(kind.clip_height(), kind.lane_height() - 4.0);
        }
    }

    #[test]
    fn lane_height_follows_the_content_type_and_is_not_editable_state() {
        let mut track = Track::new("T");
        assert_eq!(track.height(), UNTYPED_LANE_H, "an empty lane is compact");
        assert!(track.add_clip(Clip::new(TrackKind::Text, "t", 0.0, 1.0)));
        assert_eq!(track.height(), TrackKind::Text.lane_height());
        assert!(track.add_clip(Clip::new(TrackKind::Text, "t2", 1.0, 1.0)));
        assert_eq!(
            track.height(),
            TrackKind::Text.lane_height(),
            "more clips never change the fixed height"
        );
        track.remove_clip(0);
        track.remove_clip(0);
        assert_eq!(track.height(), UNTYPED_LANE_H, "empty again, compact again");
    }

    #[test]
    fn tracks_of_the_same_type_share_one_height() {
        let model = TimelineModel::mock();
        for kind in TrackKind::ALL {
            let heights: Vec<f32> = model
                .tracks
                .iter()
                .filter(|t| t.kind() == Some(kind))
                .map(Track::height)
                .collect();
            assert!(!heights.is_empty(), "{kind:?} has no mock track");
            for height in heights {
                assert_eq!(height, kind.lane_height(), "{kind:?} lane height");
            }
        }
    }

    #[test]
    fn a_track_type_comes_from_its_content() {
        let mut track = Track::new("T");
        assert_eq!(track.kind(), None);
        assert!(track.can_accept(TrackKind::Audio));
        assert!(track.add_clip(Clip::new(TrackKind::Video, "a", 0.0, 1.0)));
        assert_eq!(track.clips[0].end(), 1.0, "clip end = start + duration");
        assert_eq!(track.kind(), Some(TrackKind::Video));
        assert!(track.can_accept(TrackKind::Video));
        assert!(!track.can_accept(TrackKind::Audio));
        assert!(!track.add_clip(Clip::new(TrackKind::Audio, "b", 1.0, 1.0)));
        assert_eq!(track.clips.len(), 1, "the incompatible clip is rejected");
        track.remove_clip(0);
        assert_eq!(track.kind(), None, "removing every clip makes it untyped");
        assert!(track.can_accept(TrackKind::Audio));
    }

    #[test]
    fn mock_clips_carry_real_content_and_sources() {
        let model = TimelineModel::mock();
        let clips: Vec<&Clip> = model.tracks.iter().flat_map(|t| t.clips.iter()).collect();

        let text: Vec<&str> = clips
            .iter()
            .filter(|c| c.kind == TrackKind::Text)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            text,
            [
                "开场 · 山海之间",
                "字幕 · 海浪拍打冰岛海岸",
                "字幕 · 城市街角的人流",
                "字幕 · 集市里的烟火气",
                "字幕 · 海鸥追随渡轮",
                "片尾 · 谢谢观看",
            ]
        );
        assert!(
            text.iter().all(|name| name.chars().any(|c| c as u32 >= 0x4E00)),
            "the text track must contain Chinese characters"
        );

        let effects: Vec<&str> = clips
            .iter()
            .filter(|c| c.kind == TrackKind::Effect)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(effects, ["调色 · 暖阳", "虚化 · 柔光", "锐化 · 细节"]);

        let stickers: Vec<&str> = clips
            .iter()
            .filter(|c| c.kind == TrackKind::Sticker)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(stickers, ["海浪贴纸", "集市贴纸", "飞鸟贴纸"]);

        let demo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo");
        for clip in &clips {
            match clip.kind {
                TrackKind::Video | TrackKind::Image | TrackKind::Audio => {
                    let source = clip.source.as_ref().expect("media clips have a real source");
                    assert!(
                        source.starts_with(&demo),
                        "{} is not demo media",
                        source.display()
                    );
                    assert!(source.is_file(), "{} is missing", source.display());
                    assert_eq!(
                        clip.source_name().as_deref(),
                        source.file_name().and_then(|n| n.to_str())
                    );
                }
                _ => assert!(
                    clip.source.is_none(),
                    "text, effect and sticker clips carry no media source"
                ),
            }
        }
    }

    #[test]
    fn mock_has_real_video_photos_music_and_narration() {
        let model = TimelineModel::mock();
        let clips: Vec<&Clip> = model.tracks.iter().flat_map(|t| t.clips.iter()).collect();
        let videos: Vec<String> = clips
            .iter()
            .filter(|c| c.kind == TrackKind::Video)
            .filter_map(|c| c.source_name())
            .collect();
        assert_eq!(
            videos,
            [
                "ocean_cliffs.mp4",
                "seagulls.mp4",
                "city_street.mp4",
                "train_station.mp4",
                "market_crowd.mp4",
            ],
            "three to five real video clips across the main and supporting tracks"
        );

        let images: Vec<String> = clips
            .iter()
            .filter(|c| c.kind == TrackKind::Image)
            .filter_map(|c| c.source_name())
            .collect();
        assert_eq!(images, ["mountain_lake.png", "studio_portrait.png"]);

        let audio: Vec<String> = clips
            .iter()
            .filter(|c| c.kind == TrackKind::Audio)
            .filter_map(|c| c.source_name())
            .collect();
        assert_eq!(audio, ["piano_theme.wav", "narration.wav"]);
    }

    #[test]
    fn mock_has_all_six_types_ordered_above_and_below_the_main_video() {
        let model = TimelineModel::mock();
        let kinds: Vec<TrackKind> = model.tracks.iter().filter_map(|t| t.kind()).collect();
        for kind in TrackKind::ALL {
            assert!(kinds.contains(&kind), "{kind:?} missing");
        }
        let main = model
            .tracks
            .iter()
            .position(|t| t.name == "Main video")
            .expect("main video");
        assert!(model.tracks[..main].iter().all(|t| t.kind() != Some(TrackKind::Audio)));
        assert!(model.tracks[main + 1..]
            .iter()
            .all(|t| t.kind() == Some(TrackKind::Audio)));
        // 24 + 24 + 24 + 76 + 76 + 76 + 50 + 50 = 400, plus 7 gaps of 4.
        assert_eq!(model.content_height(), 428.0);
    }

    #[test]
    fn playhead_snapping_uses_the_real_project_rate() {
        assert_eq!(snap_to_frame(1.0, FrameRate::Fps24), 1.0);
        assert!((snap_to_frame(2.03, FrameRate::Fps24) - 49.0 / 24.0).abs() < 1e-9);
        // 29.97 is not rounded to 30: frame 30 is exactly at 30 / (30000/1001) s.
        assert!(
            (snap_to_frame(1.0, FrameRate::Fps29_97) - 30.0 * 1001.0 / 30_000.0).abs() < 1e-9
        );
        assert!(
            (snap_to_frame(1.0, FrameRate::Fps59_94) - 60.0 * 1001.0 / 60_000.0).abs() < 1e-9
        );
        assert_eq!(
            snap_to_frame(0.5, FrameRate::Fps30),
            FrameRate::Fps30.time_of_frame(15).to_f64_lossy()
        );
    }
}
