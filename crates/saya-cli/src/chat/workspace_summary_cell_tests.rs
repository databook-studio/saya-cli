//! When the launch scan lands: at its own pace, never in front of a message.
//!
//! The scan's bounds and what it says are `workspace_summary_tests`'s subject.
//! This is the cell's: it is started and forgotten, its answer is taken once,
//! and a message sent before it lands is answered without it.

use std::thread;
use std::time::{Duration, Instant};

use super::{SUMMARY_LABEL, WorkspaceSummary, WorkspaceSummaryCell};

/// A scan that takes its time: the fixture for "the message does not wait".
const SLOW: Duration = Duration::from_millis(300);

/// The scan is gathered in the background: starting it returns at once, and a
/// message sent while it runs is answered without it.
#[test]
fn the_scan_starts_in_the_background_and_is_never_waited_for() {
    let started = Instant::now();
    let cell = WorkspaceSummaryCell::spawn(|| {
        thread::sleep(SLOW);
        WorkspaceSummary {
            body: "folder: slow".to_owned(),
            partial: false,
        }
    });
    let spawn_took = started.elapsed();
    assert!(
        spawn_took < Duration::from_millis(100),
        "spawning a scan waited for it: {spawn_took:?}"
    );
    assert!(
        cell.block_for_turn(1).is_none(),
        "a scan still running adds no block: the first message is never held"
    );
    let block = await_block(&cell);
    assert_eq!(block.label, SUMMARY_LABEL, "the block is the summary's own");
    assert!(
        block.body.contains("folder: slow"),
        "the finished scan's body rides the block: {}",
        block.body
    );
}

/// The summary is the run's launch fact: the first turn carries it once, and
/// no later turn carries it again — the request after the first pays nothing
/// for a folder the model has already been told about.
#[test]
fn only_the_first_turn_carries_the_summary() {
    let cell = WorkspaceSummaryCell::spawn(|| WorkspaceSummary {
        body: "folder: repo\nentries (top level): src/\n".to_owned(),
        partial: true,
    });
    let first = await_block(&cell);
    assert!(first.truncated, "the partial flag rides the block");
    assert!(
        first.body.contains("src/"),
        "the entries ride the block: {}",
        first.body
    );
    assert!(
        cell.block_for_turn(2).is_none(),
        "the second turn carries no summary: it was taken by the first"
    );
    assert!(
        cell.take().is_none(),
        "the summary is taken once and never re-read"
    );
}

/// The first turn's block, waited for within a load ceiling a test may hold.
fn await_block(cell: &WorkspaceSummaryCell) -> saya_agent::ContextBlock {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(block) = cell.block_for_turn(1) {
            return block;
        }
        assert!(
            Instant::now() < deadline,
            "the background scan never landed its summary"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
