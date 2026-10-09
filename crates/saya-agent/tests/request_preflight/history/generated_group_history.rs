use super::*;

#[tokio::test]
async fn actual_request_preflight_reuses_safe_group_digest_and_keeps_current_context() {
    let calls = (0..14)
        .map(|index| {
            turn(
                "",
                vec![call(
                    format!("call-{index}"),
                    "query",
                    serde_json::json!({"sql": format!("SELECT {index}")}),
                )],
            )
        })
        .chain(std::iter::once(turn("finished", Vec::new())))
        .collect();
    let (provider, requests) = ScriptedProvider::new(calls);
    let result = serde_json::json!({
        "columns": ["value"],
        "rows": [[format!("ROW_VALUE_SENTINEL{}", "x".repeat(1_200))]],
        "row_count": 1,
        "truncated": false,
        "executed_sql": "SELECT value FROM source"
    });
    let executor = RecordingExecutor::with_result(result);
    let limits = AgentLimits {
        context_byte_budget: 3_000,
        ..AgentLimits::default()
    };
    let output = run(
        &provider,
        &executor,
        request("current question", Some("required system anchor".into())),
        vec![definition("query")],
        control(
            limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(Some(1024 + 128 + 4_500), 128),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        output.answer,
        "finished",
        "events: {:?}; requests: {:?}",
        output.events,
        requests
            .lock()
            .unwrap()
            .iter()
            .map(provider_projection_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(executor.calls.load(Ordering::SeqCst), 14);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 15);
    assert!(
        requests
            .iter()
            .all(|request| provider_projection_bytes(request) <= 3_000)
    );
    assert!(
        requests.iter().any(|request| request
            .messages
            .iter()
            .any(|message| { message.content.starts_with("…[compacted:") })),
        "an evicted complete group is represented by the established digest; request bytes: {:?}",
        requests
            .iter()
            .map(provider_projection_bytes)
            .collect::<Vec<_>>()
    );
    let compacted = requests
        .iter()
        .find_map(|request| {
            request
                .messages
                .iter()
                .find(|message| message.content.starts_with("…[compacted:"))
        })
        .expect("a safe digest reaches the provider");
    assert!(compacted.content.contains("SELECT"));
    assert!(!compacted.content.contains("ROW_VALUE_SENTINEL"));

    let latest = requests.last().unwrap();
    assert!(latest.tools.iter().any(|tool| tool.name == "query"));
    assert!(
        latest.messages[0]
            .content
            .contains("required system anchor")
    );
    assert!(
        latest.messages.iter().any(|message| {
            message.role == "user" && message.content.contains("current question")
        })
    );
    let last_call = latest
        .messages
        .iter()
        .find(|message| message.tool_calls.iter().any(|call| call.id == "call-13"))
        .expect("the newest complete group remains");
    let last_id = &last_call.tool_calls[0].id;
    assert!(
        latest.messages.iter().any(|message| {
            message.role == "tool" && message.tool_call_id.as_deref() == Some(last_id)
        }),
        "the newest assistant call keeps its paired result"
    );
}
