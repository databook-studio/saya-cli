//! Tests for the U04 terminal handoff boundary: the release → work →
//! reacquire ordering over a `Recorder` double, every outcome (value, error
//! value, panic, failed release, failed reacquire), and the guided-setup
//! wrapper over scripted streams. No real terminal is touched; the PTY proof
//! of byte-exact restoration is U06's.

use super::terminal_handoff::{
    SetupHandoff, TerminalModes, handle_setup_outcome, run_guided_setup, run_guided_setup_with,
    with_terminal_released,
};
use crate::cli::GlobalOptions;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::interactive::tui::loop_tick::pending::tick_pending;
use crate::interactive::tui::transcript::{BlockKind, Transcript};
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

#[test]
fn order_is_release_work_reacquire_and_result_is_returned() {
    let mut recorder = Recorder::ok();
    let log = recorder.log.clone();
    let result = with_terminal_released(&mut recorder, || {
        log.push("work");
        42
    });
    assert_eq!(result.unwrap(), 42, "the work's value comes back");
    assert_eq!(recorder.log.entries(), vec!["release", "work", "reacquire"]);
}

#[test]
fn work_error_value_still_reacquires() {
    let mut recorder = Recorder::ok();
    let log = recorder.log.clone();
    let result = with_terminal_released(&mut recorder, || {
        log.push("work");
        Err::<i32, &str>("wizard said no")
    });
    assert_eq!(
        result.unwrap(),
        Err("wizard said no"),
        "an error value is a result, not a terminal failure"
    );
    assert_eq!(recorder.log.entries(), vec!["release", "work", "reacquire"]);
}

#[test]
fn panic_in_work_reacquires_then_resumes() {
    let mut recorder = Recorder::ok();
    let log = recorder.log.clone();
    let result = catch_unwind(AssertUnwindSafe(|| {
        with_terminal_released(&mut recorder, || {
            log.push("work");
            panic!("wizard blew up");
        })
    }));
    let payload = result.expect_err("a panic in the work resumes after the terminal is back");
    let message = payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or_default();
    assert!(
        message.contains("wizard blew up"),
        "the original panic message: {message}"
    );
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "work", "reacquire"],
        "the panic path still reacquires"
    );
}

#[test]
fn failed_release_skips_work_and_repairs_best_effort() {
    let mut recorder = Recorder::failing_release();
    let log = recorder.log.clone();
    let mut work_ran = false;
    let result = with_terminal_released(&mut recorder, || {
        log.push("work");
        work_ran = true;
        7
    });
    let error = result.expect_err("a failed release runs no work");
    assert!(error.to_string().contains("release failed"), "{error}");
    assert!(!work_ran, "work never ran after a failed release");
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "reacquire"],
        "the half-released terminal gets a best-effort repair"
    );
}

#[test]
fn failed_reacquire_reports_the_terminal_lost() {
    let mut recorder = Recorder::failing_reacquire();
    let log = recorder.log.clone();
    let result = with_terminal_released(&mut recorder, || {
        log.push("work");
        9
    });
    let error = result.expect_err("a failed reacquire is an error");
    let message = error.to_string();
    assert!(
        message.contains("could not be re-entered"),
        "the error says the terminal is lost: {message}"
    );
    assert!(
        message.contains("reacquire failed"),
        "the underlying cause is kept: {message}"
    );
    assert_eq!(recorder.log.entries(), vec!["release", "work", "reacquire"]);
}

/// The cancellation path: EOF at the first prompt cancels the wizard with
/// exit 0 and "No files were changed.", and the handoff still restored the
/// terminal exactly once each way.
#[test]
fn guided_setup_eof_cancels_and_restores_once() {
    let dir = temp_dir("eof");
    let mut recorder = Recorder::ok();
    let (probes, calls) = recording_probes();
    let mut options = flow_options(&dir, probes);
    let mut input = Cursor::new("");
    let mut output = Vec::new();
    let result = run_guided_setup_with(&mut recorder, &mut options, &mut input, &mut output);
    let handoff = result.expect("EOF cancels the wizard, never the handoff");
    assert_eq!(
        handoff,
        SetupHandoff::Finished(0),
        "the wizard's own cancel is exit 0"
    );
    let out = String::from_utf8(output).unwrap();
    assert!(
        out.contains("No files were changed."),
        "the cancel line: {out}"
    );
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "reacquire"],
        "exactly one release and one reacquire"
    );
    let recorded = calls.lock().unwrap().clone();
    assert!(recorded.is_empty(), "no probe ever ran: {recorded:?}");
    assert!(
        !dir.join("connections.toml").exists(),
        "nothing was written"
    );
    let _ = fs::remove_dir_all(dir);
}

