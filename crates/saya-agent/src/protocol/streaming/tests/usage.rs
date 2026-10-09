use super::*;

/// A stream that emits a `Usage` event produces a response
/// carrying it. The last event wins: Anthropic emits cumulative
/// snapshots, so summing would double-count; the final snapshot is the
/// truth, folded here into the one accumulator the stream already keeps.
#[tokio::test]
async fn collect_threads_the_last_usage_event_into_the_response() {
    let provider = CannedProvider {
        events: vec![
            Ok(ProviderEvent::TextDelta("hi".into())),
            Ok(ProviderEvent::Usage(TokenUsage {
                input_tokens: 5,
                output_tokens: 6,
                cached_input_tokens: Some(90),
                ..Default::default()
            })),
            // A later snapshot supersedes the earlier one — the cumulative
            // output count grows; the cache detail, once set, is not reset.
            Ok(ProviderEvent::Usage(TokenUsage {
                input_tokens: 5,
                output_tokens: 34,
                cached_input_tokens: Some(90),
                ..Default::default()
            })),
            Ok(ProviderEvent::Done),
        ],
    };
    let response = provider
        .complete(ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            ..Default::default()
        })
        .await
        .expect("collect succeeds");
    let usage = response.usage.expect("usage reached the response");
    // The last snapshot, not the first, and not a sum.
    assert_eq!(usage.output_tokens, 34);
    assert_eq!(usage.input_tokens, 5);
    assert_eq!(usage.cached_input_tokens, Some(90));
}

/// Deliverable 2 (the absent case): a stream that emits no `Usage` event
/// leaves `response.usage` `None`, not `Some(TokenUsage::default())` — a
/// silent provider is not mistaken for one that reported a free turn
/// (absent is not zero).
#[tokio::test]
async fn collect_leaves_usage_none_when_the_stream_emits_none() {
    let provider = CannedProvider {
        events: vec![
            Ok(ProviderEvent::TextDelta("hi".into())),
            Ok(ProviderEvent::Done),
        ],
    };
    let response = provider
        .complete(ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            ..Default::default()
        })
        .await
        .expect("collect succeeds");
    assert_eq!(response.usage, None);
}

/// `collect()` still rejects a stream whose
/// accumulated bytes exceed `MAX_STREAM_BYTES`. The size check fires before
/// `usage` is read, so accumulating usage cannot reorder or weaken it.
#[tokio::test]
async fn collect_rejects_a_stream_exceeding_max_stream_bytes() {
    let oversized = "A".repeat(MAX_STREAM_BYTES + 1);
    let provider = CannedProvider {
        events: vec![
            Ok(ProviderEvent::TextDelta(oversized)),
            Ok(ProviderEvent::Done),
        ],
    };
    let error = provider
        .complete(ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            ..Default::default()
        })
        .await
        .expect_err("oversized stream must be rejected");
    assert!(
        error.to_string().contains("size limit"),
        "size-limit error, got: {error}"
    );
}

#[tokio::test]
async fn collect_rejects_a_stream_exceeding_the_semantic_item_limit() {
    const ITEM_LIMIT: usize = 65_536;
    let mut events = vec![
        Ok(ProviderEvent::Usage(TokenUsage::new(40, 10))),
        Ok(ProviderEvent::TextDelta("answer".into())),
    ];
    events.extend((0..ITEM_LIMIT).map(|_| Ok(ProviderEvent::TextDelta(String::new()))));
    events.push(Ok(ProviderEvent::Done));
    let provider = CannedProvider { events };

    let error = provider
        .complete(ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            ..Default::default()
        })
        .await
        .expect_err("the semantic item cap applies even when deltas are empty");

    assert!(matches!(
        error,
        ProviderError::StreamItemLimit {
            usage: Some(usage)
        } if *usage == TokenUsage::new(40, 10)
    ));
}

/// A serialized `ChatResponse`
/// written before `usage` existed — with a `message` key but no `usage` key —
/// deserializes to `usage: None`, so old serialized responses stay valid.
#[test]
fn chat_response_without_usage_key_defaults_to_none() {
    let json =
        r#"{"message":{"role":"assistant","content":"hi","tool_calls":[],"tool_call_id":null}}"#;
    let response: ChatResponse = serde_json::from_str(json).expect("old form deserializes");
    assert_eq!(response.usage, None);
    assert_eq!(response.message.content, "hi");
}

/// A response whose `usage` key carries `null` (an explicit "no usage" on
/// the wire) also deserializes to `None`, distinct from a present usage
/// reporting zeros.
#[test]
fn chat_response_with_null_usage_deserializes_to_none() {
    let json = r#"{"message":{"role":"assistant","content":"hi","tool_calls":[],"tool_call_id":null},"usage":null}"#;
    let response: ChatResponse = serde_json::from_str(json).expect("null usage deserializes");
    assert_eq!(response.usage, None);
}
