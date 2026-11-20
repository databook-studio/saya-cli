//! A second root turn of an open run is as rewindable as the first, and the run
//! still finishes once (phase-3 unit 3-C).
//!
//! The run checkpoints the folder at every turn boundary, so the checkpoint the
//! second turn started from is the state the first turn left — rewinding to it
//! puts the second turn's file back and leaves the first turn's alone. The one
//! `finish` of the run's timeline still happens once, after its last turn.
#![cfg(unix)]

use super::super::super::lifecycle;
use super::super::parent_turn_fixture::{in_place, in_place_writing, pause};
use super::super::parent_turn_provider::Scripted;
use super::records::{events, read, records, terminal, turn, two_writes};
use saya_types::CodingEventV1;

#[tokio::test]
async fn rewinding_to_the_checkpoint_before_the_second_turn_restores_it() {
    let provider = Scripted::start_with_script(two_writes());
    let fixture = in_place_writing("records-rewind", &provider.endpoint).await;
    let folder = fixture.root.join("repo");

    turn(&fixture.job, None).await;
    turn(&fixture.job, Some("second message")).await;
    assert_eq!(read(&folder, "turn-two.txt").as_deref(), Some("two\n"));

    let checkpoints = fixture.job.rewind_store().list().expect("the store lists");
    assert_eq!(
        checkpoints.len(),
        2,
        "each turn that moved the checkout is checkpointed: {checkpoints:?}"
    );
    // The checkpoint the second turn started from: the first turn's state.
    let before_second = checkpoints[0].turn;
    let named: Vec<&str> = checkpoints[0]
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(
        named,
        ["turn-one.txt"],
        "the checkpoint before the second turn holds the first turn's file alone"
    );
    assert_eq!(checkpoints[1].turn, before_second + 1);

    // A rewind is refused while the durable job is running, and the run is still
    // open after its last turn; ending it admits the rewind.
    pause(&fixture.job).await;
    let report = fixture
        .job
        .application
        .rewind(fixture.job.job_id(), Some(before_second), false)
        .await
        .expect("the rewind restores the checkpoint before the second turn");
    assert_eq!(report["turn"], serde_json::json!(before_second));
    assert_eq!(
        read(&folder, "turn-one.txt").as_deref(),
        Some("one\n"),
        "the first turn's file stays as the first turn left it"
    );
    assert_eq!(
        read(&folder, "turn-two.txt"),
        None,
        "the second turn's file is back to its pre-turn-2 content: absent"
    );
    assert_eq!(
        report["removed_paths"],
        serde_json::json!(1),
        "the rewind reports the file it removed: {report}"
    );
}

#[tokio::test]
async fn the_run_finishes_once_after_its_last_turn() {
    let provider = Scripted::start();
    let fixture = in_place("records-finish", &provider.endpoint).await;

    turn(&fixture.job, None).await;
    let last = turn(&fixture.job, Some("second message")).await;

    assert!(
        !events(&fixture.job)
            .await
            .iter()
            .any(|event| terminal(event.event())),
        "two turns leave the run open: no terminal record yet"
    );
    assert!(
        !records(&fixture.job, lifecycle::FILE)
            .iter()
            .any(|line| line["summary"] == serde_json::json!(true)),
        "the run's timeline has not ended before its last turn is done"
    );
    let last_sequence = events(&fixture.job)
        .await
        .iter()
        .map(|event| event.sequence().get())
        .max()
        .unwrap_or(0);

    // The run end, exactly as `run()` performs it after its turn: the terminal
    // state, then the one finish of the timeline.
    fixture
        .job
        .finalize_run(Ok(last))
        .await
        .expect("the run ends");
    fixture
        .job
        .lifecycle()
        .finish(&fixture.job.application.store, fixture.job.job_id())
        .await;

    let events = events(&fixture.job).await;
    let finished: Vec<&CodingEventV1> = events
        .iter()
        .filter(|event| terminal(event.event()))
        .collect();
    assert_eq!(
        finished.len(),
        1,
        "the run records one terminal state, not one per turn: {events:?}"
    );
    assert_eq!(
        finished[0].sequence().get(),
        last_sequence + 1,
        "the run's finish follows its last turn"
    );
    let timeline =
        std::fs::read(fixture.job.records.join(lifecycle::FILE)).expect("the run's timeline");
    assert!(
        !timeline.is_empty(),
        "the run's timeline records its finish"
    );

    // A second finish writes no second record: the timeline ends once.
    fixture
        .job
        .lifecycle()
        .finish(&fixture.job.application.store, fixture.job.job_id())
        .await;
    assert_eq!(
        std::fs::read(fixture.job.records.join(lifecycle::FILE)).expect("the run's timeline"),
        timeline,
        "the run's finish is written once"
    );
}
