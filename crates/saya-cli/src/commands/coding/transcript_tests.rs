//! The writer's own bound on a recorded transcript: redacted, re-bounded, fitted
//! to the journal's record framing, and stopped at the job's ceiling with a
//! single marker row.

use super::{MAX_RECORD_BYTES, TranscriptBudget};
use saya_agent::{TOOL_TRANSCRIPT_ELISION_MARKER, ToolCallTranscript};
use serde_json::{Value, json};

/// The small bound every call's arguments are cut to, whether or not the
/// result-body budget still has room. Stated here as the contract the writer
/// must meet, not imported from the writer under test.
const COMPACT_ARGUMENT_BYTES: usize = 512;

/// The row shape the writer builds, with a reason text as long as the writer's
/// own bound allows after redaction: the case the fit loop exists for.
fn row(reason_text: &str) -> Value {
    json!({"job_id":"job-1","agent_id":null,"invocation_id":null,"epoch":1,
        "tool":"write","status":"failed","reason":"tool_failed","source":"actual_tool_loop",
        "failure_class":"executor_error","error_text":"the tool executor returned an error",
        "reason_text":reason_text})
}

fn transcript(arguments: &str, result: &str) -> ToolCallTranscript {
    ToolCallTranscript::new("call-1", arguments, Some(result), 12)
}

/// The row a tool call leaves before the writer touches it: the tool name and
/// status are the caller's own facts and must survive whatever the writer does.
fn scripted_row(tool: &str, status: &str) -> Value {
    json!({"job_id":"job-1","agent_id":null,"invocation_id":null,"epoch":1,
        "tool":tool,"status":status,"reason":"executor_completed","source":"actual_tool_loop"})
}

fn serialized(row: &Value) -> String {
    serde_json::to_string(row).unwrap()
}

#[test]
fn a_record_inside_the_framing_keeps_the_whole_transcript() {
    let budget = TranscriptBudget::new();
    let mut row = row("write failed");
    let call = transcript("{\"path\":\"a.txt\"}", "wrote 3 bytes");
    assert!(budget.attach(&mut row, &call));
    assert_eq!(row["transcript"]["call_id"], "call-1");
    assert_eq!(row["transcript"]["arguments"], "{\"path\":\"a.txt\"}");
    assert_eq!(row["transcript"]["result"], "wrote 3 bytes");
    assert_eq!(row["transcript"]["duration_ms"], 12);
    assert!(serialized(&row).len() <= MAX_RECORD_BYTES);
}

#[test]
fn an_over_long_record_shrinks_the_fields_until_it_fits() {
    let budget = TranscriptBudget::new();
    // A redacted reason text and a maximal transcript can together overrun the
    // framing; the fields are cut again, keeping both ends, until the record
    // fits whole — an over-long record would be rejected entirely.
    let mut row = row(&"r".repeat(5 * 1024));
    let head = "TRANSCRIPT-HEAD-4a7c";
    let tail = "TRANSCRIPT-TAIL-9f21";
    let call = transcript(
        &format!("{head}{}{tail}", "a".repeat(8 * 1024)),
        &"b".repeat(8 * 1024),
    );
    assert!(budget.attach(&mut row, &call));
    let record = serialized(&row);
    assert!(
        record.len() <= MAX_RECORD_BYTES,
        "the record fits the framing: {} bytes",
        record.len()
    );
    let arguments = row["transcript"]["arguments"].as_str().unwrap();
    assert!(
        arguments.starts_with(head) && arguments.ends_with(tail),
        "{arguments}"
    );
    assert!(
        arguments.contains(TOOL_TRANSCRIPT_ELISION_MARKER),
        "{arguments}"
    );
}

#[test]
fn a_transcript_that_cannot_fit_is_dropped_whole_and_the_row_survives() {
    let budget = TranscriptBudget::new();
    let mut row = row(&"r".repeat(MAX_RECORD_BYTES));
    let call = transcript(&"a".repeat(8 * 1024), &"b".repeat(8 * 1024));
    assert!(!budget.attach(&mut row, &call));
    assert!(row.get("transcript").is_none(), "{row}");
    assert_eq!(row["tool"], "write");
}

#[test]
fn a_secret_in_a_recorded_field_is_redacted_before_it_is_stored() {
    let budget = TranscriptBudget::new();
    let mut row = row("write failed");
    let call = transcript("password=hunter2", "Authorization: Bearer abc123");
    assert!(budget.attach(&mut row, &call));
    let record = serialized(&row);
    assert!(
        !record.contains("hunter2") && !record.contains("abc123"),
        "{record}"
    );
    assert!(record.contains("[redacted]"), "{record}");
}

