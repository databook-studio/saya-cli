//! The bounded, redacted transcript one durable tool row carries: the call id,
//! the arguments the model sent, the result the tool returned and how long the
//! call took — for the root and for a helper alike. Each field is bounded with
//! an elision marker, every field is redacted exactly like the rest of the
//! job's evidence, and a row written before the transcript existed still reads.

#[cfg(target_os = "macos")]
use super::provider::TRANSCRIPT_STDOUT;
use super::{
    journal,
    provider::{
        OVERFLOW_CALLS, OVERFLOW_PATH, Provider, TRANSCRIPT_BIG_HEAD, TRANSCRIPT_BIG_TAIL,
        TRANSCRIPT_COMMAND, TRANSCRIPT_PATH, TRANSCRIPT_SECRET_ARGUMENT, TRANSCRIPT_SECRET_RESULT,
        overflow_body,
    },
    run::{drive, drive_seeded},
};
use saya_agent::{MAX_TOOL_TRANSCRIPT_FIELD_BYTES, TOOL_TRANSCRIPT_ELISION_MARKER};
use serde_json::Value;

/// The transcript of the first row for `tool` whose recorded arguments carry
/// `needle`.
fn transcript_with<'a>(records: &'a [Value], tool: &str, needle: &str) -> &'a Value {
    records
        .iter()
        .find(|record| {
            record["tool"] == tool
                && record["transcript"]["arguments"]
                    .as_str()
                    .is_some_and(|arguments| arguments.contains(needle))
        })
        .unwrap_or_else(|| {
            panic!("no {tool} row records arguments containing {needle:?}: {records:?}")
        })
        .get("transcript")
        .expect("the row carries a transcript object")
}

