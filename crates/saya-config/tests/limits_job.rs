//! `[limits.job]` — the ceilings a coding job's root and its helpers run
//! under.
//!
//! The table holds the six `max_*` ceilings and `budget_mode` that `[coding]`
//! carried.
//! It is protected whole, like `[coding]` was: a repository's
//! `.saya/config.toml` is untrusted input and must not raise a ceiling the
//! user set. `budget_mode = "uncapped"` is the one unlimited spelling here —
//! the durable budget policy the harness stores is either finite or uncapped
//! and cannot represent a mixed one, so a per-dimension `"unlimited"` is
//! refused naming that key.

use saya_config::{
    BudgetLimit, CodingBudgetMode, ConfigError, ConfigFile, ConnectionsFile, ResolutionInput,
    ResolvedConfig, ResolvedJobLimits, resolve,
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

fn resolve_layers(user: &str, project: &str) -> ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(user).expect("user fixture parses"))
            .with_project(ConfigFile::from_toml(project).expect("project fixture parses")),
    )
    .expect("resolution succeeds")
}

/// The defaults: runaway backstops, unlimited tokens, and the two progress
/// guards. The old 80-request / 300-tool / 1,200 s / 400k-token stops are no
/// longer defaults; a config that states them still binds (see
/// `retired_job_defaults_still_bind_when_stated`).
#[test]
fn limits_job_defaults_to_runaway_backstops() {
    let limits = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("default config resolves")
        .limits;
    assert_eq!(limits.job, ResolvedJobLimits::default());
    assert_eq!(limits.job.budget_mode, CodingBudgetMode::Bounded);
    assert_eq!(limits.job.logical_requests, finite(2_000));
    assert_eq!(limits.job.transport_attempts, finite(6_000));
    assert_eq!(limits.job.tool_calls, finite(10_000));
    assert_eq!(limits.job.known_tokens, BudgetLimit::Unlimited);
    assert_eq!(limits.job.job_tokens, BudgetLimit::Unlimited);
    assert_eq!(limits.job.elapsed_seconds, finite(604_800));
    assert_eq!(limits.job.no_progress_turns, 8);
    assert_eq!(limits.job.repeat_limit, 3);
}

