use super::{Reply, anthropic, request, server};
use saya_agent::ChatProvider;

#[tokio::test]
async fn anthropic_truncated_stream_reports_usage_when_delta_and_stop_share_a_read() {
    use futures_util::StreamExt;
    use saya_agent::{CancellationToken, ProviderError, ProviderEvent};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":9}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}

"#,
            r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":11}}

event: message_stop
data: {"type":"message_stop"}

"#,
        ],
    }]);
    let mut stream = anthropic(base)
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        let terminal = item.is_err();
        items.push(item);
        if terminal {
            break;
        }
    }
    handle.join().unwrap();
    let last = items.last().expect("a terminal item");
    let error = last.as_ref().unwrap_err();
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        panic!("expected typed truncation, got: {error:?}");
    };
    assert_eq!(partial_text, "partial");
    let usage = items.iter().rev().find_map(|item| match item {
        Ok(ProviderEvent::Usage(usage)) => Some(*usage),
        _ => None,
    });
    let usage = usage.expect("a truncated response still reports its usage");
    assert_eq!(usage.output_tokens, 11, "items: {items:?}");
}

#[tokio::test]
async fn anthropic_truncated_stream_reports_usage_when_frames_arrive_separately() {
    use futures_util::StreamExt;
    use saya_agent::{CancellationToken, ProviderError, ProviderEvent};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":9}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}

"#,
            r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":11}}

"#,
            r#"event: message_stop
data: {"type":"message_stop"}

"#,
        ],
    }]);
    let mut stream = anthropic(base)
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        let terminal = item.is_err();
        items.push(item);
        if terminal {
            break;
        }
    }
    handle.join().unwrap();
    let last = items.last().expect("a terminal item");
    let error = last.as_ref().unwrap_err();
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        panic!("expected typed truncation, got: {error:?}");
    };
    assert_eq!(partial_text, "partial");
    let usage = items.iter().rev().find_map(|item| match item {
        Ok(ProviderEvent::Usage(usage)) => Some(*usage),
        _ => None,
    });
    let usage = usage.expect("a truncated response still reports its usage");
    assert_eq!(usage.output_tokens, 11, "items: {items:?}");
}
