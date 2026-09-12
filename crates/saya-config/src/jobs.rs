//! `[jobs]` resolution — the default budgets a run is declared with.
//!
//! The section holds what the run's specification and each step may leave
//! unset, so the engine can layer RunSpec and step budgets over these
//! resolved defaults per dimension (`RunSpec > step > [jobs] >
//! `[run].max_iterations``). There are no environment overrides for `[jobs]`
//! keys by design: run budgets must be reproducible from the run's
//! specification and config alone (plan G3).

use std::collections::BTreeMap;
use std::time::Duration;

use saya_types::{
    Budgets, MAX_BUDGET_ENDPOINTS, MAX_RUNNER_PROGRAMS, is_bare_name, is_name_shaped,
    is_refused_runner_program,
};

use crate::{
    ConfigError,
    model::{FetchJobsFile, InterpreterJobsFile, JobsFile, RunnerJobsFile},
};

/// Smallest accepted `[jobs]` ceiling. A zero on any dimension means "pause
/// before doing anything" — zero turns or zero tool calls stop the run
/// before its first model turn, a zero-second wall clock is the same instant
/// pause, zero tokens starve the endpoint on its first call, and zero
/// download bytes stop `http_download` before its first byte. As
/// *defaults* these are typos, not intents, so they are rejected rather than
/// silently clamped, matching the `context_byte_budget` discipline. No
/// upper bound: a long-running job may set any ceiling it needs, and the
/// unlimited case is "leave it unset".
const MIN_BUDGET: u64 = 1;

/// Provisional `[jobs.fetch]` default per-file download bound: large enough
/// for a corpus artifact, small enough that one URL cannot silently fill a
/// disk. Provisional until M5 measures real runs (U8).
pub(crate) const FETCH_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Provisional `[jobs.fetch]` default total run download bytes: several
/// files, bounded so one run cannot silently fill a disk. Provisional until
/// M5 measures real runs (U8).
pub(crate) const FETCH_MAX_RUN_BYTES: u64 = 1024 * 1024 * 1024;

/// Provisional `[jobs.fetch]` default per-request timeout: matches the
/// `[ai] timeout_seconds` default. With resumable partials a longer
/// transfer is a sequence of budgeted attempts. Provisional until M5
/// measures real runs (U8).
pub(crate) const FETCH_TIMEOUT_SECONDS: u64 = 60;

/// Provisional `[jobs.runner]` default wall-clock ceiling for one child
/// process: minutes of headroom for a data program, tight enough that a
/// wedged child cannot hold a step open for an hour. Provisional until M5
/// measures real runs (U8).
pub(crate) const RUNNER_TIMEOUT_SECONDS: u64 = 300;

/// Effective download budget defaults, resolved from `[jobs.fetch]`.
/// Always concrete: a run with nothing declared is bounded by these
/// conservative defaults rather than unbounded — the point of the download
/// budget is that it exists without being asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFetchJobs {
    /// Per-file download ceiling, in bytes.
    pub max_file_bytes: u64,
    /// Ceiling on total download bytes across the whole run.
    pub max_run_bytes: u64,
    /// Wall-clock budget for one download request, in seconds.
    pub timeout_seconds: u64,
}

impl Default for ResolvedFetchJobs {
    /// The conservative defaults — the same numbers an absent
    /// `[jobs.fetch]` resolves to.
    fn default() -> Self {
        Self {
            max_file_bytes: FETCH_MAX_FILE_BYTES,
            max_run_bytes: FETCH_MAX_RUN_BYTES,
            timeout_seconds: FETCH_TIMEOUT_SECONDS,
        }
    }
}

