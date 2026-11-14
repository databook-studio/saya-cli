//! Unit tests for the direct-mode generation digest: it binds the snapshot,
//! the mode, and W's source content. There is no job-level write-path ceiling,
//! so the digest folds no merge record.

use super::generation_digest;
use crate::commands::coding::create::digest;
use saya_types::{Sha256Digest, SnapshotId};

fn sha(text: &str) -> Sha256Digest {
    digest(text.as_bytes()).unwrap()
}

fn snapshot() -> SnapshotId {
    SnapshotId::parse("snap-1").unwrap()
}

fn direct(source: &str) -> String {
    generation_digest(&snapshot(), &sha("snapshot"), &sha(source))
        .unwrap()
        .as_str()
        .to_owned()
}

#[test]
fn direct_digest_repeats_for_the_same_generation() {
    assert_eq!(direct("one"), direct("one"));
}

#[test]
fn direct_digest_moves_with_the_working_source() {
    assert_ne!(direct("one"), direct("two"));
}

#[test]
fn direct_digest_moves_with_the_snapshot() {
    let one = generation_digest(&snapshot(), &sha("a"), &sha("source")).unwrap();
    let two = generation_digest(&snapshot(), &sha("b"), &sha("source")).unwrap();
    assert_ne!(one, two);
}

#[test]
fn direct_digest_differs_from_the_child_sourced_preimage() {
    // The child-sourced digest (prepare.rs) hashes `{snapshot_id,
    // snapshot_sha256, sources}` with no mode: the mode field keeps a
    // root-sourced candidate id from ever colliding with a child-sourced one.
    let child_sourced = serde_json::to_vec(&serde_json::json!({
        "snapshot_id": snapshot(),
        "snapshot_sha256": sha("snapshot"),
        "sources": [],
    }))
    .unwrap();
    let child = digest(&child_sourced).unwrap();
    assert_ne!(child.as_str(), direct("source"));
    assert_ne!(child.as_str(), direct(""));
}
