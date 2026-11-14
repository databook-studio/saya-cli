/// A first request cut twice is still the same run: the two recoveries are
/// consecutive cuts, well inside the consecutive-cut bound, and the third
/// request completes. The request after a cut is never treated as the run's
/// first request, so it does not go out at High again.
#[tokio::test]
async fn a_first_request_cut_twice_then_succeeds_completes() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CutThenAnswerProvider {
        cuts: 2,
        requests: requests.clone(),
        calls: Mutex::new(0),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("two consecutive cuts are recovered and the run completes");
    assert_eq!(output.answer, "final answer");

    let captured = requests.lock().unwrap();
    assert_eq!(
        captured.len(),
        3,
        "two cuts recovered, then the answer"
    );
    assert_eq!(
        captured[0].reasoning_effort,
        saya_agent::ReasoningEffort::High,
        "the run's first request thinks hard"
    );
    for (index, request) in captured.iter().enumerate().skip(1) {
        assert_eq!(
            request.reasoning_effort,
            saya_agent::ReasoningEffort::Low,
            "request {index} follows a cut and is not the run's first request"
        );
    }

    let seen = events.lock().unwrap();
    let cuts = seen
        .iter()
        .filter(|event| matches!(event, AgentEvent::StreamCutRecovered { .. }))
        .count();
    assert_eq!(cuts, 2, "each recovered cut is reported: {seen:?}");
}
