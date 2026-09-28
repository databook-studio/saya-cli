//! The setup probes: one database connection attempt and — only after
//! explicit consent — one provider request. Each is bounded to 15 seconds and
//! runs one at a time; neither executes SQL and neither sends schema or rows.
//! The seams (`*_with`) take the future factory as a parameter so tests can
//! inject never-resolving or instantly-failing probes.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ChatMessage, ChatRequest};
use saya_config::{MapSecretResolver, ResolvedAi, SecretResolver};
use saya_connectors::{ConnectorOptions, build_connector};
use saya_types::{DatabaseProfile, SecretRef};

use super::draft::ProviderDraft;

/// Each probe gets this window, one at a time (D11: probes are 15 s each).
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// One probe's outcome: `ok` and the exact line shown to the user. The
/// message names what was actually verified — "database reachable" is never
/// "configuration valid".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProbeResult {
    pub(crate) ok: bool,
    pub(crate) message: String,
}

impl ProbeResult {
    pub(crate) fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
        }
    }

    pub(crate) fn failure(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
        }
    }
}

/// The probe seams the flow drives. Tests inject closures that answer
/// instantly; the real wiring is [`FlowProbes::real`].
pub(crate) type DatabaseProbe = Arc<
    dyn Fn(&DatabaseProfile) -> Pin<Box<dyn Future<Output = ProbeResult> + Send>> + Send + Sync,
>;
pub(crate) type ProviderProbe =
    Arc<dyn Fn(&ProviderDraft) -> Pin<Box<dyn Future<Output = ProbeResult> + Send>> + Send + Sync>;

pub(crate) struct FlowProbes {
    pub(crate) database: DatabaseProbe,
    pub(crate) provider: ProviderProbe,
}

impl FlowProbes {
    /// The real probes: an env-backed resolver built from `env`, the real
    /// connector factory, and the real provider builder.
    pub(crate) fn real(env: &BTreeMap<String, String>) -> Self {
        let database_env = env.clone();
        let db: DatabaseProbe = Arc::new(move |profile| {
            let profile = profile.clone();
            let env = database_env.clone();
            Box::pin(async move {
                let resolver = MapSecretResolver::new(env);
                database(&profile, &resolver).await
            })
        });
        let provider_env = env.clone();
        let prov: ProviderProbe = Arc::new(move |draft| {
            let draft = draft.clone();
            let env = provider_env.clone();
            Box::pin(async move {
                let resolver = MapSecretResolver::new(env);
                provider(&draft, &resolver).await
            })
        });
        Self {
            database: db,
            provider: prov,
        }
    }
}

/// Connects to the profile once. A build or connect failure is classified
/// best-effort; the original message is always included.
pub(crate) async fn database(
    profile: &DatabaseProfile,
    resolver: &dyn SecretResolver,
) -> ProbeResult {
    let connect = || async {
        let options = ConnectorOptions {
            read_only: true,
            ..Default::default()
        };
        match build_connector(profile, resolver, options).await {
            Ok(connector) => connector
                .connect()
                .await
                .map_err(|error| classify(&error.to_string())),
            Err(error) => Err(classify(&error.to_string())),
        }
    };
    database_with(PROBE_TIMEOUT, connect).await
}

/// Sends one "ping" request to the drafted provider. Building the provider or
/// completing the request fails are reported; the response's content is
/// deliberately discarded — the probe claims nothing about what the model
/// said, only that it answered.
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

/// The database probe with an injectable connect step.
pub(crate) async fn database_with<F, Fut>(timeout: Duration, connect: F) -> ProbeResult
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    match tokio::time::timeout(timeout, connect()).await {
        Ok(Ok(())) => ProbeResult::ok("database reachable"),
        Ok(Err(reason)) => ProbeResult::failure(format!("database probe failed: {reason}")),
        Err(_) => ProbeResult::failure(format!(
            "database probe timed out after {:.1}s: the connection never completed",
            timeout.as_secs_f64()
        )),
    }
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
pub(crate) fn ping_request(model: &str) -> ChatRequest {
    ChatRequest::new(model.to_owned(), vec![ChatMessage::text("user", "ping")])
}

/// Best-effort classification from the error text: the connector error does
/// not carry structured kinds for these, so the wording decides — and the
/// raw message always survives so the user sees the real cause.
pub(crate) fn classify(error: &str) -> String {
    let lower = error.to_lowercase();
    let kind = if [
        "auth",
        "password",
        "credential",
        "access denied",
        "permission",
    ]
    .into_iter()
    .any(|needle| lower.contains(needle))
    {
        "authentication failed"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timed out"
    } else if lower.contains("tls") || lower.contains("ssl") || lower.contains("certificate") {
        "tls handshake failed"
    } else if [
        "does not exist",
        "no such",
        "not found",
        "unknown database",
        "unable to open",
    ]
    .into_iter()
    .any(|needle| lower.contains(needle))
    {
        "database not found"
    } else {
        "could not connect"
    };
    format!("{kind}: {error}")
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
