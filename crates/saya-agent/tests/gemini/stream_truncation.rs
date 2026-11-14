use super::{Reply, gemini, request, server};
use saya_agent::{CancellationToken, ChatProvider, ProviderError};

#[tokio::test]
async fn gemini_truncated_stream_reports_usage_before_the_typed_error() {
    use futures_util::StreamExt;
    use saya_agent::{ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":11}}"#,
        ],
    }]);
    let mut stream = gemini(base)
        .stream(request(), CancellationToken::new())
        .await
        .expect("a truncated response is a stream that ends in the typed error");
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        let terminal = item.is_err();
        items.push(item);
        if terminal {
            break;
        }
    }
    handle.join().unwrap();
    assert_eq!(items.len(), 2, "usage then error, got: {items:?}");
    assert_eq!(items[0], Ok(ProviderEvent::Usage(TokenUsage::new(9, 11))));
    let error = items[1].as_ref().unwrap_err();
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        panic!("expected typed truncation, got: {error:?}");
    };
    assert_eq!(partial_text, "partial");
}

#[tokio::test]
async fn gemini_truncated_stream_without_usage_metadata_emits_no_usage_event() {
    use futures_util::StreamExt;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}]}"#,
        ],
    }]);
    let mut stream = gemini(base)
        .stream(request(), CancellationToken::new())
        .await
        .expect("a truncated response is a stream that ends in the typed error");
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item);
    }
    handle.join().unwrap();
    assert_eq!(items.len(), 1, "only the typed error, got: {items:?}");
    assert!(matches!(
        items[0].as_ref().unwrap_err(),
        ProviderError::OutputTruncated { .. }
    ));
}
