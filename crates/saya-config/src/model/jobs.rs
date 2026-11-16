use super::*;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFile {
    pub read_only: Option<bool>,
    pub max_rows: Option<usize>,
    /// Independent agent attempts per question. The resolved default is `1`,
    /// which is the single-run behaviour — a user who sets nothing changes
    /// nothing. Each additional candidate is another full agent run (model
    /// calls and database queries), so this multiplies cost roughly linearly.
    /// Bounded at resolve time; the selection logic that consumes it is a
    /// separate task and nothing reads this yet.
    pub candidates: Option<usize>,
    pub query_timeout_seconds: Option<u64>,
}

/// The `[jobs]` section. Its four run ceilings live in `[limits.run]` — one
/// vocabulary with every other surface's — leaving the download budgets a
/// run's `http_download` calls spend from. `ConfigFile` is
/// `deny_unknown_fields`, so this section must be declared here before any
/// config may carry it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobsFile {
    /// Default download budgets for the `http_download` tool, as the
    /// `[jobs.fetch]` sub-table: per-file bytes, total run download
    /// bytes, and the per-request timeout. Absent: the harness's
    /// conservative defaults. There is deliberately no environment override
    /// for any of it. A download budget is not a run ceiling, so it
    /// stays here rather than moving to `[limits.run]`.
    pub fetch: Option<FetchJobsFile>,
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
    /// default. Provisional until real runs are measured.
    pub max_file_bytes: Option<u64>,
    /// Ceiling on total download bytes across the whole run. Absent: the
    /// conservative default. Provisional until real runs are measured.
    pub max_run_bytes: Option<u64>,
    /// Wall-clock budget for one download request, in seconds. With
    /// resumable partials a longer transfer is a sequence of budgeted
    /// attempts. Absent: the conservative default. Provisional until real runs
    /// are measured.
    pub timeout_seconds: Option<u64>,
}
