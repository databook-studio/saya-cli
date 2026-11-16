//! A `helpers` call registers its child only once the child's brief composes,
//! so a composition failure leaves no child row and no task-history entry.
//! Several `helpers` calls in one turn reach the delegation together; a call
//! that cannot be admitted registers nothing of its own, and a re-delegation
//! naming an agent already in the task history is refused.
use super::{child_checkouts, direct_fixture, tool_results, write_job};
use crate::{provider::Provider, support::Fixture};
use saya_agent::NoopEventSink;
use saya_cli::NativeCodingJob;
use saya_types::JobState;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, sync::Arc, time::Duration};

async fn run_job(
    label: &str,
    mode: u8,
) -> (Fixture, Arc<NativeCodingJob>, BTreeMap<String, Value>) {
    let provider = Provider::script(false, mode);
    let fixture = direct_fixture(label, &provider).await;
    let job = write_job(&fixture, label, &["new-a.txt", "new-b.txt"]).await;
    tokio::time::timeout(Duration::from_secs(120), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .expect("a failed delegation does not fail the job");
    let results = tool_results(&provider);
    (fixture, job, results)
}

fn recorded_tasks(fixture: &Fixture, job: &NativeCodingJob) -> String {
    let path = fixture
        .root
        .join("coding")
        .join(job.job_id().as_str())
        .join("tasks.jsonl");
    fs::read_to_string(path).unwrap_or_default()
}

async fn children(fixture: &Fixture, job: &NativeCodingJob) -> Vec<(String, JobState)> {
    fixture
        .store
        .list_coding_children(job.job_id(), None, 10)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|child| (child.agent_id.as_str().to_owned(), child.state))
        .collect()
}

/// The advisory view of a job a rolled-back delegation left where it was:
/// nothing is live, nothing is ready, and the view suggests nothing. Direct
/// readiness is advisory, so there is no forced next action to name a tool.
async fn assert_idle(job: &NativeCodingJob) {
    let status = job.status().await.unwrap();
    let readiness = &status["readiness"];
    assert_eq!(readiness["live_children"], json!([]), "{readiness}");
    assert_eq!(readiness["candidate_ready"], false, "{readiness}");
    assert_eq!(readiness["suggestion"], "none", "{readiness}");
    assert!(status["next_action"].is_null(), "{status}");
}

#[tokio::test]
async fn a_composition_failure_on_a_single_task_leaves_no_child_and_nothing_to_await() {
    let (fixture, job, results) = run_job("unscheduled-single", 52).await;

    assert!(
        results["delegate-bad-0"]["error"].is_string(),
        "{}",
        results["delegate-bad-0"]
    );
    assert_eq!(
        children(&fixture, &job).await,
        vec![],
        "the failed child was never registered"
    );
    assert_eq!(
        recorded_tasks(&fixture, &job),
        "",
        "no task-history entry for it"
    );
    // The next root call is a plain read of the shared checkout; the advisory
    // view has nothing live and nothing to prepare.
    assert!(
        results["status-after"].get("error").is_none(),
        "{}",
        results["status-after"]
    );
    assert_idle(&job).await;
}

#[tokio::test]
async fn the_failed_delegation_creates_no_child_checkout_and_its_effect_stays_uncertain() {
    let (fixture, job, _) = run_job("unscheduled-effect", 52).await;

    // Helpers share the root's working checkout W: a failed delegation
    // creates no per-helper checkout, only W.
    assert_eq!(
        child_checkouts(&fixture, &job).await,
        0,
        "no per-helper checkout exists; W is not counted"
    );
    assert_eq!(
        crate::direct_prepare::fixture::checkouts_named(&fixture, &job, "work-").await,
        1,
        "the shared working checkout W is the only one"
    );
    // The uncomposable brief is refused after the child was registered (and
    // rolled back), so the call is past its pre-staging window: its effect
    // stays uncertain.
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        1,
        "a composition failure after registration may have started an effect, so its effect stays uncertain"
    );
}

#[tokio::test]
async fn two_helpers_calls_in_one_turn_each_run_and_return_their_own_summary() {
    let (fixture, job, results) = run_job("unscheduled-batch", 53).await;

    // Both `helpers` calls in the turn are admitted and run at the same time
    // (parallel helpers), so neither is refused as `Busy` and each returns
    // its own completed summary for the agent it named.
    assert_eq!(
        results["delegate-bad-0"][0]["agent"], "child-a",
        "the first call ran its helper: {}",
        results["delegate-bad-0"]
    );
    assert_eq!(
        results["delegate-bad-0"][0]["outcome"], "completed",
        "{}",
        results["delegate-bad-0"]
    );
    assert_eq!(
        results["delegate-bad-1"][0]["agent"], "child-b",
        "the second call ran its own helper: {}",
        results["delegate-bad-1"]
    );
    assert_eq!(
        results["delegate-bad-1"][0]["outcome"], "completed",
        "{}",
        results["delegate-bad-1"]
    );
    assert_eq!(
        children(&fixture, &job).await,
        vec![
            ("child-a".to_owned(), JobState::Completed),
            ("child-b".to_owned(), JobState::Completed)
        ],
        "both helpers registered and completed"
    );
    let history = recorded_tasks(&fixture, &job);
    assert_eq!(
        history.lines().count(),
        2,
        "each admitted call is durable: {history}"
    );
}

#[tokio::test]
async fn a_re_delegation_of_a_recorded_agent_is_refused_while_a_new_agent_is_admitted() {
    let (fixture, job, results) = run_job("unscheduled-retry", 54).await;

    assert!(
        results["delegate-bad-1"]["error"].is_string(),
        "{}",
        results["delegate-bad-1"]
    );
    // The corrected turn names `child-a`, already in the task history, so the
    // delegation refuses it; `child-b`, never recorded, is admitted.
    assert!(
        results["delegate-fixed-0"]["error"].is_string(),
        "a recorded agent cannot be delegated again: {}",
        results["delegate-fixed-0"]
    );
    assert_eq!(
        results["delegate-fixed-1"][0]["agent"], "child-b",
        "{}",
        results["delegate-fixed-1"]
    );
    assert_eq!(
        results["delegate-fixed-1"][0]["outcome"], "completed",
        "{}",
        results["delegate-fixed-1"]
    );
    assert_eq!(
        children(&fixture, &job).await,
        vec![
            ("child-a".to_owned(), JobState::Completed),
            ("child-b".to_owned(), JobState::Completed)
        ]
    );
    let history = recorded_tasks(&fixture, &job);
    assert_eq!(
        history.lines().count(),
        2,
        "each admitted call is durable: {history}"
    );
    assert_eq!(
        child_checkouts(&fixture, &job).await,
        0,
        "helpers share W, so no per-helper checkout exists"
    );
    // The recorded-agent refusal happens before any registration or checkout,
    // so it leaves nothing to reconcile. The one reservation that stays
    // uncertain is the first turn's uncomposable `child-b`, refused after its
    // registration.
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        1,
        "only the refusal after registration stays uncertain"
    );
}
