//! The provider probe (S16 invariant 2f): one "ping" request, sent only
//! after explicit consent, bounded to 15 seconds. The response's content is
//! deliberately discarded — the probe claims nothing about what the model
//! said, only that it answered.

use std::future::Future;
use std::time::Duration;

use saya_config::{ResolvedAi, SecretResolver};
use saya_types::SecretRef;

use super::draft::ProviderDraft;
use super::probe::{PROBE_TIMEOUT, ProbeResult};

/// Sends one "ping" request to the drafted provider. A provider build or a
/// failed request is reported; the reply itself is dropped.
pub(crate) async fn provider(draft: &ProviderDraft, resolver: &dyn SecretResolver) -> ProbeResult {
    let config = resolved_ai(draft);
    let send = || async {
        let built = crate::agent::provider::build(&config, resolver)
            .map_err(|error| format!("provider unavailable: {error}"))?;
        built
            .complete(ping_request(&config.model))
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    };
    provider_with(PROBE_TIMEOUT, send).await
}

/// The provider probe with an injectable send step.
pub(crate) async fn provider_with<F, Fut>(timeout: Duration, send: F) -> ProbeResult
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    match tokio::time::timeout(timeout, send()).await {
        Ok(Ok(())) => ProbeResult::ok("provider answered"),
        Ok(Err(reason)) => ProbeResult::failure(format!("provider probe failed: {reason}")),
        Err(_) => ProbeResult::failure(format!(
            "provider probe timed out after {:.1}s",
            timeout.as_secs_f64()
        )),
    }
}

/// The provider probe's request: exactly one user message carrying the word
/// "ping" — no tools, no system message, no schema, nothing database-shaped.
pub(crate) fn ping_request(model: &str) -> saya_agent::ChatRequest {
    saya_agent::ChatRequest::new(
        model.to_owned(),
        vec![saya_agent::ChatMessage::text("user", "ping")],
    )
}

/// The probe-only `ResolvedAi` for a draft: config-file defaults for the
/// sampling fields, empty retry delays so a single attempt cannot stretch
/// past the probe window, and the API key as an env reference only.
fn resolved_ai(draft: &ProviderDraft) -> ResolvedAi {
    ResolvedAi {
        provider: draft.provider,
        model: draft.model.clone(),
        base_url: draft.base_url.clone(),
        api_key: draft.api_key_env.clone().map(|env| SecretRef::Env { env }),
        allow_data_sharing: false,
        temperature: 0.1,
        timeout_seconds: 60,
        idle_timeout_seconds: 90,
        max_output_tokens: 4096,
        max_output_tokens_is_default: true,
        context_byte_budget: 256 * 1024,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: Vec::new(),
    }
}
