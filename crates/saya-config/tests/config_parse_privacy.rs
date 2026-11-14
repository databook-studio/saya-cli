use saya_config::{ConfigError, ConfigFile, MAX_CONFIG_FILE_BYTES};

#[test]
fn malformed_values_do_not_echo_the_toml_source_line() {
    let secret = "c03b-parse-source-sentinel";
    let input = format!("[run]\nmax_rows = '{secret}'\n");
    let error = ConfigFile::from_toml(&input).expect_err("the row cap is numeric");
    assert!(
        !error.to_string().contains(secret),
        "parse diagnostics must omit the raw source value: {error}"
    );
    assert!(error.to_string().contains("line 2, column"));
    assert!(error.to_string().contains("expected usize"));
}

/// The `[agents]` shape refusals are read from the text, so they must name the
/// key and nothing else: a value in that table can be a model or a credential
/// reference and never belongs in a diagnostic.
#[test]
fn an_agents_shape_refusal_does_not_echo_the_source_value() {
    let secret = "c03b-agents-source-sentinel";
    let error = ConfigFile::from_toml(&format!("[agents]\nreviewer = '{secret}'\n"))
        .expect_err("a helper type is a table");
    assert!(
        error.to_string().contains("reviewer") && !error.to_string().contains(secret),
        "the refusal names the key and not the value: {error}"
    );
}

#[test]
fn config_toml_is_bounded_before_parsing() {
    let input = format!("#{}", "x".repeat(MAX_CONFIG_FILE_BYTES));
    let error = ConfigFile::from_toml(&input).expect_err("oversized config is refused");
    assert!(matches!(
        error,
        ConfigError::ConfigFileTooLarge {
            bytes,
            max_bytes: MAX_CONFIG_FILE_BYTES
        } if bytes == MAX_CONFIG_FILE_BYTES + 1
    ));
}
