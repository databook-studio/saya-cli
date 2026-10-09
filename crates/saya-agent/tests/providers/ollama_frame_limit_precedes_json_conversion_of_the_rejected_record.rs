#[tokio::test]
async fn ollama_frame_limit_precedes_json_conversion_of_the_rejected_record() {
    use saya_agent::{OllamaProvider, ProviderError};
    const FRAME_LIMIT: usize = 65_536;
    let mut body = b"{\"done\":false}\n".repeat(FRAME_LIMIT);
    body.extend_from_slice(b"not-json\n");
    let (base, handle) = byte_server(vec![body]);
    let provider = OllamaProvider::new(ProviderSettings::new("m", Some(base))).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let error = stream.next().await.unwrap().unwrap_err();
    handle.join().unwrap();
    assert_eq!(error, ProviderError::StreamItemLimit { usage: None });
}
