/// The note a failed result carries once this exact call has failed three
/// times in one run. Offered once per key, never on the fourth failure.
const REPEAT_NOTE: &str = "This exact call has now failed 3 times. Try a different approach.";

/// The model-facing tool result for `id` on the last provider turn — i.e. the
/// exact text the model received for that call.
fn tool_message(requests: &[ChatRequest], id: &str) -> Option<String> {
    requests.last().and_then(|request| {
        request
            .messages
            .iter()
            .find(|message| message.role == "tool" && message.tool_call_id.as_deref() == Some(id))
            .map(|message| message.content.clone())
    })
}

/// A byte-identical repeat of a failed call is EXECUTED: the loop keeps no
/// refusal for a repeat, so the executor sees all three calls. Against the
/// code before the fix the second and third submissions were refused and the
/// executor ran once.
#[tokio::test]
async fn an_identical_failing_call_is_executed_every_time() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "syntax error near 'bad'".into(),
    };
    let (_output, calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT bad"),
            tool_turn("c2", "SELECT bad"),
            tool_turn("c3", "SELECT bad"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(
        calls.len(),
        3,
        "every identical retry must run at the executor: {calls:?}"
    );
}

/// The third identical failure carries the note, and only the third: the
/// first two failures carry none, and the fourth does not repeat it. Against
/// the code before the fix the repeats were refused and no note existed.
#[tokio::test]
async fn the_third_identical_failure_carries_the_note_once() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "syntax error near 'bad'".into(),
    };
    let (_output, calls, requests) = run(
        vec![
            tool_turn("c1", "SELECT bad"),
            tool_turn("c2", "SELECT bad"),
            tool_turn("c3", "SELECT bad"),
            tool_turn("c4", "SELECT bad"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(calls.len(), 4, "all four identical calls run: {calls:?}");

    let first = tool_message(&requests, "c1").expect("the first result reaches the model");
    let second = tool_message(&requests, "c2").expect("the second result reaches the model");
    let third = tool_message(&requests, "c3").expect("the third result reaches the model");
    let fourth = tool_message(&requests, "c4").expect("the fourth result reaches the model");
    assert!(!first.contains(REPEAT_NOTE), "the first failure has no note: {first}");
    assert!(
        !second.contains(REPEAT_NOTE),
        "the second failure has no note: {second}"
    );
    assert!(
        third.contains(REPEAT_NOTE),
        "the third identical failure carries the note: {third}"
    );
    assert!(
        !fourth.contains(REPEAT_NOTE),
        "the note is offered once per key, not on the fourth failure: {fourth}"
    );
    assert!(
        requests
            .last()
            .expect("a final request")
            .messages
            .iter()
            .all(|message| !message.content.contains("already failed earlier in this run")),
        "no repeat is ever refused"
    );
}
