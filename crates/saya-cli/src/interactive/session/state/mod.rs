use crate::interactive::session::universe::SessionUniverse;
use crate::interactive::tui::types::SessionUsage;
use saya_agent::{AgentOutput, ChatMessage, ToolMetadata};
use saya_store::{
    ParentLink, RedactedSession, RedactedToolMetadata, RedactedToolResultShape, RedactedTurn,
    SESSION_VERSION, StoredContinuation,
};
use serde::{Deserialize, Serialize};

// `continuation` holds the store's typed-or-unknown view (`serde_json::Value`
// behind `Unsupported`), so the state is `PartialEq` but not `Eq`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionState {
    pub id: String,
    pub profile: Option<String>,
    pub included_profiles: Vec<String>,
    pub provider: String,
    pub model: String,
    /// The provider endpoint selected for this session. Persisting it keeps
    /// resumed sessions under the same privacy classification as the session
    /// that created their history.
    #[serde(default)]
    pub provider_endpoint: Option<String>,
    /// Distinguishes an explicit endpoint clear from a legacy session with no
    /// endpoint field, so resume can safely fall back to current config only
    /// for the latter.
    #[serde(default)]
    pub provider_endpoint_bound: bool,
    pub allow_data_sharing: bool,
    pub approval_mode: String,
    /// The session's task posture (`AgentMode::as_str` spelling), beside
    /// `approval_mode`: `build` does the work, `plan` investigates read-only.
    /// Carried as the mode's own string so the persisted session file stays
    /// a plain string like `approval_mode`; parsed at the composition root.
    /// Defaults to Build; `#[serde(default)]` keeps old session files
    /// loading (they predate the field and resume as build).
    #[serde(default = "default_agent_mode")]
    pub agent_mode: String,
    /// The session's pinned workspace root, canonical (SESSION-WORKSPACE.md):
    /// resolved once at first start — the git worktree top, or an explicit
    /// `--workspace` — persisted here, and re-opened on a resume, never
    /// re-derived from wherever the shell happens to be. `None` binds no
    /// root: the write-shaped tools stay hidden and workspace reads refuse
    /// with their typed error, which is every session written before the
    /// workspace existed.
    #[serde(default)]
    pub workspace_root: Option<String>,
    /// The session's task list: conversation metadata tracking what the
    /// session is building, binding no authority. Modelled on `agent_mode`:
    /// an additive `#[serde(default)]` field, so a session file written
    /// before it existed resumes with an empty list and no `SESSION_VERSION`
    /// bump is owed.
    #[serde(default)]
    pub task_list: saya_types::SessionTaskList,
    /// The last live output's bounded tool outcomes. Runtime-only: a resumed
    /// session must treat historical free-form statuses as unknown.
    #[serde(skip, default)]
    pub(crate) prior_tool_outcomes:
        Option<crate::interactive::session::continuation::PriorToolOutcomes>,
    /// Bounded observations of what this session's tools actually did, this
    /// session: the raw material the continuation facts are built from.
    /// Runtime-only (`#[serde(skip)]`, the `prior_tool_outcomes` precedent):
    /// a resumed session starts with none — historical statuses never feed
    /// it — and `/clear` empties it with the conversation.
    #[serde(skip)]
    pub(crate) observations: crate::interactive::session::continuation::SessionObservations,
    /// The session's optional continuation, as the store typed it on load or
    /// a compaction built it: a record this binary understands, or the
    /// verbatim JSON of one it does not (`Unsupported` — a newer binary's
    /// data, carried opaquely and never replaced). Runtime-carried
    /// (`#[serde(skip)]`, the `observations` precedent): `redacted()` hands
    /// it to the store so the next save persists it, and a resume reads it
    /// back verbatim.
    #[serde(skip)]
    pub(crate) continuation: Option<StoredContinuation>,
    /// The session's rotation lineage link to its parent, carried through
    /// `redacted()` the same way. LINEAGE ONLY: the link records where the
    /// session came from and grants nothing.
    #[serde(skip)]
    pub(crate) parent: Option<ParentLink>,
    pub messages: Vec<SessionLine>,
    pub turns: Vec<RedactedTurn>,
    /// The `/compact` summary of the older turns, replayed ahead of the
    /// verbatim tail. Working memory only (`#[serde(skip)]`, the `usage`
    /// precedent): the transcript keeps what was said, compaction rewrites
    /// only what the model replays. `/clear` resets it with the rest of the
    /// conversation.
    #[serde(skip)]
    pub compaction_summary: Option<String>,
    /// How many oldest turns the summary replaces. `#[serde(skip)]` with the
    /// summary: a resumed session replays its persisted turns in full.
    #[serde(skip)]
    pub compacted_turns: usize,
    /// Whether the model's chain-of-thought is shown in the transcript. Off by
    /// default; toggled by `/thinking` or `--show-thinking`. In-memory only: it
    /// is a display preference, not conversation data. A resumed session
    /// deserializes it as off and the startup path re-derives it from the
    /// config setting and `--show-thinking`. `#[serde(skip)]` keeps it out of
    /// persisted session files.
    #[serde(skip)]
    pub show_thinking: bool,
    /// In-memory session token accumulator. `#[serde(skip)]` keeps it
    /// out of persisted session files (no new persisted state);
    /// a resumed session starts with a fresh total. `/clear` resets it.
    #[serde(skip)]
    pub(crate) usage: SessionUsage,
    /// Whether the host-command lane composed for this session: the status
    /// header's `host:` segment reads this. In-memory only — a resumed
    /// session recomposes the lane from its launch statement, never from
    /// the record — so `#[serde(skip)]` keeps it out of persisted files.
    #[serde(skip)]
    pub host_composed: bool,
    /// Whether the context-window warning already fired for the current
    /// above-threshold stretch. Set on the upward crossing of
    /// `CONTEXT_WARN_PERCENT`, cleared when utilisation falls back below it
    /// and by `/clear`. In-memory only (`#[serde(skip)]`, the `show_thinking`
    /// precedent): a resumed session re-derives it from the next turn's
    /// usage rather than persisting a stale flag.
    #[serde(skip, default = "default_context_warned")]
    pub context_warned: bool,
    /// Whether the near-limit capacity notice already fired for this session
    /// process: one notice after the first successful save whose stored
    /// record sits at or above the 3 MiB warning level. In-memory only
    /// (`#[serde(skip)]`, the `context_warned` precedent): a resumed session
    /// — and a rotation successor — starts with it armed again.
    #[serde(skip, default = "default_capacity_warned")]
    pub capacity_warned: bool,
    /// Whether an automatic compaction already tried and failed this session.
    /// Set on an automatic failure, cleared by `/clear` and by any
    /// successful compaction (manual or automatic). In-memory only
    /// (`#[serde(skip)]`): a resumed session re-derives it from the next
    /// turn's outcome rather than persisting a stale refusal. While set the
    /// automatic trigger stays silent — a session that fails to summarise
    /// every turn forever would burn the budget it was trying to save — and
    /// only an explicit `/compact` (or a fresh context) re-arms it.
    #[serde(skip, default = "default_auto_compact_failed")]
    pub auto_compact_failed: bool,
    /// The session's deny list: bare program names every door refuses. The
    /// status header lists them. In-memory only — recomposed from the launch
    /// statement and user-layer config, never from the record.
    #[serde(skip)]
    pub denied_programs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLine {
    pub role: String,
    pub content: String,
}
pub type Session = SessionState;

include!("recording.rs");
include!("continuation.rs");
include!("redacted.rs");

/// The default task posture's own spelling: `AgentMode::Build`, rendered
/// where the mode is named. The `serde(default)` entry point for
/// `SessionState::agent_mode`, so session files written before the field
/// existed resume as build.
fn default_agent_mode() -> String {
    saya_agent::AgentMode::Build.as_str().into()
}

/// A deserialized session starts with the context warning armed: the flag is
/// `#[serde(skip)]`, so nothing on disk sets it, and an armed start means the
/// next turn's usage re-derives the flag from a live report.
fn default_context_warned() -> bool {
    false
}

/// A deserialized session starts with the capacity notice armed: the flag is
/// `#[serde(skip)]`, so nothing on disk sets it, and a resumed session — a
/// new session process — may see the near-limit notice once again.
fn default_capacity_warned() -> bool {
    false
}

/// A deserialized session starts with automatic compaction armed: the flag is
/// `#[serde(skip)]`, so nothing on disk sets it, and an armed start means the
/// next crossing may fire from a live report.
fn default_auto_compact_failed() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::ToolResultShape;

    include!("context_flags.rs");

    include!("persistence_safety.rs");

    include!("replay_and_posture.rs");
}
