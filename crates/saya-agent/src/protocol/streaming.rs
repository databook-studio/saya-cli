use crate::{ChatMessage, ChatRequest, ChatResponse, ProviderError, ToolCall};
use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Notify;

/// Ceiling on one provider response's accumulated bytes. A misbehaving or
/// hostile endpoint must not be able to stream unbounded data into memory.
pub const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;

/// Token counts reported by a provider for one response. Providers that do
/// not report usage simply never emit it.
///
/// Every field beyond `input_tokens`/`output_tokens` is `Option<u64>`: a
/// provider that does not report a number leaves it `None` (invariant: absent
/// is not zero — a cache hit rate computed over `None` is *unknown*, not 0%).
/// The two existing fields keep their meaning and type (`u64`) so callers
/// that sum or copy them are unaffected. The struct stays `Copy` because every
/// field is `Copy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens served from the provider's prompt cache. **Inclusive of
    /// `input_tokens`** (a subset of the prompt): a 90% cache hit means 90% of
    /// `input_tokens` was read from cache, not added on top. Wire sources:
    /// OpenAI `usage.prompt_tokens_details.cached_tokens`,
    /// Anthropic `usage.cache_read_input_tokens`,
    /// Gemini `usageMetadata.cachedContentTokenCount`. `None` when the provider
    /// does not report cache reads (Ollama; any provider that omits the field).
    pub cached_input_tokens: Option<u64>,
    /// Input tokens spent *creating* a cache entry (Anthropic only). Inclusive
    /// of `input_tokens`, and kept distinct from `cached_input_tokens` because
    /// creation bills differently from reads. Wire source: Anthropic
    /// `usage.cache_creation_input_tokens`. `None` for providers with no
    /// cache-write concept (OpenAI, Gemini, Ollama).
    pub cache_creation_input_tokens: Option<u64>,
    /// Tokens spent on chain-of-thought reasoning. The inclusive/separate
    /// split is per-provider and documented here because a normalising layer
    /// that guesses wrong is worse than the raw number: on **OpenAI** this is
    /// `usage.completion_tokens_details.reasoning_tokens`, which sits *inside*
    /// `completion_tokens` and is therefore **inclusive of `output_tokens`**
    /// — do not add it to `output_tokens`. On **Gemini** it is
    /// `usageMetadata.thoughtsTokenCount`, reported *separately* from
    /// `candidatesTokenCount` (not part of `output_tokens`). Anthropic counts
    /// reasoning inside its output tokens (no separate field) and Ollama
    /// reports none; both leave this `None`.
    pub reasoning_tokens: Option<u64>,
}

