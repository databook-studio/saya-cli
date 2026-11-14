/// Recovery: a truncation on turn 1 followed by a cooperative turn completes
/// the run. The partial is discarded, never replayed: it appears nowhere in
/// the messages the provider received on turn 2, and exactly one user-role
/// continuation note does.
#[tokio::test]
async fn continuation_recovers_after_truncation_and_discards_the_partial() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = continuation_script(
        &requests,
        false,
        "TRUNCATED-PROSE-SENTINEL-7QZ",
        vec!["PARTIAL-TOOL-SENTINEL-7QZ".into()],
    );
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .expect("the run recovers after one truncation");
    assert_eq!(output.answer, "cooperative answer");
    let seen = requests.lock().unwrap().clone();
    assert_eq!(
        seen.len(),
        2,
        "one truncated turn plus one cooperative turn"
    );
    let turn_two = seen[1].messages.clone();
    for message in &turn_two {
        assert!(
            !message.content.contains("TRUNCATED-PROSE-SENTINEL-7QZ"),
            "the partial text must never be replayed: {}",
            message.content
        );
        assert!(
            !message.content.contains("PARTIAL-TOOL-SENTINEL-7QZ"),
            "the partial tool JSON must never be replayed: {}",
            message.content
        );
        let calls = serde_json::to_string(&message.tool_calls).unwrap();
        assert!(
            !calls.contains("TRUNCATED-PROSE-SENTINEL-7QZ")
                && !calls.contains("PARTIAL-TOOL-SENTINEL-7QZ"),
            "no partial may hide in a replayed tool call: {calls}"
        );
    }
    assert_eq!(
        continuation_notes(&turn_two),
        1,
        "exactly one user-role continuation note on turn 2"
    );
}
