#[tokio::test]
async fn openai_sse_handles_fragmented_text_and_done() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel",
            "lo\"}}]}\n\n: harmless\n\ndata: [DONE]\n\n",
        ],
    }]);
    let response = openai(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "Hello");
    let sent = &requests.lock().unwrap()[0];
    assert!(sent.contains("\"stream\":true"));
    assert!(sent.contains("Bearer secret-sentinel"));
}
