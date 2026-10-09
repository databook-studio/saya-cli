/// O1 property 1 (red first): a capped OpenAI response must surface the typed
/// truncation error — not a generic request failure — and carry the partial
/// text the wire had already emitted.
#[tokio::test]
async fn openai_length_truncation_is_a_typed_error_carrying_partial_text() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n",
        ],
    }]);
    let error = openai(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(error, ProviderError::OutputTruncated { .. }),
        "a capped response must be the typed truncation error, got: {error:?}"
    );
    let saya_agent::ProviderError::OutputTruncated { partial_text, .. } = error else {
        unreachable!("matched above");
    };
    assert_eq!(partial_text, "partial");
}