/// Effective run budget defaults, resolved from `[jobs]` (and `[run]
/// max_iterations` for the turn ceiling) plus safe defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedJobs {
    /// Wall-clock ceiling for a run, in seconds. `None` is the resolved
    /// default: nothing limits a run's wall clock until something declares
    /// it — the engine pauses on a declared ceiling, never overruns.
    pub wall_clock_seconds: Option<u64>,
    /// Per-endpoint token ceilings keyed by run-scoped endpoint name, the
    /// same shape the run contracts carry. Empty when nothing is declared.
    pub tokens_per_endpoint: BTreeMap<String, u64>,
    /// Turn ceiling for a run's episodes: `[jobs] turns` when declared,
    /// otherwise `[run] max_iterations` — that knob's first behavioural
    /// reader, and the only place it is consumed. Always concrete: a run
    /// with nothing declared is bounded by the `max_iterations` default
    /// rather than unlimited, which is the point of wiring it (plan G2).
    pub turns: u64,
    /// Ceiling on total tool calls across a run's episodes. `None` by
    /// default, with the same pausing semantics as the wall clock.
    pub tool_calls: Option<u64>,
    /// Download budget defaults for `http_download` (M3-3). Always
    /// concrete: a run with nothing declared is bounded by the conservative
    /// defaults.
    pub fetch: ResolvedFetchJobs,
    /// Runner defaults for `run_program` (M5-4): the program universe a
    /// run's approved runner scope may draw from (empty when nothing is
    /// declared — no default program universe exists) and the default
    /// per-process wall-clock ceiling. Always concrete.
    pub runner: ResolvedRunnerJobs,
    /// Interpreter defaults for the `--allow interpreter:<program>` family:
    /// the universe a run's approved interpreter scope may draw from (empty
    /// when nothing is declared — the run has no interpreter capability,
    /// whatever `--allow` says). Always concrete.
    pub interpreter: ResolvedInterpreterJobs,
}

/// Effective runner defaults, resolved from `[jobs.runner]`. `allow` empty
/// means the run has no runner capability — there is no program universe a
/// run gets for free; approving programs is a deliberate act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRunnerJobs {
    /// The programs a run's approved runner scope may name. Bare names in
    /// the run-scoped name shape; shells and interpreters were refused at
    /// resolve time, so a name here is one the runner can actually honour.
    pub allow: Vec<String>,
    /// The absolute path of the operator-owned directory the allowlisted
    /// programs are staged in. `None` when undeclared: a run that approved
    /// the runner then fails closed at assemble — the resolve never checks
    /// existence, so a dangling path cannot break `saya ask`.
    pub program_dir: Option<std::path::PathBuf>,
    /// Default wall-clock ceiling for one child process, in seconds.
    pub timeout_seconds: u64,
}

impl Default for ResolvedRunnerJobs {
    /// The conservative defaults — the same values an absent `[jobs.runner]`
    /// resolves to: no programs approved, no directory, the conservative
    /// timeout.
    fn default() -> Self {
        Self {
            allow: Vec::new(),
            program_dir: None,
            timeout_seconds: RUNNER_TIMEOUT_SECONDS,
        }
    }
}

/// Effective interpreter defaults, resolved from `[jobs.interpreter]`.
/// `allow` empty means the run has no interpreter capability — there is no
/// interpreter a run gets for free; approving one is a deliberate act, and
/// the bytes that answer the approved name are staged by the trusted layers
/// into the runner's one program directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInterpreterJobs {
    /// The interpreters a run's approved interpreter scope may name. Bare
    /// names on the runner's refusal list — the family's own mirror, checked
    /// at resolve time.
    pub allow: Vec<String>,
}

impl Default for ResolvedInterpreterJobs {
    /// The conservative default — the same value an absent
    /// `[jobs.interpreter]` resolves to: no interpreter approved.
    fn default() -> Self {
        Self { allow: Vec::new() }
    }
}

impl ResolvedJobs {
    /// The resolved defaults as the run contract's budget shape — the type
    /// the engine layers RunSpec and step budgets over, per dimension. The
    /// result always satisfies the contract's own validation.
    pub fn budgets(&self) -> Budgets {
        let mut budgets = Budgets::default();
        budgets.wall_clock = self.wall_clock_seconds.map(Duration::from_secs);
        budgets.tokens_per_endpoint = self.tokens_per_endpoint.clone();
        budgets.turns = Some(self.turns);
        budgets.tool_calls = self.tool_calls;
        budgets
    }
}

