//! Engine and journal threads (docs/01 §5, docs/08 §4.3–4.6).
//!
//! - The **engine thread** owns [`Engine`]. It applies a batch, publishes the new snapshot
//!   speculatively, and hands the journal entry to the journal thread. The caller's reply is sent
//!   only after the journal thread reports a **complete write**.
//! - The **journal thread** owns [`ProjectStore`]: appends, group flush (≤ 1 s), autosave
//!   snapshots, and explicit saves, strictly in submission order.
//! - A write failure rolls back every unacknowledged batch, answers each with
//!   `journal_unavailable`, and puts the session into read-only degraded mode.

use crate::effects::EffectCatalog;
use crate::engine::{ApplyRequest, ApplyResult, Control, Diff, Engine, JournalEntry, Source};
use crate::error::{CmdResult, CommandError, ErrorKind};
use crate::journal::Durability;
use crate::model::Project;
use crate::store::{ProjectStore, RecoveryReport, StoreError};
use arc_swap::ArcSwap;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Autosave cadence (docs/08 §4.5).
pub const AUTOSAVE_EVERY_BATCHES: u64 = 30;
pub const AUTOSAVE_EVERY: Duration = Duration::from_secs(60);
/// Group-flush bound in default durability (docs/08 §4.4).
pub const FLUSH_EVERY: Duration = Duration::from_secs(1);

/// The latest published state. Readers never lock.
#[derive(Debug, Clone)]
pub struct Published {
    pub project: Arc<Project>,
    /// Latest applied revision (may be speculative).
    pub revision: u64,
    /// Latest acknowledged (completely journaled) revision.
    pub confirmed_revision: u64,
    pub control: Control,
    pub degraded: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Change notifications (docs/13 §6).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Changed {
        revision: u64,
        source: Source,
        diff: Diff,
    },
    ControlChanged {
        control: Control,
    },
    RolledBack {
        revisions: Vec<u64>,
    },
    Degraded {
        reason: String,
    },
    Saved {
        revision: u64,
    },
}

type Reply<T> = Sender<CmdResult<T>>;

enum Msg {
    Apply(ApplyRequest, Reply<ApplyResult>),
    Undo(Source, Reply<ApplyResult>),
    Redo(Source, Reply<ApplyResult>),
    Save(Reply<u64>),
    SetControl(Control),
    Subscribe(Sender<Event>),
    Written {
        revision: u64,
        ok: bool,
        error: Option<String>,
    },
    Saved {
        revision: u64,
        result: Result<(), String>,
    },
    Shutdown(Sender<()>),
}

enum Job {
    Append(JournalEntry),
    Snapshot(Arc<Project>, u64),
    Save(Arc<Project>, u64),
    Shutdown(u64),
}

#[derive(Debug)]
pub struct EngineHost {
    tx: Sender<Msg>,
    published: Arc<ArcSwap<Published>>,
    engine_thread: Option<JoinHandle<()>>,
    journal_thread: Option<JoinHandle<()>>,
    recovery: RecoveryReport,
}

fn journal_unavailable() -> CommandError {
    CommandError::new(
        ErrorKind::JournalUnavailable,
        "the project journal cannot be written; the session is read-only",
    )
}

impl EngineHost {
    /// Create a new project file and start a session on it.
    pub fn create(
        path: &Path,
        project: Project,
        catalog: Arc<dyn EffectCatalog>,
        durability: Durability,
        overwrite: bool,
    ) -> Result<EngineHost, HostError> {
        let store = ProjectStore::create(path, &project, durability, overwrite)?;
        let engine = Engine::new(project, 0, catalog)?;
        let report = RecoveryReport {
            recovered: false,
            base: "new".into(),
            base_revision: 0,
            replayed: 0,
            discarded_records: false,
            tail_error: None,
        };
        Ok(Self::start(engine, store, report))
    }

