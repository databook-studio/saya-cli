/// The bound is one recovery per run: a follow-up turn that also designates
/// with no prose completes with what the run already has — the first
/// designation — instead of looping.
#[tokio::test]
async fn a_second_empty_designation_does_not_recover_again() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            calls_only(vec![designation_call("d2", "SELECT count(*) FROM other")]),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a second empty designation completes rather than recovering again");
    assert_eq!(
        output.answer, "",
        "no prose was ever produced; the run must not invent one"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the first designation wins"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "one recovery turn only: the designation turn plus the follow-up"
    );
    assert_eq!(
        designated_events(&output).len(),
        1,
        "the AnswerDesignated event is emitted exactly once per run"
    );
}

/// The follow-up turn failing must not fail the run: a reply with neither
/// content nor tool calls is fatal for an ordinary turn, but after an empty
/// designation the run still ends with the SQL it already has.
#[tokio::test]
async fn an_empty_follow_up_reply_keeps_the_designation() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            vec![Ok(ProviderEvent::Done)],
        ],
        requests: requests.clone(),
    };
    let output = match run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    {
        Ok(output) => output,
        Err(error) => panic!("a failed follow-up turn must not fail the run: {error}"),
    };
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "the follow-up turn must actually run and fail, not be skipped"
    );
    assert_eq!(output.answer, "");
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the designation survives the failed follow-up"
    );
}

/// Every tool call riding the same message as an empty designation is answered
/// — a provider rejects a request whose assistant tool calls lack replies —
/// and none of them is executed: the run already has its answer.
#[tokio::test]
async fn sibling_calls_of_an_empty_designation_are_all_answered() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![
                ToolCall {
                    id: "b1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql": "SELECT 1"}),
                },
                designation_call("d1", "SELECT count(*) FROM t"),
            ]),
            text_then_done("The answer is 42."),
        ],
        requests: requests.clone(),
    };
    let executed = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: executed.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the designation recovers its prose");
    assert_eq!(
        output.answer, "The answer is 42.",
        "an empty-prose designation must recover the prose"
    );
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "the designation turn plus one follow-up turn"
    );
    let answered = answered_call_ids(&requests[1]);
    for id in ["b1", "d1"] {
        assert!(
            answered.iter().any(|seen| seen == id),
            "the follow-up request must answer call {id}: {answered:?}"
        );
    }
    assert!(
        executed.lock().unwrap().is_empty(),
        "the sibling call must not be executed: the answer was already designated"
    );
}
