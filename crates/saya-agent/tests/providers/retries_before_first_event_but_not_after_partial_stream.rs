#[tokio::test]
async fn retries_before_first_event_but_not_after_partial_stream() {
    let (base, requests, handle) = server(vec![
        Reply {
            status: 429,
            chunks: vec![],
        },
        Reply {
            status: 200,
            chunks: vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
            ],
        },
    ]);
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
    assert_eq!(requests.lock().unwrap().len(), 2);
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n"],
    }]);
    assert!(matches!(
        openai(base).complete(request()).await,
        Err(ProviderError::InvalidResponse)
    ));
    handle.join().unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}
