//! Timeline mock model: track types, clips and the rules that derive a track's type from its
//! content. No engine types are involved; real integration replaces this module's data source.

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
            TrackKind::Video => Color32::from_rgb(0x1f, 0x6f, 0x6a),
            TrackKind::Image => Color32::from_rgb(0x3f, 0x58, 0x78),
            TrackKind::Audio => Color32::from_rgb(0x24, 0x50, 0x7a),
        }
    }
}

/// One clip on a lane.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub kind: TrackKind,
    pub name: String,
    /// Start on the timeline, in seconds.
    pub start: f64,
    /// Duration, in seconds.
    pub duration: f64,
    /// Mock media file name for video/image/audio clips.
    pub file: Option<String>,
}

impl Clip {
    pub fn new(kind: TrackKind, name: &str, start: f64, duration: f64) -> Clip {
        Clip {
            kind,
            name: name.to_string(),
            start,
            duration,
            file: None,
        }
    }

    pub fn with_file(mut self, file: &str) -> Clip {
        self.file = Some(file.to_string());
        self
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
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
}

impl TimelineModel {
    /// Mock project used by the app until the engine feed is wired.
    pub fn mock() -> TimelineModel {
        if std::env::var_os("ARCLINE_TIMELINE_EMPTY").is_some() {
            return TimelineModel::empty();
        }
        let mut tracks = Vec::new();

        let mut text = Track::new("Text");
        text.add_clip(Clip::new(TrackKind::Text, "Opening title", 0.0, 2.8));
        text.add_clip(Clip::new(TrackKind::Text, "Lower third", 4.5, 2.5));
        tracks.push(text);

        let mut effect = Track::new("Effect");
        effect.add_clip(Clip::new(TrackKind::Effect, "Color correct", 1.0, 4.0));
        tracks.push(effect);

        let mut sticker = Track::new("Sticker");
        sticker.add_clip(Clip::new(TrackKind::Sticker, "Badge", 3.0, 2.0));
        tracks.push(sticker);

        let mut image = Track::new("Image");
        image.add_clip(Clip::new(TrackKind::Image, "logo.png", 6.0, 3.0).with_file("logo.png"));
        tracks.push(image);

        let mut supporting = Track::new("Video (b-roll)");
        supporting.add_clip(
            Clip::new(TrackKind::Video, "b-roll.mp4", 0.5, 5.5).with_file("b-roll.mp4"),
        );
        tracks.push(supporting);

        let mut main = Track::new("Main video");
        main.add_clip(
            Clip::new(TrackKind::Video, "interview.mp4", 0.0, 9.5).with_file("interview.mp4"),
        );
        tracks.push(main);

        let mut music = Track::new("Music");
        music.add_clip(Clip::new(TrackKind::Audio, "music.wav", 0.0, 12.0).with_file("music.wav"));
        tracks.push(music);

        let mut voice = Track::new("Voice");
        voice.add_clip(Clip::new(TrackKind::Audio, "voice.wav", 3.0, 6.0).with_file("voice.wav"));
        tracks.push(voice);

        TimelineModel {
            tracks,
            playhead: 2.0,
            duration: 20.0,
        }
    }

    /// No clips at all: the timeline keeps its chrome and shows the drop prompt.
    pub fn empty() -> TimelineModel {
        TimelineModel {
            tracks: Vec::new(),
            playhead: 0.0,
            duration: 0.0,
        }
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
}
