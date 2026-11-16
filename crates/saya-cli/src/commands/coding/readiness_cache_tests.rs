//! The advisory readiness cache's invalidation contract: a key hits only while
//! every term that can move the facts is unchanged, and the children's states
//! are read from the store's own records.

use super::{
    ReadinessCache, ReadinessFacts, ReadinessKey, children_signature, receipt_digest, still_current,
};
use saya_store::CodingChildRecord;
use saya_types::{AgentId, CodingResultOutcome, InvocationId, JobId, JobState};
use std::sync::Arc;

fn key(epoch: u64, receipt: u64, children: u64) -> ReadinessKey {
    ReadinessKey {
        epoch,
        receipt,
        children,
    }
}

fn facts() -> Arc<ReadinessFacts> {
    Arc::new(ReadinessFacts {
        w_changed: false,
        generation: None,
        receipt_ready: false,
        changed_paths: Vec::new(),
        changed_paths_truncated: false,
        child_failures: Vec::new(),
        history_truncated: false,
        checkout: None,
    })
}

fn child(agent: &str, state: JobState) -> CodingChildRecord {
    CodingChildRecord {
        job_id: JobId::parse("readiness-cache").unwrap(),
        agent_id: AgentId::parse(agent).unwrap(),
        current_invocation_id: InvocationId::parse("inv-1").unwrap(),
        state,
        result_source_sha256: None,
        result_outcome: None,
        created_unix_ms: 0,
        updated_unix_ms: 0,
    }
}

#[test]
fn an_unchanged_key_still_describes_the_working_checkout() {
    let cached = key(3, 9, 7);
    assert!(still_current(&cached, 3, 9, 7));
}

#[test]
fn every_term_of_the_key_can_invalidate_the_facts() {
    let cached = key(3, 9, 7);
    assert!(
        !still_current(&cached, 4, 9, 7),
        "a root write ATTEMPT moves the epoch"
    );
    assert!(
        !still_current(&cached, 3, 11, 7),
        "the cached generation's receipt is no longer the one on disk"
    );
    assert!(
        !still_current(&cached, 3, 0, 7),
        "a receipt that is gone leaves the key's term absent"
    );
    assert!(
        !still_current(&cached, 3, 9, 8),
        "a child state change moves the children signature"
    );
    assert!(
        still_current(&cached, 3, 9, 7),
        "and only those terms decide"
    );
}

#[test]
fn a_child_state_change_moves_the_children_signature() {
    let running = children_signature(&[child("child-a", JobState::Running)]);
    assert_eq!(
        children_signature(&[child("child-a", JobState::Running)]),
        running,
        "the same children sign the same"
    );
    assert_ne!(
        children_signature(&[child("child-a", JobState::Failed)]),
        running,
        "a child that fails under delegate without an await moves it"
    );
    assert_ne!(
        children_signature(&[
            child("child-a", JobState::Running),
            child("child-b", JobState::Running)
        ]),
        running,
        "a second child moves it"
    );
    assert_ne!(
        children_signature(&[child("child-b", JobState::Running)]),
        running,
        "and so does a different child"
    );
    let recorded = CodingChildRecord {
        result_outcome: Some(CodingResultOutcome::Failed),
        ..child("child-a", JobState::Running)
    };
    assert_ne!(
        children_signature(&[recorded]),
        running,
        "a recorded outcome moves it too"
    );
}

#[test]
fn the_cache_hands_back_what_it_stored() {
    let cache = ReadinessCache::default();
    assert_eq!(cache.computations(), 0);
    assert!(cache.peek().is_none());
    cache.note_computation();
    cache.store(key(1, 0, 0), facts());
    let (stored, cached) = cache.peek().expect("the entry is cached");
    assert_eq!(cache.computations(), 1);
    assert!(still_current(&stored, 1, 0, 0));
    assert!(!cached.w_changed);
    assert!(cached.generation.is_none());
    assert!(cached.changed_paths.is_empty());
    assert!(cached.child_failures.is_empty());
}

#[test]
fn the_receipt_term_fingerprints_the_receipts_own_bytes() {
    let root = std::env::temp_dir().join(format!(
        "saya-readiness-receipt-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("prepared-candidate-abc.json");
    assert_eq!(receipt_digest(&path), 0, "no receipt is an absent term");
    std::fs::write(&path, b"{\"candidate_ready\":true}").unwrap();
    let ready = receipt_digest(&path);
    assert_ne!(ready, 0, "a receipt on disk is a present term");
    assert_eq!(
        receipt_digest(&path),
        ready,
        "the same bytes fingerprint the same"
    );
    std::fs::write(&path, b"{\"candidate_ready\":false}").unwrap();
    assert_ne!(
        receipt_digest(&path),
        ready,
        "a receipt rewritten in place moves the term, even at the same path"
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        receipt_digest(&path),
        0,
        "a removed receipt is absent again"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
