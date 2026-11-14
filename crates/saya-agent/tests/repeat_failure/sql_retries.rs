/// A statement that failed once is executed again when it is resubmitted: the
/// loop keeps no refusal, and the executor runs the statement every time.
#[tokio::test]
async fn a_failed_statement_is_re_executed() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "syntax error near 'bad'".into(),
    };
    let (output, calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT bad"),
            tool_turn("c2", "SELECT bad"),
            ChatMessage::text("assistant", "changed approach"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(output.answer, "changed approach", "the run completes");
    assert_eq!(
        calls.len(),
        2,
        "the identical failing statement must execute both times: {calls:?}"
    );
}

/// A statement that differs by one character is NOT a repeat — there is no
/// whitespace or case normalisation, so a genuine edit is always executed.
/// Against a normalising implementation this would execute once.
#[tokio::test]
async fn a_statement_differing_by_one_character_is_executed() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "bad".into(),
    };
    let (_output, calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT 1"),
            tool_turn("c2", "SELECT 2"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(
        calls.len(),
        2,
        "a one-character edit is a different statement and must execute: {calls:?}"
    );
}

/// A statement that succeeded is not tracked — re-running a successful query
/// is legitimate. The same SQL submitted twice, both succeeding, executes
/// twice.
#[tokio::test]
async fn a_successful_statement_may_be_re_run() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: false,
        error: String::new(),
    };
    let (_output, calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT 1"),
            tool_turn("c2", "SELECT 1"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(
        calls.len(),
        2,
        "a successful statement may be re-run: {calls:?}"
    );
}

/// A normal run that never repeats a failure executes every call exactly once
/// — the repeat-refusal logic must not suppress a first execution. Three
/// distinct statements, none repeated, all execute.
#[tokio::test]
async fn a_normal_run_never_repeating_a_failure_executes_every_call_once() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: false,
        error: String::new(),
    };
    let (output, calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT 1"),
            tool_turn("c2", "SELECT 2"),
            tool_turn("c3", "SELECT 3"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(output.answer, "done");
    assert_eq!(
        calls.len(),
        3,
        "distinct statements must all execute once, none suppressed: {calls:?}"
    );
}