// The shell half runs a real command in the sandboxed lane, which only macOS
// proves; the Linux counterpart below records the host-lane refusal instead.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_write_and_a_shell_call_record_their_arguments_results_and_duration() {
    let provider = Provider::transcript();
    let (fixture, _job) = drive(
        "diag-transcript-calls",
        &provider,
        vec![TRANSCRIPT_PATH.into()],
    )
    .await;
    let records = journal::read_journal(&fixture.root, "diag-transcript-calls");
    journal::assert_bounded_metadata(&records, "diag-transcript-calls");
    let write = transcript_with(&records, "write", TRANSCRIPT_PATH);
    assert_eq!(
        write["call_id"], "call-write",
        "the transcript names the provider's call id: {write}"
    );
    assert!(
        write["duration_ms"].as_u64().is_some(),
        "the transcript records how long the call took: {write}"
    );
    let write_result = write["result"].as_str().unwrap_or_else(|| {
        panic!("a completed write records the result the tool returned: {write}")
    });
    assert!(
        write_result.contains(TRANSCRIPT_PATH),
        "the recorded result is the write's own return: {write}"
    );
    let shell = transcript_with(&records, "shell", TRANSCRIPT_COMMAND);
    assert_eq!(
        shell["call_id"], "call-shell",
        "the transcript names the provider's call id: {shell}"
    );
    assert!(
        shell["arguments"]
            .as_str()
            .is_some_and(|arguments| arguments.contains(TRANSCRIPT_STDOUT)),
        "the recorded arguments carry the command the model sent: {shell}"
    );
    assert!(
        shell["result"]
            .as_str()
            .is_some_and(|result| result.contains(TRANSCRIPT_STDOUT)),
        "the recorded result carries the command's stdout: {shell}"
    );
    // Both calls belong to the helper that made them.
    for tool in ["write", "shell"] {
        let row = records
            .iter()
            .find(|record| record["tool"] == tool && record["transcript"].is_object())
            .unwrap_or_else(|| panic!("no transcript row for {tool}: {records:?}"));
        assert_eq!(row["agent_id"], "child-c1", "{row}");
        assert!(
            row["invocation_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
    }
}

#[tokio::test]
async fn an_over_cap_field_is_bounded_with_the_elision_marker() {
    let provider = Provider::transcript();
    let (fixture, _job) = drive(
        "diag-transcript-bounded",
        &provider,
        vec![TRANSCRIPT_PATH.into()],
    )
    .await;
    let records = journal::read_journal(&fixture.root, "diag-transcript-bounded");
    journal::assert_bounded_metadata(&records, "diag-transcript-bounded");
    let big = transcript_with(&records, "write", TRANSCRIPT_BIG_HEAD);
    let arguments = big["arguments"].as_str().unwrap();
    assert!(
        arguments.len() <= MAX_TOOL_TRANSCRIPT_FIELD_BYTES,
        "the recorded arguments stay inside the bound: {} bytes",
        arguments.len()
    );
    assert!(
        arguments.contains(TOOL_TRANSCRIPT_ELISION_MARKER),
        "a cut field says what it dropped: {arguments:?}"
    );
    assert!(
        arguments.contains(TRANSCRIPT_BIG_HEAD) && arguments.contains(TRANSCRIPT_BIG_TAIL),
        "both ends of the field are kept: {arguments:?}"
    );
    assert!(
        arguments.contains(TRANSCRIPT_PATH),
        "the call's own path survives the cut: {arguments:?}"
    );
}

#[tokio::test]
async fn a_secret_shaped_value_in_a_recorded_argument_or_result_is_redacted() {
    let provider = Provider::transcript();
    let (fixture, _job) = drive(
        "diag-transcript-secret",
        &provider,
        vec![TRANSCRIPT_PATH.into()],
    )
    .await;
    let records = journal::read_journal(&fixture.root, "diag-transcript-secret");
    journal::assert_bounded_metadata(&records, "diag-transcript-secret");
    let write = transcript_with(&records, "write", TRANSCRIPT_PATH);
    assert!(
        !write["arguments"].as_str().unwrap().contains("hunter2"),
        "the store's redactor scrubs the keyed value the model sent: {write}"
    );
    let shell = transcript_with(&records, "shell", TRANSCRIPT_COMMAND);
    assert!(
        !shell["arguments"].as_str().unwrap().contains("abc123"),
        "the store's redactor scrubs a credential header in an argument: {shell}"
    );
    assert!(
        !shell["result"].as_str().unwrap().contains("abc123"),
        "the store's redactor scrubs a credential header in a result: {shell}"
    );
    let text = journal::retained_text(&fixture.root, "diag-transcript-secret");
    assert!(
        text.contains("[redacted]"),
        "the scrubbed text names the replacement: {text}"
    );
    assert!(
        !text.contains(TRANSCRIPT_SECRET_ARGUMENT) && !text.contains(TRANSCRIPT_SECRET_RESULT),
        "no planted secret survives in the retained evidence"
    );
}

/// The compact arguments bound the writer states: every call keeps at most
/// this much of what the model sent, whatever the result-body budget says.
const COMPACT_ARGUMENT_BYTES: usize = 512;

#[tokio::test]
async fn every_call_after_the_result_budget_is_spent_keeps_its_compact_facts() {
    let provider = Provider::overflow();
    // A write path gives the root its working checkout, which is what
    // advertises the workspace tools the reads use.
    let (fixture, job) = drive_seeded(
        "diag-transcript-overflow",
        &provider,
        vec![OVERFLOW_PATH.into()],
        OVERFLOW_PATH,
        overflow_body(),
    )
    .await;
    let records = journal::read_journal(&fixture.root, "diag-transcript-overflow");
    journal::assert_bounded_metadata(&records, "diag-transcript-overflow");
    let reads: Vec<&Value> = records
        .iter()
        .filter(|record| record["tool"] == "read" && record["status"] == "completed")
        .collect();
    assert_eq!(
        reads.len(),
        OVERFLOW_CALLS,
        "every scripted read is recorded: {:?}",
        records
            .iter()
            .map(|record| (
                record["tool"].as_str().unwrap_or("?").to_owned(),
                record["status"].as_str().unwrap_or("?").to_owned()
            ))
            .collect::<Vec<_>>()
    );
    // Every call keeps the compact facts, including the calls whose result
    // body the budget could not hold.
    for record in &reads {
        let transcript = record
            .get("transcript")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("every call records compact facts: {record}"));
        assert!(
            transcript["duration_ms"].as_u64().is_some(),
            "every call records how long it took: {record}"
        );
        let arguments = transcript["arguments"]
            .as_str()
            .unwrap_or_else(|| panic!("every call records its arguments: {record}"));
        assert!(
            !arguments.is_empty() && arguments.len() <= COMPACT_ARGUMENT_BYTES,
            "the compact arguments stay inside the stated bound: {} bytes",
            arguments.len()
        );
        assert!(
            serde_json::to_vec(record).unwrap().len() < journal::MAX_LINE_BYTES,
            "a compact row stays inside the record framing: {record}"
        );
    }
    let marker_at = records
        .iter()
        .position(|record| record["tool"] == journal::TRANSCRIPT_MARKER_TOOL)
        .expect("the overflow writes one marker row");
    let dropped_from = records[marker_at]["result_bodies_dropped_from_call"]
        .as_u64()
        .expect("the marker names the first call whose body was dropped");
    assert!(
        dropped_from > 1 && dropped_from <= OVERFLOW_CALLS as u64,
        "the marker names a real call: {dropped_from}"
    );
    assert!(
        records[..marker_at]
            .iter()
            .any(|record| record["transcript"]["result"].is_string()),
        "the budget holds bodies before it runs out"
    );
    for record in &records[marker_at + 1..] {
        if let Some(transcript) = record.get("transcript") {
            assert!(
                transcript.get("result").is_none(),
                "no result body is recorded after the marker: {record}"
            );
        }
    }
    // The same compact rows reach `job export`'s bounded diagnostic page.
    let exported = fixture
        .application
        .export(job.job_id(), None, None)
        .await
        .unwrap();
    let diagnostic_export = exported["artifacts"]["diagnostics"].to_string();
    assert!(
        diagnostic_export.contains("result_bodies_dropped_from_call"),
        "the export carries the marker row"
    );
    assert!(
        diagnostic_export.contains(OVERFLOW_PATH) && diagnostic_export.contains("duration_ms"),
        "the export carries the compact facts of every call"
    );
}

