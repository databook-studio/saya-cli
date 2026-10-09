use super::*;

#[tokio::test]
async fn shared_scope_request_limit_stops_second_invocation_before_provider() {
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(answering_limits(Some(1), None, None), cancellation.clone());
    let sink = RecordingSink::new(cancellation);
    let first = ScriptedProvider::new(vec![turn("first answer", Vec::new())]).0;
    let first_output = run_in_scope(
        &first,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(50), Some(50), Some(50)),
    )
    .await
    .expect("the first admitted invocation answers");

    let (second, second_requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let stopped_output = run_in_scope(
        &second,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(50), Some(50), Some(50)),
    )
    .await
    .expect("the scope returns a typed pre-work budget refusal");

    assert_eq!(first_output.answer, "first answer");
    assert!(stopped(&stopped_output, BudgetDimension::LogicalRequests));
    assert!(stopped_output.answer.is_empty());
    assert!(stopped_output.truncated);
    assert!(second_requests.lock().unwrap().is_empty());
    let observations = sink.observations.lock().unwrap();
    assert_eq!(request_count(&observations), 1);
    assert_eq!(
        observations
            .iter()
            .filter(|observation| matches!(observation, AnsweringObservation::Initialized))
            .count(),
        1,
        "reusing a scope must not reinitialize its spent counts"
    );
    assert_eq!(
        scope.cancellation_token().reason(),
        Some(CancellationReason::Budget(BudgetDimension::LogicalRequests))
    );
}

#[tokio::test]
async fn shared_scope_tool_limit_refuses_the_crossing_batch_before_effects() {
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(
        answering_limits(Some(8), Some(1), None),
        cancellation.clone(),
    );
    let sink = RecordingSink::new(cancellation);
    let executor = RecordingExecutor::default();
    let (first, _) = ScriptedProvider::new(vec![
        turn("", vec![query_call("first-query")]),
        turn("first attempt finished", Vec::new()),
    ]);
    run_in_scope(
        &first,
        &executor,
        &sink,
        &scope,
        answering_limits(Some(8), Some(8), None),
    )
    .await
    .expect("the first tool call fits the question scope");

    let (second, second_requests) =
        ScriptedProvider::new(vec![turn("second prose", vec![query_call("second-query")])]);
    let output = run_in_scope(
        &second,
        &executor,
        &sink,
        &scope,
        answering_limits(Some(8), Some(8), None),
    )
    .await
    .expect("the crossing batch becomes a typed partial result");

    assert_eq!(second_requests.lock().unwrap().len(), 1);
    assert!(stopped(&output, BudgetDimension::RequestedToolCalls));
    assert_eq!(
        executor.calls.lock().unwrap().as_slice(),
        ["query"],
        "the complete crossing batch has no prefix effect"
    );
    assert_eq!(
        batch_counts(&sink.observations.lock().unwrap()),
        [1, 0, 1],
        "complete source batches include the natural zero-call answer"
    );
}
