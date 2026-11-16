//! The rewind as the job and the adapters reach it: the job's own capture
//! records the working checkout the operator lists, a live job refuses a
//! rewind, and both adapters parse their spelling into the one operation.

use crate::cli::CodingCommand;
use crate::commands::coding::CodingError;
use std::path::Path;

/// The job's own capture records the working checkout's changed state at a
/// turn boundary, and the operator's listing shows it.
#[cfg(unix)]
#[tokio::test]
async fn the_job_capture_records_the_working_checkout() {
    let fixture = crate::commands::coding::root_checkout_tests::fixture(
        "rewind-capture",
        vec!["tracked.txt".into()],
    )
    .await;
    fixture.job.ensure_root_checkout().await.unwrap().unwrap();
    let checkout = fixture
        .job
        .verified_root_checkout()
        .await
        .unwrap()
        .unwrap()
        .path()
        .as_str()
        .to_owned();
    std::fs::write(
        Path::new(&checkout).join("tracked.txt"),
        "written in turn one",
    )
    .unwrap();

    fixture.job.capture_rewind().await;

    let checkpoints = fixture.job.rewind_store().list().unwrap();
    assert_eq!(checkpoints.len(), 1, "one turn boundary, one checkpoint");
    assert_eq!(checkpoints[0].turn, 1);
    assert_eq!(
        checkpoints[0]
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["tracked.txt"]
    );
    // The operator's listing reads the same store the job's capture wrote: the
    // two resolve the store root from the job's own private directory.
    let listed = fixture
        .job
        .application
        .rewind(fixture.job.job_id(), None, false)
        .await
        .unwrap();
    assert_eq!(listed["checkpoints"][0]["turn"], 1);
    assert_eq!(listed["checkpoints"][0]["paths"][0]["path"], "tracked.txt");
    // A second capture with the checkout untouched records nothing: a retried
    // turn must not spend a checkpoint number on a state already held.
    fixture.job.capture_rewind().await;
    assert_eq!(fixture.job.rewind_store().list().unwrap().len(), 1);
}

/// The run that follows a rewind is told which checkpoint the checkout was
/// rewound to, once: the note is taken by the first continuation context that
/// reads it, and a later one carries the continuation block alone.
#[cfg(unix)]
#[tokio::test]
async fn the_next_runs_context_states_the_rewind_once() {
    let fixture = crate::commands::coding::root_checkout_tests::fixture(
        "rewind-note",
        vec!["tracked.txt".into()],
    )
    .await;

    let before = fixture.job.continuation_context().await.unwrap();
    assert_eq!(before.len(), 1, "without a rewind: the continuation block");

    fixture.job.rewind_store().record_rewind(3, 1_000).unwrap();
    let blocks = fixture.job.continuation_context().await.unwrap();
    assert_eq!(blocks.len(), 2);
    assert!(
        blocks[1].label.contains("rewound by the operator"),
        "the block is framed as untrusted context: {}",
        blocks[1].label
    );
    let note: serde_json::Value = serde_json::from_str(&blocks[1].body).unwrap();
    assert_eq!(note["rewound_to_checkpoint"], 3);
    assert_eq!(note["authority"], "none");
    assert!(
        note["meaning"].as_str().unwrap().contains("checkpoint 3"),
        "the block names the checkpoint it restored: {note}"
    );

    // Stated once: the next run's context is the continuation block alone.
    assert_eq!(fixture.job.continuation_context().await.unwrap().len(), 1);
}

/// A job that is still running refuses a rewind from the one operation both
/// adapters call, and its listing is still available. A turn it never
/// checkpointed is refused by name first: there is nothing to restore, whether
/// or not the job is live.
#[cfg(unix)]
#[tokio::test]
async fn a_rewind_while_the_job_is_running_is_refused() {
    let fixture = crate::commands::coding::root_checkout_tests::fixture(
        "rewind-running",
        vec!["tracked.txt".into()],
    )
    .await;
    let job_id = fixture.job.job_id().clone();

    assert!(
        fixture.job.rewind_store().list().unwrap().is_empty(),
        "a job that never checkpointed lists none"
    );
    assert!(
        fixture
            .job
            .application
            .rewind(&job_id, None, false)
            .await
            .is_ok(),
        "listing is allowed whatever the job is doing"
    );
    assert!(matches!(
        fixture
            .job
            .application
            .rewind(&job_id, Some(1), false)
            .await,
        Err(CodingError::RewindUnknownTurn { turn: 1 })
    ));

    // With turn 1 present, the live job is what refuses the rewind.
    fixture.job.ensure_root_checkout().await.unwrap().unwrap();
    let checkout = fixture
        .job
        .verified_root_checkout()
        .await
        .unwrap()
        .unwrap()
        .path()
        .as_str()
        .to_owned();
    std::fs::write(
        Path::new(&checkout).join("tracked.txt"),
        "written while live",
    )
    .unwrap();
    fixture.job.capture_rewind().await;
    assert_eq!(fixture.job.rewind_store().list().unwrap().len(), 1);
    assert!(matches!(
        fixture
            .job
            .application
            .rewind(&job_id, Some(1), false)
            .await,
        Err(CodingError::RewindWhileRunning)
    ));
    assert_eq!(
        std::fs::read_to_string(Path::new(&checkout).join("tracked.txt")).unwrap(),
        "written while live",
        "a refused rewind writes nothing"
    );
}

/// Both adapters parse their own spelling into the one command, and both reach
/// the one operation: `--to` and the session's positional turn resolve to the
/// same checkpoint, while stating both is refused rather than resolved by
/// picking one.
#[test]
fn both_adapters_resolve_the_same_rewind() {
    use clap::Parser;

    let from_cli = crate::Cli::try_parse_from(["saya", "job", "rewind", "job-a", "--to", "2"])
        .expect("the CLI parses `--to`");
    let Some(crate::Command::Job { command, .. }) = from_cli.command else {
        panic!("the CLI parsed a job subcommand");
    };
    let from_slash = crate::parse_slash_command("/job rewind job-a 2")
        .expect("the session parses the positional turn");
    let Some(crate::SlashCommand::Coding(from_slash)) = from_slash else {
        panic!("the session parsed a coding subcommand");
    };

    let CodingCommand::Rewind { id, to, turn, .. } = command else {
        panic!("the CLI spelling is the rewind command");
    };
    assert_eq!(id, "job-a");
    assert_eq!(super::target(to, turn).unwrap(), Some(2));
    let CodingCommand::Rewind { id, to, turn, .. } = from_slash else {
        panic!("the session spelling is the rewind command");
    };
    assert_eq!(id, "job-a");
    assert_eq!(super::target(to, turn).unwrap(), Some(2));

    // No turn stated is a listing, on either surface.
    assert_eq!(super::target(None, None).unwrap(), None);
    // Two spellings that could disagree name two requests, so they are refused.
    assert!(matches!(
        super::target(Some(1), Some(2)),
        Err(CodingError::InvalidRequest)
    ));
}
