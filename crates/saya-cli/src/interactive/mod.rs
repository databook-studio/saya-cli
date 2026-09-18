pub(crate) mod session_activation;
// The `/allow` composition refusals: why a scope that parses still gates
// nothing in this session's composition.
pub(crate) mod allow_refusal;
pub(crate) mod auto_compact;
pub(crate) mod compact_task;
mod session_commands;
pub(crate) mod session_compact;
pub(crate) mod session_compact_call;
#[cfg(test)]
#[path = "session_compact_tests.rs"]
mod session_compact_tests;
pub(crate) mod session_deny;
#[cfg(test)]
mod session_deny_red_tests;
pub(crate) mod session_host;
pub(crate) mod session_tasks;
#[cfg(test)]
#[path = "session_tasks_plan_tests.rs"]
mod session_tasks_plan_tests;
#[cfg(test)]
#[path = "session_tasks_red_tests.rs"]
mod session_tasks_red_tests;
pub(crate) mod session_tasks_render;
// The session's write-shaped definitions are crate-reachable: the approval
// tests build the real grantable shapes the frontends are asked about.
pub(crate) mod session_definitions;
mod session_emit;
mod session_loop;
pub(crate) mod session_paths;
// `/allow` and `/grants`: the session grant store's one behaviour, shared
// by the headless loop and the TUI dispatch.
// `/allow` and `/grants`: the session grant store's one behaviour, shared
// by the headless loop and the TUI dispatch — and the journal-warning
// wording every journaling site renders.
pub(crate) mod session_grants;
#[cfg(test)]
mod session_grants_tests;
pub(crate) mod session_prompt;
mod session_request;
mod session_resume;
mod session_run;
mod session_runner;
pub(crate) mod session_runtime;
mod session_schema;
mod session_sql;
mod session_state;
pub(crate) mod session_trust;
#[cfg(test)]
mod session_trust_tests;
pub(crate) mod session_universe;
mod session_workspace;
mod tui;

pub use session_commands::SessionAction;
pub use session_loop::run;
pub use session_state::{Session, SessionState};
