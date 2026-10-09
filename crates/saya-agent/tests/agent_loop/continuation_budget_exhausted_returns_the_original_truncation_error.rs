/// Budget exhausted: with `max_continuations: Some(3)` and a provider that
/// truncates every turn, exactly 1 + 3 provider calls happen, then the run
/// returns the original truncation error — same payload, same `Display`.
#[tokio::test]
async fn continuation_budget_exhausted_returns_the_original_truncation_error() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = continuation_script(&requests, true, "partial", Vec::new());
    let limits = AgentLimits {
        max_continuations: Some(3),
        ..AgentLimits::default()
    };
    let error = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        limits,
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        requests.lock().unwrap().len(),
        4,
        "one initial turn plus three continuations"
    );
    let saya_agent::ProviderError::OutputTruncated {
        partial_text,
        partial_tool_json,
    } = saya_agent::ProviderError::output_truncated("partial".into(), Vec::new())
    else {
        unreachable!("the constructor builds the compared variant");
    };
    assert!(
        matches!(
            &error,
            AgentError::Provider(saya_agent::ProviderError::OutputTruncated { .. })
        ),
        "the original truncation error surfaces: {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "provider response truncated: the model hit its output-token limit",
        "the Display string is unchanged"
    );
    let AgentError::Provider(saya_agent::ProviderError::OutputTruncated {
        partial_text: got_text,
        partial_tool_json: got_json,
    }) = error
    else {
        unreachable!("matched above");
    };
    assert_eq!(got_text, partial_text, "the partial text is byte-identical");
    assert_eq!(
        got_json, partial_tool_json,
        "the partial tool JSON is byte-identical"
    );
}
