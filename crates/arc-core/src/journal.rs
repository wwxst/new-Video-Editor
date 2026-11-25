//! Append-only command journal (docs/08 §4.2–4.4).
//!
//! Record: `magic u32 | length u32 | revision u64 | payload (CBOR JournalEntry) | crc32c u32`,
//! little-endian; `length` covers the whole record; the CRC covers every preceding byte.
//!
//! Write states:
//! - **complete**: every byte accepted by the OS (and `sync` succeeded in durable mode) → committed;
//! - **partial**: a short write — keep writing the remainder;
//! - **failure**: an I/O error — truncate back to the record start and report.

use crate::engine::JournalEntry;
use std::io;

pub const MAGIC: u32 = 0x4A43_5241; // "ARCJ"
const HEADER: usize = 4 + 4 + 8;
const TRAILER: usize = 4;
/// Upper bound on a single record (guards against corrupt length fields).
pub const MAX_RECORD: usize = 64 * 1024 * 1024;

// ---- CRC32C (Castagnoli), table-driven ---------------------------------------------------------

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0x82F6_3B78 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}
static CRC_TABLE: [u32; 256] = crc_table();

pub fn crc32c(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("journal write failed: {0}")]
    Write(io::Error),
    #[error("journal sync failed: {0}")]
    Sync(io::Error),
    #[error("journal encode failed: {0}")]
    Encode(String),
    #[error("record exceeds the maximum size")]
    TooLarge,
}

/// Encode one entry as a complete record.
pub fn encode(entry: &JournalEntry) -> Result<Vec<u8>, JournalError> {
    let mut payload = Vec::new();
    ciborium::ser::into_writer(entry, &mut payload)
        .map_err(|e| JournalError::Encode(e.to_string()))?;
    let len = HEADER + payload.len() + TRAILER;
    if len > MAX_RECORD {
        return Err(JournalError::TooLarge);
    }
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&entry.revision.to_le_bytes());
    out.extend_from_slice(&payload);
    let crc = crc32c(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(out)
}

/// Why a journal tail was not trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailError {
    /// Record runs past end of file (process died during a write).
    Torn,
    BadMagic,
    BadLength,
    BadCrc,
    BadPayload(String),
}

#[derive(Debug)]
pub struct Decoded {
    pub entries: Vec<JournalEntry>,
    /// Byte length of the valid prefix.
    pub valid_len: usize,
    /// Set when bytes after `valid_len` were rejected.
    pub tail: Option<TailError>,
}

/// Decode records until the first invalid one. Nothing after it is trusted.
pub fn decode(bytes: &[u8]) -> Decoded {
    let mut entries = Vec::new();
    let mut off = 0usize;
    let u32_at =
        |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    while off < bytes.len() {
        if bytes.len() - off < HEADER + TRAILER {
            return Decoded {
                entries,
                valid_len: off,
                tail: Some(TailError::Torn),
            };
        }
        if u32_at(off) != MAGIC {
            return Decoded {
                entries,
                valid_len: off,
                tail: Some(TailError::BadMagic),
            };
        }
        let len = u32_at(off + 4) as usize;
        if !(HEADER + TRAILER..=MAX_RECORD).contains(&len) {
            return Decoded {
                entries,
                valid_len: off,
                tail: Some(TailError::BadLength),
            };
        }
        if off + len > bytes.len() {
            return Decoded {
                entries,
                valid_len: off,
                tail: Some(TailError::Torn),
            };
        }
        let rec = &bytes[off..off + len];
        let crc = u32::from_le_bytes([rec[len - 4], rec[len - 3], rec[len - 2], rec[len - 1]]);
        if crc32c(&rec[..len - 4]) != crc {
            return Decoded {
                entries,
                valid_len: off,
                tail: Some(TailError::BadCrc),
            };
        }
        let payload = &rec[HEADER..len - TRAILER];
        match ciborium::de::from_reader::<JournalEntry, _>(payload) {
            Ok(e) => entries.push(e),
            Err(e) => {
                return Decoded {
                    entries,
                    valid_len: off,
                    tail: Some(TailError::BadPayload(e.to_string())),
                };
            }
        }
        off += len;
    }
    Decoded {
        entries,
        valid_len: off,
        tail: None,
    }
}

/// The file operations the writer needs; implemented for `std::fs::File` and fault injectors.
pub trait JournalFile: Send {
    /// May write fewer bytes than given (a partial write).
    fn write_some(&mut self, buf: &[u8]) -> io::Result<usize>;
    fn sync(&mut self) -> io::Result<()>;
    fn truncate_to(&mut self, len: u64) -> io::Result<()>;
}