/// One increment from a provider stream.
///
/// `#[non_exhaustive]` is here for semver headroom: `saya-agent` is a published
/// crate, and without it every new variant is a breaking change for anyone
/// matching on this enum.
///
/// It is deliberately *not* a defence against the "unhandled variant becomes
/// `Not implemented: …` under a correct answer" regression that has happened
/// three times here (most recently `a4fe388`). The attribute forces external
/// matches to carry a `_` arm, and a `_` arm is precisely what swallows a new
/// variant — it mandates the catch-all rather than preventing it. That
/// regression was in `AgentEvent` handling in `saya-cli`, and the fix for it is
/// `stream_render.rs` returning `Option`, not anything on this type. Nothing
/// outside this crate matches `ProviderEvent` today.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderEvent {
    TextDelta(String),
    /// Chain-of-thought from a reasoning model. Captured unconditionally
    /// (invariant 4: parse whether or not the user has asked to see it — the
    /// `show_thinking` toggle is S24, not a precondition for capture) and
    /// accumulated under the same `MAX_STREAM_BYTES` bound as `TextDelta`, so a
    /// hostile endpoint cannot stream unbounded "thinking" into memory. Never
    /// reaches a session file or the provider as history: reasoning lives on
    /// `ChatResponse` (transport for one call), never on `ChatMessage` (what
    /// gets replayed and persisted), so there is no field to copy it through
    /// (S23 invariants 1 and 2, made structural by the type choice in Q2).
    ReasoningDelta(String),
    ToolCalls(Vec<ToolCall>),
    /// The provider's cumulative token counts so far for this response.
    Usage(TokenUsage),
    Done,
}
pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send>>;

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<CancellationState>);
#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}
impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    pub async fn cancelled(&self) {
        let notified = self.0.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

#[async_trait]
pub trait ChatProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError>;
    async fn stream(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let response = self.complete(request).await?;
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let events = if response.message.tool_calls.is_empty() {
            vec![
                ProviderEvent::TextDelta(response.message.content),
                ProviderEvent::Done,
            ]
        } else {
            vec![
                ProviderEvent::ToolCalls(response.message.tool_calls),
                ProviderEvent::Done,
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
    async fn collect(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut stream = self.stream(request, CancellationToken::new()).await?;
        let (mut content, mut tool_calls, mut complete) = (String::new(), Vec::new(), false);
        // Reasoning is accumulated alongside content (Q1/Q3): a single string
        // per turn, no ordering relative to text. The bound covers reasoning
        // too — a hostile endpoint streaming unbounded "thinking" must not
        // exhaust memory, so each delta is checked against `MAX_STREAM_BYTES`
        // exactly as `TextDelta` is (invariant 2 of `collect()`). `None` when
        // the stream emits no `ReasoningDelta`, so a provider that reports no
        // reasoning stays distinguishable from one that reported an empty
        // string (absent is not zero, mirrored from usage).
        let mut reasoning = None;
        // `None` until the stream emits a `Usage` event; the last event wins
        // (Q2). OpenAI emits one trailing usage-only chunk; Anthropic emits
        // cumulative snapshots on `message_start`/`message_delta`, so summing
        // would double-count — the final snapshot is the truth, exactly as the
        // stream's own accumulator already folds them into one running total.
        // Keeping `None` when no event arrives preserves "absent is not zero"
        // (invariant 1): a provider that reports nothing stays distinguishable
        // from one that reported zeros.
        let mut usage = None;
        while let Some(event) = stream.next().await {
            match event? {
                ProviderEvent::TextDelta(value) => {
                    if content.len().saturating_add(value.len()) > MAX_STREAM_BYTES {
                        return Err(ProviderError::Request(
                            "provider stream exceeded size limit".into(),
                        ));
                    }
                    content.push_str(&value);
                }
                ProviderEvent::ReasoningDelta(value) => {
                    let accumulated = reasoning.get_or_insert_with(String::new);
                    if accumulated.len().saturating_add(value.len()) > MAX_STREAM_BYTES {
                        return Err(ProviderError::Request(
                            "provider stream exceeded size limit".into(),
                        ));
                    }
                    accumulated.push_str(&value);
                }
                ProviderEvent::ToolCalls(calls) => tool_calls.extend(calls),
                ProviderEvent::Usage(reported) => usage = Some(reported),
                ProviderEvent::Done => complete = true,
            }
        }
        if !complete || (content.trim().is_empty() && tool_calls.is_empty()) {
            return Err(ProviderError::InvalidResponse);
        }
        Ok(ChatResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content,
                tool_calls,
                tool_call_id: None,
            },
            reasoning,
            usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CancellationToken, ChatProvider, ChatRequest, ChatResponse, MAX_STREAM_BYTES,
        ProviderError, ProviderEvent, ProviderStream, TokenUsage,
    };
    use crate::ChatMessage;
    use async_trait::async_trait;
    use futures_util::{StreamExt, stream};
    use std::time::Duration;

    /// Deliverable 5: `Some(0)` is a *report* of zero and must survive
    /// distinct from `None` (an unreported number). If this cannot be written,
    /// the type is wrong — here the two compare unequal and `Some(0)` survives
    /// a `Copy`.
    #[test]
    fn some_zero_usage_is_distinct_from_absent() {
        let reported_zero = TokenUsage {
            cached_input_tokens: Some(0),
            ..Default::default()
        };
        let unreported = TokenUsage::default();
        assert_ne!(reported_zero, unreported);
        let copy = reported_zero;
        assert_eq!(copy.cached_input_tokens, Some(0));
        assert_eq!(unreported.cached_input_tokens, None);
    }

    /// `TokenUsage` stays `Copy` (invariant 3): assigning rebinds a value, not
    /// a borrow, and the new fields default to `None` without disturbing the
    /// two existing counters.
    #[test]
    fn token_usage_is_copy_and_new_fields_default_to_none() {
        let mut usage = TokenUsage {
            input_tokens: 3,
            output_tokens: 7,
            ..Default::default()
        };
        assert_eq!(usage.cached_input_tokens, None);
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.reasoning_tokens, None);
        let copy = usage;
        usage.cached_input_tokens = Some(5);
        // The copy is independent: assigning to `usage` did not mutate `copy`.
        assert_eq!(copy.cached_input_tokens, None);
        assert_eq!(usage.cached_input_tokens, Some(5));
    }

    #[tokio::test]
    async fn cancellation_waiter_does_not_miss_a_notification() {
        for _ in 0..64 {
            let token = CancellationToken::new();
            let waiter = token.clone();
            let task = tokio::spawn(async move { waiter.cancelled().await });
            tokio::task::yield_now().await;
            token.cancel();
            tokio::time::timeout(Duration::from_millis(100), task)
                .await
                .unwrap()
                .unwrap();
        }
    }

    /// A minimal provider whose `stream()` replays a canned event list, so the
    /// default `collect()` can be exercised without a network. `complete()`
    /// forwards to `collect()` exactly as the three real providers do.
    struct CannedProvider {
        events: Vec<Result<ProviderEvent, ProviderError>>,
    }

    #[async_trait]
    impl ChatProvider for CannedProvider {
        fn name(&self) -> &str {
            "canned"
        }
        async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.collect(request).await
        }
        async fn stream(
            &self,
            _request: ChatRequest,
            _cancellation: CancellationToken,
        ) -> Result<ProviderStream, ProviderError> {
            Ok(Box::pin(stream::iter(self.events.clone())))
        }
    }

    /// Deliverable 2: a stream that emits a `Usage` event produces a response
    /// carrying it. The last event wins (Q2): Anthropic emits cumulative
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
    /// (invariant 1: absent is not zero).
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

    /// Deliverable 5 / invariant 2: `collect()` still rejects a stream whose
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

    /// The back-compat guarantee (deliverable 1): a serialized `ChatResponse`
    /// written before this slice — with a `message` key but no `usage` key —
    /// deserializes to `usage: None`, so old serialized responses stay valid.
    #[test]
    fn chat_response_without_usage_key_defaults_to_none() {
        let json = r#"{"message":{"role":"assistant","content":"hi","tool_calls":[],"tool_call_id":null}}"#;
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

    // --- S23: reasoning capture -------------------------------------------------

    /// S23 deliverable 1 / Q1: `collect()` accumulates `ReasoningDelta` events
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

    /// S23 deliverable 6 (the absent case): a stream that emits no
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

    /// S23 Q1: `collect()`'s `MAX_STREAM_BYTES` bound covers reasoning too. A
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

    /// S23 deliverable 1: an unhandled-by-design consumer of `ProviderEvent`
    /// renders `ReasoningDelta` to **nothing**, not to an error line. The
    /// "unhandled variant became `Not implemented: unrecognized agent event`
    /// under a correct answer" regression has happened three times in this
    /// repo; this pins that a catch-all consumer stays silent. The loop in
    /// `receive` is *not* such a consumer (the main loop keeps its reasoning,
    /// S20 invariant 2) — this models the other kind: a drain that only cares
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

    /// S23 deliverable 5 / invariant 2: reasoning is never replayed to the
    /// provider as history. The replay path consumes `&[ChatMessage]`; the
    /// turn's reasoning lives on `ChatResponse` and `ChatMessage` has no field
    /// for it. So a follow-up request built by taking the response's message
    /// into history cannot carry the reasoning, however hard the builder tries
    /// — there is nothing to copy. This is the structural guarantee Q2 makes.
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
        let next = build_messages(None, &[], "follow up", &history).expect("builds");
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
}
