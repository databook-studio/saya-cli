//! The panel's facts: what `/status` shows, gathered from what the session
//! already holds.

use std::path::Path;

use saya_store::{FsSessionStore, StoreError};

use crate::config::runtime::RuntimeConfig;
use crate::interactive::SessionUsage;
use crate::interactive::session::state::SessionState;

use super::ops_status::{Note, Section, StatusPanel, with_note};

/// The facts the panel shows, gathered from what the session already holds.
///
/// Every field is optional-by-line: an absent fact renders as the reason it is
/// absent, never as a zero and never as an invention.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StatusFacts {
    /// The session's id.
    pub(crate) session_id: String,
    /// How full the saved record is, already worded; `None` when no record has
    /// been measured.
    pub(crate) record: Option<String>,
    /// The run this session belongs to, already worded; `None` when the
    /// session drives no run.
    pub(crate) run: Option<String>,
    /// The provider in force.
    pub(crate) provider: String,
    /// The model in force.
    pub(crate) model: String,
    /// How much of the model's window the last answering call used, already
    /// worded; `None` when no call has reported one.
    pub(crate) context_use: Option<String>,
    /// This session's token totals, as `/usage` rendered them.
    pub(crate) tokens: String,
    /// What the provider reported this session has cost so far, as money;
    /// `None` when no call reported a cost.
    pub(crate) cost: Option<String>,
    /// The pinned workspace root.
    pub(crate) workspace_root: Option<String>,
    /// Whether the host-command lane composed for this session.
    pub(crate) host_lane: bool,
    /// The instruction sources the next request would load, as `/context`
    /// renders them.
    pub(crate) instructions: Option<String>,
    /// The run's `[limits.job]` guards, one line each.
    pub(crate) limits: Vec<String>,
    /// The database budget's lines; empty unless a database is bound.
    pub(crate) database_budget: Vec<String>,
}

impl StatusFacts {
    /// The facts the session itself holds: no store, no runtime, no probe.
    pub(crate) fn of_session(state: &SessionState) -> Self {
        Self {
            session_id: state.id.clone(),
            run: state.run.as_ref().map(|link| format!("{link:?}")),
            provider: state.provider.clone(),
            model: state.model.clone(),
            tokens: state.usage.render(),
            workspace_root: state.workspace_root.clone(),
            host_lane: state.host_composed,
            ..Self::default()
        }
    }

    /// The three facts only the runtime and the store know: how full the saved
    /// record is, the instruction sources the next request would load, and the
    /// ceilings in force. Reading them is a file and a metadata call — never a
    /// provider, a database, or the network.
    pub(crate) fn with_runtime(
        mut self,
        state: &SessionState,
        runtime: &RuntimeConfig,
        store: &FsSessionStore,
        workspace_root: Option<&Path>,
    ) -> Self {
        self.record = record_size(store, &state.id);
        // The session's live root is what the file tools are contained to, so
        // it is what the panel states; the record's binding is the fallback
        // for a session that has not acquired one.
        if self.workspace_root.is_none() {
            self.workspace_root = workspace_root.map(|path| path.display().to_string());
        }
        self.instructions = Some(instructions(state, runtime, workspace_root));
        self.context_use = context_use(&state.usage, super::ops_usage::window_tokens(runtime));
        self.cost = cost_fact(&state.usage);
        self.limits = crate::config::limits_report::job_guards(runtime);
        // The database budget is the plugin's, and the plugin only arms it on a
        // session that bound a database: with no profile there is no budget to
        // state, so the line is absent rather than zero.
        self.database_budget = if database_bound(state) {
            crate::config::limits_report::database_guards(runtime)
        } else {
            Vec::new()
        };
        self
    }
}

/// The panel as both surfaces print it: one string, five sections.
pub(crate) fn panel_text(
    state: &SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    workspace_root: Option<&Path>,
) -> String {
    StatusPanel::build(&facts(state, runtime, store, workspace_root)).render()
}

/// `/context`'s answer: the panel's Instructions section, plus the alias's
/// note the first time this session says it.
pub(crate) fn instructions_answer(
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    workspace_root: Option<&Path>,
) -> String {
    let section = StatusPanel::build(&StatusFacts {
        instructions: Some(instructions(state, runtime, workspace_root)),
        ..StatusFacts::of_session(state)
    })
    .render_section(Section::Instructions);
    with_note(state, Note::Context, section)
}

/// The facts one surface can give: the session's, plus the runtime's three.
fn facts(
    state: &SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    workspace_root: Option<&Path>,
) -> StatusFacts {
    StatusFacts::of_session(state).with_runtime(state, runtime, store, workspace_root)
}

/// The instruction-source view `/context` has always printed, read from the
/// runtime's settings and the session's workspace root.
fn instructions(
    _state: &SessionState,
    runtime: &RuntimeConfig,
    workspace_root: Option<&Path>,
) -> String {
    crate::interactive::session::context_view::context_view(runtime, workspace_root)
}

/// Whether this session bound a database — the database plugin's own
/// condition for arming `[limits.database]`.
fn database_bound(state: &SessionState) -> bool {
    state.profile.is_some() || !state.included_profiles.is_empty()
}

/// The Model section's context-use fact: the share of the model's window the
/// last answering request used, or `None` when the window is unknown or no
/// request has reported one. An unknown window is stated as unknown, never
/// inferred from a token count.
pub(crate) fn context_use(usage: &SessionUsage, window: Option<u64>) -> Option<String> {
    let window = window?;
    let percent = saya_agent::context_utilisation_percent(usage.last_input_tokens(), Some(window))?;
    Some(format!("{percent}% of a {window}-token window"))
}

/// The Model section's cost fact: what the provider reported this session has
/// cost, or `None` when no call reported a cost.
pub(crate) fn cost_fact(usage: &SessionUsage) -> Option<String> {
    usage.cost_micros().map(crate::interactive::cost_text)
}

/// How full the session's saved record is. A store that cannot answer says so
/// rather than refusing the panel: `/status` never fails a turn.
fn record_size(store: &FsSessionStore, id: &str) -> Option<String> {
    let capacity = match store.stored_capacity(id) {
        Ok(Some(capacity)) => capacity,
        Ok(None) | Err(StoreError::NotFound) => return Some("no saved record yet".to_owned()),
        Err(_) => return None,
    };
    let mib = capacity.used_bytes as f64 / (1024.0 * 1024.0);
    let cap = capacity.max_bytes / (1024 * 1024);
    Some(format!("{mib:.1} MiB of the {cap} MiB limit"))
}