/// Resolves `[jobs]` against the resolved `[run] max_iterations`, which
/// remains the run-episode default turn ceiling when `[jobs] turns` is
/// undeclared. Numeric bounds are checked here, at resolve time, with typed
/// errors — never silently clamped at the point of use.
pub(crate) fn resolve(file: &JobsFile, max_iterations: u64) -> Result<ResolvedJobs, ConfigError> {
    if let Some(seconds) = file.wall_clock_seconds {
        require_at_least_one("wall_clock_seconds", seconds)?;
    }
    let tokens_per_endpoint = match &file.tokens_per_endpoint {
        Some(map) => resolve_token_map(map)?,
        None => BTreeMap::new(),
    };
    let turns = file.turns.unwrap_or(max_iterations);
    require_at_least_one("turns", turns)?;
    if let Some(tool_calls) = file.tool_calls {
        require_at_least_one("tool_calls", tool_calls)?;
    }
    let fetch = resolve_fetch(file.fetch.as_ref())?;
    let runner = resolve_runner(file.runner.as_ref())?;
    let interpreter = resolve_interpreter(file.interpreter.as_ref(), runner.program_dir.as_ref())?;
    Ok(ResolvedJobs {
        wall_clock_seconds: file.wall_clock_seconds,
        tokens_per_endpoint,
        turns,
        tool_calls: file.tool_calls,
        fetch,
        runner,
        interpreter,
    })
}

/// Resolves `[jobs.fetch]`: each declared key is checked against the
/// minimum discipline at resolve time, with typed errors — never silently
/// clamped at the point of use. Every key is optional; the defaults are the
/// provisional conservative numbers documented above.
fn resolve_fetch(fetch: Option<&FetchJobsFile>) -> Result<ResolvedFetchJobs, ConfigError> {
    let fetch = fetch.cloned().unwrap_or_default();
    let max_file_bytes = fetch.max_file_bytes.unwrap_or(FETCH_MAX_FILE_BYTES);
    require_at_least_one("fetch.max_file_bytes", max_file_bytes)?;
    let max_run_bytes = fetch.max_run_bytes.unwrap_or(FETCH_MAX_RUN_BYTES);
    require_at_least_one("fetch.max_run_bytes", max_run_bytes)?;
    let timeout_seconds = fetch.timeout_seconds.unwrap_or(FETCH_TIMEOUT_SECONDS);
    require_at_least_one("fetch.timeout_seconds", timeout_seconds)?;
    Ok(ResolvedFetchJobs {
        max_file_bytes,
        max_run_bytes,
        timeout_seconds,
    })
}

/// Resolves `[jobs.runner]`: each declared key is checked at resolve time
/// with typed errors — never silently clamped at the point of use. Program
/// names must have the run-scoped name shape, must not repeat, must stay
/// within the contract's program-count bound, and must never name a shell or
/// interpreter — the runner refuses those structurally, so an allowlist that
/// carried one would approve a capability that cannot exist. The program
/// directory must be absolute (the canonical form must not depend on the
/// working directory the config was loaded from), and an `allow` that names
/// programs requires it.
fn resolve_runner(runner: Option<&RunnerJobsFile>) -> Result<ResolvedRunnerJobs, ConfigError> {
    let runner = runner.cloned().unwrap_or_default();
    let program_dir = match runner.program_dir {
        Some(dir) if !dir.is_absolute() => {
            return Err(ConfigError::RelativeRunnerProgramDir {
                path: dir.display().to_string(),
            });
        }
        other => other,
    };
    let mut allow = Vec::new();
    if let Some(programs) = runner.allow {
        if programs.len() > MAX_RUNNER_PROGRAMS {
            return Err(ConfigError::SettingAboveMaximum {
                field: "runner.allow",
                value: programs.len(),
                max: MAX_RUNNER_PROGRAMS,
            });
        }
        for program in programs {
            if !is_bare_name(&program) {
                return Err(ConfigError::InvalidRunnerProgram {
                    field: "runner.allow",
                    program: program.clone(),
                    reason: "a program entry is a bare name, never a path or traversal",
                });
            }
            if is_refused_runner_program(&program) {
                return Err(ConfigError::InvalidRunnerProgram {
                    field: "runner.allow",
                    program: program.clone(),
                    reason: "shells and interpreters are refused: the runner runs one \
                             allowlisted program with typed argv, and an interpreter would \
                             spawn arbitrary children from inside the allowlist",
                });
            }
            if allow.contains(&program) {
                return Err(ConfigError::InvalidRunnerProgram {
                    field: "runner.allow",
                    program: program.clone(),
                    reason: "declared more than once",
                });
            }
            allow.push(program);
        }
    }
    if !allow.is_empty() && program_dir.is_none() {
        return Err(ConfigError::RunnerAllowWithoutProgramDir);
    }
    let timeout_seconds = runner.timeout_seconds.unwrap_or(RUNNER_TIMEOUT_SECONDS);
    require_at_least_one("runner.timeout_seconds", timeout_seconds)?;
    Ok(ResolvedRunnerJobs {
        allow,
        program_dir,
        timeout_seconds,
    })
}

