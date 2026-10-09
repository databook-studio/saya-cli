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
