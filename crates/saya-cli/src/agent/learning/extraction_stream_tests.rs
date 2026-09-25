//! Tests for the fail-fast extraction stream collector.

use super::{ExtractionStreamError, collect_extraction};
use crate::agent::learning::{ExtractionError, ExtractionRunnerError, TurnRecord, run_extraction};
use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;
use async_trait::async_trait;
use futures_util::StreamExt;
use saya_agent::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, MAX_STREAM_BYTES, ProviderError,
    ProviderEvent, ProviderStream, TokenUsage,
};
use saya_store::SqliteStateStore;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agent::learning::turn_table::TurnObjectTable;

/// A provider whose stream replays a canned event sequence, optionally never
/// ending, and records the cancellation token the collector handed it.
/// `complete` forwards to `collect` so the stub phase of the collector can be
/// exercised through the same trait object `run_extraction` receives.
struct ScriptedProvider {
    events: Vec<Result<ProviderEvent, ProviderError>>,
    hangs_after: bool,
    captured: Mutex<Option<CancellationToken>>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted-extraction"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.collect(request).await
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.captured.lock().unwrap() = Some(cancellation);
        let scripted = futures_util::stream::iter(self.events.clone());
        let full: ProviderStream = if self.hangs_after {
            Box::pin(scripted.chain(futures_util::stream::pending()))
        } else {
            Box::pin(scripted)
        };
        Ok(full)
    }
}

fn delta(text: &str) -> Result<ProviderEvent, ProviderError> {
    Ok(ProviderEvent::TextDelta(text.into()))
}

fn reasoning(text: &str) -> Result<ProviderEvent, ProviderError> {
    Ok(ProviderEvent::ReasoningDelta(text.into()))
}

fn usage() -> Result<ProviderEvent, ProviderError> {
    Ok(ProviderEvent::Usage(TokenUsage::new(11, 7)))
}

fn done() -> Result<ProviderEvent, ProviderError> {
    Ok(ProviderEvent::Done)
}

fn request() -> ChatRequest {
    ChatRequest::new("test-model", Vec::new())
}

fn scripted(
    events: Vec<Result<ProviderEvent, ProviderError>>,
    hangs_after: bool,
) -> ScriptedProvider {
    ScriptedProvider {
        events,
        hangs_after,
        captured: Mutex::new(None),
    }
}

const PROSE: &str = "Let me analyze this conversation to extract knowledge.";

/// The measured failure shape: a model ignoring JSON mode writes prose until
/// its output ceiling truncates it. The first visible character decides, so
/// the collector returns within 1s with an error naming non-JSON instead of
/// waiting out a stream that can never parse.
#[tokio::test]
async fn prose_first_reply_fails_fast() {
    let provider = scripted(vec![delta(PROSE)], true);
    let collected = tokio::time::timeout(
        Duration::from_secs(1),
        collect_extraction(&provider, request()),
    )
    .await
    .expect("the collector must return within 1s, not wait out the stream");
    let error = collected.expect_err("a prose reply must be refused");
    assert!(
        error.to_string().contains("not JSON"),
        "the error must name non-JSON, got: {error}"
    );
}

#[tokio::test]
async fn the_stop_cancels_the_provider_token() {
    let provider = scripted(vec![delta("Sure, let me think about this database.")], true);
    let collected = tokio::time::timeout(
        Duration::from_secs(1),
        collect_extraction(&provider, request()),
    )
    .await
    .expect("the collector must return within 1s, not wait out the stream");
    let _ = collected.expect_err("a prose reply must be refused");
    let token = provider
        .captured
        .lock()
        .unwrap()
        .clone()
        .expect("the stream was handed a cancellation token");
    assert!(
        token.is_cancelled(),
        "the stop path must cancel the token handed to the provider"
    );
}

