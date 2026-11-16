/// Disabled: `Some(0)` and `None` both surface the truncation immediately —
/// one provider call, no continuation note pushed.
#[tokio::test]
async fn continuation_disabled_surfaces_truncation_immediately() {
    for max_continuations in [Some(0), None] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = continuation_script(&requests, true, "partial", Vec::new());
        let limits = AgentLimits {
            max_continuations,
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
        assert!(
            matches!(
                error,
                AgentError::Provider(saya_agent::ProviderError::OutputTruncated { .. })
            ),
            "max_continuations={max_continuations:?}: the error surfaces: {error:?}"
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "max_continuations={max_continuations:?}: no second call, so no note was pushed"
        );
    }
}
