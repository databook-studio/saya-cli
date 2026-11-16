//! `[limits]` — one table for every run ceiling a surface enforces.
//!
//! This file covers the answering surfaces: `[limits.session]` and the
//! optional `[limits.ask]`, whose every unset key falls back to the session's.
//! Both are protected, so an untrusted project layer can state neither.
//!
//! It also covers `[limits.run]`, which holds the four `[jobs]` ceilings and
//! is deliberately *not* protected — `[jobs]` is project-settable and a
//! run ceiling is a cost control, not a security boundary.

use saya_config::{
    BudgetLimit, CliOverrides, ConfigError, ConfigFile, ConnectionsFile, ResolutionInput,
    ResolvedAnsweringLimits, ResolvedConfig, ResolvedLimits, SettingSource, resolve,
};

fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

fn resolve_user(toml: &str) -> ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("user fixture parses")),
    )
    .expect("resolution succeeds")
}

fn resolve_layers(user: &str, project: &str, trust_project: bool) -> ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(user).expect("user fixture parses"))
            .with_project(ConfigFile::from_toml(project).expect("project fixture parses"))
            .with_cli(CliOverrides {
                trust_project_config: trust_project,
                ..Default::default()
            }),
    )
    .expect("resolution succeeds")
}

/// The defaults are the four numbers the single `[ai]` budget carried, so a
/// config with no `[limits]` at all resolves exactly what it did before.
#[test]
fn limits_default_to_todays_answer_budgets() {
    let limits = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("default config resolves")
        .limits;
    assert_eq!(limits, ResolvedLimits::default());
    assert_eq!(
        limits.session,
        ResolvedAnsweringLimits {
            requests: finite(24),
            tool_calls: finite(64),
            elapsed_seconds: finite(300),
            known_tokens: finite(250_000),
        }
    );
    assert_eq!(
        limits.ask, limits.session,
        "an unstated ask table takes every session value"
    );
}

#[test]
fn limits_session_states_every_dimension() {
    let limits = resolve_user(
        "[limits.session]\nrequests = 8\ntool_calls = 12\n\
         elapsed_seconds = 45\nknown_tokens = 9000\n",
    )
    .limits;
    assert_eq!(
        limits.session,
        ResolvedAnsweringLimits {
            requests: finite(8),
            tool_calls: finite(12),
            elapsed_seconds: finite(45),
            known_tokens: finite(9000),
        }
    );
}

#[test]
fn a_dimension_may_be_unlimited() {
    let limits = resolve_user("[limits.session]\nknown_tokens = 'unlimited'\n").limits;
    assert_eq!(limits.session.known_tokens, BudgetLimit::Unlimited);
    assert_eq!(
        limits.session.requests,
        finite(24),
        "the other keys keep defaults"
    );
    assert_eq!(
        limits.ask.known_tokens,
        BudgetLimit::Unlimited,
        "ask falls back per key to the session's"
    );
}

/// `[limits.ask]` overrides exactly the keys it states and nothing else.
#[test]
fn limits_ask_overrides_only_the_keys_it_states() {
    let limits = resolve_user(
        "[limits.session]\nrequests = 8\nelapsed_seconds = 45\n\
         [limits.ask]\nelapsed_seconds = 'unlimited'\n",
    )
    .limits;
    assert_eq!(limits.ask.elapsed_seconds, BudgetLimit::Unlimited);
    assert_eq!(
        limits.ask.requests,
        finite(8),
        "an unstated ask key falls back to the session's"
    );
    assert_eq!(
        limits.session.elapsed_seconds,
        finite(45),
        "the session keeps its own value"
    );
}

/// Every key names itself in the error, so the fix is never in doubt.
#[test]
fn a_dimension_that_is_not_a_positive_integer_or_unlimited_is_refused() {
    for (value, key) in [
        ("0", "limits.session.requests"),
        ("-1", "limits.session.tool_calls"),
        ("'not-unlimited'", "limits.session.elapsed_seconds"),
        ("'top-secret-token'", "limits.session.known_tokens"),
        ("''", "limits.session.known_tokens"),
        ("'18446744073709551616'", "limits.ask.requests"),
        ("'unlimited '", "limits.ask.tool_calls"),
        ("1.5", "limits.ask.elapsed_seconds"),
        ("true", "limits.ask.known_tokens"),
        ("[]", "limits.ask.requests"),
    ] {
        let (section, name) = key.rsplit_once('.').expect("key is dotted");
        let config = ConfigFile::from_toml(&format!("[{section}]\n{name} = {value}\n"))
            .expect("the raw value parses as TOML");
        let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .expect_err("an invalid dimension is refused");
        assert!(
            matches!(&error, ConfigError::InvalidLimit { key: field } if *field == key),
            "{key}: {error}"
        );
        assert!(!error.to_string().contains(value), "{key}: {error}");
    }
}

#[test]
fn the_largest_toml_integer_is_accepted_and_a_bigger_one_is_not_toml() {
    assert!(matches!(
        ConfigFile::from_toml("[limits.session]\nrequests = 9223372036854775808\n"),
        Err(ConfigError::Parse(_))
    ));
    let limits = resolve_user("[limits.session]\nrequests = 9223372036854775807\n").limits;
    assert_eq!(limits.session.requests, finite(i64::MAX as u64));
}

