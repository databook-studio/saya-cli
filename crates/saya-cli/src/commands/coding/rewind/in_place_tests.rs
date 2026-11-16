//! Rewind over an in-place job's own folder: the checkpoint captures the
//! folder's changes against its local baseline, a rewind restores and removes
//! through the job's `--write` scope, a live job refuses, and a path outside
//! the scope is never touched.
#![cfg(unix)]

use super::in_place_fixture::{fixture, fixture_with, journal, pause, read, record, write};
use crate::commands::coding::CodingError;
use serde_json::json;

#[tokio::test]
async fn in_place_checkpoint_captures_changes() {
    let fixture = fixture("capture", vec![".".into()]).await;
    record(&fixture, "tracked.txt", Some(b"original\n"), 1);
    write(&fixture, "tracked.txt", "one\n");
    fixture.job.capture_rewind().await;

    let checkpoints = fixture.job.rewind_store().list().unwrap();
    assert_eq!(checkpoints.len(), 1, "one changed turn, one checkpoint");
    assert_eq!(checkpoints[0].turn, 1);
    assert_eq!(
        checkpoints[0]
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["tracked.txt"]
    );
    let listed = fixture
        .application
        .rewind(fixture.job.job_id(), None, false)
        .await
        .unwrap();
    assert_eq!(listed["checkpoints"][0]["turn"], 1);
}

