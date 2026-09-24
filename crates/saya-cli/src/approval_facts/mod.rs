//! The per-call approval facts: the risk lines both approval frontends
//! render — the terminal prompt (`prompt_approval`) and the TUI modal
//! (`interactive/tui/ui/panels`) — built by one builder so the two surfaces
//! cannot state different facts for the same call.
//!
//! The one rule: **a prompt may not claim a bound the code does not apply.**
//! Every fact line comes from the tool's own enforcement — the sandbox
//! policy, the scope, the row cap, the timeout, the containment seam — never
//! from prose. A fact the composition does not carry (`None` here) produces
//! no line at all, never a placeholder: the prompt states only what the call
//! demonstrates, and the number it states for a bound is the same value the
//! enforcement applies (imported from the enforcing constant or composed
//! member, not re-typed).

mod fetch_tools;
mod run_command;
mod run_program;
mod scope;
mod session_line;
mod session_tools;
mod sql_family;

use saya_agent::SessionGrants;
use serde_json::Value;
use std::path::PathBuf;

/// What the approval prompts may state about this surface's composition.
/// Built once per session (or per one-shot `ask`) from the members the
/// surface actually composed; every member is `None` where that capability
/// does not exist here, and a `None` member contributes no fact lines.
#[derive(Clone, Default)]
pub(crate) struct ApprovalFacts {
    /// The model-facing row cap one SQL call runs under —
    /// `state_tools::model_row_cap(resolved.max_rows)`.
    pub(crate) row_cap: usize,
    /// The resolved query timeout, in seconds, every connector applies.
    pub(crate) sql_timeout_seconds: u64,
    /// The session's runner composition, when the host proved.
    pub(crate) runner: Option<RunnerFacts>,
    /// The fetch member's bounds and the shared download wallet.
    pub(crate) fetch: Option<FetchFacts>,
    /// The scratch member's bounds, where scratch was composed.
    pub(crate) scratch: Option<ScratchFacts>,
    /// The bound workspace root, where one binds.
    pub(crate) workspace_root: Option<PathBuf>,
    /// The host-command lane's facts, when the lane composed (H1): a
    /// workspace root bound plus a launch-or-user-layer statement. `None`
    /// contributes no lines and parses no `command:` token.
    pub(crate) host: Option<HostFacts>,
    /// The session's host lane ran a command — the fact `run_program`'s
    /// prompt states (§3 rule 6): a prior host child can have rewritten a
    /// staged binary, so staged-binary integrity is outside saya's control.
    /// Set by the runtime after a host call settles; prompts read it.
    pub(crate) host_ran: bool,
    /// The session's deny list: bare program names every door refuses
    /// before grant, prompt, and bypass — session-wide, lane-blind. Empty
    /// refuses nothing. Present even when the host lane is off: deny gates
    /// the doors every session already has.
    pub(crate) denied_programs: Vec<String>,
}

impl ApprovalFacts {
    /// The one-shot `ask` shape: the SQL bounds from the resolved config,
    /// and no session members — `saya ask` advertises only the database
    /// surface, so no run-tool prompt can occur here.
    pub(crate) fn for_ask(runtime: &crate::config::runtime::RuntimeConfig) -> Self {
        Self {
            row_cap: crate::agent::state_tools::model_row_cap(runtime.resolved.max_rows),
            sql_timeout_seconds: runtime.resolved.query_timeout_seconds,
            ..ApprovalFacts::default()
        }
    }
}

/// The runner composition's facts, read off the composed spawn and scopes.
#[derive(Clone, Default)]
pub(crate) struct RunnerFacts {
    /// The spawn's filesystem roots (`fs_roots`), in composition order —
    /// the first is the child's pinned cwd.
    pub(crate) fs_roots: Vec<PathBuf>,
    /// The spawn's egress endpoints; empty is the fail-closed composition.
    pub(crate) net_allow: Vec<(String, u16)>,
    /// The configured per-child timeout ceiling, in seconds; a call may
    /// narrow it, never widen it.
    pub(crate) timeout_seconds: u64,
    /// The runner door's programs.
    pub(crate) runner_programs: Vec<String>,
    /// The interpreter door's programs, when the config staged any.
    pub(crate) interpreter_programs: Vec<String>,
    /// The credentials the composition declared for the child's environment.
    pub(crate) credentials_declared: usize,
}

