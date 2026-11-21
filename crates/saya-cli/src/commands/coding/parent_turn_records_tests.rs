//! Every root turn of an open run leaves its own durable record (phase-3 unit
//! 3-C).
//!
//! A second message on an open run is another turn of the same run, so it is as
//! durable as the first: its own turn boundary record in the job's observations
//! — same epoch, fresh sequence, monotonic — and its own progress events. The
//! rewind side is `parent_turn_rewind_tests`, over these same helpers.
#![cfg(unix)]

use super::super::super::NativeCodingJob;
use super::super::TurnInputs;
use super::super::parent_turn_fixture::in_place_writing;
use super::super::parent_turn_provider::{Reply, Scripted};
use saya_agent::{AgentOutput, NoopEventSink};
use saya_types::{CodingEventKind, CodingEventV1, JobState};
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The script two writing turns share: each turn creates one file of its own,
/// so the run's checkpoint captures a state that moved between the turns.
pub(super) fn two_writes() -> Vec<Reply> {
    vec![
        Reply::Write {
            path: "turn-one.txt",
            content: "one\n",
        },
        Reply::Text("wrote the first turn"),
        Reply::Write {
            path: "turn-two.txt",
            content: "two\n",
        },
        Reply::Text("wrote the second turn"),
    ]
}

/// Runs the run's first turn (`None`) or a further turn, and hands back the
/// turn's output, which the run end takes as its result.
pub(super) async fn turn(job: &Arc<NativeCodingJob>, prompt: Option<&str>) -> AgentOutput {
    let settled = match prompt {
        None => {
            tokio::time::timeout(RUN, job.run_open(&NoopEventSink, TurnInputs::default())).await
        }
        Some(prompt) => tokio::time::timeout(RUN, job.run_next_turn(&NoopEventSink, prompt)).await,
    };
    settled
        .expect("the turn must settle")
        .expect("the turn must run")
}

/// The job's durable turn boundary records, in write order.
fn boundaries(job: &NativeCodingJob) -> Vec<Value> {
    records(job, "observations.jsonl")
        .into_iter()
        .filter(|record| record["phase"] == "turn_started")
        .collect()
}

/// The job's durable events, in sequence order.
pub(super) async fn events(job: &NativeCodingJob) -> Vec<CodingEventV1> {
    let mut all = Vec::new();
    let mut cursor = None;
    loop {
        let page = job
            .application
            .store
            .list_coding_events(job.job_id(), cursor, 100)
            .await
            .expect("the job's events read");
        all.extend(page.events);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return all,
        }
    }
}

/// The sequences of the job's durable progress checks, in order.
fn progress(events: &[CodingEventV1]) -> Vec<u64> {
    events
        .iter()
        .filter(|event| matches!(event.event(), CodingEventKind::ProgressReported { .. }))
        .map(|event| event.sequence().get())
        .collect()
}

/// The record the run's end writes: its terminal state, once.
pub(super) fn terminal(event: &CodingEventKind) -> bool {
    matches!(event, CodingEventKind::RunTerminated { .. })
        || matches!(
            event,
            CodingEventKind::StateChanged { state }
                if matches!(
                    state,
                    JobState::Completed | JobState::Failed | JobState::Cancelled
                )
        )
}

/// The JSON lines of one of the job's private diagnostic streams.
pub(super) fn records(job: &NativeCodingJob, name: &str) -> Vec<Value> {
    let Ok(bytes) = std::fs::read_to_string(job.records.join(name)) else {
        return Vec::new();
    };
    bytes
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// One file of the job's working folder, as it stands.
pub(super) fn read(folder: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(folder.join(name)).ok()
}

#[tokio::test]
async fn each_root_turn_writes_its_own_boundary_record() {
    let provider = Scripted::start_with_script(two_writes());
    let fixture = in_place_writing("records-boundaries", &provider.endpoint).await;
    let folder = fixture.root.join("repo");
    let epoch = fixture.job.epoch;

    turn(&fixture.job, None).await;
    turn(&fixture.job, Some("second message")).await;

    assert_eq!(
        read(&folder, "turn-one.txt").as_deref(),
        Some("one\n"),
        "the first turn wrote its own file"
    );
    assert_eq!(
        read(&folder, "turn-two.txt").as_deref(),
        Some("two\n"),
        "the second turn wrote its own file"
    );

    let boundaries = boundaries(&fixture.job);
    assert_eq!(
        boundaries.len(),
        2,
        "each root turn of the run writes its own boundary record: {boundaries:?}"
    );
    assert_eq!(
        boundaries[0]["epoch"], boundaries[1]["epoch"],
        "a further turn is another turn of the same epoch: {boundaries:?}"
    );
    assert_eq!(
        boundaries[1]["epoch"],
        serde_json::json!(epoch),
        "the boundary records carry the run's epoch: {boundaries:?}"
    );
    let sequences: Vec<u64> = boundaries
        .iter()
        .map(|record| {
            record["turn"]
                .as_u64()
                .unwrap_or_else(|| panic!("a boundary record states its turn: {record}"))
        })
        .collect();
    assert!(
        sequences[0] < sequences[1],
        "the later turn's sequence is fresh and larger: {sequences:?}"
    );
}

#[tokio::test]
async fn each_root_turn_appends_its_own_progress_events() {
    let provider = Scripted::start_with_script(two_writes());
    let fixture = in_place_writing("records-progress", &provider.endpoint).await;

    turn(&fixture.job, None).await;
    let after_first = progress(&events(&fixture.job).await);
    assert!(
        !after_first.is_empty(),
        "the first turn records its progress checks"
    );

    turn(&fixture.job, Some("second message")).await;
    let after_second = progress(&events(&fixture.job).await);
    assert!(
        after_second.len() > after_first.len(),
        "the second turn appends its own progress checks: {after_first:?} then {after_second:?}"
    );
    assert!(
        after_second.windows(2).all(|pair| pair[0] < pair[1]),
        "the durable checks keep one increasing sequence: {after_second:?}"
    );
}
