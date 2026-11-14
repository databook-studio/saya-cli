//! The operator follow-up of a direct child that changed nothing keeps its
//! follow-up, and the follow-up's OWN invocation writes the shared working
//! checkout W the candidate is built from.

use super::direct_task::{root_checkout, run_direct};
use saya_agent::CancellationToken;
use saya_cli::{ChildFollowUpRequest, CodingApplication, CodingAuthorization, NativeCodingJob};
use saya_types::AgentId;
use std::sync::Arc;

fn child(agent: &str) -> AgentId {
    AgentId::parse(agent).unwrap()
}

/// The resumed job's own view of the follow-up request for one completion.
fn request(agent: &str, sequence: std::num::NonZeroU64, instruction: &str) -> ChildFollowUpRequest {
    ChildFollowUpRequest {
        agent_id: child(agent),
        expected_completion_sequence: sequence,
        instruction: instruction.into(),
    }
}

async fn resume(app: &Arc<CodingApplication>, job: &NativeCodingJob) -> Arc<NativeCodingJob> {
    let approval =
        CodingAuthorization::scoped(&app.propose_resume(job.job_id()).await.unwrap()).unwrap();
    app.resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_follow_up_of_a_child_that_changed_nothing_still_works() {
    // Mode 107: the root hands `child-a` the path `tracked.txt` and the child
    // inspects it without changing it, so its first invocation leaves W alone.
    let (fixture, app, job, _provider) = run_direct(
        "direct-follow-up-unmerged",
        107,
        vec!["tracked.txt".into()],
        vec![],
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(root_checkout(&fixture, &job).await.join("tracked.txt")).unwrap(),
        "original\n",
        "W still holds the baseline content: the child changed nothing"
    );
    let completion = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();

    let resumed = resume(&app, &job).await;
    let receipt = resumed
        .run_follow_up(request(
            "child-a",
            completion.sequence,
            "write for child-a; second pass",
        ))
        .await
        .expect("an unmerged child keeps its operator follow-up");
    assert_ne!(
        receipt["follow_up"]["invocation_id"].as_str(),
        Some(completion.result.invocation_id().as_str()),
        "the follow-up ran a new invocation: {receipt}"
    );
    assert_eq!(
        fixture
            .store
            .coding_child_invocation_history(job.job_id(), &child("child-a"))
            .await
            .unwrap()
            .len(),
        2,
        "the follow-up child completed its own invocation"
    );
}

#[tokio::test]
async fn a_follow_ups_own_work_reaches_the_working_checkout() {
    // Mode 108: the child's first invocation changes nothing; the follow-up's
    // own invocation writes the path the child was assigned.
    let (fixture, app, job, _provider) = run_direct(
        "direct-follow-up-lands",
        108,
        vec!["tracked.txt".into()],
        vec![],
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(root_checkout(&fixture, &job).await.join("tracked.txt")).unwrap(),
        "original\n",
        "W still holds the baseline content: the first invocation changed nothing"
    );
    let completion = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();

    let resumed = resume(&app, &job).await;
    resumed
        .run_follow_up(request(
            "child-a",
            completion.sequence,
            "write for child-a; second pass",
        ))
        .await
        .expect("an unmerged child keeps its operator follow-up");
    assert_eq!(
        std::fs::read_to_string(root_checkout(&fixture, &job).await.join("tracked.txt")).unwrap(),
        "followed up\n",
        "the follow-up's own completion reached W"
    );
    // The follow-up's completion is a NEW completion of the same agent: it is
    // recorded against its own sequence, not skipped as a repeat of the first.
    let follow_up_completion = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    assert_ne!(follow_up_completion.sequence, completion.sequence);
    // The follow-up ran its own invocation of the same agent: the child's
    // invocation history carries both, and its work is in W directly.
    assert_eq!(
        fixture
            .store
            .coding_child_invocation_history(job.job_id(), &child("child-a"))
            .await
            .unwrap()
            .len(),
        2,
        "the follow-up ran a second invocation of child-a"
    );
}
