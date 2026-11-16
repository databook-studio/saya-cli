/// The batch path records failures too: two distinct statements that fail
/// concurrently in one turn are both remembered, so a later single-turn repeat
/// of either is executed and counted on the sequential path. This proves the
/// two execution paths share the same failure memory.
#[tokio::test]
async fn the_batch_path_records_failures_so_a_later_repeat_is_counted() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CannedProvider {
        responses: Mutex::new(vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![call("c1", "SELECT a"), call("c2", "SELECT b")],
                tool_call_id: None,

                images: Vec::new(),
                            reasoning: None,
                            reasoning_signature: None,
            },
            // The later turn repeats "SELECT a" beside a fresh statement, so
            // the executor's rendezvous still pairs up.
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![call("c3", "SELECT a"), call("c4", "SELECT c")],
                tool_call_id: None,

                images: Vec::new(),
                            reasoning: None,
                            reasoning_signature: None,
            },
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
        4,
        "both batch turns execute every call, the repeat included: {calls:?}"
    );
    let repeated = last_tool_message_containing(&requests, "bad batch")
        .expect("the repeat's own failure reaches the model");
    assert!(
        !repeated.contains("already failed earlier in this run"),
        "the repeat is executed, not refused: {repeated}"
    );
}

/// An identical failing non-SQL tool call is executed on its second attempt —
/// the failure memory is not SQL-only, and it never blocks a call. Against the
/// code before the generalization the executor ran the identical failing call
/// every turn too, but only because a call with no `sql` argument was never
/// tracked as a repeat; both paths run it and both count it.
#[tokio::test]
async fn an_identical_failing_non_sql_call_is_executed_on_the_second_attempt() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "program exited 1".into(),
    };
    let (output, calls, _requests) = run_with_tools(
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
    assert_eq!(output.answer, "changed approach", "the run completes");
    assert_eq!(
        calls.len(),
        2,
        "the identical failing tool call must execute both times: {calls:?}"
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
