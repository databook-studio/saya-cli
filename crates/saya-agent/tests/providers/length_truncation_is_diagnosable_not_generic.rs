#[tokio::test]
async fn length_truncation_is_diagnosable_not_generic() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n",
        ],
    }]);
    let error = openai(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    let text = error.to_string();
    assert!(
        text.contains("truncated") && text.contains("output-token"),
        "{text}"
    );
}
