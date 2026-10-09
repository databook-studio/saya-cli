/// The batch path records failures too: two distinct statements that fail
/// concurrently in one turn are both remembered, so a later single-turn repeat
/// of either is refused on the sequential path (the batch is not taken when a
/// repeat is present). This proves the two execution paths share the same
/// failure memory.
#[tokio::test]
async fn the_batch_path_records_failures_so_a_later_repeat_is_refused() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CannedProvider {
        responses: Mutex::new(vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![call("c1", "SELECT a"), call("c2", "SELECT b")],
                tool_call_id: None,
            },
            tool_turn("c3", "SELECT a"),
            ChatMessage::text("assistant", "done"),
        ]),
        requests: requests.clone(),
    };
    let executor = BatchFailingExecutor {
        calls: calls.clone(),
        barrier: Arc::new(tokio::sync::Barrier::new(2)),
    };
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run_agent_with_sink(
            &provider,
            &executor,
            request(),
            vec![query_tool()],
            AgentLimits::default(),
            &AllowReadOnlyApproval,
            &RecordingSink {
                events: Arc::new(Mutex::new(Vec::new())),
            },
            saya_agent::CancellationToken::new(),
        ),
    )
    .await
    .expect("the batch calls must overlap at the executor")
    .expect("run completes");
    let calls = calls.lock().unwrap().clone();
    let requests = requests.lock().unwrap().clone();
    assert_eq!(output.answer, "done");
    assert_eq!(
        calls.len(),
        2,
        "the two distinct batch calls execute; the later repeat is refused: {calls:?}"
    );
    let refusal = last_tool_message_containing(&requests, "already failed")
        .expect("the repeat refusal must reach the model");
    assert!(
        refusal.contains("bad batch"),
        "the refusal must name the earlier batch error, got: {refusal}"
    );
}

/// An identical failing non-SQL tool call is refused on its second attempt —
/// the anti-repeat memory is not SQL-only. Against the code before the
/// generalization the executor ran the identical failing call every turn,
/// because a call with no `sql` argument was never a repeat.
#[tokio::test]
async fn an_identical_failing_non_sql_call_is_refused_on_the_second_attempt() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "program exited 1".into(),
    };
    let (output, calls, requests) = run_with_tools(
        vec![
            program_turn("c1", "make report"),
            program_turn("c2", "make report"),
            ChatMessage::text("assistant", "changed approach"),
        ],
        executor,
        AgentLimits::default(),
        vec![program_tool()],
    )
    .await;
    assert_eq!(
        output.answer, "changed approach",
        "the run completes after the refusal"
    );
    assert_eq!(
        calls.len(),
        1,
        "the identical failing tool call must execute once, not twice: {calls:?}"
    );
    let refusal = last_tool_message_containing(&requests, "already failed")
        .expect("the refusal must reach the model as a tool message");
    assert!(
        refusal.contains("program exited 1"),
        "the refusal must name the earlier error, got: {refusal}"
    );
}

/// A different argument value for the same tool is NOT a repeat — the whole
/// invocation (name and arguments) is the failure, so changed arguments are a
/// genuine change of approach and must execute.
#[tokio::test]
async fn a_different_argument_for_the_same_non_sql_tool_is_executed() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "program exited 1".into(),
    };
    let (_output, calls, _requests) = run_with_tools(
        vec![
            program_turn("c1", "make report"),
            program_turn("c2", "make clean"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
        vec![program_tool()],
    )
    .await;
    assert_eq!(
        calls.len(),
        2,
        "different arguments are a different call and must execute: {calls:?}"
    );
}
