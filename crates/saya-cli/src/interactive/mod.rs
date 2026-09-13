pub(crate) mod session_activation;
mod session_commands;
// The session's write-shaped definitions are crate-reachable: the approval
// tests build the real grantable shapes the frontends are asked about.
pub(crate) mod session_definitions;
mod session_emit;
mod session_loop;
pub(crate) mod session_paths;
// `/allow` and `/grants`: the session grant store's one behaviour, shared
// by the headless loop and the TUI dispatch.
mod session_grants;
#[cfg(test)]
mod session_grants_tests;
mod session_prompt;
mod session_request;
mod session_resume;
mod session_run;
mod session_runner;
pub(crate) mod session_runtime;
mod session_schema;
mod session_sql;
mod session_state;
pub(crate) mod session_universe;
mod session_workspace;
mod tui;

pub use session_commands::SessionAction;
pub use session_loop::run;
pub use session_state::{Session, SessionState};
