//! The TUI session's inputs and the `run` entry point.

use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use saya_store::{FsSessionStore, SqliteStateStore};

/// The TUI session's inputs: the runtime, stores, format, live session
/// state and engine, the plain-REPL trust echo (said only where the
/// plain-REPL prompt bound a directory — never on the TUI path), whether
/// the startup trust modal opens after the splash paints, and the launch's
/// host statement for a modal trust answer's recomposition. A bundle
/// rather than nine positional parameters, so the call sites read by name.
pub(crate) struct TuiSession<'a> {
    pub(crate) runtime: &'a RuntimeConfig,
    pub(crate) store: &'a FsSessionStore,
    pub(crate) state_db: &'a SqliteStateStore,
    pub(crate) format: RenderFormat,
    pub(crate) state: &'a mut SessionState,
    pub(crate) session: &'a mut SessionRuntime,
    pub(crate) trusted_echo: Option<&'a str>,
    pub(crate) trust_pending: bool,
    pub(crate) launch: &'a crate::interactive::session_host::HostLaunch,
}

pub(crate) mod outcome;
pub(crate) mod run;
pub(crate) mod startup;

pub(crate) use outcome::TrustOutcome;
pub(crate) use run::run;
#[allow(unused_imports)]
pub(crate) use startup::build_app;
