/// Anthropic accepts a previous turn's thinking back only when it carries the
/// provider's signature. The loop keeps both the text and the signature on the
/// assistant message, so the next request can replay the signed block instead
/// of paying to derive the same reasoning again.
#[tokio::test]
async fn a_signed_thinking_turn_is_replayed_on_the_next_request() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = SignatureProvider {
        requests: requests.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "done");

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2);
    let assistant = captured[1]
        .messages
        .iter()
        .find(|message| !message.tool_calls.is_empty())
        .expect("the assistant turn that called the tool");
    assert_eq!(assistant.reasoning.as_deref(), Some("plan"));
    assert_eq!(
        assistant.reasoning_signature.as_deref(),
        Some("sig-1"),
        "the signature is carried onto the stored assistant message"
    );
}
