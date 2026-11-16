use super::*;

#[tokio::test]
async fn one_request_ceiling_covers_ordinary_recovery_continuation_and_salvage() {
    struct Case {
        name: &'static str,
        attempts: Vec<Turn>,
        limits: AgentLimits,
        definitions: Vec<ToolDefinition>,
        expected_batches: Vec<u64>,
        expected_effects: usize,
    }

    let salvage_limits = AgentLimits {
        max_tool_calls: Some(0),
        ..AgentLimits::default()
    };
    let cases = vec![
        Case {
            name: "ordinary",
            attempts: vec![
                turn("", vec![query_call("q1")]),
                turn("must not be requested", Vec::new()),
            ],
            limits: AgentLimits::default(),
            definitions: definitions(),
            expected_batches: vec![1],
            expected_effects: 1,
        },
        Case {
            name: "continuation",
            attempts: vec![
                vec![Err(ProviderError::output_truncated(
                    "partial".into(),
                    Vec::new(),
                ))],
                turn("must not be requested", Vec::new()),
            ],
            limits: AgentLimits::default(),
            definitions: definitions(),
            expected_batches: Vec::new(),
            expected_effects: 0,
        },
        Case {
            name: "clarification recovery",
            attempts: vec![
                turn(
                    "",
                    vec![call("ask", "request_clarification", serde_json::json!({}))],
                ),
                turn("must not be requested", Vec::new()),
            ],
            limits: AgentLimits::default(),
            definitions: definitions(),
            expected_batches: vec![1],
            expected_effects: 0,
        },
        Case {
            name: "designation recovery",
            attempts: vec![turn(
                "",
                vec![call(
                    "designation",
                    "designate_answer",
                    serde_json::json!({"sql": ""}),
                )],
            )],
            limits: AgentLimits::default(),
            definitions: definitions(),
            expected_batches: vec![1],
            expected_effects: 0,
        },
        Case {
            name: "legacy tool-ceiling salvage",
            attempts: vec![
                turn("", vec![query_call("q1")]),
                turn("must not be requested", Vec::new()),
            ],
            limits: salvage_limits,
            definitions: definitions(),
            expected_batches: vec![1],
            expected_effects: 0,
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
                definitions: case.definitions,
                limits: case.limits,
                answering: answering_limits(Some(1), None, None),
            },
            &AllowReadOnlyApproval,
            &sink,
            cancellation,
        )
        .await
        .unwrap_or_else(|error| panic!("{}: {error}", case.name));

        assert_eq!(requests.lock().unwrap().len(), 1, "{}", case.name);
        assert_eq!(
            executor.calls.lock().unwrap().len(),
            case.expected_effects,
            "{}",
            case.name
        );
        assert!(
            stopped(&output, BudgetDimension::LogicalRequests),
            "{}",
            case.name
        );
        let observations = sink.observations.lock().unwrap();
        assert_eq!(
            observations.first(),
            Some(&AnsweringObservation::Initialized)
        );
        assert_eq!(
            observations
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
        assert_eq!(
            observations
                .iter()
                .filter(|observation| matches!(observation, AnsweringObservation::RequestAdmitted))
                .count(),
            1,
            "{}",
            case.name
        );
    }
}

#[tokio::test]
async fn an_established_stream_retry_is_one_logical_request() {
    let (provider, requests) = ScriptedProvider::new(vec![
        vec![
            Ok(ProviderEvent::TextDelta("discarded".into())),
            Err(ProviderError::Request("stream interrupted".into())),
        ],
        turn("answered after retry", Vec::new()),
    ]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let output = controlled_run(
        &provider,
        &RecordingExecutor::default(),
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(1), None, None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect("the internal retry is part of one admitted request");

    assert_eq!(output.answer, "answered after retry");
    assert_eq!(requests.lock().unwrap().len(), 2, "both attempts ran");
    assert_eq!(
        sink.observations
            .lock()
            .unwrap()
            .iter()
            .filter(|observation| matches!(observation, AnsweringObservation::RequestAdmitted))
            .count(),
        1
    );
}

#[tokio::test]
async fn failed_request_observation_prevents_provider_admission() {
    let (provider, requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::cancelling_on(cancellation.clone(), CancelOnObservation::Request);
    let error = controlled_run(
        &provider,
        &RecordingExecutor::default(),
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(1), None, None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation.clone(),
    )
    .await
    .expect_err("owner observation failure fails closed");

    assert!(matches!(error, AgentError::Cancelled));
    assert_eq!(requests.lock().unwrap().len(), 0);
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure)
    );
}
