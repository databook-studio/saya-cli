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
mod run_program;
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
        "run_program" => run_program::facts(arguments, facts, session_line),
        "workspace_write" => session_tools::workspace_write_facts(arguments, facts, session_line),
        "scratch_sql" => session_tools::scratch_facts(arguments, facts, session_line),
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
