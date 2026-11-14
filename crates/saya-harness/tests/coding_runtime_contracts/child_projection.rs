//! What one child's projected events carry: identity, progress and bounded
//! outcome words — never the arguments the child sent, the result it received
//! or its answer. The transcript the evidence writer records rides beside the
//! projection, in the same message, and is not part of it.
use super::{drain, support};
use saya_agent::CancellationToken;
use saya_harness::coding::coordinator::{ChildEventKind, child_event_channel, run_child};

/// Planted by the scripted child in both the call's arguments and its result.
const ARGUMENT_SENTINEL: &str = "PRIVATE-TOOL-ARG-SENTINEL";
const RESULT_SENTINEL: &str = "PRIVATE-TOOL-RESULT-SENTINEL";

#[tokio::test]
async fn a_tool_call_projects_its_identity_and_outcome_and_nothing_it_sent() {
    let factory = support::ScenarioFactory::new(1);
    let spec = support::spec(
        "agent-projection",
        "invocation-projection",
        support::request(),
        2,
    );
    let (sender, receiver) = child_event_channel(&spec);
    let cancellation = CancellationToken::new();
    let (output, events) = tokio::join!(
        run_child(spec, &factory, sender, cancellation),
        drain(receiver),
    );
    assert!(output.is_ok(), "the child run must complete: {output:?}");
    for event in &events {
        let projected = String::from_utf8(event.to_json_bytes().unwrap()).unwrap();
        assert!(!projected.contains(ARGUMENT_SENTINEL), "{projected}");
        assert!(!projected.contains(RESULT_SENTINEL), "{projected}");
        assert!(!projected.contains("child answer"), "{projected}");
    }
    let projected = events
        .iter()
        .filter(|event| matches!(event.kind(), ChildEventKind::ToolDiagnostic { .. }))
        .map(|event| String::from_utf8(event.to_json_bytes().unwrap()).unwrap())
        .collect::<Vec<_>>();
    // One `inspect` call: the dispatch fact and its terminal outcome, each
    // projected exactly as it was before a transcript existed — the identity
    // envelope, the source-owned name, the status and the closed outcome word,
    // and nothing else.
    assert_eq!(
        projected,
        vec![
            r#"{"job_id":"job-a","agent_id":"agent-projection","invocation_id":"invocation-projection","kind":{"kind":"tool_diagnostic","diagnostic":{"tool":"inspect","status":"requested","reason":"dispatch_requested"}}}"#,
            r#"{"job_id":"job-a","agent_id":"agent-projection","invocation_id":"invocation-projection","kind":{"kind":"tool_diagnostic","diagnostic":{"tool":"inspect","status":"completed","reason":"executor_completed"}}}"#,
        ]
    );
    // The transcript the evidence writer records travels beside that projection
    // rather than inside it, so the child's call is still recorded in full.
    let transcripts = events
        .iter()
        .filter_map(|event| match event.kind() {
            ChildEventKind::ToolDiagnostic { diagnostic } => diagnostic.transcript(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        transcripts.len(),
        1,
        "only the terminal row of the call carries the transcript"
    );
    let transcript = transcripts[0];
    assert_eq!(transcript.call_id(), "tool-0");
    assert!(transcript.arguments().contains(ARGUMENT_SENTINEL));
    assert!(
        transcript
            .result()
            .is_some_and(|result| result.contains(RESULT_SENTINEL)),
        "the writer is handed the result the tool returned: {transcript:?}"
    );
}
