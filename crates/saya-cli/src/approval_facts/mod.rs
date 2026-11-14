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

mod command_file;
mod fetch_tools;
mod plan;
mod run_command;
mod scope;
mod session_line;
mod session_tools;
mod shell;
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
    /// The fetch member's bounds and the shared download wallet.
    pub(crate) fetch: Option<FetchFacts>,
    /// The bound workspace root, where one binds.
    pub(crate) workspace_root: Option<PathBuf>,
    /// The host-command route's facts, when the route is composed: a
    /// workspace root bound plus a launch-or-user-layer statement. `None`
    /// contributes no lines and parses no `command:` token.
    pub(crate) host: Option<HostFacts>,
    /// The session's deny list: bare program names every door refuses
    /// before grant, prompt, and bypass — session-wide, route-blind. Empty
    /// refuses nothing. Present even when the host route is off: deny gates
    /// the doors every session already has.
    pub(crate) denied_programs: Vec<String>,
    /// The session state directory where an over-bound command's exact bytes
    /// are saved for the operator, when this decider belongs to a session.
    /// The deciders fill it from the journal they already hold; `None` — a
    /// headless or one-shot decider — makes the prompt show the whole command
    /// rather than hide a tail it cannot save.
    pub(crate) approval_dir: Option<PathBuf>,
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

    /// Binds the session state directory an over-bound command's exact bytes
    /// are saved under. The deciders call this with the journal's directory;
    /// every other construction leaves it `None`.
    pub(crate) fn with_approval_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.approval_dir = dir;
        self
    }
}

/// The fetch member's facts: the tool route bounds the composed member runs
/// under, and the shared download wallet (by clone, so the remaining budget
/// is read live at ask time).
#[derive(Clone)]
pub(crate) struct FetchFacts {
    pub(crate) fetch_body_bytes: usize,
    pub(crate) fetch_seconds: u64,
    pub(crate) fetch_redirects: usize,
    pub(crate) download: Option<saya_harness::fetch::DownloadBudget>,
}

/// The host-command route's composition facts: what the route's own
/// prompt may state, and what `/allow command:<x>` consults. The route is
/// off unless stated at launch or in the user layer, and it never composes
/// without a bound workspace root.
///
/// The composition gate and the universe read these facts. The prompt body
/// that states the per-call fields (`timeout_seconds`, `pass_env`) is not
/// wired yet.
#[derive(Clone)]
pub(crate) struct HostFacts {
    /// The workspace root the child runs with as its cwd.
    pub(crate) workspace_root: PathBuf,
    /// The per-call timeout ceiling, in seconds — a call may narrow it,
    /// never widen it. Stored at composition; the prompt body reads it when
    /// wired.
    #[cfg_attr(not(test), allow(dead_code))]
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) timeout_seconds: u64,
    /// The parent variable names the built child environment carries. Stored
    /// at composition; the prompt body reads it when wired.
    #[cfg_attr(not(test), allow(dead_code))]
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) pass_env: Vec<String>,
}

impl HostFacts {
    /// The test seam: a composed route over a fixed root, the executor's own
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
        "run_command" => run_command::facts(arguments, facts, session_line, grant),
        "shell" => shell::facts(arguments, facts, session_line),
        "write" => session_tools::workspace_write_facts(arguments, facts, session_line, grant),
        "edit" => session_tools::workspace_edit_facts(arguments, facts, session_line, grant),
        "http_fetch" | "http_download" => {
            fetch_tools::facts(name, arguments, grant, facts, session_line)
        }
        "submit_plan" => plan::facts(arguments),
        _ => None,
    }
}

/// Whether the answer to an ask is still bound to the bytes the operator was
/// shown: for an over-bound shell command, the private file must hold exactly
/// the command that will run. The file is removed either way — the answer,
/// allow or deny, ends the pending approval. `true` where no over-bound
/// command was shown.
pub(crate) fn answer_binding_holds(name: &str, arguments: &Value, facts: &ApprovalFacts) -> bool {
    shell::answer_binding_holds(name, arguments, facts)
}

/// Removes every saved approval command under a session state directory. The
/// session's end sweeps what an abandoned ask left; an answered ask removes
/// its own file as it answers.
pub(crate) fn remove_saved_commands(state_dir: &std::path::Path) {
    command_file::remove_all(state_dir);
}

/// The saved approval commands under `state_dir`, for the tests that pin the
/// one-file lifecycle.
#[cfg(test)]
pub(crate) fn saved_commands(state_dir: &std::path::Path) -> Vec<PathBuf> {
    command_file::saved(state_dir)
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
