use super::*;

// --- reasoning capture -------------------------------------------------

/// `collect()` accumulates `ReasoningDelta` events
/// into `response.reasoning` the way it accumulates `TextDelta` into
/// content. Two deltas concatenate; the result reaches `ChatResponse`.
#[tokio::test]
async fn collect_threads_reasoning_into_the_response() {
    let provider = CannedProvider {
        events: vec![
            Ok(ProviderEvent::ReasoningDelta("first I considered ".into())),
            Ok(ProviderEvent::ReasoningDelta("the time column".into())),
            Ok(ProviderEvent::TextDelta("ok".into())),
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
    assert_eq!(
        response.reasoning.as_deref(),
        Some("first I considered the time column"),
        "reasoning deltas must concatenate into one turn string"
    );
    assert_eq!(response.message.content, "ok");
}

/// a stream that emits no
/// `ReasoningDelta` leaves `response.reasoning` `None`, not
/// `Some(String::new())` — a provider that reports no reasoning is not
/// mistaken for one that reasoned and produced nothing (absent is not
/// zero, mirroring usage).
#[tokio::test]
async fn collect_leaves_reasoning_none_when_the_stream_emits_none() {
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
    assert_eq!(response.reasoning, None);
}

/// `collect()`'s `MAX_STREAM_BYTES` bound covers reasoning too. A
/// hostile endpoint streaming unbounded "thinking" must be rejected exactly
/// as an oversized content stream is — extending the bound, not duplicating
/// it. The check fires before the turn completes.
#[tokio::test]
async fn collect_rejects_a_reasoning_stream_exceeding_max_stream_bytes() {
    let oversized = "A".repeat(MAX_STREAM_BYTES + 1);
    let provider = CannedProvider {
        events: vec![
            Ok(ProviderEvent::ReasoningDelta(oversized)),
            Ok(ProviderEvent::TextDelta("ok".into())),
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
        .expect_err("oversized reasoning stream must be rejected");
    assert!(
        error.to_string().contains("size limit"),
        "size-limit error, got: {error}"
    );
}

/// an unhandled-by-design consumer of `ProviderEvent`
/// renders `ReasoningDelta` to **nothing**, not to an error line. The
/// "unhandled variant became `Not implemented: unrecognized agent event`
/// under a correct answer" regression has happened three times in this
/// repo; this pins that a catch-all consumer stays silent. The loop in
/// `receive` is *not* such a consumer (the main loop keeps its reasoning,
/// a reported zero) — this models the other kind: a drain that only cares
/// about `Usage`/`Done` and ignores the rest.
#[tokio::test]
async fn an_unhandled_consumer_renders_reasoning_to_nothing_not_an_error() {
    let provider = CannedProvider {
        events: vec![
            Ok(ProviderEvent::ReasoningDelta(
                "private chain of thought".into(),
            )),
            Ok(ProviderEvent::TextDelta("ok".into())),
            Ok(ProviderEvent::Done),
        ],
    };
    let mut stream = provider
        .stream(
            ChatRequest {
                model: "m".into(),
                messages: Vec::new(),
                tools: Vec::new(),
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await
        .expect("stream opens");
    // A consumer that only inspects Usage/Done and ignores everything
    // else — the shape of `drain` in tests/providers.rs. It must not error
    // and must not surface the reasoning text.
    let mut saw_done = false;
    let mut errored = None;
    while let Some(event) = stream.next().await {
        match event {
            Ok(ProviderEvent::Done) => saw_done = true,
            Ok(_) => {}
            Err(error) => errored = Some(error),
        }
    }
    assert!(saw_done, "the stream must complete");
    assert!(errored.is_none(), "ignoring ReasoningDelta must not error");
}

/// Reasoning is never replayed to the
/// provider as history. The replay path consumes `&[ChatMessage]`; the
/// turn's reasoning lives on `ChatResponse` and `ChatMessage` has no field
/// for it. So a follow-up request built by taking the response's message
/// into history cannot carry the reasoning, however hard the builder tries
/// — there is nothing to copy. That is the structural guarantee.
#[tokio::test]
async fn reasoning_is_not_replayed_to_the_provider_as_history() {
    use crate::history::build_messages;
    let reasoning_text = "the secret reasoning about row values 9f3a";
    let response = ChatResponse {
        message: ChatMessage::text("assistant", "the answer is 42"),
        reasoning: Some(reasoning_text.into()),
        ..Default::default()
    };
    // The only way history is built: from `ChatMessage`s. The response's
    // reasoning is intentionally not on the message, so it cannot reach
    // history no matter how the caller assembles the next turn.
    let history = vec![
        ChatMessage::text("user", "what is the answer"),
        response.message.clone(),
    ];
    let next = build_messages(None, &[], "follow up", &history, 32 * 1024).expect("builds");
    for message in &next {
        assert!(
            !message.content.contains(reasoning_text),
            "reasoning leaked into replayed history at role {}: {}",
            message.role,
            message.content
        );
    }
    // And the response's message itself — the thing that gets replayed —
    // carries no reasoning.
    assert!(
        !response.message.content.contains(reasoning_text),
        "the replayed ChatMessage carries reasoning"
    );
}
