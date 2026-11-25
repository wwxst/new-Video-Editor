//! C2 scheduler (docs/specs/phase-c-plan.md C2): latest-wins coalescing, settle upgrade, cancel,
//! backpressure and shutdown — all with a stub source, so no GPU is involved.

use arc_cli::preview::{PreviewError, PreviewFrame, PreviewQuality};
use arc_cli::preview_scheduler::{Clock, PreviewPolicy, PreviewScheduler, PreviewSource};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Manually advanced clock for the scrub-window policy.
struct ManualClock {
    now: Mutex<Instant>,
}

impl ManualClock {
    fn new() -> Arc<ManualClock> {
        Arc::new(ManualClock {
            now: Mutex::new(Instant::now()),
        })
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("clock")
    }
}

#[derive(Default)]
struct StubState {
    renders: Vec<(i64, PreviewQuality)>,
    gate_open: bool,
}

type Gate = Arc<(Mutex<StubState>, Condvar)>;

/// Records every render request and can hold the first one, simulating a slow frame.
struct Stub {
    state: Gate,
    block_first: bool,
}

impl Stub {
    fn new(block_first: bool) -> (Stub, Gate) {
        let state: Gate = Arc::new((Mutex::new(StubState::default()), Condvar::new()));
        (
            Stub {
                state: state.clone(),
                block_first,
            },
            state,
        )
    }

    fn release(state: &Gate) {
        let (lock, cv) = &**state;
        lock.lock().expect("stub").gate_open = true;
        cv.notify_all();
    }

    fn renders(state: &Gate) -> Vec<(i64, PreviewQuality)> {
        state.0.lock().expect("stub").renders.clone()
    }
}

