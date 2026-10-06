//! Tests for the U04 terminal handoff boundary: the release → work →
//! reacquire ordering over a `Recorder` double, every outcome (value, error
//! value, panic, failed release, failed reacquire), and the guided-setup
//! wrapper over scripted streams. No real terminal is touched; the PTY proof
//! of byte-exact restoration is U06's.

use super::terminal_handoff::{
    SetupHandoff, TerminalModes, run_guided_setup, run_guided_setup_with, with_terminal_released,
};
use crate::cli::GlobalOptions;
use crate::setup::flow_options::FlowOptions;
use crate::setup::probe::{DatabaseProbe, FlowProbes, ProbeResult, ProviderProbe};
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
