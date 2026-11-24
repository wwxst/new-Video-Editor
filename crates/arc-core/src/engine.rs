//! The single-writer engine (docs/01 §5, §7; docs/13 §5).
//!
//! `Engine` is a synchronous state machine: validate → apply → publish. Threading, journaling,
//! and acknowledgement live in [`crate::runtime`]; this type is what both use and what tests drive.

use crate::command::{Command, apply_command, inverse_of, validate_project};
use crate::effects::EffectCatalog;
use crate::error::{CmdResult, CommandError, ErrorKind};
use crate::ids::{ClipId, TrackId};
use crate::model::Project;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;

/// Who submitted a batch. `System` is reserved for journal replay and engine-generated inverses.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "id")]
pub enum Source {
    Human,
    Client(String),
    System,
}

/// Who may submit edits (docs/13 §5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "client")]
pub enum Control {
    #[default]
    Shared,
    Human,
    Agent(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Edit,
    Undo,
    Redo,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyRequest {
    /// Optimistic concurrency. `None` means "current" (GUI and single-client tools).
    pub base_revision: Option<u64>,
    pub commands: Vec<Command>,
    pub dry_run: bool,
    pub idempotency_key: Option<String>,
    pub source: Option<Source>,
    pub comment: Option<String>,
}

/// Compact change summary for notifications (`session.changed`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Diff {
    pub tracks: Vec<TrackId>,
    pub clips_added: Vec<ClipId>,
    pub clips_removed: Vec<ClipId>,
    pub clips_changed: Vec<ClipId>,
    pub assets_changed: bool,
    pub meta_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyResult {
    pub revision: u64,
    pub applied: usize,
    pub resulting: Vec<Value>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub diff: Diff,
    pub dry_run: bool,
}

/// What must be journaled for a committed revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub revision: u64,
    pub kind: EntryKind,
    pub source: Source,
    /// Fully resolved commands; replaying them on the previous revision reproduces this one.
    pub commands: Vec<Command>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Committed {
    pub result: ApplyResult,
    pub entry: JournalEntry,
    pub snapshot: Arc<Project>,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    forward: Vec<Command>,
    inverse: Command,
}

#[derive(Debug, Clone)]
struct State {
    current: Arc<Project>,
    revision: u64,
    undo: im::Vector<Arc<HistoryEntry>>,
    redo: im::Vector<Arc<HistoryEntry>>,
}

pub const MAX_BATCH: usize = 1000;
const IDEMPOTENCY_CACHE: usize = 256;
const MAX_UNDO: usize = 5000;

#[derive(Debug)]
pub struct Engine {
    state: State,
    /// State before each unacknowledged revision, oldest first.
    pending: VecDeque<(u64, State)>,
    confirmed_revision: u64,
    catalog: Arc<dyn EffectCatalog>,
    control: Control,
    idempotency: VecDeque<(String, ApplyResult)>,
}

impl Engine {
    /// Start a session at `revision` (0 for a new project; recovered revision otherwise).
    pub fn new(project: Project, revision: u64, catalog: Arc<dyn EffectCatalog>) -> CmdResult<Engine> {
        validate_project(&project, catalog.as_ref())?;
        Ok(Engine {
            state: State { current: Arc::new(project), revision, undo: im::Vector::new(), redo: im::Vector::new() },
            pending: VecDeque::new(),
            confirmed_revision: revision,
            catalog,
            control: Control::Shared,
            idempotency: VecDeque::new(),
        })
    }

    pub fn snapshot(&self) -> Arc<Project> {
        self.state.current.clone()
    }
    pub fn revision(&self) -> u64 {
        self.state.revision
    }
    pub fn confirmed_revision(&self) -> u64 {
        self.confirmed_revision
    }
    pub fn catalog(&self) -> &Arc<dyn EffectCatalog> {
        &self.catalog
    }
    pub fn control(&self) -> &Control {
        &self.control
    }
    pub fn can_undo(&self) -> bool {
        !self.state.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.state.redo.is_empty()
    }

    pub fn set_control(&mut self, c: Control) {
        self.control = c;
    }

    /// The human edits while an agent holds control → automatic takeover (docs/13 §5 rule 1).
    fn check_control(&mut self, source: &Source) -> CmdResult<()> {
        match (source, &self.control) {
            (Source::System, _) | (_, Control::Shared) => Ok(()),
            (Source::Human, Control::Human) => Ok(()),
            (Source::Human, Control::Agent(_)) => {
                self.control = Control::Human;
                Ok(())
            }
            (Source::Client(c), Control::Agent(holder)) if c == holder => Ok(()),
            (Source::Client(_), _) => {
                Err(CommandError::new(ErrorKind::NotInControl, "another participant holds control of this session"))
            }
        }
    }

    /// Validate and apply a batch atomically. On success the new snapshot is current but
    /// **unacknowledged** until [`Engine::confirm`] (after the journal write) — see docs/08 §4.3.
    pub fn apply(&mut self, req: ApplyRequest) -> CmdResult<Committed> {
        let source = req.source.clone().unwrap_or(Source::Human);
        if let Some(key) = &req.idempotency_key {
            if let Some((_, r)) = self.idempotency.iter().find(|(k, _)| k == key) {
                return Ok(Committed {
                    result: r.clone(),
                    entry: JournalEntry {
                        revision: r.revision,
                        kind: EntryKind::Edit,
                        source,
                        commands: Vec::new(),
                        comment: None,
                    },
                    snapshot: self.snapshot(),
                });
            }
        }
        if req.commands.is_empty() {
            return Err(CommandError::new(ErrorKind::EmptyBatch, "batch contains no commands"));
        }
        if req.commands.len() > MAX_BATCH {
            return Err(CommandError::new(ErrorKind::BatchTooLarge, format!("batch exceeds {MAX_BATCH} commands")));
        }
        if source != Source::System {
            if let Some((i, _)) = req.commands.iter().enumerate().find(|(_, c)| matches!(c, Command::InternalRestore { .. })) {
                return Err(CommandError::new(ErrorKind::ForbiddenOp, "internal.restore is engine-only").at(format!("commands[{i}]")));
            }
        }
        if let Some(base) = req.base_revision {
            if base != self.state.revision {
                return Err(CommandError::new(
                    ErrorKind::StaleRevision,
                    format!("base_revision {base} is not the current revision {}", self.state.revision),
                )
                .with_details(serde_json::json!({"current_revision": self.state.revision})));
            }
        }
        if !req.dry_run {
            self.check_control(&source)?;
        }

        let before = self.state.current.clone();
        let mut work: Project = (*before).clone();
        let mut commands = req.commands;
        let mut resulting = Vec::with_capacity(commands.len());
        for (i, cmd) in commands.iter_mut().enumerate() {
            let r = apply_command(&mut work, cmd, self.catalog.as_ref()).map_err(|e| e.at(format!("commands[{i}]")))?;
            resulting.push(r);
        }
        validate_project(&work, self.catalog.as_ref())?;

        let diff = diff(&before, &work);
        let applied = commands.len();
        if req.dry_run {
            return Ok(Committed {
                result: ApplyResult { revision: self.state.revision, applied, resulting, warnings: vec![], diff, dry_run: true },
                entry: JournalEntry { revision: self.state.revision, kind: EntryKind::Edit, source, commands, comment: req.comment },
                snapshot: before,
            });
        }

        let inverse = inverse_of(&before, &work);
        self.push_pending();
        let revision = self.state.revision + 1;
        let snapshot = Arc::new(work);
        self.state.current = snapshot.clone();
        self.state.revision = revision;
        self.state.undo.push_back(Arc::new(HistoryEntry { forward: commands.clone(), inverse }));
        if self.state.undo.len() > MAX_UNDO {
            self.state.undo.pop_front();
        }
        self.state.redo.clear();

        let result = ApplyResult { revision, applied, resulting, warnings: vec![], diff, dry_run: false };
        if let Some(key) = req.idempotency_key {
            self.idempotency.push_back((key, result.clone()));
            if self.idempotency.len() > IDEMPOTENCY_CACHE {
                self.idempotency.pop_front();
            }
        }
        Ok(Committed {
            result,
            entry: JournalEntry { revision, kind: EntryKind::Edit, source, commands, comment: req.comment },
            snapshot,
        })
    }

    pub fn undo(&mut self, source: Source) -> CmdResult<Committed> {
        self.check_control(&source)?;
        let entry = self
            .state
            .undo
            .back()
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorKind::NothingToUndo, "nothing to undo"))?;
        let mut inv = entry.inverse.clone();
        let c = self.apply_internal(&mut inv, EntryKind::Undo, source)?;
        self.state.undo.pop_back();
        self.state.redo.push_back(entry);
        Ok(c)
    }

