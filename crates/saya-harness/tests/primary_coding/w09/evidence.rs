//! W09 evidence binding tests: verify-at-use fails closed — only an exact
//! digest match passes, any edit is stale, and any unprovable tree is
//! `Unverifiable` rather than assumed valid.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, DEFAULT_DELTA_DEADLINE, EvidenceBinding, EvidenceError, IntegrateInput,
    ReceiptOutcome, compute_delta, evaluate_receipt, integrate_candidate,
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

/// Pins the candidate's content distinct from child `a`'s so the exactness
/// check compares genuinely different trees.
fn pin_candidate_content(t: &Trio) {
    fs::write(
        t.candidate.root.join("exact.txt"),
        b"only in the candidate\n",
    )
    .unwrap();
}

#[tokio::test]
async fn conflict_and_edit_invalidate_review() {
    let label = "w09-ev-edit";
    let t = trio(label).await;
    let token = CancellationToken::new();
    fs::create_dir_all(t.a.root.join("notes")).unwrap();
    fs::write(t.a.root.join("notes/new.txt"), b"new bytes\n").unwrap();
    let delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&t.a.verified, &delta, &assigned(&["notes"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = [accepted_for(&t.a, &delta, &receipt)];
    let input = integrate_input(&t, &accepted);
    integrate_candidate(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .unwrap();
    let binding = EvidenceBinding::bind(&t.candidate.verified, &token, DEFAULT_DELTA_DEADLINE)
        .expect("the candidate tree must be provable");
    assert_eq!(
        binding.candidate_checkout_id(),
        t.candidate.verified.checkout_id()
    );
    binding
        .verify_at_use(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE)
        .expect("an unchanged tree must verify");
    fs::write(t.candidate.root.join("tracked.txt"), b"edited\n").unwrap();
    assert!(matches!(
        binding.verify_at_use(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE),
        Err(EvidenceError::Stale)
    ));
    let rebound = EvidenceBinding::bind(&t.candidate.verified, &token, DEFAULT_DELTA_DEADLINE)
        .expect("the edited tree must still be provable");
    assert_ne!(
        rebound.source_sha256().as_str(),
        binding.source_sha256().as_str()
    );
}

#[tokio::test]
async fn evidence_fails_closed_on_unknown() {
    let label = "w09-ev-unknown";
    let t = trio(label).await;
    let token = CancellationToken::new();
    let binding = EvidenceBinding::bind(&t.candidate.verified, &token, DEFAULT_DELTA_DEADLINE)
        .expect("the candidate tree must be provable");
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        binding.verify_at_use(&t.candidate.root, &cancelled, DEFAULT_DELTA_DEADLINE),
        Err(EvidenceError::Unverifiable(_))
    ));
    fs::remove_dir_all(&t.candidate.root).unwrap();
    assert!(matches!(
        binding.verify_at_use(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE),
        Err(EvidenceError::Unverifiable(_))
    ));
}

#[tokio::test]
async fn binding_is_exact_to_one_candidate() {
    let label = "w09-ev-exact";
    let t = trio(label).await;
    let token = CancellationToken::new();
    pin_candidate_content(&t);
    let binding = EvidenceBinding::bind(&t.candidate.verified, &token, DEFAULT_DELTA_DEADLINE)
        .expect("the candidate tree must be provable");
    assert_eq!(
        binding.candidate_checkout_id(),
        t.candidate.verified.checkout_id()
    );
    assert!(matches!(
        binding.verify_at_use(&t.a.root, &token, DEFAULT_DELTA_DEADLINE),
        Err(EvidenceError::Stale)
    ));
    binding
        .verify_at_use(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE)
        .expect("the bound tree must verify");
}
