//! `[[ai.endpoints]]` — the named AI endpoint pool a run's roles bind to.
//!
//! Resolution: each endpoint is a delta over the plain `[ai]` block (unset
//! fields inherit it), the resolved map is keyed by run-scoped name with
//! duplicates rejected as a typed error, and `orchestrator` always resolves —
//! to the plain `[ai]` block when nothing is declared with that name, so an
//! existing config keeps working untouched.

use saya_config::{
    AiProvider, ConfigError, ConfigFile, ConnectionsFile, MAX_ENDPOINT_STRING_CHARS,
    ResolutionInput, resolve,
};

fn resolve_with_user(toml: &str) -> Result<saya_config::ResolvedConfig, ConfigError> {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
    )
}

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

#[test]
fn duplicate_endpoint_names_are_rejected_not_last_wins() {
    let error = resolve_with_user(
        "[[ai.endpoints]]\nname = 'planner'\nmodel = 'a'\n\
         [[ai.endpoints]]\nname = 'planner'\nmodel = 'b'\n",
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::DuplicateEndpointName(_)),
        "duplicates must be a typed error, not last-wins: {error:?}"
    );
    assert!(error.to_string().contains("planner"));
}

#[test]
fn a_malformed_endpoint_name_is_rejected() {
    let error = resolve_with_user("[[ai.endpoints]]\nname = 'bad name'\n").unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidEndpointName {
            field: "ai.endpoints",
            ..
        }
    ));
    assert!(error.to_string().contains("bad name"));
}

#[test]
fn the_endpoint_pool_is_bounded() {
    let mut toml = String::new();
    for index in 0..9 {
        toml.push_str(&format!("[[ai.endpoints]]\nname = 'ep{index}'\n"));
    }
    let error = resolve_with_user(&toml).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::SettingAboveMaximum {
            field: "ai.endpoints",
            ..
        }
    ));
}

#[test]
fn unknown_keys_inside_an_endpoint_are_rejected() {
    let error = ConfigFile::from_toml("[[ai.endpoints]]\nname = 'p'\nbaz = 'x'\n").unwrap_err();
    assert!(
        error.to_string().contains("baz"),
        "deny_unknown_fields must name the key: {error}"
    );
}

#[test]
fn an_endpoint_without_a_name_is_rejected_at_parse() {
    let error = ConfigFile::from_toml("[[ai.endpoints]]\nmodel = 'm'\n").unwrap_err();
    assert!(
        error.to_string().contains("name"),
        "the name is the map key; its absence must fail loudly: {error}"
    );
}

#[test]
fn endpoint_diagnostics_never_carry_a_secret_value() {
    const SECRET_VALUE: &str = "super-secret-value-424242";
    let user = ConfigFile::from_toml(
        "[ai]\napi_key = { env = 'SAYA_ENDPOINT_SECRET' }\n\
         [[ai.endpoints]]\nname = 'planner'\n\
         base_url = 'https://gateway.internal/v1?token=abc'\napi_key = { file = 'secrets.txt' }\n",
    )
    .unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user.clone())
            .with_process_env([("SAYA_ENDPOINT_SECRET", SECRET_VALUE)]),
    )
    .unwrap();
    let views = [
        serde_json::to_string(&user.redacted_diagnostics()).unwrap(),
        serde_json::to_string(&resolved.redacted_diagnostics()).unwrap(),
    ];
    for view in &views {
        assert!(
            !view.contains(SECRET_VALUE),
            "a resolved secret value leaked into a diagnostics view: {view}"
        );
        assert!(
            view.contains("env:SAYA_ENDPOINT_SECRET"),
            "the reference label is the view's contract: {view}"
        );
        assert!(
            view.contains("file:[redacted path]"),
            "file references redact the path: {view}"
        );
        assert!(
            view.contains("https://gateway.internal/v1?[redacted]"),
            "base_url redacts its query: {view}"
        );
    }
    assert!(
        views[0].contains("\"planner\""),
        "the file mirror names the declared endpoint"
    );
    assert!(
        views[1].contains("\"orchestrator\""),
        "the resolved mirror carries the orchestrator fallback"
    );
}

#[test]
fn endpoint_model_is_bounded_at_resolution() {
    let model = "m".repeat(MAX_ENDPOINT_STRING_CHARS + 1);
    let error = resolve_with_user(&format!(
        "[[ai.endpoints]]\nname = 'planner'\nmodel = '{model}'\n"
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::EndpointStringTooLong {
            field: "ai.endpoints.model",
            ..
        }
    ));
}

#[test]
fn endpoint_base_url_is_bounded_at_resolution() {
    let base_url = format!("https://{}", "a".repeat(MAX_ENDPOINT_STRING_CHARS));
    let error = resolve_with_user(&format!(
        "[[ai.endpoints]]\nname = 'planner'\nbase_url = '{base_url}'\n"
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::EndpointStringTooLong {
            field: "ai.endpoints.base_url",
            ..
        }
    ));
}

#[test]
fn endpoint_secret_reference_is_bounded_at_resolution() {
    let env = "E".repeat(MAX_ENDPOINT_STRING_CHARS + 1);
    let error = resolve_with_user(&format!(
        "[[ai.endpoints]]\nname = 'planner'\napi_key = {{ env = '{env}' }}\n"
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::EndpointStringTooLong {
            field: "ai.endpoints.api_key",
            ..
        }
    ));
}

#[test]
fn inherited_endpoint_secret_reference_is_bounded_at_resolution() {
    let env = "E".repeat(MAX_ENDPOINT_STRING_CHARS + 1);
    let error = resolve_with_user(&format!(
        "[ai]\napi_key = {{ env = '{env}' }}\n[[ai.endpoints]]\nname = 'planner'\n"
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::EndpointStringTooLong {
            field: "ai.api_key",
            ..
        }
    ));
}

#[test]
fn inherited_endpoint_strings_are_bounded_without_losing_inheritance() {
    let model = "m".repeat(MAX_ENDPOINT_STRING_CHARS + 1);
    let error = resolve_with_user(&format!(
        "[ai]\nmodel = '{model}'\n[[ai.endpoints]]\nname = 'planner'\n"
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::EndpointStringTooLong {
            field: "ai.model",
            ..
        }
    ));
}

#[test]
fn inherited_endpoint_diagnostics_remain_redacted() {
    let user = ConfigFile::from_toml(
        "[ai]\nmodel = 'gateway-model'\nbase_url = 'https://user:pass@gateway.test/v1?token=secret'\n\
         api_key = { env = 'SAYA_KEY' }\n[[ai.endpoints]]\nname = 'planner'\n",
    )
    .unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(user)).unwrap();
    let planner = &resolved.redacted_diagnostics().endpoints["planner"];
    assert_eq!(planner.model.as_deref(), Some("gateway-model"));
    assert_eq!(
        planner.base_url.as_deref(),
        Some("https://gateway.test/v1?[redacted]")
    );
    assert_eq!(planner.api_key_reference.as_deref(), Some("env:SAYA_KEY"));
}
