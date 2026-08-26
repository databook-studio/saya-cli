use saya_config::{
    AiProvider, ColorChoice, ConfigFile, ConnectionsFile, OutputFormat, ResolutionInput, resolve,
};
use saya_types::SecretRef;

#[test]
fn process_environment_overrides_env_file_for_all_mapped_runtime_settings() {
    let config = ConfigFile::from_toml(
        "[ai]\napi_key = { file = '/private/key' }\n\
         [run]\nmax_rows = 55\n\
         [output]\ncolor = 'always'\n",
    )
    .unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(config)
            .with_env_file([
                ("SAYA_AI_PROVIDER", "anthropic"),
                ("SAYA_AI_BASE_URL", "https://env-file.invalid"),
                ("SAYA_READ_ONLY", "false"),
                ("SAYA_MAX_ITERATIONS", "4"),
                ("SAYA_QUERY_TIMEOUT_SECONDS", "20"),
                ("SAYA_OUTPUT_FORMAT", "json"),
            ])
            .with_process_env([
                ("SAYA_AI_PROVIDER", "openai"),
                ("SAYA_AI_BASE_URL", "https://process.invalid"),
                ("SAYA_READ_ONLY", "true"),
                ("SAYA_MAX_ITERATIONS", "5"),
                ("SAYA_QUERY_TIMEOUT_SECONDS", "30"),
                ("SAYA_OUTPUT_FORMAT", "ndjson"),
            ]),
    )
    .unwrap();

    assert_eq!(resolved.ai.provider, AiProvider::Openai);
    assert_eq!(
        resolved.ai.base_url.as_deref(),
        Some("https://process.invalid")
    );
    assert_eq!(
        resolved.ai.api_key,
        Some(SecretRef::File {
            file: "/private/key".into()
        })
    );
    assert!(resolved.read_only);
    assert_eq!(resolved.max_iterations, 5);
    assert_eq!(resolved.query_timeout_seconds, 30);
    assert_eq!(resolved.output_format, OutputFormat::Ndjson);
    assert_eq!(resolved.output_color, ColorChoice::Always);
}

#[test]
fn provider_environment_names_resolve_to_runtime_secret_references() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_PROVIDER", "openai_compatible"),
            ("SAYA_MODEL", "test-model"),
            ("SAYA_PROVIDER_BASE_URL", "http://localhost:8080/v1"),
            ("SAYA_API_KEY", "sentinel"),
        ]),
    )
    .unwrap();
    assert_eq!(resolved.ai.provider, AiProvider::OpenaiCompatible);
    assert_eq!(resolved.ai.model, "test-model");
    assert_eq!(
        resolved.ai.base_url.as_deref(),
        Some("http://localhost:8080/v1")
    );
    assert_eq!(
        resolved.ai.api_key,
        Some(SecretRef::Env {
            env: "SAYA_API_KEY".into()
        })
    );
}

#[test]
fn resolved_diagnostics_redact_file_secret_paths() {
    let config = ConfigFile::from_toml("[ai]\napi_key = { file = '/private/key' }\n").unwrap();
    let rendered = serde_json::to_string(
        &resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .unwrap()
            .redacted_diagnostics(),
    )
    .unwrap();
    assert!(!rendered.contains("/private/key"));
    assert!(rendered.contains("file:[redacted path]"));
}

#[test]
fn resolved_diagnostics_masks_base_url_userinfo_and_query_string() {
    let config = ConfigFile::from_toml(
        "[ai]\nbase_url = 'https://user:password@example.test/v1?api_key=secret&mode=fast'\n",
    )
    .unwrap();
    let rendered = serde_json::to_string(
        &resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .unwrap()
            .redacted_diagnostics(),
    )
    .unwrap();
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("api_key=secret"));
    assert!(rendered.contains("https://example.test/v1"));
    assert!(rendered.contains("[redacted]"));
}

#[test]
fn ai_request_budgets_resolve_from_file_with_defaults() {
    let config = ConfigFile::from_toml(
        "[ai]\ntimeout_seconds = 120\nidle_timeout_seconds = 45\nmax_output_tokens = 2048\ntemperature = 0.7\n",
    )
    .unwrap();
    let resolved = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default()).with_user(config),
    )
    .unwrap();
    assert_eq!(resolved.ai.timeout_seconds, 120);
    assert_eq!(resolved.ai.idle_timeout_seconds, 45);
    assert_eq!(resolved.ai.max_output_tokens, 2048);
    assert_eq!(resolved.ai.temperature, 0.7);

    let defaults =
        saya_config::resolve(saya_config::ResolutionInput::new(ConnectionsFile::default()))
            .unwrap();
    assert_eq!(defaults.ai.timeout_seconds, 60);
    assert_eq!(defaults.ai.idle_timeout_seconds, 90);
    assert_eq!(defaults.ai.max_output_tokens, 4096);
}
