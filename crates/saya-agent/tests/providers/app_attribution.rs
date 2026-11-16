/// The product identity every model-provider request carries, whatever the
/// route or host. Each provider is driven once against the existing capture
/// server and the raw request is inspected.
#[tokio::test]
async fn every_provider_request_carries_the_saya_user_agent() {
    use saya_agent::{AnthropicProvider, GeminiProvider, OllamaProvider};

    let openai_reply = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
    let anthropic_reply = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let gemini_reply = "{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]}}]}";
    let ollama_reply = "{\"message\":{\"content\":\"ok\"},\"done\":true}";

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![openai_reply],
    }]);
    OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1"))),
        Some("k"),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap();
    handle.join().unwrap();
    assert_user_agent(&requests.lock().unwrap()[0], "openai");

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![anthropic_reply],
    }]);
    AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k"))
        .unwrap()
        .complete(request())
        .await
        .unwrap();
    handle.join().unwrap();
    assert_user_agent(&requests.lock().unwrap()[0], "anthropic");

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![gemini_reply],
    }]);
    GeminiProvider::new(ProviderSettings::new("m", Some(base)), Some("k"))
        .unwrap()
        .complete(request())
        .await
        .unwrap();
    handle.join().unwrap();
    assert_user_agent(&requests.lock().unwrap()[0], "gemini");

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![ollama_reply],
    }]);
    OllamaProvider::new(ProviderSettings::new("m", Some(base)))
        .unwrap()
        .complete(request())
        .await
        .unwrap();
    handle.join().unwrap();
    assert_user_agent(&requests.lock().unwrap()[0], "ollama");
}

fn assert_user_agent(request: &str, route: &str) {
    let request = request.to_ascii_lowercase();
    assert!(
        request.contains(&format!("user-agent: saya-cli/{}", env!("CARGO_PKG_VERSION"))),
        "the {route} request names the client: {request}"
    );
}

/// No other host receives OpenRouter's attribution headers.
#[tokio::test]
async fn a_non_openrouter_host_gets_no_attribution_headers() {
    use saya_agent::{AnthropicProvider, GeminiProvider, OllamaProvider};

    let openai_reply = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
    let anthropic_reply = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let gemini_reply = "{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]}}]}";
    let ollama_reply = "{\"message\":{\"content\":\"ok\"},\"done\":true}";

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![openai_reply],
    }]);
    OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1"))),
        Some("k"),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap();
    handle.join().unwrap();
    assert_no_attribution(&requests.lock().unwrap()[0], "openai");

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![anthropic_reply],
    }]);
    AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k"))
        .unwrap()
        .complete(request())
        .await
        .unwrap();
    handle.join().unwrap();
    assert_no_attribution(&requests.lock().unwrap()[0], "anthropic");

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![gemini_reply],
    }]);
    GeminiProvider::new(ProviderSettings::new("m", Some(base)), Some("k"))
        .unwrap()
        .complete(request())
        .await
        .unwrap();
    handle.join().unwrap();
    assert_no_attribution(&requests.lock().unwrap()[0], "gemini");

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![ollama_reply],
    }]);
    OllamaProvider::new(ProviderSettings::new("m", Some(base)))
        .unwrap()
        .complete(request())
        .await
        .unwrap();
    handle.join().unwrap();
    assert_no_attribution(&requests.lock().unwrap()[0], "ollama");
}

fn assert_no_attribution(request: &str, route: &str) {
    let request = request.to_ascii_lowercase();
    assert!(
        !request.contains("http-referer:") && !request.contains("x-title:"),
        "the {route} request to a non-OpenRouter host carries no attribution: {request}"
    );
}

/// Adding the identity header does not remove a header the request already
/// carries: the Anthropic route keeps its version and key.
#[tokio::test]
async fn existing_provider_headers_survive_the_identity_header() {
    use saya_agent::AnthropicProvider;

    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k"))
        .unwrap()
        .complete(request())
        .await
        .unwrap();
    handle.join().unwrap();
    let request = requests.lock().unwrap()[0].to_ascii_lowercase();
    assert!(request.contains("anthropic-version: 2023-06-01"), "{request}");
    assert!(request.contains("x-api-key: k"), "{request}");
    assert!(request.contains("user-agent: saya-cli/"), "{request}");
}
