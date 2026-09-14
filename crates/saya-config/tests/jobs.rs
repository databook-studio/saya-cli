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

/// `[jobs.fetch]` (M3-3) resolves to conservative download-budget defaults
/// when absent, and the declared keys resolve when present.
#[test]
fn jobs_fetch_resolves_defaults_and_declared_values() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    let fetch = &resolved.jobs.fetch;
    assert_eq!(
        *fetch,
        saya_config::ResolvedFetchJobs::default(),
        "the absent [jobs.fetch] resolves to the conservative defaults"
    );

    let declared = resolve_with_user(
        "[jobs.fetch]\nmax_file_bytes = 1024\nmax_run_bytes = 4096\ntimeout_seconds = 30\n",
    );
    assert_eq!(
        declared.jobs.fetch,
        saya_config::ResolvedFetchJobs {
            max_file_bytes: 1024,
            max_run_bytes: 4096,
            timeout_seconds: 30,
        }
    );
}

/// A below-minimum `[jobs.fetch]` value is a typed resolve error naming the
/// field — zero download bytes or a zero-second timeout would pause every
/// download before its first byte, a typo, not an intent. Never a silent
/// clamp.
#[test]
fn jobs_fetch_below_one_is_rejected_naming_the_field() {
    for (toml, field) in [
        ("[jobs.fetch]\nmax_file_bytes = 0\n", "fetch.max_file_bytes"),
        ("[jobs.fetch]\nmax_run_bytes = 0\n", "fetch.max_run_bytes"),
        (
            "[jobs.fetch]\ntimeout_seconds = 0\n",
            "fetch.timeout_seconds",
        ),
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

/// An unknown key inside `[jobs.fetch]` is rejected at parse time with the
/// offending key named — `deny_unknown_fields` reaches the sub-table too.
#[test]
fn jobs_fetch_unknown_keys_are_rejected_naming_the_key() {
    let error = ConfigFile::from_toml("[jobs.fetch]\nfrobnicate = 1\n")
        .expect_err("an unknown [jobs.fetch] key must be rejected");
    let display = format!("{error}");
    assert!(
        display.contains("frobnicate"),
        "the parse error must name the unknown key: {display}"
    );
}

/// `[jobs.runner]` (M5-4) resolves to no programs and the conservative
/// timeout when absent, and the declared keys resolve when present. The
/// empty allow is the point: there is no default program universe a run
/// gets for free — approving programs is a deliberate act.
#[test]
fn jobs_runner_resolves_defaults_and_declared_values() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    assert_eq!(
        resolved.jobs.runner,
        saya_config::ResolvedRunnerJobs::default(),
        "the absent [jobs.runner] resolves to the conservative defaults"
    );
    assert!(
        resolved.jobs.runner.allow.is_empty(),
        "no default runner programs: {:?}",
        resolved.jobs.runner.allow
    );

    let declared = resolve_with_user(
        "[jobs.runner]\nallow = [\"duckdb\", \"jq\"]\ntimeout_seconds = 60\nprogram_dir = \"/opt/saya-programs\"\n",
    );
    assert_eq!(
        declared.jobs.runner.allow,
        vec!["duckdb".to_owned(), "jq".to_owned()]
    );
    assert_eq!(declared.jobs.runner.timeout_seconds, 60);
    assert_eq!(
        declared.jobs.runner.program_dir.as_deref(),
        Some(std::path::Path::new("/opt/saya-programs")),
        "the staged programs' directory resolves as declared"
    );
}

/// A `[jobs.runner]` value below the floor is a typed resolve error naming
/// the field — a zero-second timeout would kill every child before its first
/// byte, a typo, not an intent. Never a silent clamp.
#[test]
fn jobs_runner_timeout_below_one_is_rejected_naming_the_field() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[jobs.runner]\ntimeout_seconds = 0\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::SettingBelowMinimum { min: 1, .. }),
        "expected SettingBelowMinimum for a zero runner timeout, got {error:?}"
    );
    let display = format!("{error}");
    assert!(
        display.contains("runner.timeout_seconds"),
        "error must name the field: {display}"
    );
}

/// A `[jobs.runner] allow` entry that is not a bare program name is refused
/// at resolve time with the reason — a path-shaped or control-carrying entry
/// can never name a program the runner will run.
#[test]
fn jobs_runner_allow_outside_the_name_shape_is_rejected() {
    for (toml, program) in [
        ("[jobs.runner]\nallow = [\"/bin/echo\"]\n", "/bin/echo"),
        ("[jobs.runner]\nallow = [\"two words\"]\n", "two words"),
        ("[jobs.runner]\nallow = [\"\"]\n", ""),
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        assert!(
            matches!(&error, ConfigError::InvalidRunnerProgram { program: p, .. } if p == program),
            "expected InvalidRunnerProgram for {program:?}, got {error:?}"
        );
    }
}