/// The bounded mode accepts a raised request ceiling up to its stated hard
/// cap; the transport ceiling must fund it.
#[test]
fn bounded_mode_accepts_logical_requests_up_to_the_stated_hard_cap() {
    let limits =
        resolve_user("[limits.job]\nlogical_requests = 100_000\ntransport_attempts = 300_000\n")
            .limits;
    assert_eq!(limits.job.logical_requests, finite(100_000));
    assert_eq!(limits.job.transport_attempts, finite(300_000));

    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[limits.job]\nlogical_requests = 100_001\n")
                .expect("the raw value parses as TOML"),
        ),
    )
    .expect_err("a request ceiling above the hard cap is refused");
    assert!(
        matches!(
            error,
            ConfigError::SettingOutOfRange {
                field: "limits.job.logical_requests",
                max: 100_000,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn limits_job_states_every_dimension() {
    let limits = resolve_user(
        "[limits.job]\nlogical_requests = 45\ntransport_attempts = 90\n\
         tool_calls = 30\nknown_tokens = 9000\njob_tokens = 600_000\n\
         elapsed_seconds = 60\n",
    )
    .limits;
    assert_eq!(limits.job.logical_requests, finite(45));
    assert_eq!(limits.job.transport_attempts, finite(90));
    assert_eq!(limits.job.tool_calls, finite(30));
    assert_eq!(limits.job.known_tokens, finite(9000));
    assert_eq!(limits.job.job_tokens, finite(600_000));
    assert_eq!(limits.job.elapsed_seconds, finite(60));
    assert_eq!(
        limits.job.budget_mode,
        CodingBudgetMode::Bounded,
        "stating a dimension leaves the mode at its default"
    );
}

/// The one unlimited spelling: `budget_mode = "uncapped"` removes every
/// ceiling at once, because the durable policy is finite-or-uncapped.
#[test]
fn uncapped_is_the_one_unlimited_spelling() {
    let limits = resolve_user("[limits.job]\nbudget_mode = \"uncapped\"\n").limits;
    assert_eq!(limits.job.budget_mode, CodingBudgetMode::Uncapped);
    assert_eq!(limits.job.logical_requests, BudgetLimit::Unlimited);
    assert_eq!(limits.job.transport_attempts, BudgetLimit::Unlimited);
    assert_eq!(limits.job.tool_calls, BudgetLimit::Unlimited);
    assert_eq!(limits.job.known_tokens, BudgetLimit::Unlimited);
    assert_eq!(limits.job.job_tokens, BudgetLimit::Unlimited);
    assert_eq!(limits.job.elapsed_seconds, BudgetLimit::Unlimited);
}

/// A per-dimension `"unlimited"` has no durable spelling, so it is refused
/// with the fix named: the mode key, not the dimension.
#[test]
fn a_coding_dimension_refuses_unlimited_and_names_budget_mode() {
    for key in [
        "logical_requests",
        "transport_attempts",
        "tool_calls",
        "known_tokens",
        "job_tokens",
        "elapsed_seconds",
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default()).with_user(
                ConfigFile::from_toml(&format!("[limits.job]\n{key} = 'unlimited'\n"))
                    .expect("the raw value parses as TOML"),
            ),
        )
        .expect_err("a per-dimension unlimited has no durable spelling");
        let message = error.to_string();
        assert!(
            matches!(&error, ConfigError::JobLimitUnlimited { key: field } if *field == format!("limits.job.{key}")),
            "{key}: {error}"
        );
        assert!(
            message.contains("limits.job.budget_mode"),
            "{key} must name the one unlimited spelling: {message}"
        );
    }
}

/// `uncapped` states "no ceiling at all", so a dimension beside it is a
/// contradiction rather than an override.
#[test]
fn uncapped_conflicts_with_every_stated_dimension() {
    for (key, value) in [
        ("logical_requests", "45"),
        ("transport_attempts", "90"),
        ("tool_calls", "30"),
        ("known_tokens", "9000"),
        ("job_tokens", "600000"),
        ("elapsed_seconds", "60"),
    ] {
        let config = ConfigFile::from_toml(&format!(
            "[limits.job]\nbudget_mode = \"uncapped\"\n{key} = {value}\n"
        ))
        .expect("the raw config parses as TOML");
        let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .expect_err("uncapped mode refuses an explicit ceiling");
        assert!(
            matches!(&error, ConfigError::JobBudgetModeConflict { setting }
                if *setting == format!("limits.job.{key}")),
            "{key}: {error}"
        );
    }
}

/// The six `[coding] max_*` keys and `[coding] budget_mode` are refused, and each
/// says where it went.
#[test]
fn the_coding_limit_keys_name_their_limits_job_replacement() {
    for (key, replacement) in [
        ("budget_mode", "limits.job.budget_mode"),
        ("max_logical_model_requests", "limits.job.logical_requests"),
        (
            "max_physical_transport_attempts",
            "limits.job.transport_attempts",
        ),
        ("max_tool_calls", "limits.job.tool_calls"),
        ("max_known_tokens", "limits.job.known_tokens"),
        ("max_job_tokens", "limits.job.job_tokens"),
        ("max_elapsed_seconds", "limits.job.elapsed_seconds"),
    ] {
        let value = if key == "budget_mode" {
            "\"bounded\""
        } else {
            "5"
        };
        let error = ConfigFile::from_toml(&format!("[coding]\n{key} = {value}\n"))
            .expect_err("a moved key is refused");
        let message = error.to_string();
        assert!(
            message.contains(&format!("was moved: write {replacement} instead")),
            "{key}: {message}"
        );
    }
}

/// A coding ceiling stays a trusted-layer decision, like the `[coding]` table
/// it replaces.
#[test]
fn a_project_layer_cannot_state_the_job_limits() {
    let resolved = resolve_layers(
        "[limits.job]\nlogical_requests = 45\n",
        "[limits.job]\nlogical_requests = 80\nbudget_mode = \"uncapped\"\n",
    );
    assert_eq!(resolved.limits.job.logical_requests, finite(45));
    assert_eq!(resolved.limits.job.budget_mode, CodingBudgetMode::Bounded);
    assert_eq!(
        resolved.ignored_project_overrides,
        vec!["limits.job".to_owned()]
    );
}

/// Every coding key names itself in the error, so the fix is never in doubt.
#[test]
fn a_coding_dimension_that_is_not_a_positive_integer_is_refused() {
    for (value, key) in [
        ("0", "limits.job.logical_requests"),
        ("-1", "limits.job.transport_attempts"),
        ("1.5", "limits.job.tool_calls"),
        ("true", "limits.job.known_tokens"),
        ("'600000'", "limits.job.job_tokens"),
        ("[]", "limits.job.elapsed_seconds"),
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

/// The ceilings those keys carried still bind, and each refusal names the
/// new dotted key.
#[test]
fn a_coding_dimension_above_its_ceiling_is_refused() {
    for (key, value) in [
        ("logical_requests", 100_001),
        ("transport_attempts", 300_001),
        ("tool_calls", 1_000_001),
        ("known_tokens", 1_000_000_001),
        ("job_tokens", 100_000_001),
        ("elapsed_seconds", 2_592_001),
    ] {
        let config = ConfigFile::from_toml(&format!("[limits.job]\n{key} = {value}\n"))
            .expect("the raw value parses as TOML");
        let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .expect_err("a ceiling above its maximum is refused");
        assert!(
            matches!(&error, ConfigError::SettingOutOfRange { field, .. }
                if *field == format!("limits.job.{key}")),
            "{key}: {error}"
        );
    }
}

/// Transport attempts fund the logical requests, so a ceiling below them would
/// refuse work the job was admitted to do.
#[test]
fn transport_attempts_may_not_understate_the_logical_requests() {
    let config =
        ConfigFile::from_toml("[limits.job]\nlogical_requests = 45\ntransport_attempts = 44\n")
            .expect("the raw config parses as TOML");
    let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
        .expect_err("transport attempts below the logical requests are refused");
    assert!(
        matches!(&error, ConfigError::SettingOutOfRange { field, .. }
            if *field == "limits.job.transport_attempts"),
        "{error}"
    );
}