/// Resolves `[jobs.interpreter]`, mirroring `resolve_runner` exactly: each
/// declared key is checked at resolve time with typed errors — never
/// silently clamped at the point of use. Program names must have the
/// run-scoped name shape, must not repeat, must stay within the contract's
/// program-count bound, and must BE a shell or interpreter the runner
/// refuses — the family is the refusal list, so a member the runner would
/// run is a typed resolve error pointing at `[jobs.runner] allow`, and the
/// two universes stay disjoint by construction. An `allow` naming
/// interpreters requires `[jobs.runner] program_dir`: the interpreters are
/// staged in that one directory, and staging input is trusted-layer
/// business.
fn resolve_interpreter(
    interpreter: Option<&InterpreterJobsFile>,
    program_dir: Option<&std::path::PathBuf>,
) -> Result<ResolvedInterpreterJobs, ConfigError> {
    let interpreter = interpreter.cloned().unwrap_or_default();
    let mut allow = Vec::new();
    if let Some(programs) = interpreter.allow {
        if programs.len() > MAX_RUNNER_PROGRAMS {
            return Err(ConfigError::SettingAboveMaximum {
                field: "interpreter.allow",
                value: programs.len(),
                max: MAX_RUNNER_PROGRAMS,
            });
        }
        for program in programs {
            if !is_bare_name(&program) {
                return Err(ConfigError::InvalidInterpreterProgram {
                    field: "interpreter.allow",
                    program: program.clone(),
                    reason: "an interpreter entry is a bare name, never a path or traversal",
                });
            }
            if !is_refused_runner_program(&program) {
                return Err(ConfigError::InvalidInterpreterProgram {
                    field: "interpreter.allow",
                    program: program.clone(),
                    reason: "not a shell or interpreter the runner refuses — the interpreter \
                             family is the refusal list; declare programs the runner can run \
                             in [jobs.runner] allow",
                });
            }
            if allow.contains(&program) {
                return Err(ConfigError::InvalidInterpreterProgram {
                    field: "interpreter.allow",
                    program: program.clone(),
                    reason: "declared more than once",
                });
            }
            allow.push(program);
        }
    }
    if !allow.is_empty() && program_dir.is_none() {
        return Err(ConfigError::InterpreterAllowWithoutProgramDir);
    }
    Ok(ResolvedInterpreterJobs { allow })
}

/// Rejects a `[run] max_iterations` of zero. It is now the run-episode
/// default turn ceiling, so zero would pause every run before its first
/// turn — a typo, not an intent. There is no upper bound: the value is the
/// fallback ceiling, not a cost multiplier.
pub(crate) fn require_max_iterations(value: usize) -> Result<(), ConfigError> {
    require_at_least_one("max_iterations", value as u64)
}

fn require_at_least_one(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value >= MIN_BUDGET {
        Ok(())
    } else {
        Err(ConfigError::SettingBelowMinimum {
            field,
            value: value as usize,
            min: MIN_BUDGET as usize,
        })
    }
}

/// Validates the token map against the shape the run contracts carry, so a
/// config the contract would reject at plan-validation time is caught at
/// resolve time instead. The endpoint-count bound is the contract's own
/// (`MAX_BUDGET_ENDPOINTS`), and keys must have the run-scoped name shape.
fn resolve_token_map(map: &BTreeMap<String, u64>) -> Result<BTreeMap<String, u64>, ConfigError> {
    if map.len() > MAX_BUDGET_ENDPOINTS {
        return Err(ConfigError::SettingAboveMaximum {
            field: "tokens_per_endpoint",
            value: map.len(),
            max: MAX_BUDGET_ENDPOINTS,
        });
    }
    for (endpoint, tokens) in map {
        if !is_name_shaped(endpoint) {
            return Err(ConfigError::InvalidEndpointName {
                field: "tokens_per_endpoint",
                key: endpoint.clone(),
            });
        }
        require_at_least_one("tokens_per_endpoint", *tokens)?;
    }
    Ok(map.clone())
}