/// A shell or interpreter must not look approved: the runner refuses those
/// structurally (an interpreter spawns arbitrary children from inside the
/// allowlist), so the config refuses the name at resolve time with the
/// reason named.
#[test]
fn jobs_runner_allow_refuses_shells_and_interpreters() {
    for toml in [
        "[jobs.runner]\nallow = [\"bash\"]\n",
        "[jobs.runner]\nallow = [\"sh\"]\n",
        "[jobs.runner]\nallow = [\"python3\"]\n",
        "[jobs.runner]\nallow = [\"env\"]\n",
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        let display = format!("{error}");
        assert!(
            matches!(&error, ConfigError::InvalidRunnerProgram { .. }),
            "expected InvalidRunnerProgram for {toml:?}, got {error:?}"
        );
        assert!(
            display.contains("interpreter"),
            "the refusal must say why shells and interpreters are refused: {display}"
        );
    }
}

/// A repeated `[jobs.runner] allow` entry is a typo, not a wider approval:
/// rejected rather than silently de-duplicated.
#[test]
fn jobs_runner_allow_refuses_duplicates() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[jobs.runner]\nallow = [\"jq\", \"jq\"]\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::InvalidRunnerProgram { program, .. } if program == "jq"),
        "expected InvalidRunnerProgram for the duplicate, got {error:?}"
    );
}

/// The `[jobs.runner] allow` list is bounded like every set-valued approval
/// surface the run contracts carry (`MAX_RUNNER_PROGRAMS`).
#[test]
fn jobs_runner_allow_above_the_contract_count_is_rejected() {
    let entries = (0..33)
        .map(|i| format!("\"prog-{i}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(
        ConfigFile::from_toml(&format!("[jobs.runner]\nallow = [{entries}]\n")).unwrap(),
    ))
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingAboveMaximum {
                field: "runner.allow",
                value: 33,
                max: 32
            }
        ),
        "expected SettingAboveMaximum for 33 programs, got {error:?}"
    );
}

/// An unknown key inside `[jobs.runner]` is rejected at parse time with the
/// offending key named — `deny_unknown_fields` reaches the sub-table too.
#[test]
fn jobs_runner_unknown_keys_are_rejected_naming_the_key() {
    let error = ConfigFile::from_toml("[jobs.runner]\nfrobnicate = 1\n")
        .expect_err("an unknown [jobs.runner] key must be rejected");
    let display = format!("{error}");
    assert!(
        display.contains("frobnicate"),
        "the parse error must name the unknown key: {display}"
    );
}

/// A relative `[jobs.runner] program_dir` is a typed resolve error: the
/// canonical form must not depend on the working directory the config was
/// loaded from, and the run's probe verdict is only as real as the one
/// directory it proved.
#[test]
fn jobs_runner_program_dir_relative_is_rejected_naming_the_field() {
    let error =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[jobs.runner]\nprogram_dir = \"programs\"\n").unwrap(),
        ))
        .unwrap_err();
    assert!(
        matches!(&error, ConfigError::RelativeRunnerProgramDir { path } if path == "programs"),
        "expected RelativeRunnerProgramDir, got {error:?}"
    );
    let display = format!("{error}");
    assert!(
        display.contains("runner.program_dir") && display.contains("absolute"),
        "the refusal must name the field and the requirement: {display}"
    );
}

/// `allow` that names programs requires `program_dir`: the runner resolves
/// every allowlisted program inside one directory, so an allowlist without
/// its directory approves programs that cannot run — a typed resolve error,
/// the same class every other `[jobs]` mistake gets.
#[test]
fn jobs_runner_allow_without_program_dir_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[jobs.runner]\nallow = [\"bench\"]\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::RunnerAllowWithoutProgramDir),
        "expected RunnerAllowWithoutProgramDir, got {error:?}"
    );
    let display = format!("{error}");
    assert!(
        display.contains("program_dir"),
        "the refusal must name the missing key: {display}"
    );
}

/// `program_dir` alone (empty `allow`) is harmless and allowed: it declares
/// where programs would be staged without approving any program.
#[test]
fn jobs_runner_program_dir_alone_resolves() {
    let resolved = resolve_with_user("[jobs.runner]\nprogram_dir = \"/opt/saya-programs\"\n");
    assert!(resolved.jobs.runner.allow.is_empty());
    assert_eq!(
        resolved.jobs.runner.program_dir.as_deref(),
        Some(std::path::Path::new("/opt/saya-programs"))
    );
}

