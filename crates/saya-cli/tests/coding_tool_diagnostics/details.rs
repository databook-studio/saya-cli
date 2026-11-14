//! Persisted detail for failed, denied and refused tool events: the durable
//! record gains when the call happened, the digest of what the model sent, the
//! closed failure class with its closed rendering, and `reason_text` — the
//! redacted, bounded tool-result error text the model received. The call's own
//! arguments and result are recorded beside it under `transcript`, bounded and
//! redacted; provider text still never reaches the record. `reason` itself stays
//! the closed outcome word every row carries.

use super::{
    journal,
    provider::{ARG_SENTINEL, Provider, RESULT_SENTINEL},
    run::drive,
};
use serde_json::Value;

/// Every `error` string the model was handed back in a tool message, read from
/// the request bodies the provider actually received.
fn model_tool_error_texts(provider: &Provider) -> Vec<String> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            serde_json::from_str::<Value>(body).unwrap()["messages"]
                .as_array()
                .unwrap()
                .clone()
        })
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| {
            let content = message["content"].as_str()?;
            serde_json::from_str::<Value>(content)
                .ok()?
                .get("error")?
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}

/// Every closed failure class the durable record may carry.
const FAILURE_CLASSES: [&str; 3] = ["approval_denied", "admission_refused", "executor_error"];

/// The closed rendering of each class; the record's `error_text` is one of
/// these and nothing else.
const ERROR_TEXTS: [&str; 3] = [
    "the call was denied by approval policy",
    "the effect admission refused the call before execution",
    "the tool executor returned an error",
];

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Rows the record keeps for a call that did not succeed.
fn failed_rows(records: &[Value]) -> Vec<&Value> {
    records
        .iter()
        .filter(|record| {
            matches!(
                record["status"].as_str(),
                Some("denied" | "failed" | "refused")
            )
        })
        .collect()
}

#[tokio::test]
async fn every_failed_denied_or_refused_row_carries_time_digest_class_and_closed_text() {
    let provider = Provider::diagnostics();
    let (fixture, _job) = drive("diag-detail", &provider, vec!["diag-a.txt".into()]).await;
    let records = journal::read_journal(&fixture.root, "diag-detail");
    let rows = failed_rows(&records);
    assert!(
        rows.iter().any(|row| row["status"] == "denied"),
        "the run must deny a write so the detail is proven on a real denial: {records:?}"
    );
    assert!(
        rows.iter().any(|row| row["status"] == "failed"),
        "the run must fail a call so the detail is proven on a real failure: {records:?}"
    );
    for row in &rows {
        assert!(
            row["at_unix_millis"].as_u64().is_some_and(|ms| ms > 0),
            "a failed call records when it happened: {row}"
        );
        let digest = row["arguments_sha256"]
            .as_str()
            .unwrap_or_else(|| panic!("a failed call records an arguments digest: {row}"));
        assert!(is_sha256_hex(digest), "the digest is a sha256 hex: {row}");
        let class = row["failure_class"]
            .as_str()
            .unwrap_or_else(|| panic!("a failed call records its closed class: {row}"));
        assert!(
            FAILURE_CLASSES.contains(&class),
            "the class is closed vocabulary: {row}"
        );
        let text = row["error_text"]
            .as_str()
            .unwrap_or_else(|| panic!("a failed call records its closed text: {row}"));
        assert!(
            ERROR_TEXTS.contains(&text),
            "the text is the closed rendering of a class: {row}"
        );
        assert!(
            text.len() <= 200,
            "the error text stays inside its bound: {row}"
        );
    }
}

