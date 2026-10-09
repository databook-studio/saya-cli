#[tokio::test]
async fn ollama_frame_limit_does_not_parse_usage_from_the_rejected_record() {
    use saya_agent::{OllamaProvider, ProviderError};
    const FRAME_LIMIT: usize = 65_536;
    let mut body = b"{\"done\":false}\n".repeat(FRAME_LIMIT);
    body.extend_from_slice(b"{\"done\":true,\"prompt_eval_count\":0,\"eval_count\":0}\n");
    let (base, handle) = byte_server(vec![body]);
    let provider = OllamaProvider::new(ProviderSettings::new("m", Some(base))).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let error = loop {
        match stream.next().await {
            Some(Err(error)) => break error,
            Some(Ok(_)) => continue,
            None => panic!("the over-limit record must fail"),
        }
    };
    handle.join().unwrap();
    assert_eq!(error, ProviderError::StreamItemLimit { usage: None });
}
