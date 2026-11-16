#[tokio::test]
async fn terminal_sentinels_allow_only_trailing_ascii_whitespace() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n \r\n\t",
        ],
    }]);
    assert_eq!(
        openai(base)
            .complete(request())
            .await
            .unwrap()
            .message
            .content,
        "ok"
    );
    handle.join().unwrap();
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\nnot-whitespace",
        ],
    }]);
    assert!(matches!(
        openai(base).complete(request()).await,
        Err(ProviderError::InvalidResponse)
    ));
    handle.join().unwrap();
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["{\"message\":{\"content\":\"ok\"},\"done\":true}\n \r\n\t"],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    assert_eq!(
        provider.complete(request()).await.unwrap().message.content,
        "ok"
    );
    handle.join().unwrap();
}
