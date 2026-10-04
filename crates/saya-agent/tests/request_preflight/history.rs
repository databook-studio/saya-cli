use super::*;

#[tokio::test]
async fn known_window_removes_old_ordinary_pair_and_keeps_required_request() {
    let (provider, requests) = ScriptedProvider::new(vec![turn("finished", Vec::new())]);
    let mut req = request(
        "current goal: inspect customers",
        Some("required system anchor".into()),
    );
    req.history = vec![
        ChatMessage::text(
            "user",
            format!("OLD_HISTORY_SENTINEL {}", "old ".repeat(350)),
        ),
        ChatMessage::text("assistant", format!("old answer {}", "answer ".repeat(350))),
        ChatMessage::text("user", "NEWEST_HISTORY_SENTINEL"),
        ChatMessage::text("assistant", "newest history answer"),
    ];
    let limits = AgentLimits {
        context_byte_budget: 16 * 1024,
        ..AgentLimits::default()
    };
    let output = run(
        &provider,
        &RecordingExecutor::default(),
        req,
        vec![definition("query")],
        control(
            limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(Some(1024 + 64 + 1_500), 64),
        ),
    )
    .await
    .unwrap();

    assert_eq!(output.answer, "finished");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let dispatched = &requests[0];
    assert!(provider_projection_bytes(dispatched) <= 1_500);
    assert!(
        dispatched.messages[0]
            .content
            .contains("required system anchor")
    );
    assert!(dispatched.messages.iter().any(|message| {
        message.role == "user" && message.content.contains("current goal: inspect customers")
    }));
    assert!(dispatched.messages.iter().all(|message| {
        !message.content.contains("OLD_HISTORY_SENTINEL") && !message.content.contains("old answer")
    }));
    assert!(dispatched.messages.iter().any(|message| {
        message.role == "user" && message.content.contains("NEWEST_HISTORY_SENTINEL")
    }));
    assert!(dispatched.tools.iter().any(|tool| tool.name == "query"));
}

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

#[tokio::test]
async fn digest_text_before_real_boundary_cannot_evict_original_question() {
    for (name, call_prefix, history_answer, prompt) in [
        (
            "old assistant history",
            "history-marker",
            "…[compacted: user-authored history text]",
            "original question: inspect customers",
        ),
        (
            "current user",
            "current-marker",
            "ordinary old history answer",
            "…[compacted: user-authored current text] original question: inspect customers",
        ),
    ] {
        let calls = (0..14)
            .map(|index| {
                turn(
                    "",
                    vec![call(
                        format!("{call_prefix}-{index}"),
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
            "rows": [[format!("PRESSURE_ROW_SENTINEL{}", "x".repeat(1_200))]],
            "row_count": 1,
            "truncated": false,
            "executed_sql": "SELECT value FROM source"
        });
        let executor = RecordingExecutor::with_result(result);
        let mut req = request(prompt, Some("required system anchor".into()));
        req.history = vec![
            ChatMessage::text("user", "distinguishable old question"),
            ChatMessage::text("assistant", history_answer),
        ];
        let limits = AgentLimits {
            context_byte_budget: 3_000,
            ..AgentLimits::default()
        };
        let output = run(
            &provider,
            &executor,
            req,
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
        .unwrap_or_else(|error| panic!("{name}: unexpected run failure: {error}"));

        assert_eq!(output.answer, "finished", "{name}");
        assert_eq!(executor.calls.load(Ordering::SeqCst), 14, "{name}");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 15, "{name}");
        assert!(
            provider_projection_bytes(&requests[0]) <= 3_000,
            "{name}: the initial request must fit before generated groups create pressure"
        );
        assert!(
            requests
                .iter()
                .any(|request| request.messages.iter().any(|message| {
                    message.content.starts_with("…[compacted:")
                        && message.content.contains("SELECT")
                })),
            "{name}: real tool groups must be compacted under pressure"
        );
        for (turn_index, request) in requests.iter().enumerate() {
            assert!(
                provider_projection_bytes(request) <= 3_000,
                "{name} request {turn_index} exceeds the request limit"
            );
            assert!(
                request
                    .messages
                    .iter()
                    .any(|message| { message.role == "user" && message.content == prompt }),
                "{name} request {turn_index} lost the exact original question"
            );
            assert!(
                request.messages[0]
                    .content
                    .contains("required system anchor")
            );
            assert!(request.tools.iter().any(|tool| tool.name == "query"));
            for assistant in request
                .messages
                .iter()
                .filter(|message| message.role == "assistant" && !message.tool_calls.is_empty())
            {
                for call in &assistant.tool_calls {
                    assert_eq!(
                        request
                            .messages
                            .iter()
                            .filter(|message| {
                                message.role == "tool"
                                    && message.tool_call_id.as_deref() == Some(call.id.as_str())
                            })
                            .count(),
                        1,
                        "{name} request {turn_index} must retain exactly one result for {}",
                        call.id
                    );
                }
            }
        }
    }
}
