//! Bounded-output behavior of the durable tool journal: the records stay
//! private, per-line and whole-file bounds hold on a real run, truncation is
//! explicit, and an unknown tool name never leaks.

use super::{
    journal,
    provider::{Provider, unknown_tool_name},
    run::drive,
};

#[tokio::test]
async fn tool_events_journal_bounds_unknown_tool_names_and_loop_summaries() {
    let provider = Provider::unknown_tool();
    let (fixture, _job) = drive("diag-unknown-tool", &provider, vec![]).await;
    let records = journal::read_journal(&fixture.root, "diag-unknown-tool");
    journal::assert_bounded_metadata(&records, "diag-unknown-tool");
    assert!(
        records.iter().any(|record| record["status"] == "failed"),
        "the invalid call must be recorded as a real failure, not completed: {records:?}"
    );
    let text =
        std::fs::read_to_string(journal::journal_path(&fixture.root, "diag-unknown-tool")).unwrap();
    let unknown = unknown_tool_name();
    // The failed row records the tool-result error text the model read, and
    // that text names the unknown tool — that is what `reason_text` is for. The
    // bounded `tool` field still refuses the name, so the name reaches the
    // record only where the model already saw it.
    let failed = records
        .iter()
        .find(|record| record["status"] == "failed")
        .expect("the unknown call is recorded as failed");
    assert!(
        failed["reason_text"]
            .as_str()
            .is_some_and(|reason_text| reason_text.contains(unknown.as_str())),
        "the reason text is the text the model read, name and all: {failed}"
    );
    for record in &records {
        assert_eq!(record["tool"], "unknown_tool");
        assert!(
            !record["tool"].as_str().unwrap().contains(unknown.as_str()),
            "the tool field must not carry the unknown name: {record}"
        );
    }
    // The refused call's transcript records the refusal text the model read, so
    // the durable record can name the rule without widening the `tool` field.
    assert!(
        failed["transcript"]["result"]
            .as_str()
            .is_some_and(|result| result.contains(unknown.as_str())),
        "the refused call records the text the model was handed back: {failed}"
    );
    assert!(
        !text.contains("tool call failed validation"),
        "the loop's arbitrary completion summary must never be persisted"
    );
}
#[tokio::test]
async fn valid_legacy_journal_rolls_over_and_the_run_completes() {
    let provider = Provider::status();
    let (fixture, job) = super::run::create("diag-full", &provider, vec![]).await;
    let path = journal::journal_path(&fixture.root, "diag-full");
    journal::prefill_legacy(&fixture.root, "diag-full");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        job.run(&saya_agent::NoopEventSink),
    )
    .await
    .unwrap();
    assert!(
        result.is_ok(),
        "valid diagnostic capacity must not fail a healthy run: {result:?}"
    );
    let records = journal::read_journal(&fixture.root, "diag-full");
    assert!(records.iter().any(|record| record["status"] == "completed"));
    let previous = path.with_file_name("tool-events.previous.jsonl");
    assert!(
        previous.is_file(),
        "a whole legacy segment must be retained after rollover"
    );
    for path in [&path, &previous] {
        assert!(std::fs::metadata(path).unwrap().len() <= journal::MAX_FILE_BYTES as u64);
    }
    let status = job.status().await.unwrap();
    assert_ne!(status["state"], "failed");
    assert_eq!(status["diagnostic_history"]["state"], "unknown");
    let exported = fixture
        .application
        .export(job.job_id(), None, None)
        .await
        .unwrap();
    assert_eq!(
        exported["artifacts"]["diagnostics"]["history"]["state"],
        "unknown"
    );
    assert_eq!(exported["artifacts"]["complete"], false);
    assert_eq!(
        exported["artifacts"]["diagnostics"]["ordering"],
        "previous_then_current"
    );
    let text = exported["artifacts"]["diagnostics"].to_string();
    for sentinel in [
        super::provider::ARG_SENTINEL,
        super::provider::RESULT_SENTINEL,
        super::provider::REASONING_SENTINEL,
    ] {
        assert!(!text.contains(sentinel));
    }
}

/// An independent process holds the stable lock inode: contention must fail
/// explicitly before appending, rather than race the shared cap check.
#[tokio::test]
async fn journal_lock_contention_fails_without_appending() {
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, process::Stdio, time::Duration};
    const CHILD_PATH: &str = "SAYA_TEST_TOOL_JOURNAL_LOCK_PATH";
    const CHILD_READY: &str = "SAYA_TEST_TOOL_JOURNAL_LOCK_READY";
    if let (Some(path), Some(ready)) = (std::env::var_os(CHILD_PATH), std::env::var_os(CHILD_READY))
    {
        let file = OpenOptions::new().append(true).open(path).unwrap();
        file.try_lock().unwrap();
        std::fs::write(&ready, b"locked").unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            while std::path::Path::new(&ready).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        return;
    }
    let provider = Provider::status();
    let (fixture, job) = super::run::create("diag-locked", &provider, vec![]).await;
    let path = journal::records_dir(&fixture.root, "diag-locked").join(".diagnostics.lock");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    drop(file);
    let ready = fixture.root.join("journal-locked.ready");
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bounds::journal_lock_contention_fails_without_appending",
        ])
        .env(CHILD_PATH, &path)
        .env(CHILD_READY, &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("external process must hold the journal lock before the run");
    let result =
        tokio::time::timeout(Duration::from_secs(10), job.run(&saya_agent::NoopEventSink)).await;
    std::fs::remove_file(&ready).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(
        result.unwrap().is_err(),
        "journal lock contention must fail the real run"
    );
    assert_eq!(job.status().await.unwrap()["state"], "failed");
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        0,
        "no append while another process owns the file lock"
    );
}
