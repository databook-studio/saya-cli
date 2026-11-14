//! What a root `shell` run **outside** the sandbox may see of this process's
//! own environment.
//!
//! A sandboxed run needs none of this: its environment is the declared four,
//! built by the harness. A host run inherits this process's whole
//! environment, so the one thing standing between the child and saya's own
//! credentials is [`HostPolicy`] — and the policy is built here, where the
//! resolved config and the values saya already holds are in hand.
//!
//! Two lists, and neither is derived by the shell:
//!
//! - `remove_env` is every environment-variable *name* the resolved config
//!   references as a secret (provider API keys and every database profile's
//!   password, TLS material and service-account key), plus `SAYA_API_KEY`
//!   and every name already in this process's environment that starts with
//!   `SAYA_` — saya's own namespace, removed wholesale rather than by list.
//! - `secret_values` is the *value* saya already resolved for each of those
//!   names, so a child that echoes one is scrubbed on top of `redact()`. Only
//!   values already in hand are used: a file- or keyring-backed reference is
//!   never opened here, because building a shell must not read a secret.

use std::collections::BTreeSet;

use saya_harness::runner::HostPolicy;
use saya_types::{DatabaseProfile, SecretRef};

use crate::config::runtime::RuntimeConfig;

/// The names in this process's own namespace that are always removed: the
/// one credential name saya documents, and the prefix every other one shares.
const SAYA_API_KEY: &str = "SAYA_API_KEY";
const SAYA_PREFIX: &str = "SAYA_";

/// The host policy for a `sandbox: false` run: the names to take out of the
/// inherited environment, and the values to scrub from the capture.
pub(crate) fn host_policy(runtime: &RuntimeConfig) -> HostPolicy {
    let mut remove_env = configured_secret_names(runtime);
    remove_env.insert(SAYA_API_KEY.to_owned());
    for (name, _) in std::env::vars_os() {
        if let Some(name) = name.to_str()
            && name.starts_with(SAYA_PREFIX)
        {
            remove_env.insert(name.to_owned());
        }
    }
    // Only values already resolved at load time: a name whose reference is a
    // file or the keyring has no value here, and none is read to get one.
    let secret_values = remove_env
        .iter()
        .filter_map(|name| runtime.secret_values.get(name).cloned())
        .collect();
    HostPolicy {
        remove_env: remove_env.into_iter().collect(),
        secret_values,
    }
}

/// Every environment-variable name the resolved config names as a secret: the
/// AI endpoints' `api_key`s (including `orchestrator`, which is `[ai]`), and
/// every database profile's secret references, in the connections file and in
/// the selected profile.
fn configured_secret_names(runtime: &RuntimeConfig) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut take = |reference: Option<&SecretRef>| {
        if let Some(SecretRef::Env { env }) = reference {
            names.insert(env.clone());
        }
    };
    take(runtime.resolved.ai.api_key.as_ref());
    for endpoint in runtime.resolved.endpoints.values() {
        take(endpoint.api_key.as_ref());
    }
    for profile in runtime.connections.profiles.values() {
        profile_secret_names(profile, &mut names);
    }
    if let Some(profile) = runtime.resolved.profile.as_ref() {
        profile_secret_names(profile, &mut names);
    }
    names
}

/// The secret references one database profile carries, by variant: every
/// backend's credential, and none of the ones that hold no secret at all.
fn profile_secret_names(profile: &DatabaseProfile, names: &mut BTreeSet<String>) {
    let mut take = |reference: Option<&SecretRef>| {
        if let Some(SecretRef::Env { env }) = reference {
            names.insert(env.clone());
        }
    };
    match profile {
        DatabaseProfile::Postgres { password, .. } => take(password.as_ref()),
        DatabaseProfile::Mysql {
            ssl_ca, password, ..
        } => {
            take(ssl_ca.as_ref());
            take(password.as_ref());
        }
        DatabaseProfile::Snowflake {
            private_key,
            password,
            passphrase,
            ..
        } => {
            take(private_key.as_ref());
            take(password.as_ref());
            take(passphrase.as_ref());
        }
        DatabaseProfile::ClickHouse { password, .. } => take(password.as_ref()),
        DatabaseProfile::BigQuery {
            service_account_key,
            ..
        } => take(Some(service_account_key)),
        DatabaseProfile::Sqlite { .. } => {}
    }
}

#[cfg(test)]
#[path = "shell_secrets_tests.rs"]
mod tests;
