//! Preview request scheduler (docs/specs/phase-c-plan.md C2).
//!
//! The viewer posts playhead positions from the UI thread; one worker thread owns the
//! [`PreviewEngine`] and composes frames. The scheduler is **latest-wins**: only the newest pending
//! request is kept (O(1) slots, so a fast producer cannot grow a queue), a result whose generation
//! was superseded is dropped instead of delivered, and while the playhead keeps moving frames are
//! composed at half quality. When the playhead settles for [`PreviewPolicy::settle`], the last
//! position is re-rendered at full quality.
//!
//! The scrub window is measured with an injectable [`Clock`], so the quality policy is testable
//! without sleeping. The settle debounce uses real time, like any UI debounce; tests use a short
//! settle and bounded waits.

use crate::preview::{PreviewEngine, PreviewError, PreviewFrame, PreviewQuality};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Source of composed frames. Implemented by [`PreviewEngine`]; tests provide a stub.
pub trait PreviewSource: Send + 'static {
    fn frame(&mut self, frame: i64, quality: PreviewQuality) -> Result<PreviewFrame, PreviewError>;
}

impl PreviewSource for PreviewEngine {
    fn frame(&mut self, frame: i64, quality: PreviewQuality) -> Result<PreviewFrame, PreviewError> {
        self.set_quality(quality);
        PreviewEngine::frame(self, frame)
    }
}

/// Time source for the scrub-window decision (tests inject a manual clock).
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// Wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Scheduling policy (docs/03 §3: half resolution while scrubbing, full quality when settled).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewPolicy {
    /// Two requests closer together than this are treated as scrubbing.
    pub scrub_window: Duration,
    /// Quiescence after the last frame before the full-quality upgrade.
    pub settle: Duration,
}

impl Default for PreviewPolicy {
    fn default() -> Self {
        PreviewPolicy {
            scrub_window: Duration::from_millis(250),
            settle: Duration::from_millis(150),
        }
    }
}

/// One delivered frame (or the error that stopped it).
#[derive(Debug, Clone)]
pub struct PreviewResult {
    pub frame: i64,
    pub quality: PreviewQuality,
    pub generation: u64,
    /// True for the settle-time full-quality re-render of the last delivered frame.
    pub upgraded: bool,
    pub result: Result<PreviewFrame, PreviewError>,
}

#[derive(Debug, Clone, Copy)]
struct Request {
    frame: i64,
    quality: PreviewQuality,
    generation: u64,
    upgraded: bool,
}

struct SharedState {
    pending: Option<Request>,
    result: Option<PreviewResult>,
    generation: u64,
    shutdown: bool,
}

struct Shared {
    state: Mutex<SharedState>,
    cv: Condvar,
    worker_done: AtomicBool,
}

impl Shared {
    fn finish(&self) {
        self.worker_done.store(true, Ordering::SeqCst);
        self.cv.notify_all();
    }
}

/// Latest-wins preview scheduler: one worker thread, bounded O(1) pending/result slots.
pub struct PreviewScheduler {
    shared: Arc<Shared>,
    policy: PreviewPolicy,
    clock: Arc<dyn Clock>,
    last_request_at: Option<Instant>,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for PreviewScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewScheduler")
            .field("policy", &self.policy)
            .field("worker_done", &self.worker_done())
            .finish()
    }
}

impl PreviewScheduler {
    /// Scheduler over a real [`PreviewEngine`] with the system clock.
    pub fn new(engine: PreviewEngine, policy: PreviewPolicy) -> PreviewScheduler {
        PreviewScheduler::with_clock(engine, policy, Arc::new(SystemClock))
    }

    /// Scheduler over any [`PreviewSource`] and clock (tests, or a future GPU pipeline).
    pub fn with_clock<S: PreviewSource>(
        source: S,
        policy: PreviewPolicy,
        clock: Arc<dyn Clock>,
    ) -> PreviewScheduler {
        let shared = Arc::new(Shared {
            state: Mutex::new(SharedState {
                pending: None,
                result: None,
                generation: 0,
                shutdown: false,
            }),
            cv: Condvar::new(),
            worker_done: AtomicBool::new(false),
        });
        let worker_shared = shared.clone();
        let worker = std::thread::spawn(move || {
            worker(source, policy, worker_shared);
        });
        PreviewScheduler {
            shared,
            policy,
            clock,
            last_request_at: None,
            worker: Some(worker),
        }
    }

    pub fn policy(&self) -> PreviewPolicy {
        self.policy
    }

