#[path = "post_edit_workspace_failure_support.rs"]
mod owner;
#[cfg(target_os = "macos")]
use super::support::jsonl;
use super::support::{Run, record_path};
use owner::run_known_commit_append_failure;
#[cfg(target_os = "macos")]
use owner::run_observer_dual_failure;
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservationValue, CodingEventKind,
    CodingFailureClass, CodingResultOutcome, JobState,
};
use std::fs;

#[cfg(target_os = "macos")]
pub(super) async fn run_stale_after_exchange() -> Run {
    owner::run_stale_after_exchange().await
}

#[tokio::test]
async fn known_commit_receipt_failure_is_fatal_and_preserves_the_committed_write() {
    let run = run_known_commit_append_failure().await;
    let written = fs::read(run.checkout.join("new.txt")).unwrap();

    assert_eq!(written, b"known commit survives observer failure\n");
    assert!(run.outcome.is_err());
    assert!(record_path(&run, "known-commits-").is_dir());
    assert!(!record_path(&run, "extensions-").exists());
    let terminal = run
        .fixture
        .store
        .latest_coding_run_event(&run.job_id)
        .await
        .unwrap()
        .unwrap();
    // Direct mode does not latch a child's own failure class as the job's
    // (`failure.rs`), so the job reports the conservative owner-level class for
    // a child whose effect could not be settled. What survives verbatim is the
    // part that matters here: the run is fatal, the committed write and its
    // strict receipt conflict are intact, and the child's tool effect is still
    // settled conservatively.
    assert!(
        matches!(
            terminal.event(),
            CodingEventKind::RunTerminated {
                state: JobState::Failed,
                failure,
                ..
            } if failure.agent_id.is_none()
        ),
        "terminal: {:?}",
        terminal.event()
    );
    // The child's own class is re-pinned on its durable completion: the failed
    // strict receipt append is the latched cause of the uncertain outcome.
    let completions = run
        .fixture
        .store
        .list_coding_completions(&run.job_id, None, 8)
        .await
        .unwrap();
    let child = completions
        .items
        .iter()
        .find(|item| item.result.outcome() == CodingResultOutcome::Uncertain)
        .unwrap_or_else(|| {
            panic!(
                "the failed known-commit receipt left an uncertain child completion: {:?}",
                completions
                    .items
                    .iter()
                    .map(|item| (item.result.outcome(), item.result.failure_class()))
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        child.result.failure_class(),
        Some(CodingFailureClass::MandatoryPersistence),
        "the failed receipt append is the child's own class: {:?}",
        child.result
    );
    assert_uncertain_child_tool_call(&run).await;
    assert!(
        run.fixture
            .application
            .export(&run.job_id, None, None)
            .await
            .is_err(),
        "an invalid strict known-commit path makes export refuse"
    );
}

// The exchange needs the confined LSP extension process, which only macOS
// proves; Linux fails closed before the process starts.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn exchange_then_observer_failure_keeps_primary_and_secondary_and_exports_commit() {
    let run = run_observer_dual_failure().await;
    let written = fs::read(run.checkout.join("new.txt")).unwrap();
    assert_eq!(written, b"known commit survives observer failure\n");
    assert!(run.checkout.join(".post-edit-exchange").exists());
    assert!(run.outcome.is_err());
    let observer_records = jsonl(&record_path(&run, "extensions-"));
    assert_eq!(observer_records.len(), 2);
    assert_eq!(observer_records[0]["fact"]["phase"], "spawned");
    assert!(observer_records[1].get("fact").is_none());

    let terminal = run
        .fixture
        .store
        .latest_coding_run_event(&run.job_id)
        .await
        .unwrap()
        .unwrap();
    // The same two classes the delegate-only run reported, with their roles
    // swapped by the direct policy: the persistence failure is the one latched
    // with the child's attribution, and the exchange failure is reported as the
    // child's own class in its record rather than latched as the job's.
    assert!(
        matches!(
            terminal.event(),
            CodingEventKind::RunTerminated {
                failure,
                secondary: Some(CodingFailureClass::EffectsUncertain),
                ..
            } if failure.class == CodingFailureClass::MandatoryPersistence
                && failure.agent_id.is_some()
        ),
        "terminal: {:?}",
        terminal.event()
    );
    assert_uncertain_child_tool_call(&run).await;
    let exported = run
        .fixture
        .application
        .export(&run.job_id, None, None)
        .await
        .unwrap();
    let files = exported["artifacts"]["files"].as_array().unwrap();
    let known = files
        .iter()
        .find(|file| file["name"].as_str().unwrap().starts_with("known-commits-"))
        .unwrap();
    assert_eq!(known["records"].as_array().unwrap().len(), 1);
    assert!(known["records"][0]["path"]["projection"] == "private_relative_path_sha256");
}

async fn assert_uncertain_child_tool_call(run: &Run) {
    let children = run
        .fixture
        .store
        .list_coding_children(&run.job_id, None, 10)
        .await
        .unwrap();
    let child = children
        .items
        .iter()
        .find(|child| child.agent_id.as_str() == "child-post-edit")
        .expect("the failed post-edit owner is a registered child");
    let reservations = run
        .fixture
        .store
        .list_coding_budget_reservations(&run.job_id, None, 100)
        .await
        .unwrap();
    let uncertain = reservations
        .items
        .iter()
        .filter(|record| {
            record.reservation.agent_id() == Some(&child.agent_id)
                && record.reservation.invocation_id() == Some(&child.current_invocation_id)
                && record
                    .reservation
                    .charges()
                    .iter()
                    .any(|charge| charge.dimension() == BudgetDimension::ToolCalls)
                && record.settlement.as_ref().is_some_and(|settlement| {
                    settlement.effect_state() == BudgetEffectState::Uncertain
                })
        })
        .collect::<Vec<_>>();
    assert!(!uncertain.is_empty(), "child tool reservation is present");
    for record in uncertain {
        assert_eq!(record.reservation.charges().len(), 1);
        assert_eq!(
            record.reservation.charges()[0].dimension(),
            BudgetDimension::ToolCalls
        );
        assert_eq!(record.reservation.charges()[0].reserved().get(), 1);
        let settlement = record
            .settlement
            .as_ref()
            .expect("failed tool call settles conservatively");
        assert_eq!(settlement.effect_state(), BudgetEffectState::Uncertain);
        assert_eq!(settlement.observations().len(), 1);
        assert_eq!(
            settlement.observations()[0].dimension(),
            BudgetDimension::ToolCalls
        );
        assert_eq!(
            settlement.observations()[0].value(),
            BudgetObservationValue::Unknown
        );
    }
}