/// The wizard-error path: the simplest forced `run_with` error (the
/// flow_tests pattern — an unreadable recovery marker) surfaces as
/// `Failed(..)`, and the terminal is still reacquired.
#[test]
fn guided_setup_wizard_error_still_reacquires() {
    let dir = temp_dir("wizard-error");
    fs::write(dir.join(MARKER_FILE), b"{ interrupted, but corrupt").unwrap();
    let mut recorder = Recorder::ok();
    let (probes, calls) = recording_probes();
    let mut options = flow_options(&dir, probes);
    let mut input = Cursor::new(FULL_SCRIPT);
    let mut output = Vec::new();
    let result = run_guided_setup_with(&mut recorder, &mut options, &mut input, &mut output);
    let handoff = result.expect("a wizard error is a Failed handoff, not an io error");
    match handoff {
        SetupHandoff::Failed(message) => {
            assert!(
                message.contains(MARKER_FILE),
                "the failure names the marker: {message}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "reacquire"],
        "the wizard's error still returns the terminal"
    );
    let recorded = calls.lock().unwrap().clone();
    assert!(recorded.is_empty(), "no probe ever ran: {recorded:?}");
    assert!(
        !dir.join("connections.toml").exists(),
        "nothing was written"
    );
    let _ = fs::remove_dir_all(dir);
}

/// `run_guided_setup` locks the real stdin/stdout, so calling it inside a
/// test would block on the developer's terminal; U06 calls it in production.
/// This pins its existence and signature — the honest alternative to a
/// dead-code allow.
#[test]
fn run_guided_setup_is_the_real_streams_entry_point() {
    let entry: fn(&mut Recorder, &mut FlowOptions) -> io::Result<SetupHandoff> =
        run_guided_setup::<Recorder>;
    let _ = entry;
}

// ---------------------------------------------------------------------------
// U06: the `/setup` outcome rendering and the tick_pending gate.
// ---------------------------------------------------------------------------

/// The pinned closed wording: `Finished(0)` — which includes the wizard's own
/// cancel — is a System block saying the new configuration applies to the
/// NEXT session. No config reload happens in this packet.
#[test]
fn finished_zero_pushes_the_closed_system_block() {
    let mut transcript = Transcript::new();
    handle_setup_outcome(Ok(SetupHandoff::Finished(0)), &mut transcript).expect("renders");
    let blocks = transcript.blocks();
    assert_eq!(blocks.len(), 1, "one block, not two: {:?}", blocks);
    assert_eq!(blocks[0].kind, BlockKind::System);
    assert_eq!(
        blocks[0].text,
        "Guided setup closed. Any new configuration applies to your next saya session."
    );
}

/// A non-zero exit code is an Error block naming the code and what was NOT
/// applied beyond what the wizard itself reported.
#[test]
fn finished_nonzero_pushes_the_exit_code_error_block() {
    let mut transcript = Transcript::new();
    handle_setup_outcome(Ok(SetupHandoff::Finished(2)), &mut transcript).expect("renders");
    let blocks = transcript.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].kind, BlockKind::Error);
    assert_eq!(
        blocks[0].text,
        "Guided setup ended with exit code 2. Nothing was applied that the wizard did not report."
    );
}

/// A wizard failure surfaces its rendered error text as an Error block.
#[test]
fn failed_pushes_the_failure_error_block() {
    let mut transcript = Transcript::new();
    handle_setup_outcome(Ok(SetupHandoff::Failed("x".into())), &mut transcript).expect("renders");
    let blocks = transcript.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].kind, BlockKind::Error);
    assert_eq!(blocks[0].text, "Guided setup failed: x");
}

/// A lost terminal (the reacquire failed) propagates and renders nothing:
/// the loop must stop, not keep drawing into a terminal it no longer owns.
#[test]
fn io_error_propagates_without_a_block() {
    let mut transcript = Transcript::new();
    let error = handle_setup_outcome(Err(io::Error::other("reacquire failed")), &mut transcript)
        .expect_err("a lost terminal is an error, not a rendered outcome");
    assert!(error.to_string().contains("reacquire failed"), "{error}");
    assert!(
        transcript.blocks().is_empty(),
        "nothing renders on a lost terminal"
    );
}

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

/// `/setup` typed while busy waits like every other command: the busy gate
/// holds the queued line and the handoff never starts.
#[test]
fn setup_typed_while_busy_waits_like_every_other_command() {
    let mut fx = PendingFixture::build("busy-gate");
    fx.app.pending = Some("/setup".to_string());
    // A running compaction makes the app busy, exactly like a query or an
    // agent stream would.
    let (_tx, rx) = std::sync::mpsc::channel::<CompactOutcome>();
    fx.app.compact_task = Some(rx);
    let mut recorder = Recorder::ok();
    fx.drain(&mut recorder).expect("the busy tick runs");
    assert_eq!(
        fx.app.pending.as_deref(),
        Some("/setup"),
        "the busy gate holds the line"
    );
    assert!(
        recorder.log.entries().is_empty(),
        "the handoff never ran while busy: {:?}",
        recorder.log.entries()
    );
}

/// A handoff whose release fails returns the io error — the loop stops —
/// and the best-effort repair still reacquired the half-released terminal.
/// The release failing means the wizard never ran (no stdin read), so this
/// exercises the real `tick_pending` path with a fake.
#[test]
fn a_failed_handoff_returns_the_error_and_stops_the_loop() {
    let mut fx = PendingFixture::build("failed-handoff");
    fx.app.pending = Some("/setup".to_string());
    let mut recorder = Recorder::failing_release();
    let error = fx
        .drain(&mut recorder)
        .expect_err("a failed handoff stops the loop");
    assert!(error.to_string().contains("release failed"), "{error}");
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "reacquire"],
        "the half-released terminal gets its best-effort repair"
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .all(|block| !block.text.contains("Guided setup")),
        "nothing renders on a lost handoff"
    );
}
