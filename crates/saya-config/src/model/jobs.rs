use super::*;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFile {
    pub read_only: Option<bool>,
    pub max_rows: Option<usize>,
    pub max_iterations: Option<usize>,
    /// Independent agent attempts per question. The resolved default is `1`,
    /// which is today's single-run behaviour — a user who sets nothing changes
    /// nothing. Each additional candidate is another full agent run (model
    /// calls and database queries), so this multiplies cost roughly linearly.
    /// Bounded at resolve time; the selection logic that consumes it is a
    /// separate task and nothing reads this yet.
    pub candidates: Option<usize>,
    pub query_timeout_seconds: Option<u64>,
}

/// The `[jobs]` section: the default budgets a *run* is declared with when
/// the run's specification and each of its steps declare none. `ConfigFile`
/// is `deny_unknown_fields`, so this section must be declared here before
/// any config may carry it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobsFile {
    /// Default wall-clock ceiling for a run, in seconds. Absent: a run has
    /// no wall-clock ceiling until the run itself declares one.
    pub wall_clock_seconds: Option<u64>,
    /// Default token ceilings keyed by run-scoped endpoint name — the same
    /// per-endpoint shape the run contracts carry. Absent or empty: no
    /// endpoint has a token ceiling.
    pub tokens_per_endpoint: Option<BTreeMap<String, u64>>,
    /// Default turn ceiling for a run's episodes. Absent: the run has no
    /// turn ceiling until the run itself declares one — a ceiling left
    /// unset is unlimited at the contract level.
    pub turns: Option<u64>,
    /// Default ceiling on total tool calls across a run's episodes. Absent:
    /// no ceiling.
    pub tool_calls: Option<u64>,
    /// Default download budgets for the `http_download` tool, as the
    /// `[jobs.fetch]` sub-table (M3-3): per-file bytes, total run download
    /// bytes, and the per-request timeout. Absent: the harness's
    /// conservative defaults. There is deliberately no environment override
    /// for any of it (plan G3).
    pub fetch: Option<FetchJobsFile>,
    /// The `[jobs.runner]` sub-table (M5-4): the universe of programs a run's
    /// approved runner scope may name, and the default per-process timeout.
    /// Absent: no runner programs are approved and the conservative timeout
    /// default applies. There is deliberately no environment override for
    /// any of it (plan G3).
    pub runner: Option<RunnerJobsFile>,
    /// The `[jobs.interpreter]` sub-table: the universe of interpreters a
    /// run's approved interpreter scope may name (`--allow
    /// interpreter:<program>`). Absent or empty: the run has no interpreter
    /// capability, whatever `--allow` says — approving an interpreter is a
    /// deliberate act, the same rule `[jobs.runner] allow` already follows.
    /// The whole sub-table is on the protected list, so an untrusted project
    /// layer cannot declare it (the staging inputs decide which bytes answer
    /// an approved name). There is deliberately no environment override for
    /// any of it (plan G3), and no timeout key: an interpreter child is one
    /// child process, so `[jobs.runner] timeout_seconds` is its ceiling — a
    /// per-family timeout would be a second knob for one fact.
    pub interpreter: Option<InterpreterJobsFile>,
}

