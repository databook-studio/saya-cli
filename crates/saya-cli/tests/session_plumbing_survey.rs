//! The session-plumbing survey X3 starts from: what is dead, what is live, and
//! why the dead half cannot be deleted on its own.
//!
//! Step 1 of the brief is a survey of `SessionUniverse|session::universe|
//! session::runtime|session_request|session_definitions|
//! session_file_definitions`. It comes back mixed: the terminal launch still
//! composes `SessionRuntime`/`SessionUniverse` (`universe().tasks()`,
//! `sync_host_posture`), `front/tui/direct_trust.rs` still holds a
//! `SessionRuntime`, and the chat's mode surface still reads the permission
//! families — while the per-turn request body is the old turn engine's last
//! production caller, and the engine in turn is what keeps the session's
//! definition/executor/grant-binding surface off the dead-code list.
//!
//! These guards pin the three statements that shape the unit: the plumbing the
//! live fronts compose stays, the live readers stay, and the request body is
//! the engine's only production caller (so the two go together, with the pins
//! re-pointed — `REPORT.md`). They read the tree the way `tui_trunk_removed`
//! and `turn_config_single_home` do — paths and source text — because each
//! statement is about the layout. Needles are spelled in halves so this file
//! never matches itself.

use std::path::{Path, PathBuf};

/// The modules the survey found live, and therefore kept. A live front imports
/// each of these on a path this crate builds.
const KEPT: &[&str] = &[
    "interactive/session/universe/mod.rs",
    "interactive/session/runtime/mod.rs",
    "interactive/session_definitions.rs",
    "interactive/session_file_definitions.rs",
    "interactive/session/permissions/grant_surface.rs",
    "interactive/session/permissions/grant_families.rs",
];

/// Each kept module's live caller, and the needle that proves the read: the
/// terminal launch's session, the TUI's trust store, recovery's attach, and the
/// chat's mode surface over the permission families.
const LIVE_READERS: &[(&str, &str)] = &[
    (
        "interactive/session/loop/launch_surface.rs",
        "session.universe().tasks()",
    ),
    (
        "interactive/session/loop/launch_terminal.rs",
        "state.sync_host_posture(&session.universe())",
    ),
    (
        "front/tui/direct_trust.rs",
        concat!("session::", "runtime::SessionRuntime"),
    ),
    (
        "agent/session_recovery_attach.rs",
        concat!("session::", "universe::SessionUniverse"),
    ),
    (
        "agent/tools/coding/direct_mode.rs",
        "grant_surface::workspace_files_available",
    ),
    (
        "agent/tools/coding/direct_mode.rs",
        "grant_families::fetch_available",
    ),
    (
        "agent/tools/coding/direct_mode.rs",
        "grant_families::command_available",
    ),
];

/// The old turn engine's one production entry point: the request body is what
/// keeps the engine — and the session surface the engine alone drives — off the
/// dead-code list.
const ENGINE_ENTRY: &str = concat!("run_prompt_with_sink", "_and_outcomes");

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every first-party source under this crate, tests included: a caller is a
/// statement about the layout too. Yields the crate-relative path and its text.
fn sources() -> Vec<(String, String)> {
    fn walk(directory: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    let root = crate_root();
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    walk(&root.join("tests"), &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let relative = path
                .strip_prefix(&root)
                .ok()?
                .to_string_lossy()
                .into_owned();
            let relative = relative
                .strip_prefix("src/")
                .unwrap_or(&relative)
                .to_owned();
            let text = std::fs::read_to_string(&path).ok()?;
            Some((relative, text))
        })
        .collect()
}

/// The kept session plumbing is still there: deleting one of these takes the
/// tools, the trust store or the chat's mode decisions with it.
#[test]
fn the_live_session_plumbing_survives() {
    let root = crate_root();
    let missing: Vec<String> = KEPT
        .iter()
        .filter(|path| !root.join("src").join(path).exists())
        .map(|path| format!("src/{path} is gone"))
        .collect();
    assert!(
        missing.is_empty(),
        "the session plumbing a live front composes must stay ({} path(s)):\n{}",
        missing.len(),
        missing.join("\n")
    );
}

/// Nothing deletes a kept module's only reader: each live front still names the
/// module it reads, so a later unit that removes the caller (rather than
/// re-pointing it) is caught here.
#[test]
fn the_live_readers_still_read_them() {
    let texts = sources();
    let mut offenders: Vec<String> = Vec::new();
    for (file, needle) in LIVE_READERS {
        let found = texts
            .iter()
            .find(|(path, _)| path == file)
            .is_some_and(|(_, text)| text.contains(needle));
        if !found {
            offenders.push(format!("{file} no longer names `{needle}`"));
        }
    }
    assert!(
        offenders.is_empty(),
        "a kept session module's live reader moved or went away ({} \
         offender(s)):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// The request body is the old engine's only production caller: what deletes one
/// deletes both, and deleting the body alone leaves the engine and the session
/// surface it alone drives with no production reader at all — 78 dead-code
/// errors over more than 40 files, measured in `REPORT.md`.
#[test]
fn the_request_body_is_the_engines_only_production_caller() {
    let root = crate_root();
    let body = root
        .join("src/interactive")
        .join(concat!("session", "_request.rs"));
    assert!(
        body.exists(),
        "the request body is the engine's only production caller: deleting it \
         alone is the one-step dead-code cascade `REPORT.md` records, so it goes \
         with the engine in one commit"
    );
    // Outside the engine module, its own test modules and the request body,
    // nothing calls the engine's production entry: the chat, the line front and
    // the job engine do not. A doc comment may name it (it explains what the
    // engine awaits); a call is what makes a production caller.
    let call = format!("{ENGINE_ENTRY}(");
    let mut callers: Vec<String> = Vec::new();
    for (path, text) in sources() {
        let is_body = path.ends_with(concat!("session", "_request.rs"));
        let is_engine = path.starts_with("agent/runtime");
        if is_body || is_engine {
            continue;
        }
        let calls_it = text
            .lines()
            .any(|line| !line.trim_start().starts_with("//") && line.contains(&call));
        if calls_it {
            callers.push(path);
        }
    }
    assert!(
        callers.is_empty(),
        "these files call the old engine's production entry besides the request \
         body ({} caller(s)):\n{}",
        callers.len(),
        callers.join("\n")
    );
}