    /// Post a playhead position. Non-blocking; replaces any pending request (latest wins).
    /// Returns the generation of this request.
    pub fn request(&mut self, frame: i64) -> u64 {
        let now = self.clock.now();
        let scrubbing = self
            .last_request_at
            .is_some_and(|t| now.saturating_duration_since(t) < self.policy.scrub_window);
        self.last_request_at = Some(now);
        let quality = if scrubbing {
            PreviewQuality::Half
        } else {
            PreviewQuality::Full
        };
        let mut st = self.shared.state.lock().expect("preview state");
        st.generation += 1;
        let generation = st.generation;
        st.pending = Some(Request {
            frame,
            quality,
            generation,
            upgraded: false,
        });
        self.shared.cv.notify_all();
        generation
    }

    /// Drop the pending request and any in-flight result; the worker stays alive.
    pub fn cancel(&mut self) {
        let mut st = self.shared.state.lock().expect("preview state");
        st.generation += 1;
        st.pending = None;
        st.result = None;
        self.last_request_at = None;
        self.shared.cv.notify_all();
    }

    /// Take the newest result, if the worker produced one.
    pub fn take_result(&self) -> Option<PreviewResult> {
        self.shared
            .state
            .lock()
            .expect("preview state")
            .result
            .take()
    }

    /// Wait up to `timeout` for a result; returns `None` on timeout or shutdown.
    pub fn next_result(&self, timeout: Duration) -> Option<PreviewResult> {
        let deadline = Instant::now() + timeout;
        let mut st = self.shared.state.lock().expect("preview state");
        loop {
            if let Some(result) = st.result.take() {
                return Some(result);
            }
            if st.shutdown {
                return None;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let (guard, _) = self
                .shared
                .cv
                .wait_timeout(st, remaining)
                .expect("preview state");
            st = guard;
        }
    }

    /// True once the worker thread has exited.
    pub fn worker_done(&self) -> bool {
        self.shared.worker_done.load(Ordering::SeqCst)
    }

    /// Stop the worker and wait for it to exit (bounded by one in-flight frame).
    pub fn shutdown(&mut self) {
        {
            let mut st = self.shared.state.lock().expect("preview state");
            st.shutdown = true;
            st.pending = None;
            self.shared.cv.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for PreviewScheduler {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker<S: PreviewSource>(mut source: S, policy: PreviewPolicy, shared: Arc<Shared>) {
    // The last delivered half-quality frame, candidate for the settle-time upgrade.
    let mut last_half: Option<(i64, u64)> = None;
    let mut settle_deadline: Option<Instant> = None;
    loop {
        let request = {
            let mut st = shared.state.lock().expect("preview state");
            loop {
                if st.shutdown {
                    shared.finish();
                    return;
                }
                if let Some(request) = st.pending.take() {
                    settle_deadline = None;
                    break request;
                }
                let timeout = settle_deadline
                    .map(|d| d.saturating_duration_since(Instant::now()))
                    .unwrap_or(Duration::from_secs(3600));
                let (guard, timed) = shared
                    .cv
                    .wait_timeout(st, timeout)
                    .expect("preview state");
                st = guard;
                if !timed.timed_out() {
                    continue;
                }
                if st.shutdown {
                    shared.finish();
                    return;
                }
                if st.pending.is_some() {
                    continue;
                }
                if settle_deadline.is_some_and(|d| Instant::now() >= d) {
                    settle_deadline = None;
                    if let Some((frame, generation)) = last_half.take()
                        && generation == st.generation
                    {
                        break Request {
                            frame,
                            quality: PreviewQuality::Full,
                            generation,
                            upgraded: true,
                        };
                    }
                }
            }
        };

        let composed = source.frame(request.frame, request.quality);
        let mut st = shared.state.lock().expect("preview state");
        // A newer request (or a cancel) bumps the generation; the slow result is dropped.
        if st.shutdown || request.generation != st.generation {
            continue;
        }
        match request.quality {
            PreviewQuality::Half => {
                last_half = Some((request.frame, request.generation));
                settle_deadline = Some(Instant::now() + policy.settle);
            }
            PreviewQuality::Full => {
                last_half = None;
                settle_deadline = None;
            }
        }
        st.result = Some(PreviewResult {
            frame: request.frame,
            quality: request.quality,
            generation: request.generation,
            upgraded: request.upgraded,
            result: composed,
        });
        shared.cv.notify_all();
    }
}