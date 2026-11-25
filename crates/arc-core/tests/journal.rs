//! Journal write states and decoding (docs/08 §4.2–4.4, §6 "journal fault injection").

use arc_core::Command;
use arc_core::engine::{EntryKind, JournalEntry, Source};
use arc_core::journal::{self, Durability, FaultyFile, JournalWriter, TailError};

fn entry(rev: u64) -> JournalEntry {
    JournalEntry {
        revision: rev,
        kind: EntryKind::Edit,
        source: Source::Client("agent".into()),
        commands: vec![Command::ProjectRename {
            name: format!("name {rev}"),
        }],
        comment: None,
    }
}

#[test]
fn round_trip_many_records() {
    let mut bytes = Vec::new();
    for r in 1..=50 {
        bytes.extend(journal::encode(&entry(r)).expect("enc"));
    }
    let d = journal::decode(&bytes);
    assert_eq!(d.entries.len(), 50);
    assert_eq!(d.valid_len, bytes.len());
    assert!(d.tail.is_none());
    assert_eq!(d.entries[49], entry(50));
}

#[test]
fn torn_tail_at_every_offset_is_rejected_cleanly() {
    let a = journal::encode(&entry(1)).expect("a");
    let b = journal::encode(&entry(2)).expect("b");
    let mut all = a.clone();
    all.extend(&b);
    for cut in a.len() + 1..all.len() {
        let d = journal::decode(&all[..cut]);
        assert_eq!(d.entries.len(), 1, "cut {cut}");
        assert_eq!(d.valid_len, a.len());
        assert_eq!(d.tail, Some(TailError::Torn), "cut {cut}");
    }
}

#[test]
fn corruption_is_detected() {
    let rec = journal::encode(&entry(1)).expect("rec");
    let mut bad = rec.clone();
    bad[20] ^= 0xFF;
    assert_eq!(journal::decode(&bad).tail, Some(TailError::BadCrc));
    let mut bad = rec.clone();
    bad[0] ^= 0xFF;
    assert_eq!(journal::decode(&bad).tail, Some(TailError::BadMagic));
    let mut bad = rec.clone();
    bad[4..8].copy_from_slice(&3u32.to_le_bytes());
    assert_eq!(journal::decode(&bad).tail, Some(TailError::BadLength));
}

#[test]
fn partial_writes_complete() {
    let f = FaultyFile {
        max_chunk: 1,
        ..Default::default()
    };
    let mut w = JournalWriter::new(f, 0, Durability::Default);
    for r in 1..=3 {
        w.append(&journal::encode(&entry(r)).expect("enc"))
            .expect("append");
    }
    let f = w.into_inner();
    assert_eq!(journal::decode(&f.data).entries.len(), 3);
}

#[test]
fn failure_mid_record_truncates_to_record_start() {
    let first = journal::encode(&entry(1)).expect("1");
    let second = journal::encode(&entry(2)).expect("2");
    let f = FaultyFile {
        fail_after_bytes: Some(first.len() + 10),
        ..Default::default()
    };
    let mut w = JournalWriter::new(f, 0, Durability::Default);
    w.append(&first).expect("first ok");
    assert!(w.append(&second).is_err());
    assert!(w.is_failed());
    assert!(
        w.append(&first).is_err(),
        "failed writer refuses further appends"
    );
    let f = w.into_inner();
    assert_eq!(f.data.len(), first.len(), "partial bytes removed");
    assert_eq!(journal::decode(&f.data).entries.len(), 1);
}

#[test]
fn failed_truncate_leaves_a_tail_that_decode_rejects() {
    let first = journal::encode(&entry(1)).expect("1");
    let second = journal::encode(&entry(2)).expect("2");
    let f = FaultyFile {
        fail_after_bytes: Some(first.len() + 10),
        fail_truncate: true,
        ..Default::default()
    };
    let mut w = JournalWriter::new(f, 0, Durability::Default);
    w.append(&first).expect("first");
    assert!(w.append(&second).is_err());
    let f = w.into_inner();
    let d = journal::decode(&f.data);
    assert_eq!(d.entries.len(), 1);
    assert_eq!(d.tail, Some(TailError::Torn));
}

#[test]
fn durable_mode_syncs_every_record_and_fails_on_sync_error() {
    let mut w = JournalWriter::new(FaultyFile::default(), 0, Durability::Durable);
    w.append(&journal::encode(&entry(1)).expect("e"))
        .expect("ok");
    w.append(&journal::encode(&entry(2)).expect("e"))
        .expect("ok");
    assert_eq!(w.into_inner().syncs, 2);

    let f = FaultyFile {
        fail_sync: true,
        ..Default::default()
    };
    let mut w = JournalWriter::new(f, 0, Durability::Durable);
    assert!(w.append(&journal::encode(&entry(1)).expect("e")).is_err());
    assert!(
        w.into_inner().data.is_empty(),
        "unsynced record removed in durable mode"
    );
}

#[test]
fn default_mode_group_flush_failure_marks_failed() {
    let f = FaultyFile {
        fail_sync: true,
        ..Default::default()
    };
    let mut w = JournalWriter::new(f, 0, Durability::Default);
    w.append(&journal::encode(&entry(1)).expect("e"))
        .expect("write completes without sync");
    assert!(w.flush().is_err());
    assert!(w.is_failed());
}

#[test]
fn crc32c_known_vector() {
    // RFC 3720 test vector: 32 bytes of zeros
    assert_eq!(journal::crc32c(&[0u8; 32]), 0x8A91_36AA);
    assert_eq!(journal::crc32c(b"123456789"), 0xE306_9283);
}
