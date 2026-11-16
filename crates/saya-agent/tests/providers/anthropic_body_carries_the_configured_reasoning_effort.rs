// The operator's `[ai] reasoning_effort` reaches the Anthropic body — and an
// unset effort leaves the body byte-for-byte what it is.
//
// The Anthropic route spells effort two ways, and this pins both: `high` is
// the adaptive-thinking request (`thinking: adaptive` plus
// `output_config.effort`, and the beta header that request requires), while
// `low`/`medium` are fixed thinking budgets. An unset effort sends neither.

/// The entire body of a `Default`-effort request with no configured effort:
/// no `thinking`, no `output_config`, temperature present.
const UNCONFIGURED_ANTHROPIC_BODY: &str = concat!(
    r#"{"max_tokens":32000,"messages":[{"content":"hello","role":"user"}],"#,
    r#""model":"test-model","stream":true,"temperature":0.10000000149011612,"#,
    r#""tools":[{"cache_control":{"type":"ephemeral"},"description":"schema","#,
    r#""input_schema":{"type":"object"},"name":"schema_discovery"}]}"#
);

fn anthropic_effort_body_of(captured: &str) -> String {
    captured
        .split("\r\n\r\n")
        .nth(1)
        .expect("the captured request carries a body")
        .to_owned()
}

/// One `complete` against a local server, returning the whole request it
/// received — headers included, because the adaptive effort rides a header.
async fn capture_anthropic_effort(configure: impl FnOnce(ProviderSettings) -> ProviderSettings) -> String {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let settings = configure(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
    );
    let provider = saya_agent::AnthropicProvider::new(settings, Some("k")).unwrap();
    provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    requests.lock().unwrap()[0].clone()
}

/// With no effort configured the body is exactly the bytes a saya that never
/// had the setting sends, and no adaptive beta header is claimed.
#[tokio::test]
async fn an_unconfigured_effort_sends_the_unchanged_anthropic_body() {
    let sent = capture_anthropic_effort(|settings| settings).await;
    assert_eq!(
        anthropic_effort_body_of(&sent),
        UNCONFIGURED_ANTHROPIC_BODY,
        "an unset reasoning_effort must not change the request at all"
    );
    assert!(
        !sent.contains("anthropic-beta"),
        "an unset effort must not claim the effort beta: {sent}"
    );
}

/// `high` is the adaptive request: `thinking: adaptive`, `output_config.effort`
/// of `high`, the beta header that shape requires, and no temperature —
/// adaptive thinking rejects it.
#[tokio::test]
async fn a_high_effort_sends_the_adaptive_request() {
    let sent = capture_anthropic_effort(|settings| {
        settings.with_reasoning_effort(Some(saya_agent::ReasoningEffort::High))
    })
    .await;
    let body: serde_json::Value = serde_json::from_str(&anthropic_effort_body_of(&sent)).unwrap();
    assert_eq!(body["thinking"], serde_json::json!({"type": "adaptive"}));
    assert_eq!(body["output_config"], serde_json::json!({"effort": "high"}));
    assert!(
        body.get("temperature").is_none(),
        "adaptive thinking rejects temperature: {body}"
    );
    assert!(
        sent.contains("anthropic-beta: effort-2025-11-24"),
        "the adaptive request must claim its beta header: {sent}"
    );
}

/// `low` and `medium` stay fixed thinking budgets, and the beta header is not
/// claimed for them.
#[tokio::test]
async fn a_budgeted_effort_sends_a_thinking_budget() {
    for (configured, budget) in [
        (saya_agent::ReasoningEffort::Low, 2048),
        (saya_agent::ReasoningEffort::Medium, 4096),
    ] {
        let sent =
            capture_anthropic_effort(move |settings| settings.with_reasoning_effort(Some(configured))).await;
        let body: serde_json::Value = serde_json::from_str(&anthropic_effort_body_of(&sent)).unwrap();
        assert_eq!(
            body["thinking"],
            serde_json::json!({"type": "enabled", "budget_tokens": budget}),
            "a budgeted effort must send its budget: {body}"
        );
        assert!(
            body.get("output_config").is_none(),
            "only the adaptive request carries output_config: {body}"
        );
        assert!(
            !sent.contains("anthropic-beta"),
            "a budgeted effort must not claim the effort beta: {sent}"
        );
    }
}
