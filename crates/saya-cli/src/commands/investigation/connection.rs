//! Saving-profile resolution for `investigation save`: maps the
//! `--connection` name (or the resolved default profile) to the name, dialect,
//! and opaque identity the document and binding need — with the identity
//! never in any message.

use super::EXIT_INVESTIGATION_ERROR;
use crate::config::runtime::RuntimeConfig;
use crate::profile_identity::profile_identity;
use saya_types::{ProfileIdentity, SqlDialect};

/// Resolves the saving profile: `--connection` wins, else the resolved
/// default profile; none is a usage error naming the flag (invariant 1).
/// Returns the profile name, its dialect, and its opaque identity.
pub(super) fn resolve_connection(
    runtime: &RuntimeConfig,
    connection: Option<&str>,
) -> Result<(String, SqlDialect, ProfileIdentity), (i32, String)> {
    let (name, profile) = match connection {
        Some(name) => match runtime.named_profile(name) {
            Ok(profile) => (name.to_string(), profile.clone()),
            Err(_) => return Err(unknown_profile(runtime, name)),
        },
        None => match (&runtime.resolved.profile_name, &runtime.resolved.profile) {
            (Some(name), Some(profile)) => (name.clone(), profile.clone()),
            _ => {
                return Err((
                    EXIT_INVESTIGATION_ERROR,
                    "no connection: pass --connection <profile>".to_string(),
                ));
            }
        },
    };
    let identity = profile_identity(&name, &profile, &runtime.cache_scope);
    Ok((name, profile.dialect(), identity))
}

/// The available-profiles message, shaped like the contracts adapter's (the
/// identity never appears); duplicated locally because that helper is private
/// to the contracts module.
fn unknown_profile(runtime: &RuntimeConfig, name: &str) -> (i32, String) {
    let mut available: Vec<&str> = runtime
        .connections
        .profiles
        .keys()
        .map(String::as_str)
        .collect();
    if let Some(active) = runtime.resolved.profile_name.as_deref()
        && !available.contains(&active)
    {
        available.push(active);
    }
    available.sort();
    (
        EXIT_INVESTIGATION_ERROR,
        format!(
            "unknown profile {name:?}; available profiles: {}",
            available.join(", ")
        ),
    )
}
