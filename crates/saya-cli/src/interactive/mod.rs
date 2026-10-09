#[cfg(test)]
pub(crate) use session::permissions::allow_refusal;
// The session record's capacity seam: the one near-limit notice and the
// friendly refusal at the 4 MiB cap, shared by every save surface.
pub(crate) mod compact_task;
#[cfg(test)]
#[path = "compact_task_tui_metrics_tests.rs"]
mod compact_task_tui_metrics_tests;
#[cfg(test)]
#[path = "session/continuation/compaction/manual_bounds_tests.rs"]
mod session_compact_bounds_tests;
#[cfg(test)]
#[path = "session/continuation/compaction/manual_tests.rs"]
mod session_compact_tests;
#[cfg(test)]
#[path = "session_narrative_feed_tests.rs"]
mod session_narrative_feed_tests;
// Transitional continuation path for existing callers until M07.
#[cfg(test)]
#[path = "session/permissions/deny_red_tests.rs"]
mod session_deny_red_tests;
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
// decisions (scratch, fetch, runner, command) beside the workspace file
// family's own. Their tests ride the module itself
// (`session_grant_families_tests.rs`).
pub(crate) mod session_prompt;
mod session_request;
// The one terminal result a scripted turn reports: its status, exit code,
// and the honest tool tally.
pub(crate) mod session;
#[cfg(test)]
#[path = "session/permissions/launch_posture_tests.rs"]
mod session_launch_posture_tests;
mod session_runner;
mod session_schema;
mod session_sql;
#[cfg(test)]
#[path = "session/permissions/trust_tests.rs"]
mod session_trust_tests;
pub(crate) mod turn_summary;
// Crate-reachable (M04a): the `saya preferences` subcommand derives its
// worktree identity through the same `resolve_root` the session binds — one
// resolution, never a second walk-up.
pub(crate) mod session_workspace;
pub(crate) mod sql_execution;
pub(crate) mod sql_operation;
pub(crate) mod tui;

pub use session::commands::SessionAction;
pub use session::turn_loop::run;
pub use session::{Session, SessionState};
