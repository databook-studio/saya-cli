//! `/permissions`: one read-only view of what this session may do.
//!
//! The view states exactly five facts — the mode in force, the grant store,
//! the deny list, whether the host lane composed, and data sharing — each one
//! the session already holds or a runtime read with no probe: no provider
//! call, no database query, no network. `/doctor` stays the command that
//! checks.
//!
//! `/grants` is the one-release alias of `/permissions`: it answers with the
//! same view, plus a one-line note the first time this session says it, so
//! the two spellings cannot disagree about the store.

use saya_agent::{ApprovalPolicy, SessionGrants};

use crate::interactive::session::commands::approval_name;
use crate::interactive::session::permissions::grants;
use crate::interactive::session::state::SessionState;
use crate::slash::PermissionsRequest;

use super::ops_status::{Note, with_note};

/// The five facts the view states, in the order it prints them.
pub(crate) const LABELS: [&str; 5] = ["Mode", "Grants", "Deny list", "Host lane", "Data sharing"];

/// What the view shows, gathered from what the session already holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PermissionsFacts {
    /// The mode the run's next call reads, in the engine's own words.
    pub(crate) mode: String,
    /// The grant store in the listing's own words — the record, verbatim.
    pub(crate) grants: String,
    /// The session's deny list: bare program names every door refuses.
    pub(crate) denied: Vec<String>,
    /// Whether the host-command lane composed for this session.
    pub(crate) host_lane: bool,
    /// Whether row values may be sent to the provider.
    pub(crate) sharing: bool,
}

impl PermissionsFacts {
    /// The five facts: the session's own, the engine's mode, the live grant
    /// store and the runtime's deny list. Nothing here probes anything.
    pub(crate) fn of_session(
        state: &SessionState,
        mode: ApprovalPolicy,
        store: &SessionGrants,
        denied: &[String],
    ) -> Self {
        Self {
            mode: approval_name(mode),
            grants: grants::listing(mode, store),
            denied: denied.to_vec(),
            host_lane: state.host_composed,
            sharing: state.allow_data_sharing,
        }
    }

    /// One fact, by its label, as it prints.
    pub(crate) fn fact(&self, label: &str) -> String {
        match label {
            "Mode" => self.mode.clone(),
            "Grants" => self.grants.clone(),
            "Deny list" => deny_words(&self.denied),
            "Host lane" => host_words(self.host_lane),
            "Data sharing" => sharing_words(self.sharing),
            other => panic!("/permissions states no fact named {other}"),
        }
    }
}

/// The view as both surfaces print it: exactly the five labelled facts, in
/// [`LABELS`] order. A one-line fact prints as `Label: value`; a fact that
/// carries several lines (the grant store) prints its label alone and nests
/// its own words under it, so the store stays the record verbatim.
pub(crate) fn panel_text(facts: &PermissionsFacts) -> String {
    let mut out = String::new();
    for label in LABELS {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(label);
        out.push(':');
        let value = facts.fact(label);
        let lines: Vec<&str> = value.lines().collect();
        if let [single] = lines.as_slice() {
            out.push(' ');
            out.push_str(single);
            continue;
        }
        for line in lines {
            out.push_str("\n  ");
            out.push_str(line);
        }
    }
    out
}

/// `/permissions` — the one view, and `/grants`'s spelling of it: the same
/// five facts, plus the alias's note the first time this session says it.
pub(crate) fn answer(
    state: &mut SessionState,
    request: PermissionsRequest,
    mode: ApprovalPolicy,
    store: &SessionGrants,
    denied: &[String],
) -> String {
    let text = panel_text(&PermissionsFacts::of_session(state, mode, store, denied));
    if request.via_grants_alias {
        with_note(state, Note::Grants, text)
    } else {
        text
    }
}

/// The deny list's words: the programs, or the fact that nothing is refused.
fn deny_words(denied: &[String]) -> String {
    if denied.is_empty() {
        "none — no --deny program refuses a call this session".to_owned()
    } else {
        denied.join(", ")
    }
}

/// The host lane's words: composed or not, never a bare boolean.
fn host_words(composed: bool) -> String {
    if composed {
        "composed".to_owned()
    } else {
        "not composed".to_owned()
    }
}

/// The data-sharing gate's words, matching the status bar's `sharing:`
/// segment: `on` means row values go to the provider, `off` means they do not.
fn sharing_words(sharing: bool) -> String {
    if sharing { "on" } else { "off" }.to_owned()
}

#[cfg(test)]
#[path = "ops_permissions_tests.rs"]
mod tests;
