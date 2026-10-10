//! W09 conflict retention tests: the queue retains both blocking inputs by
//! identity and digests only, never resolves on its own, and fails closed.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, ChangeReceipt, CheckoutDelta, ConflictDecision, ConflictError, ConflictQueue,
    ConflictResolution, DEFAULT_DELTA_DEADLINE, IntegrateError, IntegrateInput,
    MAX_RETAINED_CONFLICTS, OverlapConflict, ReceiptOutcome, candidate_source_digest,
    compute_delta, detect_overlaps, evaluate_receipt, integrate_candidate,
};
use saya_types::ManagedCheckoutId;
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

/// Two children that both rewrite `tracked.txt` differently: the accepted
/// slice blocks `integrate_candidate` with a real overlap conflict.
fn overlapping_deltas(
    t: &Trio,
    token: &CancellationToken,
) -> (CheckoutDelta, CheckoutDelta, ChangeReceipt, ChangeReceipt) {
    let paths = ["tracked.txt"];
    fs::write(t.a.root.join("tracked.txt"), b"alpha\n").unwrap();
    fs::write(t.b.root.join("tracked.txt"), b"beta\n").unwrap();
    let a_delta =
        compute_delta(&t.a.verified, &t.a.snapshot, token, DEFAULT_DELTA_DEADLINE).unwrap();
    let b_delta =
        compute_delta(&t.b.verified, &t.b.snapshot, token, DEFAULT_DELTA_DEADLINE).unwrap();
    let a_receipt = evaluate_receipt(&t.a.verified, &a_delta, &assigned(&paths), u64::MAX);
    let b_receipt = evaluate_receipt(&t.b.verified, &b_delta, &assigned(&paths), u64::MAX);
    assert_eq!(a_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    assert_eq!(b_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    (a_delta, b_delta, a_receipt, b_receipt)
}

/// Runs `integrate_candidate` and unwraps the retained overlap conflict.
async fn overlap_conflict(input: IntegrateInput<'_>, token: &CancellationToken) -> OverlapConflict {
    let err = integrate_candidate(&input, token, DEFAULT_DELTA_DEADLINE)
        .await
        .err()
        .expect("the overlapping slice must be rejected");
    match err {
        IntegrateError::Overlap(conflict) => conflict,
        _ => panic!("expected an overlap conflict"),
    }
}

#[tokio::test]
async fn conflict_retains_both_inputs_and_blocks_apply() {
    let label = "w09-conf-retain";
    let t = trio(label).await;
    let token = CancellationToken::new();
    let store = &t.fixture.store;
    let job = &t.fixture.job;
    let (a_delta, b_delta, a_receipt, b_receipt) = overlapping_deltas(&t, &token);
    let accepted = [
        accepted_for(&t.a, &a_delta, &a_receipt),
        accepted_for(&t.b, &b_delta, &b_receipt),
    ];
    let digest_before =
        candidate_source_digest(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let operations_before = store.count_checkout_operations(job).await.unwrap();
    let conflict = overlap_conflict(integrate_input(&t, &accepted), &token).await;
    let candidate = candidate_id(label);
    let mut queue = ConflictQueue::new();
    queue.record(&candidate, &accepted, &conflict).unwrap();
    let retained = queue.open_for(&candidate).expect("the conflict is open");
    assert_eq!(retained.candidate_checkout_id(), &candidate);
    let inputs = retained.inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].checkout_id(), t.a.verified.checkout_id());
    assert_eq!(inputs[1].checkout_id(), t.b.verified.checkout_id());
    let a_deltas = a_delta.entries().iter().map(|e| e.content_sha256());
    let b_deltas = b_delta.entries().iter().map(|e| e.content_sha256());
    let a_kept = inputs[0].content_sha256s().iter().map(Option::as_ref);
    let b_kept = inputs[1].content_sha256s().iter().map(Option::as_ref);
    assert!(a_deltas.eq(a_kept));
    assert!(b_deltas.eq(b_kept));
    let pairs = retained.pairs();
    assert_eq!(pairs.len(), conflict.pairs().len());
    assert_eq!(pairs[0].first(), 0);
    assert_eq!(pairs[0].second(), 1);
    assert_eq!(pairs[0].path(), "tracked.txt");
    let digest_after =
        candidate_source_digest(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(digest_after.as_str(), digest_before.as_str());
    assert_eq!(
        store.count_checkout_operations(job).await.unwrap(),
        operations_before
    );
}

#[tokio::test]
async fn resolution_keep_first_allows_resubmission() {
    let label = "w09-conf-keepfirst";
    let t = trio(label).await;
    let token = CancellationToken::new();
    let (a_delta, b_delta, a_receipt, b_receipt) = overlapping_deltas(&t, &token);
    let accepted = [
        accepted_for(&t.a, &a_delta, &a_receipt),
        accepted_for(&t.b, &b_delta, &b_receipt),
    ];
    let conflict = overlap_conflict(integrate_input(&t, &accepted), &token).await;
    let candidate = candidate_id(label);
    let mut queue = ConflictQueue::new();
    queue.record(&candidate, &accepted, &conflict).unwrap();
    let resolution = queue
        .resolve(&candidate, ConflictDecision::KeepFirst)
        .unwrap();
    assert_eq!(resolution, ConflictResolution::KeptIndexes(vec![0]));
    let kept = [accepted_for(&t.a, &a_delta, &a_receipt)];
    integrate_candidate(&integrate_input(&t, &kept), &token, DEFAULT_DELTA_DEADLINE)
        .await
        .unwrap();
    assert!(queue.open_for(&candidate).is_none());
}

#[tokio::test]
async fn no_automatic_resolution() {
    let label = "w09-conf-noauto";
    let t = trio(label).await;
    let token = CancellationToken::new();
    let (a_delta, b_delta, a_receipt, b_receipt) = overlapping_deltas(&t, &token);
    let accepted = [
        accepted_for(&t.a, &a_delta, &a_receipt),
        accepted_for(&t.b, &b_delta, &b_receipt),
    ];
    let conflict = overlap_conflict(integrate_input(&t, &accepted), &token).await;
    let candidate = candidate_id(label);
    let mut queue = ConflictQueue::new();
    queue.record(&candidate, &accepted, &conflict).unwrap();
    assert!(queue.open_for(&candidate).is_some());
    let input = integrate_input(&t, &accepted);
    let err = integrate_candidate(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .err()
        .expect("must still be rejected");
    assert!(matches!(err, IntegrateError::Overlap(_)));
    assert!(queue.open_for(&candidate).is_some());
}

#[tokio::test]
async fn manual_edit_routes_to_fresh_submission() {
    let label = "w09-conf-manual";
    let t = trio(label).await;
    let token = CancellationToken::new();
    let (a_delta, b_delta, a_receipt, b_receipt) = overlapping_deltas(&t, &token);
    let accepted = [
        accepted_for(&t.a, &a_delta, &a_receipt),
        accepted_for(&t.b, &b_delta, &b_receipt),
    ];
    let conflict = overlap_conflict(integrate_input(&t, &accepted), &token).await;
    let candidate = candidate_id(label);
    let mut queue = ConflictQueue::new();
    queue.record(&candidate, &accepted, &conflict).unwrap();
    let resolution = queue
        .resolve(&candidate, ConflictDecision::ManualEdit)
        .unwrap();
    assert_eq!(resolution, ConflictResolution::ManualEdit);
    assert!(queue.open_for(&candidate).is_none());
    fs::write(t.a.root.join("tracked.txt"), b"gamma\n").unwrap();
    let edited =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(
        &t.a.verified,
        &edited,
        &assigned(&["tracked.txt"]),
        u64::MAX,
    );
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let fresh = [accepted_for(&t.a, &edited, &receipt)];
    integrate_candidate(&integrate_input(&t, &fresh), &token, DEFAULT_DELTA_DEADLINE)
        .await
        .unwrap();
}

#[tokio::test]
async fn queue_is_bounded_and_resolution_errors_fail_closed() {
    let label = "w09-conf-bounds";
    let t = trio(label).await;
    let token = CancellationToken::new();
    let (a_delta, b_delta, a_receipt, b_receipt) = overlapping_deltas(&t, &token);
    let accepted = [
        accepted_for(&t.a, &a_delta, &a_receipt),
        accepted_for(&t.b, &b_delta, &b_receipt),
    ];
    let conflict = detect_overlaps(&accepted).err().expect("expected overlap");
    let mut queue = ConflictQueue::new();
    for i in 0..MAX_RETAINED_CONFLICTS {
        let id = ManagedCheckoutId::parse(&format!("cand-{i:02}")).unwrap();
        queue.record(&id, &accepted, &conflict).unwrap();
    }
    let overflow = ManagedCheckoutId::parse("cand-64").unwrap();
    assert!(matches!(
        queue.record(&overflow, &accepted, &conflict),
        Err(ConflictError::QueueFull)
    ));
    let unknown = ManagedCheckoutId::parse(&format!("{label}-unknown")).unwrap();
    assert!(matches!(
        queue.resolve(&unknown, ConflictDecision::KeepFirst),
        Err(ConflictError::NotOpen)
    ));
    let first = ManagedCheckoutId::parse("cand-00").unwrap();
    queue.resolve(&first, ConflictDecision::KeepFirst).unwrap();
    assert!(matches!(
        queue.resolve(&first, ConflictDecision::KeepSecond),
        Err(ConflictError::AlreadyResolved)
    ));
    let chained = [
        accepted_for(&t.a, &a_delta, &a_receipt),
        accepted_for(&t.b, &b_delta, &b_receipt),
        accepted_for(&t.a, &a_delta, &a_receipt),
    ];
    let chained_conflict = detect_overlaps(&chained).err().expect("expected overlap");
    let mut chain_queue = ConflictQueue::new();
    let chain_candidate = ManagedCheckoutId::parse(&format!("{label}-chain")).unwrap();
    chain_queue
        .record(&chain_candidate, &chained, &chained_conflict)
        .unwrap();
    assert!(matches!(
        chain_queue.resolve(&chain_candidate, ConflictDecision::KeepFirst),
        Err(ConflictError::AmbiguousDecision)
    ));
}