#[tokio::test]
async fn a_failed_calls_reason_text_is_the_error_text_the_model_received() {
    let provider = Provider::unknown_tool();
    let (fixture, _job) = drive("diag-reason", &provider, vec![]).await;
    let records = journal::read_journal(&fixture.root, "diag-reason");
    journal::assert_bounded_metadata(&records, "diag-reason");
    let failed = records
        .iter()
        .find(|record| record["status"] == "failed")
        .expect("the unknown tool call fails");
    let reason_text = failed["reason_text"]
        .as_str()
        .unwrap_or_else(|| panic!("a failed call records the text the model read: {failed}"));
    assert!(
        reason_text.contains("unknown tool"),
        "the reason text names the rule that refused the call: {failed}"
    );
    // The recorded text is exactly the one the model was handed back for the
    // call — the record can name the rule because the model could read it.
    let seen = model_tool_error_texts(&provider);
    assert!(
        seen.iter().any(|text| text == reason_text),
        "the reason text is the error text the model received: {reason_text:?} not in {seen:?}"
    );
}

#[tokio::test]
async fn a_completed_call_keeps_its_reason_word_and_records_no_reason_text() {
    let provider = Provider::status();
    // A granted path is what attaches the workspace tools, and the run needs
    // one real tool call to complete.
    let (fixture, _job) = drive("diag-no-reason", &provider, vec!["diag-a.txt".into()]).await;
    let records = journal::read_journal(&fixture.root, "diag-no-reason");
    journal::assert_bounded_metadata(&records, "diag-no-reason");
    let completed: Vec<&Value> = records
        .iter()
        .filter(|record| record["status"] == "completed")
        .collect();
    assert!(
        !completed.is_empty(),
        "the run must complete a call to prove the absence: {records:?}"
    );
    for row in completed {
        assert_eq!(
            row["reason"], "executor_completed",
            "a call that succeeded keeps the closed outcome word under `reason`: {row}"
        );
        assert!(
            row.get("reason_text").is_none(),
            "a call that succeeded carries no reason text: {row}"
        );
        assert!(
            row.get("failure_class").is_none(),
            "a call that succeeded carries no failure detail: {row}"
        );
    }
}

