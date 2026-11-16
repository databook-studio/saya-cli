//! The `ai` task's readiness checks: what provider and model are set,
//! whether the provider's key reference resolves, and what endpoint the
//! config names — facts only, never a connection attempt. Failing and
//! warning checks carry an edit hint naming the setting or file, never a
//! secret value.

use saya_types::SecretRef;

use saya_config::{AiProvider, MapSecretResolver, SecretResolver};

use super::super::runtime::RuntimeConfig;
use super::{CheckStatus, TaskReadiness, check, check_hint, ready, user_config_file};

pub(super) fn readiness(runtime: &RuntimeConfig) -> TaskReadiness {
    let ai = &runtime.resolved.ai;
    let provider = if ai.model.is_empty() {
        check_hint(
            "provider",
            CheckStatus::Fail,
            "no model is configured — set [ai] model",
            format!("Set [ai] model in {}, or pass --model.", user_config_file()),
        )
    } else {
        check(
            "provider",
            CheckStatus::Ok,
            format!("{}, model {}", ai.provider.as_str(), ai.model),
        )
    };
    ready(
        "ai",
        vec![
            provider,
            credentials_check(runtime),
            endpoint_check(runtime),
        ],
    )
}

fn credentials_check(runtime: &RuntimeConfig) -> super::TaskCheck {
    let ai = &runtime.resolved.ai;
    let cloud = matches!(
        ai.provider,
        AiProvider::Openai | AiProvider::Anthropic | AiProvider::Gemini
    );
    let Some(reference) = ai.api_key.as_ref() else {
        return if cloud {
            check_hint(
                "credentials",
                CheckStatus::Fail,
                format!(
                    "no api_key reference configured — requests to {} will be unauthenticated",
                    ai.provider.as_str()
                ),
                format!(
                    "Set [ai] api_key = {{ env = \"NAME\" }} in {} (or export SAYA_AI_API_KEY).",
                    user_config_file()
                ),
            )
        } else {
            check(
                "credentials",
                CheckStatus::Ok,
                format!("no credentials required for {}", ai.provider.as_str()),
            )
        };
    };
    let resolver = MapSecretResolver::new(runtime.secret_values.clone());
    let (status, detail) = match resolver.resolve(reference) {
        Ok(_) => (
            CheckStatus::Ok,
            format!("api key reference {} resolves", reference.redacted_label()),
        ),
        Err(error) => (
            CheckStatus::Fail,
            format!("{} does not resolve ({error})", reference.redacted_label()),
        ),
    };
    match (status, unresolved_export_hint(reference)) {
        (CheckStatus::Ok, _) | (_, None) => check("credentials", status, detail),
        (status, Some(hint)) => check_hint("credentials", status, detail, hint),
    }
}

/// The edit hint for an unresolved reference: an env reference names the
/// variable to export (and the redacted label); a file reference gets no
/// hint — "export" would be wrong advice and the label alone already says
/// what failed.
fn unresolved_export_hint(reference: &SecretRef) -> Option<String> {
    match reference {
        SecretRef::Env { env } => Some(format!(
            "Export {env} in your shell or --env-file; the reference is {}.",
            reference.redacted_label()
        )),
        SecretRef::File { .. } | SecretRef::Keyring { .. } => None,
    }
}

fn endpoint_check(runtime: &RuntimeConfig) -> super::TaskCheck {
    let ai = &runtime.resolved.ai;
    match super::super::doctor::provider_endpoint(ai.provider, ai.base_url.as_deref()) {
        Some((host, port)) => check(
            "endpoint",
            CheckStatus::Ok,
            format!("configured endpoint: {host}:{port}"),
        ),
        None if ai.base_url.is_some() => check_hint(
            "endpoint",
            CheckStatus::Warn,
            "configured endpoint: none — the configured base_url could not be parsed",
            format!("Set [ai] base_url in {}.", user_config_file()),
        ),
        None => check_hint(
            "endpoint",
            CheckStatus::Warn,
            "configured endpoint: none",
            format!("Set [ai] base_url in {}.", user_config_file()),
        ),
    }
}
