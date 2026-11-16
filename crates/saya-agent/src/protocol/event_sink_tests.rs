//! The failure detail a denied, failed or refused call carries: a digest of the
//! arguments, a closed class, its closed rendering, and the bounded tool-result
//! error text the model received — plus the bounded transcript of the call
//! itself, which stays out of the projection the diagnostic serializes to and
//! which a payload too large to deliver drops rather than the metadata it must
//! carry.
use super::{
    MAX_TOOL_DIAGNOSTIC_BYTES, MAX_TOOL_REASON_BYTES, TOOL_REASON_TRUNCATION_MARKER,
    ToolDiagnostic, ToolDiagnosticStatus,
};
use crate::{MAX_TOOL_TRANSCRIPT_FIELD_BYTES, ToolCallTranscript};

/// A transcript whose two texts are as long as one field may be.
fn max_transcript() -> ToolCallTranscript {
    ToolCallTranscript::new(
        "call-1",
        &"a".repeat(MAX_TOOL_TRANSCRIPT_FIELD_BYTES),
        Some(&"b".repeat(MAX_TOOL_TRANSCRIPT_FIELD_BYTES)),
        7,
    )
}

const REFUSAL_TEXT: &str = "unknown tool 'probe'; available tools: read, write";

fn diagnostic(status: ToolDiagnosticStatus) -> ToolDiagnostic {
    ToolDiagnostic::new(None, status)
        .with_failure_detail(r#"{"path":"secret.txt"}"#, || REFUSAL_TEXT.to_owned())
}

#[test]
fn a_failing_status_carries_the_closed_class_and_its_rendering() {
    for (status, class, text) in [
        (
            ToolDiagnosticStatus::Denied,
            "approval_denied",
            "the call was denied by approval policy",
        ),
        (
            ToolDiagnosticStatus::Refused,
            "admission_refused",
            "the effect admission refused the call before execution",
        ),
        (
            ToolDiagnosticStatus::Failed,
            "executor_error",
            "the tool executor returned an error",
        ),
    ] {
        let detail = diagnostic(status)
            .detail()
            .expect("a failing status carries a detail")
            .clone();
        assert_eq!(detail.failure_class(), class);
        assert_eq!(detail.error_text(), text);
        assert!(
            detail.error_text().len() <= 200,
            "the closed rendering stays inside its bound"
        );
    }
}

#[test]
fn a_failing_status_carries_the_tool_result_error_text_the_model_received() {
    for status in [
        ToolDiagnosticStatus::Denied,
        ToolDiagnosticStatus::Refused,
        ToolDiagnosticStatus::Failed,
    ] {
        let detail = diagnostic(status).detail().expect("a detail").clone();
        assert_eq!(
            detail.reason_text(),
            REFUSAL_TEXT,
            "{status:?} must record the exact text the model received"
        );
    }
}

#[test]
fn a_reason_is_cut_to_the_bound_on_a_utf8_boundary_with_a_trailing_marker() {
    // A multi-byte character straddles the cut: the bound must land on a
    // boundary, so the cut can only ever fall short of it.
    let long = format!("{}{}", "é".repeat(3 * 1024), "tail");
    let diagnostic = ToolDiagnostic::new(None, ToolDiagnosticStatus::Failed)
        .with_failure_detail("{}", || long.clone());
    let reason_text = diagnostic.detail().expect("a detail").reason_text();
    assert!(
        reason_text.len() <= MAX_TOOL_REASON_BYTES,
        "the reason text stays inside its bound: {} bytes",
        reason_text.len()
    );
    assert!(
        reason_text.ends_with(TOOL_REASON_TRUNCATION_MARKER),
        "a cut reason text says so: {reason_text:?}"
    );
    assert!(
        reason_text.starts_with("ééé"),
        "the kept prefix is the text's own start: {reason_text:?}"
    );
    assert!(
        reason_text.is_char_boundary(reason_text.len()),
        "the cut lands on a UTF-8 boundary"
    );
    // A reason text inside the bound is kept whole and unmarked.
    let short = ToolDiagnostic::new(None, ToolDiagnosticStatus::Failed)
        .with_failure_detail("{}", || "the tool executor returned an error".to_owned());
    let reason_text = short.detail().expect("a detail").reason_text();
    assert_eq!(reason_text, "the tool executor returned an error");
    assert!(!reason_text.contains(TOOL_REASON_TRUNCATION_MARKER));
}

#[test]
fn the_result_error_text_is_the_results_own_error_string() {
    assert_eq!(
        ToolDiagnostic::result_error_text(&serde_json::json!({"error": "denied by policy"})),
        "denied by policy"
    );
    // A result without an `error` string still yields the text the model saw.
    assert_eq!(
        ToolDiagnostic::result_error_text(&serde_json::json!({"outcome": "refused"})),
        r#"{"outcome":"refused"}"#
    );
}

#[test]
fn a_succeeding_or_requested_call_carries_no_detail() {
    for status in [
        ToolDiagnosticStatus::Requested,
        ToolDiagnosticStatus::Completed,
    ] {
        assert!(
            diagnostic(status).detail().is_none(),
            "{status:?} must not carry a failure detail"
        );
    }
}

#[test]
fn the_digest_identifies_the_arguments_without_carrying_them() {
    let denied = diagnostic(ToolDiagnosticStatus::Denied);
    let detail = denied.detail().expect("a denied call carries a detail");
    let digest = detail.arguments_sha256();
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
        "the digest is lowercase sha256 hex: {digest}"
    );
    assert!(
        !digest.contains("secret.txt"),
        "the digest carries no argument text"
    );
    // The digest is exactly the sha256 of the serialized arguments, so two
    // different argument strings never share one.
    assert_eq!(
        digest,
        saya_types::sha256_hex(br#"{"path":"secret.txt"}"#.as_slice())
    );
    let other = ToolDiagnostic::new(None, ToolDiagnosticStatus::Denied)
        .with_failure_detail(r#"{"path":"other.txt"}"#, || REFUSAL_TEXT.to_owned());
    assert_ne!(
        digest,
        other.detail().expect("a detail").arguments_sha256(),
        "different arguments must not share a digest"
    );
}

#[test]
fn a_diagnostic_that_carries_a_transcript_stays_inside_its_delivery_bound() {
    let diagnostic = ToolDiagnostic::new(None, ToolDiagnosticStatus::Failed)
        .with_failure_detail("{}", || REFUSAL_TEXT.to_owned())
        .with_transcript(max_transcript());
    let transcript = diagnostic.transcript().expect("the transcript survives");
    let payload = serde_json::to_vec(&diagnostic).unwrap().len()
        + serde_json::to_vec(transcript).unwrap().len();
    assert!(
        payload <= MAX_TOOL_DIAGNOSTIC_BYTES,
        "a delivered call fits the child message bound: {payload} bytes"
    );
}

/// The arguments a child sent and the result it received, planted where the
/// projection must never carry them.
const ARGUMENT_SENTINEL: &str = "PRIVATE-TOOL-ARG-SENTINEL";
const RESULT_SENTINEL: &str = "PRIVATE-TOOL-RESULT-SENTINEL";

#[test]
fn the_projection_of_a_call_never_carries_its_transcript() {
    let carried = ToolDiagnostic::new(None, ToolDiagnosticStatus::Completed).with_transcript(
        ToolCallTranscript::new(
            "call-1",
            &format!(r#"{{"path":"src/owned.rs","secret":"{ARGUMENT_SENTINEL}"}}"#),
            Some(&format!(
                r#"{{"status":"inspected","row":"{RESULT_SENTINEL}"}}"#
            )),
            3,
        ),
    );
    // The evidence writer reads the transcript off the value...
    let transcript = carried.transcript().expect("the loop recorded the call");
    assert!(transcript.arguments().contains(ARGUMENT_SENTINEL));
    assert!(
        transcript
            .result()
            .is_some_and(|result| result.contains(RESULT_SENTINEL))
    );
    // ...and the projection a child's event carries is byte-for-byte the call
    // as it was before a transcript existed: no arguments, no result.
    let projected = serde_json::to_vec(&carried).unwrap();
    let text = String::from_utf8(projected.clone()).unwrap();
    assert!(!text.contains(ARGUMENT_SENTINEL), "{text}");
    assert!(!text.contains(RESULT_SENTINEL), "{text}");
    assert_eq!(
        text,
        r#"{"tool":"unknown_tool","status":"completed","reason":"executor_completed"}"#
    );
    assert_eq!(
        projected,
        serde_json::to_vec(&ToolDiagnostic::new(None, ToolDiagnosticStatus::Completed)).unwrap(),
        "a transcript must not change one byte of the projection"
    );
}

#[test]
fn a_transcript_that_would_not_be_delivered_is_dropped_whole() {
    // A reason text of raw control characters escapes to six bytes each, so this
    // detail plus a maximal transcript cannot travel as one child event: the
    // transcript goes and the call keeps everything a postmortem needs to name
    // it.
    let pathological = "\u{1}".repeat(MAX_TOOL_REASON_BYTES);
    let diagnostic = ToolDiagnostic::new(None, ToolDiagnosticStatus::Failed)
        .with_failure_detail("{}", || pathological)
        .with_transcript(max_transcript());
    assert!(
        diagnostic.transcript().is_none(),
        "the content is dropped rather than the call"
    );
    let detail = diagnostic.detail().expect("the detail survives");
    assert_eq!(detail.reason_text().len(), MAX_TOOL_REASON_BYTES);
    assert!(detail.arguments_sha256().len() == 64);
    assert!(serde_json::to_vec(&diagnostic).unwrap().len() <= MAX_TOOL_DIAGNOSTIC_BYTES);
    // The same call without the pathological reason text keeps its transcript.
    let delivered = ToolDiagnostic::new(None, ToolDiagnosticStatus::Failed)
        .with_failure_detail("{}", || REFUSAL_TEXT.to_owned())
        .with_transcript(max_transcript());
    assert!(delivered.transcript().is_some());
}
