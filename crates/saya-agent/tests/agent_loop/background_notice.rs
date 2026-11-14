// A background shell run that ends is told to the model, not polled for: the
// loop appends one short notice to the next request, and a headless run whose
// model replies with no tool calls waits for the exit instead of finishing and
// asking again. An attended run ends as it always did, and the notice rides the
// user's next message.

/// Two exits that happen in one turn coalesce into one notice message on the
/// next request, and the notice carries no command output.
#[tokio::test]
async fn exits_coalesce_into_one_notice_on_the_next_request() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider {
        answers: Mutex::new(VecDeque::from([tool_call("start_bg"), text("done")])),
        requests: Arc::clone(&requests),
    };
    let tools = FakeBackground::default();
    tools.state.lock().unwrap().starts = vec![
        exit(1, Some(0), false, 2, 7),
        exit(3, None, true, 5, 0),
    ];
    let sink = saya_agent::NoopEventSink;
    let output = saya_agent::run_agent_with_control(
        &provider,
        &tools,
        bg_request(),
        bg_definitions(),
        control(
            &sink,
            CancellationToken::new(),
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(5)),
        ),
    )
    .await
    .expect("the run completes");
    assert_eq!(output.answer, "done");

    // The first request has no notice: nothing had ended when it was sent.
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2, "one tool turn and one answer");
    assert!(
        notice_messages(&captured[0].1).is_empty(),
        "the first request carries no notice"
    );

    // The second request carries exactly one notice block with both exits.
    let notices = notice_messages(&captured[1].1);
    assert_eq!(notices.len(), 1, "one coalesced notice message: {notices:?}");
    assert_eq!(
        notices[0],
        "background shell 1 exited with code 0 after 2 s (7 output lines; read with shell_output 1)\n\
         background shell 3 was killed after 5 s (0 output lines; read with shell_output 3)",
        "each exit is one line of the one notice"
    );
    assert!(
        !notices[0].contains("hello") && !notices[0].contains("stdout"),
        "the notice carries no command output: {}",
        notices[0]
    );
    assert!(
        captured[1].1.messages.last().is_some_and(|message| {
            message.role == "user" && message.content.contains("background shell")
        }),
        "the notice follows the turn's tool results"
    );
}

/// A headless run whose model replies with no tool calls while a run is live
/// issues no request until the run ends, then exactly one with the notice.
#[tokio::test]
async fn a_no_tool_call_reply_waits_for_the_exit_then_asks_once() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider {
        answers: Mutex::new(VecDeque::from([
            tool_call("start_bg"),
            text("waiting"),
            text("finished"),
        ])),
        requests: Arc::clone(&requests),
    };
    let tools = FakeBackground::default();
    tools.state.lock().unwrap().exit_on_wait = Some((
        Duration::from_millis(150),
        exit(9, Some(0), false, 1, 3),
    ));
    let sink = saya_agent::NoopEventSink;
    let output = saya_agent::run_agent_with_control(
        &provider,
        &tools,
        bg_request(),
        bg_definitions(),
        control(
            &sink,
            CancellationToken::new(),
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(2)),
        ),
    )
    .await
    .expect("the run completes");
    assert_eq!(output.answer, "finished");

    let captured = requests.lock().unwrap();
    assert_eq!(
        captured.len(),
        3,
        "the wait issues no request; the exit earns exactly one"
    );
    let gap = captured[2].0.saturating_duration_since(captured[1].0);
    assert!(
        gap >= Duration::from_millis(100),
        "the third request waited for the exit, not for the model: {gap:?}"
    );
    let notices = notice_messages(&captured[2].1);
    assert_eq!(notices.len(), 1, "the notice rides the one request: {notices:?}");
    assert!(
        notices[0].contains("background shell 9 exited with code 0 after 1 s (3 output lines"),
        "the notice names the run: {}",
        notices[0]
    );
}

/// An attended run (no background wait) ends its turn as it always did; the
/// notice rides the next invocation's first request.
#[tokio::test]
async fn an_attended_turn_ends_and_the_notice_rides_the_next_message() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider {
        answers: Mutex::new(VecDeque::from([
            tool_call("start_bg"),
            text("waiting"),
            text("second answer"),
        ])),
        requests: Arc::clone(&requests),
    };
    let tools = FakeBackground::default();
    let sink = saya_agent::NoopEventSink;
    let first = saya_agent::run_agent_with_control(
        &provider,
        &tools,
        bg_request(),
        bg_definitions(),
        control(&sink, CancellationToken::new(), None, None),
    )
    .await
    .expect("the attended turn completes");
    assert_eq!(first.answer, "waiting");
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "an attended turn does not wait for a background run"
    );

    // The run ends between turns; the user's next message is the next run.
    tools.exit(exit(4, Some(2), false, 3, 5));
    let second = saya_agent::run_agent_with_control(
        &provider,
        &tools,
        bg_request(),
        bg_definitions(),
        control(&sink, CancellationToken::new(), None, None),
    )
    .await
    .expect("the next message completes");
    assert_eq!(second.answer, "second answer");
    let captured = requests.lock().unwrap();
    let notices = notice_messages(&captured[2].1);
    assert_eq!(notices.len(), 1, "the notice rides the next message: {notices:?}");
    assert!(
        notices[0].contains("background shell 4 exited with code 2 after 3 s (5 output lines"),
        "the notice names the run: {}",
        notices[0]
    );
}

/// The same coalesced notice is shown to the surface's display sink exactly
/// once, so an attended transcript carries the line the request carries.
#[tokio::test]
async fn the_exit_notice_is_shown_to_the_sink_once() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider {
        answers: Mutex::new(VecDeque::from([tool_call("start_bg"), text("done")])),
        requests: Arc::clone(&requests),
    };
    let tools = FakeBackground::default();
    tools.state.lock().unwrap().starts = vec![exit(2, Some(0), false, 1, 4)];
    let sink = NoticeSink::default();
    let output = saya_agent::run_agent_with_control(
        &provider,
        &tools,
        bg_request(),
        bg_definitions(),
        control(
            &sink,
            CancellationToken::new(),
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(5)),
        ),
    )
    .await
    .expect("the run completes");
    assert_eq!(output.answer, "done");

    let shown = sink.background.lock().unwrap();
    assert_eq!(shown.len(), 1, "one activity line per notice: {shown:?}");
    assert_eq!(
        shown[0],
        "background shell 2 exited with code 0 after 1 s (4 output lines; read with shell_output 2)",
        "the line is the notice's own text"
    );
}

/// When the job's wall clock is spent, the wait gives up and the run finishes
/// as it did before: no request is opened that the clock would only cut.
#[tokio::test]
async fn a_spent_wall_clock_ends_the_run_as_today() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider {
        answers: Mutex::new(VecDeque::from([tool_call("start_bg"), text("waiting")])),
        requests: Arc::clone(&requests),
    };
    let tools = FakeBackground::default();
    let sink = saya_agent::NoopEventSink;
    let began = Instant::now();
    let output = saya_agent::run_agent_with_control(
        &provider,
        &tools,
        bg_request(),
        bg_definitions(),
        control(
            &sink,
            CancellationToken::new(),
            Some(Duration::from_millis(200)),
            Some(Duration::from_secs(5)),
        ),
    )
    .await
    .expect("the spent clock ends the run without a provider error");
    assert_eq!(output.answer, "waiting");
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "the spent clock opens no further request"
    );
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "the wait is bounded by the wall clock: {:?}",
        began.elapsed()
    );
}
