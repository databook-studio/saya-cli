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

#[cfg(test)]
#[path = "extraction_stream_tests/collector.rs"]
mod collector;
#[cfg(test)]
#[path = "extraction_stream_tests/runner.rs"]
mod runner;
