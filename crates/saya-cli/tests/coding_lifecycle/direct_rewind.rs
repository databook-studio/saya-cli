//! The operator rewind end to end: a real direct-mode job runs two turns that
//! each change the working checkout, the operator rewinds it to the first
//! checkpoint through the one operation both adapters call, and the checkout
//! matches that checkpoint exactly — the file the later turn created is gone.
//! The next run's context stating the rewind is covered in
//! `commands::coding::rewind::job_tests`.

use crate::direct_readiness_facts::support::{Setup, facts_job, run};
use saya_cli::CodingError;
use saya_types::JobState;

/// Two turns of edits, then a rewind to after turn one: the checkout holds
/// turn one's file with turn one's bytes, and the file turn two created is
/// removed. The rewind also leaves the note the next run's context states.
#[tokio::test]
async fn an_operator_rewind_restores_the_working_checkout_to_a_checkpoint() {
    let (fixture, job, _provider) = facts_job(Setup::new(
        "direct-rewind",
        113,
        vec!["turn-one.txt".into(), "turn-two.txt".into()],
    ))
    .await;
    run(&job).await;

    // The run ended, so the checkout is not live: the durable state is
    // what gates the rewind, exactly as it does for a job running elsewhere.
    let state = fixture
        .store
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .expect("the job record exists")
        .state;
    // The run ended, so no turn is live. Only `Running` refuses a rewind, and
    // a run that ends without a prepared candidate reads as `Paused`.
    assert_ne!(state, JobState::Running, "the scripted run ended");
    let checkout = crate::direct_task::root_checkout(&fixture, &job).await;
    let read = |name: &str| std::fs::read_to_string(checkout.join(name));

    let listed = fixture
        .application
        .rewind(job.job_id(), None, false)
        .await
        .expect("the listing reads the job's checkpoints");
    let turns: Vec<u64> = listed["checkpoints"]
        .as_array()
        .expect("the listing carries the checkpoints")
        .iter()
        .map(|checkpoint| checkpoint["turn"].as_u64().unwrap())
        .collect();
    assert_eq!(turns, [1, 2], "one checkpoint per turn that changed files");
    assert_eq!(
        listed["checkpoints"][0]["paths"][0]["path"], "turn-one.txt",
        "the first checkpoint is the state after turn one"
    );
    assert_eq!(read("turn-one.txt").unwrap(), "turn one\n");
    assert_eq!(read("turn-two.txt").unwrap(), "turn two\n");

    let report = fixture
        .application
        .rewind(job.job_id(), Some(1), false)
        .await
        .expect("the rewind restores the first checkpoint");
    assert_eq!(report["turn"], 1);
    assert_eq!(report["restored_paths"], 1);
    assert_eq!(report["removed_paths"], 1);
    assert_eq!(report["authority"], "none");
    assert_eq!(
        read("turn-one.txt").unwrap(),
        "turn one\n",
        "the file the first turn wrote is back at its checkpointed bytes"
    );
    assert!(
        matches!(read("turn-two.txt"), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "the file the second turn created is gone"
    );

    // A turn this job never checkpointed names nothing to restore.
    assert!(matches!(
        fixture
            .application
            .rewind(job.job_id(), Some(9), false)
            .await,
        Err(CodingError::RewindUnknownTurn { turn: 9 })
    ));
}
