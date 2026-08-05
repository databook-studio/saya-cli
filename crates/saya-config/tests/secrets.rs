use saya_config::{ConfigFile, SecretRef, parse_explicit_env_file};

#[test]
fn secret_references_parse_without_inline_secret_support() {
    let config = ConfigFile::from_toml("[ai]\napi_key = { env = 'OPENAI_API_KEY' }\n").unwrap();
    assert_eq!(
        config.ai.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".into()
        })
    );
    assert!(ConfigFile::from_toml("[ai]\napi_key = 'not-allowed'\n").is_err());
}

#[test]
fn env_files_are_parsed_only_when_explicitly_supplied() {
    let values = parse_explicit_env_file("# explicit\nSAYA_AI_MODEL=local-model\n").unwrap();
    assert_eq!(values.get("SAYA_AI_MODEL"), Some(&"local-model".into()));
}

#[test]
fn diagnostics_reveal_references_but_not_resolved_values() {
    let config = ConfigFile::from_toml("[ai]\napi_key = { env = 'OPENAI_API_KEY' }\n").unwrap();
    let diagnostic = config.redacted_diagnostics();
    let rendered = serde_json::to_string(&diagnostic).unwrap();
    assert!(rendered.contains("env:OPENAI_API_KEY"));
    assert!(!rendered.contains("not-allowed"));
}
