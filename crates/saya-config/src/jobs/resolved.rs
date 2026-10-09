use super::*;

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

/// Effective run budget defaults, resolved from `[jobs]` plus safe defaults.
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
    /// unlimited when unset — a ceiling left unset is unlimited at the
    /// contract level, matching every other budget dimension.
    pub turns: Option<u64>,
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

/// The resolved session deny list, from user-layer `[session_commands]
/// deny`. Refusal-only: bare program names, sorted and deduped, that every
/// session door refuses before grant, prompt, and bypass. Empty when nothing
/// is denied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedSessionDeny {
    /// The denied programs, sorted, deduped.
    pub programs: Vec<String>,
}

/// Resolves `[session_commands] deny`: every entry is a bare name — never a
/// path, traversal, prefix, or glob — refused at resolve time with a typed
/// error. Sorted and deduped, so the journal's start event and the status
/// line carry one canonical order.
pub(crate) fn resolve_session_deny(
    file: crate::model::SessionCommandsFile,
) -> Result<ResolvedSessionDeny, ConfigError> {
    let mut programs: Vec<String> = file.deny;
    for program in &programs {
        if !is_bare_name(program) {
            return Err(ConfigError::InvalidSessionDeny {
                program: program.clone(),
                reason: "a deny entry is a bare program name, never a path, traversal, \
                     prefix, or glob",
            });
        }
    }
    programs.sort();
    programs.dedup();
    Ok(ResolvedSessionDeny { programs })
}

/// Effective host-command defaults, resolved from `[host_commands]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHostCommands {
    /// Parent variables the built child environment carries, by name.
    pub pass_env: Vec<String>,
    /// Per-call ceiling in seconds; a call may narrow it, never widen it.
    pub timeout_seconds: u64,
}

impl Default for ResolvedHostCommands {
    fn default() -> Self {
        Self {
            pass_env: Vec::new(),
            timeout_seconds: HOST_COMMANDS_TIMEOUT_SECONDS,
        }
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
        budgets.turns = self.turns;
        budgets.tool_calls = self.tool_calls;
        budgets
    }
}
