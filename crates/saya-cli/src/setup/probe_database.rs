//! The database probe (S16 invariant 2f): one connection attempt via the real
//! connector factory, bounded to 15 seconds — 120 for Snowflake browser-SSO,
//! whose browser flow has the same window in the connector — never executing
//! SQL. Failures are classified best-effort from the error text; the raw
//! message always survives so the user sees the real cause.

use std::future::Future;
use std::time::Duration;

use saya_config::SecretResolver;
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::{DatabaseProfile, SnowflakeAuth};

use super::probe::{PROBE_TIMEOUT, ProbeResult};

/// The browser-SSO probe window; the connector's own browser flow allows 120 s.
pub(crate) const SSO_PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// Asked before a browser-SSO probe opens a browser; declining skips it.
pub(crate) const SSO_CONSENT_PROMPT: &str =
    "This opens your browser to sign in to Snowflake. Continue? [y/N] ";

/// Only Snowflake browser-SSO opens a browser during the probe.
pub(crate) fn needs_sso_consent(profile: &DatabaseProfile) -> bool {
    matches!(
        profile,
        DatabaseProfile::Snowflake {
            auth_type: SnowflakeAuth::Externalbrowser,
            ..
        }
    )
}

/// The window the probe of this profile gets: 120 s for browser-SSO, else 15 s.
pub(crate) fn probe_timeout_for(profile: &DatabaseProfile) -> Duration {
    if needs_sso_consent(profile) {
        SSO_PROBE_TIMEOUT
    } else {
        PROBE_TIMEOUT
    }
}

/// The line the flow prints before probing, naming the window actually used.
pub(crate) fn probe_window_label(profile: &DatabaseProfile) -> &'static str {
    if needs_sso_consent(profile) {
        "120 seconds"
    } else {
        "15 seconds"
    }
}

/// Connects to the profile once. A build or connect failure is classified
/// best-effort; the original message is always included.
pub(crate) async fn database(
    profile: &DatabaseProfile,
    resolver: &dyn SecretResolver,
) -> ProbeResult {
    let timeout = probe_timeout_for(profile);
    let connect = || async {
        let options = ConnectorOptions {
            read_only: true,
            ..Default::default()
        };
        // Interactive setup may drive interactive auth (browser SSO), so the
        // factory is allowed to prompt; non-interactive engines ignore this.
        match build_connector_with_prompt(profile, resolver, options, true).await {
            Ok(connector) => connector
                .connect()
                .await
                .map_err(|error| classify(&error.to_string())),
            Err(error) => Err(classify(&error.to_string())),
        }
    };
    database_with(timeout, connect).await
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
