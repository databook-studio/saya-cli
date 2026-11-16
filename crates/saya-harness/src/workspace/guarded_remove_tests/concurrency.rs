//! The race contract among Saya's own serialized writers: two concurrent
//! guarded removals of one file admit exactly one and report the other.

use std::{
    sync::{Arc, Barrier},
    thread,
};

use super::Sandbox;

use crate::HarnessError;

/// Two Saya writers race one removal of the same file behind a barrier. The
/// commit gate serializes walk → verify → unlink, so exactly one is
/// admitted and the other reports `NotFound` — never both Ok, never a panic.
#[test]
fn concurrent_remove_guarded_admits_exactly_one() {
    let sandbox = Sandbox::new("rm-race");
    sandbox.ws.write("race.txt", b"shared bytes").unwrap();
    let observed = sandbox.ws.read("race.txt", 4096).unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let ws = sandbox.ws.clone();
            let barrier = Arc::clone(&barrier);
            let digest = observed.digest.clone();
            thread::spawn(move || {
                barrier.wait();
                ws.remove_guarded("race.txt", &digest)
            })
        })
        .collect();

    let (mut removed, mut absent) = (0usize, 0usize);
    for handle in handles {
        match handle.join().unwrap() {
            Ok(()) => removed += 1,
            Err(HarnessError::NotFound { .. }) => absent += 1,
            Err(other) => panic!("unexpected concurrent failure: {other:?}"),
        }
    }
    assert_eq!(removed, 1, "exactly one of the two removals must commit");
    assert_eq!(absent, 1, "the loser must report NotFound, not success");
    assert!(!sandbox.root().join("race.txt").exists());
}
