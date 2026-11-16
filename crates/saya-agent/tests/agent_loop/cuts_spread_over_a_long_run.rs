/// Four cuts spread over a long run — each followed by a completed reply — do
/// not end the job. The bound is on *consecutive* cuts, so a completed reply
/// resets it; only an unbroken run of cuts exhausts the allowance.
#[tokio::test]
async fn cuts_spread_over_a_long_run_never_end_the_job() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = AlternatingCutProvider {
        answer_at: 7,
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
    .expect("cuts separated by completed replies never exhaust the run");
    assert_eq!(output.answer, "final answer");

    let captured = requests.lock().unwrap();
    assert_eq!(
        captured.len(),
        8,
        "four cuts and four completed replies before the answer"
    );
    let seen = events.lock().unwrap();
    let cuts = seen
        .iter()
        .filter(|event| matches!(event, AgentEvent::StreamCutRecovered { .. }))
        .count();
    assert_eq!(cuts, 4, "every spread-out cut is recovered: {seen:?}");
    assert!(
        !seen
            .iter()
            .any(|event| matches!(event, AgentEvent::TurnReset)),
        "a cut is answered with a note, never a replay"
    );
}
