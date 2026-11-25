//! Project persistence (docs/02 §6–7, docs/08 §4).
//!
//! ```text
//! Demo.arcline                 ZIP container: manifest.json + project.json (last explicit save)
//! Demo.arcline.autosave/       sidecar
//! ├── snapshot-<rev>.json      latest snapshot
//! ├── journal-<rev>.bin        batches committed after snapshot <rev>
//! ├── *.rejected               unrecoverable journal tails, kept for support
//! └── session.lock             held while a process has the project open
//! ```

use crate::engine::JournalEntry;
use crate::journal::{self, Durability, JournalWriter, TailError};
use crate::model::{Project, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("i/o error on {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("project is open in another process ({0})")]
    Locked(PathBuf),
    #[error("a project already exists at {0}")]
    Exists(PathBuf),
    #[error("not an Arcline project: {0}")]
    BadContainer(String),
    #[error("checksum mismatch in {0}")]
    Checksum(String),
    #[error(
        "project schema {found} is newer than this build supports ({supported}); update Arcline"
    )]
    FutureSchema { found: u32, supported: u32 },
    #[error("project schema {0} is not supported")]
    UnsupportedSchema(u32),
    #[error("invalid project document: {0}")]
    Document(String),
    #[error(transparent)]
    Journal(#[from] journal::JournalError),
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub app_version: String,
    pub project_id: String,
    pub revision: u64,
    /// entry name → BLAKE3 hex
    pub checksums: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotFile {
    revision: u64,
    hash: String,
    project: Project,
}

/// Paths for one project.
#[derive(Debug, Clone)]
pub struct ProjectPaths {
    pub container: PathBuf,
    pub sidecar: PathBuf,
}

impl ProjectPaths {
    pub fn new(container: impl Into<PathBuf>) -> Self {
        let container = container.into();
        let mut name = container
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(".autosave");
        let sidecar = container.with_file_name(name);
        ProjectPaths { container, sidecar }
    }
    pub fn snapshot(&self, rev: u64) -> PathBuf {
        self.sidecar.join(format!("snapshot-{rev}.json"))
    }
    pub fn journal(&self, rev: u64) -> PathBuf {
        self.sidecar.join(format!("journal-{rev}.bin"))
    }
    fn lock(&self) -> PathBuf {
        self.sidecar.join("session.lock")
    }
}

// ---- atomic file replacement -------------------------------------------------------------------

/// Write `bytes` to `path` atomically: temp file in the same directory → fsync → rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    {
        let mut f = File::create(&tmp).map_err(io_err(&tmp))?;
        f.write_all(bytes).map_err(io_err(&tmp))?;
        f.sync_all().map_err(io_err(&tmp))?;
    }
    // std::fs::rename uses MoveFileExW(MOVEFILE_REPLACE_EXISTING) on Windows.
    fs::rename(&tmp, path).map_err(io_err(path))?;
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

// ---- container ---------------------------------------------------------------------------------

pub fn encode_container(project: &Project, revision: u64) -> Result<Vec<u8>, StoreError> {
    let doc = project.canonical_json();
    let mut checksums = std::collections::BTreeMap::new();
    checksums.insert(
        "project.json".to_string(),
        blake3::hash(&doc).to_hex().to_string(),
    );
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION,
        app_version: APP_VERSION.to_string(),
        project_id: project.meta.id.to_string(),
        revision,
        checksums,
    };
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|e| StoreError::Document(e.to_string()))?;
    let mut buf = io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let zerr = |e: zip::result::ZipError| StoreError::BadContainer(e.to_string());
        zip.start_file("manifest.json", opts).map_err(zerr)?;
        zip.write_all(&manifest_bytes)
            .map_err(|e| StoreError::BadContainer(e.to_string()))?;
        zip.start_file("project.json", opts).map_err(zerr)?;
        zip.write_all(&doc)
            .map_err(|e| StoreError::BadContainer(e.to_string()))?;
        zip.finish().map_err(zerr)?;
    }
    Ok(buf.into_inner())
}