    /// Open an existing project, recovering from the sidecar journal if needed (docs/08 §4.6).
    pub fn open(
        path: &Path,
        catalog: Arc<dyn EffectCatalog>,
        durability: Durability,
    ) -> Result<EngineHost, HostError> {
        let opened = ProjectStore::open(path, durability)?;
        let mut store = opened.store;
        let mut engine = Engine::new(opened.base, opened.base_revision, catalog)?;
        let mut replayed = 0usize;
        let mut rejected: Vec<&JournalEntry> = Vec::new();
        for (i, e) in opened.entries.iter().enumerate() {
            if engine.replay(e).is_err() {
                rejected.extend(opened.entries[i..].iter());
                break;
            }
            replayed += 1;
        }
        let discarded = opened.tail.is_some() || !rejected.is_empty();
        if !rejected.is_empty() {
            // Keep the untrusted journal bytes for support, then start a clean journal at the
            // recovered revision. This must truncate even when the revision did not change:
            // appending behind the rejected record would lose the new edits at the next open
            // (audit F-01; docs/08 §4.6).
            let jpath = store.paths().journal(store.snapshot_revision());
            if let Ok(bytes) = std::fs::read(&jpath) {
                // Distinct from the torn-tail evidence that `ProjectStore::open` writes to
                // `journal-<rev>.bin.rejected`; both may occur in the same recovery.
                let _ = std::fs::write(jpath.with_extension("bin.replay-rejected"), &bytes);
            }
            let p = store
                .paths()
                .sidecar
                .join(format!("replay-rejected-{}.json", engine.revision()));
            let _ = std::fs::write(&p, serde_json::to_vec_pretty(&rejected).unwrap_or_default());
            store.restart_journal(&engine.snapshot(), engine.revision())?;
        } else if replayed > 0 || discarded {
            // start a clean journal at the recovered revision
            store.write_snapshot(&engine.snapshot(), engine.revision())?;
        }
        let report = RecoveryReport {
            recovered: opened.from_sidecar,
            base: if opened.from_sidecar {
                "sidecar".into()
            } else {
                "container".into()
            },
            base_revision: opened.base_revision,
            replayed,
            discarded_records: discarded,
            tail_error: opened.tail.map(|t| format!("{t:?}")),
        };
        Ok(Self::start(engine, store, report))
    }

    fn start(engine: Engine, store: ProjectStore, recovery: RecoveryReport) -> EngineHost {
        let published = Arc::new(ArcSwap::from_pointee(publish_of(&engine, false)));
        let (tx, rx) = mpsc::channel::<Msg>();
        let (jtx, jrx) = mpsc::channel::<Job>();
        let journal_thread = {
            let back = tx.clone();
            std::thread::Builder::new()
                .name("arc-journal".into())
                .spawn(move || journal_loop(store, jrx, back))
                .ok()
        };
        let engine_thread = {
            let published = published.clone();
            std::thread::Builder::new()
                .name("arc-engine".into())
                .spawn(move || engine_loop(engine, rx, jtx, published))
                .ok()
        };
        EngineHost {
            tx,
            published,
            engine_thread,
            journal_thread,
            recovery,
        }
    }

    pub fn recovery(&self) -> &RecoveryReport {
        &self.recovery
    }

    /// Lock-free read of the latest state.
    pub fn published(&self) -> Arc<Published> {
        self.published.load_full()
    }

    /// Submit a batch; the receiver resolves after the journal write (or immediately on error).
    pub fn apply_async(&self, req: ApplyRequest) -> Receiver<CmdResult<ApplyResult>> {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Msg::Apply(req, tx.clone())).is_err() {
            let _ = tx.send(Err(CommandError::new(
                ErrorKind::JournalUnavailable,
                "session has ended",
            )));
        }
        rx
    }

    /// Blocking form for CLIs and tests.
    pub fn apply(&self, req: ApplyRequest) -> CmdResult<ApplyResult> {
        recv(self.apply_async(req))
    }

    pub fn undo(&self, source: Source) -> CmdResult<ApplyResult> {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Msg::Undo(source, tx));
        recv(rx)
    }

    pub fn redo(&self, source: Source) -> CmdResult<ApplyResult> {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Msg::Redo(source, tx));
        recv(rx)
    }

    /// Explicit save of the latest revision; returns the saved revision.
    pub fn save(&self) -> CmdResult<u64> {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Msg::Save(tx));
        recv(rx)
    }

    pub fn set_control(&self, c: Control) {
        let _ = self.tx.send(Msg::SetControl(c));
    }

    pub fn subscribe(&self) -> Receiver<Event> {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Msg::Subscribe(tx));
        rx
    }

    /// Orderly shutdown: drains the journal and removes the sidecar if the last revision was saved.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Msg::Shutdown(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(30));
        }
        if let Some(h) = self.engine_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.journal_thread.take() {
            let _ = h.join();
        }
    }
}

