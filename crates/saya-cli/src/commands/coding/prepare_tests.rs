//! Owner preparation runs no checks: a prepared receipt carries an empty
//! `checks` list and a ready candidate whatever the configuration names, and a
//! receipt that DOES carry checks still parses.

#![cfg(unix)]

use super::super::direct_root_fixture::{edit_root, fixture};
use super::super::prepare_receipt::PreparedReceipt;

#[tokio::test]
async fn owner_preparation_records_no_checks_and_runs_none() {
    // No check profile exists, so nothing configures or runs a check; a
    // prepared receipt carries an empty `checks` list whatever the config.
    let f = fixture("prepare-no-checks", vec!["tracked.txt".into()]).await;
    edit_root(&f.job, "tracked.txt", b"root edited\n");

    let receipt = f
        .job
        .prepare_candidate()
        .await
        .expect("preparation runs no check, so a refused program cannot fail it");
    assert_eq!(
        receipt["checks"],
        serde_json::json!([]),
        "nothing populates the receipt's checks: {receipt}"
    );
    assert_eq!(
        receipt["candidate_ready"],
        serde_json::json!(true),
        "a prepared candidate is ready: {receipt}"
    );
}

#[test]
fn an_old_prepared_receipt_with_checks_still_parses() {
    let digest = "a".repeat(64);
    let bytes = serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "source_generation_sha256": digest,
        "candidate": "candidate-old",
        "reviewed_source_sha256": digest,
        "checks": [{
            "profile": "lint",
            "receipt": {
                "version": 1,
                "job_id": "job-old",
                "source_sha256": digest,
                "command_id": "lint",
                "argv_sha256": digest,
                "toolchain": "rustc-1.0",
                "environment_profile": "confined",
                "outcome": {"status": "known_exit", "code": 0}
            }
        }],
        "candidate_ready": true
    }))
    .unwrap();

    let receipt = PreparedReceipt::from_bytes(&bytes)
        .expect("an old receipt that carries checks still parses");
    assert_eq!(receipt.checks().count(), 1);
    assert!(receipt.candidate_ready());
}
