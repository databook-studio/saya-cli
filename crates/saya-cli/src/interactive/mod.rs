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
pub(crate) mod session_host;
// The session's write-shaped definitions are crate-reachable: the approval
// tests build the real grantable shapes the frontends are asked about.
pub(crate) mod session_definitions;
mod session_emit;
// The session's digest-guarded file tools, derived from the run surface's
// definitions so the parameter schemas cannot drift.
pub(crate) mod session_file_definitions;
// `/allow` and `/grants`: the session grant store's one behaviour, shared
// by the headless loop and the TUI dispatch.
// `/allow` and `/grants`: the session grant store's one behaviour, shared
// by the headless loop and the TUI dispatch — and the journal-warning
// wording every journaling site renders.
#[cfg(test)]
#[path = "session/permissions/approval_surface_tests.rs"]
mod session_approval_surface_tests;
#[cfg(test)]
#[path = "session/permissions/grants_tests.rs"]
mod session_grants_tests;
// The grant surface: the per-turn grant view the session surfaces bind and
// the per-family availability decision `definitions` consumes. Its tests
// ride the module itself (`session_grant_surface_tests.rs`).
// The remaining composed families' per-family grant availability
// decisions (scratch, fetch, command) beside the workspace file
// family's own. Their tests ride the module itself
// (`session_grant_families_tests.rs`).
// The old surface was this seam's only caller; X1 deleted it. The code here
// is X2/X3's to remove, kept compiled rather than deleted twice.
#[allow(dead_code)]
pub(crate) mod session_prompt;
// The old per-turn request body: the last production caller of the old turn
// engine (`agent::runtime`). X3 measured what deleting it alone costs — the
// engine and its appurtenances then report 78 dead-code errors over more than
// 40 files, and the `clippy -D warnings` gate goes red — so the body and this,
// its only production caller, go in one commit with the pins that drive them
// re-pointed at the chat path. See `REPORT.md`.
#[allow(dead_code)]
mod session_request;
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
