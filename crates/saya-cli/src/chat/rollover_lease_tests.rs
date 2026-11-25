//! A rollover while a job the session started still runs.
//!
//! The session's run holds the folder's lease, and the job it started over the
//! same folder is admitted beside it without a lease of its own. When the
//! session rolls that run over, the folder is handed to the running job, so the
//! successor run reads the folder rather than writing beside a job that already
//! writes it. The folder is free for the session's next run once the job ends.

#![cfg(unix)]

use std::sync::Arc;

use saya_harness::coding::local::LocalRoot;

use super::compose_fixture;
use super::operator_front_fixture;
use super::rollover::{Ending, Rollover, RolloverCause};
use super::rollover_fixture::{next_run, open, transcript};
use super::workspace_shape::{WorkspaceShape, decide};
use crate::commands::coding::folder_lease_held;
use crate::commands::coding::in_place_lease::InPlaceLease;
use crate::commands::coding::parent_turn_provider::Scripted;

/// The session's run rolls over while its dependent job still runs: the
/// successor reads the folder the job holds, and the folder is free once the
/// job ends.
#[tokio::test]
async fn a_rollover_while_a_dependent_job_runs_reads_the_folder() {
    let provider = Scripted::start();
    let fixture = compose_fixture::in_place("rollover-job-lease", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let session = open(fixture, &front, "roll-job").await;
    let mut transcript = transcript("roll-job");

    // The folder the session's run holds, and the job it started over it.
    let folder = session.application.root_path().to_path_buf();
    let state = session.application.state_root().to_path_buf();
    let root = LocalRoot::open(&folder, &state).expect("the folder is an in-place root");
    let root_id = root.id().as_str().to_owned();
    let job = InPlaceLease::acquire(
        &state,
        &root_id,
        "roll-job",
        Some("roll-job"),
        "roll-job-job",
    )
    .expect("the job is admitted beside the session's run");

    let rollover = Rollover::new(Arc::clone(&session.application));
    let rolled = rollover
        .roll(
            Ending::new(session.run),
            RolloverCause::Model,
            next_run(front.front(), "roll-job-2"),
            &mut transcript,
        )
        .await
        .expect("the rollover prepares the run beside the job");

    assert!(
        !rolled.job().holds_folder_lease(),
        "the successor run reads the folder the running job holds"
    );
    assert!(
        folder_lease_held(&state, &root_id).expect("the folder's lease is readable"),
        "the running job holds the folder the session let go"
    );
    assert_eq!(
        decide(&folder, &state, "second-session"),
        WorkspaceShape::ReadOnlyFiles {
            reason: "another session owns this folder".to_owned()
        },
        "a second session reads the folder the running job holds"
    );

    drop(job);
    assert!(
        !folder_lease_held(&state, &root_id).expect("the folder's lease is readable"),
        "the folder is free once the job ends"
    );
}