    pub fn redo(&mut self, source: Source) -> CmdResult<Committed> {
        self.check_control(&source)?;
        let entry = self
            .state
            .redo
            .back()
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorKind::NothingToRedo, "nothing to redo"))?;
        let before = self.state.current.clone();
        let mut work: Project = (*before).clone();
        let mut cmds = entry.forward.clone();
        let mut resulting = Vec::new();
        for c in cmds.iter_mut() {
            resulting.push(apply_command(&mut work, c, self.catalog.as_ref())?);
        }
        validate_project(&work, self.catalog.as_ref())?;
        let d = diff(&before, &work);
        self.push_pending();
        let revision = self.state.revision + 1;
        let snapshot = Arc::new(work);
        self.state.current = snapshot.clone();
        self.state.revision = revision;
        self.state.redo.pop_back();
        self.state.undo.push_back(entry);
        Ok(Committed {
            result: ApplyResult { revision, applied: cmds.len(), resulting, warnings: vec![], diff: d, dry_run: false },
            entry: JournalEntry { revision, kind: EntryKind::Redo, source, commands: cmds, comment: None },
            snapshot,
        })
    }

    fn apply_internal(&mut self, cmd: &mut Command, kind: EntryKind, source: Source) -> CmdResult<Committed> {
        let before = self.state.current.clone();
        let mut work: Project = (*before).clone();
        let r = apply_command(&mut work, cmd, self.catalog.as_ref())?;
        validate_project(&work, self.catalog.as_ref())?;
        let d = diff(&before, &work);
        self.push_pending();
        let revision = self.state.revision + 1;
        let snapshot = Arc::new(work);
        self.state.current = snapshot.clone();
        self.state.revision = revision;
        Ok(Committed {
            result: ApplyResult { revision, applied: 1, resulting: vec![r], warnings: vec![], diff: d, dry_run: false },
            entry: JournalEntry { revision, kind, source, commands: vec![cmd.clone()], comment: None },
            snapshot,
        })
    }

    /// Replay a journal entry during recovery. Entries are re-validated through the normal path.
    pub fn replay(&mut self, entry: &JournalEntry) -> CmdResult<()> {
        if entry.revision != self.state.revision + 1 {
            return Err(CommandError::new(
                ErrorKind::StaleRevision,
                format!("journal revision {} does not follow {}", entry.revision, self.state.revision),
            ));
        }
        let before = self.state.current.clone();
        let mut work: Project = (*before).clone();
        let mut cmds = entry.commands.clone();
        for c in cmds.iter_mut() {
            apply_command(&mut work, c, self.catalog.as_ref())?;
        }
        validate_project(&work, self.catalog.as_ref())?;
        // Rebuild history so undo works after recovery.
        match entry.kind {
            EntryKind::Edit | EntryKind::Redo => {
                let inverse = inverse_of(&before, &work);
                self.state.undo.push_back(Arc::new(HistoryEntry { forward: cmds, inverse }));
                if entry.kind == EntryKind::Edit {
                    self.state.redo.clear();
                }
            }
            EntryKind::Undo => {
                if let Some(e) = self.state.undo.pop_back() {
                    self.state.redo.push_back(e);
                }
            }
        }
        self.state.current = Arc::new(work);
        self.state.revision = entry.revision;
        self.confirmed_revision = entry.revision;
        Ok(())
    }

    fn push_pending(&mut self) {
        self.pending.push_back((self.state.revision + 1, self.state.clone()));
    }

    /// The journal record for `revision` was completely written: acknowledge it and all before it.
    pub fn confirm(&mut self, revision: u64) {
        while self.pending.front().is_some_and(|(r, _)| *r <= revision) {
            self.pending.pop_front();
        }
        self.confirmed_revision = self.confirmed_revision.max(revision);
    }

    /// Journal write failed: roll back every unacknowledged revision (docs/08 §4.3).
    /// Returns the rolled-back revisions, oldest first.
    pub fn rollback_unconfirmed(&mut self) -> Vec<u64> {
        let revs: Vec<u64> = self.pending.iter().map(|(r, _)| *r).collect();
        if let Some((_, state)) = self.pending.pop_front() {
            self.state = state;
        }
        self.pending.clear();
        // drop idempotency results for rolled-back revisions
        self.idempotency.retain(|(_, r)| !revs.contains(&r.revision));
        revs
    }
}

