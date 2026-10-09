use super::*;

/// The claim: a run capped mid-file-write resumes with the append variant
/// using the `size`/`digest` the harness reported, and the finished file is
/// byte-exact — chunk one followed by chunk two, no duplication, no gap.
#[tokio::test]
async fn a_capped_run_finishes_the_file_by_appending() {
    let sandbox = Sandbox::new("capped-append");
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let tools = SpiedTools {
        inner: sandbox.tools(),
        calls: tool_calls.clone(),
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CappedWriteProvider {
        requests: requests.clone(),
        calls: Mutex::new(0),
    };
    let output = saya_agent::run_agent_with_sink(
        &provider,
        &tools,
        request(),
        vec![workspace_edit_definition()],
        limits(),
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .expect("the capped run must finish after the resume");
    assert_eq!(
        output.answer, FINAL_ANSWER,
        "the run's final answer is the last turn's, not a salvaged one"
    );
    let expected = format!("{CHUNK_ONE}{CHUNK_TWO}");
    let actual = fs::read(sandbox.ws_root().join("report.txt")).expect("the file must exist");
    assert_eq!(
        actual,
        expected.as_bytes(),
        "the finished file is byte-exact: chunk one followed by chunk two"
    );
    let seen_calls = tool_calls.lock().unwrap().clone();
    assert_eq!(
        seen_calls.len(),
        2,
        "exactly two workspace_edit calls: the first chunk and the resume"
    );
    assert_eq!(
        seen_calls[1]["chunk"], CHUNK_TWO,
        "the resume sends only the second chunk, never resending turn 1's bytes"
    );
    assert!(
        !seen_calls[1].to_string().contains(CHUNK_ONE),
        "turn 1's bytes appear nowhere in the resume call: {}",
        seen_calls[1]
    );
    assert_eq!(
        seen_calls[1]["offset"],
        CHUNK_ONE.len() as u64,
        "the resume offset is the size turn 1's tool result reported"
    );
    let seen_requests = requests.lock().unwrap().clone();
    assert_eq!(
        seen_requests.len(),
        4,
        "first write, cap, resume, and final answer"
    );
    // The continuation note follows the cap: it is in the resume turn's
    // request (index 2), and the partial must be absent there.
    let turn_two = seen_requests[2].messages.clone();
    for message in &turn_two {
        assert!(
            !message.content.contains(PARTIAL_TEXT),
            "the capped turn's partial text appears nowhere the provider saw on the resume: {}",
            message.content
        );
        let calls = serde_json::to_string(&message.tool_calls).unwrap();
        assert!(
            !calls.contains(PARTIAL_TEXT) && !calls.contains(PARTIAL_TOOL_JSON),
            "no partial may hide in a replayed tool call: {calls}"
        );
    }
    assert!(
        !turn_two
            .iter()
            .any(|message| message.role == "tool" && message.content.contains(PARTIAL_TEXT)),
        "the partial is not smuggled in through a tool message either"
    );
    assert_eq!(
        continuation_notes(&turn_two),
        1,
        "the continuation note appears exactly once"
    );
    // The resume's precondition is the digest turn 1's tool result reported:
    // re-read that result from the resume request's own messages (what the
    // model saw) and compare against what the resume call actually sent.
    let reported = turn_one_result(&turn_two);
    assert!(
        reported["digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "the tool result the model sees carries the digest: {reported}"
    );
    assert_eq!(
        seen_calls[1]["expected_digest"], reported["digest"],
        "the resume echoes the digest the harness reported, not a guess"
    );
}
