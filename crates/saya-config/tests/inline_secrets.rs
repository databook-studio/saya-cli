//! Inline secrets must be rejected with a message that names the field and
//! the fix — not serde's raw "untagged enum" noise.

use saya_config::{ConfigFile, ConnectionsFile};

#[test]
fn inline_api_key_error_names_the_field_and_the_fix() {
    let error = ConfigFile::from_toml("[ai]\napi_key = \"sk-demo-123\"\n").unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("api_key") && text.contains("{ env =") && text.contains("not allowed"),
        "error must be actionable: {text}"
    );
    assert!(
        !text.contains("untagged enum"),
        "raw serde noise must not leak: {text}"
    );
}

#[test]
fn inline_profile_password_error_names_the_profile() {
    let toml = r#"
[profiles.pg]
type = "postgresql"
host = "h"
database = "d"
user = "u"
password = "hunter2"
"#;
    let error = ConnectionsFile::from_toml(toml).unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("password") && text.contains("pg") && text.contains("{ env ="),
        "error must name field, profile, and fix: {text}"
    );
    assert!(!text.contains("untagged enum"));
}

#[test]
fn reference_form_still_parses() {
    let config = ConfigFile::from_toml("[ai]\napi_key = { env = \"SAYA_API_KEY\" }\n").unwrap();
    assert!(config.ai.api_key.is_some());
}
