/// a stream with no `reasoning_content`
/// leaves `ChatResponse.reasoning` `None` — no error, no behaviour change
///.
#[tokio::test]
async fn openai_stream_without_reasoning_leaves_it_none() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: [DONE]\n\n",
        ],
    }]);
    let response = openai(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(response.reasoning, None);
}
