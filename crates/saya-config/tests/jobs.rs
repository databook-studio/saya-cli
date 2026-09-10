//! `[jobs]` — the default budgets a run is declared with when the run's
//! spec and each of its steps declare none.
//!
//! The section mirrors the four budget dimensions M1-6 adds (wall-clock,
//! tokens per endpoint, turns, tool calls); `runner` and `fetch` keys arrive
//! in later items. The turn ceiling falls back to `[run] max_iterations`,
//! which this resolution gives its first behavioural reader. The engine never
//! reads the environment for budgets (plan G3), so no `[jobs]` key has an
//! environment override.

use std::collections::BTreeMap;
use std::time::Duration;

use saya_config::{ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

fn resolve_with_user(toml: &str) -> saya_config::ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
    )
    .expect("resolution succeeds")
}

/// Absent from the config, the resolved `[jobs]` defaults carry the
/// `[run] max_iterations` default as the turn ceiling and leave every other
/// dimension unlimited — the engine layers more specific budgets over these.
#[test]
fn jobs_default_to_the_max_iterations_turn_ceiling_and_nothing_else() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    let jobs = &resolved.jobs;
    assert_eq!(
        jobs.turns, 12,
        "the [jobs] turn default is the [run] max_iterations default"
    );
    assert_eq!(
        jobs.wall_clock_seconds, None,
        "no wall-clock ceiling by default"
    );
    assert_eq!(jobs.tool_calls, None, "no tool-call ceiling by default");
    assert!(
        jobs.tokens_per_endpoint.is_empty(),
        "no token ceilings by default: {:?}",
        jobs.tokens_per_endpoint
    );
}

/// `[jobs]` resolves every declared key into the effective default budgets.
#[test]
fn jobs_resolve_from_the_file() {
    let resolved = resolve_with_user(
        "[jobs]\nturns = 40\ntool_calls = 25\nwall_clock_seconds = 1800\n\
         [jobs.tokens_per_endpoint]\n\"local-ollama\" = 200_000\n",
    );
    let jobs = &resolved.jobs;
    assert_eq!(jobs.turns, 40);
    assert_eq!(jobs.tool_calls, Some(25));
    assert_eq!(jobs.wall_clock_seconds, Some(1800));
    assert_eq!(
        jobs.tokens_per_endpoint,
        BTreeMap::from([("local-ollama".into(), 200_000)])
    );
}

/// `run.max_iterations` is the run-episode default turn ceiling: with no
/// `[jobs] turns` declared, the resolved jobs turn budget is exactly the
/// resolved `max_iterations`. This is the knob's first behavioural reader.
#[test]
fn jobs_turn_ceiling_falls_back_to_run_max_iterations() {
    let resolved = resolve_with_user("[run]\nmax_iterations = 30\n");
    assert_eq!(resolved.jobs.turns, 30);
    assert_eq!(resolved.max_iterations, 30);
}

/// `[jobs] turns` is more specific than the legacy `[run] max_iterations`, so
/// it wins when both are declared.
#[test]
fn jobs_turn_ceiling_beats_run_max_iterations() {
    let resolved = resolve_with_user("[run]\nmax_iterations = 30\n[jobs]\nturns = 7\n");
    assert_eq!(resolved.jobs.turns, 7);
}

/// Zero is meaningless on every budget default: zero turns or zero tool calls
/// pause a run before it does anything, and a zero-second wall clock is the
/// same instant pause. Each is rejected at resolve time with a typed error
/// naming the floor, matching the `context_byte_budget` discipline.
#[test]
fn jobs_below_one_is_rejected_naming_the_field() {
    for (toml, field) in [
        ("[jobs]\nturns = 0\n", "turns"),
        ("[jobs]\ntool_calls = 0\n", "tool_calls"),
        ("[jobs]\nwall_clock_seconds = 0\n", "wall_clock_seconds"),
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        assert!(
            matches!(error, ConfigError::SettingBelowMinimum { min: 1, .. }),
            "expected SettingBelowMinimum for {toml:?}, got {error:?}"
        );
        let display = format!("{error}");
        assert!(
            display.contains(field),
            "error must name the field {field:?}: {display}"
        );
    }
}

/// `[jobs] tokens_per_endpoint` values are token ceilings, and zero has the
/// same degenerate-default meaning: every run pauses the moment the endpoint
/// spends a token. Rejected like its sibling budgets.
#[test]
fn jobs_zero_token_ceiling_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[jobs.tokens_per_endpoint]\nollama = 0\n")
                .expect("fixture must parse"),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingBelowMinimum {
                value: 0,
                min: 1,
                ..
            }
        ),
        "expected SettingBelowMinimum for a zero token ceiling, got {error:?}"
    );
}

/// `run.max_iterations` is now the run-episode default turn ceiling, so the
/// same below-one discipline applies to it: a dead knob tolerated `0`, but a
/// reader turns `0` into "pause before the first turn", which is a typo, not
/// an intent.
#[test]
fn max_iterations_below_one_is_rejected_once_it_has_a_reader() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[run]\nmax_iterations = 0\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingBelowMinimum {
                field: "max_iterations",
                value: 0,
                min: 1
            }
        ),
        "expected SettingBelowMinimum for max_iterations = 0, got {error:?}"
    );
}

