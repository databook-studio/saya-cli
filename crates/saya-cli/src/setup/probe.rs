//! The setup probe seams: the injectable probe types the flow drives
//! ([`FlowProbes`]) and the shared result shape. The real probes live in
//! `probe_database` (the database connection) and `probe_provider` (the
//! consented "ping" request); each is bounded to 15 seconds, one at a time.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use saya_config::MapSecretResolver;
use saya_types::DatabaseProfile;

use super::draft::ProviderDraft;
use super::probe_database::database;
use super::probe_provider::provider;

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
