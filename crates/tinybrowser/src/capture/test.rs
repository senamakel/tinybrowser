//! Tests for the held-output store.
//!
//! The store is where a screenshot waits, and every bound on it is a bound on
//! memory inside somebody else's process. These fix the chunking arithmetic —
//! which is where an off-by-one turns into a corrupted image a host cannot tell
//! from a good one — and the eviction rules.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use tinybrowser_bus::OutputId;

use super::store::OutputStore;
use crate::error::Error;

fn store_with(bytes: Vec<u8>) -> (OutputStore, OutputId) {
    let mut store = OutputStore::default();
    let handle = store
        .insert(bytes, "image/png", 1280, 800)
        .expect("within the cap");
    let id = handle.id.clone();
    (store, id)
}

#[test]
fn a_stored_output_reports_its_size_and_digest() {
    let mut store = OutputStore::default();
    let handle = store
        .insert(b"hello".to_vec(), "image/png", 100, 50)
        .expect("within the cap");

    assert_eq!(handle.total_bytes, 5);
    assert_eq!(handle.media_type, "image/png");
    assert_eq!((handle.width, handle.height), (100, 50));
    // The digest is what lets a host verify what it reassembled.
    assert_eq!(
        handle.sha256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[test]
fn every_output_gets_a_distinct_identity() {
    let mut store = OutputStore::default();
    let first = store.insert(b"a".to_vec(), "image/png", 1, 1).expect("stored");
    let second = store.insert(b"a".to_vec(), "image/png", 1, 1).expect("stored");

    assert_ne!(first.id, second.id);
}

#[test]
fn a_whole_small_output_comes_back_in_one_chunk() {
    let (mut store, id) = store_with(b"hello".to_vec());
    let chunk = store.read(&id, 0, 1024).expect("reads");

    assert_eq!(BASE64.decode(&chunk.data).expect("base64"), b"hello");
    assert!(chunk.eof);
    assert_eq!(chunk.offset, 0);
}

#[test]
fn reads_reassemble_into_the_original_bytes() {
    let bytes: Vec<u8> = (0..=255u8).cycle().take(5_000).collect();
    let (mut store, id) = store_with(bytes.clone());

    let mut collected = Vec::new();
    let mut offset = 0;
    loop {
        let chunk = store.read(&id, offset, 1_000).expect("reads");
        let decoded = BASE64.decode(&chunk.data).expect("base64");
        collected.extend_from_slice(&decoded);
        offset += decoded.len() as u64;
        if chunk.eof {
            break;
        }
    }

    assert_eq!(collected, bytes);
}

#[test]
fn the_last_chunk_is_short_rather_than_padded() {
    let (mut store, id) = store_with(vec![7; 10]);
    let chunk = store.read(&id, 8, 1_000).expect("reads");

    assert_eq!(BASE64.decode(&chunk.data).expect("base64").len(), 2);
    assert!(chunk.eof);
}

#[test]
fn a_read_at_the_exact_end_is_an_empty_final_chunk() {
    // Not an error: a host that read up to `total_bytes` and asks once more is
    // behaving correctly, and failing it would make the natural loop wrong.
    let (mut store, id) = store_with(vec![7; 10]);
    let chunk = store.read(&id, 10, 1_000).expect("reads");

    assert!(BASE64.decode(&chunk.data).expect("base64").is_empty());
    assert!(chunk.eof);
}

#[test]
fn a_read_past_the_end_is_refused() {
    let (mut store, id) = store_with(vec![7; 10]);
    let error = store.read(&id, 11, 1_000).expect_err("refused");

    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[test]
fn a_zero_length_read_still_makes_progress() {
    // Clamped to at least one byte: a host that passes zero would otherwise loop
    // forever reading nothing and never reaching eof.
    let (mut store, id) = store_with(vec![7; 10]);
    let chunk = store.read(&id, 0, 0).expect("reads");

    assert_eq!(BASE64.decode(&chunk.data).expect("base64").len(), 1);
    assert!(!chunk.eof);
}

#[test]
fn an_unknown_output_is_reported_as_such() {
    let mut store = OutputStore::default();
    let error = store
        .read(&OutputId::new("nope"), 0, 10)
        .expect_err("refused");

    assert!(matches!(error, Error::NoSuchOutput { .. }), "{error}");
}

#[test]
fn releasing_removes_it() {
    let (mut store, id) = store_with(b"hello".to_vec());
    store.release(&id);

    assert_eq!(store.len(), 0);
    assert!(store.handle(&id).is_none());
    assert!(store.read(&id, 0, 10).is_err());
}

#[test]
fn releasing_something_already_gone_succeeds() {
    // A host retrying a release must not have to tell "never existed" apart from
    // "already cleaned up".
    let mut store = OutputStore::default();
    store.release(&OutputId::new("never-existed"));

    assert_eq!(store.len(), 0);
}

#[test]
fn the_store_evicts_the_oldest_rather_than_refusing_the_newest() {
    // The screenshot just taken is the one a caller is about to read.
    let mut store = OutputStore::default();
    let first = store.insert(b"first".to_vec(), "image/png", 1, 1).expect("stored");

    let mut newest = first.clone();
    for index in 0..32 {
        newest = store
            .insert(vec![index], "image/png", 1, 1)
            .expect("stored");
    }

    assert!(store.len() <= 16);
    assert!(store.handle(&first.id).is_none(), "the oldest survived");
    assert!(store.handle(&newest.id).is_some(), "the newest was evicted");
}

#[test]
fn an_output_larger_than_the_cap_is_refused_rather_than_held() {
    let mut store = OutputStore::default();
    let error = store
        .insert(vec![0; 64 * 1024 * 1024 + 1], "image/png", 1, 1)
        .expect_err("refused");

    assert!(matches!(error, Error::LimitExceeded { .. }), "{error}");
    assert_eq!(store.len(), 0);
}
