//! `[limits.job]` progress guards and the retired request/tool/time defaults.
//!
//! Requests and tokens report what a job spent; the progress guards decide
//! whether it is still getting anywhere. These tests pin the defaults, the
//! explicit overrides, and the fact that the ceilings a config written against
//! an earlier build states still bind.

use saya_config::{
    BudgetLimit, CodingBudgetMode, ConfigError, ConfigFile, ConnectionsFile, ResolutionInput,
    resolve,
};

fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

fn resolve_user(toml: &str) -> saya_config::ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("user fixture parses")),
    )
    .expect("resolution succeeds")
}

fn resolve_err(toml: &str) -> ConfigError {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("the raw value parses as TOML")),
    )
    .expect_err("resolution refuses the setting")
}

#[test]
fn job_progress_guards_have_stated_defaults() {
    let job = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("default config resolves")
        .limits
        .job;
    assert_eq!(job.no_progress_turns, 8);
    assert_eq!(job.repeat_limit, 3);
    assert_eq!(job.wall_clock_seconds, BudgetLimit::Unlimited);
    assert_eq!(job.max_cost_micros, None);
}

#[test]
fn job_backstops_are_runaway_only() {
    let job = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("default config resolves")
        .limits
        .job;
    assert_eq!(job.budget_mode, CodingBudgetMode::Bounded);
    assert_eq!(job.logical_requests, finite(2_000));
    assert_eq!(job.transport_attempts, finite(6_000));
    assert_eq!(job.tool_calls, finite(10_000));
    assert_eq!(job.known_tokens, BudgetLimit::Unlimited);
    assert_eq!(job.job_tokens, BudgetLimit::Unlimited);
    // The durable policy still needs a finite deadline; it is a runaway
    // backstop far beyond any healthy job, not the 1,200 s stop it was.
    assert!(
        job.elapsed_seconds
            .value()
            .is_some_and(|seconds| seconds > 86_400)
    );
}

#[test]
fn job_progress_guards_take_explicit_values() {
    let job = resolve_user(
        "[limits.job]\nno_progress_turns = 12\nrepeat_limit = 5\n\
         wall_clock_seconds = 900\nmax_cost_usd = 2.5\n",
    )
    .limits
    .job;
    assert_eq!(job.no_progress_turns, 12);
    assert_eq!(job.repeat_limit, 5);
    assert_eq!(job.wall_clock_seconds, finite(900));
    assert_eq!(job.max_cost_micros, Some(2_500_000));
}

#[test]
fn a_zero_progress_count_disables_that_guard() {
    let job = resolve_user("[limits.job]\nno_progress_turns = 0\nrepeat_limit = 0\n")
        .limits
        .job;
    assert_eq!(job.no_progress_turns, 0);
    assert_eq!(job.repeat_limit, 0);
}

#[test]
fn an_integer_cost_is_usd_and_converts_to_micros() {
    let job = resolve_user("[limits.job]\nmax_cost_usd = 3\n").limits.job;
    assert_eq!(job.max_cost_micros, Some(3_000_000));
}

#[test]
fn retired_job_defaults_still_bind_when_stated() {
    let job = resolve_user(
        "[limits.job]\nlogical_requests = 80\ntransport_attempts = 240\n\
         tool_calls = 300\nknown_tokens = 400_000\nelapsed_seconds = 1_200\n",
    )
    .limits
    .job;
    assert_eq!(job.logical_requests, finite(80));
    assert_eq!(job.transport_attempts, finite(240));
    assert_eq!(job.tool_calls, finite(300));
    assert_eq!(job.known_tokens, finite(400_000));
    assert_eq!(job.elapsed_seconds, finite(1_200));
}

#[test]
fn uncapped_removes_backstops_but_keeps_the_progress_guards() {
    let job = resolve_user(
        "[limits.job]\nbudget_mode = \"uncapped\"\nno_progress_turns = 4\nrepeat_limit = 2\n",
    )
    .limits
    .job;
    assert_eq!(job.budget_mode, CodingBudgetMode::Uncapped);
    assert_eq!(job.logical_requests, BudgetLimit::Unlimited);
    assert_eq!(job.tool_calls, BudgetLimit::Unlimited);
    assert_eq!(job.known_tokens, BudgetLimit::Unlimited);
    assert_eq!(job.no_progress_turns, 4);
    assert_eq!(job.repeat_limit, 2);
}

#[test]
fn uncapped_still_conflicts_with_a_stated_backstop() {
    let error = resolve_err("[limits.job]\nbudget_mode = \"uncapped\"\nlogical_requests = 45\n");
    assert!(
        matches!(&error, ConfigError::JobBudgetModeConflict { setting }
            if *setting == "limits.job.logical_requests"),
        "{error}"
    );
}

#[test]
fn an_invalid_progress_guard_names_its_key() {
    for (key, value) in [
        ("no_progress_turns", "-1"),
        ("no_progress_turns", "1.5"),
        ("repeat_limit", "'three'"),
        ("wall_clock_seconds", "0"),
        ("max_cost_usd", "-1.0"),
        ("max_cost_usd", "'cheap'"),
    ] {
        let error = resolve_err(&format!("[limits.job]\n{key} = {value}\n"));
        assert!(
            matches!(&error, ConfigError::InvalidLimit { key: field }
                if *field == format!("limits.job.{key}")),
            "{key} = {value}: {error}"
        );
    }
}

#[test]
fn a_progress_guard_above_its_ceiling_is_refused() {
    for (key, value) in [
        ("no_progress_turns", 10_001),
        ("repeat_limit", 10_001),
        ("wall_clock_seconds", 2_592_001),
        ("max_cost_usd", 1_000_001),
    ] {
        let error = resolve_err(&format!("[limits.job]\n{key} = {value}\n"));
        assert!(
            matches!(&error, ConfigError::SettingOutOfRange { field, .. }
                if *field == format!("limits.job.{key}")),
            "{key}: {error}"
        );
    }
}

#[test]
fn run_steps_get_the_same_progress_defaults_and_overrides() {
    let defaults = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("default config resolves")
        .limits
        .run;
    assert_eq!(defaults.no_progress_turns, 8);
    assert_eq!(defaults.repeat_limit, 3);
    let stated = resolve_user("[limits.run]\nno_progress_turns = 6\nrepeat_limit = 4\n")
        .limits
        .run;
    assert_eq!(stated.no_progress_turns, 6);
    assert_eq!(stated.repeat_limit, 4);
}

/// The run contract's budget shape carries the guards: the defaults bind,
/// and a zero disables one by leaving the dimension unset.
#[test]
fn run_budgets_carry_the_progress_guards() {
    let defaults = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("default config resolves")
        .limits
        .run
        .budgets();
    assert_eq!(defaults.no_progress_turns, Some(8));
    assert_eq!(defaults.repeat_limit, Some(3));
    let disabled = resolve_user("[limits.run]\nno_progress_turns = 0\nrepeat_limit = 0\n")
        .limits
        .run
        .budgets();
    assert_eq!(disabled.no_progress_turns, None);
    assert_eq!(disabled.repeat_limit, None);
}

#[test]
fn an_invalid_run_progress_guard_names_its_key() {
    let error = resolve_err("[limits.run]\nno_progress_turns = -1\n");
    assert!(
        matches!(&error, ConfigError::InvalidLimit { key }
            if *key == "limits.run.no_progress_turns"),
        "{error}"
    );
}
