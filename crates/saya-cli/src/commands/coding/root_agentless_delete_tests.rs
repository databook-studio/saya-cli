//! Least privilege on the deletion checkout guard, re-pointed at the shared
//! working checkout. W is agent-less and every helper writes it, so a
//! live child's authority pointed at W is a legitimate deletion authority —
//! the checkout identity is the snapshot W was materialized from, not an
//! agent. What survives is the child-currency check: an invocation with no
//! live child row is refused before the checkout is touched, and the checkout
//! itself is left alone.

// The fixture composes child operations over a confined checkout; the Linux
// probe fails closed, so this suite needs a host that proves confinement.
#![cfg(target_os = "macos")]

use super::direct_root_fixture::{
    agent, child_identity, fixture, operations, root_checkout_id, root_path,
};
use saya_harness::workspace::Workspace;
use saya_store::NewCodingChild;
use saya_types::{
    CodingEventKind, CodingEventV1, InvocationId, JobState, Sha256Digest, WorkspacePath,
};
use std::num::NonZeroU64;

/// A live (Running) child whose durable row passes the child-currency check,
/// so the deletion reaches the checkout guard itself.
async fn make_running_child(job: &super::NativeCodingJob, agent_name: &str) -> InvocationId {
    let identity = child_identity(job, agent_name);
    let invocation = InvocationId::parse(&format!("inv-{}", &identity.as_str()[..32])).unwrap();
    job.application
        .store
        .create_coding_child(NewCodingChild {
            job_id: job.job_id().clone(),
            agent_id: agent(agent_name),
            current_invocation_id: invocation.clone(),
        })
        .await
        .unwrap();
    job.ledger
        .sequencer()
        .append_event(|sequence| {
            CodingEventV1::new(
                NonZeroU64::new(sequence).unwrap(),
                job.job_id().clone(),
                Some(agent(agent_name)),
                Some(invocation.clone()),
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            )
            .unwrap()
        })
        .await
        .unwrap();
    invocation
}

/// The digest of a tracked file as it sits in the root working checkout W.
fn tracked_digest(job: &super::NativeCodingJob, path: &str) -> Sha256Digest {
    Sha256Digest::parse(
        &Workspace::open(&root_path(job))
            .unwrap()
            .read(path, 1024 * 1024)
            .unwrap()
            .digest,
    )
    .unwrap()
}

/// A live child shares W, so its authority may delete a tracked file there.
#[tokio::test]
async fn a_live_child_authority_may_delete_in_the_shared_checkout() {
    let f = fixture("ra", vec!["tracked.txt".into()]).await;
    let invocation = make_running_child(&f.job, "child-a").await;
    let w = root_checkout_id(&f.job);
    let digest = tracked_digest(&f.job, "tracked.txt");
    let path = WorkspacePath::try_from("tracked.txt".to_string()).unwrap();
    // The child's delete authority is pointed at the shared working checkout W.
    let child_ops = operations(&f.job, &w, &agent("child-a"), &invocation).await;
    child_ops
        .delete_workspace(path, digest, f.job.cancellation())
        .await
        .expect("a live child shares W and may delete in it");
    assert!(
        !root_path(&f.job).join("tracked.txt").exists(),
        "the shared working checkout loses the deleted file"
    );
}

/// The surviving least-privilege rule: a child invocation with no live child
/// row is refused before the checkout is touched.
#[tokio::test]
async fn a_child_invocation_without_a_live_row_is_refused() {
    let f = fixture("ra-norow", vec!["tracked.txt".into()]).await;
    let w = root_checkout_id(&f.job);
    let identity = child_identity(&f.job, "child-a");
    let invocation = InvocationId::parse(&format!("inv-{}", &identity.as_str()[..32])).unwrap();
    let digest = tracked_digest(&f.job, "tracked.txt");
    let path = WorkspacePath::try_from("tracked.txt".to_string()).unwrap();
    let child_ops = operations(&f.job, &w, &agent("child-a"), &invocation).await;
    let error = child_ops
        .delete_workspace(path, digest, f.job.cancellation())
        .await
        .expect_err("a child with no live row must not delete");
    assert!(
        error.proves_no_effect()
            && error.to_string() == "deletion child invocation is no longer current",
        "the child-currency refusal is expected: {error:?}"
    );
    assert!(
        root_path(&f.job).join("tracked.txt").exists(),
        "the shared working checkout is untouched"
    );
}
