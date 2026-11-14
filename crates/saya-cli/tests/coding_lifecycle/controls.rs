#[path = "control_support.rs"]
pub(crate) mod control_support;

use super::support::Fixture;
use control_support::ControlProvider;
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, CodingTask};
use saya_harness::coding::coordinator::{ChildRunError, ScheduleChildError};
use saya_types::{AgentId, JobId};
use std::{fs, time::Duration};

/// Opens a direct job over the approved `write_paths`.
async fn job(
    label: &str,
    endpoint: &str,
    objective: &str,
    write_paths: &[&str],
) -> (Fixture, std::sync::Arc<saya_cli::NativeCodingJob>) {
    let fixture = Fixture::with_endpoint(label, Some(endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        objective.into(),
        write_paths.iter().map(|path| (*path).to_owned()).collect(),
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    (fixture, job)
}

#[tokio::test]
async fn refused_delegate_does_not_pollute_the_corrected_assignment_history() {
    let provider = ControlProvider::new();
    let (fixture, job) = job(
        "delegate-admission",
        &provider.endpoint,
        "Admit only validated native children",
        &["a.txt"],
    )
    .await;

    // The surviving assignment-content refusal: a helper assignment over
    // the 32 KiB cap. `write_paths` admits nothing any more (helpers share
    // the root working checkout), so it is the assignment text that the model
    // corrects, not the declared paths.
    let rejected = CodingTask {
        agent_id: AgentId::parse("child-a").unwrap(),
        subagent_type: "general".into(),
        task: "x".repeat(32 * 1024 + 1),
        write_paths: vec!["a.txt".into()],
        dependencies: vec![],
    };
    assert!(matches!(
        job.delegate(vec![rejected]).await,
        Err(CodingError::AssignmentTooLong)
    ));

    provider.release();
    job.delegate(vec![task("child-a", "a.txt")]).await.unwrap();
    let status = job.status().await.unwrap();
    let child = status["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|child| child["agent_id"] == "child-a")
        .unwrap();
    assert_eq!(child["write_paths"], serde_json::json!(["a.txt"]));
    assert_eq!(child["dependencies"], serde_json::json!([]));
    let history =
        fs::read_to_string(fixture.root.join("coding/delegate-admission/tasks.jsonl")).unwrap();
    assert_eq!(
        history.lines().count(),
        1,
        "only the admitted batch is durable"
    );
    job.join().await.unwrap();
}

#[tokio::test]
async fn duplicate_agents_in_one_batch_refuse_before_history_checkout_or_budget_effects() {
    let provider = ControlProvider::new();
    let (fixture, job) = job(
        "duplicate-delegate",
        &provider.endpoint,
        "Refuse ambiguous child ownership before effects",
        &["a.txt", "b.txt"],
    )
    .await;

    assert!(matches!(
        job.delegate(vec![task("child-a", "a.txt"), task("child-a", "a.txt")])
            .await,
        Err(CodingError::InvalidRequest)
    ));
    no_delegate_effects(&fixture, &job).await;
}

/// The job's durable effect counts a refused batch must leave untouched: the
/// recorded children, the checkouts, and the budget reservations.
async fn effect_counts(
    fixture: &Fixture,
    job: &saya_cli::NativeCodingJob,
) -> (usize, usize, usize) {
    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 10)
        .await
        .unwrap()
        .items
        .len();
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 10)
        .await
        .unwrap()
        .items
        .len();
    let reservations = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap()
        .items
        .len();
    (children, checkouts, reservations)
}

#[tokio::test]
async fn a_later_assignment_for_an_admitted_child_refuses_before_effects() {
    let provider = ControlProvider::new();
    let (fixture, job) = job(
        "repeat-delegate",
        &provider.endpoint,
        "Refuse a second assignment for an admitted child before effects",
        &["tree", "other"],
    )
    .await;

    provider.release();
    job.delegate(vec![task("child-a", "tree")]).await.unwrap();
    job.join().await.unwrap();
    let history = fixture.root.join("coding/repeat-delegate/tasks.jsonl");
    let before_history = fs::read_to_string(&history).unwrap();
    let before = effect_counts(&fixture, &job).await;
    // No per-child path ownership exists, so an overlapping *path* is no
    // longer refused: the task history is authoritative for admission, so a
    // later batch that names an already-recorded child fails closed before any
    // history, checkout or budget effect.
    let refusal = job
        .delegate(vec![task("child-a", "tree/nested.txt")])
        .await
        .unwrap_err();
    assert!(
        matches!(refusal, CodingError::InvalidRequest),
        "a re-assignment of an admitted child is refused: {refusal}"
    );
    assert_eq!(
        fs::read_to_string(&history).unwrap(),
        before_history,
        "a later assignment cannot add a second admitted batch"
    );
    assert_eq!(
        effect_counts(&fixture, &job).await,
        before,
        "a refused later batch adds no child, checkout or reservation"
    );
}

async fn no_delegate_effects(fixture: &Fixture, job: &saya_cli::NativeCodingJob) {
    assert!(
        !fixture
            .root
            .join("coding")
            .join(job.job_id().as_str())
            .join("tasks.jsonl")
            .exists(),
        "a rejected batch has no durable assignment record"
    );
    assert!(
        fixture
            .store
            .list_coding_children(job.job_id(), None, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        fixture
            .store
            .list_managed_checkouts(job.job_id(), None, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        fixture
            .store
            .list_coding_budget_reservations(job.job_id(), None, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn live_controls_acknowledge_only_at_the_safe_boundary() {
    let provider = ControlProvider::new();
    // The fixture is held (its `Drop` owns the temp state) though this test
    // reads the job's status, not the store.
    let (_fixture, job) = job(
        "live-controls",
        &provider.endpoint,
        "Run controlled native children",
        &["a.txt", "b.txt"],
    )
    .await;
    assert!(matches!(
        job.control_command(CodingCommand::Pause {
            id: "live-controls".into(),
            agent: "coordinator".into(),
        })
        .await,
        Err(CodingError::NotOwned)
    ));
    job.delegate(vec![task("child-a", "a.txt"), task("child-b", "b.txt")])
        .await
        .unwrap();
    provider.wait_for_calls(2).await;

    // The status names each child's owned paths; a child with no unfinished
    // dependency is not blocked.
    let status = job.status().await.unwrap();
    let child_a = status["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|child| child["agent_id"] == "child-a")
        .unwrap();
    assert_eq!(child_a["write_paths"], serde_json::json!(["a.txt"]));
    assert_eq!(child_a["blocked_by"], serde_json::json!([]));
    assert_eq!(
        status["readiness"]["state"], "children_active",
        "the advisory view names the live children instead of a forced step"
    );

    assert_eq!(
        job.control_command(CodingCommand::Message {
            id: "live-controls".into(),
            agent: "child-a".into(),
            text: "check the boundary".into(),
        })
        .await
        .unwrap()["queued"],
        true
    );
    assert!(matches!(
        job.control_command(CodingCommand::Pause {
            id: "wrong-job".into(),
            agent: "child-a".into(),
        })
        .await,
        Err(CodingError::NotOwned)
    ));
    assert!(matches!(
        job.control_command(CodingCommand::Pause {
            id: "live-controls".into(),
            agent: "missing-child".into(),
        })
        .await,
        Err(CodingError::NotOwned)
    ));
    let pause = job
        .control_command(CodingCommand::Pause {
            id: "live-controls".into(),
            agent: "child-a".into(),
        })
        .await
        .unwrap();
    let ticket = pause["pause_requested"].as_u64().unwrap();
    assert_eq!(pause["acknowledged"], false);
    let wait = job.wait_pause("live-controls", "child-a", ticket);
    tokio::pin!(wait);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut wait)
            .await
            .is_err()
    );
    // The children are held on their first request, so the pause is provably
    // unacknowledged above; releasing them lets child-a reach its first turn
    // boundary, where the pause is acknowledged.
    provider.release();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), &mut wait)
            .await
            .unwrap()
            .unwrap()
    );
    assert_eq!(
        job.control_command(CodingCommand::Continue {
            id: "live-controls".into(),
            agent: "child-a".into(),
        })
        .await
        .unwrap()["continue_requested"],
        true
    );
    assert_eq!(
        job.control_command(CodingCommand::CancelChild {
            id: "live-controls".into(),
            agent: "child-b".into(),
        })
        .await
        .unwrap()["cancel_requested"],
        true
    );
    let report = job.join().await.unwrap();
    let child_a = report
        .outcomes
        .iter()
        .find(|outcome| outcome.agent_id.as_str() == "child-a")
        .expect("child-a outcome is retained after join");
    assert!(child_a.result.is_ok());
    assert!(child_a.completion_sequence.is_some());
    let child_b = report
        .outcomes
        .iter()
        .find(|outcome| outcome.agent_id.as_str() == "child-b")
        .expect("child-b outcome is retained after join");
    assert!(matches!(
        &child_b.result,
        Err(ScheduleChildError::Run(ChildRunError::Cancelled {
            reason: saya_agent::CancellationReason::User
        }))
    ));
    // Direct readiness is advisory: the root working checkout has a source
    // generation, but nothing is prepared and nothing is running, so the view
    // names no step — `next_action` is null and the suggestion is `none`. The
    // cancelled child is reported by the child record itself; it is not a
    // failed or uncertain one, so the readiness names no child failure either.
    let status = job.status().await.unwrap();
    assert_eq!(status["readiness"]["state"], "working", "{status}");
    assert!(status["next_action"].is_null(), "{status}");
    assert_eq!(status["readiness"]["candidate_ready"], false, "{status}");
    assert_eq!(status["readiness"]["suggestion"], "none", "{status}");
    assert_eq!(
        status["readiness"]["live_children"],
        serde_json::json!([]),
        "{status}"
    );
    assert_eq!(
        status["readiness"]["child_failures"],
        serde_json::json!([]),
        "a cancelled child is not a failure: {status}"
    );
    let cancelled = status["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|child| child["agent_id"] == "child-b")
        .expect("child-b is on record after the join");
    assert_eq!(cancelled["state"], "cancelled");
}

fn task(agent: &str, path: &str) -> CodingTask {
    CodingTask {
        agent_id: AgentId::parse(agent).unwrap(),
        subagent_type: "general".into(),
        task: format!("write for {agent}"),
        write_paths: vec![path.into()],
        dependencies: vec![],
    }
}
