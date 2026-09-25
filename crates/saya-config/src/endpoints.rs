//! `[[ai.endpoints]]` resolution — the named AI endpoint pool a run's roles
//! bind to.
//!
//! Each endpoint is a delta over the plain `[ai]` block: a field the endpoint
//! declares wins, an unset field inherits `[ai]`'s resolved value. The map is
//! keyed by run-scoped name — the same shape the run contracts use for
//! `EndpointBindings` — and always contains `orchestrator`: when no endpoint
//! is declared with that name, it is synthesized from `[ai]`, so an existing
//! config keeps working untouched once runs bind roles to endpoints.

use std::collections::{BTreeMap, BTreeSet};

use saya_types::{MAX_ENDPOINT_BINDINGS, SecretRef, is_name_shaped};

use crate::{
    AiProvider, ConfigError,
    model::{AiFile, EndpointFile},
    resolve::ResolvedAi,
};

/// The role every run has, and the name the plain `[ai]` block resolves
/// under. When a run binds no endpoint to `orchestrator`, it is served by
/// `[ai]` itself — the fallback that keeps an existing config unchanged.
pub const ORCHESTRATOR_ROLE: &str = "orchestrator";

/// Maximum size of a provider model name or endpoint URL carried through
/// resolution. Both values are copied into provider settings and diagnostics,
/// so accepting arbitrary config-sized strings would make those paths
/// unbounded.
pub const MAX_ENDPOINT_STRING_CHARS: usize = 2_048;

/// A resolved endpoint: everything a provider connection needs, with the
/// defaults and inheritance already applied. `api_key` stays a *reference* —
/// values are resolved only at request time, never into this map.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedEndpoint {
    pub name: String,
    pub provider: AiProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<SecretRef>,
}

pub(crate) fn resolve_endpoints(
    file: &AiFile,
    ai: &ResolvedAi,
) -> Result<BTreeMap<String, ResolvedEndpoint>, ConfigError> {
    if file.endpoints.len() > MAX_ENDPOINT_BINDINGS {
        return Err(ConfigError::SettingAboveMaximum {
            field: "ai.endpoints",
            value: file.endpoints.len(),
            max: MAX_ENDPOINT_BINDINGS,
        });
    }
    require_endpoint_string("ai.model", ORCHESTRATOR_ROLE, &ai.model)?;
    if let Some(base_url) = ai.base_url.as_deref() {
        require_endpoint_string("ai.base_url", ORCHESTRATOR_ROLE, base_url)?;
    }
    if let Some(api_key) = ai.api_key.as_ref() {
        require_secret_reference("ai.api_key", ORCHESTRATOR_ROLE, api_key)?;
    }
    let mut map = BTreeMap::new();
    for endpoint in &file.endpoints {
        if !is_name_shaped(&endpoint.name) {
            return Err(ConfigError::InvalidEndpointName {
                field: "ai.endpoints",
                key: endpoint.name.clone(),
            });
        }
        if let Some(model) = endpoint.model.as_deref() {
            require_endpoint_string("ai.endpoints.model", &endpoint.name, model)?;
        }
        if let Some(base_url) = endpoint.base_url.as_deref() {
            require_endpoint_string("ai.endpoints.base_url", &endpoint.name, base_url)?;
        }
        if let Some(api_key) = endpoint.api_key.as_ref() {
            require_secret_reference("ai.endpoints.api_key", &endpoint.name, api_key)?;
        }
        let resolved = ResolvedEndpoint {
            name: endpoint.name.clone(),
            provider: endpoint.provider.unwrap_or(ai.provider),
            model: endpoint.model.clone().unwrap_or_else(|| ai.model.clone()),
            base_url: endpoint.base_url.clone().or_else(|| ai.base_url.clone()),
            api_key: endpoint.api_key.clone().or_else(|| ai.api_key.clone()),
        };
        if map.insert(endpoint.name.clone(), resolved).is_some() {
            return Err(ConfigError::DuplicateEndpointName(endpoint.name.clone()));
        }
    }
    map.entry(ORCHESTRATOR_ROLE.to_string())
        .or_insert_with(|| ResolvedEndpoint {
            name: ORCHESTRATOR_ROLE.to_string(),
            provider: ai.provider,
            model: ai.model.clone(),
            base_url: ai.base_url.clone(),
            api_key: ai.api_key.clone(),
        });
    Ok(map)
}

fn require_secret_reference(
    field: &'static str,
    endpoint: &str,
    reference: &SecretRef,
) -> Result<(), ConfigError> {
    let value = match reference {
        SecretRef::Env { env } => env,
        SecretRef::File { file } => file,
        SecretRef::Keyring { keyring } => keyring,
    };
    require_endpoint_string(field, endpoint, value)
}

fn require_endpoint_string(
    field: &'static str,
    endpoint: &str,
    value: &str,
) -> Result<(), ConfigError> {
    let length = value.chars().count();
    if length <= MAX_ENDPOINT_STRING_CHARS {
        Ok(())
    } else {
        Err(ConfigError::EndpointStringTooLong {
            field,
            endpoint: endpoint.to_owned(),
            value: length,
            max: MAX_ENDPOINT_STRING_CHARS,
        })
    }
}

/// Rejects a single configuration layer that declares two endpoints with the
/// same name, so a duplicate is a typed error rather than last-wins layering.
/// Called per layer before merging: across layers a same-name endpoint is an
/// override (the trust boundary decides what it may change), but within one
/// layer it is ambiguous authoring.
pub(crate) fn require_unique_endpoints(endpoints: &[EndpointFile]) -> Result<(), ConfigError> {
    let mut seen = BTreeSet::new();
    for endpoint in endpoints {
        if !seen.insert(&endpoint.name) {
            return Err(ConfigError::DuplicateEndpointName(endpoint.name.clone()));
        }
    }
    Ok(())
}