#[tokio::test]
async fn in_place_rewind_restores_and_removes() {
    let fixture = fixture("restore", vec![".".into()]).await;
    record(&fixture, "tracked.txt", Some(b"original\n"), 1);
    write(&fixture, "tracked.txt", "one\n");
    fixture.job.capture_rewind().await;
    write(&fixture, "tracked.txt", "two\n");
    record(&fixture, "added.txt", None, 2);
    write(&fixture, "added.txt", "added\n");
    fixture.job.capture_rewind().await;
    assert_eq!(fixture.job.rewind_store().list().unwrap().len(), 2);
    pause(&fixture).await;

    let report = fixture
        .application
        .rewind(fixture.job.job_id(), Some(1), false)
        .await
        .expect("the rewind restores the first checkpoint");
    assert_eq!(report["turn"], 1);
    assert_eq!(report["restored_paths"], 1);
    assert_eq!(report["removed_paths"], 1);
    assert_eq!(read(&fixture, "tracked.txt").unwrap(), "one\n");
    assert!(
        matches!(read(&fixture, "added.txt"), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "the file the later turn created is gone"
    );
}

#[tokio::test]
async fn in_place_rewind_refuses_running() {
    let fixture = fixture("running", vec![".".into()]).await;
    record(&fixture, "tracked.txt", Some(b"original\n"), 1);
    write(&fixture, "tracked.txt", "one\n");
    fixture.job.capture_rewind().await;
    assert_eq!(fixture.job.rewind_store().list().unwrap().len(), 1);

    assert!(matches!(
        fixture
            .application
            .rewind(fixture.job.job_id(), Some(1), false)
            .await,
        Err(CodingError::RewindWhileRunning)
    ));
    assert_eq!(
        read(&fixture, "tracked.txt").unwrap(),
        "one\n",
        "a refused rewind writes nothing"
    );
}

/// A path a concurrent editor changed after the job's checkpoint is reported
/// and left as it stands; `--force` is the operator's explicit override.
#[tokio::test]
async fn foreign_change_not_rewound() {
    let fixture = fixture("foreign-rewind", vec![".".into()]).await;
    record(&fixture, "tracked.txt", Some(b"original\n"), 1);
    write(&fixture, "tracked.txt", "job\n");
    journal(&fixture, &[("tracked.txt", "job\n", "write", 1)]);
    fixture.job.capture_rewind().await;
    // The user edits the file after the checkpoint the job took.
    write(&fixture, "tracked.txt", "user\n");
    pause(&fixture).await;

    let report = fixture
        .application
        .rewind(fixture.job.job_id(), Some(1), false)
        .await
        .expect("the rewind restores what the journal accounts for");
    assert_eq!(report["foreign_paths"], json!(["tracked.txt"]));
    assert_eq!(report["restored_paths"], 0);
    assert_eq!(
        read(&fixture, "tracked.txt").unwrap(),
        "user\n",
        "a concurrent edit is never silently overwritten"
    );

    let forced = fixture
        .application
        .rewind(fixture.job.job_id(), Some(1), true)
        .await
        .expect("the forced rewind restores the checkpoint");
    assert_eq!(forced["foreign_paths"], json!([]));
    assert_eq!(read(&fixture, "tracked.txt").unwrap(), "job\n");
}

#[tokio::test]
async fn out_of_scope_foreign_change_untouched() {
    let fixture = fixture("foreign", vec!["tracked.txt".into()]).await;
    record(&fixture, "tracked.txt", Some(b"original\n"), 1);
    write(&fixture, "tracked.txt", "two\n");
    record(&fixture, "foreign.txt", None, 1);
    write(&fixture, "foreign.txt", "foreign\n");
    fixture.job.capture_rewind().await;
    // Change both after the checkpoint: only `tracked.txt` is in scope.
    write(&fixture, "tracked.txt", "three\n");
    write(&fixture, "foreign.txt", "changed\n");
    pause(&fixture).await;

    let report = fixture
        .application
        .rewind(fixture.job.job_id(), Some(1), false)
        .await
        .expect("the rewind restores what the scope admits");
    assert_eq!(read(&fixture, "tracked.txt").unwrap(), "two\n");
    assert_eq!(
        read(&fixture, "foreign.txt").unwrap(),
        "changed\n",
        "a path outside `--write` is never touched"
    );
    assert_eq!(
        report["out_of_scope_paths"],
        serde_json::json!(["foreign.txt"]),
        "the checkpoint held a path the job's scope does not admit"
    );
}

/// A baseline restore reads the job's before-image store: an edit is put back,
/// a creation is removed, a deletion is recreated, and a change no tool made is
/// reported as out of the restore's reach and left as it stands.
#[tokio::test]
async fn in_place_rewind_baseline_from_before_images() {
    let fixture = fixture_with(
        "baseline-before-images",
        vec![".".into()],
        &[("deleted.txt", b"deleted\n")],
    )
    .await;
    record(&fixture, "tracked.txt", Some(b"original\n"), 1);
    write(&fixture, "tracked.txt", "edited\n");
    record(&fixture, "created.txt", None, 1);
    write(&fixture, "created.txt", "created\n");
    record(&fixture, "deleted.txt", Some(b"deleted\n"), 1);
    std::fs::remove_file(fixture.folder.join("deleted.txt")).unwrap();
    // The user's own shell, which no tool of the job recorded.
    write(&fixture, "shell.txt", "shell\n");
    pause(&fixture).await;

    let report = fixture
        .application
        .rewind_baseline(fixture.job.job_id(), false)
        .await
        .expect("the baseline restore runs from the before-image store");
    assert_eq!(report["restored_paths"], 2, "{report}");
    assert_eq!(report["removed_paths"], 1, "{report}");
    assert_eq!(read(&fixture, "tracked.txt").unwrap(), "original\n");
    assert!(
        matches!(read(&fixture, "created.txt"), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "a file the job created is removed"
    );
    assert_eq!(read(&fixture, "deleted.txt").unwrap(), "deleted\n");
    assert_eq!(
        read(&fixture, "shell.txt").unwrap(),
        "shell\n",
        "a shell-made change is reported, never rewound"
    );
    assert!(
        report["note"]
            .as_str()
            .is_some_and(|note| note.contains("never rewound")),
        "{report}"
    );
}

/// A path the checkpoint does not hold returns to its before-image: it existed
/// at that turn and was simply not changed yet, so a checkpoint restore must
/// never remove it.
#[tokio::test]
async fn checkpoint_restore_uses_before_image() {
    let fixture = fixture_with(
        "checkpoint-before-image",
        vec![".".into()],
        &[("a.txt", b"orig-a\n"), ("b.txt", b"orig-b\n")],
    )
    .await;
    record(&fixture, "a.txt", Some(b"orig-a\n"), 1);
    write(&fixture, "a.txt", "one\n");
    fixture.job.capture_rewind().await;
    assert_eq!(fixture.job.rewind_store().list().unwrap().len(), 1);
    record(&fixture, "b.txt", Some(b"orig-b\n"), 2);
    write(&fixture, "b.txt", "two\n");
    fixture.job.capture_rewind().await;
    assert_eq!(fixture.job.rewind_store().list().unwrap().len(), 2);
    pause(&fixture).await;

    let report = fixture
        .application
        .rewind(fixture.job.job_id(), Some(1), false)
        .await
        .expect("the rewind restores the first checkpoint");
    assert_eq!(report["turn"], 1);
    assert_eq!(
        read(&fixture, "a.txt").unwrap(),
        "one\n",
        "the checkpoint's own image restores it"
    );
    assert_eq!(
        read(&fixture, "b.txt").unwrap(),
        "orig-b\n",
        "a path the checkpoint does not hold returns to its before-image, never removed"
    );
}
