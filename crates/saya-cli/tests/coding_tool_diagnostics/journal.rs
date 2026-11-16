//! Reading and checking the durable tool journal. The contract under
//! test: the private records directory of a real job gains
//! `tool-events.jsonl`, one JSON object per line — identity, a source-owned
//! tool name, one of the five outcome statuses, the bounded, redacted
//! transcript of the call (its id, the arguments the model sent, the result
//! the tool returned and how long it took), and nothing else: no loop
//! summaries, no provider reasoning, no model names beyond the fixed field
//! vocabulary. When the result-body budget runs out, every call keeps its
//! compact facts (name, status, duration, and a small arguments bound) and one
//! marker row says from which call the bodies were dropped.

use saya_agent::{MAX_TOOL_CALL_ID_BYTES, MAX_TOOL_REASON_BYTES, MAX_TOOL_TRANSCRIPT_FIELD_BYTES};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) const JOURNAL_FILE: &str = "tool-events.jsonl";
/// Mirrors the child event message bound (`MAX_CHILD_EVENT_BYTES`): a single
/// journal record can never smuggle unbounded content.
pub(crate) const MAX_LINE_BYTES: usize = 8 * 1024;
/// Diagnostic retained segments each keep the historical one-MiB bound.
pub(crate) const MAX_FILE_BYTES: usize = 1024 * 1024;
pub(crate) const OUTCOME_STATUSES: [&str; 5] =
    ["requested", "completed", "denied", "failed", "refused"];
/// The closed outcome word every row carries under `reason`.
pub(crate) const CLOSED_REASONS: [&str; 5] = [
    "dispatch_requested",
    "executor_completed",
    "policy_denied",
    "tool_failed",
    "execution_refused",
];

/// The whole permitted key vocabulary: a key outside this set is a leak
/// channel even if its value looks harmless. Four belong only to a failed,
/// denied or refused call: a time, a digest, a closed class and that class's
/// closed rendering, plus `reason_text` — the redacted, bounded tool-result
/// error text the model received. `reason` itself is not one of them: it is
/// the closed outcome word on every row. `transcript` is the bounded, redacted
/// record of the call itself. `transcript_bytes` and
/// `result_bodies_dropped_from_call` belong only to the one marker row that
/// says the run's result-body budget was reached.
pub(crate) const METADATA_KEYS: [&str; 16] = [
    "job_id",
    "agent_id",
    "invocation_id",
    "tool",
    "status",
    "reason",
    "reason_text",
    "source",
    "epoch",
    "at_unix_millis",
    "arguments_sha256",
    "failure_class",
    "error_text",
    "transcript",
    "transcript_bytes",
    "result_bodies_dropped_from_call",
];

/// The whole permitted key vocabulary of one `transcript` object.
pub(crate) const TRANSCRIPT_KEYS: [&str; 4] = ["call_id", "arguments", "result", "duration_ms"];

/// The bound every call's arguments are cut to, whether or not the result-body
/// budget still has room. Mirrors the writer's own compact bound.
pub(crate) const COMPACT_ARGUMENT_BYTES: usize = 512;

/// The reserved tool name and closed words of the one marker row a run writes
/// when its transcript byte ceiling is reached.
pub(crate) const TRANSCRIPT_MARKER_TOOL: &str = "transcript";
pub(crate) const TRANSCRIPT_MARKER_STATUS: &str = "truncated";
pub(crate) const TRANSCRIPT_MARKER_REASON: &str = "transcript_truncated";

pub(crate) fn records_dir(root: &Path, job: &str) -> PathBuf {
    root.join("coding").join(job)
}

pub(crate) fn journal_path(root: &Path, job: &str) -> PathBuf {
    records_dir(root, job).join(JOURNAL_FILE)
}

/// Reads and validates the whole journal left by production.
pub(crate) fn read_journal(root: &Path, job: &str) -> Vec<Value> {
    let current = journal_path(root, job);
    let previous = current.with_file_name("tool-events.previous.jsonl");
    let mut records = Vec::new();
    for path in [previous, current] {
        if !path.exists() {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.len() <= MAX_FILE_BYTES);
        assert!(
            bytes.ends_with(b"\n"),
            "whole-record framing must survive rotation"
        );
        for (index, line) in std::str::from_utf8(&bytes).unwrap().lines().enumerate() {
            assert!(
                line.len() < MAX_LINE_BYTES,
                "record {index} exceeds framing limit"
            );
            records.push(serde_json::from_str(line).unwrap());
        }
    }
    assert!(
        !records.is_empty(),
        "the journal must record at least one tool event"
    );
    records
}

