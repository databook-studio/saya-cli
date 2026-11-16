use super::*;

#[tokio::test]
async fn latest_failed_and_successful_usage_share_the_known_token_ceiling() {
    let (provider, requests) = ScriptedProvider::new(vec![
        vec![
            Ok(ProviderEvent::Usage(TokenUsage::new(9, 0))),
            Ok(ProviderEvent::Usage(TokenUsage::new(2, 1))),
            Err(ProviderError::Request("stream interrupted".into())),
        ],
        turn_with_usage(
            "partial answer",
            vec![query_call("must-not-run")],
            TokenUsage::new(3, 2),
        ),
    ]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let executor = RecordingExecutor::default();
    let output = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(3), Some(5), Some(8)),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect("the settled answer is retained while its tool effects are stopped");

    assert_eq!(requests.lock().unwrap().len(), 2, "one retry, one request");
    assert_eq!(output.answer, "partial answer");
    assert!(output.truncated);
    assert!(stopped(&output, BudgetDimension::ReportedTokens));
    assert!(executor.calls.lock().unwrap().is_empty());
    assert_eq!(output.usage, TokenUsage::new(5, 3));
    assert_eq!(
        output
            .events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::FailedAttemptUsage { usage } => Some(*usage),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![TokenUsage::new(2, 1)]
    );
}

#[tokio::test]
async fn absent_usage_stays_unknown_while_reported_zero_closes_a_zero_ceiling() {
    let (reported_provider, reported_requests) = ScriptedProvider::new(vec![turn_with_usage(
        "",
        vec![query_call("zero")],
        TokenUsage::default(),
    )]);
    let cancellation = CancellationToken::new();
    let reported_sink = RecordingSink::new(cancellation.clone());
    let reported_executor = RecordingExecutor::default();
    let reported = controlled_run(
        &reported_provider,
        &reported_executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), Some(1), Some(0)),
        },
        &AllowReadOnlyApproval,
        &reported_sink,
        cancellation,
    )
    .await
    .expect("reported zero is a settled report");
    assert_eq!(reported_requests.lock().unwrap().len(), 1);
    assert!(reported_executor.calls.lock().unwrap().is_empty());
    assert!(stopped(&reported, BudgetDimension::ReportedTokens));
    assert!(
        reported
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::Usage { .. }))
    );

    let (unknown_provider, unknown_requests) = ScriptedProvider::new(vec![
        turn("", vec![query_call("unknown")]),
        turn("completed with unknown usage", Vec::new()),
    ]);
    let cancellation = CancellationToken::new();
    let unknown_sink = RecordingSink::new(cancellation.clone());
    let unknown_executor = RecordingExecutor::default();
    let unknown = controlled_run(
        &unknown_provider,
        &unknown_executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), Some(1), Some(0)),
        },
        &AllowReadOnlyApproval,
        &unknown_sink,
        cancellation,
    )
    .await
    .expect("unknown usage does not become a reported zero");
    assert_eq!(unknown.answer, "completed with unknown usage");
    assert_eq!(unknown_requests.lock().unwrap().len(), 2);
    assert_eq!(unknown_executor.calls.lock().unwrap().as_slice(), ["query"]);
    assert!(!unknown.events.iter().any(|event| matches!(
        event,
        AgentEvent::AnsweringBudgetStopped {
            dimension: BudgetDimension::ReportedTokens
        }
    )));
    assert!(
        !unknown
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::Usage { .. }))
    );
}

#[path = "usage_cancellation/provider_cancellation.rs"]
mod provider_cancellation;
#[path = "usage_cancellation/tool_cancellation.rs"]
mod tool_cancellation;

#[test]
fn cancellation_reason_is_first_writer_wins() {
    let token = CancellationToken::new();
    assert!(token.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ReportedTokens,)));
    assert!(!token.cancel_with_reason(CancellationReason::User));
    assert_eq!(
        token.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ReportedTokens))
    );
    assert!(token.is_cancelled());
}