impl Drop for EngineHost {
    fn drop(&mut self) {
        self.stop();
    }
}

fn recv<T>(rx: Receiver<CmdResult<T>>) -> CmdResult<T> {
    rx.recv().unwrap_or_else(|_| {
        Err(CommandError::new(
            ErrorKind::JournalUnavailable,
            "session has ended",
        ))
    })
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Command(#[from] CommandError),
}

fn publish_of(engine: &Engine, degraded: bool) -> Published {
    Published {
        project: engine.snapshot(),
        revision: engine.revision(),
        confirmed_revision: engine.confirmed_revision(),
        control: engine.control().clone(),
        degraded,
        can_undo: engine.can_undo(),
        can_redo: engine.can_redo(),
    }
}

struct EngineState {
    engine: Engine,
    jtx: Sender<Job>,
    published: Arc<ArcSwap<Published>>,
    pending: BTreeMap<u64, (Reply<ApplyResult>, ApplyResult, Source)>,
    subscribers: Vec<Sender<Event>>,
    degraded: bool,
    since_snapshot: u64,
    last_snapshot: Instant,
    pending_saves: Vec<Reply<u64>>,
}

impl EngineState {
    fn publish(&self) {
        self.published
            .store(Arc::new(publish_of(&self.engine, self.degraded)));
    }
    fn emit(&mut self, e: Event) {
        self.subscribers.retain(|s| s.send(e.clone()).is_ok());
    }

    fn commit(&mut self, r: CmdResult<crate::engine::Committed>, reply: Reply<ApplyResult>) {
        match r {
            Err(e) => {
                let _ = reply.send(Err(e));
            }
            Ok(c) if c.result.dry_run || c.entry.commands.is_empty() => {
                // dry run, or idempotent replay of an already acknowledged batch
                let _ = reply.send(Ok(c.result));
            }
            Ok(c) => {
                let rev = c.result.revision;
                let source = c.entry.source.clone();
                self.publish();
                self.pending.insert(rev, (reply, c.result, source));
                let _ = self.jtx.send(Job::Append(c.entry));
            }
        }
    }

    fn on_written(&mut self, revision: u64, ok: bool, error: Option<String>) {
        if ok {
            self.engine.confirm(revision);
            // Publish the confirmed state before replying: a client that reads `published()`
            // after its reply must see its own revision acknowledged (found by
            // `acknowledgement_and_notifications` under parallel test load).
            self.publish();
            if let Some((reply, result, source)) = self.pending.remove(&revision) {
                let diff = result.diff.clone();
                let _ = reply.send(Ok(result));
                self.emit(Event::Changed {
                    revision,
                    source,
                    diff,
                });
            }
            self.since_snapshot += 1;
            if self.pending.is_empty()
                && (self.since_snapshot >= AUTOSAVE_EVERY_BATCHES
                    || self.last_snapshot.elapsed() >= AUTOSAVE_EVERY)
            {
                let _ = self.jtx.send(Job::Snapshot(
                    self.engine.snapshot(),
                    self.engine.revision(),
                ));
                self.since_snapshot = 0;
                self.last_snapshot = Instant::now();
            }
            self.publish();
        } else {
            let revisions = self.engine.rollback_unconfirmed();
            for (_, (reply, _, _)) in std::mem::take(&mut self.pending) {
                let _ = reply.send(Err(journal_unavailable()));
            }
            self.degraded = true;
            self.publish();
            self.emit(Event::RolledBack { revisions });
            self.emit(Event::Degraded {
                reason: error.unwrap_or_else(|| "journal write failed".into()),
            });
        }
    }
}

fn engine_loop(
    engine: Engine,
    rx: Receiver<Msg>,
    jtx: Sender<Job>,
    published: Arc<ArcSwap<Published>>,
) {
    let mut st = EngineState {
        engine,
        jtx,
        published,
        pending: BTreeMap::new(),
        subscribers: Vec::new(),
        degraded: false,
        since_snapshot: 0,
        last_snapshot: Instant::now(),
        pending_saves: Vec::new(),
    };
    let mut shutdown_reply: Option<Sender<()>> = None;
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Apply(req, reply) => {
                if st.degraded && !req.dry_run {
                    let _ = reply.send(Err(journal_unavailable()));
                    continue;
                }
                let r = st.engine.apply(req);
                st.commit(r, reply);
            }
            Msg::Undo(source, reply) => {
                if st.degraded {
                    let _ = reply.send(Err(journal_unavailable()));
                    continue;
                }
                let r = st.engine.undo(source);
                st.commit(r, reply);
            }
            Msg::Redo(source, reply) => {
                if st.degraded {
                    let _ = reply.send(Err(journal_unavailable()));
                    continue;
                }
                let r = st.engine.redo(source);
                st.commit(r, reply);
            }
            Msg::Save(reply) => {
                if st.degraded {
                    let _ = reply.send(Err(journal_unavailable()));
                    continue;
                }
                st.pending_saves.push(reply);
                let _ = st
                    .jtx
                    .send(Job::Save(st.engine.snapshot(), st.engine.revision()));
            }
            Msg::Saved { revision, result } => {
                let r = result
                    .map(|_| revision)
                    .map_err(|e| CommandError::new(ErrorKind::JournalUnavailable, e));
                if r.is_ok() {
                    st.since_snapshot = 0;
                    st.last_snapshot = Instant::now();
                    st.emit(Event::Saved { revision });
                }
                if !st.pending_saves.is_empty() {
                    let reply = st.pending_saves.remove(0);
                    let _ = reply.send(r);
                }
            }
            Msg::SetControl(c) => {
                st.engine.set_control(c.clone());
                st.publish();
                st.emit(Event::ControlChanged { control: c });
            }
            Msg::Subscribe(s) => st.subscribers.push(s),
            Msg::Written {
                revision,
                ok,
                error,
            } => {
                let before = st.engine.control().clone();
                st.on_written(revision, ok, error);
                if st.engine.control() != &before {
                    let c = st.engine.control().clone();
                    st.emit(Event::ControlChanged { control: c });
                }
            }
            Msg::Shutdown(reply) => {
                let _ = st.jtx.send(Job::Shutdown(st.engine.revision()));
                shutdown_reply = Some(reply);
                // keep draining until the journal thread finishes and drops its sender
            }
        }
        // auto-takeover may have changed control during apply
        if shutdown_reply.is_some() && st.pending.is_empty() {
            break;
        }
    }
    // answer anything still waiting
    for (_, (reply, _, _)) in std::mem::take(&mut st.pending) {
        let _ = reply.send(Err(journal_unavailable()));
    }
    if let Some(r) = shutdown_reply {
        let _ = r.send(());
    }
}

