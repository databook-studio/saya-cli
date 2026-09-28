//! The database probe (S16 invariant 2f): one connection attempt via the real
//! connector factory, bounded to 15 seconds, never executing SQL. Failures
//! are classified best-effort from the error text; the raw message always
//! survives so the user sees the real cause.

use std::future::Future;
use std::time::Duration;

use saya_config::SecretResolver;
use saya_connectors::{ConnectorOptions, build_connector};
use saya_types::DatabaseProfile;

use super::probe::{PROBE_TIMEOUT, ProbeResult};

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
