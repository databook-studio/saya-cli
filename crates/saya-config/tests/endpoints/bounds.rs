//! Endpoint strings are bounded at resolution — declared values and values
//! inherited from the plain `[ai]` block alike, each naming its own field.

use saya_config::{ConfigError, MAX_ENDPOINT_STRING_CHARS};

use super::resolve_with_user;

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
