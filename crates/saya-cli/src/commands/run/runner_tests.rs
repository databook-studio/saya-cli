//! The composition-root runner gates: the placement guard, the admission
//! check, and the build-time refusals — each with its own test, against a
//! private tree per test. The proven-arm end-to-end pin is macOS-only (the
//! probe decides per host; nothing else constructs a `RunnerSpawn`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use saya_config::{ResolvedInterpreterJobs, ResolvedRunnerJobs};
use saya_types::{Capabilities, InterpreterScope, RunnerScope};

use super::runner::{admit, admit_interpreters, build, place};

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-run-runner-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn scope(programs: &[&str]) -> RunnerScope {
    RunnerScope::new(programs.iter().map(|p| (*p).to_owned()).collect()).expect("shaped")
}

fn interpreter_scope(programs: &[&str]) -> InterpreterScope {
    InterpreterScope::new(programs.iter().map(|p| (*p).to_owned()).collect()).expect("refused name")
}

fn runner_scopes(programs: &[&str]) -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.runner = Some(scope(programs));
    capabilities
}

#[path = "runner_tests/interpreters.rs"]
mod interpreters;
#[path = "runner_tests/placement.rs"]
mod placement;
#[path = "runner_tests/programs.rs"]
mod programs;
