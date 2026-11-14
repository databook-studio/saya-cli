/// Provider counts are untrusted numeric input: retries must saturate receipts
/// instead of panicking in debug builds or wrapping in release builds.
#[tokio::test]
async fn failed_and_successful_usage_saturates_at_u64_max() {
    let output = run_agent_with_sink(
        &MaxThenSuccess {
            attempts: Mutex::new(0),
        },
        &NoTools {
            calls: Arc::new(Mutex::new(0)),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("the retry completes");
    assert_eq!(output.usage.input_tokens, u64::MAX);
    assert_eq!(output.usage.output_tokens, u64::MAX);
    assert_eq!(output.usage.cached_input_tokens, Some(u64::MAX));
    assert_eq!(output.usage.cache_creation_input_tokens, Some(u64::MAX));
    assert_eq!(output.usage.reasoning_tokens, Some(u64::MAX));
}