/// A dangling `program_dir` resolves fine: existence is deliberately not a
/// resolve-time question, so `saya ask` and `saya query` are unaffected by
/// any state of the key; a run that approved the runner fails closed at
/// assemble instead.
#[test]
fn jobs_runner_dangling_program_dir_resolves() {
    let resolved = resolve_with_user(
        "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = \"/nonexistent/saya-programs\"\n",
    );
    assert_eq!(
        resolved.jobs.runner.program_dir.as_deref(),
        Some(std::path::Path::new("/nonexistent/saya-programs"))
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

/// The `[jobs.runner]` settings mirror beside their neighbours in both
/// views: the file view shows what was declared (`None` when unset), the
/// resolved view shows the effective values, and the JSON `config show`
/// names the fields — the path itself is user-declared, not a secret.
#[test]
fn diagnostics_report_the_runner_settings_like_their_neighbours() {
    let unset = ConfigFile::from_toml("[run]\nmax_rows = 10\n").unwrap();
    let declared_unset = unset.redacted_diagnostics();
    assert_eq!(declared_unset.jobs_runner_allow, None);
    assert_eq!(declared_unset.jobs_runner_program_dir, None);
    assert_eq!(declared_unset.jobs_runner_timeout_seconds, None);

    let set = ConfigFile::from_toml(
        "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = \"/opt/saya-programs\"\ntimeout_seconds = 45\n",
    )
    .unwrap();
    let declared = set.redacted_diagnostics();
    assert_eq!(declared.jobs_runner_allow, Some(vec!["bench".to_owned()]));
    assert_eq!(
        declared.jobs_runner_program_dir,
        Some("/opt/saya-programs".to_owned())
    );
    assert_eq!(declared.jobs_runner_timeout_seconds, Some(45));

    let shown = resolve_with_user(
        "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = \"/opt/saya-programs\"\n",
    )
    .redacted_diagnostics();
    assert_eq!(shown.jobs_runner_allow, vec!["bench".to_owned()]);
    assert_eq!(
        shown.jobs_runner_program_dir,
        Some("/opt/saya-programs".to_owned())
    );
    assert_eq!(shown.jobs_runner_timeout_seconds, 300);
    let rendered = serde_json::to_string(&shown).unwrap();
    assert!(
        rendered.contains("\"jobs_runner_program_dir\":\"/opt/saya-programs\""),
        "resolved diagnostics must report the program directory: {rendered}"
    );
}

/// The universe disjointness holds by construction (the interpreter
/// approval's design §1): a `[jobs.interpreter] allow` member the runner
/// does not refuse is a typed resolve error pointing at `[jobs.runner]
/// allow` — a non-refused name never rides the interpreter family — while
/// the `[jobs.runner]` entry keeps its exact error, so the two universes
/// cannot drift into each other.
#[test]
fn jobs_interpreter_allow_refuses_programs_the_runner_can_run() {
    for toml in [
        "[jobs.interpreter]\nallow = [\"ripgrep\"]\n",
        "[jobs.interpreter]\nallow = [\"bench\"]\n",
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        let display = format!("{error}");
        assert!(
            matches!(&error, ConfigError::InvalidInterpreterProgram { .. }),
            "expected InvalidInterpreterProgram for {toml:?}, got {error:?}"
        );
        assert!(
            display.contains("[jobs.runner] allow"),
            "the refusal must point at the runner's family: {display}"
        );
    }
}

/// `[jobs.interpreter] allow` requires the one program directory the
/// interpreters are staged in: the resolve refuses the combination with the
/// same typed error class the runner's own allow-without-dir refusal uses.
#[test]
fn jobs_interpreter_allow_without_program_dir_refuses() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[jobs.interpreter]\nallow = [\"python3\"]\n")
                .expect("fixture must parse"),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::InterpreterAllowWithoutProgramDir),
        "expected InterpreterAllowWithoutProgramDir, got {error:?}"
    );
}

/// H1 red: `[host_commands]` resolves `enable`, `pass_env`, and
/// `timeout_seconds` from the user layer. Written before the section exists,
/// so the fixture's unknown section fails parse today.
#[test]
fn host_commands_resolve_from_the_user_layer() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml(
                "[host_commands]\nenable = true\npass_env = ['CI_TOKEN']\ntimeout_seconds = 42\n",
            )
            .expect("fixture parses"),
        ),
    )
    .expect("resolution succeeds");
    assert!(
        resolved.host_commands.enabled,
        "the user layer enables the lane"
    );
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

/// H1 red: `[host_commands]` defaults resolve to lane-off with an empty
/// `pass_env` and the executor's ceiling. Written before the fields exist.
#[test]
fn host_commands_default_to_lane_off() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    assert!(
        !resolved.host_commands.enabled,
        "the lane is off unless stated"
    );
    assert!(
        resolved.host_commands.pass_env.is_empty(),
        "no pass_env by default"
    );
    assert_eq!(
        resolved.host_commands.timeout_seconds, 600,
        "the default ceiling is the executor's own"
    );
}
