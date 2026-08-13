//! Profile resolution for the contracts adapter: maps a profile *name* the user
//! supplied to the opaque `ProfileIdentity` the operations layer needs, while
//! keeping the identity out of every message. An unknown name lists the
//! available names; the identity is never in the listing.

use super::EXIT_CONTRACT_ERROR;
use crate::config::runtime::RuntimeConfig;
use crate::profile_identity::profile_identity;
use saya_types::ProfileIdentity;

/// Resolves a profile name to its name + identity. `name` overrides the
/// active/default profile. An unknown name is a typed error listing available
/// names; the identity is never in the message. The returned name is the one a
/// render DTO carries; the identity stays in the request and out of every message.
pub(super) fn resolve_profile(
    runtime: &RuntimeConfig,
    name: Option<&str>,
) -> Result<(String, ProfileIdentity), (i32, String)> {
    let (name, profile) = match name {
        Some(name) => match runtime.named_profile(name) {
            Ok(profile) => (name.to_string(), profile.clone()),
            Err(_) => return Err(unknown_profile(runtime, name)),
        },
        None => match (&runtime.resolved.profile_name, &runtime.resolved.profile) {
            (Some(name), Some(profile)) => (name.clone(), profile.clone()),
            _ => {
                return Err((
                    EXIT_CONTRACT_ERROR,
                    "no connection profile was selected".into(),
                ));
            }
        },
    };
    let identity = profile_identity(&name, &profile, &runtime.cache_scope);
    Ok((name, identity))
}

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
    let message = format!(
        "unknown profile {name:?}; available profiles: {}",
        available.join(", ")
    );
    (EXIT_CONTRACT_ERROR, message)
}
