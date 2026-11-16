/// One model call is bounded by a total deadline. A stream that keeps flowing
/// but never ends is cut at that deadline; the loop answers with its existing
/// stream-cut continuation, so the run finishes instead of waiting forever.
#[tokio::test]
async fn a_total_request_deadline_cuts_the_stream_and_the_loop_continues() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = DeadlineProvider {
        requests: requests.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };
    let started = std::time::Instant::now();
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "final answer");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the deadline, not the test harness, must end the first call"
    );

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2, "the cut spends exactly one continuation");
    let note = captured[1].messages.last().expect("a continuation note");
    assert_eq!(note.role, "user");
    assert!(
        note.content
            .contains("cut off by the request time limit after 0s"),
        "the note names the deadline that cut the stream: {}",
        note.content
    );

    let seen = events.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|event| matches!(event, AgentEvent::StreamCutRecovered { .. })),
        "the operator sees the cut: {seen:?}"
    );
}
