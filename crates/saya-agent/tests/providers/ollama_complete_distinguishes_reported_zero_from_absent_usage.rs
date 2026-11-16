#[tokio::test]
async fn ollama_complete_distinguishes_reported_zero_from_absent_usage() {
    use saya_agent::{OllamaProvider, TokenUsage};
    let cases = [
        ("{\"done\":true}", None),
        (
            "{\"done\":true,\"prompt_eval_count\":0,\"eval_count\":0}",
            Some(TokenUsage::new(0, 0)),
        ),
        (
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}",
            Some(TokenUsage::new(9, 11)),
        ),
        (
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11,\"prompt_eval_cached_count\":0}",
            Some(TokenUsage::new(9, 11).with_cached_input(Some(0))),
        ),
        (
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11,\"prompt_eval_cached_count\":4}",
            Some(TokenUsage::new(9, 11).with_cached_input(Some(4))),
        ),
    ];
    for (done_record, expected) in cases {
        let (base, _, handle) = server(vec![Reply {
            status: 200,
            chunks: vec![
                "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
                done_record,
            ],
        }]);
        let provider = OllamaProvider::new(ProviderSettings::new("m", Some(base))).unwrap();
        let response = provider.complete(request()).await.unwrap();
        handle.join().unwrap();
        assert_eq!(response.usage, expected);

        let (base, _, handle) = server(vec![Reply {
            status: 200,
            chunks: vec![
                "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
                done_record,
            ],
        }]);
        let provider = OllamaProvider::new(ProviderSettings::new("m", Some(base))).unwrap();
        let mut stream = provider
            .stream(request(), CancellationToken::new())
            .await
            .unwrap();
        let events = drain(&mut stream).await;
        handle.join().unwrap();
        let usage = events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::Usage(usage) => Some(*usage),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(usage, expected.into_iter().collect::<Vec<_>>());
    }
}
