#[tokio::test]
async fn config_context_byte_budget_bounds_the_history_the_loop_sends() {
    let history = vec![
        ChatMessage::text(
            "user",
            format!("OLD-PAIR-SENTINEL-{}", "x".repeat(64 * 1024)),
        ),
        ChatMessage::text("assistant", "old answer"),
        ChatMessage::text("user", "NEW-PAIR-SENTINEL"),
        ChatMessage::text("assistant", "new answer"),
    ];
    let (result, counters) = run_turn(16 * 1024, history).await;
    result.expect("the turn completes");
    let messages = counters
        .first_messages()
        .expect("the first request was recorded");
    assert_eq!(
        messages.len(),
        4,
        "system + the one fitting pair + the current user turn: {messages:?}"
    );
    let joined = messages
        .iter()
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("NEW-PAIR-SENTINEL"),
        "the newest pair replays under the planted budget: {joined}"
    );
    assert!(
        !joined.contains("OLD-PAIR-SENTINEL"),
        "the oversized oldest pair is trimmed by the planted budget: {joined}"
    );
}

#[tokio::test]
async fn actual_runtime_request_prefix_is_stable_and_last_sql_stays_in_the_user_tail() {
    let measurements = capture_repeated_requests().await;
    assert_eq!(measurements.len(), 5, "five fixed local runs were captured");
    let first = &measurements[0];
    let request_bytes = serde_json::to_vec(&first.request).unwrap().len();
    let system_bytes =
        first.request.messages[0].role.len() + first.request.messages[0].content.len();
    let message_bytes = first
        .request
        .messages
        .iter()
        .map(|message| message.role.len() + message.content.len())
        .sum::<usize>();
    let tool_bytes = serde_json::to_vec(&first.request.tools).unwrap().len();
    let request = serde_json::to_vec(&first.request).unwrap();
    assert!(
        measurements
            .iter()
            .all(|measurement| serde_json::to_vec(&measurement.request).unwrap() == request),
        "fixed runtime inputs must produce the same complete request on all five runs"
    );
    assert!(
        measurements.iter().all(|measurement| {
            measurement.request_count == 1
                && measurement.tool_calls == 0
                && measurement.retries == 0
        }),
        "the fixed provider made one request with no tool calls or retries"
    );
    assert!(
        measurements
            .iter()
            .all(|measurement| measurement.first_assistant_text.is_some()),
        "each fixed run emitted nonempty assistant text through the event sink"
    );
    let changed = record_first_request(
        "orders by customer",
        Some("SELECT missing FROM missing_table"),
    )
    .await;
    let non_ascii = record_first_request("orders in München 😀", None).await;
    let profile_changed = record_first_request_for(
        "orders by month",
        None,
        test_ai(16 * 1024),
        "warehouse",
        &NoopEventSink,
    )
    .await;
    assert_eq!(
        serde_json::to_vec(&first.request.tools).unwrap(),
        serde_json::to_vec(&changed.tools).unwrap(),
        "question and last SQL cannot reorder or alter tool schemas"
    );
    assert_eq!(
        first.request.messages[0], changed.messages[0],
        "per-turn inputs must not alter the system prefix"
    );
    assert!(
        changed
            .messages
            .last()
            .unwrap()
            .content
            .contains("orders by customer")
    );
    assert!(
        changed
            .messages
            .last()
            .unwrap()
            .content
            .contains("SELECT missing FROM missing_table")
    );
    assert_eq!(
        first.request.tools.len(),
        non_ascii.tools.len(),
        "the full tool set stays attached"
    );
    assert!(
        non_ascii
            .messages
            .last()
            .unwrap()
            .content
            .contains("München 😀")
    );
    assert_ne!(
        first.request.messages[0], profile_changed.messages[0],
        "the selected profile changes the production-built session context"
    );
    assert!(profile_changed.messages[0].content.contains("warehouse"));
    let elapsed = measurements.iter().map(|measurement| measurement.elapsed);
    let first_text = measurements
        .iter()
        .map(|measurement| measurement.first_assistant_text.unwrap());
    eprintln!(
        "request corpus local fixed-provider: runs=5 request_bytes={request_bytes} \
         system_bytes={system_bytes} messages_bytes={message_bytes} tools_bytes={tool_bytes} \
         requests=1 tool_calls=0 retries=0 elapsed_us={}..{} first_assistant_text_us={}..{}",
        elapsed.clone().min().unwrap().as_micros(),
        elapsed.max().unwrap().as_micros(),
        first_text.clone().min().unwrap().as_micros(),
        first_text.max().unwrap().as_micros(),
    );
}
