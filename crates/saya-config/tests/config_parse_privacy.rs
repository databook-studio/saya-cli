use saya_config::{ConfigError, ConfigFile, MAX_CONFIG_FILE_BYTES};

#[test]
fn malformed_values_do_not_echo_the_toml_source_line() {
    let secret = "c03b-parse-source-sentinel";
    let input = format!("[coding]\nmax_children = '{secret}'\n");
    let error = ConfigFile::from_toml(&input).expect_err("max_children is numeric");
    assert!(
        !error.to_string().contains(secret),
        "parse diagnostics must omit the raw source value: {error}"
    );
    assert!(error.to_string().contains("line 2, column"));
    assert!(error.to_string().contains("expected usize"));
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