impl JournalFile for std::fs::File {
    fn write_some(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(self, buf)
    }
    fn sync(&mut self) -> io::Result<()> {
        self.sync_data()
    }
    fn truncate_to(&mut self, len: u64) -> io::Result<()> {
        self.set_len(len)?;
        io::Seek::seek(self, io::SeekFrom::Start(len))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Durability {
    /// Group `fsync` at most 1 s after the oldest unflushed record (caller drives [`JournalWriter::flush`]).
    #[default]
    Default,
    /// `fsync` before every acknowledgement.
    Durable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Bytes are in the OS; with `Durable`, also flushed to stable storage.
    Complete,
}

/// Appends records; enforces complete-write semantics.
pub struct JournalWriter<F: JournalFile> {
    file: F,
    end: u64,
    durability: Durability,
    unflushed: bool,
    failed: bool,
}

impl<F: JournalFile> std::fmt::Debug for JournalWriter<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JournalWriter")
            .field("end", &self.end)
            .field("failed", &self.failed)
            .finish()
    }
}

impl<F: JournalFile> JournalWriter<F> {
    /// `end` is the current valid length of the file (after recovery truncation).
    pub fn new(file: F, end: u64, durability: Durability) -> Self {
        JournalWriter {
            file,
            end,
            durability,
            unflushed: false,
            failed: false,
        }
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }
    pub fn len(&self) -> u64 {
        self.end
    }
    pub fn is_empty(&self) -> bool {
        self.end == 0
    }

    /// Append one record. Returns only after a complete write, or an error after truncating
    /// any partial bytes. Once a write fails the writer refuses further appends.
    pub fn append(&mut self, record: &[u8]) -> Result<WriteOutcome, JournalError> {
        if self.failed {
            return Err(JournalError::Write(io::Error::other(
                "journal is in failed state",
            )));
        }
        let start = self.end;
        let mut written = 0usize;
        while written < record.len() {
            match self.file.write_some(&record[written..]) {
                Ok(0) => {
                    return Err(self.fail(
                        start,
                        io::Error::new(io::ErrorKind::WriteZero, "write returned 0 bytes"),
                    ));
                }
                Ok(n) => written += n, // partial writes simply continue
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(self.fail(start, e)),
            }
        }
        self.end = start + record.len() as u64;
        if self.durability == Durability::Durable {
            if let Err(e) = self.file.sync() {
                self.failed = true;
                let _ = self.file.truncate_to(start);
                self.end = start;
                return Err(JournalError::Sync(e));
            }
        } else {
            self.unflushed = true;
        }
        Ok(WriteOutcome::Complete)
    }

    fn fail(&mut self, start: u64, e: io::Error) -> JournalError {
        self.failed = true;
        // Best effort: if truncation fails too, recovery discards the torn tail.
        if self.file.truncate_to(start).is_ok() {
            self.end = start;
        }
        JournalError::Write(e)
    }

    /// Group flush (default mode). A failure here cannot un-acknowledge batches; the caller
    /// must enter read-only degraded mode (docs/08 §4.3 step 4).
    pub fn flush(&mut self) -> Result<(), JournalError> {
        if !self.unflushed {
            return Ok(());
        }
        match self.file.sync() {
            Ok(()) => {
                self.unflushed = false;
                Ok(())
            }
            Err(e) => {
                self.failed = true;
                Err(JournalError::Sync(e))
            }
        }
    }

    pub fn into_inner(self) -> F {
        self.file
    }
}

/// In-memory journal file with scripted faults, for tests and fault-injection suites.
#[derive(Debug, Default, Clone)]
pub struct FaultyFile {
    pub data: Vec<u8>,
    /// Maximum bytes accepted per `write_some` call (simulates short writes). 0 = unlimited.
    pub max_chunk: usize,
    /// Fail with an I/O error once this many total bytes have been written.
    pub fail_after_bytes: Option<usize>,
    pub fail_sync: bool,
    pub fail_truncate: bool,
    pub syncs: usize,
}

impl JournalFile for FaultyFile {
    fn write_some(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut n = buf.len();
        if self.max_chunk > 0 {
            n = n.min(self.max_chunk);
        }
        if let Some(limit) = self.fail_after_bytes {
            if self.data.len() >= limit {
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected: disk full",
                ));
            }
            n = n.min(limit - self.data.len());
        }
        self.data.extend_from_slice(&buf[..n]);
        Ok(n)
    }
    fn sync(&mut self) -> io::Result<()> {
        if self.fail_sync {
            return Err(io::Error::other("injected: fsync failed"));
        }
        self.syncs += 1;
        Ok(())
    }
    fn truncate_to(&mut self, len: u64) -> io::Result<()> {
        if self.fail_truncate {
            return Err(io::Error::other("injected: truncate failed"));
        }
        self.data.truncate(len as usize);
        Ok(())
    }
}
