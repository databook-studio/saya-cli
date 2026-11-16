//! `rewind --baseline` over an in-place job reads the job's before-image
//! store: a path outside `--write` is reported and never touched, and a path
//! the store could not record is reported as not restorable.
#![cfg(unix)]

use super::in_place_fixture::{fixture_with, pause, read, record, write};

#[tokio::test]
async fn baseline_restore_reports_out_of_scope() {
    let fixture = fixture_with(
        "baseline-foreign",
        vec!["tracked.txt".into()],
        &[("foreign.txt", b"foreign\n")],
    )
    .await;
    record(&fixture, "tracked.txt", Some(b"original\n"), 1);
    write(&fixture, "tracked.txt", "changed\n");
    record(&fixture, "foreign.txt", Some(b"foreign\n"), 1);
    write(&fixture, "foreign.txt", "foreign changed\n");
    pause(&fixture).await;

    let report = fixture
        .application
        .rewind_baseline(fixture.job.job_id(), false)
        .await
        .expect("the baseline restore restores what the scope admits");
    assert_eq!(read(&fixture, "tracked.txt").unwrap(), "original\n");
    assert_eq!(
        read(&fixture, "foreign.txt").unwrap(),
        "foreign changed\n",
        "a recorded path outside `--write` is never touched"
    );
    assert_eq!(
        report["out_of_scope_paths"],
        serde_json::json!(["foreign.txt"])
    );
}

#[tokio::test]
async fn baseline_restore_reports_unrecorded() {
    // Larger than the before-image per-file bound, so the store holds no
    // bytes to put back and the restore reports the path instead.
    let large = vec![b'x'; 9 * 1024 * 1024];
    let fixture = fixture_with(
        "baseline-unrecorded",
        vec![".".into()],
        &[("large.bin", &large)],
    )
    .await;
    record(&fixture, "large.bin", Some(&large), 1);
    write(&fixture, "large.bin", "changed\n");
    record(&fixture, "tracked.txt", Some(b"original\n"), 1);
    write(&fixture, "tracked.txt", "changed\n");
    pause(&fixture).await;

    let report = fixture
        .application
        .rewind_baseline(fixture.job.job_id(), false)
        .await
        .expect("an unrecorded path is reported, never an error");
    assert_eq!(
        report["not_restorable_paths"],
        serde_json::json!(["large.bin"])
    );
    assert_eq!(read(&fixture, "large.bin").unwrap(), "changed\n");
    assert_eq!(read(&fixture, "tracked.txt").unwrap(), "original\n");
}
