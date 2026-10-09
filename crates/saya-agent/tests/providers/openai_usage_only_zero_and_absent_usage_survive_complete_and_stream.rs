#[tokio::test]
async fn openai_usage_only_zero_and_absent_usage_survive_complete_and_stream() {
    use saya_agent::{ProviderEvent, TokenUsage};
    let cases = [
        (
            vec![
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":0,\"completion_tokens\":0}}\n\n",
                "data: [DONE]\n\n",
            ],
            Some(TokenUsage::new(0, 0)),
        ),
        (
            vec![
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ],
            None,
        ),
        (
            vec![
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":200,\"prompt_tokens_details\":{\"cached_tokens\":90},\"completion_tokens_details\":{\"reasoning_tokens\":170}}}\n\n",
                "data: [DONE]\n\n",
            ],
            Some(
                TokenUsage::new(100, 200)
                    .with_cached_input(Some(90))
                    .with_reasoning(Some(170)),
            ),
        ),
        (
            vec![
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":200,\"prompt_tokens_details\":{\"cached_tokens\":90,\"cache_write_tokens\":0},\"completion_tokens_details\":{\"reasoning_tokens\":170}}}\n\n",
                "data: [DONE]\n\n",
            ],
            Some(
                TokenUsage::new(100, 200)
                    .with_cached_input(Some(90))
                    .with_cache_creation(Some(0))
                    .with_reasoning(Some(170)),
            ),
        ),
        (
            vec![
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":200,\"prompt_tokens_details\":{\"cached_tokens\":90,\"cache_write_tokens\":35},\"completion_tokens_details\":{\"reasoning_tokens\":170}}}\n\n",
                "data: [DONE]\n\n",
            ],
            Some(
                TokenUsage::new(100, 200)
                    .with_cached_input(Some(90))
                    .with_cache_creation(Some(35))
                    .with_reasoning(Some(170)),
            ),
        ),
    ];
    for (terminal, expected) in cases {
        let mut chunks = vec!["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n"];
        chunks.extend(terminal.iter().copied());
        let (base, _, handle) = server(vec![Reply {
            status: 200,
            chunks,
        }]);
        let response = openai(base).complete(request()).await.unwrap();
        handle.join().unwrap();
        assert_eq!(response.usage, expected);

        let mut chunks = vec!["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n"];
        chunks.extend(terminal.iter().copied());
        let (base, _, handle) = server(vec![Reply {
            status: 200,
            chunks,
        }]);
        let mut stream = openai(base)
            .stream(request(), CancellationToken::new())
            .await
            .unwrap();
        let events = drain(&mut stream).await;
        handle.join().unwrap();
        let actual = events.iter().find_map(|event| match event {
            ProviderEvent::Usage(usage) => Some(*usage),
            _ => None,
        });
        assert_eq!(actual, expected);
    }
}