pub fn decode_container(bytes: &[u8]) -> Result<(Project, Manifest), StoreError> {
    let bad = |e: zip::result::ZipError| StoreError::BadContainer(e.to_string());
    let mut zip = zip::ZipArchive::new(io::Cursor::new(bytes)).map_err(bad)?;
    let mut read = |name: &str| -> Result<Vec<u8>, StoreError> {
        let mut f = zip.by_name(name).map_err(bad)?;
        let mut v = Vec::new();
        f.read_to_end(&mut v)
            .map_err(|e| StoreError::BadContainer(e.to_string()))?;
        Ok(v)
    };
    let manifest: Manifest = serde_json::from_slice(&read("manifest.json")?)
        .map_err(|e| StoreError::BadContainer(e.to_string()))?;
    let doc = read("project.json")?;
    if let Some(sum) = manifest.checksums.get("project.json") {
        if blake3::hash(&doc).to_hex().as_str() != sum {
            return Err(StoreError::Checksum("project.json".into()));
        }
    }
    let project = parse_document(&doc)?;
    Ok((project, manifest))
}

/// Parse and migrate a project document (forward-only migrations, docs/02 §7).
pub fn parse_document(doc: &[u8]) -> Result<Project, StoreError> {
    let mut v: serde_json::Value =
        serde_json::from_slice(doc).map_err(|e| StoreError::Document(e.to_string()))?;
    let found = v
        .get("schema_version")
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
    if found > SCHEMA_VERSION {
        return Err(StoreError::FutureSchema {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    v = migrate(v, found)?;
    serde_json::from_value(v).map_err(|e| StoreError::Document(e.to_string()))
}

/// Migration chain. Schema 7 is the baseline; add `migrate_v7_to_v8` etc. here with a fixture.
fn migrate(v: serde_json::Value, from: u32) -> Result<serde_json::Value, StoreError> {
    match from {
        SCHEMA_VERSION => Ok(v),
        other => Err(StoreError::UnsupportedSchema(other)),
    }
}

pub fn save_container(path: &Path, project: &Project, revision: u64) -> Result<(), StoreError> {
    write_atomic(path, &encode_container(project, revision)?)
}

pub fn load_container(path: &Path) -> Result<(Project, Manifest), StoreError> {
    let bytes = fs::read(path).map_err(io_err(path))?;
    decode_container(&bytes)
}

// ---- lock ----------------------------------------------------------------------------------------

/// Held for the lifetime of an open session. On Windows the file is opened without sharing, so
/// a crashed process releases it automatically.
#[derive(Debug)]
pub struct SessionLock {
    path: PathBuf,
    _file: File,
}

impl SessionLock {
    pub fn acquire(path: &Path) -> Result<SessionLock, StoreError> {
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            opts.share_mode(0);
        }
        #[cfg(not(windows))]
        if let Ok(s) = fs::read_to_string(path) {
            if let Some(pid) = s
                .split_whitespace()
                .next()
                .and_then(|p| p.parse::<u32>().ok())
            {
                let alive = Path::new(&format!("/proc/{pid}")).exists();
                if alive && pid != std::process::id() {
                    return Err(StoreError::Locked(path.to_path_buf()));
                }
            }
        }
        let mut f = match opts.open(path) {
            Ok(f) => f,
            Err(e)
                if e.raw_os_error() == Some(32) || e.kind() == io::ErrorKind::PermissionDenied =>
            {
                return Err(StoreError::Locked(path.to_path_buf()));
            }
            Err(e) => {
                return Err(StoreError::Io {
                    path: path.to_path_buf(),
                    source: e,
                });
            }
        };
        let _ = writeln!(f, "{} {}", std::process::id(), APP_VERSION);
        let _ = f.sync_all();
        Ok(SessionLock {
            path: path.to_path_buf(),
            _file: f,
        })
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

// ---- sidecar store -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveryReport {
    pub recovered: bool,
    pub base: String,
    pub base_revision: u64,
    pub replayed: usize,
    pub discarded_records: bool,
    pub tail_error: Option<String>,
}

/// What `ProjectStore::open` hands to the runtime for replay.
#[derive(Debug)]
pub struct Opened {
    pub store: ProjectStore,
    pub base: Project,
    pub base_revision: u64,
    pub entries: Vec<JournalEntry>,
    pub tail: Option<TailError>,
    pub from_sidecar: bool,
}

#[derive(Debug)]
pub struct ProjectStore {
    paths: ProjectPaths,
    _lock: SessionLock,
    snapshot_rev: u64,
    journal: JournalWriter<File>,
    durability: Durability,
    saved_revision: u64,
}

fn latest_snapshot(
    paths: &ProjectPaths,
    expected_id: Option<&str>,
) -> (Option<(u64, Project)>, bool) {
    let Ok(rd) = fs::read_dir(&paths.sidecar) else {
        return (None, false);
    };
    let mut revs: Vec<u64> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_prefix("snapshot-")?
                .strip_suffix(".json")?
                .parse()
                .ok()
        })
        .collect();
    revs.sort_unstable();
    let mut foreign = false;
    for rev in revs.into_iter().rev() {
        let Ok(bytes) = fs::read(paths.snapshot(rev)) else {
            continue;
        };
        let Ok(s) = serde_json::from_slice::<SnapshotFile>(&bytes) else {
            continue;
        };
        if s.revision != rev || s.project.canonical_hash() != s.hash {
            continue;
        }
        // A sidecar can outlive the container it belonged to (audit F-05). Never recover a
        // snapshot into a project whose identity differs.
        if expected_id.is_some_and(|id| s.project.meta.id.as_str() != id) {
            foreign = true;
            continue;
        }
        return (Some((rev, s.project)), foreign);
    }
    (None, foreign)
}

/// Move every sidecar file except the lock aside, so a stale autosave from another project cannot
/// be recovered by the project being created here (audit F-05). Files are never deleted.
fn move_stale_sidecar_aside(paths: &ProjectPaths) -> Result<bool, StoreError> {
    let rd = fs::read_dir(&paths.sidecar).map_err(io_err(&paths.sidecar))?;
    let mut names = Vec::new();
    for e in rd {
        let e = e.map_err(io_err(&paths.sidecar))?;
        if e.file_name() == "session.lock" {
            continue;
        }
        names.push(e.file_name());
    }
    if names.is_empty() {
        return Ok(false);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let base = paths
        .sidecar
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "autosave".into());
    let mut stale = paths.sidecar.with_file_name(format!("{base}.stale-{stamp}"));
    let mut n = 1u32;
    while stale.exists() {
        stale = paths
            .sidecar
            .with_file_name(format!("{base}.stale-{stamp}-{n}"));
        n += 1;
    }
    fs::create_dir(&stale).map_err(io_err(&stale))?;
    for name in names {
        let from = paths.sidecar.join(&name);
        let to = stale.join(&name);
        fs::rename(&from, &to).map_err(io_err(&from))?;
    }
    Ok(true)
}

impl ProjectStore {
    /// Create a new project on disk and open it.
    ///
    /// The session lock is taken **before** anything is written (audit F-04), and an existing
    /// container is refused unless `overwrite` is set, so creating a project can never destroy a
    /// saved project that another session has open.
    pub fn create(
        path: &Path,
        project: &Project,
        durability: Durability,
        overwrite: bool,
    ) -> Result<ProjectStore, StoreError> {
        let paths = ProjectPaths::new(path);
        fs::create_dir_all(&paths.sidecar).map_err(io_err(&paths.sidecar))?;
        let lock = SessionLock::acquire(&paths.lock())?;
        if !overwrite && path.exists() {
            return Err(StoreError::Exists(path.to_path_buf()));
        }
        // Any sidecar at this path belongs to a project that predates this one; keep it, but move
        // it out of the way so it can never be recovered as the new project (audit F-05).
        move_stale_sidecar_aside(&paths)?;
        save_container(path, project, 0)?;
        let mut store = ProjectStore {
            journal: JournalWriter::new(open_journal(&paths.journal(0), 0)?, 0, durability),
            paths,
            _lock: lock,
            snapshot_rev: 0,
            durability,
            saved_revision: 0,
        };
        store.write_snapshot(project, 0)?;
        Ok(store)
    }

    /// Open an existing project and gather what recovery must replay (docs/08 §4.6).
    pub fn open(path: &Path, durability: Durability) -> Result<Opened, StoreError> {
        let paths = ProjectPaths::new(path);
        let container = load_container(path);
        fs::create_dir_all(&paths.sidecar).map_err(io_err(&paths.sidecar))?;
        let lock = SessionLock::acquire(&paths.lock())?;

        let expected_id = container
            .as_ref()
            .ok()
            .map(|(p, _)| p.meta.id.as_str().to_string());
        let (snap, foreign) = latest_snapshot(&paths, expected_id.as_deref());
        if container.is_ok() && foreign {
            // The sidecar belongs to a different project (the container was replaced outside the
            // API). Keep the files for support, but never replay them (audit F-05).
            move_stale_sidecar_aside(&paths)?;
        }
        let (base, base_revision, from_sidecar, saved_revision) = match (container, snap) {
            (Ok((_p, m)), Some((rev, sp))) if rev >= m.revision => {
                (sp, rev, rev > m.revision, m.revision)
            }
            (Ok((p, m)), _) => (p, m.revision, false, m.revision),
            (Err(_), Some((rev, sp))) => (sp, rev, true, 0),
            (Err(e), None) => return Err(e),
        };

        let jpath = paths.journal(base_revision);
        let bytes = fs::read(&jpath).unwrap_or_default();
        let decoded = journal::decode(&bytes);
        if decoded.tail.is_some() {
            // keep the untrusted tail for support, then cut it off
            let rejected = jpath.with_extension("bin.rejected");
            let _ = fs::write(&rejected, &bytes[decoded.valid_len..]);
        }
        let file = open_journal(&jpath, decoded.valid_len as u64)?;
        let store = ProjectStore {
            journal: JournalWriter::new(file, decoded.valid_len as u64, durability),
            paths,
            _lock: lock,
            snapshot_rev: base_revision,
            durability,
            saved_revision,
        };
        let from_sidecar = from_sidecar || !decoded.entries.is_empty();
        Ok(Opened {
            store,
            base,
            base_revision,
            entries: decoded.entries,
            tail: decoded.tail,
            from_sidecar,
        })
    }

    pub fn paths(&self) -> &ProjectPaths {
        &self.paths
    }
    pub fn snapshot_revision(&self) -> u64 {
        self.snapshot_rev
    }
    pub fn saved_revision(&self) -> u64 {
        self.saved_revision
    }
    pub fn journal_failed(&self) -> bool {
        self.journal.is_failed()
    }

    /// Append a committed entry. Returns only after a complete write (docs/08 §4.3).
    pub fn append(&mut self, entry: &JournalEntry) -> Result<(), StoreError> {
        let rec = journal::encode(entry)?;
        self.journal.append(&rec)?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), StoreError> {
        self.journal.flush()?;
        Ok(())
    }

    /// Autosave snapshot at `rev`; starts a new journal and deletes the previous pair.
    pub fn write_snapshot(&mut self, project: &Project, rev: u64) -> Result<(), StoreError> {
        let file = SnapshotFile {
            revision: rev,
            hash: project.canonical_hash(),
            project: project.clone(),
        };
        let bytes = serde_json::to_vec(&file).map_err(|e| StoreError::Document(e.to_string()))?;
        let path = self.paths.snapshot(rev);
        write_atomic(&path, &bytes)?;
        let old = self.snapshot_rev;
        let jpath = self.paths.journal(rev);
        if old != rev || !jpath.exists() {
            let f = open_journal(&jpath, 0)?;
            self.journal = JournalWriter::new(f, 0, self.durability);
        }
        if old != rev {
            let _ = fs::remove_file(self.paths.snapshot(old));
            let _ = fs::remove_file(self.paths.journal(old));
        }
        self.snapshot_rev = rev;
        Ok(())
    }

    /// Rebuild the sidecar after a replay rejection: the snapshot is written at `rev` and the
    /// journal at `rev` is replaced by an empty one **even when `rev` is unchanged**. Otherwise
    /// new records would be appended behind the rejected record, with a duplicate revision, and the
    /// next recovery would discard them too (audit F-01).
    pub fn restart_journal(&mut self, project: &Project, rev: u64) -> Result<(), StoreError> {
        let file = SnapshotFile {
            revision: rev,
            hash: project.canonical_hash(),
            project: project.clone(),
        };
        let bytes = serde_json::to_vec(&file).map_err(|e| StoreError::Document(e.to_string()))?;
        write_atomic(&self.paths.snapshot(rev), &bytes)?;
        let jpath = self.paths.journal(rev);
        let f = open_journal(&jpath, 0)?;
        self.journal = JournalWriter::new(f, 0, self.durability);
        let old = self.snapshot_rev;
        if old != rev {
            let _ = fs::remove_file(self.paths.snapshot(old));
            let _ = fs::remove_file(self.paths.journal(old));
        }
        self.snapshot_rev = rev;
        Ok(())
    }

    /// Explicit save: atomic container replace, then rotate the sidecar to the saved revision.
    pub fn save(&mut self, project: &Project, rev: u64) -> Result<(), StoreError> {
        save_container(&self.paths.container, project, rev)?;
        self.saved_revision = rev;
        self.write_snapshot(project, rev)
    }

    /// Clean close after the latest revision was saved: remove the sidecar.
    pub fn close_clean(self, current_revision: u64) {
        if self.saved_revision == current_revision {
            let sidecar = self.paths.sidecar.clone();
            drop(self);
            let _ = fs::remove_dir_all(sidecar);
        }
    }
}

fn open_journal(path: &Path, valid_len: u64) -> Result<File, StoreError> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(io_err(path))?;
    journal::JournalFile::truncate_to(&mut f, valid_len).map_err(io_err(path))?;
    Ok(f)
}
