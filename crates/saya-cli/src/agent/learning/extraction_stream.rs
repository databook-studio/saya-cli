//! Stream collection for the post-turn extraction call, failing fast on
//! non-JSON replies.

use futures_util::StreamExt;
use saya_agent::{
    CancellationToken, ChatProvider, ChatRequest, MAX_STREAM_BYTES, ProviderError, ProviderEvent,
    TokenUsage,
};

/// The extraction reply's content plus the usage the provider reported —
/// the `complete` payload, collected from the stream instead.
#[derive(Debug)]
pub(crate) struct ExtractionReply {
    pub content: String,
    pub usage: Option<TokenUsage>,
}

/// Why collecting the extraction reply ended without a usable reply.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ExtractionStreamError {
    /// The reply's first visible character was neither `{` nor a markdown
    /// fence: the text can never parse as JSON, so the provider was stopped.
    /// Carries any usage reported before the stop — those tokens were billed
    /// regardless of the failure.
    #[error("extraction reply is not JSON")]
    NotJson { usage: Option<TokenUsage> },
    /// A provider or stream failure, exactly as `complete` would report it.
    #[error(transparent)]
    Provider(ProviderError),
}

/// Collects the extraction reply from the provider stream.
///
/// The extraction call asks for JSON, but a provider that ignores JSON mode
/// writes prose until its output ceiling truncates it — measured at 50–91 s
/// per turn before surfacing as `extraction timed out`. A reply that does not
/// start with JSON can never parse, so the first non-whitespace content
/// character decides: `{` or a markdown fence (`strip_markdown_fences`
/// handles the latter) keeps reading to the end, anything else cancels the
/// provider immediately. Reasoning deltas never decide — a reasoning model
/// may think before writing JSON — but they still count toward the byte
/// bound, exactly as in `collect`.
pub(crate) async fn collect_extraction(
    provider: &dyn ChatProvider,
    request: ChatRequest,
) -> Result<ExtractionReply, ExtractionStreamError> {
    let cancellation = CancellationToken::new();
    let mut stream = provider
        .stream(request, cancellation.clone())
        .await
        .map_err(ExtractionStreamError::Provider)?;
    let (mut content, mut usage) = (String::new(), None);
    let (mut bytes, mut decided) = (0usize, false);
    while let Some(event) = stream.next().await {
        match event.map_err(ExtractionStreamError::Provider)? {
            ProviderEvent::TextDelta(delta) => {
                bytes = bounded(bytes, &delta)?;
                content.push_str(&delta);
                // One decision, at the first visible content character; a
                // whitespace-only delta decides nothing.
                if !decided && let Some(first) = content.chars().find(|c| !c.is_whitespace()) {
                    decided = true;
                    if first != '{' && first != '`' {
                        cancellation.cancel();
                        return Err(ExtractionStreamError::NotJson { usage });
                    }
                }
            }
            ProviderEvent::ReasoningDelta(delta) => bytes = bounded(bytes, &delta)?,
            ProviderEvent::Usage(reported) => usage = Some(reported),
            ProviderEvent::ToolCalls(_) => {}
            ProviderEvent::Done => break,
            // `ProviderEvent` is non-exhaustive; an event this collector does
            // not know cannot affect the reply's shape.
            _ => {}
        }
    }
    Ok(ExtractionReply { content, usage })
}

/// The assembled-bytes ceiling from `collect`, covering content and reasoning.
fn bounded(accumulated: usize, delta: &str) -> Result<usize, ExtractionStreamError> {
    let total = accumulated.saturating_add(delta.len());
    if total > MAX_STREAM_BYTES {
        return Err(ExtractionStreamError::Provider(ProviderError::Request(
            "provider stream exceeded size limit".into(),
        )));
    }
    Ok(total)
}

#[cfg(test)]
#[path = "extraction_stream_tests.rs"]
mod tests;
