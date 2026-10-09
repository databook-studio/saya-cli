//! Chain tests for the TUI dispatch chain (P0): every registered slash
//! command must survive the real `dispatch` without panicking, and the
//! file-writing commands must write through it. The chain used to panic
//! whenever a helper consumed an action and then reported "not mine"
//! (`.expect("helper passes the action through")`); these tests prove no
//! registered command can trip that again.

use super::Dispatch;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use crate::interactive::tui::types::{App, LastQuery};
use crate::render::RenderFormat;
use crate::slash::registry::KNOWN_COMMANDS;
use saya_agent::ApprovalPolicy;
use saya_store::{FsSessionStore, SessionStore};
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, SqlDialect,
};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

include!("chain_tests/harness.rs");
include!("chain_tests/dispatch_basics.rs");

include!("chain_tests/report_and_context.rs");

include!("chain_tests/setup_parse.rs");

include!("chain_tests/sessions.rs");

include!("chain_tests/preferences.rs");

/// Compacts the fixture state through the production compaction path, so
/// `/rotate` has a summary and a continuation record to carry.
fn compact_fixture_state(state: &mut SessionState) {
    use crate::interactive::session::continuation::compaction::manual::{
        apply_with_continuation, plan as compact_plan,
    };
    for index in 0..6 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let history = state.provider_history();
    let plan = compact_plan(&state.turns, &history).expect("six turns plan");
    apply_with_continuation(state, &plan, "older turns discussed orders", &|_| None)
        .expect("the summary applies");
}

include!("chain_tests/rotation.rs");

/// Gives the loop's background save a moment to land before cleanup, as
/// the resume fixtures do.
fn wait_for_background_save(store_root: &std::path::Path) {
    for _ in 0..200 {
        if std::fs::read_dir(store_root).is_ok_and(|entries| entries.count() > 0) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

include!("chain_tests/notes.rs");

include!("chain_tests/recovery.rs");

/// The handoff inputs the real `tick_pending` takes: a no-op terminal double
/// (no `/setup` line is drained in these tests, so neither half runs) and
/// default CLI options.
struct NoModes;

impl crate::interactive::tui::terminal_handoff::TerminalModes for NoModes {
    fn release(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn reacquire(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Drains the queued line through the real `tick_pending`, as the live loop
/// does. The fixture's launch options ride `SetupInputs`, exactly as the
/// live loop threads them.
fn drain_through_tick_pending(fx: &mut ChainFixture) {
    let mut modes = NoModes;
    let mut setup = crate::interactive::tui::loop_tick::pending::SetupInputs {
        modes: &mut modes,
        options: &fx.launch_options,
    };
    let _ = crate::interactive::tui::loop_tick::pending::tick_pending(
        &mut fx.app,
        &mut fx.state,
        &fx.runtime,
        &fx.store,
        RenderFormat::Text,
        &mut fx.session,
        &mut setup,
    );
}

include!("chain_tests/reload_lifecycle.rs");

// ---------------------------------------------------------------------------
// U07: the shared runtime swap and the post-setup apply
// ---------------------------------------------------------------------------

include!("chain_tests/setup_reload.rs");
