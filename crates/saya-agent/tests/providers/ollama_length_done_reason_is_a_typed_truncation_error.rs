/// O1 property 4 (Ollama): a final done record with `done_reason: "length"`
/// surfaces the typed truncation error with the partial text. Ollama's
/// `/api/chat` documents `done_reason` on the final record (values `stop`,
/// `length`, `load`, `unload`); `length` mirrors the OpenAI vocabulary and is
/// the reliable signal.
#[tokio::test]
async fn ollama_length_done_reason_is_a_typed_truncation_error() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"partial\"},\"done\":false}\n",
            "{\"done\":true,\"done_reason\":\"length\"}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let error = provider.complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(error, ProviderError::OutputTruncated { .. }),
        "ollama done_reason length must be typed truncation, got: {error:?}"
    );
}
