//! The `ai` task's readiness checks: what provider and model are set,
//! whether the provider's key reference resolves, and what endpoint the
//! config names — facts only, never a connection attempt.

use saya_config::{AiProvider, MapSecretResolver, SecretResolver};

use super::super::runtime::RuntimeConfig;
use super::{CheckStatus, TaskReadiness, check, ready};

pub(super) fn readiness(runtime: &RuntimeConfig) -> TaskReadiness {
    let ai = &runtime.resolved.ai;
    let provider = if ai.model.is_empty() {
        check(
            "provider",
            CheckStatus::Fail,
            "no model is configured — set [ai] model",
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
            check(
                "credentials",
                CheckStatus::Fail,
                format!(
                    "no api_key reference configured — requests to {} will be unauthenticated",
                    ai.provider.as_str()
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
    check("credentials", status, detail)
}

fn endpoint_check(runtime: &RuntimeConfig) -> super::TaskCheck {
    let ai = &runtime.resolved.ai;
    match super::super::doctor::provider_endpoint(ai.provider, ai.base_url.as_deref()) {
        Some((host, port)) => check(
            "endpoint",
            CheckStatus::Ok,
            format!("configured endpoint: {host}:{port}"),
        ),
        None if ai.base_url.is_some() => check(
            "endpoint",
            CheckStatus::Warn,
            "configured endpoint: none — the configured base_url could not be parsed",
        ),
        None => check("endpoint", CheckStatus::Warn, "configured endpoint: none"),
    }
}
