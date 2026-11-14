// The operator's `[ai] reasoning_effort` reaches the OpenAI-compatible body —
// and an unset effort leaves the body byte-for-byte what it is.
//
// Both halves matter. The setting is opt-in, so "unset changes nothing" is a
// promise about every request every existing user already sends, not a
// nicety: the frozen body below is the whole request a saya without this
// setting puts on the wire.

/// The entire body of a `Default`-effort request with no configured effort.
const UNCONFIGURED_OPENAI_BODY: &str = concat!(
    r#"{"model":"test-model","messages":[{"role":"user","content":"hello"}],"#,
    r#""tools":[{"type":"function","function":{"name":"schema_discovery","#,
    r#""description":"schema","parameters":{"type":"object"}}}],"#,
    r#""stream":true,"temperature":0.1,"stream_options":{"include_usage":true}}"#
);

fn openai_effort_body_of(captured: &str) -> String {
    captured
        .split("\r\n\r\n")
        .nth(1)
        .expect("the captured request carries a body")
        .to_owned()
}

/// One `complete` against a local server, returning the body it received.
async fn capture_openai_effort(configure: impl FnOnce(ProviderSettings) -> ProviderSettings) -> String {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n"],
    }]);
    let settings = configure(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
    );
    let provider = OpenAiCompatibleProvider::new(settings, Some("k")).unwrap();
    provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    openai_effort_body_of(&requests.lock().unwrap()[0])
}

/// With no effort configured the body is exactly the bytes a saya that never
/// had the setting sends — the `reasoning_effort` field is absent, not empty.
#[tokio::test]
async fn an_unconfigured_effort_sends_the_unchanged_openai_body() {
    let body = capture_openai_effort(|settings| settings).await;
    assert_eq!(
        body, UNCONFIGURED_OPENAI_BODY,
        "an unset reasoning_effort must not change the request at all"
    );
}

/// Each settable effort rides the wire under its own spelling, and nothing
/// else in the body moves with it.
#[tokio::test]
async fn each_configured_effort_rides_the_body_and_changes_nothing_else() {
    let unconfigured: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(UNCONFIGURED_OPENAI_BODY).unwrap();
    for (configured, spelling) in [
        (
            saya_agent::ReasoningEffort::Low,
            "low",
        ),
        (
            saya_agent::ReasoningEffort::Medium,
            "medium",
        ),
        (
            saya_agent::ReasoningEffort::High,
            "high",
        ),
    ] {
        let body = capture_openai_effort(move |settings| settings.with_reasoning_effort(Some(configured))).await;
        let mut fields: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&body).unwrap();
        assert_eq!(
            fields.remove("reasoning_effort"),
            Some(serde_json::json!(spelling)),
            "the configured effort must ride the body as {spelling}: {body}"
        );
        assert_eq!(
            fields, unconfigured,
            "the configured effort must change nothing else in the body: {body}"
        );
    }
}