/// Compact structural diff between two snapshots.
pub fn diff(before: &Project, after: &Project) -> Diff {
    let mut d = Diff {
        assets_changed: before.assets != after.assets,
        meta_changed: before.meta != after.meta,
        ..Diff::default()
    };
    let (Some(b), Some(a)) = (before.sequence(), after.sequence()) else {
        return d;
    };
    let mut bclips = std::collections::HashMap::new();
    for t in b.tracks.iter() {
        for c in t.clips.iter() {
            bclips.insert(c.id.clone(), (t.id.clone(), c));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for t in a.tracks.iter() {
        let bt = b.tracks.iter().find(|x| x.id == t.id);
        if bt != Some(t) {
            d.tracks.push(t.id.clone());
        }
        for c in t.clips.iter() {
            seen.insert(c.id.clone());
            match bclips.get(&c.id) {
                None => d.clips_added.push(c.id.clone()),
                Some((btid, bc)) => {
                    if *bc != c || *btid != t.id {
                        d.clips_changed.push(c.id.clone());
                    }
                }
            }
        }
    }
    for t in b.tracks.iter() {
        if !a.tracks.iter().any(|x| x.id == t.id) {
            d.tracks.push(t.id.clone());
        }
    }
    for id in bclips.keys() {
        if !seen.contains(id) {
            d.clips_removed.push(id.clone());
        }
    }
    d.clips_removed.sort();
    d
}
