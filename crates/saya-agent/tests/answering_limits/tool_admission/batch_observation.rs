use super::*;

#[tokio::test]
async fn invalid_whole_batch_is_observed_before_protocol_refusal_without_effects() {
    let (provider, requests) = ScriptedProvider::new(vec![turn(
        "",
        vec![query_call("valid-prefix"), query_call(" ")],
    )]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let executor = RecordingExecutor::default();
    let error = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(1), Some(0), None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect_err("invalid IDs remain a terminal protocol refusal");

    assert!(matches!(error, AgentError::InvalidToolCall));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(executor.calls.lock().unwrap().is_empty());
    assert_eq!(
        sink.observations.lock().unwrap().last(),
        Some(&AnsweringObservation::ToolBatchObserved { requested: 2 })
    );
}

#[tokio::test]
async fn denied_calls_are_counted_and_a_complete_empty_batch_is_explicit() {
    let mut denied_query = definition("query");
    denied_query.effect.requires_approval = true;
    let (provider, requests) = ScriptedProvider::new(vec![
        turn("", vec![query_call("denied")]),
        turn("finished", Vec::new()),
    ]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let executor = RecordingExecutor::default();
    let output = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: vec![denied_query, definition("request_clarification")],
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), Some(1), None),
        },
        &DenyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect("denial is fed back and the answer completes");

    assert_eq!(output.answer, "finished");
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(executor.calls.lock().unwrap().is_empty());
    assert!(output.events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolDenied { name, .. } if name == "query"
    )));
    assert_eq!(
        sink.observations
            .lock()
            .unwrap()
            .iter()
            .filter_map(|observation| match observation {
                AnsweringObservation::ToolBatchObserved { requested } => Some(*requested),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
}

#[tokio::test]
async fn failed_tool_batch_observation_prevents_shortcuts_and_effects() {
    let (provider, requests) = ScriptedProvider::new(vec![turn(
        "must not be trusted",
        vec![designation_call("designation"), query_call("query")],
    )]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::cancelling_on(cancellation.clone(), CancelOnObservation::ToolBatch);
    let executor = RecordingExecutor::default();
    let error = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), None, None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation.clone(),
    )
    .await
    .expect_err("failed batch observation prevents all later handling");

    assert!(matches!(error, AgentError::Cancelled));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure)
    );
    assert!(executor.calls.lock().unwrap().is_empty());
    assert!(
        !sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentEvent::AnswerDesignated { .. }))
    );
}
