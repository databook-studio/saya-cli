//! `run.candidates` — the number of independent agent attempts per question.
//!
//! Mirrors `run.max_iterations` end to end: file model, layering, resolution,
//! diagnostics, and the CLI override. The default is `1` (today's single-run
//! behaviour), so a user who sets nothing changes nothing. Each extra candidate
//! is another full agent run, so the value is bounded to `1..=16`. Nothing
//! reads it yet — the selection logic is a separate task.

use saya_config::{
    CliOverrides, ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, resolve,
};

fn resolve_with_user(toml: &str) -> saya_config::ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
    )
    .expect("resolution succeeds")
}

/// Absent from the config, the resolved value is `1` — exactly today's
/// single-run behaviour. A user who sets nothing must see no change.
#[test]
fn candidates_default_to_one_when_absent() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    assert_eq!(resolved.candidates, 1);
}

/// `[run] candidates = 4` resolves to `4`.
#[test]
fn candidates_resolve_from_file() {
    let resolved = resolve_with_user("[run]\ncandidates = 4\n");
    assert_eq!(resolved.candidates, 4);
}

/// The `--candidates` CLI override has the highest precedence, so a flag wins
/// over a `[run] candidates` value the config file declared.
#[test]
fn cli_candidates_flag_overrides_the_config_file_value() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[run]\ncandidates = 4\n").unwrap())
            .with_cli(CliOverrides {
                candidates: Some(3),
                ..Default::default()
            }),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.candidates, 3);
}

/// `0` is meaningless (zero attempts answer nothing) and is rejected at
/// resolve time with a typed error whose message names the accepted range.
#[test]
fn candidates_below_one_is_rejected_naming_the_range() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[run]\ncandidates = 0\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingOutOfRange {
                field: "candidates",
                value: 0,
                min: 1,
                max: 16
            }
        ),
        "expected SettingOutOfRange for candidates = 0, got {error:?}"
    );
    let display = format!("{error}");
    assert!(
        display.contains("candidates") && display.contains("1..=16"),
        "error must name the field and the accepted range: {display}"
    );
}

/// An unbounded value would let a typo start hundreds of agent runs, so `17`
/// is rejected at resolve time with the same range-naming error.
#[test]
fn candidates_above_the_maximum_is_rejected_naming_the_range() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[run]\ncandidates = 17\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingOutOfRange {
                field: "candidates",
                value: 17,
                min: 1,
                max: 16
            }
        ),
        "expected SettingOutOfRange for candidates = 17, got {error:?}"
    );
    let display = format!("{error}");
    assert!(
        display.contains("1..=16"),
        "error must name the accepted range: {display}"
    );
}

/// The boundary values `1` and `16` resolve, matching the inclusive-bounds
/// style this crate already uses for `[memory]` range checks.
#[test]
fn candidates_bounds_are_inclusive() {
    assert_eq!(resolve_with_user("[run]\ncandidates = 1\n").candidates, 1);
    assert_eq!(resolve_with_user("[run]\ncandidates = 16\n").candidates, 16);
}

/// `SAYA_CANDIDATES` mirrors `SAYA_MAX_ITERATIONS`: the process environment
/// overrides a file value, so a script can raise candidates per-invocation.
#[test]
fn candidates_env_var_overrides_the_file_value() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[run]\ncandidates = 2\n").unwrap())
            .with_process_env([("SAYA_CANDIDATES", "5")]),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.candidates, 5);
}

/// `run.max_iterations` is not on the protected list, so neither is
/// `candidates`: it is a cost control, not an exfiltration or read-only
/// control, and a project layer may set it without `--trust-project-config`.
/// This pins the layering rule found for this setting.
#[test]
fn project_layer_candidates_applies_because_it_is_not_protected() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_project(ConfigFile::from_toml("[run]\ncandidates = 6\n").unwrap()),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.candidates, 6);
    assert!(
        !resolved
            .ignored_project_overrides
            .iter()
            .any(|name| name == "run.candidates"),
        "candidates is not security-critical, so it must not be reported as ignored: {:?}",
        resolved.ignored_project_overrides
    );
}

/// Diagnostics report `candidates` next to `max_iterations`: the file view
/// shows what was declared (`None` when unset), the resolved view shows the
/// effective value (`1` when unset). `saya config show` prints the latter.
#[test]
fn diagnostics_report_candidates_like_their_neighbours() {
    // File view: a declared value is carried; an unset value is `None`.
    let set = ConfigFile::from_toml("[run]\ncandidates = 4\n").unwrap();
    assert_eq!(set.redacted_diagnostics().candidates, Some(4));
    let unset = ConfigFile::from_toml("[run]\nmax_rows = 10\n").unwrap();
    assert_eq!(unset.redacted_diagnostics().candidates, None);

    // Resolved view: the effective value, `1` when unset.
    let resolved_set = resolve_with_user("[run]\ncandidates = 4\n");
    assert_eq!(resolved_set.redacted_diagnostics().candidates, 4);
    let resolved_default =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    assert_eq!(resolved_default.redacted_diagnostics().candidates, 1);

    // The JSON `config show` prints names the field, so a user debugging cost
    // can find it next to its neighbours.
    let rendered = serde_json::to_string(&resolved_set.redacted_diagnostics()).unwrap();
    assert!(
        rendered.contains("\"candidates\":4"),
        "resolved diagnostics must report candidates: {rendered}"
    );
}
