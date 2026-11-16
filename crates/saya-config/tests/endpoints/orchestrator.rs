//! The resolved pool: `orchestrator` always resolves (from the plain `[ai]`
//! block when nothing declares it), unset endpoint fields inherit that
//! block, and a declared `orchestrator` entry replaces the fallback.

use saya_config::{AiProvider, ConnectionsFile, ResolutionInput, resolve};

use super::resolve_with_user;

#[test]
fn an_absent_endpoints_section_resolves_an_orchestrator_from_the_ai_block() {
    let resolved =
        resolve_with_user("[ai]\nmodel = 'm'\napi_key = { env = 'SAYA_KEY' }\n").unwrap();
    assert_eq!(resolved.endpoints.len(), 1);
    let orchestrator = &resolved.endpoints["orchestrator"];
    assert_eq!(orchestrator.model, "m");
    assert_eq!(
        orchestrator
            .api_key
            .as_ref()
            .map(|key| key.redacted_label()),
        Some("env:SAYA_KEY".to_string())
    );
}

#[test]
fn with_nothing_declared_the_orchestrator_still_resolves() {
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    let orchestrator = resolved
        .endpoints
        .get("orchestrator")
        .expect("fallback exists");
    assert_eq!(orchestrator.provider, AiProvider::Ollama);
    assert_eq!(orchestrator.base_url, None);
    assert_eq!(orchestrator.api_key, None);
}

#[test]
fn a_declared_endpoint_inherits_unset_fields_from_the_ai_block() {
    let resolved = resolve_with_user(
        "[ai]\nmodel = 'm'\napi_key = { env = 'SAYA_KEY' }\n\
         [[ai.endpoints]]\nname = 'planner'\nbase_url = 'https://gateway.internal/v1'\n",
    )
    .unwrap();
    let planner = &resolved.endpoints["planner"];
    assert_eq!(
        planner.base_url.as_deref(),
        Some("https://gateway.internal/v1")
    );
    assert_eq!(planner.model, "m", "model inherits [ai]");
    assert_eq!(
        planner.api_key.as_ref().map(|key| key.redacted_label()),
        Some("env:SAYA_KEY".to_string()),
        "api_key inherits [ai]"
    );
    assert_eq!(planner.provider, resolved.ai.provider);
}

#[test]
fn an_endpoint_named_orchestrator_replaces_the_fallback() {
    let resolved = resolve_with_user(
        "[ai]\nmodel = 'm'\n[[ai.endpoints]]\nname = 'orchestrator'\n\
         base_url = 'https://special/v1'\n",
    )
    .unwrap();
    assert_eq!(resolved.endpoints.len(), 1);
    assert_eq!(
        resolved.endpoints["orchestrator"].base_url.as_deref(),
        Some("https://special/v1")
    );
}
