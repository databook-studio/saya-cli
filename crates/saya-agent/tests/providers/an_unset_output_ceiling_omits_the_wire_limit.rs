/// With no configured output ceiling the route sends no limit at all on
/// the APIs that allow omitting one, so the model's own default applies.
/// Anthropic requires `max_tokens`, so that route falls back to 32,000.
#[tokio::test]
async fn an_unset_output_ceiling_omits_the_wire_limit() {
    const OPENAI: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
    const GEMINI: &str = r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}"#;
    const OLLAMA: &str = "{\"message\":{\"content\":\"ok\"},\"done\":true}\n";
    const ANTHROPIC: &str = "event: message_start\ndata: {\"type\":\"message_start\"}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let (base, requests, handle) = server(vec![
        Reply {
            status: 200,
            chunks: vec![OPENAI],
        },
        Reply {
            status: 200,
            chunks: vec![GEMINI],
        },
        Reply {
            status: 200,
            chunks: vec![OLLAMA],
        },
        Reply {
            status: 200,
            chunks: vec![ANTHROPIC],
        },
    ]);
    let openai = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("k"),
    )
    .unwrap();
    let gemini = saya_agent::GeminiProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1beta")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("k"),
    )
    .unwrap();
    let ollama = OllamaProvider::new(
        ProviderSettings::new("test-model", Some(base.clone()))
            .with_retry_delays(vec![Duration::ZERO]),
    )
    .unwrap();
    let anthropic = saya_agent::AnthropicProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("k"),
    )
    .unwrap();

    openai.complete(request()).await.unwrap();
    gemini.complete(request()).await.unwrap();
    ollama.complete(request()).await.unwrap();
    anthropic.complete(request()).await.unwrap();
    handle.join().unwrap();

    let captured = requests.lock().unwrap();
    let openai_body = captured[0].clone();
    assert!(
        !openai_body.contains("max_completion_tokens") && !openai_body.contains("\"max_tokens\""),
        "an unset ceiling must send no output limit on the OpenAI wire: {openai_body}"
    );
    let gemini_body = captured[1].clone();
    assert!(
        !gemini_body.contains("maxOutputTokens"),
        "an unset ceiling must send no output limit on the Gemini wire: {gemini_body}"
    );
    let ollama_body: serde_json::Value = serde_json::from_str(
        captured[2]
            .split_once("\r\n\r\n")
            .expect("HTTP request has a body")
            .1,
    )
    .unwrap();
    assert_eq!(
        ollama_body["options"].get("num_predict"),
        None,
        "an unset ceiling must send no output limit on the Ollama wire: {ollama_body}"
    );
    let anthropic_body = captured[3].clone();
    assert!(
        anthropic_body.contains("\"max_tokens\":32000"),
        "Anthropic requires max_tokens, so an unset ceiling sends 32,000: {anthropic_body}"
    );
}
