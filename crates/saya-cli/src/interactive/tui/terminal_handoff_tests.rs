//! Tests for the U04 terminal handoff boundary: the release → work →
//! reacquire ordering over a `Recorder` double, every outcome (value, error
//! value, panic, failed release, failed reacquire), and the guided-setup
//! wrapper over scripted streams. No real terminal is touched; the PTY proof
//! of byte-exact restoration is U06's.

use super::terminal_handoff::{
    SetupHandoff, TerminalModes, run_guided_setup, run_guided_setup_with, with_terminal_released,
};
use crate::cli::GlobalOptions;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::interactive::tui::loop_tick::pending::tick_pending;
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::CompactOutcome;
use crate::render::RenderFormat;
use crate::setup::flow_options::FlowOptions;
use crate::setup::probe::{DatabaseProbe, FlowProbes, ProbeResult, ProviderProbe};
use saya_agent::ApprovalPolicy;
use saya_store::FsSessionStore;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Cursor};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MARKER_FILE: &str = ".setup-commit.json";

/// The release/reacquire log: shared so the `work` closure can record too.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<&'static str>>>);

impl Log {
    fn push(&self, entry: &'static str) {
        self.0.lock().unwrap().push(entry);
    }

    fn entries(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

/// The `TerminalModes` double: records every call so the tests assert the
/// exact order, and can fail either half on demand.
struct Recorder {
    log: Log,
    fail_release: bool,
    fail_reacquire: bool,
}

impl Recorder {
    fn ok() -> Self {
        Self {
            log: Log::default(),
            fail_release: false,
            fail_reacquire: false,
        }
    }

    fn failing_release() -> Self {
        Self {
            fail_release: true,
            ..Self::ok()
        }
    }

    fn failing_reacquire() -> Self {
        Self {
            fail_reacquire: true,
            ..Self::ok()
        }
    }
}

impl TerminalModes for Recorder {
    fn release(&mut self) -> io::Result<()> {
        self.log.push("release");
        if self.fail_release {
            Err(io::Error::other("release failed"))
        } else {
            Ok(())
        }
    }

    fn reacquire(&mut self) -> io::Result<()> {
        self.log.push("reacquire");
        if self.fail_reacquire {
            Err(io::Error::other("reacquire failed"))
        } else {
            Ok(())
        }
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-tui-handoff-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Probes that record every call and would succeed if one ever ran — the
/// handoff tests cancel or fail before any probe, so the log must stay empty
/// (the flow_tests.rs instant-probe pattern).
fn recording_probes() -> (FlowProbes, Arc<Mutex<Vec<String>>>) {
    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let db_calls = calls.clone();
    let database: DatabaseProbe = Arc::new(move |profile| {
        db_calls
            .lock()
            .unwrap()
            .push(format!("database:{}", profile.dialect().as_str()));
        Box::pin(async { ProbeResult::ok("database reachable") })
    });
    let prov_calls = calls.clone();
    let provider: ProviderProbe = Arc::new(move |draft| {
        prov_calls
            .lock()
            .unwrap()
            .push(format!("provider:{}", draft.provider.as_str()));
        Box::pin(async { ProbeResult::ok("provider answered") })
    });
    (FlowProbes { database, provider }, calls)
}

/// Flow options over a temp directory; never `real` (no network probes).
fn flow_options(dir: &Path, probes: FlowProbes) -> FlowOptions {
    FlowOptions {
        options: GlobalOptions::default(),
        user_dir: dir.to_path_buf(),
        cwd: dir.to_path_buf(),
        env: BTreeMap::new(),
        reload: None,
        probes,
    }
}

/// A full valid script: if the flow ran past its first stop, this would run
/// probes and write files.
const FULL_SCRIPT: &str = "6\n1\n/tmp/saya-handoff-test.db\nteam\ny\n";

include!("terminal_handoff_tests/handoff_reacquire.rs");

include!("terminal_handoff_tests/guided_setup.rs");

// ---------------------------------------------------------------------------
// U07: the `/setup` outcome rendering through the post-setup apply.
// ---------------------------------------------------------------------------

include!("terminal_handoff_tests/setup_outcomes.rs");

/// One fixture for the `tick_pending` gate: the pieces the live loop passes,
/// over a throwaway root.
struct PendingFixture {
    app: crate::interactive::tui::types::App,
    state: SessionState,
    session: SessionRuntime,
    runtime: crate::config::runtime::RuntimeConfig,
    store: FsSessionStore,
    root: PathBuf,
}

impl PendingFixture {
    fn build(tag: &str) -> Self {
        let root = temp_dir(tag);
        let runtime = unused_runtime();
        let session = SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            "handoff-busy",
            ApprovalPolicy::Ask,
            &root.join("session-dirs"),
        )
        .expect("the pending-fixture session acquires");
        let state = SessionState::new("handoff-busy", None, "model");
        let store = FsSessionStore::new(root.join("store"));
        Self {
            app: idle_app(),
            state,
            session,
            runtime,
            store,
            root,
        }
    }

    /// The real queued-prompt drain with a `TerminalModes` double, exactly as
    /// the live loop passes the guard.
    fn drain(&mut self, modes: &mut dyn TerminalModes) -> io::Result<()> {
        let options = GlobalOptions::default();
        let mut setup = crate::interactive::tui::loop_tick::pending::SetupInputs {
            modes,
            options: &options,
        };
        tick_pending(
            &mut self.app,
            &mut self.state,
            &self.runtime,
            &self.store,
            RenderFormat::Text,
            &mut self.session,
            &mut setup,
        )
    }
}

impl Drop for PendingFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

include!("terminal_handoff_tests/pending_loop.rs");
