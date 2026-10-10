//! W08b apply tests: one accepted child delta lands in the candidate, drift
//! and oversize fail closed, and overlaps are refused before any write.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, ApplyError, DEFAULT_DELTA_DEADLINE, ReceiptOutcome, apply_delta, compute_delta,
    detect_overlaps, evaluate_receipt,
};
use std::{fs, os::unix::fs::PermissionsExt};

use super::apply_support::{accepted_for, assigned, trio};

fn token() -> CancellationToken {
    CancellationToken::new()
}

#[tokio::test]
async fn apply_added_modified_deleted_roundtrip() {
    let t = trio("w08-apply-roundtrip").await;
    let a = &t.a;
    fs::create_dir_all(a.root.join("notes")).unwrap();
    fs::write(a.root.join("notes/new.txt"), b"new bytes\n").unwrap();
    let mut permissions = fs::metadata(a.root.join("notes/new.txt"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(a.root.join("notes/new.txt"), permissions).unwrap();
    fs::write(a.root.join("tracked.txt"), b"changed\n").unwrap();
    fs::remove_file(a.root.join("a/one.txt")).unwrap();

    let token = token();
    let delta = compute_delta(&a.verified, &a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(
        &a.verified,
        &delta,
        &assigned(&["notes", "tracked.txt", "a"]),
        u64::MAX,
    );
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted = accepted_for(a, &delta, &receipt);

    let c = &t.candidate;
    apply_delta(&c.root, &accepted, &c.snapshot, &token).unwrap();

    assert_eq!(
        fs::read_to_string(c.root.join("notes/new.txt")).unwrap(),
        "new bytes\n"
    );
    let mode = fs::metadata(c.root.join("notes/new.txt"))
        .unwrap()
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0);
    assert_eq!(
        fs::read_to_string(c.root.join("tracked.txt")).unwrap(),
        "changed\n"
    );
    assert!(!c.root.join("a/one.txt").exists());

    let candidate_delta =
        compute_delta(&c.verified, &c.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(candidate_delta.len(), delta.len());
    for (applied, expected) in candidate_delta.entries().iter().zip(delta.entries()) {
        assert_eq!(applied.path(), expected.path());
        assert_eq!(applied.kind(), expected.kind());
        assert_eq!(
            applied.content_sha256().map(|d| d.as_str()),
            expected.content_sha256().map(|d| d.as_str()),
        );
    }
}

#[tokio::test]
async fn apply_rejects_child_drift() {
    let t = trio("w08-apply-drift").await;
    let a = &t.a;
    fs::write(a.root.join("drift.txt"), b"aaaa").unwrap();
    let token = token();
    let delta = compute_delta(&a.verified, &a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["drift.txt"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);

    fs::write(a.root.join("drift.txt"), b"bbbb").unwrap();
    let accepted = accepted_for(a, &delta, &receipt);
    let c = &t.candidate;
    let err = apply_delta(&c.root, &accepted, &c.snapshot, &token)
        .err()
        .unwrap();
    assert!(matches!(err, ApplyError::ContentMismatch));
    assert!(!c.root.join("drift.txt").exists());
}

#[tokio::test]
async fn overlapping_rewrites_of_one_path_conflict() {
    let t = trio("w08-ovl-same").await;
    let token = token();
    let paths = ["notes", "tracked.txt", "a"];

    fs::write(t.a.root.join("tracked.txt"), b"alpha\n").unwrap();
    fs::write(t.b.root.join("tracked.txt"), b"beta\n").unwrap();
    let a_delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let b_delta =
        compute_delta(&t.b.verified, &t.b.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let a_receipt = evaluate_receipt(&t.a.verified, &a_delta, &assigned(&paths), u64::MAX);
    let b_receipt = evaluate_receipt(&t.b.verified, &b_delta, &assigned(&paths), u64::MAX);
    let accepted_a = accepted_for(&t.a, &a_delta, &a_receipt);
    let accepted_b = accepted_for(&t.b, &b_delta, &b_receipt);
    let conflict = detect_overlaps(&[accepted_a, accepted_b]).err().unwrap();
    let pairs = conflict.pairs();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].first(), 0);
    assert_eq!(pairs[0].second(), 1);
    assert_eq!(pairs[0].path(), "tracked.txt");
}

#[tokio::test]
async fn disjoint_paths_do_not_overlap() {
    let t = trio("w08-ovl-disjoint").await;
    let token = token();
    let paths = ["notes", "tracked.txt", "a"];

    fs::write(t.a.root.join("tracked.txt"), b"alpha\n").unwrap();
    fs::create_dir_all(t.b.root.join("notes")).unwrap();
    fs::write(t.b.root.join("notes/b.txt"), b"b bytes\n").unwrap();
    let a_delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let b_delta =
        compute_delta(&t.b.verified, &t.b.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let a_receipt = evaluate_receipt(&t.a.verified, &a_delta, &assigned(&paths), u64::MAX);
    let b_receipt = evaluate_receipt(&t.b.verified, &b_delta, &assigned(&paths), u64::MAX);
    let accepted_a = accepted_for(&t.a, &a_delta, &a_receipt);
    let accepted_b = accepted_for(&t.b, &b_delta, &b_receipt);
    assert!(detect_overlaps(&[accepted_a, accepted_b]).is_ok());
}

#[tokio::test]
async fn sibling_paths_in_one_directory_do_not_overlap() {
    let t = trio("w08-ovl-sibling").await;
    let token = token();
    let paths = ["a"];

    fs::remove_file(t.a.root.join("a/one.txt")).unwrap();
    fs::write(t.b.root.join("a/two.txt"), b"two bytes\n").unwrap();
    let a_delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let b_delta =
        compute_delta(&t.b.verified, &t.b.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let a_receipt = evaluate_receipt(&t.a.verified, &a_delta, &assigned(&paths), u64::MAX);
    let b_receipt = evaluate_receipt(&t.b.verified, &b_delta, &assigned(&paths), u64::MAX);
    let accepted_a = accepted_for(&t.a, &a_delta, &a_receipt);
    let accepted_b = accepted_for(&t.b, &b_delta, &b_receipt);
    assert!(detect_overlaps(&[accepted_a, accepted_b]).is_ok());
}

#[tokio::test]
async fn ancestor_path_conflict_is_detected() {
    let t = trio("w08-ovl-ancestor").await;
    let token = token();
    let paths = ["a"];

    fs::remove_file(t.a.root.join("a/one.txt")).unwrap();
    fs::remove_dir(t.a.root.join("a")).unwrap();
    fs::write(t.a.root.join("a"), b"now a file\n").unwrap();
    fs::write(t.b.root.join("a/two.txt"), b"two bytes\n").unwrap();
    let a_delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let b_delta =
        compute_delta(&t.b.verified, &t.b.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let a_receipt = evaluate_receipt(&t.a.verified, &a_delta, &assigned(&paths), u64::MAX);
    let b_receipt = evaluate_receipt(&t.b.verified, &b_delta, &assigned(&paths), u64::MAX);
    assert_eq!(a_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    assert_eq!(b_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let accepted_a = accepted_for(&t.a, &a_delta, &a_receipt);
    let accepted_b = accepted_for(&t.b, &b_delta, &b_receipt);
    let conflict = detect_overlaps(&[accepted_a, accepted_b]).err().unwrap();
    let pairs = conflict.pairs();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].first(), 0);
    assert_eq!(pairs[0].second(), 1);
    assert_eq!(pairs[0].path(), "a/two.txt");
}

#[tokio::test]
async fn oversize_delta_refuses() {
    let t = trio("w08-apply-oversize").await;
    let a = &t.a;
    fs::File::create(a.root.join("big.bin"))
        .unwrap()
        .set_len(9 * 1024 * 1024)
        .unwrap();
    let token = token();
    let delta = compute_delta(&a.verified, &a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["big.bin"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);

    let accepted = accepted_for(a, &delta, &receipt);
    let c = &t.candidate;
    let err = apply_delta(&c.root, &accepted, &c.snapshot, &token)
        .err()
        .unwrap();
    assert!(matches!(err, ApplyError::Oversize));
    assert!(!c.root.join("big.bin").exists());
}

#[tokio::test]
async fn accepted_delta_requires_a_within_assignment_receipt() {
    let t = trio("w08-apply-receipt").await;
    let token = token();

    fs::write(t.a.root.join("x.txt"), b"x bytes\n").unwrap();
    let a_delta =
        compute_delta(&t.a.verified, &t.a.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let rejected = evaluate_receipt(&t.a.verified, &a_delta, &assigned(&["other"]), u64::MAX);
    assert_eq!(rejected.outcome(), ReceiptOutcome::OutsideAssignment);
    let err = AcceptedDelta::new(&t.a.verified, &rejected, &a_delta)
        .err()
        .unwrap();
    assert!(matches!(err, ApplyError::ReceiptRejected));

    fs::write(t.b.root.join("y.txt"), b"y bytes\n").unwrap();
    let b_delta =
        compute_delta(&t.b.verified, &t.b.snapshot, &token, DEFAULT_DELTA_DEADLINE).unwrap();
    let a_receipt = evaluate_receipt(&t.a.verified, &a_delta, &assigned(&["x.txt"]), u64::MAX);
    assert_eq!(a_receipt.outcome(), ReceiptOutcome::WithinAssignment);
    let err = AcceptedDelta::new(&t.a.verified, &a_receipt, &b_delta)
        .err()
        .unwrap();
    assert!(matches!(err, ApplyError::ReceiptMismatch));
}
