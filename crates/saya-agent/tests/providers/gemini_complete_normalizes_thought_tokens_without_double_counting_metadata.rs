#[tokio::test]
async fn gemini_complete_normalizes_thought_tokens_without_double_counting_metadata() {
    use saya_agent::{GeminiProvider, TokenUsage};
    let cases = [
        (
            "{\"promptTokenCount\":100,\"candidatesTokenCount\":200,\"thoughtsTokenCount\":270,\"totalTokenCount\":570,\"cachedContentTokenCount\":90}",
            TokenUsage::new(100, 470)
                .with_cached_input(Some(90))
                .with_reasoning(Some(270)),
        ),
        (
            "{\"promptTokenCount\":4,\"candidatesTokenCount\":5}",
            TokenUsage::new(4, 5),
        ),
        (
            "{\"promptTokenCount\":4,\"candidatesTokenCount\":5,\"thoughtsTokenCount\":0}",
            TokenUsage::new(4, 5).with_reasoning(Some(0)),
        ),
        (
            "{\"promptTokenCount\":4,\"candidatesTokenCount\":18446744073709551615,\"thoughtsTokenCount\":1}",
            TokenUsage::new(4, u64::MAX).with_reasoning(Some(1)),
        ),
    ];
    for (metadata, expected) in cases {
        let (base, _, handle) = server(vec![Reply {
            status: 200,
            chunks: vec![
                "{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]}}],\"usageMetadata\":",
                metadata,
                "}",
            ],
        }]);
        let provider =
            GeminiProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
        let response = provider.complete(request()).await.unwrap();
        handle.join().unwrap();
        assert_eq!(response.usage, Some(expected));
    }
}