fn journal_loop(mut store: ProjectStore, rx: Receiver<Job>, back: Sender<Msg>) {
    let mut last_flush = Instant::now();
    loop {
        let job = match rx.recv_timeout(FLUSH_EVERY) {
            Ok(j) => Some(j),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match job {
            None => {}
            Some(Job::Append(entry)) => {
                let revision = entry.revision;
                let (ok, error) = if store.journal_failed() {
                    (false, Some("journal previously failed".to_string()))
                } else {
                    match store.append(&entry) {
                        Ok(()) => (true, None),
                        Err(e) => (false, Some(e.to_string())),
                    }
                };
                let _ = back.send(Msg::Written {
                    revision,
                    ok,
                    error,
                });
            }
            Some(Job::Snapshot(p, rev)) => {
                if !store.journal_failed() {
                    let _ = store.flush();
                    let _ = store.write_snapshot(&p, rev);
                }
            }
            Some(Job::Save(p, rev)) => {
                let result = if store.journal_failed() {
                    Err("journal previously failed".to_string())
                } else {
                    store
                        .flush()
                        .and_then(|_| store.save(&p, rev))
                        .map_err(|e| e.to_string())
                };
                let _ = back.send(Msg::Saved {
                    revision: rev,
                    result,
                });
            }
            Some(Job::Shutdown(current)) => {
                let _ = store.flush();
                store.close_clean(current);
                return;
            }
        }
        if last_flush.elapsed() >= FLUSH_EVERY {
            if let Err(e) = store.flush() {
                let _ = back.send(Msg::Written {
                    revision: u64::MAX,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
            last_flush = Instant::now();
        }
    }
}

/// Convenience for tests and tools: a path to a fresh project file inside `dir`.
pub fn project_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.arcline"))
}
