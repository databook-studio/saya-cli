//! The redacted, capped, authority-free child summary. Helpers share W,
//! so the summary is the child's own answer: there is no merge outcome and no
//! per-helper changed-path list to report.

#![cfg(unix)]

use super::direct_root_fixture::{agent, fixture, helper_wrote_w};
use saya_types::CodingResultOutcome;

#[tokio::test]
async fn child_summary_is_redacted_capped_and_carries_no_authority() {
    let f = fixture("summary", vec!["tracked.txt".into()]).await;
    // A short answer with a secret-shaped token: redacted by the same redactor
    // the result views use (saya_types::redact), capped to 8 KiB.
    helper_wrote_w(
        &f.job,
        "child-a",
        vec!["tracked.txt".into()],
        CodingResultOutcome::Completed,
        "done; api_key=secret123",
        Some(("tracked.txt", b"child edit\n")),
    )
    .await;
    let summary = f.job.child_summary(&agent("child-a")).await.unwrap();
    assert_eq!(summary.authority, "none");
    assert_eq!(summary.outcome, CodingResultOutcome::Completed);
    assert!(summary.failure_class.is_none());
    assert!(
        summary.summary.contains("[redacted]") && !summary.summary.contains("secret123"),
        "secrets are redacted: {:?}",
        summary.summary
    );
    assert!(
        !summary.summary_truncated,
        "a short answer is not truncated"
    );
    // A shared tree attributes no delta to one helper and has no merge to
    // report, so the wire shape carries neither field.
    let wire = serde_json::to_value(&summary).unwrap();
    assert!(wire.get("changed_paths").is_none(), "{wire}");
    assert!(wire.get("merged").is_none(), "{wire}");
    assert!(
        summary.assignment_added.is_empty(),
        "an unmarked report exposes no assignment-added directives"
    );
    // A long answer is capped to 8 KiB with a truncated flag.
    let big = "x".repeat(super::task_merge_record::MAX_SUMMARY_BYTES + 100);
    helper_wrote_w(
        &f.job,
        "child-b",
        Vec::new(),
        CodingResultOutcome::Completed,
        &big,
        Some(("other.txt", b"b\n")),
    )
    .await;
    let summary = f.job.child_summary(&agent("child-b")).await.unwrap();
    assert!(summary.summary_truncated, "a long answer is truncated");
    assert!(summary.summary.len() <= super::task_merge_record::MAX_SUMMARY_BYTES);
}

#[tokio::test]
async fn a_child_report_can_mark_an_assignment_added_requirement() {
    let f = fixture("summary-assignment-added", vec!["tracked.txt".into()]).await;
    helper_wrote_w(
        &f.job,
        "child-a",
        vec!["tracked.txt".into()],
        CodingResultOutcome::Completed,
        "followed SPEC.md\nassignment-added: invent <path>: line <N>:\ndone",
        Some(("tracked.txt", b"child edit\n")),
    )
    .await;
    let summary = f.job.child_summary(&agent("child-a")).await.unwrap();
    assert_eq!(
        summary.assignment_added,
        vec!["invent <path>: line <N>:".to_owned()]
    );
    assert_eq!(summary.authority, "none");
}
