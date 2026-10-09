use super::*;

#[test]
fn json_argv_is_checked_before_string_cloning() {
    let too_many = vec![serde_json::json!(""); MAX_ARG_COUNT + 1];
    assert!(matches!(
        validate_json_argv(&too_many),
        Err(RunnerError::ArgsNotTyped { .. })
    ));

    let too_long = vec![serde_json::json!("x".repeat(MAX_ARG_BYTES + 1))];
    assert!(matches!(
        validate_json_argv(&too_long),
        Err(RunnerError::ArgsNotTyped { .. })
    ));

    let item = "x".repeat(MAX_ARG_BYTES);
    let too_wide = (0..(MAX_ARGV_BYTES / MAX_ARG_BYTES + 1))
        .map(|_| serde_json::json!(item.clone()))
        .collect::<Vec<_>>();
    assert!(matches!(
        validate_json_argv(&too_wide),
        Err(RunnerError::ArgsNotTyped { .. })
    ));
}
