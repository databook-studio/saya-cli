//! What the MCP tool bodies share (task Db): the composed runtime and the
//! state store, built once at startup and never reloaded. A profile call is
//! resolved through the startup allowlist first — the allowlist, not the
//! config, is what a client can reach — then through the same
//! `named_profile` path every other surface uses.

use crate::config::runtime::RuntimeConfig;
use crate::profile_identity::profile_identity;
use saya_store::SqliteStateStore;
use saya_types::{DatabaseProfile, ProfileIdentity};

pub(crate) struct McpContext {
    pub(crate) runtime: RuntimeConfig,
    pub(crate) store: SqliteStateStore,
    /// The replay capture is thread-local (the process-output seam), so only
    /// one investigation replay's capture window may be open at a time; this
    /// is the one-at-a-time slot.
    pub(crate) replay_slot: tokio::sync::Mutex<()>,
}

impl McpContext {
    /// Resolves an argument-named profile through the startup allowlist. A
    /// name outside the allowlist is refused even when it is configured —
    /// the client can never widen the set (invariant 3). The refusal message
    /// names only the requested name; the profile's own details are never in
    /// it.
    pub(crate) fn allowed_profile(
        &self,
        allowlist: &[super::policy::ProfileSummary],
        name: &str,
    ) -> Result<(DatabaseProfile, ProfileIdentity), String> {
        if !allowlist.iter().any(|summary| summary.name == name) {
            return Err(format!("profile not available: {name}"));
        }
        let profile = self.runtime.named_profile(name).map_err(|_| {
            "profile not available: the allowlisted profile could not be resolved".to_owned()
        })?;
        let identity = profile_identity(name, profile, &self.runtime.cache_scope);
        Ok((profile.clone(), identity))
    }
}
