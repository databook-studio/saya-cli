//! The summariser call behind `/compact`: one bounded provider call, at most
//! one retry, and nothing else. Split from `session_compact` so the planning
//! and validation logic stays readable beside it — the pure half decides what
//! to summarise and whether the answer may be believed, this half only asks.

use saya_agent::{ChatMessage, ChatProvider, ChatRequest, ProviderError};

use super::session_compact::{
    COMPACT_MAX_ATTEMPTS, COMPACT_SUMMARY_MAX_CHARS, COMPACT_TIMEOUT, CompactionOutcome,
    CompactionPlan, summarise_prompt, validate,
};

/// Runs one summarising attempt: the same provider and model, its own small
/// output cap and timeout — never inside `receive`, never a turn. The caller
/// retries at most once. A truncated response is detected through the
/// provider's own signal (`ProviderError::OutputTruncated`, which also
/// answers for a summary cut at the cap); every failure leaves the session
/// untouched.
pub(crate) async fn summarise_once(
    provider: &dyn ChatProvider,
    model: &str,
    plan: &CompactionPlan,
) -> Result<CompactionOutcome, String> {
    let request = ChatRequest::new(
        model,
        vec![ChatMessage::text("user", summarise_prompt(&plan.summarise))],
    );
    let response = match tokio::time::timeout(COMPACT_TIMEOUT, provider.complete(request)).await {
        Err(_) => return Err("the summariser timed out".to_string()),
        Ok(Err(ProviderError::OutputTruncated { .. })) => {
            return Err("the summariser response was truncated".to_string());
        }
        Ok(Err(error)) => return Err(format!("the summariser errored: {error}")),
        Ok(Ok(response)) => response,
    };
    let mut summary = response.message.content.trim().to_string();
    // The call's own output cap: bound the summary text so one compaction
    // cannot grow working memory instead of shrinking it. A cut here fails
    // validation (a pinned token may sit past the cut) rather than silently
    // keeping a half-summary.
    if summary.len() > COMPACT_SUMMARY_MAX_CHARS {
        summary.truncate(COMPACT_SUMMARY_MAX_CHARS);
    }
    validate(&summary, &plan.pinned)?;
    Ok(CompactionOutcome {
        summary,
        usage: response.usage,
    })
}

/// Runs the summarising call with at most one retry, then gives up: a failed
/// compaction never fails the session. A pin-dropping summary is not
/// retried — the model already spoke, and a second ask is a second hope.
pub(crate) async fn summarise(
    provider: &dyn ChatProvider,
    model: &str,
    plan: &CompactionPlan,
) -> Result<CompactionOutcome, String> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        match summarise_once(provider, model, plan).await {
            Ok(outcome) => return Ok(outcome),
            Err(reason) if attempts < COMPACT_MAX_ATTEMPTS && retryable(&reason) => continue,
            Err(reason) => return Err(reason),
        }
    }
}

/// Only transport failures are retried: a timeout or a request error may pass
/// on a second attempt. A truncation, an empty summary, or a dropped pin is
/// the model's answer, not the wire — retrying rewrites nothing.
fn retryable(reason: &str) -> bool {
    reason.contains("timed out") || reason.contains("the summariser errored")
}
