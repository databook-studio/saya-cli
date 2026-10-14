//! W08d proof tests: two disjoint child results integrate through
//! `integrate_candidate` without mutating the original repository, plus
//! hidden additions, rename endpoints, and unowned rename endpoints.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, ApplyError, DEFAULT_DELTA_DEADLINE, IntegrateInput, ReceiptOutcome,
    candidate_source_digest, compute_delta, evaluate_receipt, integrate_candidate,
};
use std::fs;
use std::path::Path;

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

/// The sorted top-level entry names of `root`.
fn top_level_listing(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn two_disjoint_results_integrate_without_original_mutation() {
    let label = "w08-proof-disjoint";
    let t = trio(label).await;
    let status_before = t
        .fixture
        .git_output(&t.fixture.repo_root, &["status", "--porcelain"]);
    let head_before = t
        .fixture
        .git_output(&t.fixture.repo_root, &["rev-parse", "HEAD"]);
    let tracked_before = fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap();
    let one_before = fs::read(t.fixture.repo_root.join("a/one.txt")).unwrap();
    let listing_before = top_level_listing(&t.fixture.repo_root);
    fs::write(t.a.root.join("tracked.txt"), b"alpha\n").unwrap();
    fs::create_dir_all(t.b.root.join("notes")).unwrap();
    fs::write(t.b.root.join("notes/b.txt"), b"b bytes\n").unwrap();
    let token = CancellationToken::new();
    let a_delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let b_delta =
        compute_delta(&t.b.verified, &t.b.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let paths = assigned(&["tracked.txt", "notes"]);
    let a_receipt = evaluate_receipt(&t.a.verified, &a_delta, &paths, u64::MAX);
    let b_receipt = evaluate_receipt(&t.b.verified, &b_delta, &paths, u64::MAX);
    assert_eq!(a_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    assert_eq!(b_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = [
        accepted_for(&t.a, &a_delta, &a_receipt),
        accepted_for(&t.b, &b_delta, &b_receipt),
    ];
    let input = integrate_input(&t, &accepted);
    let result = integrate_candidate(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .unwrap();
    assert_eq!(result.applied(), 2);
    assert_eq!(
        fs::read_to_string(t.candidate.root.join("tracked.txt")).unwrap(),
        "alpha\n"
    );
    assert_eq!(
        fs::read_to_string(t.candidate.root.join("notes/b.txt")).unwrap(),
        "b bytes\n"
    );
    let digest =
        candidate_source_digest(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(result.source_sha256().as_str(), digest.as_str());
    let digest_again =
        candidate_source_digest(&t.candidate.root, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(digest.as_str(), digest_again.as_str());
    assert_eq!(
        t.fixture
            .git_output(&t.fixture.repo_root, &["status", "--porcelain"]),
        status_before
    );
    assert_eq!(
        t.fixture
            .git_output(&t.fixture.repo_root, &["rev-parse", "HEAD"]),
        head_before
    );
    assert_eq!(
        fs::read(t.fixture.repo_root.join("tracked.txt")).unwrap(),
        tracked_before
    );
    assert_eq!(
        fs::read(t.fixture.repo_root.join("a/one.txt")).unwrap(),
        one_before
    );
    assert_eq!(top_level_listing(&t.fixture.repo_root), listing_before);
    assert!(!t.candidate.root.starts_with(&t.fixture.repo_root));
    assert_eq!(t.candidate.root.parent(), t.a.root.parent());
}

#[tokio::test]
async fn hidden_and_omitted_added_files_integrate() {
    let label = "w08-proof-hidden";
    let t = trio(label).await;
    let a = &t.a;
    fs::write(a.root.join(".hidden-note"), b"hidden\n").unwrap();
    fs::create_dir_all(a.root.join("notes")).unwrap();
    fs::write(a.root.join("notes/new.txt"), b"new bytes\n").unwrap();
    let token = CancellationToken::new();
    let delta = compute_delta(&a.verified, &a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let paths = assigned(&[".hidden-note", "notes"]);
    let receipt = evaluate_receipt(&a.verified, &delta, &paths, u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = [accepted_for(a, &delta, &receipt)];
    let input = integrate_input(&t, &accepted);
    let result = integrate_candidate(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .unwrap();
    assert_eq!(result.applied(), 1);
    assert_eq!(
        fs::read_to_string(t.candidate.root.join(".hidden-note")).unwrap(),
        "hidden\n"
    );
    assert_eq!(
        fs::read_to_string(t.candidate.root.join("notes/new.txt")).unwrap(),
        "new bytes\n"
    );
}

#[tokio::test]
async fn renamed_endpoints_apply_as_delete_plus_add() {
    let label = "w08-proof-rename";
    let t = trio(label).await;
    let a = &t.a;
    fs::create_dir_all(a.root.join("moved")).unwrap();
    fs::rename(a.root.join("a/one.txt"), a.root.join("moved/one.txt")).unwrap();
    let token = CancellationToken::new();
    let delta = compute_delta(&a.verified, &a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["a", "moved"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = [accepted_for(a, &delta, &receipt)];
    let input = integrate_input(&t, &accepted);
    let result = integrate_candidate(&input, &token, DEFAULT_DELTA_DEADLINE)
        .await
        .unwrap();
    assert_eq!(result.applied(), 1);
    assert!(!t.candidate.root.join("a/one.txt").exists());
    assert_eq!(
        fs::read_to_string(t.candidate.root.join("moved/one.txt")).unwrap(),
        "nested one bytes\n"
    );
}

#[tokio::test]
async fn unowned_rename_endpoint_never_reaches_the_candidate() {
    let label = "w08-proof-unowned";
    let t = trio(label).await;
    let a = &t.a;
    fs::create_dir_all(a.root.join("other")).unwrap();
    fs::rename(a.root.join("a/one.txt"), a.root.join("other/one.txt")).unwrap();
    let token = CancellationToken::new();
    let delta = compute_delta(&a.verified, &a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["a"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::OutsideAssignment);
    let err = AcceptedDelta::new(&a.verified, &receipt, &delta)
        .err()
        .unwrap();
    assert!(matches!(err, ApplyError::ReceiptRejected));
    assert!(t.candidate.root.join("a/one.txt").exists());
    assert!(!t.candidate.root.join("other").exists());
}