impl PreviewSource for Stub {
    fn frame(&mut self, frame: i64, quality: PreviewQuality) -> Result<PreviewFrame, PreviewError> {
        let (lock, cv) = &*self.state;
        {
            let mut st = lock.lock().expect("stub");
            st.renders.push((frame, quality));
            let first = st.renders.len() == 1;
            if self.block_first && first {
                while !st.gate_open {
                    st = cv.wait(st).expect("stub");
                }
            }
        }
        Ok(PreviewFrame {
            frame,
            width: 2,
            height: 2,
            rgba: vec![0; 2 * 2 * 4],
            warnings: Vec::new(),
        })
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for a condition");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A burst of playhead positions renders the in-flight frame and the newest one only; the
/// superseded results are dropped, and the delivered frame is the last requested.
#[test]
fn rapid_requests_render_only_the_latest() {
    let policy = PreviewPolicy::default();
    let clock = ManualClock::new();
    let (stub, gate) = Stub::new(true);
    let mut scheduler =
        PreviewScheduler::with_clock(stub, policy, clock.clone() as Arc<dyn Clock>);

    let first = scheduler.request(1);
    wait_until(|| Stub::renders(&gate).len() == 1);
    for frame in 2..=10 {
        scheduler.request(frame);
    }
    Stub::release(&gate);

    let result = scheduler.next_result(Duration::from_secs(3)).expect("latest result");
    assert_eq!(result.frame, 10, "the newest request wins");
    assert_eq!(result.quality, PreviewQuality::Half, "rapid requests are a scrub");
    assert!(result.generation > first);

    let renders = Stub::renders(&gate);
    assert_eq!(
        renders.len(),
        2,
        "one in-flight frame plus the latest; the older pending frames were replaced: {renders:?}"
    );
    assert_eq!(renders[1].0, 10);
}

/// A slow render delays, but never queues, the newest request: the latest frame arrives promptly
/// once the slow one finishes, and the slow result is discarded.
#[test]
fn slow_render_does_not_block_the_newest() {
    let clock = ManualClock::new();
    let (stub, gate) = Stub::new(true);
    let mut scheduler =
        PreviewScheduler::with_clock(stub, PreviewPolicy::default(), clock as Arc<dyn Clock>);

    scheduler.request(1);
    wait_until(|| Stub::renders(&gate).len() == 1);
    scheduler.request(2);
    scheduler.request(3);

    let released = Instant::now();
    Stub::release(&gate);
    let result = scheduler
        .next_result(Duration::from_secs(2))
        .expect("latest frame later");
    assert_eq!(result.frame, 3);
    assert!(
        released.elapsed() < Duration::from_secs(1),
        "the newest frame must not wait for another queue round: {:?}",
        released.elapsed()
    );
    // Frames 1 and 2 were never delivered.
    assert!(scheduler.take_result().is_none());
}

/// When the playhead stops, the last scrubbed position is re-rendered at full quality.
#[test]
fn settling_after_scrub_renders_full_quality() {
    let policy = PreviewPolicy {
        scrub_window: Duration::from_millis(300),
        settle: Duration::from_millis(40),
    };
    let (stub, _gate) = Stub::new(false);
    let mut scheduler = PreviewScheduler::with_clock(stub, policy, Arc::new(arc_cli::preview_scheduler::SystemClock));

    scheduler.request(1);
    let first = scheduler.next_result(Duration::from_secs(2)).expect("first frame");
    assert_eq!(first.frame, 1);
    assert_eq!(first.quality, PreviewQuality::Full, "a jump from idle is full quality");

    scheduler.request(2); // within the scrub window
    let half = scheduler.next_result(Duration::from_secs(2)).expect("scrub frame");
    assert_eq!((half.frame, half.quality), (2, PreviewQuality::Half));
    assert!(!half.upgraded);

    let upgrade = scheduler.next_result(Duration::from_secs(2)).expect("settle upgrade");
    assert_eq!((upgrade.frame, upgrade.quality), (2, PreviewQuality::Full));
    assert!(upgrade.upgraded, "the settle frame is an upgrade of the same position");
}

/// Cancel drops the pending request and the in-flight result; the worker stays alive for the next
/// request, and generations keep increasing.
#[test]
fn cancel_drops_pending_and_in_flight_results() {
    let clock = ManualClock::new();
    let (stub, gate) = Stub::new(true);
    let mut scheduler =
        PreviewScheduler::with_clock(stub, PreviewPolicy::default(), clock as Arc<dyn Clock>);

    let first = scheduler.request(1);
    wait_until(|| Stub::renders(&gate).len() == 1);
    scheduler.cancel();
    Stub::release(&gate);
    assert!(
        scheduler.next_result(Duration::from_millis(400)).is_none(),
        "a cancelled result must not be delivered"
    );

    let second = scheduler.request(7);
    assert!(second > first, "cancel and request advance the generation");
    let result = scheduler.next_result(Duration::from_secs(2)).expect("new request after cancel");
    assert_eq!(result.frame, 7);
    assert_eq!(result.generation, second);
}

/// 200 rapid requests while one frame is in flight keep the queue at one slot: two renders total.
#[test]
fn backpressure_keeps_a_single_pending_request() {
    let clock = ManualClock::new();
    let (stub, gate) = Stub::new(true);
    let mut scheduler =
        PreviewScheduler::with_clock(stub, PreviewPolicy::default(), clock as Arc<dyn Clock>);

    scheduler.request(0);
    wait_until(|| Stub::renders(&gate).len() == 1);
    for frame in 1..=200 {
        scheduler.request(frame);
    }
    Stub::release(&gate);
    let result = scheduler.next_result(Duration::from_secs(3)).expect("latest of 200");
    assert_eq!(result.frame, 200);
    let renders = Stub::renders(&gate);
    assert_eq!(
        renders.len(),
        2,
        "the pending slot holds one request, not a queue: {renders:?}"
    );
}

/// Shutdown joins the worker; dropping the scheduler without an explicit shutdown does not hang.
#[test]
fn shutdown_and_drop_leave_no_running_worker() {
    let clock = ManualClock::new();
    let (stub, _gate) = Stub::new(false);
    let mut scheduler =
        PreviewScheduler::with_clock(stub, PreviewPolicy::default(), clock as Arc<dyn Clock>);
    scheduler.request(0);
    assert!(scheduler.next_result(Duration::from_secs(2)).is_some());
    scheduler.shutdown();
    assert!(scheduler.worker_done(), "shutdown joins the worker");

    let clock = ManualClock::new();
    let (stub, _gate) = Stub::new(false);
    let mut scheduler =
        PreviewScheduler::with_clock(stub, PreviewPolicy::default(), clock as Arc<dyn Clock>);
    scheduler.request(0);
    let started = Instant::now();
    drop(scheduler);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "dropping the scheduler must not hang"
    );
}
