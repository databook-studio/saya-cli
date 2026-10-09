use super::{Reply, anthropic, request, server};
use saya_agent::ChatProvider;

/// O1 property 2: Anthropic `stop_reason: "max_tokens"` on `message_delta` is
/// detected — today it ends `Done` and looks complete. The error is typed and
/// carries the partial text the wire already emitted.
#[tokio::test]
async fn anthropic_max_tokens_stop_reason_is_a_typed_truncation_error() {
    use saya_agent::ProviderError;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":34}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let error = anthropic(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(error, ProviderError::OutputTruncated { .. }),
        "anthropic stop_reason max_tokens must be typed truncation, got: {error:?}"
    );
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        unreachable!("matched above");
    };
    assert_eq!(partial_text, "partial");
}

/// O1 non-truncation pin: `stop_reason: "end_turn"` still completes with its
/// content — the new truncation check must not change normal responses.
#[tokio::test]
async fn anthropic_end_turn_still_completes_with_content() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let response = anthropic(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
}
