use super::*;

/// G2 property 3 — `[host_commands]` resolves `pass_env` and
/// `timeout_seconds` from the user layer: shaping keys work, and neither
/// gates the lane. Moved from `host_commands_resolve_from_the_user_layer`
/// (reason: the `enable` key the old test pinned was deleted; shaping stays).
#[test]
fn user_layer_pass_env_and_timeout_shape_the_lane() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml(
                "[host_commands]\npass_env = ['CI_TOKEN']\ntimeout_seconds = 42\n",
            )
            .expect("fixture parses"),
        ),
    )
    .expect("resolution succeeds");
    assert_eq!(
        resolved.host_commands.pass_env,
        vec!["CI_TOKEN".to_owned()],
        "pass_env resolves verbatim"
    );
    assert_eq!(
        resolved.host_commands.timeout_seconds, 42,
        "timeout_seconds resolves"
    );
}

/// G2 — a stale `enable = true` is a typed parse error: unknown-field deny
/// refuses it rather than silently ignoring it. Extra pin the list's
/// property 3/4 pair implies (reason: the design names this behaviour, and
/// no other test pins it).
#[test]
fn a_stale_enable_key_is_a_typed_parse_error() {
    let error = match ConfigFile::from_toml("[host_commands]\nenable = true\n") {
        Err(error) => error.to_string(),
        Ok(_) => panic!("a stale `enable = true` must refuse at parse"),
    };
    assert!(
        error.contains("enable") && error.contains("pass_env"),
        "the typed error names the stale key against the known keys: {error}"
    );
}

/// G2 — `[host_commands]` defaults resolve to an empty `pass_env` and the
/// executor's ceiling: shaping defaults, no lane gate. Moved from
/// `host_commands_default_to_lane_off` (reason: the `enabled` gate the old
/// test pinned was deleted; shaping defaults stay).
#[test]
fn host_commands_default_to_shaping_defaults() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    assert!(
        resolved.host_commands.pass_env.is_empty(),
        "no pass_env by default"
    );
    assert_eq!(
        resolved.host_commands.timeout_seconds, 600,
        "the default ceiling is the executor's own"
    );
}
