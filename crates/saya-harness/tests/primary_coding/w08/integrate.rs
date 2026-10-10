//! W08c integrate tests: one accepted child delta journals a single intent
//! and a terminal outcome; overlaps, pre-cancelled runs, and child drift
//! record nothing or a `Failed` outcome exactly per the outcome policy.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, DEFAULT_DELTA_DEADLINE, IntegrateError, IntegrateInput, ReceiptOutcome,
    candidate_source_digest, compute_delta, evaluate_receipt, integrate_candidate,
};
use saya_types::{
    CheckoutOperationState, ManagedCheckoutId, ManagedCheckoutLifecycle, RepositoryMutationAction,
};
use std::fs;

use super::apply_support::{Trio, accepted_for, assigned, trio};

/// The borrowed handles for one `integrate_candidate` run over the trio's
/// candidate, with no owning agent (matching the fixture's ownership).
fn integrate_input<'a>(t: &'a Trio, accepted: &'a [AcceptedDelta<'a>]) -> IntegrateInput<'a> {
    IntegrateInput {
        store: &t.fixture.store,
        sequencer: &t.fixture.sequencer,
        job_id: &t.fixture.job,
        agent_id: None,
        candidate: &t.candidate.verified,
        baseline: &t.candidate.snapshot,
        accepted,
    }
}

/// The candidate checkout id for a trio labeled `label` (suffix `c`).
fn candidate_id(label: &str) -> ManagedCheckoutId {
    ManagedCheckoutId::parse(&format!("{label}-c")).unwrap()
}

#[tokio::test]
async fn integrate_journals_intent_and_single_outcome() {
    let label = "w08-int-journal";
    let t = trio(label).await;
    let a = &t.a;
    fs::create_dir_all(a.root.join("notes")).unwrap();
    fs::write(a.root.join("notes/new.txt"), b"new bytes\n").unwrap();
    let token = CancellationToken::new();
    let delta = compute_delta(&a.verified, &a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["notes"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = [accepted_for(a, &delta, &receipt)];
    let input = integrate_input(&t, &accepted);
    let result = integrate_candidate(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .unwrap();
    assert_eq!(result.applied(), 1);
    assert_eq!(
        fs::read_to_string(t.candidate.root.join("notes/new.txt")).unwrap(),
        "new bytes\n"
    );
    let expected =
        candidate_source_digest(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(result.source_sha256().as_str(), expected.as_str());
    let record = t
        .fixture
        .record(&candidate_id(label))
        .await
        .expect("candidate record");
    assert!(matches!(
        record.latest_operation.action(),
        RepositoryMutationAction::IntegrateCandidate
    ));
    assert!(matches!(
        record.latest_operation.state(),
        CheckoutOperationState::Completed
    ));
    assert!(matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Retained
    ));
}

#[tokio::test]
async fn integrate_overlap_records_nothing() {
    let label = "w08-int-overlap";
    let t = trio(label).await;
    let token = CancellationToken::new();
    let paths = ["tracked.txt"];
    fs::write(t.a.root.join("tracked.txt"), b"alpha\n").unwrap();
    fs::write(t.b.root.join("tracked.txt"), b"beta\n").unwrap();
    let a_delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let b_delta =
        compute_delta(&t.b.verified, &t.b.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let a_receipt = evaluate_receipt(&t.a.verified, &a_delta, &assigned(&paths), u64::MAX);
    let b_receipt = evaluate_receipt(&t.b.verified, &b_delta, &assigned(&paths), u64::MAX);
    assert_eq!(a_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    assert_eq!(b_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = [
        accepted_for(&t.a, &a_delta, &a_receipt),
        accepted_for(&t.b, &b_delta, &b_receipt),
    ];
    let before = fs::read(t.candidate.root.join("tracked.txt")).unwrap();
    let input = integrate_input(&t, &accepted);
    let err = integrate_candidate(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, IntegrateError::Overlap(_)));
    let record = t
        .fixture
        .record(&candidate_id(label))
        .await
        .expect("candidate record");
    assert!(matches!(
        record.latest_operation.action(),
        RepositoryMutationAction::CreateCheckout
    ));
    assert!(matches!(
        record.latest_operation.state(),
        CheckoutOperationState::Completed
    ));
    assert_eq!(
        fs::read(t.candidate.root.join("tracked.txt")).unwrap(),
        before
    );
}

#[tokio::test]
async fn cancelled_before_intent_records_nothing() {
    let label = "w08-int-cancel";
    let t = trio(label).await;
    let token = CancellationToken::new();
    fs::create_dir_all(t.a.root.join("notes")).unwrap();
    fs::write(t.a.root.join("notes/new.txt"), b"new bytes\n").unwrap();
    let delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&t.a.verified, &delta, &assigned(&["notes"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let input = integrate_input(&t, &accepted);
    let err = integrate_candidate(&input, &cancelled, DEFAULT_DELTA_DEADLINE)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, IntegrateError::Cancelled));
    let record = t
        .fixture
        .record(&candidate_id(label))
        .await
        .expect("candidate record");
    assert!(matches!(
        record.latest_operation.action(),
        RepositoryMutationAction::CreateCheckout
    ));
    assert!(matches!(
        record.latest_operation.state(),
        CheckoutOperationState::Completed
    ));
}

#[tokio::test]
async fn apply_drift_records_failed_outcome() {
    let label = "w08-int-drift";
    let t = trio(label).await;
    let a = &t.a;
    fs::write(a.root.join("drift.txt"), b"aaaa").unwrap();
    let token = CancellationToken::new();
    let delta = compute_delta(&a.verified, &a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["drift.txt"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    fs::write(a.root.join("drift.txt"), b"bbbb").unwrap();
    let accepted = [accepted_for(a, &delta, &receipt)];
    let input = integrate_input(&t, &accepted);
    let err = integrate_candidate(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, IntegrateError::Apply(_)));
    assert!(!t.candidate.root.join("drift.txt").exists());
    let record = t
        .fixture
        .record(&candidate_id(label))
        .await
        .expect("candidate record");
    assert!(matches!(
        record.latest_operation.state(),
        CheckoutOperationState::Failed
    ));
    assert!(matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Retained
    ));
}