#[tokio::test]
async fn reasoning_before_json_is_accepted() {
    let provider = scripted(
        vec![
            reasoning("thinking"),
            reasoning("thinking"),
            reasoning("thinking"),
            delta(r#"{"proposals": []}"#),
            done(),
        ],
        false,
    );
    let reply = collect_extraction(&provider, request())
        .await
        .expect("reasoning before JSON must not stop the collector");
    assert_eq!(reply.content, r#"{"proposals": []}"#);
}

#[tokio::test]
async fn json_split_across_deltas_is_accepted() {
    let provider = scripted(
        vec![
            delta("  \n"),
            delta("{"),
            delta(r#""proposals": []}"#),
            done(),
        ],
        false,
    );
    let reply = collect_extraction(&provider, request())
        .await
        .expect("whitespace before the JSON must not stop the collector");
    assert_eq!(reply.content, "  \n{\"proposals\": []}");
}

#[tokio::test]
async fn fenced_json_is_accepted() {
    let provider = scripted(
        vec![delta("```json\n{\"proposals\": []}\n```"), done()],
        false,
    );
    let reply = collect_extraction(&provider, request())
        .await
        .expect("a backtick-led reply may be fenced JSON");
    assert_eq!(reply.content, "```json\n{\"proposals\": []}\n```");
}

#[tokio::test]
async fn usage_survives_both_paths() {
    let reported = TokenUsage::new(11, 7);

    let success = scripted(vec![usage(), delta(r#"{"proposals": []}"#), done()], false);
    let reply = collect_extraction(&success, request())
        .await
        .expect("a JSON reply succeeds");
    assert_eq!(
        reply.usage,
        Some(reported),
        "usage reaches the success path"
    );

    let stopped = scripted(vec![usage(), delta(PROSE)], true);
    let collected = tokio::time::timeout(
        Duration::from_secs(1),
        collect_extraction(&stopped, request()),
    )
    .await
    .expect("the collector must return within 1s, not wait out the stream");
    match collected.expect_err("a prose reply must be refused") {
        ExtractionStreamError::NotJson { usage } => {
            assert_eq!(
                usage,
                Some(reported),
                "usage reported before the stop is kept"
            );
        }
        other => panic!("expected NotJson, got {other:?}"),
    }
}

#[tokio::test]
async fn oversize_stream_is_rejected() {
    let oversized = format!("{{{}", "A".repeat(MAX_STREAM_BYTES));
    let provider = scripted(vec![delta(&oversized)], false);
    let error = collect_extraction(&provider, request())
        .await
        .expect_err("a stream past the byte bound must be rejected");
    match error {
        ExtractionStreamError::Provider(ProviderError::Request(message)) => {
            assert!(
                message.contains("size limit"),
                "size-limit error, got: {message}"
            );
        }
        other => panic!("expected the provider size-limit error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_mid_stream_error_is_returned_as_the_error() {
    let provider = scripted(
        vec![
            delta("{"),
            Err(ProviderError::Request("mid-stream failure".into())),
        ],
        false,
    );
    let error = collect_extraction(&provider, request())
        .await
        .expect_err("a stream error must be returned as the error");
    match error {
        ExtractionStreamError::Provider(ProviderError::Request(message)) => {
            assert_eq!(message, "mid-stream failure");
        }
        other => panic!("expected the provider error, got {other:?}"),
    }
}

/// The runner-level stop path: a prose reply surfaces through
/// `ExtractionOutcome::failed` as `NotJson`, so the user sees the existing
/// "memory not recorded · extraction failed" line and the trace prints the
/// cause.
#[tokio::test]
async fn a_prose_extraction_reply_makes_run_extraction_return_a_failed_outcome() {
    let registry = ConnectionRegistry::new("analytics");
    let root = temp_root("prose_runner");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let receipt = RecallReceipt::ran_empty(false);

    let mut object_table = TurnObjectTable::new();
    object_table.register("analytics", "raw.orders", &["status".into()]);

    let record = TurnRecord {
        prompt: "what is orders status".into(),
        assistant_answer: "status is pending".into(),
        object_table,
        user_corrections: Vec::new(),
        override_findings: Vec::new(),
        supplied_claims: Vec::new(),
        omitted: 0,
    };

    let provider = scripted(vec![delta(PROSE), done()], false);

    let outcome = run_extraction(
        &provider,
        "test-model",
        &record,
        &registry,
        &store,
        &receipt,
    )
    .await;

    assert!(
        matches!(
            outcome.dtos,
            Err(ExtractionRunnerError::Extraction(ExtractionError::NotJson))
        ),
        "a prose reply must fail the run as NotJson, got: {:?}",
        outcome.dtos
    );

    let _ = fs::remove_dir_all(root);
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-extraction-stream-test-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