/// Every record is an object, stays inside the metadata key vocabulary,
/// names the job it belongs to, carries one of the five statuses, and records
/// a transcript whose every field is inside its own bound.
pub(crate) fn assert_bounded_metadata(records: &[Value], job: &str) {
    let mut markers = 0;
    for (index, record) in records.iter().enumerate() {
        let object = record
            .as_object()
            .unwrap_or_else(|| panic!("journal record {index} must be an object: {record}"));
        for key in object.keys() {
            assert!(
                METADATA_KEYS.contains(&key.as_str()),
                "journal record {index} carries key {key:?} outside the metadata vocabulary"
            );
        }
        assert_eq!(
            record["job_id"].as_str(),
            Some(job),
            "journal record {index} must name the real job"
        );
        assert!(record["epoch"].as_u64().is_some());
        if record["tool"] == TRANSCRIPT_MARKER_TOOL {
            markers += 1;
            assert_eq!(record["status"], TRANSCRIPT_MARKER_STATUS, "{record}");
            assert_eq!(record["reason"], TRANSCRIPT_MARKER_REASON, "{record}");
            assert!(
                record["transcript_bytes"].as_u64().is_some(),
                "the marker row says how much was recorded: {record}"
            );
            assert!(
                record["result_bodies_dropped_from_call"]
                    .as_u64()
                    .is_some_and(|call| call > 0),
                "the marker names the first call whose body was dropped: {record}"
            );
            assert!(
                record.get("transcript").is_none(),
                "the marker row records no transcript of its own: {record}"
            );
            // Compact rows keep coming after the marker; only the bulky result
            // body stops there.
            assert!(
                records[index + 1..].iter().all(|later| later
                    .get("transcript")
                    .and_then(|transcript| transcript.get("result"))
                    .is_none()),
                "no result body follows the marker row: {record}"
            );
            continue;
        }
        assert_transcript(record);
        let status = record["status"].as_str().unwrap_or_else(|| {
            panic!("journal record {index} must carry an outcome status: {record}")
        });
        assert!(
            OUTCOME_STATUSES.contains(&status),
            "journal record {index} carries status {status:?} outside the outcome vocabulary"
        );
        // `reason` is the closed outcome word, on every row, old and new alike.
        assert!(
            CLOSED_REASONS.contains(&record["reason"].as_str().unwrap_or_else(|| panic!(
                "journal record {index} must carry its closed outcome word: {record}"
            ))),
            "journal record {index} carries a reason outside the closed vocabulary: {record}"
        );
        // `reason_text` is the redacted, bounded tool-result error text the model
        // received. A row records one exactly when it records the failure detail,
        // and only a denied, failed or refused call records that.
        let has_detail = record.get("failure_class").is_some();
        assert_eq!(
            record.get("reason_text").is_some(),
            has_detail,
            "record {index} records `reason_text` exactly when it carries a failure detail: {record}"
        );
        if has_detail {
            assert!(
                matches!(status, "denied" | "failed" | "refused"),
                "record {index} carries a failure detail on a {status} call: {record}"
            );
            let reason_text = record["reason_text"].as_str().unwrap();
            assert!(
                !reason_text.is_empty() && reason_text.len() <= MAX_TOOL_REASON_BYTES,
                "record {index} keeps its reason text inside the bound: {record}"
            );
        }
    }
    assert!(
        markers <= 1,
        "a run writes at most one transcript marker row, found {markers}"
    );
}

/// One row's transcript, when it carries one: the fixed key vocabulary, a call
/// id, the arguments, the optional result the tool returned, and a duration —
/// every text inside its own bound.
fn assert_transcript(record: &Value) {
    let Some(transcript) = record.get("transcript") else {
        return;
    };
    let object = transcript
        .as_object()
        .unwrap_or_else(|| panic!("a transcript is an object: {record}"));
    for key in object.keys() {
        assert!(
            TRANSCRIPT_KEYS.contains(&key.as_str()),
            "the transcript carries key {key:?} outside its vocabulary: {record}"
        );
    }
    let call_id = transcript["call_id"]
        .as_str()
        .unwrap_or_else(|| panic!("a transcript names its call: {record}"));
    assert!(
        !call_id.is_empty() && call_id.len() <= MAX_TOOL_CALL_ID_BYTES,
        "the call id stays inside its bound: {record}"
    );
    let arguments = transcript["arguments"]
        .as_str()
        .unwrap_or_else(|| panic!("a transcript records the arguments sent: {record}"));
    assert!(
        !arguments.is_empty() && arguments.len() <= COMPACT_ARGUMENT_BYTES,
        "the recorded arguments stay inside the compact bound: {record}"
    );
    if let Some(result) = transcript.get("result") {
        let result = result
            .as_str()
            .unwrap_or_else(|| panic!("a recorded result is text: {record}"));
        assert!(
            !result.is_empty() && result.len() <= MAX_TOOL_TRANSCRIPT_FIELD_BYTES,
            "the recorded result stays inside its bound: {record}"
        );
    }
    assert!(
        transcript["duration_ms"].as_u64().is_some(),
        "a transcript records how long the call took: {record}"
    );
}

pub(crate) fn count(records: &[Value], tool: &str, status: &str) -> usize {
    records
        .iter()
        .filter(|record| record["tool"] == tool && record["status"] == status)
        .count()
}

/// A record for one tool outcome attributed to exactly one child identity.
pub(crate) fn has_child_record(records: &[Value], tool: &str, status: &str, agent: &str) -> bool {
    records.iter().any(|record| {
        record["tool"] == tool
            && record["status"] == status
            && record["agent_id"] == agent
            && record["invocation_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
    })
}

/// Valid metadata from a prior epoch makes the real run cross the legacy cap.
pub(crate) fn prefill_legacy(root: &Path, job: &str) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(journal_path(root, job))
        .unwrap();
    let record = serde_json::to_vec(&serde_json::json!({
        "job_id":job,"agent_id":null,"invocation_id":null,"epoch":0,
        "tool":"coding_status","status":"completed","reason":"executor_completed",
        "source":"actual_tool_loop"
    }))
    .unwrap();
    for _ in 0..(MAX_FILE_BYTES / (record.len() + 1)) {
        file.write_all(&record).unwrap();
        file.write_all(b"\n").unwrap();
    }
    file.sync_all().unwrap();
}
pub(crate) fn retained_text(root: &Path, job: &str) -> String {
    [
        JOURNAL_FILE,
        "tool-events.previous.jsonl",
        "diagnostics-summary.json",
    ]
    .into_iter()
    .filter_map(|name| fs::read_to_string(records_dir(root, job).join(name)).ok())
    .collect::<Vec<_>>()
    .join("\n")
}
