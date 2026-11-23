#[cfg(test)]
pub(crate) use session::permissions::allow_refusal;
// The session record's capacity seam: the one near-limit notice and the
// friendly refusal at the 4 MiB cap, shared by every save surface.
pub(crate) mod compact_task;
#[cfg(test)]
#[path = "session/continuation/compaction/manual_tests.rs"]
mod session_compact_tests;
#[cfg(test)]
#[path = "session_narrative_feed_tests.rs"]
mod session_narrative_feed_tests;
// Transitional continuation path kept for existing callers.
#[cfg(test)]
#[path = "session/permissions/deny_red_tests.rs"]
mod session_deny_red_tests;
// What still reaches the `run_command` matcher in `session/permissions/deny.rs`:
// the one-release alias of `shell` and the legacy `command:<program>` grant, so
// a later unit cannot remove that branch by claiming it was unreachable.
#[cfg(test)]
#[path = "session/permissions/deny_run_command_reach_tests.rs"]
mod session_deny_run_command_reach_tests;
// The old session surface's tool definitions, kept as test fixtures: the
// approval, deny and prompt-facts tests pin the exact shape of each old door.
#[cfg(test)]
pub(crate) mod session_definitions;
mod session_emit;
#[cfg(test)]
pub(crate) mod session_file_definitions;
pub(crate) mod session_host;
// `/allow` and `/grants`: the session grant store's one behaviour, shared
// by the headless loop and the TUI dispatch — and the journal-warning
// wording every journaling site renders.
#[cfg(test)]
#[path = "session/permissions/grants_tests.rs"]
mod session_grants_tests;
// The status header the line front and the headless loop print: profile,
// provider, mode, workspace root, host lane, sharing.
pub(crate) mod session_prompt;
// The one terminal result a scripted turn reports: its status, exit code,
// and the honest tool tally.
pub(crate) mod session;
#[cfg(test)]
#[path = "session/permissions/launch_posture_tests.rs"]
mod session_launch_posture_tests;
mod session_schema;
mod session_sql;
#[cfg(test)]
#[path = "session/permissions/trust_tests.rs"]
mod session_trust_tests;
pub(crate) mod turn_summary;
// Crate-reachable: the `saya preferences` subcommand derives its worktree
// identity through the same `resolve_root` the session binds — one resolution,
// never a second walk-up.
pub(crate) mod session_workspace;
pub(crate) mod sql_execution;
pub(crate) mod sql_operation;
// What the new surface reuses from the old TUI: the composer's buffer and
// completion, the wrap and fuzzy rules, the input history, the tables, the
// clipboard attempt, the terminal guard and its `/setup` handoff, the export
// writer, the query worker, the trust answer and the job plumbing. X1 deleted
// the surface itself; nothing here is a second frame.
pub(crate) mod tui;

// The session's token accumulator — the answering and extraction totals
// `/usage` reads — is the one a line turn's own stream folds into.
mod usage;
pub(crate) use usage::SessionUsage;

pub use session::commands::SessionAction;
pub use session::turn_loop::run;
pub use session::{Session, SessionState};