/// The token map is bounded exactly like the run contract's budget shape
/// (`MAX_BUDGET_ENDPOINTS`), so a config the contract would reject cannot
/// resolve.
#[test]
fn jobs_more_than_the_contract_endpoint_count_is_rejected() {
    let entries = (0..9)
        .map(|i| format!("endpoint-{i} = 1000"))
        .collect::<Vec<_>>()
        .join("\n");
    let toml = format!("[jobs.tokens_per_endpoint]\n{entries}\n");
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(&toml).expect("fixture must parse")),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingAboveMaximum {
                field: "tokens_per_endpoint",
                value: 9,
                max: 8
            }
        ),
        "expected SettingAboveMaximum for nine endpoints, got {error:?}"
    );
}

/// An endpoint key must have the run-scoped name shape the contracts demand;
/// a key the contract rejects at plan-validation time must be caught at
/// resolve time instead, far from the config mistake.
#[test]
fn jobs_endpoint_key_outside_the_name_shape_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[jobs.tokens_per_endpoint]\n\"two words\" = 1000\n")
                .expect("fixture must parse"),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::InvalidEndpointName { key, .. } if key == "two words"),
        "expected InvalidEndpointName for a whitespace key, got {error:?}"
    );
}

/// The resolved defaults convert into the run contract's `Budgets` shape —
/// the type the engine layers RunSpec and step budgets over — and the result
/// passes the contract's own validation.
#[test]
fn resolved_jobs_convert_into_contract_budgets_that_validate() {
    let resolved = resolve_with_user(
        "[jobs]\nturns = 40\ntool_calls = 25\nwall_clock_seconds = 1800\n\
         [jobs.tokens_per_endpoint]\n\"local-ollama\" = 200_000\n",
    );
    let budgets = resolved.jobs.budgets();
    budgets
        .validate()
        .expect("resolved jobs satisfy the run contract");
    assert_eq!(budgets.turns, Some(40));
    assert_eq!(budgets.tool_calls, Some(25));
    assert_eq!(budgets.wall_clock, Some(Duration::from_secs(1800)));
    assert_eq!(
        budgets.tokens_per_endpoint.get("local-ollama"),
        Some(&200_000)
    );

    // With nothing declared, the conversion still carries the turn default.
    let default_budgets = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("resolution succeeds")
        .jobs
        .budgets();
    default_budgets
        .validate()
        .expect("the default jobs satisfy the run contract");
    assert_eq!(default_budgets.turns, Some(12));
}

/// `ConfigFile` is `deny_unknown_fields`, so an unknown key inside `[jobs]`
/// fails at parse time with the offending key named — a typo can never
/// silently fall back to a default.
#[test]
fn jobs_unknown_keys_are_rejected_naming_the_key() {
    let error = ConfigFile::from_toml("[jobs]\nfrobnicate = 1\n")
        .expect_err("an unknown [jobs] key must be rejected");
    let display = format!("{error}");
    assert!(
        display.contains("frobnicate"),
        "the parse error must name the unknown key, not just the section: {display}"
    );
}

/// `[jobs]` is a cost control, not a security-critical setting, so the
/// project layer may set it and the merge must carry it across layers.
#[test]
fn project_layer_jobs_applies_and_is_not_protected() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[jobs]\nturns = 40\n").unwrap())
            .with_project(ConfigFile::from_toml("[jobs]\nturns = 7\n").unwrap()),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.jobs.turns, 7);
    assert!(
        resolved.ignored_project_overrides.is_empty(),
        "jobs is not security-critical, so nothing may be reported as ignored: {:?}",
        resolved.ignored_project_overrides
    );
}

/// Diagnostics mirror the new settings in both views: the file view shows
/// what was declared (`None`/empty when unset), the resolved view shows the
/// effective values, and the JSON `config show` prints names them.
#[test]
fn diagnostics_report_jobs_like_their_neighbours() {
    // File view: declared values are carried; unset ones are absent.
    let set = ConfigFile::from_toml("[jobs]\nturns = 40\ntool_calls = 25\n").unwrap();
    let declared = set.redacted_diagnostics();
    assert_eq!(declared.jobs_turns, Some(40));
    assert_eq!(declared.jobs_tool_calls, Some(25));
    assert_eq!(declared.jobs_wall_clock_seconds, None);
    assert_eq!(declared.jobs_tokens_per_endpoint, None);
    let unset = ConfigFile::from_toml("[run]\nmax_rows = 10\n").unwrap();
    assert_eq!(unset.redacted_diagnostics().jobs_turns, None);

    // Resolved view: the effective values, with the max_iterations fallback.
    let resolved_set = resolve_with_user("[run]\nmax_iterations = 30\n");
    let shown = resolved_set.redacted_diagnostics();
    assert_eq!(shown.jobs_turns, 30);
    assert_eq!(shown.jobs_tool_calls, None);
    assert_eq!(shown.jobs_wall_clock_seconds, None);
    assert!(shown.jobs_tokens_per_endpoint.is_empty());
    let rendered = serde_json::to_string(&shown).unwrap();
    assert!(
        rendered.contains("\"jobs_turns\":30"),
        "resolved diagnostics must report jobs_turns: {rendered}"
    );

    // A token ceiling is echoed in both views without redaction: endpoint
    // names and token counts are user-declared figures, not secrets.
    let tokens = resolve_with_user("[jobs.tokens_per_endpoint]\nollama = 200_000\n");
    let shown_tokens = tokens.redacted_diagnostics();
    assert_eq!(
        shown_tokens.jobs_tokens_per_endpoint,
        BTreeMap::from([("ollama".into(), 200_000)])
    );
    let rendered = serde_json::to_string(&tokens.redacted_diagnostics()).unwrap();
    assert!(
        rendered.contains("\"jobs_tokens_per_endpoint\":{\"ollama\":200000}"),
        "resolved diagnostics must report the token ceilings: {rendered}"
    );
}