/// The fetch member's facts: the tool-lane bounds the composed member runs
/// under, and the shared download wallet (by clone, so the remaining budget
/// is read live at ask time).
#[derive(Clone)]
pub(crate) struct FetchFacts {
    pub(crate) fetch_body_bytes: usize,
    pub(crate) fetch_seconds: u64,
    pub(crate) fetch_redirects: usize,
    pub(crate) download: Option<saya_harness::fetch::DownloadBudget>,
}

/// The scratch member's bounds — the harness's own enforcement constants,
/// which the session's scratch runs under (the session never narrows them).
#[derive(Clone, Copy)]
pub(crate) struct ScratchFacts {
    pub(crate) row_cap: usize,
    pub(crate) timeout_seconds: u64,
}

/// The host-command lane's composition facts (H1): what the lane's own
/// prompt may state, and what `/allow command:<x>` consults. The lane is
/// off unless stated at launch or in the user layer, and it never composes
/// without a bound workspace root.
///
/// H1 carries the facts the composition gate and the universe read; the
/// per-call prompt body that states `timeout_seconds` and `pass_env` lands
/// with H2 (facts slice), which is why no prompt code reads them yet.
#[derive(Clone)]
pub(crate) struct HostFacts {
    /// The workspace root the child runs with as its cwd.
    pub(crate) workspace_root: PathBuf,
    /// The per-call timeout ceiling, in seconds — a call may narrow it,
    /// never widen it. Read by H2's prompt body; H1 composes and stores it.
    #[cfg_attr(not(test), allow(dead_code))]
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) timeout_seconds: u64,
    /// The parent variable names the built child environment carries. Read
    /// by H2's prompt body; H1 composes and stores it.
    #[cfg_attr(not(test), allow(dead_code))]
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) pass_env: Vec<String>,
}

impl HostFacts {
    /// The test seam: a composed lane over a fixed root, the executor's own
    /// ceiling, and no passed variables.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self {
            workspace_root: PathBuf::from("/home/user/proj"),
            timeout_seconds: saya_harness::host::resolve::DEFAULT_HOST_TIMEOUT.as_secs(),
            pass_env: Vec::new(),
        }
    }
}

/// The per-call fact body both frontends render: the family's risk lines —
/// the containment that makes the call safe, the bounds that cap it, the
/// session's grant history — without the answers line (that line belongs to
/// `grant_token::session_answers_line`, which both frontends append). `None`
/// for a call with no facts worth showing: the frontends then render the
/// generic sentence naming the tool.
pub(crate) fn call_facts(
    name: &str,
    arguments: &Value,
    grant: Option<&str>,
    facts: &ApprovalFacts,
    primary: Option<&str>,
    grants: Option<&SessionGrants>,
) -> Option<String> {
    let session_line = session_line::session_history_line(grant, grants);
    match name {
        "bounded_sql_query"
        | "bounded_sql_query_all"
        | "result_shape"
        | "column_health"
        | "join_check" => sql_family::sql_facts(name, arguments, facts, primary, session_line),
        "render_chart" => sql_family::chart_facts(arguments, primary),
        "run_program" => run_program::facts(arguments, facts, session_line, grant),
        "run_command" => run_command::facts(arguments, facts, session_line, grant),
        "workspace_write" => {
            session_tools::workspace_write_facts(arguments, facts, session_line, grant)
        }
        "scratch_sql" => session_tools::scratch_facts(arguments, facts, session_line),
        "scratch_import" => session_tools::scratch_import_facts(arguments, facts, session_line),
        "http_fetch" | "http_download" => {
            fetch_tools::facts(name, arguments, grant, facts, session_line)
        }
        _ => None,
    }
}

/// Renders the fact lines under a family header; every line keeps the
/// two-space indent the design's bodies use.
pub(super) fn body(header: String, lines: Vec<String>) -> String {
    let mut text = header;
    for line in lines {
        text.push('\n');
        text.push_str(&line);
    }
    text
}