/// The `[jobs.runner]` sub-table: the runner programs a run's approved
/// runner scope may draw from, the one directory they are staged in, and the
/// default wall-clock ceiling for one child process (M5-4). Each key is
/// optional: an absent `allow` approves no programs (the runner capability
/// is absent entirely — there is no default program universe a run gets for
/// free), and an absent `timeout_seconds` resolves to the conservative
/// default. Entries are bare program names in the run-scoped name shape;
/// shells and interpreters are refused at resolve time — a program the
/// runner will never honour must not look approved. The directory is
/// operator-owned and staged before the run: the engine only ever reads it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerJobsFile {
    /// The programs a run's runner scope may name. Absent or empty: the run
    /// has no runner capability. Each entry is validated against the same
    /// name shape the run contracts carry, bounded like every set-valued
    /// approval surface, and refused when it names a shell or interpreter.
    pub allow: Option<Vec<String>>,
    /// The absolute path of the directory the allowlisted programs are
    /// staged in. An `allow` that names programs requires it — programs are
    /// resolved inside one directory and nowhere else — while a `program_dir`
    /// alone (empty `allow`) is harmless. It must be absolute: the canonical
    /// form must not depend on the working directory the config was loaded
    /// from. Existence is deliberately not checked at resolve time — a
    /// dangling path must not break `saya ask`; a run that approved the
    /// runner fails closed at assemble instead. The engine never writes the
    /// directory, at claim or at any other point in a run.
    pub program_dir: Option<PathBuf>,
    /// Default wall-clock ceiling for one child process, in seconds. A
    /// declared zero is a typed resolve error, never a silent clamp — a
    /// zero-second timeout would kill every child before its first byte, a
    /// typo, not an intent. Provisional until M5 measures real runs (U8).
    pub timeout_seconds: Option<u64>,
}

/// The `[jobs.interpreter]` sub-table: the interpreters a run's approved
/// interpreter scope may draw from. Entries are bare program names that
/// MUST be on the runner's refusal list — the interpreter family is that
/// list, mirrored at resolve time, so a member the runner does not refuse
/// is a typed resolve error pointing at `[jobs.runner] allow`. The two
/// universes stay disjoint by construction, not by convention. The sub-table
/// is protected (see `layers.rs`): it decides which bytes answer an
/// approved interpreter's name, so only the trusted layers may declare it.
/// Its bytes are staged in the runner's one program directory
/// (`[jobs.runner] program_dir`), so an `allow` naming interpreters
/// requires that key.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpreterJobsFile {
    /// The interpreters a run's interpreter scope may name. Absent or
    /// empty: the run has no interpreter capability. Each entry is
    /// validated against the same name shape the run contracts carry,
    /// bounded like every set-valued approval surface, and required to be
    /// a name the runner refuses — the family's own mirror.
    pub allow: Option<Vec<String>>,
}

/// The `[host_commands]` section: the unsandboxed second lane's own
/// shaping. `pass_env` names parent variables the built child environment
/// carries; `timeout_seconds` is the per-call ceiling a call may narrow,
/// never widen. Neither shapes whether the lane composes — it composes
/// wherever a workspace root binds — so there is no `enable` key: unknown
/// fields (a stale `enable = true` included) refuse at parse.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HostCommandsFile {
    /// Parent variables the child receives, by name.
    #[serde(default)]
    pub pass_env: Vec<String>,
    /// Per-call ceiling in seconds. `None` resolves to the executor default.
    pub timeout_seconds: Option<u64>,
}

/// The `[session_commands]` section: the user-stated deny list of bare
/// program names. Refusal-only: it composes nothing and gates the doors
/// every session already has, even with the host lane off. Entries are bare
/// names — never paths, traversals, prefixes, or globs — checked at resolve.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionCommandsFile {
    /// The denied programs, by bare name. Absent or empty: nothing denied.
    #[serde(default)]
    pub deny: Vec<String>,
}

/// The `[jobs.fetch]` sub-table: the download budgets a run's
/// `http_download` calls spend from. Each key is optional and resolved with
/// a conservative default; a declared zero is a typed resolve error, never a
/// silent clamp (the same discipline as the sibling `[jobs]` budgets — a
/// zero here would pause every download before its first byte, a typo, not
/// an intent).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchJobsFile {
    /// Per-file download ceiling, in bytes. Absent: the conservative
    /// default. Provisional until M5 measures real runs (U8).
    pub max_file_bytes: Option<u64>,
    /// Ceiling on total download bytes across the whole run. Absent: the
    /// conservative default. Provisional until M5 measures real runs (U8).
    pub max_run_bytes: Option<u64>,
    /// Wall-clock budget for one download request, in seconds. With
    /// resumable partials a longer transfer is a sequence of budgeted
    /// attempts. Absent: the conservative default. Provisional until M5
    /// measures real runs (U8).
    pub timeout_seconds: Option<u64>,
}
