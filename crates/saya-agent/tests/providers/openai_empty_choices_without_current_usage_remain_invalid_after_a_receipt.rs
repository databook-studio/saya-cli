#[tokio::test]
async fn openai_empty_choices_without_current_usage_remain_invalid_after_a_receipt() {
    use saya_agent::{ProviderError, ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n\n",
            "data: {\"choices\":[]}\n\n",
            "data: [DONE]\n\n",
        ],
    }]);
    let mut stream = openai(base)
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut receipt_seen = false;
    let error = loop {
        match stream.next().await {
            Some(Ok(ProviderEvent::Usage(usage))) => {
                receipt_seen |= usage == TokenUsage::new(4, 2);
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => break error,
            None => panic!("the empty-choices frame without usage must fail"),
        }
    };
    handle.join().unwrap();
    assert!(
        receipt_seen,
        "the actual usage receipt precedes the failed frame"
    );
    assert_eq!(error, ProviderError::InvalidResponse);
}