/// The four `[ai] investigation_*` keys are refused, and each says where it went.
#[test]
fn the_ai_investigation_keys_name_their_limits_replacement() {
    for (key, replacement) in [
        (
            "investigation_max_logical_answering_requests",
            "limits.session.requests",
        ),
        (
            "investigation_max_requested_tool_calls",
            "limits.session.tool_calls",
        ),
        (
            "investigation_max_elapsed_seconds",
            "limits.session.elapsed_seconds",
        ),
        (
            "investigation_max_known_reported_tokens",
            "limits.session.known_tokens",
        ),
    ] {
        let error = ConfigFile::from_toml(&format!("[ai]\n{key} = 5\n"))
            .expect_err("a retired key is refused");
        let message = error.to_string();
        assert!(
            message.contains(&format!("was moved: write {replacement} instead")),
            "{key}: {message}"
        );
    }
}

#[test]
fn a_project_layer_cannot_raise_the_session_limits() {
    let resolved = resolve_layers(
        "[limits.session]\nrequests = 5\n",
        "[limits.session]\nrequests = 'unlimited'\n",
        false,
    );
    assert_eq!(resolved.limits.session.requests, finite(5));
    assert_eq!(
        resolved.ignored_project_overrides,
        vec!["limits.session".to_owned()]
    );
    assert_eq!(
        resolved
            .provenance
            .get("limits.session.requests")
            .map(|origin| &origin.source),
        Some(&SettingSource::UserFile),
        "the ignored name rolls the per-key records back to the trusted layer"
    );
}

#[test]
fn a_project_layer_cannot_state_the_ask_limits_either() {
    let resolved = resolve_layers("", "[limits.ask]\nelapsed_seconds = 'unlimited'\n", false);
    assert_eq!(resolved.limits.ask.elapsed_seconds, finite(300));
    assert_eq!(
        resolved.ignored_project_overrides,
        vec!["limits.ask".to_owned()]
    );
}

#[test]
fn explicit_trust_lets_the_project_layer_state_limits() {
    let resolved = resolve_layers(
        "",
        "[limits.session]\nrequests = 'unlimited'\n[limits.ask]\nrequests = 3\n",
        true,
    );
    assert_eq!(resolved.limits.session.requests, BudgetLimit::Unlimited);
    assert_eq!(resolved.limits.ask.requests, finite(3));
    assert!(resolved.ignored_project_overrides.is_empty());
}

// --- `[limits.run]` ---

/// A run ceiling that nobody declares is unlimited. `[limits.run]` resolves
/// every unset dimension to "no ceiling", matching `[jobs]`.
#[test]
fn limits_run_defaults_to_unlimited() {
    let limits = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("default config resolves")
        .limits;
    assert_eq!(limits.run, saya_config::ResolvedRunLimits::default());
    assert_eq!(limits.run.turns, BudgetLimit::Unlimited);
    assert_eq!(limits.run.tool_calls, BudgetLimit::Unlimited);
    assert_eq!(limits.run.wall_clock_seconds, BudgetLimit::Unlimited);
    assert!(limits.run.tokens_per_endpoint.is_empty());
}

#[test]
fn limits_run_states_every_dimension() {
    let limits = resolve_user(
        "[limits.run]\nturns = 40\ntool_calls = 25\nwall_clock_seconds = 1800\n\
         [limits.run.tokens_per_endpoint]\n\"local-ollama\" = 200_000\n",
    )
    .limits;
    assert_eq!(limits.run.turns, finite(40));
    assert_eq!(limits.run.tool_calls, finite(25));
    assert_eq!(limits.run.wall_clock_seconds, finite(1800));
    assert_eq!(
        limits.run.tokens_per_endpoint,
        std::collections::BTreeMap::from([("local-ollama".into(), 200_000)])
    );
}

/// `"unlimited"` is accepted here too, as a synonym for leaving the key out:
/// a config that states it reads as a deliberate choice rather than an
/// omission.
#[test]
fn a_run_dimension_may_be_unlimited() {
    let limits = resolve_user("[limits.run]\nturns = 'unlimited'\n").limits;
    assert_eq!(limits.run.turns, BudgetLimit::Unlimited);
}

/// Every run key names itself in the error, so the fix is never in doubt.
#[test]
fn a_run_dimension_that_is_not_a_positive_integer_or_unlimited_is_refused() {
    for (value, key) in [
        ("0", "limits.run.turns"),
        ("-1", "limits.run.tool_calls"),
        ("'not-unlimited'", "limits.run.wall_clock_seconds"),
        ("1.5", "limits.run.turns"),
        ("true", "limits.run.tool_calls"),
    ] {
        let (section, name) = key.rsplit_once('.').expect("key is dotted");
        let config = ConfigFile::from_toml(&format!("[{section}]\n{name} = {value}\n"))
            .expect("the raw value parses as TOML");
        let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .expect_err("an invalid dimension is refused");
        assert!(
            matches!(&error, ConfigError::InvalidLimit { key: field } if *field == key),
            "{key}: {error}"
        );
    }
}

/// `[run] max_iterations` never fed a run's behaviour and is now removed
/// outright: the error names the replacement rather than a bare unknown field.
#[test]
fn the_run_max_iterations_key_names_its_limits_run_replacement() {
    let error =
        ConfigFile::from_toml("[run]\nmax_iterations = 5\n").expect_err("a retired key is refused");
    let message = error.to_string();
    assert!(
        message.contains("limits.run.turns"),
        "the error must name the replacement: {message}"
    );
}
