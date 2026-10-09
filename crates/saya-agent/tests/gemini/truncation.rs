use super::{Reply, gemini, request, server};
use saya_agent::ChatProvider;

/// O1 property 3: Gemini `finishReason: "MAX_TOKENS"` is detected — today it
/// passes silently and looks complete. The error is typed and carries the
/// partial text.
#[tokio::test]
async fn gemini_max_tokens_finish_reason_is_a_typed_truncation_error() {
    use saya_agent::ProviderError;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}]}"#,
        ],
    }]);
    let error = gemini(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(error, ProviderError::OutputTruncated { .. }),
        "gemini finishReason MAX_TOKENS must be typed truncation, got: {error:?}"
    );
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        unreachable!("matched above");
    };
    assert_eq!(partial_text, "partial");
}

/// O1 non-truncation pin: a normal Gemini response (`finishReason: "STOP"`)
/// still completes with its content.
#[tokio::test]
async fn gemini_stop_finish_reason_still_completes_with_content() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#,
        ],
    }]);
    let response = gemini(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
}
