use std::fs;

use super::delta::{Active, active};
use saya_harness::coding::integration::{
    AssignedPaths, ChangeScope, DEFAULT_DELTA_DEADLINE, DeltaKind, ReceiptOutcome, RejectionReason,
    compute_delta, evaluate_receipt,
};

fn assigned(paths: &[&str]) -> AssignedPaths {
    AssignedPaths::parse(paths.iter().map(|p| p.to_string())).unwrap()
}

fn rename_within(a: &Active, from: &str, to: &str) {
    fs::rename(a.root.join(from), a.root.join(to)).unwrap();
}

fn write_file(a: &Active, path: &str, bytes: &[u8]) {
    fs::write(a.root.join(path), bytes).unwrap();
}

#[tokio::test]
async fn receipt_rejects_unowned_rename_and_hidden_file() {
    let a = active("w07-receipt-unowned", "checkout-w07-receipt-unowned").await;
    fs::create_dir_all(a.root.join("other")).unwrap();
    rename_within(&a, "a/one.txt", "other/one.txt");
    fs::write(a.root.join(".cache-note"), b"hidden\n").unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["a"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::OutsideAssignment);
    assert_eq!(receipt.reasons(), &[RejectionReason::UnownedPath]);
    assert_eq!(receipt.unowned_count(), 2);
    let changes = receipt.changes();
    assert_eq!(changes.len(), 3);
    assert_eq!(changes[0].path(), ".cache-note");
    assert_eq!(changes[0].kind(), DeltaKind::Added);
    assert_eq!(changes[0].scope(), ChangeScope::Unowned);
    assert_eq!(changes[1].path(), "a/one.txt");
    assert_eq!(changes[1].kind(), DeltaKind::Deleted);
    assert_eq!(changes[1].scope(), ChangeScope::Owned);
    assert_eq!(changes[2].path(), "other/one.txt");
    assert_eq!(changes[2].kind(), DeltaKind::Added);
    assert_eq!(changes[2].scope(), ChangeScope::Unowned);
}

#[tokio::test]
async fn unowned_endpoint_of_a_rename_into_scope_still_rejects() {
    let a = active("w07-rename-in", "checkout-w07-rename-in").await;
    rename_within(&a, "tracked.txt", "a/tracked.txt");
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["a"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::OutsideAssignment);
    assert_eq!(receipt.reasons(), &[RejectionReason::UnownedPath]);
    assert_eq!(receipt.unowned_count(), 1);
    let changes = receipt.changes();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].path(), "a/tracked.txt");
    assert_eq!(changes[0].kind(), DeltaKind::Added);
    assert_eq!(changes[0].scope(), ChangeScope::Owned);
    assert_eq!(changes[1].path(), "tracked.txt");
    assert_eq!(changes[1].kind(), DeltaKind::Deleted);
    assert_eq!(changes[1].scope(), ChangeScope::Unowned);
}

#[tokio::test]
async fn in_scope_edit_stays_within_assignment() {
    let a = active("w07-receipt-edit", "checkout-w07-receipt-edit").await;
    write_file(&a, "a/one.txt", b"edited\n");
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["a"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    assert!(receipt.reasons().is_empty());
    assert_eq!(receipt.unowned_count(), 0);
    assert_eq!(receipt.changes().len(), 1);
    let change = &receipt.changes()[0];
    assert_eq!(change.path(), "a/one.txt");
    assert_eq!(change.kind(), DeltaKind::Modified);
    assert_eq!(change.scope(), ChangeScope::Owned);
    assert_eq!(change.bytes(), 7);
    assert_eq!(receipt.total_bytes(), 7);
    assert_eq!(receipt.checkout_id(), a.verified.checkout_id());
    assert_eq!(receipt.snapshot_id(), a.verified.snapshot_id());
    assert_eq!(receipt.snapshot_sha256(), a.verified.snapshot_sha256());
    assert_eq!(receipt.plan_sha256(), a.verified.plan_sha256());
}

#[tokio::test]
async fn byte_bound_rejects_in_scope_changes() {
    let a = active("w07-receipt-bound", "checkout-w07-receipt-bound").await;
    write_file(&a, "a/one.txt", b"edited\n");
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    let outside = evaluate_receipt(&a.verified, &delta, &assigned(&["a"]), 6);
    assert_eq!(outside.outcome(), ReceiptOutcome::OutsideAssignment);
    assert_eq!(outside.reasons(), &[RejectionReason::OverByteBound]);
    assert_eq!(outside.unowned_count(), 0);
    let within = evaluate_receipt(&a.verified, &delta, &assigned(&["a"]), 7);
    assert_eq!(within.outcome(), ReceiptOutcome::WithinAssignment);
    assert!(within.reasons().is_empty());
}

#[tokio::test]
async fn empty_delta_is_within_assignment() {
    let a = active("w07-receipt-empty", "checkout-w07-receipt-empty").await;
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert!(delta.is_empty());
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["a"]), u64::MAX);
    assert_eq!(receipt.outcome(), ReceiptOutcome::WithinAssignment);
    assert!(receipt.reasons().is_empty());
    assert!(receipt.changes().is_empty());
}

#[tokio::test]
async fn receipt_debug_carries_no_path() {
    let a = active("w07-receipt-debug", "checkout-w07-receipt-debug").await;
    fs::create_dir_all(a.root.join("secret-dir")).unwrap();
    write_file(&a, "secret-dir/x.txt", b"secret\n");
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    let receipt = evaluate_receipt(&a.verified, &delta, &assigned(&["a"]), u64::MAX);
    assert_eq!(receipt.unowned_count(), 1);
    assert!(!format!("{receipt:?}").contains("secret-dir"));
    assert!(!format!("{:?}", receipt.changes()[0]).contains("secret-dir"));
}
