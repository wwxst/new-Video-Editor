//! C3 transport tests: play/pause/seek/step over the scheduler, the composited-frame cache and the
//! settle upgrade, all with a stub source (no GPU, no window).

use arc_cli::preview::{PreviewError, PreviewFrame, PreviewQuality};
use arc_cli::preview_scheduler::{Clock, PreviewPolicy, PreviewScheduler, PreviewSource, SystemClock};
use arc_cli::preview_transport::PreviewTransport;
use arc_core::time::Ratio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct StubState {
    renders: Vec<(i64, PreviewQuality)>,
}

struct Stub {
    state: Arc<Mutex<StubState>>,
}

impl PreviewSource for Stub {
    fn frame(&mut self, frame: i64, quality: PreviewQuality) -> Result<PreviewFrame, PreviewError> {
        self.state
            .lock()
            .expect("stub")
            .renders
            .push((frame, quality));
        Ok(PreviewFrame {
            frame,
            width: 2,
            height: 2,
            rgba: vec![0; 2 * 2 * 4],
            warnings: Vec::new(),
        })
    }
}

type Renders = Arc<Mutex<StubState>>;

fn transport(policy: PreviewPolicy) -> (PreviewTransport, Renders) {
    let renders: Renders = Arc::new(Mutex::new(StubState::default()));
    let stub = Stub {
        state: renders.clone(),
    };
    let scheduler = PreviewScheduler::with_clock(
        stub,
        policy,
        Arc::new(SystemClock) as Arc<dyn Clock>,
    );
    let rate = Ratio::new(30, 1).expect("rate");
    (PreviewTransport::new(scheduler, 100, rate), renders)
}

fn renders(state: &Renders) -> Vec<(i64, PreviewQuality)> {
    state.lock().expect("stub").renders.clone()
}

/// Poll the transport until a new frame is displayed or the deadline passes.
fn settle(transport: &mut PreviewTransport) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if transport.poll() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// Pause the scheduler-induced ambiguity: every request is a full-quality jump (scrub window 0).
fn no_scrub() -> PreviewPolicy {
    PreviewPolicy {
        scrub_window: Duration::ZERO,
        settle: Duration::from_millis(60),
    }
}

#[test]
fn the_first_frame_is_requested_and_displayed() {
    let (mut t, st) = transport(no_scrub());
    assert_eq!(t.playhead(), 0);
    assert!(settle(&mut t), "frame 0 must arrive");
    let frame = t.frame().expect("displayed frame");
    assert_eq!(frame.frame, 0);
    assert_eq!(renders(&st), vec![(0, PreviewQuality::Full)]);
    assert!(t.serial() > 0);
}

#[test]
fn seeking_requests_the_target_and_displays_it() {
    let (mut t, st) = transport(no_scrub());
    assert!(settle(&mut t));
    t.seek(10);
    assert_eq!(t.playhead(), 10);
    assert!(settle(&mut t));
    assert_eq!(t.frame().expect("frame").frame, 10);
    assert!(renders(&st).iter().any(|(f, _)| *f == 10));
}

#[test]
fn playing_advances_by_elapsed_time() {
    let (mut t, st) = transport(no_scrub());
    assert!(settle(&mut t));
    t.play();
    // 30 fps: 100 ms is exactly three frames.
    assert!(t.advance(Duration::from_millis(100)));
    assert_eq!(t.playhead(), 3);
    assert!(settle(&mut t));
    assert_eq!(t.frame().expect("frame").frame, 3);
    assert!(renders(&st).iter().any(|(f, _)| *f == 3));
}

#[test]
fn pause_stops_the_playhead() {
    let (mut t, _st) = transport(no_scrub());
    assert!(settle(&mut t));
    t.play();
    t.pause();
    assert!(!t.advance(Duration::from_secs(1)));
    assert_eq!(t.playhead(), 0);
    assert!(!t.is_playing());
}

#[test]
fn seeking_clamps_to_the_sequence() {
    let (mut t, _st) = transport(no_scrub());
    assert_eq!(t.seek(-5), 0);
    assert_eq!(t.seek(10_000), 99);
}

#[test]
fn playing_from_the_last_frame_restarts_at_zero() {
    let (mut t, _st) = transport(no_scrub());
    t.seek(99);
    assert!(settle(&mut t));
    t.play();
    assert_eq!(t.playhead(), 0, "play from the end restarts");
    assert!(t.is_playing());
}

/// Re-visiting a frame that was composed at full quality is served from the cache: the viewer
/// updates instantly and the pipeline is not asked again.
#[test]
fn revisiting_a_cached_frame_does_not_recompose() {
    let (mut t, st) = transport(no_scrub());
    assert!(settle(&mut t)); // frame 0
    t.seek(2);
    assert!(settle(&mut t)); // frame 2, full quality -> cached
    t.seek(3);
    assert!(settle(&mut t)); // frame 3, full quality -> cached
    let before = renders(&st).len();
    t.seek(2);
    assert_eq!(t.frame().expect("cached frame").frame, 2, "instant display");
    assert_eq!(renders(&st).len(), before, "no new render for a cached frame");
    assert_eq!(t.cache_hits(), 1);
    assert!(!t.poll(), "nothing new from the scheduler");
}

/// While the playhead keeps moving the scheduler composes half quality; once it settles the full
/// frame of the same position arrives and replaces it in the viewer.
#[test]
fn settle_upgrade_replaces_the_scrub_frame() {
    let (mut t, st) = transport(PreviewPolicy {
        scrub_window: Duration::from_millis(300),
        settle: Duration::from_millis(60),
    });
    assert!(settle(&mut t)); // frame 0 from idle -> full
    t.seek(1); // within the window -> scrub (half)
    assert!(settle(&mut t));
    assert_eq!(t.displayed_quality(), Some(PreviewQuality::Half));
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut upgraded = false;
    while Instant::now() < deadline {
        if t.poll()
            && let Some(frame) = t.frame()
            && frame.frame == 1
            && t.displayed_quality() == Some(PreviewQuality::Full)
        {
            upgraded = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(upgraded, "the settle upgrade must reach the viewer");
    assert!(
        renders(&st)
            .iter()
            .any(|(f, q)| *f == 1 && *q == PreviewQuality::Half),
        "the scrub frame was rendered at half quality"
    );
}
