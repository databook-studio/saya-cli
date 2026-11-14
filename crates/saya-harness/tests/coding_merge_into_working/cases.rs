//! Merge-into-working contracts: a disjoint child delta merges into W and
//! keeps the root's edit, a child-path conflict refuses fail-closed with the
//! outcome journaled terminal, and an agent-owned checkout is the wrong
//! target.

use saya_agent::CancellationToken;
use saya_harness::coding::git::IgnoredPaths;
use saya_harness::coding::integration::{
    AcceptedDelta, ApplyError, DEFAULT_DELTA_DEADLINE, IntegrateError, MergeError, MergeInput,
    ReceiptOutcome, candidate_source_digest, compute_delta, evaluate_receipt, merge_into_working,
};
use saya_types::{CheckoutOperationState, RepositoryMutationAction};
use std::fs;

use super::fixture::{Pair, accepted_for, assigned, owned_checkout, pair};

/// The borrowed handles for one merge into the pair's working checkout W,
/// guarded by the ORIGINAL baseline W was materialized from.
fn merge_input<'a>(t: &'a Pair, accepted: &'a AcceptedDelta<'a>) -> MergeInput<'a> {
    MergeInput {
        store: &t.fixture.store,
        sequencer: &t.fixture.sequencer,
        service: &t.fixture.service,
        job_id: &t.fixture.job,
        working: &t.working.verified,
        baseline: &t.working.snapshot,
        accepted,
    }
}

#[tokio::test]
async fn a_child_delta_merges_into_a_working_checkout_that_has_disjoint_root_edits() {
    let t = pair("h1-merge-disjoint").await;
    // The root edits X in W; the child's delta touches only Y.
    fs::write(t.working.root.join("tracked.txt"), b"root edited tracked\n").unwrap();
    fs::write(t.child.root.join("child-y.txt"), b"child bytes\n").unwrap();
    let token = CancellationToken::new();
    let delta = compute_delta(
        &t.child.verified,
        &t.child.snapshot,
        &IgnoredPaths::none(),
        &token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    let receipt = evaluate_receipt(
        &t.child.verified,
        &delta,
        &assigned(&["child-y.txt"]),
        u64::MAX,
    );
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = accepted_for(&t.child, &delta, &receipt);
    let input = merge_input(&t, &accepted);
    let result = merge_into_working(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .unwrap();
    assert_eq!(result.applied(), 1);
    // The root's edit survives untouched and the child's file lands as-is.
    assert_eq!(
        fs::read_to_string(t.working.root.join("tracked.txt")).unwrap(),
        "root edited tracked\n"
    );
    assert_eq!(
        fs::read_to_string(t.working.root.join("child-y.txt")).unwrap(),
        "child bytes\n"
    );
    let digest = candidate_source_digest(
        &t.working.root,
        &IgnoredPaths::none(),
        &token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_eq!(result.source_sha256().as_str(), digest.as_str());
}

#[tokio::test]
async fn a_merge_refuses_when_the_working_checkout_changed_a_child_path() {
    let t = pair("h1-merge-conflict").await;
    // W and the child both change X; the merge must refuse.
    fs::write(t.working.root.join("tracked.txt"), b"root tracked\n").unwrap();
    fs::write(t.child.root.join("tracked.txt"), b"child tracked\n").unwrap();
    let token = CancellationToken::new();
    let delta = compute_delta(
        &t.child.verified,
        &t.child.snapshot,
        &IgnoredPaths::none(),
        &token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    let receipt = evaluate_receipt(
        &t.child.verified,
        &delta,
        &assigned(&["tracked.txt"]),
        u64::MAX,
    );
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = accepted_for(&t.child, &delta, &receipt);
    let input = merge_input(&t, &accepted);
    let err = merge_into_working(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .err()
        .unwrap();
    assert!(matches!(
        err,
        MergeError::Integrate(IntegrateError::Apply(ApplyError::Fs))
    ));
    // Nothing from the child is applied: the root's edit is intact.
    assert_eq!(
        fs::read_to_string(t.working.root.join("tracked.txt")).unwrap(),
        "root tracked\n"
    );
    // The journaled outcome for W is terminal, never an open intent.
    let record = t
        .fixture
        .record(t.working.verified.checkout_id())
        .await
        .unwrap();
    assert_eq!(
        record.latest_operation.state(),
        CheckoutOperationState::Failed
    );
    assert_eq!(
        record.latest_operation.action(),
        RepositoryMutationAction::IntegrateCandidate
    );
}

#[tokio::test]
async fn an_agent_owned_checkout_is_refused_as_a_working_target() {
    let t = owned_checkout("h1-merge-owned", "child-owner").await;
    let token = CancellationToken::new();
    // An unchanged child delta is enough to prove the refusal happens before
    // any delta work or journaling.
    let delta = compute_delta(
        &t.checkout.verified,
        &t.checkout.snapshot,
        &IgnoredPaths::none(),
        &token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_eq!(delta.len(), 0);
    let receipt = evaluate_receipt(
        &t.checkout.verified,
        &delta,
        &assigned(&["tracked.txt"]),
        u64::MAX,
    );
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = accepted_for(&t.checkout, &delta, &receipt);
    let input = MergeInput {
        store: &t.fixture.store,
        sequencer: &t.fixture.sequencer,
        service: &t.fixture.service,
        job_id: &t.fixture.job,
        working: &t.checkout.verified,
        baseline: &t.checkout.snapshot,
        accepted: &accepted,
    };
    let err = merge_into_working(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, MergeError::WorkingCheckoutOwned));
    // The refusal journals nothing: the latest operation is still the create.
    let record = t
        .fixture
        .record(t.checkout.verified.checkout_id())
        .await
        .unwrap();
    assert_eq!(
        record.latest_operation.action(),
        RepositoryMutationAction::CreateCheckout
    );
}
