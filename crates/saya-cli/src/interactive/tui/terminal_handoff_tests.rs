//! Tests for the terminal handoff boundary: the release → work →
//! reacquire ordering over a `Recorder` double, every outcome (value, error
//! value, panic, failed release, failed reacquire), and the guided-setup
//! wrapper over scripted streams. No real terminal is touched; the PTY proof
//! of byte-exact restoration belongs to the terminal driver.

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

include!("terminal_handoff_tests/handoff_reacquire.rs");

include!("terminal_handoff_tests/guided_setup.rs");
