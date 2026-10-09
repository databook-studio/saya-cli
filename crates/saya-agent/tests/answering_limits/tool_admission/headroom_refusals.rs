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