#[test]
fn a_job_whose_results_overflow_the_budget_keeps_compact_facts_for_every_call() {
    let budget = TranscriptBudget::with_ceiling(8 * 1024);
    let mut rows = Vec::new();
    let mut marker = None;
    let mut marker_at = None;
    for call in 0..40u64 {
        let mut row = scripted_row("shell", "completed");
        let arguments = format!("{{\"command\":\"run step {call}\"}}");
        let result = format!("step-{call}-{}", "r".repeat(1024));
        let transcript =
            ToolCallTranscript::new(&format!("call-{call}"), &arguments, Some(&result), call);
        assert!(
            budget.attach(&mut row, &transcript),
            "call {call} keeps its compact facts"
        );
        rows.push(row);
        if let Some(value) = budget.take_marker("job-1", 1) {
            marker = Some(value);
            marker_at = Some(rows.len() - 1);
        }
    }
    let marker = marker.expect("the overflow writes one marker row");
    let marker_at = marker_at.expect("the marker follows the first dropped body");
    let dropped_from = marker["result_bodies_dropped_from_call"]
        .as_u64()
        .expect("the marker names the first call whose body was dropped");
    assert_eq!(
        dropped_from as usize,
        marker_at + 1,
        "the marker names the call whose body it dropped: {marker}"
    );
    assert_eq!(marker["tool"], "transcript");
    assert_eq!(marker["status"], "truncated");
    assert_eq!(marker["reason"], "transcript_truncated");
    assert!(
        marker["transcript_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0),
        "the marker says how much body it recorded: {marker}"
    );
    assert!(marker.get("transcript").is_none(), "{marker}");
    assert!(
        budget.take_marker("job-1", 1).is_none(),
        "only one marker row per job"
    );
    // Every call, before and after the marker, keeps its own compact facts:
    // name, status, duration and a bounded arguments text. Only the bulky
    // result body stops at the marker.
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["tool"], "shell", "{row}");
        assert_eq!(row["status"], "completed", "{row}");
        let transcript = row
            .get("transcript")
            .unwrap_or_else(|| panic!("call {index} keeps its compact facts: {row}"));
        assert_eq!(transcript["duration_ms"], index as u64, "{row}");
        let arguments = transcript["arguments"]
            .as_str()
            .unwrap_or_else(|| panic!("call {index} records its arguments: {row}"));
        assert!(
            !arguments.is_empty() && arguments.len() <= COMPACT_ARGUMENT_BYTES,
            "call {index} records arguments inside the compact bound: {} bytes",
            arguments.len()
        );
        assert!(
            serialized(row).len() <= MAX_RECORD_BYTES,
            "call {index} stays inside the record framing: {row}"
        );
        let has_body = transcript.get("result").is_some();
        assert_eq!(
            has_body,
            index + 1 < dropped_from as usize,
            "call {index} records its body exactly before the marker: {row}"
        );
    }
}

#[test]
fn compact_arguments_are_bounded_even_when_no_result_body_fits() {
    let budget = TranscriptBudget::with_ceiling(0);
    let mut row = scripted_row("shell", "completed");
    let head = "COMMAND-HEAD-1a2b";
    let tail = "COMMAND-TAIL-3c4d";
    let arguments = format!("{head}{}{tail}", "a".repeat(8 * 1024));
    let call = ToolCallTranscript::new("call-1", &arguments, Some(&"b".repeat(8 * 1024)), 7);
    assert!(budget.attach(&mut row, &call));
    let transcript = row
        .get("transcript")
        .expect("compact facts survive a spent result budget");
    assert_eq!(transcript["duration_ms"], 7);
    let recorded = transcript["arguments"].as_str().unwrap();
    assert!(
        recorded.len() <= COMPACT_ARGUMENT_BYTES,
        "the compact arguments stay inside the small bound: {} bytes",
        recorded.len()
    );
    assert!(
        recorded.starts_with(head) && recorded.ends_with(tail),
        "both ends of the arguments are kept: {recorded}"
    );
    assert!(
        recorded.contains(TOOL_TRANSCRIPT_ELISION_MARKER),
        "a cut says what it dropped: {recorded}"
    );
    assert!(
        transcript.get("result").is_none(),
        "no result body fits a spent budget: {row}"
    );
    let marker = budget
        .take_marker("job-1", 1)
        .expect("the dropped body is named");
    assert_eq!(marker["result_bodies_dropped_from_call"], 1);
}
