use super::*;

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

/// `[ai] context_byte_budget` resolves from a config file and, when unset, keeps
/// the same default the agent loop used before the setting existed (
/// a user with no setting gets exactly what they get today). The default is the
/// crate's 256 KiB conversation budget, not a new number.
#[test]
fn ai_context_byte_budget_resolves_from_file_with_unchanged_default() {
    let config = ConfigFile::from_toml("[ai]\ncontext_byte_budget = 524288\n").unwrap();
    let resolved = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default()).with_user(config),
    )
    .unwrap();
    assert_eq!(resolved.ai.context_byte_budget, 524288);

    let defaults =
        saya_config::resolve(saya_config::ResolutionInput::new(ConnectionsFile::default()))
            .unwrap();
    assert_eq!(defaults.ai.context_byte_budget, 256 * 1024);
}

/// A budget of 0 would trim the conversation to nothing on every turn, so it is
/// rejected at resolve time with a typed error naming the field,
/// not silently clamped at the point of use. The validation matches the
/// `[memory]` range-check style already in this crate rather than a third form.
#[test]
fn ai_context_byte_budget_below_the_floor_is_a_typed_error() {
    let config = ConfigFile::from_toml("[ai]\ncontext_byte_budget = 0\n").unwrap();
    let error = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default()).with_user(config),
    )
    .unwrap_err();
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("context_byte_budget"),
        "error must name the field: {rendered}"
    );
    assert!(
        matches!(
            error,
            ConfigError::SettingBelowMinimum {
                field: "context_byte_budget",
                ..
            }
        ),
        "expected SettingBelowMinimum for a zero budget, got {rendered}"
    );
}

/// With no `[ai] retry_delays_ms` set, resolution falls back to the
/// three-entry [250, 500, 1000] ms schedule.
#[test]
fn ai_retry_delays_default_when_unset_matches_today() {
    let defaults =
        saya_config::resolve(saya_config::ResolutionInput::new(ConnectionsFile::default()))
            .unwrap();
    assert_eq!(defaults.ai.retry_delays_ms, vec![250, 500, 1000]);
}

#[test]
fn ai_retry_delays_resolves_from_file() {
    let config = ConfigFile::from_toml("[ai]\nretry_delays_ms = [100, 200, 300]\n").unwrap();
    let resolved = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default()).with_user(config),
    )
    .unwrap();
    assert_eq!(resolved.ai.retry_delays_ms, vec![100, 200, 300]);
}

/// An empty schedule is a valid "do not retry" choice: the provider makes one
/// attempt and sleeps nothing. Allowed rather than rejected because the intent
/// is unambiguous and the provider layer already handles an empty delay slice.
#[test]
fn ai_retry_delays_empty_list_means_do_not_retry() {
    let config = ConfigFile::from_toml("[ai]\nretry_delays_ms = []\n").unwrap();
    let resolved = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default()).with_user(config),
    )
    .unwrap();
    assert!(resolved.ai.retry_delays_ms.is_empty());
}

/// A config-supplied schedule is untrusted input: each entry repeats a full
/// failing request, so a runaway list turns one failure into many. Too long a
/// list is rejected at resolve time with a typed error naming the field,
/// matching the `[ai] context_byte_budget` floor-check style.
#[test]
fn ai_retry_delays_above_the_limit_is_a_typed_error() {
    let config = ConfigFile::from_toml(
        "[ai]\nretry_delays_ms = [100, 100, 100, 100, 100, 100, 100, 100, 100]\n",
    )
    .unwrap();
    let error = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default()).with_user(config),
    )
    .unwrap_err();
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("retry_delays_ms"),
        "error must name the field: {rendered}"
    );
    assert!(
        matches!(
            error,
            ConfigError::SettingAboveMaximum {
                field: "retry_delays_ms",
                ..
            }
        ),
        "expected SettingAboveMaximum for a too-long schedule, got {rendered}"
    );
}
