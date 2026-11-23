//! The two children whose environment is cleared keep the *same* Windows
//! system variables.
//!
//! A cleared environment on Windows leaves a process without `SystemRoot`, so
//! Winsock never initialises and the child's first HTTP request fails before it
//! reaches the network. The managed-job child learned this first; the
//! scripted-provider child did not, and every line-front e2e row timed out on
//! real Windows reporting only "provider recovery: not retried".
//!
//! No Windows machine runs this file, so it pins the *structural* fact that
//! makes the fix hold: one list, in one shared test-support file, included by
//! both harnesses — and neither harness growing a private copy again.

use std::path::{Path, PathBuf};

/// The shared support file, crate-relative, that both harnesses include.
const SHARED: &str = "support/windows_system_env.rs";

/// The child that spawns the line front / question-budget REPL rows.
const SCRIPTED_CHILD: &str = "support/scripted_provider/spawned_child.rs";

/// The child that spawns the managed-job apply-back rows.
const JOB_SUPPORT: &str = "job_apply_e2e/support.rs";

/// The variables a Windows child needs to load and to open a socket. Stated
/// here independently of the shared file so a deletion from that file is a
/// failure, not a silent narrowing.
const REQUIRED: &[&str] = &[
    "SystemRoot",
    "WINDIR",
    "SystemDrive",
    "TEMP",
    "TMP",
    "ComSpec",
    "PATHEXT",
    "LOCALAPPDATA",
    "APPDATA",
    "ProgramData",
];

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(tests_dir().join(relative))
        .unwrap_or_else(|error| panic!("tests/{relative} reads: {error}"))
}

/// One shared list, cfg-gated to Windows, with a no-op for every other target
/// so the Unix child's environment is untouched.
#[test]
fn the_shared_helper_states_the_windows_variables_a_child_needs() {
    let shared = read(SHARED);
    let missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|name| !shared.contains(&format!("\"{name}\"")))
        .collect();
    assert!(
        missing.is_empty(),
        "tests/{SHARED} must name every Windows variable a cleared child needs \
         ({missing:?} missing)"
    );
    assert!(
        shared.contains("#[cfg(windows)]") && shared.contains("#[cfg(not(windows))]"),
        "tests/{SHARED} must add the variables only on Windows and stay a no-op \
         elsewhere, so the Unix child's environment is unchanged: {shared}"
    );
}

/// Both cleared-environment children include that one file, call it, and keep
/// no private copy of the list.
#[test]
fn both_cleared_children_use_the_shared_helper() {
    let mut offenders = Vec::new();

    let scripted = read(SCRIPTED_CHILD);
    let includes_shared = scripted
        .lines()
        .any(|line| line.trim() == "include!(\"../windows_system_env.rs\");");
    if !includes_shared {
        offenders.push(format!(
            "tests/{SCRIPTED_CHILD} does not include ../windows_system_env.rs"
        ));
    }
    match (
        scripted.find(".env_clear()"),
        scripted.find("inherit_windows_system_env(&mut command)"),
    ) {
        (None, _) => offenders.push(format!("tests/{SCRIPTED_CHILD} no longer clears its env")),
        (Some(_), Some(call_at)) if call_at > scripted.find(".env_clear()").unwrap() => {}
        (Some(_), Some(_)) => offenders.push(format!(
            "tests/{SCRIPTED_CHILD} calls the helper before clearing its env, so \
             the variables are cleared away again"
        )),
        (Some(_), None) => offenders.push(format!(
            "tests/{SCRIPTED_CHILD} never calls inherit_windows_system_env, so a \
             Windows child has no SystemRoot and Winsock cannot initialise"
        )),
    }

    let job = read(JOB_SUPPORT);
    let job_includes_shared = job
        .lines()
        .any(|line| line.trim() == "include!(\"../support/windows_system_env.rs\");");
    if !job_includes_shared {
        offenders.push(format!(
            "tests/{JOB_SUPPORT} does not include the shared ../support/windows_system_env.rs"
        ));
    }

    for (path, text) in [(SCRIPTED_CHILD, &scripted), (JOB_SUPPORT, &job)] {
        if text.contains("WINDOWS_SYSTEM_ENV") {
            offenders.push(format!(
                "tests/{path} declares the Windows variable list itself; it belongs \
                 in tests/{SHARED} only"
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "both children whose env is cleared must keep the same Windows variables \
         from one shared list ({} offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
