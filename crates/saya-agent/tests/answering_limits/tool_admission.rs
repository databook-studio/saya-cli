use super::*;

#[tokio::test]
async fn over_headroom_batches_are_refused_before_shortcuts_or_effects() {
    struct Case {
        name: &'static str,
        attempts: Vec<Turn>,
        limits: AnsweringLimits,
        expected_batches: Vec<u64>,
        expected_effects: Vec<&'static str>,
        expected_answer: &'static str,
    }

    let cases = vec![
        Case {
            name: "designation shortcut",
            attempts: vec![turn(
                "partial answer",
                vec![designation_call("designation")],
            )],
            limits: answering_limits(Some(2), Some(0), None),
            expected_batches: vec![1],
            expected_effects: vec![],
            expected_answer: "partial answer",
        },
        Case {
            name: "clarification shortcut",
            attempts: vec![turn(
                "partial answer",
                vec![call(
                    "ask",
                    "request_clarification",
                    serde_json::json!({"question": "Which table?"}),
                )],
            )],
            limits: answering_limits(Some(2), Some(0), None),
            expected_batches: vec![1],
            expected_effects: vec![],
            expected_answer: "partial answer",
        },
        Case {
            name: "whole ordinary batch",
            attempts: vec![turn(
                "partial answer",
                vec![query_call("q1"), query_call("q2")],
            )],
            limits: answering_limits(Some(2), Some(1), None),
            expected_batches: vec![2],
            expected_effects: vec![],
            expected_answer: "partial answer",
        },
        Case {
            name: "multi-item batch after successful work",
            attempts: vec![
                turn_with_usage(
                    "first answer",
                    vec![query_call("q1")],
                    TokenUsage::new(4, 1),
                ),
                turn_with_usage(
                    "partial answer",
                    vec![query_call("q2"), query_call("q3")],
                    TokenUsage::new(6, 2),
                ),
            ],
            limits: answering_limits(Some(3), Some(2), None),
            expected_batches: vec![1, 2],
            expected_effects: vec!["query"],
            expected_answer: "partial answer",
        },
    ];

    for case in cases {
        let (provider, requests) = ScriptedProvider::new(case.attempts);
        let cancellation = CancellationToken::new();
        let sink = RecordingSink::new(cancellation.clone());
        let executor = RecordingExecutor::default();
        let output = controlled_run(
            &provider,
            &executor,
            RunSettings {
                definitions: definitions(),
                limits: AgentLimits::default(),
                answering: case.limits,
            },
            &AllowReadOnlyApproval,
            &sink,
            cancellation,
        )
        .await
        .unwrap_or_else(|error| panic!("{}: {error}", case.name));

        assert_eq!(
            requests.lock().unwrap().len(),
            case.expected_batches.len(),
            "{}",
            case.name
        );
        assert_eq!(
            executor.calls.lock().unwrap().as_slice(),
            case.expected_effects,
            "{}",
            case.name
        );
        assert_eq!(output.answer, case.expected_answer, "{}", case.name);
        assert!(output.truncated, "{}", case.name);
        assert!(
            stopped(&output, BudgetDimension::RequestedToolCalls),
            "{}",
            case.name
        );
        if case.expected_batches == [1] {
            assert!(
                !output.events.iter().any(|event| matches!(
                    event,
                    AgentEvent::AnswerDesignated { .. } | AgentEvent::ClarificationNeeded { .. }
                )),
                "shortcut ran before batch admission: {}",
                case.name
            );
        }
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
            case.expected_batches,
            "{}",
            case.name
        );
        if case.name == "multi-item batch after successful work" {
            assert_eq!(
                output.tool_metadata.len(),
                1,
                "prior tool evidence is retained"
            );
            assert_eq!(
                output.usage,
                TokenUsage::new(10, 3),
                "settled usage is retained"
            );
        }
    }
}

#[tokio::test]
async fn salvage_stops_on_a_tool_batch_over_answering_headroom() {
    let (provider, requests) = ScriptedProvider::new(vec![
        turn_with_usage(
            "first result",
            vec![query_call("q1")],
            TokenUsage::new(3, 4),
        ),
        turn_with_usage(
            "salvage prose",
            vec![query_call("unexpected")],
            TokenUsage::new(5, 6),
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
            limits: AgentLimits {
                max_turns: Some(1),
                max_tool_calls: Some(1),
                ..AgentLimits::default()
            },
            answering: answering_limits(Some(3), Some(1), None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect("over-headroom salvage returns typed bounded output");

    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "salvage is one paid request with room left"
    );
    assert_eq!(executor.calls.lock().unwrap().as_slice(), ["query"]);
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
        vec![1, 1]
    );
    assert!(output.truncated);
    assert!(stopped(&output, BudgetDimension::RequestedToolCalls));
    assert_eq!(
        output.tool_metadata.len(),
        1,
        "completed tool evidence survives"
    );
    assert_eq!(
        output.usage,
        TokenUsage::new(8, 10),
        "both settled call receipts survive"
    );
}

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
