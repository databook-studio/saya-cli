//! A job that is still making progress, or has a child at work, is never
//! stopped. A helper is launched by a synchronous `helpers` call, so while the
//! child runs the root is parked in that call and a live child is what keeps
//! the run alive; the readiness view reports the wait, and the run finishes
//! normally once the child completes.
use super::tool_results;
use crate::{prepare_candidate, provider::Provider};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::CodingApplication;
use saya_types::JobState;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn a_live_child_is_not_a_stall_and_the_run_waits_for_it() {
    let provider = Provider::script(false, 93);
    let mut fixture = prepare_candidate::direct_fixture("converge-active", &provider).await;
    // The child's request is held while the root waits, so it outlasts the
    // fixture's short provider timeouts.
    fixture.runtime.resolved.ai.timeout_seconds = 120;
    fixture.runtime.resolved.ai.idle_timeout_seconds = 120;
    fixture.application = CodingApplication::open(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let job = prepare_candidate::job(&fixture, "converge-active").await;
    let running = Arc::clone(&job);
    let run = tokio::spawn(async move { running.run(&NoopEventSink).await });

    // Wait until the held child is live: the root is parked in its `helpers`
    // call, so the run cannot be a stall.
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if !job.status().await.unwrap()["readiness"]["live_children"]
                .as_array()
                .unwrap()
                .is_empty()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the root's helper is live while its request is held");
    assert!(!run.is_finished(), "the run is waiting on its child");
    assert!(job.failure().is_none(), "{:?}", job.failure());

    let live = job.status().await.unwrap();
    let readiness = &live["readiness"];
    assert_eq!(
        readiness["state"], "children_active",
        "a live child is what the advisory view reports: {readiness}"
    );
    assert_eq!(
        readiness["live_children"][0]["agent_id"], "child-active",
        "{readiness}"
    );
    assert_eq!(readiness["candidate_ready"], false, "{readiness}");
    assert_eq!(
        readiness["suggestion"], "wait for the helpers call to return",
        "{readiness}"
    );
    assert!(live["next_action"].is_null(), "{live}");

    provider.release();
    tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .unwrap()
        .unwrap()
        .expect("the child completed, so the run finishes normally");

    let results = tool_results(&provider);
    assert_eq!(
        results["final-status"]["content"], "     1→FIXED\n",
        "the run read the checkout after the child's change: {}",
        results["final-status"]
    );
    let settled = job.status().await.unwrap();
    assert_eq!(settled["readiness"]["state"], "candidate_ready");
    assert!(job.failure().is_none(), "{:?}", job.failure());
}

#[tokio::test]
async fn a_normal_successful_job_is_never_stopped() {
    let provider = Provider::direct_prepare_receipt();
    let fixture = prepare_candidate::direct_fixture("converge-happy", &provider).await;
    let job = prepare_candidate::job(&fixture, "converge-happy").await;

    tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .expect("a normal job completes");

    assert!(job.failure().is_none(), "{:?}", job.failure());
    assert_eq!(
        fixture
            .store
            .get_coding_job(job.job_id())
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Completed
    );
}