#[tokio::test]
async fn a_row_written_before_the_transcript_existed_still_parses() {
    let provider = Provider::status();
    let (fixture, _job) = drive("diag-transcript-old", &provider, vec![]).await;
    // A record written before the transcript existed carries none of its keys
    // and no `transcript` object at all.
    let path = journal::journal_path(&fixture.root, "diag-transcript-old");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(
        br#"{"job_id":"diag-transcript-old","agent_id":null,"invocation_id":null,"epoch":0,"tool":"coding_status","status":"completed","reason":"executor_completed","source":"actual_tool_loop"}
"#,
    );
    std::fs::write(&path, bytes).unwrap();
    let records = journal::read_journal(&fixture.root, "diag-transcript-old");
    let old = records
        .iter()
        .find(|record| record["epoch"] == 0)
        .unwrap_or_else(|| panic!("the old line must still parse: {records:?}"));
    assert!(old.get("transcript").is_none(), "{old}");
    journal::assert_bounded_metadata(&records, "diag-transcript-old");
}

// The Linux counterpart: the sandboxed lane is not offered, so the helper's
// `shell` is refused on the host lane. The transcript still records the call's
// arguments, its duration, and the refusal the model received.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_refused_shell_call_still_records_its_arguments_result_and_duration() {
    let provider = Provider::transcript();
    let (fixture, _job) = drive(
        "diag-transcript-refused",
        &provider,
        vec![TRANSCRIPT_PATH.into()],
    )
    .await;
    let records = journal::read_journal(&fixture.root, "diag-transcript-refused");
    let shell = transcript_with(&records, "shell", TRANSCRIPT_COMMAND);
    assert_eq!(shell["call_id"], "call-shell", "{shell}");
    assert!(
        shell["duration_ms"].as_u64().is_some(),
        "the refused call still records how long it took: {shell}"
    );
    assert!(
        shell["result"]
            .as_str()
            .is_some_and(|result| result.contains("no operator is available")),
        "the recorded result carries the documented host-lane refusal: {shell}"
    );
    let write = transcript_with(&records, "write", TRANSCRIPT_PATH);
    assert!(
        write["result"]
            .as_str()
            .is_some_and(|result| result.contains(TRANSCRIPT_PATH)),
        "the write's own result is unaffected: {write}"
    );
}