/// One drive where a row succeeds and one where it fails: the closed word is on
/// both, and only the failure adds the free wording under its own key.
#[tokio::test]
async fn the_closed_reason_word_stays_on_every_row_and_only_a_failure_adds_reason_text() {
    let completed_provider = Provider::status();
    let (completed_fixture, _job) = drive(
        "diag-word-completed",
        &completed_provider,
        vec!["diag-a.txt".into()],
    )
    .await;
    let completed = journal::read_journal(&completed_fixture.root, "diag-word-completed");
    journal::assert_bounded_metadata(&completed, "diag-word-completed");
    let succeeded = completed
        .iter()
        .find(|record| record["status"] == "completed")
        .expect("the status run completes a call");
    assert_eq!(succeeded["reason"], "executor_completed");
    assert!(succeeded.get("reason_text").is_none());

    let failed_provider = Provider::unknown_tool();
    let (failed_fixture, _job) = drive("diag-word-failed", &failed_provider, vec![]).await;
    let failed_records = journal::read_journal(&failed_fixture.root, "diag-word-failed");
    journal::assert_bounded_metadata(&failed_records, "diag-word-failed");
    let failed = failed_records
        .iter()
        .find(|record| record["status"] == "failed")
        .expect("the unknown tool call fails");
    assert_eq!(
        failed["reason"], "tool_failed",
        "a failed call still names its outcome with the closed word: {failed}"
    );
    assert!(
        failed["reason_text"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "a failed call also carries the free wording under its own key: {failed}"
    );
}

#[tokio::test]
async fn an_old_line_without_reason_text_still_parses() {
    let provider = Provider::status();
    let (fixture, _job) = drive("diag-old-line", &provider, vec![]).await;
    // A record written before the free wording existed carries the closed word
    // it always had, and no `reason_text` key at all.
    let path = journal::journal_path(&fixture.root, "diag-old-line");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(
        br#"{"job_id":"diag-old-line","agent_id":null,"invocation_id":null,"epoch":0,"tool":"coding_status","status":"completed","reason":"executor_completed","source":"actual_tool_loop"}
"#,
    );
    std::fs::write(&path, bytes).unwrap();
    let records = journal::read_journal(&fixture.root, "diag-old-line");
    let old = records
        .iter()
        .find(|record| record["epoch"] == 0 && record.get("reason_text").is_none())
        .unwrap_or_else(|| panic!("the old line must still parse: {records:?}"));
    assert_eq!(old["tool"], "coding_status");
    assert_eq!(old["status"], "completed");
    assert_eq!(old["reason"], "executor_completed");
}

#[tokio::test]
async fn the_digest_is_per_call_and_the_recorded_arguments_are_the_bounded_transcript() {
    let provider = Provider::diagnostics();
    let (fixture, _job) = drive("diag-digest", &provider, vec!["diag-a.txt".into()]).await;
    let records = journal::read_journal(&fixture.root, "diag-digest");
    // Two writes with different arguments must not share a digest, and a write
    // that succeeded must not carry one at all.
    let denied = records
        .iter()
        .find(|record| record["tool"] == "write" && record["status"] == "denied")
        .expect("the unassigned write is denied");
    let completed = records
        .iter()
        .find(|record| record["tool"] == "write" && record["status"] == "completed")
        .expect("the assigned write completes");
    assert!(
        completed.get("arguments_sha256").is_none(),
        "a succeeded call carries no failure detail: {completed}"
    );
    assert!(
        denied["arguments_sha256"] != completed["arguments_sha256"],
        "different arguments must not share a digest"
    );
    // The digest is the failure detail's own key; the arguments themselves are
    // recorded under `transcript`, bounded and redacted, and the denied call's
    // row carries what the model sent it.
    let transcript = denied["transcript"]
        .as_object()
        .unwrap_or_else(|| panic!("a denied call records its call: {denied}"));
    let arguments = transcript["arguments"].as_str().unwrap();
    assert!(
        arguments.contains("diag-denied.txt") && arguments.contains("unassigned"),
        "the recorded arguments are the ones the model sent: {denied}"
    );
    assert!(
        transcript["result"]
            .as_str()
            .is_some_and(|result| result.contains("denied")),
        "a denied call records the text the model was handed back: {denied}"
    );
    // The other write's row records its own arguments, not this one's.
    assert!(
        completed["transcript"]["arguments"]
            .as_str()
            .is_some_and(|arguments| arguments.contains(RESULT_SENTINEL)
                && !arguments.contains("unassigned")),
        "each row records its own call: {completed}"
    );
    let text = journal::retained_text(&fixture.root, "diag-digest");
    assert!(text.contains(ARG_SENTINEL) && text.contains(RESULT_SENTINEL));
}

#[tokio::test]
async fn rows_written_before_the_detail_existed_still_parse_and_stay_readable() {
    let provider = Provider::diagnostics();
    let (fixture, job) =
        super::run::create("diag-legacy", &provider, vec!["diag-a.txt".into()]).await;
    // A legacy row: the shape carries none of the newer keys.
    journal::prefill_legacy(&fixture.root, "diag-legacy");
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        job.run(&saya_agent::NoopEventSink),
    )
    .await
    .unwrap()
    .unwrap();
    let records = journal::read_journal(&fixture.root, "diag-legacy");
    // The pre-change rows are the epoch-0 rows the fixture prefills.
    let legacy: Vec<&Value> = records
        .iter()
        .filter(|record| record["epoch"] == 0)
        .collect();
    assert!(
        !legacy.is_empty(),
        "the fixture must retain pre-change rows to prove the old shape still reads"
    );
    for row in legacy {
        assert!(row["job_id"].is_string());
        assert!(row["status"].is_string());
        for key in [
            "at_unix_millis",
            "arguments_sha256",
            "failure_class",
            "error_text",
        ] {
            assert!(row.get(key).is_none(), "an old row gains no new key: {row}");
        }
    }
    // A reader that only knows the old keys still reads the new rows.
    journal::assert_bounded_metadata(&records, "diag-legacy");
}
