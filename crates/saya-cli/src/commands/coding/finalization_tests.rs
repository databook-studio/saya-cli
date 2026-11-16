use super::super::{ChildFollowUpRequest, CodingApplication, CodingAuthorization};
use crate::coding_test_support::{follow_up_fixture, follow_up_provider};
use saya_agent::CancellationToken;
use saya_types::JobState;
#[path = "finalization_tests/first_cause.rs"]
mod first_cause;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
#[test]
fn prior_root_failure_precedes_diagnostic_failure_while_waiting() {
    run(
        "first-cause",
        "prior_root_failure_precedes_diagnostic_failure_while_waiting",
    );
}
#[test]
fn durable_cancel_while_waiting_without_late_child_keeps_cause() {
    run(
        "wait-cancel",
        "durable_cancel_while_waiting_without_late_child_keeps_cause",
    );
}
#[test]
fn terminal_append_waits_for_existing_operation_lease() {
    run(
        "lease",
        "terminal_append_waits_for_existing_operation_lease",
    );
}
#[test]
fn finalizer_joins_actual_late_follow_up_before_pausing() {
    run(
        "late",
        "finalizer_joins_actual_late_follow_up_before_pausing",
    );
}
#[test]
fn durable_cancel_reaches_late_finalizer_join() {
    run("cancel", "durable_cancel_reaches_late_finalizer_join");
}
#[test]
fn final_output_fences_an_unprepared_write_candidate() {
    run("fence", "final_output_fences_an_unprepared_write_candidate");
}
async fn exercise(case: &str) {
    let root = crate::state_path::state_db_path()
        .parent()
        .unwrap()
        .to_path_buf();
    let provider = follow_up_provider::Provider::new();
    let fixture = follow_up_fixture::Fixture::new(&root, &provider).await;
    let application = CodingApplication::session(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        crate::cli::RootMode::Managed,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let approval = CodingAuthorization::scoped(
        &application
            .propose_resume(fixture.job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = application
        .resume(fixture.job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    if case == "fence" {
        let output = resumed
            .finalize_run(Ok(fixture.output.clone()))
            .await
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&output.answer).unwrap();
        assert_eq!(envelope["candidate_ready"], false);
        assert_eq!(envelope["apply_requires_fresh_approval"], false);
        assert_eq!(envelope["model_prose"], "untrusted");
        // Direct mode has no forced next action: the root decides whether to
        // delegate and the owner prepares at the end, so the envelope must not
        // invent a step the direct root cannot take. The advisory facts replace
        // it, and this job's working checkout never changed.
        assert!(
            envelope.get("next_action").is_none(),
            "the direct envelope carries no forced next action: {envelope}"
        );
        assert_eq!(envelope["suggestion"], "none");
        assert_eq!(envelope["w_changed"], false);
        assert_eq!(envelope["readiness"]["state"], "working");
        assert_eq!(envelope["readiness"]["candidate_ready"], false);
        assert_eq!(
            fixture
                .store
                .get_coding_job(resumed.job_id())
                .await
                .unwrap()
                .unwrap()
                .state,
            JobState::Completed
        );
        return;
    }
    if case == "first-cause" {
        first_cause::prior_error(&resumed, &fixture).await;
        return;
    }
    if case == "wait-cancel" {
        first_cause::waiting_cancel(&resumed, &fixture, &application).await;
        return;
    }
    if case == "lease" {
        let lease = resumed.operation.lock().await;
        let finalizing = resumed.finalize_run(Ok(fixture.output.clone()));
        tokio::pin!(finalizing);
        let early = tokio::time::timeout(Duration::from_millis(300), &mut finalizing).await;
        let state = fixture
            .store
            .get_coding_job(resumed.job_id())
            .await
            .unwrap()
            .unwrap()
            .state;
        let premature = early.is_ok();
        drop(lease);
        if !premature {
            tokio::time::timeout(Duration::from_secs(10), &mut finalizing)
                .await
                .unwrap()
                .unwrap();
        }
        assert!(
            !premature,
            "finalization overtook the actual operation lease"
        );
        assert_eq!(
            state,
            JobState::Running,
            "terminal state must wait for the operation lease"
        );
        assert_eq!(
            fixture
                .store
                .get_coding_job(resumed.job_id())
                .await
                .unwrap()
                .unwrap()
                .state,
            JobState::Completed
        );
        return;
    }
    let receipt = resumed
        .follow_up(ChildFollowUpRequest {
            agent_id: fixture.completion.result.agent_id().clone(),
            expected_completion_sequence: fixture.completion.sequence,
            instruction: "bounded actual follow-up".into(),
        })
        .await
        .unwrap();
    assert!(
        wait_for(|| provider.started.load(Ordering::Acquire)).await,
        "new child reached actual held model request"
    );
    if case == "cancel" {
        let finalize = resumed.finalize_run(Ok(fixture.output.clone()));
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            application.cancel(resumed.job_id()).await.unwrap();
        };
        let combined = async {
            let (result, ()) = tokio::join!(finalize, cancel);
            result
        };
        let outcome = tokio::time::timeout(Duration::from_secs(10), combined).await;
        provider.release.store(true, Ordering::Release);
        if resumed.active.lock().await.is_some() {
            let _ = tokio::time::timeout(Duration::from_secs(10), resumed.join()).await;
        }
        assert!(
            outcome.is_ok(),
            "actual durable monitor must release the late finalizer join"
        );
        assert!(
            outcome.unwrap().is_err(),
            "cancelled late work is not successful prior output"
        );
        assert!(
            fixture
                .store
                .coding_cancel_requested(resumed.job_id())
                .await
                .unwrap()
        );
        assert!(matches!(
            fixture
                .store
                .get_coding_job(resumed.job_id())
                .await
                .unwrap()
                .unwrap()
                .state,
            JobState::Cancelled | JobState::Failed
        ));
        return;
    }
    let finalizing = resumed.finalize_run(Ok(fixture.output.clone()));
    tokio::pin!(finalizing);
    let early = tokio::time::timeout(Duration::from_millis(300), &mut finalizing).await;
    let state = fixture
        .store
        .get_coding_job(resumed.job_id())
        .await
        .unwrap()
        .unwrap()
        .state;
    let premature = early.is_ok();
    provider.release.store(true, Ordering::Release);
    if premature {
        tokio::time::timeout(Duration::from_secs(10), resumed.join())
            .await
            .unwrap()
            .unwrap();
    } else {
        tokio::time::timeout(Duration::from_secs(10), &mut finalizing)
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        !premature,
        "accepted late child was unfinished when root finalization returned"
    );
    assert_eq!(
        state,
        JobState::Running,
        "accepted unfinished child cannot coexist with root Paused"
    );
    assert!(resumed.active.lock().await.is_none());
    let history = fixture
        .store
        .list_coding_completion_history(resumed.job_id(), None, 32)
        .await
        .unwrap();
    assert_eq!(history.items.len(), 2);
    assert!(
        history
            .items
            .iter()
            .any(|item| item.result.invocation_id() == &receipt.invocation_id)
    );
    assert_eq!(
        fixture
            .store
            .get_coding_job(resumed.job_id())
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Completed
    );
}
async fn wait_for(mut ready: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        if ready() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

fn run(case: &str, name: &str) {
    let test = format!("commands::coding::failure::finalization_tests::{name}");
    if crate::coding_test_support::isolation::run_parent(case, &test) {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(tokio::task::LocalSet::new().run_until(exercise(case)));
}
