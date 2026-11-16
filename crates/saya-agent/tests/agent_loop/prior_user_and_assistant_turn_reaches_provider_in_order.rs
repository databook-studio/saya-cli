#[tokio::test]
async fn prior_user_and_assistant_turn_reaches_provider_in_order() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = HistoryProvider {
        requests: requests.clone(),
    };
    let mut request = request();
    request.history = vec![
        ChatMessage::text("user", "first prompt"),
        ChatMessage::text("assistant", "first answer"),
    ];
    run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request,
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    let captured = requests.lock().unwrap();
    assert_eq!(captured[0].messages[1].content, "first prompt");
    assert_eq!(captured[0].messages[2].content, "first answer");
    assert_eq!(captured[0].messages[3].content, "show data");
}
